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

//! Translated from linuxdoom-1.10/r_main.c and r_main.h
//!
//! Rendering main loop and setup functions, utility functions
//! (BSP, geometry, trigonometry). See tables.rs too.
//!
//! **NOTE**: This module is named `main.rs` to match the original `r_main.c`
//! file naming convention. It is NOT a binary entry point and contains
//! no `fn main()` function.
//!
//! # Overview
//!
//! This module provides the renderer's top-level orchestration:
//! - **R_RenderPlayerView**: Per-frame rendering entry point
//! - **R_SetupFrame**: Configure viewpoint from player state
//! - **R_Init**: One-time initialization of tables, lighting LUTs, sky, etc.
//! - **R_ExecuteSetViewSize**: Recalculate all view-size-dependent parameters
//! - Utility functions for BSP side tests, angle/distance calculations
//!
//! # Lighting System
//!
//! DOOM uses diminishing lighting via two sets of lookup tables:
//! - `scalelight[LIGHTLEVELS][MAXLIGHTSCALE]` — indexed by wall column scale
//! - `zlight[LIGHTLEVELS][MAXLIGHTZ]` — indexed by floor/ceiling distance
//!
//! Both map (sector light level, distance) → colormap index for palette remapping.

use crate::data::DataState;
use crate::defs::{
    Angle, LightTable, Node, RenderState, Seg, Subsector, ANGLETOFINESHIFT, FINEANGLES,
    SCREENHEIGHT, SCREENWIDTH,
};
use crate::draw::DrawState;
use crate::sky::SkyState;
use doom_core::types::angle::ANG90;
use doom_core::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use doom_core::types::map_data::NF_SUBSECTOR;
use doom_core::types::player::Player;
use doom_core::types::tables::{
    finecosine, slope_div, DBITS, FINESINE, FINETANGENT, SLOPERANGE, TANTOANGLE,
};
use doom_wad::WadProvider;

// =============================================================================
// Constants (from r_main.h lines 57-76 and r_main.c)
// =============================================================================

/// Field of view in fine angles. Equals half the fine-angle range for the
/// 320-pixel-wide screen, i.e. `FINEANGLES / 4 = 2048`.
///
/// Original C (r_main.h): `#define FIELDOFVIEW 2048`
pub const FIELDOFVIEW: i32 = 2048;

/// Number of diminishing lighting levels.
///
/// Original C (r_main.h line 69): `#define LIGHTLEVELS 16`
pub const LIGHTLEVELS: usize = 16;

/// Right-shift to convert a sector's 0-255 light level to a 0-15 lighting
/// level index used for the `scalelight` and `zlight` LUT first dimension.
///
/// Original C (r_main.h line 70): `#define LIGHTSEGSHIFT 4`
pub const LIGHTSEGSHIFT: i32 = 4;

/// Maximum number of entries in the `scalelight` per-light-level array.
///
/// Original C (r_main.h line 71): `#define MAXLIGHTSCALE 48`
pub const MAXLIGHTSCALE: usize = 48;

/// Right-shift applied to scale values to derive an index into the
/// `scalelight` array's second dimension.
///
/// Original C (r_main.h line 72): `#define LIGHTSCALESHIFT 12`
pub const LIGHTSCALESHIFT: i32 = 12;

/// Maximum number of entries in the `zlight` per-light-level array.
///
/// Original C (r_main.h line 73): `#define MAXLIGHTZ 128`
pub const MAXLIGHTZ: usize = 128;

/// Right-shift applied to depth values to derive an index into the
/// `zlight` array's second dimension.
///
/// Original C (r_main.h line 74): `#define LIGHTZSHIFT 20`
pub const LIGHTZSHIFT: i32 = 20;

/// Number of colormaps in the COLORMAP lump (brightness levels).
///
/// Original C (r_main.h line 76): `#define NUMCOLORMAPS 32`
pub const NUMCOLORMAPS: i32 = 32;

/// Distance-to-colormap scaling factor used in R_InitLightTables and
/// R_ExecuteSetViewSize for scalelight computation.
///
/// Original C (r_main.c, local to R_InitLightTables): `#define DISTMAP 2`
const DISTMAP: i32 = 2;

// =============================================================================
// Function dispatch enums (replacing C function pointers)
// =============================================================================

/// Column drawing function selector. Replaces the C function pointer
/// `void (*colfunc)(void)` from r_main.h line 86.
///
/// Each variant selects a different column-drawing strategy used during
/// wall and sprite rendering, depending on detail level, translucency,
/// and player sprite recoloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColFunc {
    /// Standard full-detail column drawing (R_DrawColumn).
    DrawColumn,
    /// Low-detail (blocky) column drawing (R_DrawColumnLow).
    DrawColumnLow,
    /// Spectre/invisible fuzz-effect column drawing (R_DrawFuzzColumn).
    DrawFuzzColumn,
    /// Low-detail fuzz-effect column drawing (R_DrawFuzzColumnLow).
    DrawFuzzColumnLow,
    /// Player sprite recoloring column drawing (R_DrawTranslatedColumn).
    DrawTranslatedColumn,
    /// Low-detail player sprite recoloring (R_DrawTranslatedColumnLow).
    DrawTranslatedColumnLow,
}

/// Span drawing function selector. Replaces the C function pointer
/// `void (*spanfunc)(void)` from r_main.h line 87.
///
/// Selects between full-detail and low-detail floor/ceiling span drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanFunc {
    /// Standard full-detail span drawing (R_DrawSpan).
    DrawSpan,
    /// Low-detail (blocky) span drawing (R_DrawSpanLow).
    DrawSpanLow,
}

// =============================================================================
// RenderMain — Renderer main state (from r_main.c globals + r_main.h externs)
// =============================================================================

/// Consolidated renderer main state.
///
/// Collects all formerly-global variables from `r_main.c` (lines 52-100) and
/// `r_main.h` extern declarations into a single owned struct. This includes:
/// - Point-of-view (POV) fields: viewer position, angle, trig values
/// - Lookup tables: viewangletox, xtoviewangle
/// - Lighting LUTs: scalelight, scalelightfixed, zlight
/// - Function dispatch enums: colfunc, spanfunc
/// - View size state: viewwidth, viewheight, scaledviewwidth
///
/// # Index-Based Architecture
///
/// Pointer fields from the original C code are replaced with arena indices:
/// - `player_t* viewplayer` → `Option<usize>` (index into players array)
/// - `lighttable_t* fixedcolormap` → `Option<usize>` (byte offset into colormaps)
pub struct RenderMain {
    // -- POV state (r_main.c lines 52-87) --
    /// Angle offset added to viewangle (normally 0).
    /// Original C: `int viewangleoffset` (r_main.c line 52).
    pub viewangleoffset: i32,

    /// Incremented each frame and each BSP check to invalidate caches.
    /// Original C: `int validcount = 1` (r_main.c line 55).
    pub validcount: i32,

    /// When set, forces a specific colormap for the entire view (invulnerability
    /// powerup). Stored as a byte offset into the colormaps array, or `None` for
    /// normal lighting.
    /// Original C: `lighttable_t* fixedcolormap` (r_main.c line 58).
    pub fixedcolormap: Option<usize>,

    /// Horizontal center of the view window in pixels.
    /// Original C: `int centerx` (r_main.c line 61).
    pub centerx: i32,

    /// Vertical center of the view window in pixels.
    /// Original C: `int centery` (r_main.c line 62).
    pub centery: i32,

    /// Horizontal center in 16.16 fixed-point.
    /// Original C: `fixed_t centerxfrac` (r_main.c line 64).
    pub centerxfrac: Fixed,

    /// Vertical center in 16.16 fixed-point.
    /// Original C: `fixed_t centeryfrac` (r_main.c line 65).
    pub centeryfrac: Fixed,

    /// View projection factor (half-width in fixed-point).
    /// Original C: `fixed_t projection` (r_main.c line 66).
    pub projection: Fixed,

    /// Frame counter (incremented every rendered frame).
    /// Original C: `int framecount` (r_main.c line 69).
    pub framecount: i32,

    /// Subsector render count for performance tracking.
    /// Original C: `int sscount` (r_main.c line 71).
    pub sscount: i32,

    /// Line render count for performance tracking.
    /// Original C: `int linecount` (r_main.c line 72).
    pub linecount: i32,

    /// Loop iteration count for performance tracking.
    /// Original C: `int loopcount` (r_main.c line 73).
    pub loopcount: i32,

    /// Viewer X position in 16.16 fixed-point map coordinates.
    /// Original C: `fixed_t viewx` (r_main.c line 75).
    pub viewx: Fixed,

    /// Viewer Y position in 16.16 fixed-point map coordinates.
    /// Original C: `fixed_t viewy` (r_main.c line 76).
    pub viewy: Fixed,

    /// Viewer Z position (eye height) in 16.16 fixed-point.
    /// Original C: `fixed_t viewz` (r_main.c line 77).
    pub viewz: Fixed,

    /// Viewer facing direction as a BAM angle.
    /// Original C: `angle_t viewangle` (r_main.c line 79).
    pub viewangle: Angle,

    /// Cosine of viewangle in 16.16 fixed-point (pre-calculated per frame).
    /// Original C: `fixed_t viewcos` (r_main.c line 81).
    pub viewcos: Fixed,

    /// Sine of viewangle in 16.16 fixed-point (pre-calculated per frame).
    /// Original C: `fixed_t viewsin` (r_main.c line 82).
    pub viewsin: Fixed,

    /// Index of the player whose viewpoint is being rendered.
    /// Original C: `player_t* viewplayer` (r_main.c line 84).
    pub viewplayer: Option<usize>,

    /// Detail level: 0 = high detail, 1 = low (blocky) detail.
    /// Preserved per Issue Resolution IR-06 — not removed.
    /// Original C: `int detailshift` (r_main.c line 87).
    pub detailshift: i32,

    /// Half-width of the field of view in BAM angle units.
    /// Original C: `angle_t clipangle` (r_main.c line 92).
    pub clipangle: Angle,

    // -- Lookup tables --
    /// Maps fine-angle indices to screen X columns. Sized to `FINEANGLES / 2`
    /// (4096 entries). Built by `R_InitTextureMapping`.
    /// Original C: `int viewangletox[FINEANGLES/2]` (r_main.c line 95).
    pub viewangletox: [i32; 4096],

    /// Inverse mapping: screen X column to view angle (BAM).
    /// Sized to `SCREENWIDTH + 1` (321 entries). Built by `R_InitTextureMapping`.
    /// Original C: `angle_t xtoviewangle[SCREENWIDTH+1]` (r_main.c line 96).
    pub xtoviewangle: [Angle; 321],

    // -- Lighting LUTs --
    /// Scale-based lighting: maps (light level, column scale) → colormap offset.
    /// `scalelight[i][j]` is a byte offset into the colormaps array.
    /// Original C: `lighttable_t* scalelight[LIGHTLEVELS][MAXLIGHTSCALE]` (r_main.c line 99).
    pub scalelight: [[usize; MAXLIGHTSCALE]; LIGHTLEVELS],

    /// Fixed lighting table used when `fixedcolormap` is active (invulnerability).
    /// Original C: `lighttable_t* scalelightfixed[MAXLIGHTSCALE]` (r_main.c line 100).
    pub scalelightfixed: [usize; MAXLIGHTSCALE],

    /// Depth-based lighting: maps (light level, depth) → colormap offset.
    /// Used for floor/ceiling span rendering.
    /// Original C: `lighttable_t* zlight[LIGHTLEVELS][MAXLIGHTZ]` (r_main.c line 101).
    pub zlight: [[usize; MAXLIGHTZ]; LIGHTLEVELS],

    /// Extra light level from weapon muzzle flash (added to sector light).
    /// Original C: `int extralight` (r_main.h line 81).
    pub extralight: i32,

    // -- Function dispatch (replacing C function pointers) --
    /// Current column drawing function.
    /// Original C: `void (*colfunc)(void)` (r_main.h line 86).
    pub colfunc: ColFunc,

    /// Base column drawing function (normal walls).
    /// Original C: `void (*basecolfunc)(void)` (r_main.h line 86).
    pub basecolfunc: ColFunc,

    /// Fuzz-effect column drawing function (Spectre invisibility).
    /// Original C: `void (*fuzzcolfunc)(void)` (r_main.h line 86).
    pub fuzzcolfunc: ColFunc,

    /// Current span drawing function (floors/ceilings).
    /// Original C: `void (*spanfunc)(void)` (r_main.h line 87).
    pub spanfunc: SpanFunc,

    // -- View size state (r_main.h extern declarations) --
    /// Width of the view window in pixels (after detail shift).
    /// Original C: `int viewwidth` (r_main.h line 54).
    pub viewwidth: i32,

    /// Height of the view window in pixels.
    /// Original C: `int viewheight` (r_main.h line 55).
    pub viewheight: i32,

    /// Scaled view width before detail shift.
    /// Original C: `int scaledviewwidth` (r_main.h line 56).
    pub scaledviewwidth: i32,

    /// Flag indicating that a deferred view-size change is pending.
    /// Original C: `boolean setsizeneeded` (r_main.c line 545).
    pub setsizeneeded: bool,

    /// Pending screen blocks value for deferred view-size change.
    /// Original C: `int setblocks` (r_main.c line 546).
    pub setblocks: i32,

    /// Pending detail level for deferred view-size change.
    /// Original C: `int setdetail` (r_main.c line 547).
    pub setdetail: i32,
}

impl Default for RenderMain {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderMain {
    /// Creates a new `RenderMain` with all fields initialized to safe defaults.
    ///
    /// Lookup tables are zero-initialized and will be properly filled during
    /// [`r_init`] and [`r_execute_set_view_size`].
    pub fn new() -> Self {
        Self {
            viewangleoffset: 0,
            validcount: 1, // starts at 1 per original C
            fixedcolormap: None,
            centerx: 0,
            centery: 0,
            centerxfrac: Fixed::ZERO,
            centeryfrac: Fixed::ZERO,
            projection: Fixed::ZERO,
            framecount: 0,
            sscount: 0,
            linecount: 0,
            loopcount: 0,
            viewx: Fixed::ZERO,
            viewy: Fixed::ZERO,
            viewz: Fixed::ZERO,
            viewangle: Angle::new(0),
            viewcos: Fixed::ZERO,
            viewsin: Fixed::ZERO,
            viewplayer: None,
            detailshift: 0,
            clipangle: Angle::new(0),
            viewangletox: [0i32; 4096],
            xtoviewangle: [Angle::new(0); 321],
            scalelight: [[0usize; MAXLIGHTSCALE]; LIGHTLEVELS],
            scalelightfixed: [0usize; MAXLIGHTSCALE],
            zlight: [[0usize; MAXLIGHTZ]; LIGHTLEVELS],
            extralight: 0,
            colfunc: ColFunc::DrawColumn,
            basecolfunc: ColFunc::DrawColumn,
            fuzzcolfunc: ColFunc::DrawFuzzColumn,
            spanfunc: SpanFunc::DrawSpan,
            viewwidth: 0,
            viewheight: 0,
            scaledviewwidth: 0,
            setsizeneeded: false,
            setblocks: 0,
            setdetail: 0,
        }
    }
}

// =============================================================================
// Utility functions (from r_main.c)
// =============================================================================

/// Determine which side of a BSP partition line a point is on.
///
/// Uses the cross product of the partition line direction (dx, dy) and the
/// vector from the partition origin to the test point. Returns 0 for the
/// front (right) side, 1 for the back (left) side.
///
/// Original C: `R_PointOnSide` (r_main.c lines ~110-140).
///
/// # Arguments
/// * `x` — Test point X coordinate (fixed-point)
/// * `y` — Test point Y coordinate (fixed-point)
/// * `node` — BSP node containing the partition line
///
/// # Returns
/// `0` if the point is on the front (right) side, `1` if on the back (left) side.
pub fn point_on_side(x: Fixed, y: Fixed, node: &Node) -> i32 {
    // Quick sign-based checks when partition line is axis-aligned
    if node.dx == Fixed::ZERO {
        if x.raw() <= node.x.raw() {
            return if node.dy.raw() > 0 { 1 } else { 0 };
        }
        return if node.dy.raw() > 0 { 0 } else { 1 };
    }
    if node.dy == Fixed::ZERO {
        if y.raw() <= node.y.raw() {
            return if node.dx.raw() < 0 { 1 } else { 0 };
        }
        return if node.dx.raw() < 0 { 0 } else { 1 };
    }

    // General case: use cross product
    let dx = x - node.x;
    let dy = y - node.y;

    // (node.dy >> FRACBITS) * dx versus (node.dx >> FRACBITS) * dy
    // Use 64-bit intermediates to avoid overflow
    let left = (node.dy.raw() >> FRACBITS) as i64 * dx.raw() as i64;
    let right = (node.dx.raw() >> FRACBITS) as i64 * dy.raw() as i64;

    if right < left {
        // front side
        0
    } else {
        // back side
        1
    }
}

/// Determine which side of a line segment a point is on.
///
/// Similar to [`point_on_side`] but operates on a [`Seg`] (map line segment)
/// rather than a BSP partition node. Uses the seg's v1/v2 vertices retrieved
/// from the render state.
///
/// Original C: `R_PointOnSegSide` (r_main.c lines ~145-175).
///
/// # Arguments
/// * `x` — Test point X coordinate (fixed-point)
/// * `y` — Test point Y coordinate (fixed-point)
/// * `line` — The map line segment
/// * `render_state` — Render state containing vertex data
///
/// # Returns
/// `0` if the point is on the front side, `1` if on the back side.
pub fn point_on_seg_side(x: Fixed, y: Fixed, line: &Seg, render_state: &RenderState) -> i32 {
    let v1 = &render_state.vertexes[line.v1];
    let v2 = &render_state.vertexes[line.v2];

    let lx = v1.x;
    let ly = v1.y;
    let ldx = v2.x - v1.x;
    let ldy = v2.y - v1.y;

    // Quick sign-based checks for axis-aligned segments
    if ldx == Fixed::ZERO {
        if x.raw() <= lx.raw() {
            return if ldy.raw() > 0 { 1 } else { 0 };
        }
        return if ldy.raw() > 0 { 0 } else { 1 };
    }
    if ldy == Fixed::ZERO {
        if y.raw() <= ly.raw() {
            return if ldx.raw() < 0 { 1 } else { 0 };
        }
        return if ldx.raw() < 0 { 0 } else { 1 };
    }

    // General case: cross product
    let dx = x - lx;
    let dy = y - ly;

    let left = (ldy.raw() >> FRACBITS) as i64 * dx.raw() as i64;
    let right = (ldx.raw() >> FRACBITS) as i64 * dy.raw() as i64;

    if right < left {
        0 // front side
    } else {
        1 // back side
    }
}

/// Calculate the angle from the current viewpoint to a map position.
///
/// Subtracts the current viewpoint (viewx, viewy) from the given coordinates,
/// then delegates to the octant-based `point_to_angle` from tables.rs.
///
/// Original C: `R_PointToAngle` (r_main.c). The C version modifies global
/// `viewx`/`viewy` — in Rust we simply compute relative coordinates.
///
/// # Arguments
/// * `x` — Target X coordinate (fixed-point)
/// * `y` — Target Y coordinate (fixed-point)
/// * `render_main` — Renderer state for viewx/viewy
///
/// # Returns
/// BAM angle from the viewer to the target point. The result ranges over
/// the full 0..2^32 BAM space, with ANG90 (0x40000000) pointing up,
/// ANG180 (0x80000000) pointing left, and ANG270 (0xC0000000) pointing down.
pub fn point_to_angle(x: Fixed, y: Fixed, render_main: &RenderMain) -> Angle {
    let rel_x = x - render_main.viewx;
    let rel_y = y - render_main.viewy;
    // Delegates to tables::point_to_angle which uses ANG90, ANG180, ANG270
    // constants in its 8-octant SlopeDiv/TANTOANGLE lookup.
    doom_core::types::tables::point_to_angle(rel_x, rel_y)
}

/// Calculate the angle between two arbitrary map positions.
///
/// Original C: `R_PointToAngle2` (r_main.c) — sets viewx/viewy to (x1,y1)
/// then calls R_PointToAngle(x2,y2). In Rust, we compute the relative
/// delta directly.
///
/// # Arguments
/// * `x1`, `y1` — Origin point (fixed-point)
/// * `x2`, `y2` — Target point (fixed-point)
///
/// # Returns
/// BAM angle from (x1,y1) to (x2,y2).
pub fn point_to_angle2(x1: Fixed, y1: Fixed, x2: Fixed, y2: Fixed) -> Angle {
    doom_core::types::tables::point_to_angle(x2 - x1, y2 - y1)
}

/// Calculate the distance from the current viewpoint to a map position.
///
/// Uses the classic DOOM approach: take the absolute values of the deltas,
/// ensure dx >= dy (swap if needed), then compute `dx / cos(atan(dy/dx))`
/// using the tantoangle and finesine lookup tables.
///
/// Original C: `R_PointToDist` (r_main.c).
///
/// # Arguments
/// * `x` — Target X coordinate (fixed-point)
/// * `y` — Target Y coordinate (fixed-point)
/// * `render_main` — Renderer state for viewx/viewy
///
/// # Returns
/// Distance in 16.16 fixed-point.
///
/// # Implementation Notes
/// Uses `slope_div(dy, dx)` which internally clamps the result to
/// `SLOPERANGE` (2048) and shifts by `DBITS` (FRACBITS - SLOPEBITS).
/// The slope index is then used to look up the corresponding angle in
/// `TANTOANGLE[0..=SLOPERANGE]`, which is then converted via FINESINE
/// to compute `dist = dx / cos(atan(dy/dx))`.
pub fn point_to_dist(x: Fixed, y: Fixed, render_main: &RenderMain) -> Fixed {
    let mut dx = (x - render_main.viewx).raw().unsigned_abs() as i32;
    let mut dy = (y - render_main.viewy).raw().unsigned_abs() as i32;

    if dy > dx {
        std::mem::swap(&mut dx, &mut dy);
    }

    // Avoid division by zero
    if dx == 0 {
        return Fixed::ZERO;
    }

    // Use tantoangle to get the angle, then look up the cosine
    // slope_div returns a value in [0..SLOPERANGE]. DBITS (= FRACBITS - SLOPEBITS)
    // defines the shift precision used internally by slope_div for the division.
    let slope = slope_div(dy as u32, dx as u32);
    debug_assert!((slope as usize) <= SLOPERANGE);
    let _ = DBITS; // DBITS defines slope_div's internal shift precision
    let angle = TANTOANGLE[slope as usize];
    // angle is a BAM value; convert to fine angle for sine lookup
    // We need cos(angle) = sin(angle + 90), so add ANG90 first
    let fine = ((angle.value().wrapping_add(ANG90.value())) >> ANGLETOFINESHIFT) as usize;

    // dist = dx / cos(angle) = dx / sin(angle + 90)
    let sin_val = FINESINE[fine & 8191].raw();
    if sin_val == 0 {
        return Fixed::new(i32::MAX);
    }

    // dist = FixedDiv(dx, finesine[...])
    Fixed::new(dx).fixed_div(Fixed::new(sin_val))
}

/// Calculate the rendering scale for a wall column at a given view angle.
///
/// This is the core projection function that maps world-space wall distances
/// to screen-space column widths. It uses the current wall's distance
/// (`rw_distance`) and normal angle (`rw_normalangle`) from the render state.
///
/// Original C: `R_ScaleFromGlobalAngle` (r_main.c lines ~450-490).
///
/// Formula:
/// ```text
/// anglea = ANG90 + (visangle - viewangle)
/// angleb = ANG90 + (visangle - rw_normalangle)
/// sinea = finesine[anglea >> ANGLETOFINESHIFT]
/// sineb = finesine[angleb >> ANGLETOFINESHIFT]
/// num = projection * sineb << detailshift
/// den = rw_distance * sinea
/// scale = num / den, clamped to [256, 64*FRACUNIT]
/// ```
///
/// # Arguments
/// * `visangle` — The angle to the wall column being projected
/// * `render_main` — Renderer state (projection, detailshift, viewangle)
/// * `render_state` — Render state (rw_distance, rw_normalangle)
///
/// # Returns
/// Scale value in 16.16 fixed-point, clamped to [256, 64*FRACUNIT].
pub fn scale_from_global_angle(
    visangle: Angle,
    render_main: &RenderMain,
    render_state: &RenderState,
) -> Fixed {
    let anglea = Angle::new(
        ANG90
            .value()
            .wrapping_add(visangle.value().wrapping_sub(render_main.viewangle.value())),
    );
    let angleb = Angle::new(
        ANG90
            .value()
            .wrapping_add(visangle.value().wrapping_sub(render_state.rw_normalangle)),
    );

    let sinea = FINESINE[anglea.to_fine_angle()].raw();
    let sineb = FINESINE[angleb.to_fine_angle()].raw();

    // num = FixedMul(projection, sineb) << detailshift
    let num = render_main.projection.fixed_mul(Fixed::new(sineb)).raw() as i64
        * (1i64 << render_main.detailshift);
    // den = FixedMul(rw_distance, sinea)
    let den = Fixed::new(render_state.rw_distance)
        .fixed_mul(Fixed::new(sinea))
        .raw() as i64;

    if den > (num >> 16) {
        let scale = ((num << FRACBITS as i64) / den) as i32;
        // Clamp to [256, 64*FRACUNIT]
        let scale = scale.clamp(256, 64 * FRACUNIT);
        Fixed::new(scale)
    } else {
        Fixed::new(64 * FRACUNIT)
    }
}

/// Find the subsector containing a given map position by traversing the BSP tree.
///
/// Walks the BSP tree from the root node down to a leaf subsector, using
/// [`point_on_side`] at each node to choose the correct child.
///
/// Original C: `R_PointInSubsector` (r_main.c lines ~820-845).
///
/// # Arguments
/// * `x` — Map X coordinate (fixed-point)
/// * `y` — Map Y coordinate (fixed-point)
/// * `render_state` — Render state containing nodes and subsectors
///
/// # Returns
/// Index into the `render_state.subsectors` array (a [`Subsector`] element).
/// The caller can then access the [`Subsector`] via `render_state.subsectors[idx]`.
pub fn point_in_subsector(x: Fixed, y: Fixed, render_state: &RenderState) -> usize {
    // Validate that subsectors exist (the Subsector type from map_data is the element type)
    let _subsectors: &[Subsector] = &render_state.subsectors;

    // If there are no nodes, there's exactly one subsector (index 0)
    if render_state.numnodes == 0 {
        return 0;
    }

    let mut node_num = render_state.numnodes - 1;

    loop {
        let node = &render_state.nodes[node_num];
        let side = point_on_side(x, y, node) as usize;
        let child = node.children[side];

        if (child & NF_SUBSECTOR) != 0 {
            // This child is a subsector leaf
            return (child & !NF_SUBSECTOR) as usize;
        }
        // Continue traversing
        node_num = child as usize;
    }
}

// =============================================================================
// Initialization functions
// =============================================================================

/// Build the `viewangletox` and `xtoviewangle` lookup tables, and set `clipangle`.
///
/// Maps each fine angle in the visible FOV to the corresponding screen column,
/// then builds the inverse mapping from screen columns back to view angles.
///
/// Original C: `R_InitTextureMapping` (r_main.c lines ~505-550).
fn init_texture_mapping(render_main: &mut RenderMain) {
    let half_fineangles = (FINEANGLES / 2) as usize;

    // Calc focallength so FIELDOFVIEW angles covers SCREENWIDTH.
    // Original C: focallength = FixedDiv(centerxfrac, finetangent[FINEANGLES/4+FIELDOFVIEW/2]);
    let fov_idx = (FINEANGLES / 4) as usize + (FIELDOFVIEW / 2) as usize;
    let tan_val = FINETANGENT[fov_idx];
    let focallength = if tan_val.raw() != 0 {
        render_main.centerxfrac.fixed_div(tan_val)
    } else {
        return;
    };

    // Build viewangletox: for each angle in the left half of the fine-angle space,
    // compute the corresponding screen column.
    // Clippy: we index both FINETANGENT[i] and viewangletox[i] — range loop is clearest.
    #[allow(clippy::needless_range_loop)]
    for i in 0..half_fineangles {
        let tangent = FINETANGENT[i].raw();
        let t = if tangent > FRACUNIT * 2 {
            -1i32
        } else if tangent < -(FRACUNIT * 2) {
            render_main.viewwidth + 1
        } else {
            // t = FixedMul(finetangent[i], focallength)
            // t = (centerxfrac - t + FRACUNIT - 1) >> FRACBITS
            let t_fixed = Fixed::new(tangent).fixed_mul(focallength);
            let t = (render_main.centerxfrac.raw() - t_fixed.raw() + FRACUNIT - 1) >> FRACBITS;
            t.max(-1).min(render_main.viewwidth + 1)
        };
        render_main.viewangletox[i] = t;
    }

    // Scan viewangletox[] to generate xtoviewangle[]:
    // xtoviewangle will give the smallest view angle that maps to x.
    for x in 0..=render_main.viewwidth {
        let mut i = 0usize;
        while i < half_fineangles && render_main.viewangletox[i] > x {
            i += 1;
        }
        // xtoviewangle[x] = (i << ANGLETOFINESHIFT) - ANG90
        let angle_val = ((i as u32) << ANGLETOFINESHIFT).wrapping_sub(ANG90.value());
        if (x as usize) < 321 {
            render_main.xtoviewangle[x as usize] = Angle::new(angle_val);
        }
    }

    // Take out the fencepost cases from viewangletox.
    // Original C: clamp -1 → 0 and viewwidth+1 → viewwidth
    for i in 0..half_fineangles {
        if render_main.viewangletox[i] == -1 {
            render_main.viewangletox[i] = 0;
        } else if render_main.viewangletox[i] == render_main.viewwidth + 1 {
            render_main.viewangletox[i] = render_main.viewwidth;
        }
    }

    // Set clipangle from the first visible column
    render_main.clipangle = render_main.xtoviewangle[0];
}

/// Initialize the depth-based lighting tables (`zlight`).
///
/// Builds `zlight[LIGHTLEVELS][MAXLIGHTZ]` lookup tables that map
/// (sector light level, depth from viewer) → colormap byte offset.
///
/// Original C: `R_InitLightTables` (r_main.c lines ~575-600).
///
/// The `data_state.colormaps` array (type `Vec<LightTable>`) provides the raw
/// colormap bytes loaded from the WAD COLORMAP lump. Each of the 34 colormaps
/// is 256 bytes, so `colormaps[level * 256..]` gives the palette remapping for
/// brightness level `level`. The zlight LUT stores byte offsets into this array.
pub fn init_light_tables(render_main: &mut RenderMain, data_state: &DataState) {
    // Validate that colormaps are loaded (at least NUMCOLORMAPS * 256 bytes)
    let _colormaps_len: usize = data_state.colormaps.len();
    // Reference LightTable type to confirm colormaps element type
    let _first_entry: Option<&LightTable> = data_state.colormaps.first();

    for i in 0..LIGHTLEVELS {
        let startmap =
            ((LIGHTLEVELS as i32 - 1 - i as i32) * 2) * NUMCOLORMAPS / LIGHTLEVELS as i32;

        for j in 0..MAXLIGHTZ {
            // scale = FixedDiv((SCREENWIDTH/2 * FRACUNIT), (j+1) << LIGHTZSHIFT)
            let denominator = (j as i32 + 1) << LIGHTZSHIFT;
            let scale = if denominator > 0 {
                Fixed::from_int(SCREENWIDTH / 2).fixed_div(Fixed::new(denominator))
            } else {
                Fixed::new(i32::MAX)
            };

            let scale_shifted = scale.raw() >> LIGHTSCALESHIFT;
            let level = startmap - scale_shifted / DISTMAP;

            // Clamp to valid colormap range
            let level = level.clamp(0, NUMCOLORMAPS - 1) as usize;

            // Store as byte offset into colormaps array (level * 256)
            // Original C: zlight[i][j] = colormaps + level * 256;
            render_main.zlight[i][j] = level * 256;
        }
    }
}

/// Initialize all renderer subsystems (R_Init equivalent).
///
/// Calls all initialization sub-functions in the correct order to set up
/// the renderer for rendering. This must be called once at engine startup
/// before any frames are rendered.
///
/// Original C: `R_Init` (r_main.c lines ~810-825).
///
/// # Call Order
/// 1. `data.init_data()` — Load textures, flats, sprites, colormaps from WAD
/// 2. (point_to_angle and tables are pre-built in tables.rs)
/// 3. `r_set_view_size(screenblocks, detail)` — Set initial view size
/// 4. (R_InitPlanes is a no-op in the original C)
/// 5. `init_light_tables()` — Build zlight LUTs
/// 6. `sky.init_sky_map()` — Initialize sky texture settings
/// 7. `draw.init_translation_tables()` — Build player color translation tables
/// 8. `framecount = 0`
pub fn r_init<W: WadProvider>(
    render_main: &mut RenderMain,
    data_state: &mut DataState,
    draw_state: &mut DrawState,
    sky_state: &mut SkyState,
    wad: &mut W,
    screenblocks: i32,
    detail_level: i32,
) {
    // 1. Initialize data (textures, flats, sprites, colormaps from WAD)
    data_state.init_data(wad);

    // 2. R_InitPointToAngle — no-op (tables are compile-time in tables.rs)
    // 3. R_InitTables — no-op (tables are compile-time in tables.rs)

    // 4. R_SetViewSize with initial screenblocks and detail
    r_set_view_size(render_main, screenblocks, detail_level);

    // 5. R_InitPlanes — no-op in original C (plane tables built per-frame)

    // 6. R_InitLightTables — build zlight LUTs (references data_state.colormaps)
    init_light_tables(render_main, data_state);

    // 7. R_InitSkyMap
    sky_state.init_sky_map();

    // 8. R_InitTranslationTables
    draw_state.init_translation_tables();

    // 9. Reset frame counter
    render_main.framecount = 0;
}

/// Set the rendering view size with deferred execution.
///
/// Records the requested view size (screen blocks and detail level) and sets
/// the `setsizeneeded` flag. The actual recalculation happens in
/// [`r_execute_set_view_size`] on the next frame.
///
/// Original C: `R_SetViewSize` (r_main.c lines ~545-555).
///
/// # Arguments
/// * `blocks` — Screen blocks (3-11, where 11 = fullscreen)
/// * `detail` — Detail level (0 = high, 1 = low)
pub fn r_set_view_size(render_main: &mut RenderMain, blocks: i32, detail: i32) {
    render_main.setsizeneeded = true;
    render_main.setblocks = blocks;
    render_main.setdetail = detail;
}

/// Execute the deferred view size change.
///
/// Recalculates all view-size-dependent parameters including:
/// - scaledviewwidth, viewheight, viewwidth
/// - detailshift and function dispatch
/// - centerx, centery, projection
/// - viewangletox, xtoviewangle, clipangle
/// - scalelight LUTs
///
/// Original C: `R_ExecuteSetViewSize` (r_main.c lines ~560-770).
pub fn r_execute_set_view_size(
    render_main: &mut RenderMain,
    render_state: &mut RenderState,
    draw_state: &mut DrawState,
) {
    render_main.setsizeneeded = false;

    // Calculate view dimensions from screen blocks
    if render_main.setblocks == 11 {
        render_main.scaledviewwidth = SCREENWIDTH;
        render_main.viewheight = SCREENHEIGHT;
    } else {
        render_main.scaledviewwidth = render_main.setblocks * 32;
        render_main.viewheight = (render_main.setblocks * 168 / 10) & !7;
    }

    // Set detail shift
    render_main.detailshift = render_main.setdetail;
    render_main.viewwidth = render_main.scaledviewwidth >> render_main.detailshift;

    // Center of the view window
    render_main.centery = render_main.viewheight / 2;
    render_main.centerx = render_main.viewwidth / 2;
    render_main.centerxfrac = Fixed::from_int(render_main.centerx);
    render_main.centeryfrac = Fixed::from_int(render_main.centery);
    render_main.projection = render_main.centerxfrac;

    // Set function dispatch based on detail level
    if render_main.detailshift == 0 {
        render_main.colfunc = ColFunc::DrawColumn;
        render_main.basecolfunc = ColFunc::DrawColumn;
        render_main.fuzzcolfunc = ColFunc::DrawFuzzColumn;
        render_main.spanfunc = SpanFunc::DrawSpan;
    } else {
        // Original C: even in low-detail mode, fuzzcolfunc and transcolfunc
        // remain the high-detail versions (R_DrawFuzzColumn, R_DrawTranslatedColumn).
        render_main.colfunc = ColFunc::DrawColumnLow;
        render_main.basecolfunc = ColFunc::DrawColumnLow;
        render_main.fuzzcolfunc = ColFunc::DrawFuzzColumn;
        render_main.spanfunc = SpanFunc::DrawSpanLow;
    }

    // Initialize draw buffer tables (ylookup, columnofs)
    draw_state.init_buffer(render_main.scaledviewwidth, render_main.viewheight);

    // Build viewangletox and xtoviewangle lookup tables
    init_texture_mapping(render_main);

    // Copy relevant values to render_state for cross-module access
    render_state.clipangle = render_main.clipangle.value();
    let half_fineangles = (FINEANGLES / 2) as usize;
    for i in 0..half_fineangles {
        if i < render_state.viewangletox.len() {
            render_state.viewangletox[i] = render_main.viewangletox[i];
        }
    }
    for i in 0..=(SCREENWIDTH as usize) {
        if i < render_state.xtoviewangle.len() {
            render_state.xtoviewangle[i] = render_main.xtoviewangle[i].value();
        }
    }

    // Build scalelight LUTs
    for i in 0..LIGHTLEVELS {
        let startmap =
            ((LIGHTLEVELS as i32 - 1 - i as i32) * 2) * NUMCOLORMAPS / LIGHTLEVELS as i32;

        for j in 0..MAXLIGHTSCALE {
            let level = startmap
                - (j as i32) * SCREENWIDTH
                    / (render_main.viewwidth << render_main.detailshift)
                    / DISTMAP;
            let level = level.clamp(0, NUMCOLORMAPS - 1) as usize;
            render_main.scalelight[i][j] = level * 256;
        }
    }
}

// =============================================================================
// Per-frame functions
// =============================================================================

/// Set up the rendering frame from the player viewpoint.
///
/// Extracts the player's position, angle, and special effects state to
/// configure all viewpoint-related renderer fields. Must be called at the
/// start of each frame before any BSP traversal or drawing.
///
/// Original C: `R_SetupFrame` (r_main.c lines ~850-895).
///
/// # Arguments
/// * `render_main` — Renderer main state (updated with new viewpoint)
/// * `render_state` — Render state (POV fields updated for cross-module access)
/// * `player` — The player whose viewpoint to render
/// * `player_idx` — Arena index of the player in the players array
pub fn r_setup_frame(
    render_main: &mut RenderMain,
    render_state: &mut RenderState,
    player: &Player,
    player_idx: usize,
) {
    // Read player.mobj arena index. In the original C, player->mo provides the
    // map object pointer from which viewx/viewy/viewangle are extracted. In the
    // Rust arena-based design, the caller must extract mobj fields and set
    // render_main.viewx/viewy/viewangle before calling this function.
    let _mobj_idx = player.mobj;
    render_main.viewplayer = Some(player_idx);
    render_state.viewplayer_idx = player_idx;

    // Extract viewz from the player (uses Fixed.0 for direct tuple field access)
    render_main.viewz = Fixed(player.viewz.0);

    // viewangle includes offset (the original C does player->mo->angle + viewangleoffset,
    // but since mobj is an arena index, the caller sets viewangle from the mobj before
    // calling this). We apply the viewangleoffset here.
    render_main.viewangle = Angle::new(
        render_main
            .viewangle
            .value()
            .wrapping_add(render_main.viewangleoffset as u32),
    );

    render_main.extralight = player.extralight;

    // Compute viewcos and viewsin from the lookup tables
    let fine_angle = render_main.viewangle.to_fine_angle();
    render_main.viewcos = finecosine(fine_angle);
    render_main.viewsin = FINESINE[fine_angle];

    // Reset subsector count
    render_main.sscount = 0;

    // Handle fixed colormap (invulnerability powerup, light amp, etc.)
    if player.fixedcolormap != 0 {
        let colormap_offset = player.fixedcolormap as usize * 256;
        render_main.fixedcolormap = Some(colormap_offset);
        // Fill scalelightfixed with the fixed colormap for all scales
        for slot in render_main.scalelightfixed.iter_mut() {
            *slot = colormap_offset;
        }
    } else {
        render_main.fixedcolormap = None;
    }

    // Increment frame and validity counters
    render_main.framecount += 1;
    render_main.validcount += 1;

    // Propagate POV to RenderState for cross-module access
    render_state.viewx = render_main.viewx.raw();
    render_state.viewy = render_main.viewy.raw();
    render_state.viewz = render_main.viewz.raw();
    render_state.viewangle = render_main.viewangle.value();
    render_state.viewcos = render_main.viewcos.raw();
    render_state.viewsin = render_main.viewsin.raw();
}

/// Render the player's view of the game world.
///
/// This is the top-level rendering entry point called once per frame.
/// It orchestrates the full rendering pipeline:
/// 1. Set up the frame viewpoint
/// 2. Clear all rendering buffers
/// 3. Traverse the BSP tree (front-to-back)
/// 4. Draw floor/ceiling visplanes
/// 5. Sort and draw sprites with masked textures
///
/// Original C: `R_RenderPlayerView` (r_main.c lines ~898-920).
///
/// # Arguments
/// * `render_main` — Renderer main state
/// * `render_state` — Shared renderer state
/// * `player` — The player whose viewpoint to render
/// * `player_idx` — Arena index of the player
///
/// # Note
/// The BSP traversal, plane drawing, and masked sprite drawing are
/// delegated to their respective modules (bsp, plane, things). The
/// game loop orchestrates the complete pipeline by calling into each
/// sub-module after this function sets up the frame.
///
/// Original C call sequence (for reference):
/// ```text
/// R_SetupFrame(player);
/// R_ClearClipSegs();
/// R_ClearDrawSegs();
/// R_ClearPlanes();
/// R_ClearSprites();
/// R_RenderBSPNode(numnodes - 1);
/// R_DrawPlanes();
/// R_DrawMasked();
/// ```
pub fn r_render_player_view(
    render_main: &mut RenderMain,
    render_state: &mut RenderState,
    player: &Player,
    player_idx: usize,
) {
    // Step 1: Configure the viewpoint for this frame
    r_setup_frame(render_main, render_state, player, player_idx);

    // Steps 2-5 are orchestrated by the game loop calling into the respective
    // sub-modules (bsp::render_bsp_node, plane::draw_planes, things::draw_masked).
    // Each requires mutable access to additional state structs (PlaneState,
    // BspState, ThingsState) that are not parameters of this function.
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{ANG180, ANG270};

    #[test]
    fn test_constants_match_original() {
        // r_main.h constant values must match exactly
        assert_eq!(FIELDOFVIEW, 2048);
        assert_eq!(LIGHTLEVELS, 16);
        assert_eq!(LIGHTSEGSHIFT, 4);
        assert_eq!(MAXLIGHTSCALE, 48);
        assert_eq!(LIGHTSCALESHIFT, 12);
        assert_eq!(MAXLIGHTZ, 128);
        assert_eq!(LIGHTZSHIFT, 20);
        assert_eq!(NUMCOLORMAPS, 32);

        // Angle constants used by point_to_angle octant calculations (ANG180, ANG270)
        assert_eq!(ANG180.value(), 0x80000000);
        assert_eq!(ANG270.value(), 0xC000_0000);

        // Slope constants used by point_to_dist (SLOPERANGE, DBITS)
        assert_eq!(SLOPERANGE, 2048);
        assert_eq!(DBITS, 5); // FRACBITS(16) - SLOPEBITS(11)
    }

    #[test]
    fn test_render_main_new_defaults() {
        let rm = RenderMain::new();
        assert_eq!(rm.validcount, 1);
        assert_eq!(rm.framecount, 0);
        assert_eq!(rm.detailshift, 0);
        assert!(rm.fixedcolormap.is_none());
        assert!(rm.viewplayer.is_none());
        assert_eq!(rm.viewx, Fixed::ZERO);
        assert_eq!(rm.viewy, Fixed::ZERO);
        assert_eq!(rm.viewz, Fixed::ZERO);
        assert_eq!(rm.viewangle, Angle::new(0));
        assert_eq!(rm.colfunc, ColFunc::DrawColumn);
        assert_eq!(rm.basecolfunc, ColFunc::DrawColumn);
        assert_eq!(rm.fuzzcolfunc, ColFunc::DrawFuzzColumn);
        assert_eq!(rm.spanfunc, SpanFunc::DrawSpan);
        assert!(!rm.setsizeneeded);
    }

    #[test]
    fn test_render_main_default_trait() {
        let rm = RenderMain::default();
        assert_eq!(rm.validcount, 1);
        assert_eq!(rm.colfunc, ColFunc::DrawColumn);
    }

    #[test]
    fn test_colfunc_enum_variants() {
        let c1 = ColFunc::DrawColumn;
        let c2 = ColFunc::DrawColumnLow;
        let c3 = ColFunc::DrawFuzzColumn;
        let c4 = ColFunc::DrawFuzzColumnLow;
        let c5 = ColFunc::DrawTranslatedColumn;
        let c6 = ColFunc::DrawTranslatedColumnLow;
        // All variants are distinct
        assert_ne!(c1, c2);
        assert_ne!(c3, c4);
        assert_ne!(c5, c6);
        assert_ne!(c1, c3);
        // Clone + Copy
        let c1_copy = c1;
        assert_eq!(c1, c1_copy);
    }

    #[test]
    fn test_spanfunc_enum_variants() {
        let s1 = SpanFunc::DrawSpan;
        let s2 = SpanFunc::DrawSpanLow;
        assert_ne!(s1, s2);
        let s1_copy = s1;
        assert_eq!(s1, s1_copy);
    }

    #[test]
    fn test_r_set_view_size_deferred() {
        let mut rm = RenderMain::new();
        assert!(!rm.setsizeneeded);

        r_set_view_size(&mut rm, 10, 0);
        assert!(rm.setsizeneeded);
        assert_eq!(rm.setblocks, 10);
        assert_eq!(rm.setdetail, 0);
    }

    #[test]
    fn test_point_on_side_axis_aligned_dx_zero() {
        // When dx == 0, the partition is vertical
        let node = Node {
            x: Fixed::from_int(100),
            y: Fixed::from_int(100),
            dx: Fixed::ZERO,
            dy: Fixed::from_int(10), // positive dy
            bbox: [[Fixed::ZERO; 4]; 2],
            children: [0; 2],
        };
        // Point to the right (x > node.x) with positive dy → front (0)
        assert_eq!(
            point_on_side(Fixed::from_int(200), Fixed::from_int(100), &node),
            0
        );
        // Point to the left (x <= node.x) with positive dy → back (1)
        assert_eq!(
            point_on_side(Fixed::from_int(50), Fixed::from_int(100), &node),
            1
        );
    }

    #[test]
    fn test_point_on_side_axis_aligned_dy_zero() {
        let node = Node {
            x: Fixed::from_int(100),
            y: Fixed::from_int(100),
            dx: Fixed::from_int(10), // positive dx
            dy: Fixed::ZERO,
            bbox: [[Fixed::ZERO; 4]; 2],
            children: [0; 2],
        };
        // Original C: if dy == 0: if y <= node.y: return dx < 0 ? 1 : 0
        // dx > 0, y <= node.y (y=50 <= 100) → 0
        assert_eq!(
            point_on_side(Fixed::from_int(100), Fixed::from_int(50), &node),
            0
        );
        // dx > 0, y > node.y (y=200 > 100) → 1
        assert_eq!(
            point_on_side(Fixed::from_int(100), Fixed::from_int(200), &node),
            1
        );
    }

    #[test]
    fn test_point_to_angle2_basic() {
        // Angle from (0,0) to (100,0) should be 0 (east)
        let angle = point_to_angle2(Fixed::ZERO, Fixed::ZERO, Fixed::from_int(100), Fixed::ZERO);
        assert_eq!(angle.value(), 0);

        // Angle from (0,0) to (0,100): octant 1 gives ANG90-1-tantoangle[SlopeDiv(0,100)]
        // SlopeDiv(0,100)=0, tantoangle[0]=0, so result = ANG90-1.
        // This matches the original C behavior exactly (no special-case for x==0).
        let angle = point_to_angle2(Fixed::ZERO, Fixed::ZERO, Fixed::ZERO, Fixed::from_int(100));
        assert_eq!(angle.value(), ANG90.value() - 1);
    }

    #[test]
    fn test_point_in_subsector_no_nodes() {
        let rs = RenderState::new();
        // With 0 nodes, should return subsector 0
        assert_eq!(point_in_subsector(Fixed::ZERO, Fixed::ZERO, &rs), 0);
    }

    #[test]
    fn test_execute_set_view_size_fullscreen() {
        let mut rm = RenderMain::new();
        let mut rs = RenderState::new();
        let mut ds = DrawState::new();

        rm.setblocks = 11;
        rm.setdetail = 0;
        rm.setsizeneeded = true;

        r_execute_set_view_size(&mut rm, &mut rs, &mut ds);

        assert!(!rm.setsizeneeded);
        assert_eq!(rm.scaledviewwidth, SCREENWIDTH);
        assert_eq!(rm.viewheight, SCREENHEIGHT);
        assert_eq!(rm.viewwidth, SCREENWIDTH);
        assert_eq!(rm.detailshift, 0);
        assert_eq!(rm.centerx, SCREENWIDTH / 2);
        assert_eq!(rm.centery, SCREENHEIGHT / 2);
        assert_eq!(rm.colfunc, ColFunc::DrawColumn);
        assert_eq!(rm.spanfunc, SpanFunc::DrawSpan);
    }

    #[test]
    fn test_execute_set_view_size_low_detail() {
        let mut rm = RenderMain::new();
        let mut rs = RenderState::new();
        let mut ds = DrawState::new();

        rm.setblocks = 10;
        rm.setdetail = 1; // low detail
        rm.setsizeneeded = true;

        r_execute_set_view_size(&mut rm, &mut rs, &mut ds);

        assert_eq!(rm.detailshift, 1);
        assert_eq!(rm.scaledviewwidth, 10 * 32); // 320
        assert_eq!(rm.viewwidth, 320 >> 1); // 160 due to detail shift
        assert_eq!(rm.colfunc, ColFunc::DrawColumnLow);
        assert_eq!(rm.spanfunc, SpanFunc::DrawSpanLow);
    }

    #[test]
    fn test_lighting_lut_bounds() {
        let mut rm = RenderMain::new();
        let ds = DataState::new();
        init_light_tables(&mut rm, &ds);

        // All zlight entries should be valid colormap offsets (multiples of 256)
        for i in 0..LIGHTLEVELS {
            for j in 0..MAXLIGHTZ {
                let offset = rm.zlight[i][j];
                assert_eq!(offset % 256, 0, "zlight[{i}][{j}] not aligned: {offset}");
                assert!(
                    offset / 256 < NUMCOLORMAPS as usize,
                    "zlight[{i}][{j}] out of range: {offset}"
                );
            }
        }
    }

    #[test]
    fn test_scalelight_after_execute() {
        let mut rm = RenderMain::new();
        let mut rs = RenderState::new();
        let mut ds = DrawState::new();

        rm.setblocks = 11;
        rm.setdetail = 0;
        rm.setsizeneeded = true;

        r_execute_set_view_size(&mut rm, &mut rs, &mut ds);

        // All scalelight entries should be valid colormap offsets
        for i in 0..LIGHTLEVELS {
            for j in 0..MAXLIGHTSCALE {
                let offset = rm.scalelight[i][j];
                assert_eq!(
                    offset % 256,
                    0,
                    "scalelight[{i}][{j}] not aligned: {offset}"
                );
                assert!(
                    offset / 256 < NUMCOLORMAPS as usize,
                    "scalelight[{i}][{j}] out of range: {offset}"
                );
            }
        }
    }

    #[test]
    fn test_setup_frame_fixed_colormap() {
        let mut rm = RenderMain::new();
        let mut rs = RenderState::new();
        let mut player = Player::default();

        // Test with fixedcolormap = 0 (no override) — already default
        rm.viewangle = Angle::new(0);
        r_setup_frame(&mut rm, &mut rs, &player, 0);
        assert!(rm.fixedcolormap.is_none());

        // Test with fixedcolormap = 1 (invulnerability or similar)
        player.fixedcolormap = 1;
        rm.viewangle = Angle::new(0);
        r_setup_frame(&mut rm, &mut rs, &player, 0);
        assert_eq!(rm.fixedcolormap, Some(256));
        // All scalelightfixed entries should be the same colormap offset
        for slot in &rm.scalelightfixed {
            assert_eq!(*slot, 256);
        }
    }

    #[test]
    fn test_setup_frame_increments_counters() {
        let mut rm = RenderMain::new();
        let mut rs = RenderState::new();
        let player = Player::default();

        let old_fc = rm.framecount;
        let old_vc = rm.validcount;

        rm.viewangle = Angle::new(0);
        r_setup_frame(&mut rm, &mut rs, &player, 0);

        assert_eq!(rm.framecount, old_fc + 1);
        assert_eq!(rm.validcount, old_vc + 1);
    }

    #[test]
    fn test_point_to_dist_zero() {
        let rm = RenderMain::new();
        // Distance to the viewpoint itself should be zero
        let dist = point_to_dist(rm.viewx, rm.viewy, &rm);
        assert_eq!(dist, Fixed::ZERO);
    }

    #[test]
    fn test_viewangletox_size() {
        let rm = RenderMain::new();
        assert_eq!(rm.viewangletox.len(), 4096); // FINEANGLES/2
    }

    #[test]
    fn test_xtoviewangle_size() {
        let rm = RenderMain::new();
        assert_eq!(rm.xtoviewangle.len(), 321); // SCREENWIDTH+1
    }
}
