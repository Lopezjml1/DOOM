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

//! Translated from linuxdoom-1.10/r_bsp.c and r_bsp.h
//!
//! BSP traversal, handling of LineSegs for rendering.
//! Clips the rendered view against walls using a sorted array of solid segments.
//!
//! The BSP tree is traversed front-to-back. Solid (single-sided) walls
//! progressively fill the clip-seg array until the entire screen width is
//! covered, at which point the traversal can stop. Two-sided walls (windows,
//! doors, height changes) are clipped to the remaining gaps but do not add to
//! the solid coverage.

use crate::data::DataState;
use crate::defs::{DrawSeg, Fixed, Node, RenderState, ANG90, ANGLETOFINESHIFT};
use crate::draw::DrawState;
use crate::main::{point_on_side, point_to_angle, RenderMain};
use crate::plane::PlaneState;
use crate::segs::SegsState;
use crate::sky::SkyState;
use crate::things::ThingsState;
use doom_core::types::angle::ANG180;
use doom_core::types::map_data::NF_SUBSECTOR;
use doom_core::types::mobj::MapObject;
use doom_core::util::bbox::{BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of drawsegs allocated per frame.
///
/// Original C (r_defs.h line 55): `#define MAXDRAWSEGS 256`
///
/// This constant is re-exported here to satisfy the BSP module's public API
/// contract. The same value also exists in `doom_core::types::map_data`.
pub const MAXDRAWSEGS: usize = 256;

/// Maximum number of entries in the solid clip-seg array.
///
/// Original C (r_bsp.c line 88): hardcoded limit of 32 clip posts.
const MAXSEGS: usize = 32;

// ---------------------------------------------------------------------------
// Internal clip-range type  (r_bsp.c lines 80-85)
// ---------------------------------------------------------------------------

/// A horizontal screen-column range that has been filled by a solid wall.
///
/// The `solidsegs` array is kept sorted by `first` and no two ranges overlap.
/// Sentinel values at both ends ensure that every search terminates.
#[derive(Clone, Copy, Default)]
struct ClipRange {
    /// Leftmost screen column in this range.
    first: i32,
    /// Rightmost screen column in this range.
    last: i32,
}

// ---------------------------------------------------------------------------
// Bounding-box corner lookup table for R_CheckBBox  (r_bsp.c lines 365-378)
// ---------------------------------------------------------------------------

/// Maps `(boxy << 2) + boxx` to four indices into the bbox array that select
/// the two corners defining the visible extent of the box from the viewpoint.
///
/// Layout per entry: `[x1_idx, y1_idx, x2_idx, y2_idx]` where each index
/// refers to one of BOXTOP(0), BOXBOTTOM(1), BOXLEFT(2), BOXRIGHT(3).
///
/// Entries at indices 3, 5, 7, 11 are unused (padding / viewer-inside-box).
#[rustfmt::skip]
static CHECKCOORD: [[usize; 4]; 12] = [
    [3, 0, 2, 1], // 0  — boxy=0, boxx=0
    [3, 0, 2, 0], // 1  — boxy=0, boxx=1
    [3, 1, 2, 0], // 2  — boxy=0, boxx=2
    [0, 0, 0, 0], // 3  — (unused padding)
    [2, 0, 2, 1], // 4  — boxy=1, boxx=0
    [0, 0, 0, 0], // 5  — viewer inside box (handled specially)
    [3, 1, 3, 0], // 6  — boxy=1, boxx=2
    [0, 0, 0, 0], // 7  — (unused padding)
    [2, 0, 3, 1], // 8  — boxy=2, boxx=0
    [2, 1, 3, 1], // 9  — boxy=2, boxx=1
    [2, 1, 3, 0], // 10 — boxy=2, boxx=2
    [0, 0, 0, 0], // 11 — (unused padding)
];

// ---------------------------------------------------------------------------
// BspState — public BSP traversal state
// ---------------------------------------------------------------------------

/// Consolidated BSP traversal state.
///
/// Replaces the collection of global variables in the original C code:
/// `curline`, `sidedef`, `linedef`, `frontsector`, `backsector`,
/// `drawsegs[]`, `ds_p`, and the internal `solidsegs[]` / `newend`.
pub struct BspState {
    // -- Current line segment being processed (arena indices) --
    /// Index into `RenderState.segs[]` for the seg currently being processed.
    /// Original C: `seg_t* curline`.
    pub curline: Option<usize>,

    /// Index into `RenderState.sides[]` for the side of the current seg.
    /// Original C: `side_t* sidedef`.
    pub sidedef: Option<usize>,

    /// Index into `RenderState.lines[]` for the linedef owning the current seg.
    /// Original C: `line_t* linedef`.
    pub linedef: Option<usize>,

    /// Index into `RenderState.sectors[]` for the front sector of the current
    /// subsector. Original C: `sector_t* frontsector`.
    pub frontsector: Option<usize>,

    /// Index into `RenderState.sectors[]` for the back sector of the current
    /// seg (None for single-sided lines). Original C: `sector_t* backsector`.
    pub backsector: Option<usize>,

    // -- Draw segment pool --
    /// Per-frame draw segment pool. Each visible wall fragment stores a
    /// `DrawSeg` here during BSP traversal.
    /// Original C: `drawseg_t drawsegs[MAXDRAWSEGS]`.
    pub drawsegs: Vec<DrawSeg>,

    /// Number of active draw segments (equal to `drawsegs.len()`).
    /// Original C: `drawseg_t* ds_p` (pointer offset from array start).
    pub ds_p: usize,

    // -- Solid clip-seg tracking (private) --
    /// Sorted array of horizontal screen-column ranges already filled by solid
    /// walls. Two sentinel entries bracket the list to simplify search logic.
    solidsegs: [ClipRange; MAXSEGS],

    /// Index one past the last valid entry in `solidsegs`.
    newend: usize,
}

impl BspState {
    // =====================================================================
    // Construction
    // =====================================================================

    /// Creates a new `BspState` with default (empty) values.
    pub fn new() -> Self {
        Self {
            curline: None,
            sidedef: None,
            linedef: None,
            frontsector: None,
            backsector: None,
            drawsegs: Vec::with_capacity(MAXDRAWSEGS),
            ds_p: 0,
            solidsegs: [ClipRange::default(); MAXSEGS],
            newend: 0,
        }
    }

    // =====================================================================
    // Draw-seg management  (r_bsp.c line 68)
    // =====================================================================

    /// Resets the draw-segment pool for a new frame.
    ///
    /// Original C: `R_ClearDrawSegs` (r_bsp.c line 68).
    pub fn clear_drawsegs(&mut self) {
        self.drawsegs.clear();
        self.ds_p = 0;
    }

    // =====================================================================
    // Clip-seg management  (r_bsp.c lines 245-252)
    // =====================================================================

    /// Initialises the solid-clip array with two sentinel entries that
    /// bracket the entire screen width.
    ///
    /// After this call:
    /// - `solidsegs[0]` covers `(-∞, -1]` (everything left of the screen)
    /// - `solidsegs[1]` covers `[viewwidth, +∞)` (everything right of the
    ///   screen)
    ///
    /// Original C: `R_ClearClipSegs` (r_bsp.c lines 245-252).
    pub fn clear_clip_segs(&mut self, viewwidth: i32) {
        self.solidsegs[0] = ClipRange {
            first: -0x7fff_ffff,
            last: -1,
        };
        self.solidsegs[1] = ClipRange {
            first: viewwidth,
            last: 0x7fff_ffff,
        };
        self.newend = 2;
    }

    // =====================================================================
    // Solid wall clipping  (r_bsp.c lines 103-185)
    // =====================================================================

    /// Clips a solid (single-sided) wall segment against the current
    /// solid-clip array. Visible fragments are sent to
    /// [`SegsState::store_wall_range`]. New solid ranges are inserted or
    /// merged into the clip array.
    ///
    /// Original C: `R_ClipSolidWallSegment` (r_bsp.c lines 103-185).
    #[allow(clippy::too_many_arguments)]
    fn clip_solid_wall_segment(
        &mut self,
        first: i32,
        last: i32,
        segs_state: &mut SegsState,
        render_state: &mut RenderState,
        render_main: &RenderMain,
        draw: &mut DrawState,
        plane: &mut PlaneState,
        data: &mut DataState,
        things: &ThingsState,
        sky: &SkyState,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
    ) {
        // Find the first clip post that touches the range (first-1, last+1).
        let mut start_idx: usize = 0;
        while self.solidsegs[start_idx].last < first - 1 {
            start_idx += 1;
        }

        if first < self.solidsegs[start_idx].first {
            if last < self.solidsegs[start_idx].first - 1 {
                // Post is entirely visible (above start) — insert a new clip
                // post and render the full range.
                let curline_idx = self.curline.unwrap_or(0);
                segs_state.store_wall_range(
                    first,
                    last,
                    curline_idx,
                    render_state,
                    render_main,
                    draw,
                    plane,
                    data,
                    things,
                    sky,
                    &mut self.drawsegs,
                    screens,
                    colormaps,
                );
                self.ds_p = self.drawsegs.len();

                // Shift existing entries right to make room for the new post.
                if self.newend < MAXSEGS {
                    let mut idx = self.newend;
                    self.newend += 1;
                    while idx > start_idx {
                        self.solidsegs[idx] = self.solidsegs[idx - 1];
                        idx -= 1;
                    }
                    self.solidsegs[start_idx] = ClipRange { first, last };
                }
                return;
            }

            // There is a visible fragment above *start.
            let curline_idx = self.curline.unwrap_or(0);
            segs_state.store_wall_range(
                first,
                self.solidsegs[start_idx].first - 1,
                curline_idx,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                &mut self.drawsegs,
                screens,
                colormaps,
            );
            self.ds_p = self.drawsegs.len();

            // Adjust the clip by lowering start's first.
            self.solidsegs[start_idx].first = first;
        }

        // Bottom is contained in start already.
        if last <= self.solidsegs[start_idx].last {
            return;
        }

        // ---- Handle fragments between adjacent clip posts ----
        let mut next_idx = start_idx;
        loop {
            if last < self.solidsegs[next_idx + 1].first - 1 {
                break; // No more overlapping posts.
            }
            // There is a visible fragment between two posts.
            let curline_idx = self.curline.unwrap_or(0);
            let frag_first = self.solidsegs[next_idx].last + 1;
            let frag_last = self.solidsegs[next_idx + 1].first - 1;
            segs_state.store_wall_range(
                frag_first,
                frag_last,
                curline_idx,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                &mut self.drawsegs,
                screens,
                colormaps,
            );
            self.ds_p = self.drawsegs.len();

            next_idx += 1;

            if last <= self.solidsegs[next_idx].last {
                // Bottom is now contained in next.
                self.solidsegs[start_idx].last = self.solidsegs[next_idx].last;
                // Jump to crunch (merge) logic below.
                self.crunch_segs(start_idx, next_idx);
                return;
            }
        }

        // There is a visible fragment after *next.
        let curline_idx = self.curline.unwrap_or(0);
        let frag_first = self.solidsegs[next_idx].last + 1;
        segs_state.store_wall_range(
            frag_first,
            last,
            curline_idx,
            render_state,
            render_main,
            draw,
            plane,
            data,
            things,
            sky,
            &mut self.drawsegs,
            screens,
            colormaps,
        );
        self.ds_p = self.drawsegs.len();

        // Extend start's range to cover everything up to `last`.
        self.solidsegs[start_idx].last = last;

        // Crunch: remove clip posts from start+1 through next.
        self.crunch_segs(start_idx, next_idx);
    }

    /// Removes solid-seg entries in the range `(start_idx, next_idx]` by
    /// compacting the array.
    ///
    /// This implements the `crunch:` label logic from the original C code
    /// (r_bsp.c lines 175-185).
    fn crunch_segs(&mut self, start_idx: usize, next_idx: usize) {
        if next_idx == start_idx {
            return; // Post just extended past one post, nothing to remove.
        }
        // Shift elements [next_idx+1 .. newend) left to [start_idx+1 ..).
        let mut dst = start_idx + 1;
        let mut src = next_idx + 1;
        while src < self.newend {
            self.solidsegs[dst] = self.solidsegs[src];
            dst += 1;
            src += 1;
        }
        self.newend = dst;
    }

    // =====================================================================
    // Pass-through wall clipping  (r_bsp.c lines 196-238)
    // =====================================================================

    /// Clips a pass-through (two-sided) wall segment against the current
    /// solid-clip array. Visible fragments are rendered but the clip array
    /// is NOT modified — two-sided walls never occlude.
    ///
    /// Original C: `R_ClipPassWallSegment` (r_bsp.c lines 196-238).
    #[allow(clippy::too_many_arguments)]
    fn clip_pass_wall_segment(
        &mut self,
        first: i32,
        last: i32,
        segs_state: &mut SegsState,
        render_state: &mut RenderState,
        render_main: &RenderMain,
        draw: &mut DrawState,
        plane: &mut PlaneState,
        data: &mut DataState,
        things: &ThingsState,
        sky: &SkyState,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
    ) {
        let mut start_idx: usize = 0;
        while self.solidsegs[start_idx].last < first - 1 {
            start_idx += 1;
        }

        if first < self.solidsegs[start_idx].first {
            if last < self.solidsegs[start_idx].first - 1 {
                // Entirely visible.
                let curline_idx = self.curline.unwrap_or(0);
                segs_state.store_wall_range(
                    first,
                    last,
                    curline_idx,
                    render_state,
                    render_main,
                    draw,
                    plane,
                    data,
                    things,
                    sky,
                    &mut self.drawsegs,
                    screens,
                    colormaps,
                );
                self.ds_p = self.drawsegs.len();
                return;
            }
            // Fragment above start.
            let curline_idx = self.curline.unwrap_or(0);
            segs_state.store_wall_range(
                first,
                self.solidsegs[start_idx].first - 1,
                curline_idx,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                &mut self.drawsegs,
                screens,
                colormaps,
            );
            self.ds_p = self.drawsegs.len();
        }

        if last <= self.solidsegs[start_idx].last {
            return;
        }

        // Walk through adjacent clip posts, emitting visible fragments.
        while last >= self.solidsegs[start_idx + 1].first - 1 {
            let curline_idx = self.curline.unwrap_or(0);
            let frag_first = self.solidsegs[start_idx].last + 1;
            let frag_last = self.solidsegs[start_idx + 1].first - 1;
            segs_state.store_wall_range(
                frag_first,
                frag_last,
                curline_idx,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                &mut self.drawsegs,
                screens,
                colormaps,
            );
            self.ds_p = self.drawsegs.len();

            start_idx += 1;

            if last <= self.solidsegs[start_idx].last {
                return;
            }
        }

        // Fragment after the last overlapping post.
        let curline_idx = self.curline.unwrap_or(0);
        let frag_first = self.solidsegs[start_idx].last + 1;
        segs_state.store_wall_range(
            frag_first,
            last,
            curline_idx,
            render_state,
            render_main,
            draw,
            plane,
            data,
            things,
            sky,
            &mut self.drawsegs,
            screens,
            colormaps,
        );
        self.ds_p = self.drawsegs.len();
    }

    // =====================================================================
    // Per-seg processing  (r_bsp.c lines 259-356)
    // =====================================================================

    /// Processes a single seg: computes view-relative angles, clips to the
    /// view cone, converts to screen X coordinates, and dispatches to either
    /// the solid or pass-through wall clipper.
    ///
    /// Original C: `R_AddLine` (r_bsp.c lines 259-356).
    #[allow(clippy::too_many_arguments)]
    pub fn add_line(
        &mut self,
        line_idx: usize,
        segs_state: &mut SegsState,
        render_state: &mut RenderState,
        render_main: &RenderMain,
        draw: &mut DrawState,
        plane: &mut PlaneState,
        data: &mut DataState,
        things: &ThingsState,
        sky: &SkyState,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
    ) {
        self.curline = Some(line_idx);

        let seg = render_state.segs[line_idx];
        let v1 = render_state.vertexes[seg.v1];
        let v2 = render_state.vertexes[seg.v2];

        // Compute world-space angles from the viewer to each vertex.
        let angle1 = point_to_angle(v1.x, v1.y, render_main);
        let angle2 = point_to_angle(v2.x, v2.y, render_main);

        // Span in BAM units. If >= 180° the wall faces away (backface cull).
        let span = angle1 - angle2;
        if span.value() >= ANG180.value() {
            return;
        }

        // Store the raw (not view-relative) angle for R_StoreWallRange.
        segs_state.rw_angle1 = angle1.value() as i32;

        // Make angles view-relative.
        let mut angle1 = angle1 - render_main.viewangle;
        let mut angle2 = angle2 - render_main.viewangle;

        let clipangle = render_main.clipangle;
        let double_clip = clipangle + clipangle;

        // ------ Clip left edge to view cone ------
        let tspan = angle1 + clipangle;
        if tspan > double_clip {
            let tspan2 = tspan - double_clip;
            // If the overshoot is >= the entire span the seg is fully outside.
            if tspan2 >= span {
                return;
            }
            angle1 = clipangle;
        }

        // ------ Clip right edge to view cone ------
        let tspan = clipangle - angle2;
        if tspan > double_clip {
            let tspan2 = tspan - double_clip;
            if tspan2 >= span {
                return;
            }
            angle2 = -clipangle;
        }

        // Convert BAM angles to fine-angle indices, then to screen columns
        // via the viewangletox lookup table.
        let a1_fine = (angle1 + ANG90) >> ANGLETOFINESHIFT;
        let a2_fine = (angle2 + ANG90) >> ANGLETOFINESHIFT;

        let a1_idx = (a1_fine.value() as usize).min(render_main.viewangletox.len() - 1);
        let a2_idx = (a2_fine.value() as usize).min(render_main.viewangletox.len() - 1);

        let x1 = render_main.viewangletox[a1_idx];
        let x2 = render_main.viewangletox[a2_idx];

        // Does not cross a pixel boundary.
        if x1 == x2 {
            return;
        }

        // Set the backsector (None for single-sided lines).
        self.backsector = seg.backsector;
        self.sidedef = Some(seg.sidedef);
        self.linedef = Some(seg.linedef);

        // ---- Determine solid vs pass-through ----
        let is_solid = if let Some(bs_idx) = self.backsector {
            let fs_idx = self.frontsector.unwrap_or(0);

            let fs_floor = render_state.sectors[fs_idx].floorheight;
            let fs_ceil = render_state.sectors[fs_idx].ceilingheight;
            let bs_floor = render_state.sectors[bs_idx].floorheight;
            let bs_ceil = render_state.sectors[bs_idx].ceilingheight;

            // Closed door: back ceiling at or below front floor, or
            // back floor at or above front ceiling.
            if bs_ceil <= fs_floor || bs_floor >= fs_ceil {
                true
            } else if bs_ceil != fs_ceil || bs_floor != fs_floor {
                // Height difference → window / step (pass-through).
                false
            } else {
                // Same heights — reject empty trigger lines.
                let fs_ceilpic = render_state.sectors[fs_idx].ceilingpic;
                let fs_floorpic = render_state.sectors[fs_idx].floorpic;
                let fs_light = render_state.sectors[fs_idx].lightlevel;
                let bs_ceilpic = render_state.sectors[bs_idx].ceilingpic;
                let bs_floorpic = render_state.sectors[bs_idx].floorpic;
                let bs_light = render_state.sectors[bs_idx].lightlevel;

                if bs_ceilpic == fs_ceilpic
                    && bs_floorpic == fs_floorpic
                    && bs_light == fs_light
                    && render_state.sides[seg.sidedef].midtexture == 0
                {
                    // Identical sectors with no middle texture — nothing to draw.
                    return;
                }
                // Pass-through (some visual difference exists).
                false
            }
        } else {
            // Single-sided line is always solid.
            true
        };

        if is_solid {
            self.clip_solid_wall_segment(
                x1,
                x2 - 1,
                segs_state,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                screens,
                colormaps,
            );
        } else {
            self.clip_pass_wall_segment(
                x1,
                x2 - 1,
                segs_state,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                screens,
                colormaps,
            );
        }
    }

    // =====================================================================
    // Bounding-box visibility check  (r_bsp.c lines 365-487)
    // =====================================================================

    /// Tests whether any part of a BSP node's bounding box is potentially
    /// visible (i.e. not entirely occluded by solid walls already drawn).
    ///
    /// The viewer's position relative to the box determines which two
    /// corners define the visible angular extent. Those corners are then
    /// converted to screen-column coordinates and checked against the
    /// solid-clip array.
    ///
    /// Returns `true` if any part of the box might be visible.
    ///
    /// Original C: `R_CheckBBox` (r_bsp.c lines 365-487).
    pub fn check_bbox(&self, bspcoord: &[Fixed; 4], render_main: &RenderMain) -> bool {
        let viewx = render_main.viewx;
        let viewy = render_main.viewy;

        // Determine the viewer's position relative to the box edges.
        let boxx: usize = if viewx <= bspcoord[BOXLEFT] {
            0
        } else if viewx < bspcoord[BOXRIGHT] {
            1
        } else {
            2
        };

        let boxy: usize = if viewy >= bspcoord[BOXTOP] {
            0
        } else if viewy > bspcoord[BOXBOTTOM] {
            1
        } else {
            2
        };

        let boxpos = (boxy << 2) + boxx;

        // boxpos == 5 means the viewer is inside the box — always visible.
        if boxpos == 5 {
            return true;
        }

        // Select the two corners that define the angular extent of the box.
        let cc = &CHECKCOORD[boxpos];
        let x1 = bspcoord[cc[0]];
        let y1 = bspcoord[cc[1]];
        let x2 = bspcoord[cc[2]];
        let y2 = bspcoord[cc[3]];

        // Compute view-relative angles to the selected corners.
        let angle1 = point_to_angle(x1, y1, render_main) - render_main.viewangle;
        let angle2 = point_to_angle(x2, y2, render_main) - render_main.viewangle;

        let span = angle1 - angle2;

        // If the span is >= 180° the viewer is sitting on the line between
        // the two corners — treat as potentially visible.
        if span.value() >= ANG180.value() {
            return true;
        }

        let clipangle = render_main.clipangle;
        let double_clip = clipangle + clipangle;

        let mut angle1 = angle1;
        let mut angle2 = angle2;

        // Clip left edge.
        let tspan = angle1 + clipangle;
        if tspan > double_clip {
            let tspan2 = tspan - double_clip;
            if tspan2 >= span {
                return false; // Totally off the left edge.
            }
            angle1 = clipangle;
        }

        // Clip right edge.
        let tspan = clipangle - angle2;
        if tspan > double_clip {
            let tspan2 = tspan - double_clip;
            if tspan2 >= span {
                return false; // Totally off the right edge.
            }
            angle2 = -clipangle;
        }

        // Convert to screen X coordinates.
        let a1_fine = (angle1 + ANG90) >> ANGLETOFINESHIFT;
        let a2_fine = (angle2 + ANG90) >> ANGLETOFINESHIFT;

        let a1_idx = (a1_fine.value() as usize).min(render_main.viewangletox.len() - 1);
        let a2_idx = (a2_fine.value() as usize).min(render_main.viewangletox.len() - 1);

        let sx1 = render_main.viewangletox[a1_idx];
        let sx2 = render_main.viewangletox[a2_idx];

        // Does not cross a pixel.
        if sx1 == sx2 {
            return false;
        }
        let sx2 = sx2 - 1;

        // Walk the solid-clip array: if the entire [sx1, sx2] range is
        // contained within a single solid post, the box is fully occluded.
        let mut idx = 0usize;
        while self.solidsegs[idx].last < sx2 {
            idx += 1;
        }

        if sx1 >= self.solidsegs[idx].first && sx2 <= self.solidsegs[idx].last {
            return false; // Fully occluded.
        }

        true
    }

    // =====================================================================
    // Subsector processing  (r_bsp.c lines 497-542)
    // =====================================================================

    /// Processes a leaf BSP node (subsector): locates or creates floor and
    /// ceiling visplanes, projects sprites, and processes every seg in the
    /// subsector.
    ///
    /// Original C: `R_Subsector` (r_bsp.c lines 497-542).
    #[allow(clippy::too_many_arguments)]
    pub fn subsector(
        &mut self,
        num: usize,
        segs_state: &mut SegsState,
        render_state: &mut RenderState,
        render_main: &mut RenderMain,
        draw: &mut DrawState,
        plane: &mut PlaneState,
        data: &mut DataState,
        things: &mut ThingsState,
        sky: &SkyState,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
        mobjs: &[MapObject],
    ) {
        // Range check.
        if num >= render_state.subsectors.len() {
            return;
        }

        render_main.sscount += 1;

        let sub = render_state.subsectors[num];
        let sec_idx = sub.sector;
        self.frontsector = Some(sec_idx);

        // Read sector properties for floor/ceiling plane setup.
        let floor_height = render_state.sectors[sec_idx].floorheight;
        let ceil_height = render_state.sectors[sec_idx].ceilingheight;
        let floor_pic = render_state.sectors[sec_idx].floorpic as i32;
        let ceil_pic = render_state.sectors[sec_idx].ceilingpic as i32;
        let light_level = render_state.sectors[sec_idx].lightlevel as i32;

        let viewz = render_main.viewz;
        let skyflatnum = sky.skyflatnum;

        // Find or create the floor visplane.
        if floor_height < viewz {
            plane.floorplane =
                Some(plane.find_plane(floor_height.raw(), floor_pic, light_level, skyflatnum));
        } else {
            plane.floorplane = None;
        }

        // Find or create the ceiling visplane.
        if ceil_height > viewz || ceil_pic == skyflatnum {
            plane.ceilingplane =
                Some(plane.find_plane(ceil_height.raw(), ceil_pic, light_level, skyflatnum));
        } else {
            plane.ceilingplane = None;
        }

        // Project all map objects in this subsector's sector into the
        // vissprite list.
        things.add_sprites(sec_idx, render_main, render_state, data, mobjs);

        // Process every seg in this subsector.
        let first_line = sub.firstline as usize;
        let count = sub.numlines as usize;

        for i in 0..count {
            self.add_line(
                first_line + i,
                segs_state,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                screens,
                colormaps,
            );
        }
    }

    // =====================================================================
    // Recursive BSP traversal  (r_bsp.c lines 552-578)
    // =====================================================================

    /// Recursively traverses the BSP tree, rendering subsectors
    /// front-to-back.
    ///
    /// If `bspnum` has the [`NF_SUBSECTOR`] flag set (bit 15), the lower
    /// bits are a subsector index and [`subsector`](Self::subsector) is
    /// called directly. Otherwise, the node's partition line is tested to
    /// determine which child is in front, the front child is recursed into,
    /// and then (if the back child's bounding box is potentially visible) the
    /// back child is recursed into as well.
    ///
    /// Original C: `R_RenderBSPNode` (r_bsp.c lines 552-578).
    #[allow(clippy::too_many_arguments)]
    pub fn render_bsp_node(
        &mut self,
        bspnum: i32,
        segs_state: &mut SegsState,
        render_state: &mut RenderState,
        render_main: &mut RenderMain,
        draw: &mut DrawState,
        plane: &mut PlaneState,
        data: &mut DataState,
        things: &mut ThingsState,
        sky: &SkyState,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
        mobjs: &[MapObject],
    ) {
        // Leaf node check: if the NF_SUBSECTOR bit is set in the 16-bit
        // child value, this is a subsector.
        if bspnum & (NF_SUBSECTOR as i32) != 0 {
            let sub_idx = if bspnum == -1 {
                // Degenerate BSP: only one subsector in the entire map.
                0usize
            } else {
                (bspnum & !(NF_SUBSECTOR as i32)) as usize
            };
            self.subsector(
                sub_idx,
                segs_state,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                screens,
                colormaps,
                mobjs,
            );
            return;
        }

        // Internal BSP node.
        let bsp_idx = bspnum as usize;
        if bsp_idx >= render_state.nodes.len() {
            return;
        }

        // Copy the node data out so we don't hold a borrow on render_state
        // across recursive calls.
        let node: Node = render_state.nodes[bsp_idx];

        // Determine which side of the partition line the viewpoint is on.
        let side = point_on_side(render_main.viewx, render_main.viewy, &node);
        let side_u = side as usize;

        // Cache child references and the back-side bounding box.
        let front_child = node.children[side_u] as i32;
        let back_child = node.children[side_u ^ 1] as i32;
        let back_bbox = node.bbox[side_u ^ 1];

        // Recurse into the front (nearer) side first.
        self.render_bsp_node(
            front_child,
            segs_state,
            render_state,
            render_main,
            draw,
            plane,
            data,
            things,
            sky,
            screens,
            colormaps,
            mobjs,
        );

        // Only recurse into the back side if its bounding box is potentially
        // visible.
        if self.check_bbox(&back_bbox, render_main) {
            self.render_bsp_node(
                back_child,
                segs_state,
                render_state,
                render_main,
                draw,
                plane,
                data,
                things,
                sky,
                screens,
                colormaps,
                mobjs,
            );
        }
    }
}

impl Default for BspState {
    fn default() -> Self {
        Self::new()
    }
}
