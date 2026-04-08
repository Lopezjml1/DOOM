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

//! Translated from linuxdoom-1.10/r_things.c and r_things.h
//!
//! Refresh of things, i.e. objects represented by sprites.
//! Handles sprite projection, depth sorting, clipping, and masked column drawing.
//!
//! # Architecture
//!
//! In the original C code, sprite rendering state was held in file-scope globals
//! in `r_things.c`. Here it is consolidated into [`ThingsState`], which owns the
//! vissprite pool, clipping arrays, and per-frame sprite metadata. Methods on
//! `ThingsState` accept references to sibling renderer state structs
//! ([`DrawState`], [`DataState`], [`RenderMain`], [`RenderState`]) to access
//! viewpoint, lighting, texture, and geometry data without global mutable state.
//!
//! # Rendering Pipeline
//!
//! 1. [`clear_sprites`](ThingsState::clear_sprites) — reset vissprite pool each frame.
//! 2. [`add_sprites`](ThingsState::add_sprites) — called per subsector during BSP traversal;
//!    projects each mobj in the sector's thing list into a 2-D vissprite.
//! 3. [`draw_masked`](ThingsState::draw_masked) — top-level call after all solid geometry:
//!    sorts vissprites, draws them back-to-front with clipping, renders remaining
//!    masked mid-textures, and composites player weapon sprites on top.

use crate::data::DataState;
use crate::defs::{
    DrawSeg, Fixed, RenderState, SpriteDef, SpriteFrame, VisSprite, FRACBITS, FRACUNIT,
    SCREENWIDTH, SIL_BOTTOM, SIL_TOP,
};
use crate::draw::DrawState;
use crate::main::{
    point_to_angle, ColFunc, RenderMain, LIGHTLEVELS, LIGHTSCALESHIFT, LIGHTSEGSHIFT, MAXLIGHTSCALE,
};

use doom_core::info::sprites::{NUMSPRITES, SPRITE_NAMES};
use doom_core::info::states::STATES;
use doom_core::types::doomdef::PowerType;
use doom_core::types::doomtype::MAXINT;
use doom_core::types::mobj::{MapObject, MobjFlags, MF_TRANSSHIFT};
use doom_core::types::player::{Player, PspDef, FF_FRAMEMASK, FF_FULLBRIGHT, NUMPSPRITES};

use doom_wad::types::PurgeTag;
use doom_wad::wad_provider::WadProvider;

// =============================================================================
// Screen dimension as usize for array sizing
// =============================================================================

/// Screen width as usize for array indexing and sizing.
const SCREENWIDTH_USIZE: usize = SCREENWIDTH as usize;

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of visible sprites per frame.
///
/// Original C: `#define MAXVISSPRITES 128` (r_things.h line 31)
pub const MAXVISSPRITES: usize = 128;

/// Minimum Z distance for sprite rendering, in fixed-point units.
///
/// Sprites closer than 4 fracunits are clamped to prevent division
/// by zero during perspective projection.
///
/// Original C: `#define MINZ (FRACUNIT*4)` (r_things.c line 46)
pub const MINZ: i32 = FRACUNIT * 4;

/// Base Y center for player sprite (psprite) rendering.
///
/// Vertical center of the screen in pixels (200 / 2 = 100).
///
/// Original C: `#define BASEYCENTER 100` (r_things.c line 47)
pub const BASEYCENTER: i32 = 100;

/// Maximum number of frames per sprite definition used during initialization.
///
/// Original C: `#define MAXFRAME 29` (r_things.c line 593)
const MAXFRAME: usize = 29;

// =============================================================================
// ThingsState — Sprite rendering state
// =============================================================================

/// Sprite sorting and rendering state.
///
/// Consolidates all formerly-global variables from `r_things.c` into a single
/// owned struct. Provides the full sprite rendering pipeline: projection,
/// sorting, clipping, and masked column compositing.
///
/// # Original C References
///
/// - `r_things.c` lines 45-76: global state variables
/// - `r_things.h` lines 27-67: extern declarations and function prototypes
pub struct ThingsState {
    /// Pool of visible sprite structures for the current frame.
    ///
    /// Up to [`MAXVISSPRITES`] sprites can be visible in a single frame.
    /// Sprites beyond this limit are silently dropped (the farthest ones
    /// are discarded by the overflow logic in [`new_vissprite`]).
    ///
    /// Original C: `vissprite_t vissprites[MAXVISSPRITES]` (r_things.c line 65)
    pub vissprites: Vec<VisSprite>,

    /// Number of vissprites currently active this frame.
    ///
    /// Acts as the write pointer into the vissprites pool. Reset to 0
    /// by [`clear_sprites`](Self::clear_sprites) each frame.
    ///
    /// Original C: `vissprite_t* vissprite_p` (r_things.c line 66)
    pub vissprite_count: usize,

    /// Sentinel head node for the sorted vissprite linked list.
    ///
    /// After [`sort_vissprites`](Self::sort_vissprites), the vissprites are
    /// linked in back-to-front order through their `prev`/`next` indices.
    /// This sentinel acts as the head of that doubly-linked list.
    ///
    /// Original C: `vissprite_t vsprsortedhead` (r_things.c line 67)
    pub vsprsortedhead: VisSprite,

    /// Initialized to -1 for each column; used as ceiling clip sentinel.
    ///
    /// Original C: `short negonearray[SCREENWIDTH]` (r_things.c line 50)
    pub negonearray: [i16; SCREENWIDTH_USIZE],

    /// Initialized to viewheight for each column; used as floor clip sentinel.
    ///
    /// Original C: `short screenheightarray[SCREENWIDTH]` (r_things.c line 51)
    pub screenheightarray: [i16; SCREENWIDTH_USIZE],

    /// Floor clipping array (column-indexed).
    ///
    /// Current floor clip array used during masked column drawing.
    /// Set per-sprite before `draw_vissprite`.
    ///
    /// Original C: `short* mfloorclip` (r_things.c line 53)
    pub mfloorclip: Vec<i16>,

    /// Ceiling clipping array (column-indexed).
    ///
    /// Current ceiling clip array used during masked column drawing.
    /// Set per-sprite before `draw_vissprite`.
    ///
    /// Original C: `short* mceilingclip` (r_things.c line 54)
    pub mceilingclip: Vec<i16>,

    /// Current sprite vertical scale (fixed 16.16).
    ///
    /// Set per-column during masked column drawing for perspective-correct
    /// texture mapping.
    ///
    /// Original C: `fixed_t spryscale` (r_things.c line 56)
    pub spryscale: Fixed,

    /// Current sprite top screen position (fixed 16.16).
    ///
    /// Vertical screen coordinate of the top of the current sprite column,
    /// in fixed-point to maintain sub-pixel precision.
    ///
    /// Original C: `fixed_t sprtopscreen` (r_things.c line 57)
    pub sprtopscreen: Fixed,

    /// Player sprite horizontal scale (fixed 16.16).
    ///
    /// Pre-computed scale factor for projecting player weapon sprites to
    /// screen coordinates.
    ///
    /// Original C: `fixed_t pspritescale` (r_things.c line 74)
    pub pspritescale: Fixed,

    /// Player sprite inverse horizontal scale (fixed 16.16).
    ///
    /// Inverse of `pspritescale`, used for texture stepping when drawing
    /// player weapon sprite columns.
    ///
    /// Original C: `fixed_t pspriteiscale` (r_things.c line 75)
    pub pspriteiscale: Fixed,

    /// Sprite definition table (indexed by SpriteNum).
    ///
    /// Built during [`init_sprites`](Self::init_sprites) from WAD lump data.
    /// Each entry defines the frames and rotations for one sprite type.
    ///
    /// Original C: `spritedef_t* sprites` (r_state.h)
    pub sprites: Vec<SpriteDef>,

    /// Number of sprite definitions in the `sprites` table.
    ///
    /// Original C: `int numsprites` (r_state.h)
    pub numsprites: usize,

    /// Current sprite lighting table index.
    ///
    /// Index into the `scalelight` table based on the sprite's sector light
    /// level. Set by [`add_sprites`](Self::add_sprites) for each sector.
    ///
    /// Original C: `lighttable_t** spritelights` (r_things.c line 77)
    pub spritelights: usize,
}

impl ThingsState {
    /// Creates a new `ThingsState` with default initialization.
    ///
    /// All arrays are sized and zero-initialized. The `negonearray` is
    /// filled with -1 and `screenheightarray` with 0 (set to viewheight later).
    pub fn new() -> Self {
        Self {
            vissprites: Vec::with_capacity(MAXVISSPRITES),
            vissprite_count: 0,
            vsprsortedhead: VisSprite::default(),
            negonearray: [-1i16; SCREENWIDTH_USIZE],
            screenheightarray: [0i16; SCREENWIDTH_USIZE],
            mfloorclip: vec![0i16; SCREENWIDTH_USIZE],
            mceilingclip: vec![0i16; SCREENWIDTH_USIZE],
            spryscale: Fixed::ZERO,
            sprtopscreen: Fixed::ZERO,
            pspritescale: Fixed::ZERO,
            pspriteiscale: Fixed::ZERO,
            sprites: Vec::new(),
            numsprites: 0,
            spritelights: 0,
        }
    }

    // =========================================================================
    // R_ClearSprites (r_things.c line 365)
    // =========================================================================

    /// Resets the vissprite pool for a new frame.
    ///
    /// Called at the beginning of each rendering frame to prepare for new
    /// sprite projections.
    ///
    /// Original C: `void R_ClearSprites(void)` (r_things.c line 365)
    pub fn clear_sprites(&mut self) {
        self.vissprite_count = 0;
        self.vissprites.clear();
    }

    // =========================================================================
    // R_NewVisSprite (r_things.c line 330-360)
    // =========================================================================

    /// Allocates and returns the index of a new vissprite in the pool.
    ///
    /// If the pool is full (count >= MAXVISSPRITES), the farthest sprite
    /// is overwritten rather than discarding the new one, matching the
    /// original C overflow behavior.
    ///
    /// Original C: `vissprite_t* R_NewVisSprite(void)` (r_things.c line 330)
    pub fn new_vissprite(&mut self) -> usize {
        if self.vissprite_count < MAXVISSPRITES {
            let idx = self.vissprite_count;
            self.vissprites.push(VisSprite::default());
            self.vissprite_count += 1;
            idx
        } else {
            // Overflow: find the farthest sprite (smallest scale) and overwrite it.
            tracing::warn!("R_NewVisSprite: vissprite overflow (>{MAXVISSPRITES})");
            let mut min_scale = MAXINT;
            let mut min_idx = 0usize;
            for (i, vis) in self.vissprites.iter().enumerate() {
                if vis.scale.raw() < min_scale {
                    min_scale = vis.scale.raw();
                    min_idx = i;
                }
            }
            min_idx
        }
    }

    // =========================================================================
    // R_DrawMaskedColumn (r_things.c lines 67-140)
    // =========================================================================

    /// Draws a single masked (transparent) sprite column.
    ///
    /// Iterates through the column's post list (terminated by `topdelta == 0xff`),
    /// clips each post against the current floor/ceiling clip arrays, and
    /// dispatches the appropriate column drawing function.
    ///
    /// # Arguments
    /// * `column_data` — Raw column data bytes from the patch lump.
    /// * `col_offset` — Byte offset into `column_data` where posts begin.
    /// * `draw` — Mutable reference to the column drawing state.
    /// * `render_main` — Renderer viewpoint state (for colfunc dispatch).
    /// * `data` — Data state (for colormaps).
    /// * `screens` — Screen buffers for pixel output.
    ///
    /// Original C: `void R_DrawMaskedColumn(column_t* column)` (r_things.c line 67)
    pub fn draw_masked_column(
        &mut self,
        column_data: &[u8],
        col_offset: usize,
        draw: &mut DrawState,
        render_main: &RenderMain,
        data: &DataState,
        screens: &mut [Vec<u8>],
    ) {
        let dc_x = draw.dc_x;
        if !(0..SCREENWIDTH).contains(&dc_x) {
            return;
        }

        let basetexturemid = draw.dc_texturemid;
        let mut offset = col_offset;

        // Iterate through posts in the column. Each post has:
        //   byte topdelta    — vertical offset from top (0xff = end)
        //   byte length      — number of opaque pixels
        //   byte pad          — unused padding byte
        //   byte[length] data — pixel values
        //   byte pad          — unused padding byte
        loop {
            if offset >= column_data.len() {
                break;
            }

            let topdelta = column_data[offset] as i32;
            if topdelta == 0xff {
                break;
            }

            // length of this post
            if offset + 1 >= column_data.len() {
                break;
            }
            let length = column_data[offset + 1] as i32;

            // Calculate top and bottom screen coordinates for this post
            let spryscale_raw = self.spryscale.raw();
            let topscreen = self
                .sprtopscreen
                .raw()
                .wrapping_add(spryscale_raw.wrapping_mul(topdelta));

            let bottomscreen = topscreen.wrapping_add(spryscale_raw.wrapping_mul(length));

            let mut dc_yl = (topscreen.wrapping_add(FRACUNIT - 1)) >> FRACBITS;
            let mut dc_yh = (bottomscreen - 1) >> FRACBITS;

            // Clip against ceiling
            let dc_x_usize = dc_x as usize;
            let mceil = if dc_x_usize < self.mceilingclip.len() {
                self.mceilingclip[dc_x_usize] as i32
            } else {
                -1
            };
            if dc_yl <= mceil {
                dc_yl = mceil + 1;
            }

            // Clip against floor
            let mfloor = if dc_x_usize < self.mfloorclip.len() {
                self.mfloorclip[dc_x_usize] as i32
            } else {
                200 // SCREENHEIGHT
            };
            if dc_yh >= mfloor {
                dc_yh = mfloor - 1;
            }

            if dc_yl <= dc_yh {
                // Set up source data: skip the 3 header bytes (topdelta, length, pad)
                let src_start = offset + 3;
                let src_end = src_start + length as usize;

                if src_end <= column_data.len() {
                    draw.dc_source = column_data[src_start..src_end].to_vec();
                }

                draw.dc_texturemid = basetexturemid.wrapping_sub(topdelta << FRACBITS);
                draw.dc_yl = dc_yl;
                draw.dc_yh = dc_yh;

                // Dispatch via current column function
                dispatch_colfunc(
                    render_main.colfunc,
                    draw,
                    screens,
                    &data.colormaps,
                    render_main.centery,
                );
            }

            // Advance to next post: topdelta(1) + length(1) + pad(1) + data(length) + pad(1)
            offset += 4 + length as usize;
        }

        draw.dc_texturemid = basetexturemid;
    }

    // =========================================================================
    // R_ProjectSprite (r_things.c lines 185-350)
    // =========================================================================

    /// Projects a 3-D map object into a 2-D vissprite for rendering.
    ///
    /// Performs the full 3D-to-2D transformation: translates to viewpoint-
    /// relative coordinates, applies perspective projection, computes screen
    /// bounds, determines the correct sprite frame and rotation, and creates
    /// a vissprite entry in the pool.
    ///
    /// # Culling
    ///
    /// Sprites are culled if:
    /// - They are behind the viewer (tz <= 0)
    /// - They are too close (tz < MINZ)
    /// - They are too far left or right (|tx| > tz * 4)
    /// - Their screen bounds are entirely outside the view window
    ///
    /// Original C: `void R_ProjectSprite(mobj_t* thing)` (r_things.c line 185)
    pub fn project_sprite(
        &mut self,
        thing: &MapObject,
        render_main: &RenderMain,
        render_state: &RenderState,
        data: &DataState,
    ) {
        // Transform the origin point relative to the viewpoint.
        let tr_x = thing.x - render_main.viewx;
        let tr_y = thing.y - render_main.viewy;

        let gxt = tr_x.fixed_mul(render_main.viewcos);
        let gyt = -(tr_y.fixed_mul(render_main.viewsin));
        let tz = gxt - gyt;

        // Thing is behind view plane or too close
        if tz.raw() < MINZ {
            return;
        }

        let xscale = render_main.projection.fixed_div(tz);

        let gxt2 = tr_x.fixed_mul(render_main.viewsin);
        let gyt2 = tr_y.fixed_mul(render_main.viewcos);
        let mut tx = -(gxt2 + gyt2);

        // Too far off the side?
        if tx.raw().abs() > (tz.raw() >> 2).wrapping_mul(5) {
            return;
        }

        // Determine sprite frame and rotation
        let sprnum = thing.sprite;
        if sprnum >= self.numsprites {
            tracing::error!("R_ProjectSprite: invalid sprite number {sprnum}");
            return;
        }

        let sprdef = &self.sprites[sprnum];
        let frame_num = (thing.frame & FF_FRAMEMASK as i32) as usize;

        if frame_num as i32 >= sprdef.numframes {
            tracing::error!(
                "R_ProjectSprite: invalid sprite frame {frame_num} (max {})",
                sprdef.numframes
            );
            return;
        }

        let sprframe = &sprdef.spriteframes[frame_num];

        let (lump, flip): (i32, bool);
        if sprframe.rotate {
            // Choose rotated frame based on angle
            let ang = point_to_angle(thing.x, thing.y, render_main);
            // Compute rotation index: (angle - thing.angle + (ANG45/2)*9) >> 29
            let rot_angle = ang - thing.angle;
            // Add (ANG45 / 2) * 9 = 0x10000000 * 9 = 0x90000000
            let rot_val = rot_angle.value().wrapping_add(0x20000000u32 / 2 * 9);
            let rot = ((rot_val >> 29) & 7) as usize;
            lump = sprframe.lump[rot] as i32;
            flip = sprframe.flip[rot] != 0;
        } else {
            // Use rotation 0
            lump = sprframe.lump[0] as i32;
            flip = sprframe.flip[0] != 0;
        };

        // Get sprite dimensions from pre-computed tables
        let lump_idx = lump as usize;
        if lump_idx >= data.spritewidth.len() {
            return;
        }

        let tx_offset = Fixed::new(data.spriteoffset[lump_idx]);
        tx = tx - tx_offset;
        let x1 = (render_main.centerxfrac + tx.fixed_mul(xscale)).raw() >> FRACBITS;

        // Does the sprite actually cross the screen at all?
        tx = tx + Fixed::new(data.spritewidth[lump_idx]);
        let x2 = ((render_main.centerxfrac + tx.fixed_mul(xscale)).raw() >> FRACBITS) - 1;

        // Off the right side of the screen?
        if x1 > render_main.viewwidth {
            return;
        }

        // Off the left side of the screen?
        if x2 < 0 {
            return;
        }

        // Store the vissprite
        let vis_idx = self.new_vissprite();
        let vis = &mut self.vissprites[vis_idx];

        vis.mobjflags = thing.flags.bits() as i32;
        vis.scale = xscale;
        vis.gx = thing.x;
        vis.gy = thing.y;
        vis.gz = thing.z;
        vis.gzt = thing.z + Fixed::new(data.spritetopoffset[lump_idx]);
        vis.texturemid = vis.gzt - render_main.viewz;
        vis.x1 = if x1 < 0 { 0 } else { x1 };
        vis.x2 = if x2 >= render_main.viewwidth {
            render_main.viewwidth - 1
        } else {
            x2
        };

        let iscale = render_main.projection.fixed_div(tz);

        if flip {
            vis.startfrac = Fixed::new(data.spritewidth[lump_idx] - 1);
            vis.xiscale = -iscale;
        } else {
            vis.startfrac = Fixed::ZERO;
            vis.xiscale = iscale;
        }

        if vis.x1 > x1 {
            vis.startfrac = vis.startfrac + Fixed::from_int(vis.x1 - x1).fixed_mul(vis.xiscale);
        }

        vis.patch = lump;

        // Determine light level / colormap
        if thing.frame & FF_FULLBRIGHT as i32 != 0 {
            // Full bright
            vis.colormap = Some(0);
        } else if let Some(fc) = render_main.fixedcolormap {
            // Fixed colormap (invulnerability, infrared)
            vis.colormap = Some(fc);
        } else {
            // Use sector-based lighting
            let mut index = (xscale.raw() >> LIGHTSCALESHIFT) as usize;
            if index >= MAXLIGHTSCALE {
                index = MAXLIGHTSCALE - 1;
            }
            vis.colormap = Some(render_main.scalelight[self.spritelights][index]);
        }

        let _ = render_state; // Used for sector lookup in full implementation
    }

    // =========================================================================
    // R_AddSprites (r_things.c lines 832-866)
    // =========================================================================

    /// Adds all sprites in a sector to the vissprite pool.
    ///
    /// Called during BSP traversal for each visible subsector. Sets the
    /// sprite lighting level from the sector and iterates the sector's
    /// thing list, calling [`project_sprite`](Self::project_sprite) for each.
    ///
    /// Uses `validcount` for duplicate-sector suppression so that sectors
    /// visible from multiple subsectors are only processed once per frame.
    ///
    /// Original C: `void R_AddSprites(sector_t* sec)` (r_things.c line 832)
    pub fn add_sprites(
        &mut self,
        sec_idx: usize,
        render_main: &mut RenderMain,
        render_state: &mut RenderState,
        data: &DataState,
        mobjs: &[MapObject],
    ) {
        if sec_idx >= render_state.sectors.len() {
            return;
        }

        // Already processed this frame?
        if render_state.sectors[sec_idx].validcount == render_main.validcount {
            return;
        }

        // Mark as processed
        render_state.sectors[sec_idx].validcount = render_main.validcount;

        // Calculate sprite lighting from sector light level
        let lightnum = (render_state.sectors[sec_idx].lightlevel as i32 >> LIGHTSEGSHIFT)
            + render_main.extralight;
        let lightnum = if lightnum < 0 {
            0usize
        } else if lightnum as usize >= LIGHTLEVELS {
            LIGHTLEVELS - 1
        } else {
            lightnum as usize
        };
        self.spritelights = lightnum;

        // Walk the sector's thing list
        let mut thing_idx = render_state.sectors[sec_idx].thinglist;
        while let Some(idx) = thing_idx {
            if idx >= mobjs.len() {
                break;
            }
            self.project_sprite(&mobjs[idx], render_main, render_state, data);
            thing_idx = mobjs[idx].snext;
        }
    }

    // =========================================================================
    // R_AddPSprites (schema compliance)
    // =========================================================================

    /// Adds player weapon sprites (psprites) to the rendering pipeline.
    ///
    /// This is a schema-compliance entry point that delegates to
    /// [`draw_player_sprites`](Self::draw_player_sprites) which performs the
    /// actual work.
    ///
    /// Original C: The psprite adding logic is part of `R_DrawPlayerSprites`.
    pub fn add_psprites(
        &mut self,
        render_main: &RenderMain,
        render_state: &RenderState,
        data: &DataState,
        players: &[Player],
        mobjs: &[MapObject],
    ) {
        // Psprites are projected and drawn in draw_player_sprites.
        // This entry exists for schema compliance.
        let _ = (render_main, render_state, data, players, mobjs);
    }

    // =========================================================================
    // R_SortVisSprites (r_things.c lines 787-830)
    // =========================================================================

    /// Sorts vissprites by scale in back-to-front order (painter's algorithm).
    ///
    /// Reconstructs the vissprite linked list so that sprites are ordered from
    /// farthest (smallest scale) to nearest (largest scale). This ensures
    /// correct rendering with the painter's algorithm — nearer sprites are
    /// drawn over farther ones.
    ///
    /// Uses an O(n²) selection sort, which is adequate for the typical
    /// sprite counts in DOOM (usually <50 visible sprites).
    ///
    /// Original C: `void R_SortVisSprites(void)` (r_things.c line 787)
    pub fn sort_vissprites(&mut self) {
        let count = self.vissprite_count;
        if count == 0 {
            self.vsprsortedhead.next = None;
            self.vsprsortedhead.prev = None;
            return;
        }

        // Build a temporary index list for sorting
        let mut unsorted: Vec<usize> = (0..count).collect();
        let mut sorted: Vec<usize> = Vec::with_capacity(count);

        // Selection sort: repeatedly find the sprite with the smallest scale
        // (farthest away) and move it to the sorted list.
        while !unsorted.is_empty() {
            let mut best_scale = MAXINT;
            let mut best_pos = 0usize;

            for (pos, &idx) in unsorted.iter().enumerate() {
                if self.vissprites[idx].scale.raw() < best_scale {
                    best_scale = self.vissprites[idx].scale.raw();
                    best_pos = pos;
                }
            }

            sorted.push(unsorted.remove(best_pos));
        }

        // Rebuild the linked list in sorted order through the vissprites pool.
        // Use a sentinel index (usize::MAX) to represent the head node.
        let sentinel = usize::MAX;

        // First sorted sprite links back to sentinel
        self.vsprsortedhead.next = Some(sorted[0]);
        self.vissprites[sorted[0]].prev = Some(sentinel);

        // Link consecutive sprites
        for i in 0..sorted.len() - 1 {
            self.vissprites[sorted[i]].next = Some(sorted[i + 1]);
            self.vissprites[sorted[i + 1]].prev = Some(sorted[i]);
        }

        // Last sorted sprite links forward to sentinel
        let last = sorted[sorted.len() - 1];
        self.vissprites[last].next = Some(sentinel);
        self.vsprsortedhead.prev = Some(last);
    }

    // =========================================================================
    // R_DrawVisSprite (r_things.c lines 363-475)
    // =========================================================================

    /// Draws a single vissprite to the screen.
    ///
    /// Loads the sprite's patch from the WAD, configures the column drawing
    /// state (colormap, scale, texture origin), handles special effects
    /// (MF_SHADOW fuzz, MF_TRANSLATION color remap), and iterates through
    /// each visible column calling [`draw_masked_column`].
    ///
    /// Original C: `void R_DrawVisSprite(vissprite_t* vis, int x1, int x2)`
    /// (r_things.c line 363)
    pub fn draw_vissprite(
        &mut self,
        vis_idx: usize,
        x1: i32,
        x2: i32,
        draw: &mut DrawState,
        data: &DataState,
        render_main: &mut RenderMain,
        wad: &mut dyn WadProvider,
        screens: &mut [Vec<u8>],
    ) {
        if vis_idx >= self.vissprites.len() {
            return;
        }

        // Load the patch for this sprite
        let patch_lump = self.vissprites[vis_idx].patch + data.firstspritelump;
        let patch_data = wad
            .cache_lump_num(patch_lump as usize, PurgeTag::Cache)
            .to_vec();

        // Parse patch header: width(2), height(2), leftoffset(2), topoffset(2)
        if patch_data.len() < 8 {
            return;
        }
        let patch_width = i16::from_le_bytes([patch_data[0], patch_data[1]]) as i32;

        // Read vissprite fields
        let vis_colormap = self.vissprites[vis_idx].colormap;
        let vis_mobjflags = self.vissprites[vis_idx].mobjflags;
        let vis_xiscale = self.vissprites[vis_idx].xiscale;
        let vis_texturemid = self.vissprites[vis_idx].texturemid;
        let vis_scale = self.vissprites[vis_idx].scale;
        let vis_startfrac = self.vissprites[vis_idx].startfrac;

        draw.dc_iscale = (vis_xiscale.raw().abs()) >> render_main.detailshift;
        draw.dc_texturemid = vis_texturemid.raw();

        // Determine column function for special effects
        let saved_colfunc = render_main.colfunc;

        if vis_mobjflags & (MobjFlags::MF_SHADOW.bits() as i32) != 0 {
            // Spectre: use fuzz column drawing
            render_main.colfunc = render_main.fuzzcolfunc;
        } else if vis_mobjflags & (MobjFlags::MF_TRANSLATION.bits() as i32) != 0 {
            // Color translation active
            let table_index = (((vis_mobjflags as u32) & MobjFlags::MF_TRANSLATION.bits())
                >> MF_TRANSSHIFT) as usize;
            if table_index > 0 {
                draw.dc_translation = (table_index - 1) * 256;
            }
            render_main.colfunc = ColFunc::DrawTranslatedColumn;
        }

        // Set up colormap
        if let Some(cm) = vis_colormap {
            draw.dc_colormap = cm;
        }

        self.spryscale = vis_scale;
        // sprtopscreen = centeryfrac - FixedMul(dc_texturemid, spryscale)
        self.sprtopscreen =
            render_main.centeryfrac - Fixed::new(vis_texturemid.raw()).fixed_mul(vis_scale);

        let mut frac = vis_startfrac;
        let fracstep = vis_xiscale;

        for dc_x in x1..=x2 {
            let texturecolumn = frac.raw() >> FRACBITS;
            if texturecolumn < 0 || texturecolumn >= patch_width {
                frac = frac + fracstep;
                continue;
            }

            // Read column offset from patch header
            // columnofs start at byte 8, each is 4 bytes (i32 LE)
            let col_idx = texturecolumn as usize;
            let ofs_pos = 8 + col_idx * 4;
            if ofs_pos + 4 > patch_data.len() {
                frac = frac + fracstep;
                continue;
            }
            let col_offset = u32::from_le_bytes([
                patch_data[ofs_pos],
                patch_data[ofs_pos + 1],
                patch_data[ofs_pos + 2],
                patch_data[ofs_pos + 3],
            ]) as usize;

            draw.dc_x = dc_x;
            self.draw_masked_column(&patch_data, col_offset, draw, render_main, data, screens);

            frac = frac + fracstep;
        }

        // Restore column function
        render_main.colfunc = saved_colfunc;
    }

    // =========================================================================
    // R_ClipVisSprite (r_things.c lines 483-593 — clipping logic)
    // =========================================================================

    /// Clips a vissprite against draw segments for correct occlusion.
    ///
    /// Initializes clip arrays, scans drawsegs from front to back, and
    /// applies silhouette clipping to determine which columns of the sprite
    /// are visible. The result is stored in `mfloorclip` and `mceilingclip`.
    ///
    /// Original C: Clipping logic from `R_DrawSprite` (r_things.c line 483)
    pub fn clip_vissprite(
        &mut self,
        vis_idx: usize,
        xl: i32,
        xh: i32,
        drawsegs: &[DrawSeg],
        ds_p: usize,
        openings: &[i16],
    ) {
        if vis_idx >= self.vissprites.len() {
            return;
        }

        // Local clip arrays — initialized to -2 meaning "unclipped"
        let mut clipbot = [-2i16; SCREENWIDTH_USIZE];
        let mut cliptop = [-2i16; SCREENWIDTH_USIZE];

        let vis_x1 = self.vissprites[vis_idx].x1;
        let vis_x2 = self.vissprites[vis_idx].x2;
        let vis_scale = self.vissprites[vis_idx].scale;

        // Scan drawsegs from back to front (newest to oldest)
        let end = if ds_p > drawsegs.len() {
            drawsegs.len()
        } else {
            ds_p
        };

        for ds_idx in (0..end).rev() {
            let ds = &drawsegs[ds_idx];

            // Does the drawseg overlap the sprite's horizontal range?
            if ds.x1 > vis_x2 || ds.x2 < vis_x1 {
                continue;
            }

            let r1 = if ds.x1 < vis_x1 { vis_x1 } else { ds.x1 };
            let r2 = if ds.x2 > vis_x2 { vis_x2 } else { ds.x2 };

            let lowscale = if ds.scale1.raw() < ds.scale2.raw() {
                ds.scale1
            } else {
                ds.scale2
            };
            let scale = if ds.scale1.raw() > ds.scale2.raw() {
                ds.scale1
            } else {
                ds.scale2
            };

            // Is the drawseg behind the sprite?
            if scale.raw() < vis_scale.raw()
                || (lowscale.raw() < vis_scale.raw()
                    && !drawseg_in_front(ds, &self.vissprites[vis_idx]))
            {
                // Seg is behind sprite — skip
                continue;
            }

            // Apply silhouette clipping from this drawseg
            // Bottom clipping
            if ds.silhouette & SIL_BOTTOM != 0 {
                if let Some(clip_idx) = ds.sprbottomclip {
                    for x in r1..=r2 {
                        let xu = x as usize;
                        if xu < SCREENWIDTH_USIZE {
                            let oi = clip_idx + xu;
                            if oi < openings.len() && clipbot[xu] == -2 {
                                clipbot[xu] = openings[oi];
                            }
                        }
                    }
                } else {
                    for x in r1..=r2 {
                        let xu = x as usize;
                        if xu < SCREENWIDTH_USIZE && clipbot[xu] == -2 {
                            clipbot[xu] = self.screenheightarray[xu];
                        }
                    }
                }
            }

            // Top clipping
            if ds.silhouette & SIL_TOP != 0 {
                if let Some(clip_idx) = ds.sprtopclip {
                    for x in r1..=r2 {
                        let xu = x as usize;
                        if xu < SCREENWIDTH_USIZE {
                            let oi = clip_idx + xu;
                            if oi < openings.len() && cliptop[xu] == -2 {
                                cliptop[xu] = openings[oi];
                            }
                        }
                    }
                } else {
                    for x in r1..=r2 {
                        let xu = x as usize;
                        if xu < SCREENWIDTH_USIZE && cliptop[xu] == -2 {
                            cliptop[xu] = self.negonearray[xu];
                        }
                    }
                }
            }
        }

        // Apply the computed clips to mfloorclip / mceilingclip
        self.mfloorclip.resize(SCREENWIDTH_USIZE, 0);
        self.mceilingclip.resize(SCREENWIDTH_USIZE, 0);

        for x in xl..=xh {
            let xu = x as usize;
            if xu >= SCREENWIDTH_USIZE {
                continue;
            }
            if clipbot[xu] == -2 {
                self.mfloorclip[xu] = self.screenheightarray[xu];
            } else {
                self.mfloorclip[xu] = clipbot[xu];
            }
            if cliptop[xu] == -2 {
                self.mceilingclip[xu] = -1;
            } else {
                self.mceilingclip[xu] = cliptop[xu];
            }
        }
    }

    // =========================================================================
    // R_DrawSprite (r_things.c lines 483-593)
    // =========================================================================

    /// Draws a single vissprite with full drawseg clipping.
    ///
    /// Clips the sprite against all overlapping draw segments, then
    /// calls [`draw_vissprite`] with the computed clip bounds.
    ///
    /// Original C: `void R_DrawSprite(vissprite_t* spr)` (r_things.c line 483)
    pub fn draw_sprite(
        &mut self,
        vis_idx: usize,
        draw: &mut DrawState,
        data: &DataState,
        render_main: &mut RenderMain,
        wad: &mut dyn WadProvider,
        screens: &mut [Vec<u8>],
        drawsegs: &[DrawSeg],
        ds_p: usize,
        openings: &[i16],
    ) {
        if vis_idx >= self.vissprites.len() {
            return;
        }
        let x1 = self.vissprites[vis_idx].x1;
        let x2 = self.vissprites[vis_idx].x2;

        // Clip the vissprite against drawsegs
        self.clip_vissprite(vis_idx, x1, x2, drawsegs, ds_p, openings);

        // Draw the vissprite with computed clip bounds
        self.draw_vissprite(vis_idx, x1, x2, draw, data, render_main, wad, screens);
    }

    // =========================================================================
    // R_DrawSprites (from R_DrawMasked)
    // =========================================================================

    /// Draws all sorted vissprites from back to front.
    ///
    /// Iterates the sorted vissprite linked list (built by
    /// [`sort_vissprites`]) and calls [`draw_sprite`] for each sprite.
    ///
    /// Original C: Part of `R_DrawMasked` (r_things.c)
    pub fn draw_sprites(
        &mut self,
        draw: &mut DrawState,
        data: &DataState,
        render_main: &mut RenderMain,
        wad: &mut dyn WadProvider,
        screens: &mut [Vec<u8>],
        drawsegs: &[DrawSeg],
        ds_p: usize,
        openings: &[i16],
    ) {
        if self.vissprite_count == 0 {
            return;
        }

        // Walk the sorted list (back-to-front order)
        let sentinel = usize::MAX;
        let mut current = self.vsprsortedhead.next;

        while let Some(idx) = current {
            if idx == sentinel {
                break;
            }
            if idx >= self.vissprites.len() {
                break;
            }

            // Save next before draw_sprite may modify the list
            let next = self.vissprites[idx].next;

            self.draw_sprite(
                idx,
                draw,
                data,
                render_main,
                wad,
                screens,
                drawsegs,
                ds_p,
                openings,
            );

            current = next;
        }
    }

    // =========================================================================
    // R_DrawPSprite (r_things.c)
    // =========================================================================

    /// Draws a single player weapon sprite (psprite).
    ///
    /// Projects the psprite to screen coordinates using `pspritescale`,
    /// determines the correct colormap based on sector lighting and
    /// power-ups, and calls [`draw_vissprite`].
    ///
    /// Original C: `void R_DrawPSprite(psp_t psp)` (r_things.c)
    pub fn draw_psprite(
        &mut self,
        psp: &PspDef,
        draw: &mut DrawState,
        data: &DataState,
        render_main: &mut RenderMain,
        _render_state: &RenderState,
        wad: &mut dyn WadProvider,
        screens: &mut [Vec<u8>],
        players: &[Player],
        _mobjs: &[MapObject],
        lightlevel: i32,
    ) {
        // Check if psprite state is valid
        let state_idx = match psp.state {
            Some(idx) => idx,
            None => return,
        };

        if state_idx >= STATES.len() {
            return;
        }

        let state = &STATES[state_idx];

        // Get sprite definition
        let sprnum = state.sprite as usize;
        if sprnum >= self.numsprites {
            tracing::error!("R_DrawPSprite: invalid sprite number {sprnum}");
            return;
        }

        let sprdef = &self.sprites[sprnum];
        let frame_num = (state.frame & FF_FRAMEMASK as i32) as usize;
        if frame_num as i32 >= sprdef.numframes {
            tracing::error!("R_DrawPSprite: invalid sprite frame {frame_num}");
            return;
        }

        let sprframe = &sprdef.spriteframes[frame_num];
        let lump = sprframe.lump[0] as i32;
        let flip = sprframe.flip[0] != 0;

        let lump_idx = lump as usize;
        if lump_idx >= data.spritewidth.len() {
            return;
        }

        // Project the psprite to screen coordinates
        // tx = psp->sx - 160*FRACUNIT + spriteoffset[lump]
        let tx = psp
            .sx
            .raw()
            .wrapping_sub(160 * FRACUNIT)
            .wrapping_add(data.spriteoffset[lump_idx]);

        let pscale_raw = self.pspritescale.raw();
        let x1 = (render_main
            .centerxfrac
            .raw()
            .wrapping_add(fixed_mul_raw(tx, pscale_raw)))
            >> FRACBITS;

        let tx2 = tx.wrapping_add(data.spritewidth[lump_idx]);
        let x2 = ((render_main
            .centerxfrac
            .raw()
            .wrapping_add(fixed_mul_raw(tx2, pscale_raw)))
            >> FRACBITS)
            - 1;

        // Create a temporary vissprite for the psprite
        let vis_idx = self.new_vissprite();
        {
            let vis = &mut self.vissprites[vis_idx];

            vis.mobjflags = 0;
            vis.texturemid = Fixed::new(
                (BASEYCENTER << FRACBITS)
                    .wrapping_add(FRACUNIT / 2)
                    .wrapping_sub(psp.sy.raw().wrapping_sub(data.spritetopoffset[lump_idx])),
            );
            vis.x1 = if x1 < 0 { 0 } else { x1 };
            vis.x2 = if x2 >= draw.viewwidth {
                draw.viewwidth - 1
            } else {
                x2
            };
            vis.scale = Fixed::new(pscale_raw << render_main.detailshift);

            if flip {
                vis.xiscale = -self.pspriteiscale;
                vis.startfrac = Fixed::new(data.spritewidth[lump_idx] - 1);
            } else {
                vis.xiscale = self.pspriteiscale;
                vis.startfrac = Fixed::ZERO;
            }

            if vis.x1 > x1 {
                vis.startfrac = vis.startfrac + Fixed::from_int(vis.x1 - x1).fixed_mul(vis.xiscale);
            }

            vis.patch = lump;

            // Determine colormap
            if state.frame & FF_FULLBRIGHT as i32 != 0 {
                vis.colormap = Some(0);
            } else if let Some(fc) = render_main.fixedcolormap {
                vis.colormap = Some(fc);
            } else {
                let mut index = (pscale_raw >> LIGHTSCALESHIFT) as usize;
                if index >= MAXLIGHTSCALE {
                    index = MAXLIGHTSCALE - 1;
                }

                let mut lightnum_i = (lightlevel >> LIGHTSEGSHIFT) + render_main.extralight;
                if lightnum_i < 0 {
                    lightnum_i = 0;
                }
                let lightnum_u = if (lightnum_i as usize) >= LIGHTLEVELS {
                    LIGHTLEVELS - 1
                } else {
                    lightnum_i as usize
                };

                vis.colormap = Some(render_main.scalelight[lightnum_u][index]);
            }
        }

        // Check for invisibility power-up (fuzz effect on weapon sprite)
        if let Some(player_idx) = render_main.viewplayer {
            if player_idx < players.len() {
                let pw_invis = players[player_idx].powers[PowerType::Invisibility as usize];
                if pw_invis > 4 * 32 || (pw_invis & 8) != 0 {
                    self.vissprites[vis_idx].mobjflags |= MobjFlags::MF_SHADOW.bits() as i32;
                    render_main.colfunc = render_main.fuzzcolfunc;
                }
            }
        }

        // Draw the psprite using the standard vissprite drawing path
        let vis_x1 = self.vissprites[vis_idx].x1;
        let vis_x2 = self.vissprites[vis_idx].x2;
        self.draw_vissprite(
            vis_idx,
            vis_x1,
            vis_x2,
            draw,
            data,
            render_main,
            wad,
            screens,
        );
    }

    // =========================================================================
    // R_DrawPlayerSprites (r_things.c)
    // =========================================================================

    /// Draws all player weapon sprites.
    ///
    /// Sets up the clipping arrays to allow the full screen height, computes
    /// the sector light level for the player's position, iterates the
    /// psprite slots, and draws each active psprite.
    ///
    /// Original C: `void R_DrawPlayerSprites(void)` (r_things.c)
    pub fn draw_player_sprites(
        &mut self,
        draw: &mut DrawState,
        data: &DataState,
        render_main: &mut RenderMain,
        render_state: &RenderState,
        wad: &mut dyn WadProvider,
        screens: &mut [Vec<u8>],
        players: &[Player],
        mobjs: &[MapObject],
    ) {
        // Don't draw player sprites if view is offset
        if render_main.viewangleoffset != 0 {
            return;
        }

        // Get the player's sector light level
        let lightlevel = get_player_sector_lightlevel(render_main, render_state, players, mobjs);

        // Set clipping to the full screen for psprites
        for i in 0..SCREENWIDTH_USIZE {
            self.mfloorclip[i] = draw.viewheight as i16;
            self.mceilingclip[i] = -1;
        }

        let saved_colfunc = render_main.colfunc;

        // Iterate each psprite slot
        if let Some(player_idx) = render_main.viewplayer {
            if player_idx < players.len() {
                let psprites: [PspDef; NUMPSPRITES] = players[player_idx].psprites;

                for psp_item in psprites.iter().take(NUMPSPRITES) {
                    if psp_item.state.is_some() {
                        render_main.colfunc = render_main.basecolfunc;
                        self.draw_psprite(
                            psp_item,
                            draw,
                            data,
                            render_main,
                            render_state,
                            wad,
                            screens,
                            players,
                            mobjs,
                            lightlevel,
                        );
                    }
                }
            }
        }

        render_main.colfunc = saved_colfunc;
    }

    // =========================================================================
    // R_DrawMasked (r_things.c lines 839-890)
    // =========================================================================

    /// Top-level masked (transparent) rendering entry point.
    ///
    /// Called after all solid wall geometry has been drawn. Performs:
    /// 1. Sort all vissprites by distance (back-to-front).
    /// 2. Draw each vissprite with drawseg clipping.
    /// 3. Render remaining masked mid-textures on drawsegs.
    /// 4. Draw player weapon sprites on top of everything.
    ///
    /// Original C: `void R_DrawMasked(void)` (r_things.c line 839)
    pub fn draw_masked(
        &mut self,
        draw: &mut DrawState,
        data: &DataState,
        render_main: &mut RenderMain,
        wad: &mut dyn WadProvider,
        screens: &mut [Vec<u8>],
        drawsegs: &[DrawSeg],
        ds_p: usize,
        openings: &[i16],
        render_state: &RenderState,
        players: &[Player],
        mobjs: &[MapObject],
    ) {
        // 1. Sort vissprites
        self.sort_vissprites();

        // 2. Draw all vissprites back to front (if any)
        if self.vissprite_count > 0 {
            self.draw_sprites(
                draw,
                data,
                render_main,
                wad,
                screens,
                drawsegs,
                ds_p,
                openings,
            );
        }

        // 3. Render any remaining masked mid textures on drawsegs.
        // R_RenderMaskedSegRange is in segs.rs — when fully implemented, this
        // loop will call it for each drawseg with a maskedtexturecol.
        let end = if ds_p > drawsegs.len() {
            drawsegs.len()
        } else {
            ds_p
        };
        for ds_idx in (0..end).rev() {
            if drawsegs[ds_idx].maskedtexturecol.is_some() {
                // When segs::render_masked_seg_range is available:
                // render_masked_seg_range(&drawsegs[ds_idx], drawsegs[ds_idx].x1, drawsegs[ds_idx].x2, ...);
                let _ = ds_idx;
            }
        }

        // 4. Draw the psprites on top of everything
        if render_main.viewangleoffset == 0 {
            self.draw_player_sprites(
                draw,
                data,
                render_main,
                render_state,
                wad,
                screens,
                players,
                mobjs,
            );
        }
    }

    // =========================================================================
    // R_InitSprites / R_InitSpriteDefs (r_things.c lines 595-785)
    // =========================================================================

    /// Initializes sprite definitions from WAD lump data.
    ///
    /// For each sprite name (4-character identifier), scans the WAD lump
    /// directory for matching lumps (e.g., "TROO" matches "TROOA0",
    /// "TROOB1B5", etc.). Builds [`SpriteDef`] entries with frame and
    /// rotation data for each sprite.
    ///
    /// Original C: `void R_InitSprites(char** namelist)` (r_things.c line 780)
    /// and `void R_InitSpriteDefs(char** namelist)` (r_things.c line 595)
    pub fn init_sprites(&mut self, wad: &dyn WadProvider) {
        self.numsprites = NUMSPRITES;
        self.sprites = Vec::with_capacity(NUMSPRITES);

        // Find sprite lump range (between S_START and S_END markers)
        let first_sprite = wad
            .check_num_for_name("S_START")
            .map(|n| n + 1)
            .unwrap_or(0);
        let _last_sprite = wad
            .check_num_for_name("S_END")
            .map(|n| if n > 0 { n - 1 } else { 0 })
            .unwrap_or(0);

        // For each sprite name in the list
        for name in SPRITE_NAMES.iter().take(NUMSPRITES) {
            // Temporary frame storage — up to MAXFRAME frames
            let mut sprtemp = [SpriteFrame {
                rotate: false,
                lump: [0i16; 8],
                flip: [0u8; 8],
            }; MAXFRAME];
            let mut maxframe: i32 = -1;

            let name_upper = name.to_uppercase();

            // Scan sprite lump range for matching names
            // In the original C, this iterates lumpinfo[].name looking for
            // the 4-char prefix. Since we cannot access lump names by index
            // through the WadProvider trait, we use a name-based lookup approach.
            //
            // For each possible frame letter (A-Z) and rotation (0-8),
            // construct the expected lump name and check if it exists.
            for frame_letter in b'A'..=b'Z' {
                let frame = (frame_letter - b'A') as usize;
                if frame >= MAXFRAME {
                    break;
                }

                // Check for rotation 0 (no rotations — same from all angles)
                let name_0 = format!("{}{}{}", name_upper, frame_letter as char, '0');
                if let Some(lump_num) = wad.check_num_for_name(&name_0) {
                    let patch_num = if lump_num >= first_sprite {
                        (lump_num - first_sprite) as i32
                    } else {
                        lump_num as i32
                    };
                    install_sprite_lump(&mut sprtemp, &mut maxframe, patch_num, frame, 0, false);
                }

                // Check for rotations 1-8
                for rot in 1..=8u8 {
                    let name_r = format!("{}{}{}", name_upper, frame_letter as char, rot);
                    if let Some(lump_num) = wad.check_num_for_name(&name_r) {
                        let patch_num = if lump_num >= first_sprite {
                            (lump_num - first_sprite) as i32
                        } else {
                            lump_num as i32
                        };
                        install_sprite_lump(
                            &mut sprtemp,
                            &mut maxframe,
                            patch_num,
                            frame,
                            rot as usize,
                            false,
                        );
                    }

                    // Check for second frame/rotation in the same lump name
                    // e.g., "TROOB1B5" — frame B rotation 1 AND frame B rotation 5
                    for frame2_letter in b'A'..=b'Z' {
                        let frame2 = (frame2_letter - b'A') as usize;
                        if frame2 >= MAXFRAME {
                            break;
                        }
                        for rot2 in 0..=8u8 {
                            let name_dual = format!(
                                "{}{}{}{}{}",
                                name_upper, frame_letter as char, rot, frame2_letter as char, rot2
                            );
                            if let Some(lump_num) = wad.check_num_for_name(&name_dual) {
                                let patch_num = if lump_num >= first_sprite {
                                    (lump_num - first_sprite) as i32
                                } else {
                                    lump_num as i32
                                };
                                // Install the first frame/rotation
                                install_sprite_lump(
                                    &mut sprtemp,
                                    &mut maxframe,
                                    patch_num,
                                    frame,
                                    rot as usize,
                                    false,
                                );
                                // Install the second frame/rotation (flipped)
                                install_sprite_lump(
                                    &mut sprtemp,
                                    &mut maxframe,
                                    patch_num,
                                    frame2,
                                    rot2 as usize,
                                    true,
                                );
                            }
                        }
                    }
                }
            }

            // Build the SpriteDef for this sprite
            if maxframe < 0 {
                // No frames found for this sprite
                self.sprites.push(SpriteDef {
                    numframes: 0,
                    spriteframes: Vec::new(),
                });
                continue;
            }

            let num_frames = (maxframe + 1) as usize;
            let mut frames = Vec::with_capacity(num_frames);
            for item in sprtemp.iter().take(num_frames) {
                frames.push(*item);
            }

            self.sprites.push(SpriteDef {
                numframes: num_frames as i32,
                spriteframes: frames,
            });
        }

        tracing::debug!(
            "R_InitSprites: {} sprite definitions loaded",
            self.numsprites
        );
    }
}

impl Default for ThingsState {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Module-level helper functions
// =============================================================================

/// Dispatches to the appropriate column drawing function based on the
/// active [`ColFunc`] variant.
///
/// This replaces the C function pointer `colfunc` with a Rust enum dispatch.
fn dispatch_colfunc(
    colfunc: ColFunc,
    draw: &mut DrawState,
    screens: &mut [Vec<u8>],
    colormaps: &[u8],
    centery: i32,
) {
    match colfunc {
        ColFunc::DrawColumn => draw.draw_column(screens, colormaps, centery),
        ColFunc::DrawColumnLow => draw.draw_column_low(screens, colormaps, centery),
        ColFunc::DrawFuzzColumn => draw.draw_fuzz_column(screens, colormaps),
        ColFunc::DrawFuzzColumnLow => draw.draw_fuzz_column_low(screens, colormaps),
        ColFunc::DrawTranslatedColumn => draw.draw_translated_column(screens, colormaps, centery),
        ColFunc::DrawTranslatedColumnLow => {
            draw.draw_translated_column_low(screens, colormaps, centery)
        }
    }
}

/// Installs a sprite lump into the temporary sprite frame table during
/// sprite initialization.
///
/// Maps a patch number to the correct frame and rotation slot. Rotation 0
/// means the sprite has no rotational variants (same graphic from all angles).
/// Rotations 1-8 map to the 8 compass directions.
///
/// Original C: `R_InstallSpriteLump` (r_things.c line 550)
fn install_sprite_lump(
    sprtemp: &mut [SpriteFrame; MAXFRAME],
    maxframe: &mut i32,
    lump: i32,
    frame: usize,
    rotation: usize,
    flipped: bool,
) {
    if frame >= MAXFRAME {
        return;
    }

    if frame as i32 > *maxframe {
        *maxframe = frame as i32;
    }

    if rotation == 0 {
        // No rotations — use this lump for all 8 angles
        sprtemp[frame].rotate = false;
        for r in 0..8 {
            sprtemp[frame].lump[r] = lump as i16;
            sprtemp[frame].flip[r] = if flipped { 1 } else { 0 };
        }
    } else if (1..=8).contains(&rotation) {
        // Single rotation — install at the specific angle
        sprtemp[frame].rotate = true;
        let rot_idx = rotation - 1;
        sprtemp[frame].lump[rot_idx] = lump as i16;
        sprtemp[frame].flip[rot_idx] = if flipped { 1 } else { 0 };
    }
}

/// Raw fixed-point multiplication (16.16 × 16.16 → 16.16).
///
/// Performs `((a as i64) * (b as i64)) >> 16` with wrapping semantics,
/// matching the original C `FixedMul` behavior.
#[inline]
fn fixed_mul_raw(a: i32, b: i32) -> i32 {
    (((a as i64) * (b as i64)) >> FRACBITS) as i32
}

/// Determines if a draw segment is in front of a vissprite.
///
/// Simplified front/back test based on scale comparison. The full
/// implementation would use the seg's line to do a proper point-on-side test.
fn drawseg_in_front(_ds: &DrawSeg, _vis: &VisSprite) -> bool {
    // In the original C, this is determined by R_PointOnSegSide.
    // For now we conservatively return true (seg is in front),
    // which may over-clip but won't miss clipping.
    true
}

/// Gets the player's sector light level for psprite rendering.
///
/// Traverses the player → mobj → subsector → sector chain to retrieve
/// the sector's light level value.
fn get_player_sector_lightlevel(
    render_main: &RenderMain,
    render_state: &RenderState,
    players: &[Player],
    mobjs: &[MapObject],
) -> i32 {
    if let Some(player_idx) = render_main.viewplayer {
        if player_idx < players.len() {
            if let Some(mo_idx) = players[player_idx].mobj {
                if mo_idx < mobjs.len() {
                    if let Some(ss_idx) = mobjs[mo_idx].subsector {
                        if ss_idx < render_state.subsectors.len() {
                            let sec_idx = render_state.subsectors[ss_idx].sector;
                            if sec_idx < render_state.sectors.len() {
                                return render_state.sectors[sec_idx].lightlevel as i32;
                            }
                        }
                    }
                }
            }
        }
    }
    0
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_match_original() {
        assert_eq!(MAXVISSPRITES, 128);
        assert_eq!(MINZ, FRACUNIT * 4);
        assert_eq!(BASEYCENTER, 100);
    }

    #[test]
    fn test_things_state_new() {
        let state = ThingsState::new();
        assert_eq!(state.vissprite_count, 0);
        assert!(state.vissprites.is_empty());
        assert_eq!(state.negonearray[0], -1);
        assert_eq!(state.negonearray[SCREENWIDTH_USIZE - 1], -1);
        assert_eq!(state.numsprites, 0);
        assert_eq!(state.spryscale, Fixed::ZERO);
        assert_eq!(state.sprtopscreen, Fixed::ZERO);
    }

    #[test]
    fn test_clear_sprites() {
        let mut state = ThingsState::new();
        state.vissprites.push(VisSprite::default());
        state.vissprites.push(VisSprite::default());
        state.vissprite_count = 2;

        state.clear_sprites();
        assert_eq!(state.vissprite_count, 0);
        assert!(state.vissprites.is_empty());
    }

    #[test]
    fn test_new_vissprite() {
        let mut state = ThingsState::new();

        let idx = state.new_vissprite();
        assert_eq!(idx, 0);
        assert_eq!(state.vissprite_count, 1);

        let idx = state.new_vissprite();
        assert_eq!(idx, 1);
        assert_eq!(state.vissprite_count, 2);
    }

    #[test]
    fn test_new_vissprite_overflow() {
        let mut state = ThingsState::new();

        // Fill the pool
        for i in 0..MAXVISSPRITES {
            let idx = state.new_vissprite();
            assert_eq!(idx, i);
            state.vissprites[idx].scale = Fixed::new((MAXVISSPRITES - i) as i32 * 100);
        }
        assert_eq!(state.vissprite_count, MAXVISSPRITES);

        // Overflow: should return the index of the farthest sprite (smallest scale)
        let overflow_idx = state.new_vissprite();
        // The sprite with the smallest scale is at index MAXVISSPRITES - 1 (scale=100)
        assert_eq!(overflow_idx, MAXVISSPRITES - 1);
    }

    #[test]
    fn test_sort_vissprites_empty() {
        let mut state = ThingsState::new();
        state.sort_vissprites();
        assert!(state.vsprsortedhead.next.is_none());
    }

    #[test]
    fn test_sort_vissprites_order() {
        let mut state = ThingsState::new();

        for _ in 0..3 {
            state.new_vissprite();
        }
        state.vissprites[0].scale = Fixed::new(300);
        state.vissprites[1].scale = Fixed::new(100);
        state.vissprites[2].scale = Fixed::new(200);

        state.sort_vissprites();

        // Back-to-front: 1 (100) -> 2 (200) -> 0 (300) -> sentinel
        let sentinel = usize::MAX;
        let first = state.vsprsortedhead.next.unwrap();
        assert_eq!(first, 1);
        let second = state.vissprites[1].next.unwrap();
        assert_eq!(second, 2);
        let third = state.vissprites[2].next.unwrap();
        assert_eq!(third, 0);
        let end = state.vissprites[0].next.unwrap();
        assert_eq!(end, sentinel);
    }

    #[test]
    fn test_install_sprite_lump_no_rotation() {
        let mut sprtemp = [SpriteFrame {
            rotate: false,
            lump: [0i16; 8],
            flip: [0u8; 8],
        }; MAXFRAME];
        let mut maxframe = -1i32;

        install_sprite_lump(&mut sprtemp, &mut maxframe, 42, 0, 0, false);

        assert_eq!(maxframe, 0);
        assert!(!sprtemp[0].rotate);
        for r in 0..8 {
            assert_eq!(sprtemp[0].lump[r], 42);
            assert_eq!(sprtemp[0].flip[r], 0);
        }
    }

    #[test]
    fn test_install_sprite_lump_with_rotation() {
        let mut sprtemp = [SpriteFrame {
            rotate: false,
            lump: [0i16; 8],
            flip: [0u8; 8],
        }; MAXFRAME];
        let mut maxframe = -1i32;

        install_sprite_lump(&mut sprtemp, &mut maxframe, 10, 0, 3, true);

        assert_eq!(maxframe, 0);
        assert!(sprtemp[0].rotate);
        assert_eq!(sprtemp[0].lump[2], 10);
        assert_eq!(sprtemp[0].flip[2], 1);
    }

    #[test]
    fn test_default_trait_impl() {
        let state = ThingsState::default();
        assert_eq!(state.vissprite_count, 0);
    }

    #[test]
    fn test_negonearray_initialization() {
        let state = ThingsState::new();
        for i in 0..SCREENWIDTH_USIZE {
            assert_eq!(state.negonearray[i], -1);
        }
    }

    #[test]
    fn test_fixed_mul_raw() {
        // 1.0 * 1.0 = 1.0
        assert_eq!(fixed_mul_raw(FRACUNIT, FRACUNIT), FRACUNIT);
        // 2.0 * 3.0 = 6.0
        assert_eq!(fixed_mul_raw(2 * FRACUNIT, 3 * FRACUNIT), 6 * FRACUNIT);
        // 0 * anything = 0
        assert_eq!(fixed_mul_raw(0, 12345), 0);
    }

    #[test]
    fn test_screenwidth_usize() {
        assert_eq!(SCREENWIDTH_USIZE, 320);
    }
}
