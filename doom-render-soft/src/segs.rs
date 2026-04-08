// Copyright (C) 1993-1996 by id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

//! Translated from linuxdoom-1.10/r_segs.c and r_segs.h
//!
//! All the clipping: columns, horizontal spans, sky columns.
//! Handles wall segment rendering with texture mapping.
//!
//! This module contains the core wall rendering logic:
//! - [`SegsState::render_masked_seg_range`]: Renders masked (transparent)
//!   mid-textures on two-sided lines after solid geometry is complete.
//! - [`SegsState::render_seg_loop`]: The core per-column loop that draws upper,
//!   middle, and lower wall textures, marks floor/ceiling visplanes, and saves
//!   masked texture columns for later compositing.
//! - [`SegsState::store_wall_range`]: The largest function in the renderer —
//!   sets up all state for a visible wall range: distance, scale, textures,
//!   clipping, stepping, lighting, and dispatches to `render_seg_loop`.

use crate::data::DataState;
use crate::defs::{
    Angle, DrawSeg, Fixed, RenderState, ANG180, ANG90, ANGLETOFINESHIFT, FRACBITS, MAXDRAWSEGS,
    SCREENWIDTH, SIL_BOTH, SIL_BOTTOM, SIL_TOP,
};
use crate::draw::DrawState;
use crate::main::{
    scale_from_global_angle, ColFunc, RenderMain, LIGHTLEVELS, LIGHTSCALESHIFT, LIGHTSEGSHIFT,
    MAXLIGHTSCALE,
};
use crate::plane::PlaneState;
use crate::sky::SkyState;
use crate::things::ThingsState;
use doom_core::types::map_data::LineFlags;
use doom_core::types::tables::{FINESINE, FINETANGENT};

// ---------------------------------------------------------------------------
// Constants (r_segs.c lines 203-204)
// ---------------------------------------------------------------------------

/// Subpixel precision bits for wall vertical stepping (r_segs.c:203).
const HEIGHTBITS: i32 = 12;

/// One unit in the subpixel stepping system (r_segs.c:204).
const HEIGHTUNIT: i32 = 1 << HEIGHTBITS;

// ---------------------------------------------------------------------------
// SegsState — all mutable wall segment rendering state (r_segs.c globals)
// ---------------------------------------------------------------------------

/// Consolidates all formerly-global variables from `r_segs.c` (lines 40-95)
/// into a single owned struct, eliminating `static mut` and enabling the Rust
/// borrow checker to enforce safe access.
///
/// The struct holds:
/// - Texture visibility flags and indices
/// - Angle and distance state for the current wall segment
/// - Fixed-point stepping values for vertical texture mapping
/// - World-space height values for front/back sector boundaries
/// - Lighting table index
/// - Masked texture column storage
pub struct SegsState {
    // -- Texture visibility flags (r_segs.c lines 46-55) --
    /// True if any wall textures are visible on this segment (r_segs.c:46).
    /// Computed as bitwise OR of midtexture, toptexture, bottomtexture,
    /// and maskedtexture (converted to bool).
    pub segtextured: bool,

    /// True if the floor plane needs marking for this segment (r_segs.c:49).
    pub markfloor: bool,

    /// True if the ceiling plane needs marking for this segment (r_segs.c:50).
    pub markceiling: bool,

    /// True if the segment has a masked (transparent) mid-texture (r_segs.c:52).
    pub maskedtexture: bool,

    /// Upper texture index for two-sided lines (r_segs.c:53). 0 = none.
    pub toptexture: i32,

    /// Lower texture index for two-sided lines (r_segs.c:54). 0 = none.
    pub bottomtexture: i32,

    /// Middle texture index for single-sided lines (r_segs.c:55). 0 = none.
    pub midtexture: i32,

    // -- Angle state (r_segs.c lines 58-60) --
    /// Wall normal angle in BAM units (r_segs.c:58).
    /// Computed as `curline.angle + ANG90`.
    pub rw_normalangle: u32,

    /// Angle from viewer to line start vertex (r_segs.c:60).
    /// Stored as raw i32 matching the C `int rw_angle1`.
    pub rw_angle1: i32,

    // -- Regular wall stepping state (r_segs.c lines 65-74) --
    /// Current screen X column being processed (r_segs.c:65).
    pub rw_x: i32,

    /// Stop X column (exclusive) — loop runs while rw_x < rw_stopx (r_segs.c:66).
    pub rw_stopx: i32,

    /// Center angle for texture column calculation (r_segs.c:67).
    /// `ANG90 + viewangle - rw_normalangle`.
    pub rw_centerangle: u32,

    /// Horizontal texture offset in fixed-point (r_segs.c:68).
    /// Combines wall normal offset, sidedef texture offset, and seg offset.
    pub rw_offset: i32,

    /// Perpendicular distance to the wall in fixed-point (r_segs.c:69).
    pub rw_distance: i32,

    /// Current column scale in fixed-point (r_segs.c:70).
    pub rw_scale: i32,

    /// Scale increment per screen column in fixed-point (r_segs.c:71).
    pub rw_scalestep: i32,

    /// Vertical texture origin for middle texture (r_segs.c:72).
    pub rw_midtexturemid: i32,

    /// Vertical texture origin for upper texture (r_segs.c:73).
    pub rw_toptexturemid: i32,

    /// Vertical texture origin for lower texture (r_segs.c:74).
    pub rw_bottomtexturemid: i32,

    // -- World heights in fixed-point (r_segs.c lines 76-79) --
    // These are shifted >> 4 before use in stepping calculations.
    /// Front sector ceiling height minus viewz (r_segs.c:76).
    pub worldtop: i32,

    /// Front sector floor height minus viewz (r_segs.c:77).
    pub worldbottom: i32,

    /// Back sector ceiling height minus viewz (r_segs.c:78).
    pub worldhigh: i32,

    /// Back sector floor height minus viewz (r_segs.c:79).
    pub worldlow: i32,

    // -- Pixel stepping for two-sided lines (r_segs.c lines 81-84) --
    /// Upper wall bottom edge in subpixel fixed-point (r_segs.c:81).
    pub pixhigh: i32,

    /// Lower wall top edge in subpixel fixed-point (r_segs.c:82).
    pub pixlow: i32,

    /// Step for pixhigh per column (r_segs.c:83).
    pub pixhighstep: i32,

    /// Step for pixlow per column (r_segs.c:84).
    pub pixlowstep: i32,

    // -- Top/bottom frac stepping (r_segs.c lines 86-90) --
    /// Top edge of wall in subpixel fixed-point (r_segs.c:86).
    pub topfrac: i32,

    /// Step for topfrac per column (r_segs.c:87).
    pub topstep: i32,

    /// Bottom edge of wall in subpixel fixed-point (r_segs.c:89).
    pub bottomfrac: i32,

    /// Step for bottomfrac per column (r_segs.c:90).
    pub bottomstep: i32,

    // -- Lighting (r_segs.c line 93) --
    /// Index into `scalelight[][]` first dimension selecting the light level
    /// row for this wall segment (r_segs.c:93).
    pub walllights: usize,

    // -- Masked texture column storage (r_segs.c line 95) --
    /// Per-column texture column values for masked mid-textures.
    /// Stored in the openings buffer; this Vec is a working copy.
    pub maskedtexturecol: Vec<i16>,
}

impl SegsState {
    /// Creates a new `SegsState` with all fields zeroed / defaulted.
    pub fn new() -> Self {
        Self {
            segtextured: false,
            markfloor: false,
            markceiling: false,
            maskedtexture: false,
            toptexture: 0,
            bottomtexture: 0,
            midtexture: 0,
            rw_normalangle: 0,
            rw_angle1: 0,
            rw_x: 0,
            rw_stopx: 0,
            rw_centerangle: 0,
            rw_offset: 0,
            rw_distance: 0,
            rw_scale: 0,
            rw_scalestep: 0,
            rw_midtexturemid: 0,
            rw_toptexturemid: 0,
            rw_bottomtexturemid: 0,
            worldtop: 0,
            worldbottom: 0,
            worldhigh: 0,
            worldlow: 0,
            pixhigh: 0,
            pixlow: 0,
            pixhighstep: 0,
            pixlowstep: 0,
            topfrac: 0,
            topstep: 0,
            bottomfrac: 0,
            bottomstep: 0,
            walllights: 0,
            maskedtexturecol: Vec::new(),
        }
    }
}

impl SegsState {
    // =========================================================================
    // R_RenderMaskedSegRange (r_segs.c lines 102-190)
    // =========================================================================

    /// Renders masked (transparent) mid-textures on two-sided lines.
    ///
    /// Called **after** all solid geometry has been rendered, during the masked
    /// column compositing pass. For each column in `[x1, x2]`, computes the
    /// correct lighting, inverse scale, and texture vertical position, then
    /// dispatches to [`ThingsState::draw_masked_column`] to actually draw the
    /// transparent pixels.
    ///
    /// Original C: `R_RenderMaskedSegRange` (r_segs.c lines 102-190).
    ///
    /// # Arguments
    /// * `ds` — The drawseg describing this wall segment
    /// * `x1` — Left screen column (inclusive)
    /// * `x2` — Right screen column (inclusive)
    /// * `render_state` — Shared renderer state (map geometry, viewz)
    /// * `render_main` — Renderer main state (lighting, view params)
    /// * `draw` — Column drawing state (dc_* parameters)
    /// * `plane` — Plane state (openings buffer for maskedtexturecol)
    /// * `data` — Texture data cache
    /// * `things` — Sprite/masked column rendering state
    /// * `screens` — Screen pixel buffers
    /// * `colormaps` — Colormap data (passed through to column drawing)
    #[allow(clippy::too_many_arguments)]
    pub fn render_masked_seg_range(
        &mut self,
        ds: &DrawSeg,
        x1: i32,
        x2: i32,
        render_state: &RenderState,
        render_main: &RenderMain,
        draw: &mut DrawState,
        plane: &mut PlaneState,
        data: &mut DataState,
        things: &mut ThingsState,
        screens: &mut [Vec<u8>],
        _colormaps: &[u8],
    ) {
        // Retrieve map structures via indices.
        let curline = &render_state.segs[ds.curline];
        let frontsector = &render_state.sectors[curline.frontsector];

        // Resolve the animated texture frame for the masked mid-texture.
        let sidedef = &render_state.sides[curline.sidedef];
        let texnum = data.texturetranslation[sidedef.midtexture as usize] as usize;

        // Calculate light table.
        // Use different light tables for horizontal / vertical / diagonal.
        let mut lightnum =
            (frontsector.lightlevel as i32 >> LIGHTSEGSHIFT) + render_main.extralight;

        // Determine the seg's V1 and V2 vertices for orientation check.
        let v1 = &render_state.vertexes[curline.v1];
        let v2 = &render_state.vertexes[curline.v2];

        if v1.y == v2.y {
            lightnum -= 1; // horizontal — slightly darker
        } else if v1.x == v2.x {
            lightnum += 1; // vertical — slightly brighter
        }

        // Clamp lightnum to valid scalelight range.
        let light_idx = if lightnum < 0 {
            0
        } else if lightnum >= LIGHTLEVELS as i32 {
            LIGHTLEVELS - 1
        } else {
            lightnum as usize
        };
        self.walllights = light_idx;

        // maskedtexturecol offset into openings buffer
        let mtc_offset = match ds.maskedtexturecol {
            Some(off) => off,
            None => return, // No masked texture column data — nothing to draw.
        };

        self.rw_scalestep = ds.scalestep.raw();
        things.spryscale = Fixed::new(
            ds.scale1
                .raw()
                .wrapping_add((x1 - ds.x1).wrapping_mul(self.rw_scalestep)),
        );

        // Set sprite clip arrays from the drawseg.
        // sprbottomclip / sprtopclip are offsets into openings.
        if let Some(bot_off) = ds.sprbottomclip {
            things.mfloorclip = plane.openings[bot_off..].to_vec();
        }
        if let Some(top_off) = ds.sprtopclip {
            things.mceilingclip = plane.openings[top_off..].to_vec();
        }

        // Find texture vertical positioning based on pegging flags.
        let linedef = &render_state.lines[curline.linedef];
        if linedef.flags & LineFlags::ML_DONTPEGBOTTOM.bits() != 0 {
            // Bottom of texture at floor — use max of front/back floor heights.
            let backsector_idx = match curline.backsector {
                Some(idx) => idx,
                None => return,
            };
            let backsector = &render_state.sectors[backsector_idx];
            let base_height = if frontsector.floorheight.raw() > backsector.floorheight.raw() {
                frontsector.floorheight.raw()
            } else {
                backsector.floorheight.raw()
            };
            draw.dc_texturemid = base_height
                .wrapping_add(data.textureheight[texnum])
                .wrapping_sub(render_state.viewz);
        } else {
            // Top of texture at ceiling — use min of front/back ceiling heights.
            let backsector_idx = match curline.backsector {
                Some(idx) => idx,
                None => return,
            };
            let backsector = &render_state.sectors[backsector_idx];
            let base_height = if frontsector.ceilingheight.raw() < backsector.ceilingheight.raw() {
                frontsector.ceilingheight.raw()
            } else {
                backsector.ceilingheight.raw()
            };
            draw.dc_texturemid = base_height.wrapping_sub(render_state.viewz);
        }
        draw.dc_texturemid = draw.dc_texturemid.wrapping_add(sidedef.rowoffset.raw());

        // If fullbright colormap is active, set it once.
        if let Some(fcm) = render_main.fixedcolormap {
            draw.dc_colormap = fcm;
        }

        // Draw the columns from x1 to x2 (inclusive).
        draw.dc_x = x1;
        while draw.dc_x <= x2 {
            // Read maskedtexturecol value from openings buffer.
            // Convention: mtc_offset is stored as (lastopening - rw_x), so
            // the actual index is mtc_offset + dc_x. But we must handle
            // the signed offset convention carefully.
            let otc_idx = (mtc_offset as isize + draw.dc_x as isize) as usize;
            let mtc_val = if otc_idx < plane.openings.len() {
                plane.openings[otc_idx]
            } else {
                i16::MAX // Sentinel: already drawn or out of bounds.
            };

            if mtc_val != i16::MAX {
                // Calculate per-column lighting (unless fullbright).
                if render_main.fixedcolormap.is_none() {
                    let index = (things.spryscale.raw() as u32 >> LIGHTSCALESHIFT) as usize;
                    let index = if index >= MAXLIGHTSCALE {
                        MAXLIGHTSCALE - 1
                    } else {
                        index
                    };
                    draw.dc_colormap = render_main.scalelight[self.walllights][index];
                }

                things.sprtopscreen = Fixed::new(
                    render_main.centeryfrac.raw().wrapping_sub(
                        Fixed::new(draw.dc_texturemid)
                            .fixed_mul(things.spryscale)
                            .raw(),
                    ),
                );
                draw.dc_iscale = (0xFFFF_FFFFu32 / (things.spryscale.raw() as u32)) as i32;

                // Get the texture column data. The C code subtracts 3 from the
                // column pointer because R_DrawMaskedColumn reads the post header
                // starting at offset 0 (topdelta byte) — the R_GetColumn return
                // already points past the 3-byte header. Our draw_masked_column
                // handles this by taking a col_offset parameter.
                let column_data = data.get_column(texnum, mtc_val as i32);
                // The column data is returned starting at column pixels; pass
                // offset 0 since get_column returns the full column structure.
                // We pass 0 because our masked column drawer handles the post
                // structure from the start of the returned slice.
                let col_data_owned = column_data.to_vec();

                things.draw_masked_column(&col_data_owned, 0, draw, render_main, data, screens);

                // Mark column as drawn.
                if otc_idx < plane.openings.len() {
                    plane.openings[otc_idx] = i16::MAX;
                }
            }
            things.spryscale = Fixed::new(things.spryscale.raw().wrapping_add(self.rw_scalestep));
            draw.dc_x += 1;
        }
    }

    // =========================================================================
    // R_RenderSegLoop (r_segs.c lines 206-364)
    // =========================================================================

    /// The core per-column wall rendering loop.
    ///
    /// Iterates from `rw_x` to `rw_stopx` (exclusive), and for each screen
    /// column:
    /// 1. Marks ceiling and floor visplane boundaries
    /// 2. Computes texture column and lighting (if textured)
    /// 3. Draws the middle wall (single-sided) or upper/lower walls (two-sided)
    /// 4. Updates clip arrays for sprite rendering
    /// 5. Saves masked texture columns for later compositing
    ///
    /// Original C: `R_RenderSegLoop` (r_segs.c lines 206-364).
    ///
    /// # Arguments
    /// * `render_state` — Shared renderer state (xtoviewangle, map data)
    /// * `render_main` — Renderer main state (lighting, centery, colfunc)
    /// * `draw` — Column drawing state
    /// * `plane` — Plane state (clip arrays, visplanes, openings)
    /// * `data` — Texture data cache
    /// * `screens` — Screen pixel buffers
    /// * `colormaps` — Colormap data for lighting
    #[allow(clippy::too_many_arguments)]
    pub fn render_seg_loop(
        &mut self,
        render_state: &RenderState,
        render_main: &RenderMain,
        draw: &mut DrawState,
        plane: &mut PlaneState,
        data: &mut DataState,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
    ) {
        let mut texturecolumn: i32 = 0;

        while self.rw_x < self.rw_stopx {
            let rw_x = self.rw_x;
            let rw_x_u = rw_x as usize;

            // -- Mark floor / ceiling areas --
            let mut yl = (self.topfrac.wrapping_add(HEIGHTUNIT - 1)) >> HEIGHTBITS;

            // No space above wall?
            if yl < (plane.ceilingclip[rw_x_u] as i32) + 1 {
                yl = (plane.ceilingclip[rw_x_u] as i32) + 1;
            }

            if self.markceiling {
                let top = (plane.ceilingclip[rw_x_u] as i32) + 1;
                let mut bottom = yl - 1;

                if bottom >= plane.floorclip[rw_x_u] as i32 {
                    bottom = (plane.floorclip[rw_x_u] as i32) - 1;
                }

                if top <= bottom {
                    if let Some(cp_idx) = plane.ceilingplane {
                        if rw_x_u < SCREENWIDTH as usize {
                            plane.visplanes[cp_idx].top[rw_x_u] = top as u8;
                            plane.visplanes[cp_idx].bottom[rw_x_u] = bottom as u8;
                        }
                    }
                }
            }

            let mut yh = self.bottomfrac >> HEIGHTBITS;

            if yh >= plane.floorclip[rw_x_u] as i32 {
                yh = (plane.floorclip[rw_x_u] as i32) - 1;
            }

            if self.markfloor {
                let mut top = yh + 1;
                let bottom = (plane.floorclip[rw_x_u] as i32) - 1;
                if top <= plane.ceilingclip[rw_x_u] as i32 {
                    top = (plane.ceilingclip[rw_x_u] as i32) + 1;
                }
                if top <= bottom {
                    if let Some(fp_idx) = plane.floorplane {
                        if rw_x_u < SCREENWIDTH as usize {
                            plane.visplanes[fp_idx].top[rw_x_u] = top as u8;
                            plane.visplanes[fp_idx].bottom[rw_x_u] = bottom as u8;
                        }
                    }
                }
            }

            // -- Texture column and lighting (independent of wall tiers) --
            if self.segtextured {
                // Calculate texture offset.
                // angle = (rw_centerangle + xtoviewangle[rw_x]) >> ANGLETOFINESHIFT
                let xta = if rw_x_u < render_state.xtoviewangle.len() {
                    render_state.xtoviewangle[rw_x_u]
                } else {
                    0
                };
                let angle = (self.rw_centerangle.wrapping_add(xta)) >> ANGLETOFINESHIFT;
                let angle_idx = angle as usize & 0x1FFF; // mask to 8191 for safety

                texturecolumn = self.rw_offset.wrapping_sub(
                    Fixed::new(FINETANGENT[angle_idx].raw())
                        .fixed_mul(Fixed::new(self.rw_distance))
                        .raw(),
                );
                texturecolumn >>= FRACBITS;

                // Calculate lighting.
                let index = (self.rw_scale as u32 >> LIGHTSCALESHIFT) as usize;
                let index = if index >= MAXLIGHTSCALE {
                    MAXLIGHTSCALE - 1
                } else {
                    index
                };

                draw.dc_colormap = render_main.scalelight[self.walllights][index];
                draw.dc_x = rw_x;
                draw.dc_iscale = (0xFFFF_FFFFu32 / (self.rw_scale as u32)) as i32;
            }

            // -- Draw the wall tiers --
            if self.midtexture != 0 {
                // Single-sided line — draw full middle texture.
                draw.dc_yl = yl;
                draw.dc_yh = yh;
                draw.dc_texturemid = self.rw_midtexturemid;
                draw.dc_source = data
                    .get_column(self.midtexture as usize, texturecolumn)
                    .to_vec();

                // Dispatch column function.
                Self::dispatch_colfunc(draw, render_main, screens, colormaps);

                // Single-sided: mark clips to block everything behind.
                plane.ceilingclip[rw_x_u] = draw.viewheight as i16;
                plane.floorclip[rw_x_u] = -1;
            } else {
                // Two-sided line — draw upper and lower textures.
                if self.toptexture != 0 {
                    // Top wall.
                    let mid = self.pixhigh >> HEIGHTBITS;
                    self.pixhigh = self.pixhigh.wrapping_add(self.pixhighstep);

                    let mut mid = mid;
                    if mid >= plane.floorclip[rw_x_u] as i32 {
                        mid = (plane.floorclip[rw_x_u] as i32) - 1;
                    }

                    if mid >= yl {
                        draw.dc_yl = yl;
                        draw.dc_yh = mid;
                        draw.dc_texturemid = self.rw_toptexturemid;
                        draw.dc_source = data
                            .get_column(self.toptexture as usize, texturecolumn)
                            .to_vec();

                        Self::dispatch_colfunc(draw, render_main, screens, colormaps);
                        plane.ceilingclip[rw_x_u] = mid as i16;
                    } else {
                        plane.ceilingclip[rw_x_u] = (yl - 1) as i16;
                    }
                } else {
                    // No top wall.
                    if self.markceiling {
                        plane.ceilingclip[rw_x_u] = (yl - 1) as i16;
                    }
                }

                if self.bottomtexture != 0 {
                    // Bottom wall.
                    let mid = (self.pixlow.wrapping_add(HEIGHTUNIT - 1)) >> HEIGHTBITS;
                    self.pixlow = self.pixlow.wrapping_add(self.pixlowstep);

                    let mut mid = mid;
                    // No space above wall?
                    if mid <= plane.ceilingclip[rw_x_u] as i32 {
                        mid = (plane.ceilingclip[rw_x_u] as i32) + 1;
                    }

                    if mid <= yh {
                        draw.dc_yl = mid;
                        draw.dc_yh = yh;
                        draw.dc_texturemid = self.rw_bottomtexturemid;
                        draw.dc_source = data
                            .get_column(self.bottomtexture as usize, texturecolumn)
                            .to_vec();

                        Self::dispatch_colfunc(draw, render_main, screens, colormaps);
                        plane.floorclip[rw_x_u] = mid as i16;
                    } else {
                        plane.floorclip[rw_x_u] = (yh + 1) as i16;
                    }
                } else {
                    // No bottom wall.
                    if self.markfloor {
                        plane.floorclip[rw_x_u] = (yh + 1) as i16;
                    }
                }

                if self.maskedtexture {
                    // Save texturecolumn for later masked mid-texture drawing.
                    if rw_x_u < self.maskedtexturecol.len() {
                        self.maskedtexturecol[rw_x_u] = texturecolumn as i16;
                    }
                }
            }

            // Advance stepping state.
            self.rw_scale = self.rw_scale.wrapping_add(self.rw_scalestep);
            self.topfrac = self.topfrac.wrapping_add(self.topstep);
            self.bottomfrac = self.bottomfrac.wrapping_add(self.bottomstep);
            self.rw_x += 1;
        }
    }

    // =========================================================================
    // R_StoreWallRange (r_segs.c lines 374-745)
    // =========================================================================

    /// Sets up all state for a visible wall range and dispatches rendering.
    ///
    /// This is the largest and most complex function in the renderer. It is
    /// called from the BSP clipping code when a visible wall segment has been
    /// identified. It:
    /// 1. Allocates a drawseg and sets its basic geometry
    /// 2. Computes perpendicular distance to the wall
    /// 3. Determines scale at both endpoints and stepping
    /// 4. Identifies front/back sector boundaries and texture assignments
    /// 5. Handles texture pegging (ML_DONTPEGTOP, ML_DONTPEGBOTTOM)
    /// 6. Implements the sky hack (matching sky ceilings)
    /// 7. Selects lighting tables based on seg orientation
    /// 8. Computes vertical stepping values
    /// 9. Marks/validates ceiling and floor visplanes
    /// 10. Calls [`render_seg_loop`] for actual pixel drawing
    /// 11. Saves sprite clipping info for later compositing
    ///
    /// Original C: `R_StoreWallRange` (r_segs.c lines 374-745).
    ///
    /// # Arguments
    /// * `start` — Left screen column of the wall range (inclusive)
    /// * `stop` — Right screen column of the wall range (inclusive)
    /// * `curline_idx` — Index of the current seg in `render_state.segs`
    /// * `render_state` — Shared renderer state (mutable for rw_distance/rw_normalangle sync)
    /// * `render_main` — Renderer main state
    /// * `draw` — Column drawing state
    /// * `plane` — Plane state (mutable for clip arrays and openings)
    /// * `data` — Texture data cache
    /// * `things` — Sprite/masked column state (sentinel arrays)
    /// * `sky` — Sky state (skyflatnum for sky hack)
    /// * `drawsegs` — Drawseg pool (mutable, new drawseg appended)
    /// * `screens` — Screen pixel buffers
    /// * `colormaps` — Colormap data for lighting
    #[allow(clippy::too_many_arguments)]
    pub fn store_wall_range(
        &mut self,
        start: i32,
        stop: i32,
        curline_idx: usize,
        render_state: &mut RenderState,
        render_main: &RenderMain,
        draw: &mut DrawState,
        plane: &mut PlaneState,
        data: &mut DataState,
        _things: &ThingsState,
        sky: &SkyState,
        drawsegs: &mut Vec<DrawSeg>,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
    ) {
        // Don't overflow the drawseg pool.
        if drawsegs.len() >= MAXDRAWSEGS {
            tracing::warn!(
                "R_StoreWallRange: drawseg overflow (MAXDRAWSEGS={})",
                MAXDRAWSEGS
            );
            return;
        }

        #[cfg(debug_assertions)]
        {
            if start >= render_main.viewwidth || start > stop {
                tracing::error!(
                    "Bad R_RenderWallRange: {} to {} (viewwidth={})",
                    start,
                    stop,
                    render_main.viewwidth
                );
            }
        }

        // Retrieve map structures from indices.
        let curline = &render_state.segs[curline_idx];
        let sidedef_idx = curline.sidedef;
        let linedef_idx = curline.linedef;
        let frontsector_idx = curline.frontsector;
        let backsector_opt = curline.backsector;

        // Capture seg vertex coordinates and properties we need repeatedly.
        let seg_v1 = curline.v1;
        let seg_v2 = curline.v2;
        let seg_angle = curline.angle;
        let seg_offset = curline.offset;

        // Mark the linedef as visible for the automap.
        render_state.lines[linedef_idx].flags |= LineFlags::ML_MAPPED.bits();

        // Calculate rw_distance for scale calculation.
        // rw_normalangle = curline->angle + ANG90
        self.rw_normalangle = seg_angle.value().wrapping_add(ANG90.value());

        // offsetangle = abs(rw_normalangle - rw_angle1)
        let raw_offset_angle = self.rw_normalangle.wrapping_sub(self.rw_angle1 as u32);
        let mut offsetangle: u32 = if raw_offset_angle > ANG180.value() {
            0u32.wrapping_sub(raw_offset_angle)
        } else {
            raw_offset_angle
        };

        if offsetangle > ANG90.value() {
            offsetangle = ANG90.value();
        }

        let distangle = ANG90.value().wrapping_sub(offsetangle);
        let v1_vtx = &render_state.vertexes[seg_v1];
        let hyp = crate::main::point_to_dist(v1_vtx.x, v1_vtx.y, render_main);
        let sineval = FINESINE[(distangle >> ANGLETOFINESHIFT) as usize & 0x1FFF].raw();
        self.rw_distance = hyp.fixed_mul(Fixed::new(sineval)).raw();

        // Sync distance and normal angle to RenderState for scale_from_global_angle.
        render_state.rw_distance = self.rw_distance;
        render_state.rw_normalangle = self.rw_normalangle;

        // Initialize drawseg.
        let mut ds = DrawSeg::default();
        self.rw_x = start;
        ds.x1 = start;
        ds.x2 = stop;
        ds.curline = curline_idx;
        self.rw_stopx = stop + 1;

        // Calculate scale at both endpoints and step.
        let start_u = start as usize;
        let stop_u = stop as usize;
        let xta_start = if start_u < render_state.xtoviewangle.len() {
            render_state.xtoviewangle[start_u]
        } else {
            0
        };
        let visangle_start = Angle::new(render_main.viewangle.value().wrapping_add(xta_start));
        self.rw_scale = scale_from_global_angle(visangle_start, render_main, render_state).raw();
        ds.scale1 = Fixed::new(self.rw_scale);

        if stop > start {
            let xta_stop = if stop_u < render_state.xtoviewangle.len() {
                render_state.xtoviewangle[stop_u]
            } else {
                0
            };
            let visangle_stop = Angle::new(render_main.viewangle.value().wrapping_add(xta_stop));
            ds.scale2 = scale_from_global_angle(visangle_stop, render_main, render_state);
            ds.scalestep =
                Fixed::new((ds.scale2.raw().wrapping_sub(self.rw_scale)) / (stop - start));
            self.rw_scalestep = ds.scalestep.raw();
        } else {
            ds.scale2 = ds.scale1;
            self.rw_scalestep = 0;
        }

        // Calculate texture boundaries and decide if floor/ceiling marks needed.
        let frontsector = &render_state.sectors[frontsector_idx];
        self.worldtop = frontsector
            .ceilingheight
            .raw()
            .wrapping_sub(render_state.viewz);
        self.worldbottom = frontsector
            .floorheight
            .raw()
            .wrapping_sub(render_state.viewz);

        self.midtexture = 0;
        self.toptexture = 0;
        self.bottomtexture = 0;
        self.maskedtexture = false;
        ds.maskedtexturecol = None;

        let sidedef = &render_state.sides[sidedef_idx];
        let linedef = &render_state.lines[linedef_idx];
        let lineflags = linedef.flags;

        if backsector_opt.is_none() {
            // ---- Single-sided line ----
            self.midtexture = data.texturetranslation[sidedef.midtexture as usize];

            // A single-sided line is terminal — must mark both ends.
            self.markfloor = true;
            self.markceiling = true;

            if lineflags & LineFlags::ML_DONTPEGBOTTOM.bits() != 0 {
                let vtop = frontsector
                    .floorheight
                    .raw()
                    .wrapping_add(data.textureheight[sidedef.midtexture as usize]);
                // Bottom of texture at bottom.
                self.rw_midtexturemid = vtop.wrapping_sub(render_state.viewz);
            } else {
                // Top of texture at top.
                self.rw_midtexturemid = self.worldtop;
            }
            self.rw_midtexturemid = self.rw_midtexturemid.wrapping_add(sidedef.rowoffset.raw());

            ds.silhouette = SIL_BOTH;
            // Sprite clip will be saved at the end of this function.
            ds.sprtopclip = None;
            ds.sprbottomclip = None;
            ds.bsilheight = Fixed::new(i32::MAX);
            ds.tsilheight = Fixed::new(i32::MIN);
        } else if let Some(backsector_idx) = backsector_opt {
            // ---- Two-sided line ----
            let backsector = &render_state.sectors[backsector_idx];

            ds.sprtopclip = None;
            ds.sprbottomclip = None;
            ds.silhouette = 0;

            if frontsector.floorheight.raw() > backsector.floorheight.raw() {
                ds.silhouette = SIL_BOTTOM;
                ds.bsilheight = frontsector.floorheight;
            } else if backsector.floorheight.raw() > render_state.viewz {
                ds.silhouette = SIL_BOTTOM;
                ds.bsilheight = Fixed::new(i32::MAX);
            }

            if frontsector.ceilingheight.raw() < backsector.ceilingheight.raw() {
                ds.silhouette |= SIL_TOP;
                ds.tsilheight = frontsector.ceilingheight;
            } else if backsector.ceilingheight.raw() < render_state.viewz {
                ds.silhouette |= SIL_TOP;
                ds.tsilheight = Fixed::new(i32::MIN);
            }

            if backsector.ceilingheight.raw() <= frontsector.floorheight.raw() {
                ds.sprbottomclip = None; // Will be set during post-render clip save.
                ds.bsilheight = Fixed::new(i32::MAX);
                ds.silhouette |= SIL_BOTTOM;
            }

            if backsector.floorheight.raw() >= frontsector.ceilingheight.raw() {
                ds.sprtopclip = None; // Will be set during post-render clip save.
                ds.tsilheight = Fixed::new(i32::MIN);
                ds.silhouette |= SIL_TOP;
            }

            self.worldhigh = backsector
                .ceilingheight
                .raw()
                .wrapping_sub(render_state.viewz);
            self.worldlow = backsector
                .floorheight
                .raw()
                .wrapping_sub(render_state.viewz);

            // Sky hack: if both sectors have sky ceiling, treat upper wall as invisible.
            if frontsector.ceilingpic == sky.skyflatnum as i16
                && backsector.ceilingpic == sky.skyflatnum as i16
            {
                self.worldtop = self.worldhigh;
            }

            // Determine if floor/ceiling planes need marking.
            self.markfloor = self.worldlow != self.worldbottom
                || backsector.floorpic != frontsector.floorpic
                || backsector.lightlevel != frontsector.lightlevel;

            self.markceiling = self.worldhigh != self.worldtop
                || backsector.ceilingpic != frontsector.ceilingpic
                || backsector.lightlevel != frontsector.lightlevel;

            // Closed door — mark both.
            if backsector.ceilingheight.raw() <= frontsector.floorheight.raw()
                || backsector.floorheight.raw() >= frontsector.ceilingheight.raw()
            {
                self.markceiling = true;
                self.markfloor = true;
            }

            // Upper texture.
            if self.worldhigh < self.worldtop {
                self.toptexture = data.texturetranslation[sidedef.toptexture as usize];

                if lineflags & LineFlags::ML_DONTPEGTOP.bits() != 0 {
                    // Top of texture at top.
                    self.rw_toptexturemid = self.worldtop;
                } else {
                    let vtop = backsector
                        .ceilingheight
                        .raw()
                        .wrapping_add(data.textureheight[sidedef.toptexture as usize]);
                    // Bottom of texture.
                    self.rw_toptexturemid = vtop.wrapping_sub(render_state.viewz);
                }
            }

            // Lower texture.
            if self.worldlow > self.worldbottom {
                self.bottomtexture = data.texturetranslation[sidedef.bottomtexture as usize];

                if lineflags & LineFlags::ML_DONTPEGBOTTOM.bits() != 0 {
                    // Bottom of texture at bottom, top at top.
                    self.rw_bottomtexturemid = self.worldtop;
                } else {
                    // Top of texture at top.
                    self.rw_bottomtexturemid = self.worldlow;
                }
            }

            self.rw_toptexturemid = self.rw_toptexturemid.wrapping_add(sidedef.rowoffset.raw());
            self.rw_bottomtexturemid = self
                .rw_bottomtexturemid
                .wrapping_add(sidedef.rowoffset.raw());

            // Allocate space for masked texture columns in the openings buffer.
            if sidedef.midtexture != 0 {
                self.maskedtexture = true;
                // The C code stores maskedtexturecol = lastopening - rw_x
                // so that accessing maskedtexturecol[col] gets openings[lastopening - rw_x + col].
                let mtc_base = plane.lastopening;
                ds.maskedtexturecol = Some(mtc_base.wrapping_sub(self.rw_x as usize));

                // Initialize masked texture columns with sentinel values.
                let range = (self.rw_stopx - self.rw_x) as usize;
                while plane.openings.len() < mtc_base + range {
                    plane.openings.push(0);
                }
                for i in 0..range {
                    plane.openings[mtc_base + i] = i16::MAX;
                }
                // Prepare the working maskedtexturecol vector.
                self.maskedtexturecol = vec![i16::MAX; (SCREENWIDTH + 1) as usize];

                plane.lastopening += range;
            }
        }

        // Calculate rw_offset (only needed for textured lines).
        let segtextured_int = self.midtexture
            | self.toptexture
            | self.bottomtexture
            | if self.maskedtexture { 1 } else { 0 };
        self.segtextured = segtextured_int != 0;

        if self.segtextured {
            // Offset angle for texture horizontal offset calculation.
            let offset_raw = self.rw_normalangle.wrapping_sub(self.rw_angle1 as u32);
            let mut offset_angle = if offset_raw > ANG180.value() {
                0u32.wrapping_sub(offset_raw)
            } else {
                offset_raw
            };

            if offset_angle > ANG90.value() {
                offset_angle = ANG90.value();
            }

            let sineval = FINESINE[(offset_angle >> ANGLETOFINESHIFT) as usize & 0x1FFF].raw();
            self.rw_offset = hyp.fixed_mul(Fixed::new(sineval)).raw();

            if self.rw_normalangle.wrapping_sub(self.rw_angle1 as u32) < ANG180.value() {
                self.rw_offset = self.rw_offset.wrapping_neg();
            }

            self.rw_offset = self
                .rw_offset
                .wrapping_add(sidedef.textureoffset.raw())
                .wrapping_add(seg_offset.raw());

            self.rw_centerangle = ANG90
                .value()
                .wrapping_add(render_state.viewangle)
                .wrapping_sub(self.rw_normalangle);

            // Calculate light table.
            if render_main.fixedcolormap.is_none() {
                let v1_vtx = &render_state.vertexes[seg_v1];
                let v2_vtx = &render_state.vertexes[seg_v2];
                let frontsector = &render_state.sectors[frontsector_idx];
                let mut lightnum =
                    (frontsector.lightlevel as i32 >> LIGHTSEGSHIFT) + render_main.extralight;

                if v1_vtx.y == v2_vtx.y {
                    lightnum -= 1; // horizontal — slightly darker
                } else if v1_vtx.x == v2_vtx.x {
                    lightnum += 1; // vertical — slightly brighter
                }

                if lightnum < 0 {
                    self.walllights = 0;
                } else if lightnum >= LIGHTLEVELS as i32 {
                    self.walllights = LIGHTLEVELS - 1;
                } else {
                    self.walllights = lightnum as usize;
                }
            }
        }

        // If a floor/ceiling plane is on the wrong side of the view plane,
        // it is definitely invisible and doesn't need to be marked.
        let frontsector = &render_state.sectors[frontsector_idx];
        if frontsector.floorheight.raw() >= render_state.viewz {
            // Floor is above view plane.
            self.markfloor = false;
        }

        if frontsector.ceilingheight.raw() <= render_state.viewz
            && frontsector.ceilingpic != sky.skyflatnum as i16
        {
            // Ceiling is below view plane.
            self.markceiling = false;
        }

        // Calculate incremental stepping values for texture edges.
        // The >>4 shift prevents overflow during fixed-point multiply.
        self.worldtop >>= 4;
        self.worldbottom >>= 4;

        self.topstep = Fixed::new(self.rw_scalestep)
            .fixed_mul(Fixed::new(self.worldtop))
            .raw()
            .wrapping_neg();
        self.topfrac = (render_main.centeryfrac.raw() >> 4).wrapping_sub(
            Fixed::new(self.worldtop)
                .fixed_mul(Fixed::new(self.rw_scale))
                .raw(),
        );

        self.bottomstep = Fixed::new(self.rw_scalestep)
            .fixed_mul(Fixed::new(self.worldbottom))
            .raw()
            .wrapping_neg();
        self.bottomfrac = (render_main.centeryfrac.raw() >> 4).wrapping_sub(
            Fixed::new(self.worldbottom)
                .fixed_mul(Fixed::new(self.rw_scale))
                .raw(),
        );

        if backsector_opt.is_some() {
            self.worldhigh >>= 4;
            self.worldlow >>= 4;

            if self.worldhigh < self.worldtop {
                self.pixhigh = (render_main.centeryfrac.raw() >> 4).wrapping_sub(
                    Fixed::new(self.worldhigh)
                        .fixed_mul(Fixed::new(self.rw_scale))
                        .raw(),
                );
                self.pixhighstep = Fixed::new(self.rw_scalestep)
                    .fixed_mul(Fixed::new(self.worldhigh))
                    .raw()
                    .wrapping_neg();
            }

            if self.worldlow > self.worldbottom {
                self.pixlow = (render_main.centeryfrac.raw() >> 4).wrapping_sub(
                    Fixed::new(self.worldlow)
                        .fixed_mul(Fixed::new(self.rw_scale))
                        .raw(),
                );
                self.pixlowstep = Fixed::new(self.rw_scalestep)
                    .fixed_mul(Fixed::new(self.worldlow))
                    .raw()
                    .wrapping_neg();
            }
        }

        // Render it: check planes, then draw.
        if self.markceiling {
            if let Some(cp_idx) = plane.ceilingplane {
                plane.ceilingplane = Some(plane.check_plane(cp_idx, self.rw_x, self.rw_stopx - 1));
            }
        }

        if self.markfloor {
            if let Some(fp_idx) = plane.floorplane {
                plane.floorplane = Some(plane.check_plane(fp_idx, self.rw_x, self.rw_stopx - 1));
            }
        }

        self.render_seg_loop(
            render_state,
            render_main,
            draw,
            plane,
            data,
            screens,
            colormaps,
        );

        // ---- Save sprite clipping info ----
        // Copy ceiling/floor clip arrays into openings for later sprite clipping.
        if ((ds.silhouette & SIL_TOP) != 0 || self.maskedtexture) && ds.sprtopclip.is_none() {
            let clip_start = start as usize;
            let clip_count = (self.rw_stopx - start) as usize;
            let save_offset = plane.lastopening;

            // Ensure openings buffer has enough space.
            while plane.openings.len() < save_offset + clip_count {
                plane.openings.push(0);
            }
            for i in 0..clip_count {
                plane.openings[save_offset + i] = plane.ceilingclip[clip_start + i];
            }
            ds.sprtopclip = Some(save_offset.wrapping_sub(clip_start));
            plane.lastopening += clip_count;
        }

        if ((ds.silhouette & SIL_BOTTOM) != 0 || self.maskedtexture) && ds.sprbottomclip.is_none() {
            let clip_start = start as usize;
            let clip_count = (self.rw_stopx - start) as usize;
            let save_offset = plane.lastopening;

            while plane.openings.len() < save_offset + clip_count {
                plane.openings.push(0);
            }
            for i in 0..clip_count {
                plane.openings[save_offset + i] = plane.floorclip[clip_start + i];
            }
            ds.sprbottomclip = Some(save_offset.wrapping_sub(clip_start));
            plane.lastopening += clip_count;
        }

        // If masked texture but no top silhouette, add it for sprite clipping.
        if self.maskedtexture && (ds.silhouette & SIL_TOP) == 0 {
            ds.silhouette |= SIL_TOP;
            ds.tsilheight = Fixed::new(i32::MIN);
        }
        if self.maskedtexture && (ds.silhouette & SIL_BOTTOM) == 0 {
            ds.silhouette |= SIL_BOTTOM;
            ds.bsilheight = Fixed::new(i32::MAX);
        }

        // Also copy maskedtexturecol entries into openings if we have a masked texture.
        if self.maskedtexture {
            if let Some(mtc_off) = ds.maskedtexturecol {
                for x in (start as usize)..(self.rw_stopx as usize) {
                    let otc_idx = (mtc_off as isize + x as isize) as usize;
                    if otc_idx < plane.openings.len() && x < self.maskedtexturecol.len() {
                        plane.openings[otc_idx] = self.maskedtexturecol[x];
                    }
                }
            }
        }

        // Advance drawseg pointer (append to pool).
        drawsegs.push(ds);
    }

    /// Dispatches column drawing based on the current [`ColFunc`] enum.
    ///
    /// Matches the C `colfunc()` function pointer dispatch pattern. Calls the
    /// appropriate draw function on [`DrawState`] based on the rendering mode.
    fn dispatch_colfunc(
        draw: &mut DrawState,
        render_main: &RenderMain,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
    ) {
        match render_main.colfunc {
            ColFunc::DrawColumn => {
                draw.draw_column(screens, colormaps, render_main.centery);
            }
            ColFunc::DrawColumnLow => {
                draw.draw_column_low(screens, colormaps, render_main.centery);
            }
            _ => {
                // For wall rendering, only DrawColumn and DrawColumnLow are used.
                // Fuzz/Translated variants are for sprites, not wall segments.
                draw.draw_column(screens, colormaps, render_main.centery);
            }
        }
    }
}

impl Default for SegsState {
    fn default() -> Self {
        Self::new()
    }
}
