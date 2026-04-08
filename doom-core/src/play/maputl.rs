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

//! Translated from linuxdoom-1.10/p_maputl.c
//!
//! Movement/collision utility functions, as used by functions in p_map.c.
//! BLOCKMAP iterator functions, and some PIT_* functions to use for iteration.
//!
//! This module provides:
//! - Approximate distance calculation ([`p_aprox_distance`])
//! - Point-on-line-side determination ([`p_point_on_line_side`],
//!   [`p_point_on_divline_side`])
//! - Bounding box vs line tests ([`p_box_on_line_side`])
//! - Divline creation and intercept vector calculation
//!   ([`p_make_divline`], [`p_intercept_vector`])
//! - Line opening computation ([`p_line_opening`])
//! - Thing position linking/unlinking ([`p_unset_thing_position`],
//!   [`p_set_thing_position`])
//! - Blockmap iteration ([`p_block_lines_iterator`],
//!   [`p_block_things_iterator`])
//! - Intercept collection and traversal ([`pit_add_line_intercepts`],
//!   [`pit_add_thing_intercepts`], [`p_traverse_intercepts`],
//!   [`p_path_traverse`])
//!
//! # State Management
//!
//! All formerly-global mutable state (intercept buffer, trace divline,
//! line opening results, early-out flag, path-traverse flags) is consolidated
//! into the [`MapUtilState`] struct. This struct is passed by mutable reference
//! through the call chain, matching the AAP §0.7.5 mandate to consolidate
//! global state into structs. No `static mut` globals are used.

use crate::types::doomtype::MAXINT;
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::map_data::{LineDef, Sector, SlopeType, Subsector, Vertex};
use crate::types::mobj::{MapObject, MobjFlags};
use crate::util::bbox::{BBox, BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};

// ==========================================================================
// Constants
// ==========================================================================

/// Maximum number of intercepts in the buffer.
/// Original C: `#define MAXINTERCEPTS 128` (p_local.h)
pub const MAXINTERCEPTS: usize = 128;

/// Path traverse flag: add line intercepts to the intercept list.
/// Original C: `#define PT_ADDLINES 1` (p_local.h)
pub const PT_ADDLINES: i32 = 1;

/// Path traverse flag: add thing intercepts to the intercept list.
/// Original C: `#define PT_ADDTHINGS 2` (p_local.h)
pub const PT_ADDTHINGS: i32 = 2;

/// Path traverse flag: enable early-out for solid line hits.
/// Original C: `#define PT_EARLYOUT 4` (p_local.h)
pub const PT_EARLYOUT: i32 = 4;

/// Blockmap cell size in fixed-point (128 map units).
/// Original C: `#define MAPBLOCKSIZE (MAPBLOCKUNITS*FRACUNIT)` (p_local.h)
const MAPBLOCKSIZE: i32 = 128 * FRACUNIT;

/// Shift to convert fixed-point map coordinates to blockmap cell indices.
/// `FRACBITS + 7 = 23` because each cell is 128 (2^7) map units.
/// Original C: `#define MAPBLOCKSHIFT (FRACBITS+7)` (p_local.h)
const MAPBLOCKSHIFT: i32 = FRACBITS + 7;

/// Shift to convert blockmap coordinates to fractional position within a block.
/// `MAPBLOCKSHIFT - FRACBITS = 7`.
/// Original C: `#define MAPBTOFRAC (MAPBLOCKSHIFT-FRACBITS)` (p_local.h)
const MAPBTOFRAC: i32 = MAPBLOCKSHIFT - FRACBITS;

// ==========================================================================
// Types
// ==========================================================================

/// Parametric dividing line segment.
///
/// Used for intercept calculations in [`p_intercept_vector`] and trace
/// operations. The line runs from `(x, y)` in direction `(dx, dy)`.
///
/// Original C: `divline_t` (p_local.h)
#[derive(Debug, Clone, Copy, Default)]
pub struct Divline {
    /// Start X coordinate (16.16 fixed-point).
    pub x: Fixed,
    /// Start Y coordinate (16.16 fixed-point).
    pub y: Fixed,
    /// Direction X component (16.16 fixed-point).
    pub dx: Fixed,
    /// Direction Y component (16.16 fixed-point).
    pub dy: Fixed,
}

/// Data associated with an intercept — either a line or a thing.
///
/// Rust equivalent of the C union `d` inside `intercept_t`. In the
/// original C, this was a union of `line_t*` and `mobj_t*`; in Rust
/// we use an enum with arena indices instead of raw pointers.
#[derive(Debug, Clone, Copy)]
pub enum InterceptData {
    /// Arena index of the intercepted line in the lines array.
    Line(usize),
    /// Arena index of the intercepted thing in the mobj arena.
    Thing(usize),
}

impl Default for InterceptData {
    fn default() -> Self {
        InterceptData::Line(0)
    }
}

/// An intercept record — represents a point where a trace ray crosses
/// a line or thing bounding box.
///
/// Original C: `intercept_t` (p_local.h)
#[derive(Debug, Clone, Copy, Default)]
pub struct Intercept {
    /// Fractional distance along the trace line (0 = start, FRACUNIT = end).
    pub frac: Fixed,
    /// True if this intercept is a line; false if it's a thing.
    pub is_a_line: bool,
    /// The intercepted object data (line index or thing index).
    pub d: InterceptData,
}

/// Type alias for intercept traverser callback function.
///
/// Called by [`p_traverse_intercepts`] for each intercept in sorted order.
/// Returns `true` to continue traversal, `false` to stop.
///
/// Original C: `typedef boolean (*traverser_t)(intercept_t*)`
#[allow(non_camel_case_types)]
pub type traverser_t = fn(&Intercept) -> bool;

// ==========================================================================
// Consolidated State (AAP §0.7.5 — no static mut)
// ==========================================================================

/// Consolidated map utility state — replaces all formerly-global mutable
/// variables from the original C `p_maputl.c`.
///
/// This struct holds the intercept buffer, trace line, line-opening results,
/// early-out flag, and path-traverse flags. It is passed by mutable reference
/// through the collision/movement call chain.
///
/// Original C globals consolidated:
/// - `intercept_t intercepts[MAXINTERCEPTS]` (p_maputl.c:544)
/// - `intercept_t* intercept_p` (p_maputl.c:545)
/// - `divline_t trace` (p_maputl.c:547)
/// - `boolean earlyout` (p_maputl.c:548)
/// - `int ptflags` (p_maputl.c:549)
/// - `fixed_t opentop` (p_maputl.c:294)
/// - `fixed_t openbottom` (p_maputl.c:295)
/// - `fixed_t openrange` (p_maputl.c:296)
/// - `fixed_t lowfloor` (p_maputl.c:297)
#[derive(Debug, Clone)]
pub struct MapUtilState {
    /// Intercept buffer — holds all intercepts found during a path traverse.
    /// Original C: `intercept_t intercepts[MAXINTERCEPTS]` (p_maputl.c:544)
    pub intercepts: [Intercept; MAXINTERCEPTS],

    /// Next free index in the intercepts array (pointer equivalent).
    /// Original C: `intercept_t* intercept_p` (p_maputl.c:545)
    pub intercept_p: usize,

    /// The trace line for current intercept operations.
    /// Original C: `divline_t trace` (p_maputl.c:547)
    pub trace: Divline,

    /// Early-out flag: if set and a solid line is hit before FRACUNIT, stop.
    /// Original C: `boolean earlyout` (p_maputl.c:548)
    pub earlyout: bool,

    /// Path traverse flags (PT_ADDLINES | PT_ADDTHINGS | PT_EARLYOUT).
    /// Original C: `int ptflags` (p_maputl.c:549)
    pub ptflags: i32,

    /// Top of the opening through a two-sided line (set by [`p_line_opening`]).
    /// Original C: `fixed_t opentop` (p_maputl.c:294)
    pub opentop: Fixed,

    /// Bottom of the opening through a two-sided line (set by [`p_line_opening`]).
    /// Original C: `fixed_t openbottom` (p_maputl.c:295)
    pub openbottom: Fixed,

    /// Size of the opening: opentop - openbottom (set by [`p_line_opening`]).
    /// Original C: `fixed_t openrange` (p_maputl.c:296)
    pub openrange: Fixed,

    /// Lowest floor on either side of the line (set by [`p_line_opening`]).
    /// Original C: `fixed_t lowfloor` (p_maputl.c:297)
    pub lowfloor: Fixed,
}

impl Default for MapUtilState {
    fn default() -> Self {
        Self {
            intercepts: [Intercept::default(); MAXINTERCEPTS],
            intercept_p: 0,
            trace: Divline::default(),
            earlyout: false,
            ptflags: 0,
            opentop: Fixed::ZERO,
            openbottom: Fixed::ZERO,
            openrange: Fixed::ZERO,
            lowfloor: Fixed::ZERO,
        }
    }
}

impl MapUtilState {
    /// Creates a new `MapUtilState` with all fields zeroed/defaulted.
    pub fn new() -> Self {
        Self::default()
    }
}

// ==========================================================================
// P_AproxDistance (p_maputl.c lines 48-58)
// ==========================================================================

/// Approximate distance calculation using octagonal estimation.
///
/// Returns an approximation of `sqrt(dx² + dy²)` within ~4% error.
/// This is much faster than true Euclidean distance and is used throughout
/// the engine for sound attenuation, sight checks, and pursuit distances.
///
/// Original C: `P_AproxDistance` (p_maputl.c:48-58)
pub fn p_aprox_distance(dx: Fixed, dy: Fixed) -> Fixed {
    let adx = Fixed(dx.0.wrapping_abs());
    let ady = Fixed(dy.0.wrapping_abs());
    if adx < ady {
        adx + ady - Fixed(adx.0 >> 1)
    } else {
        adx + ady - Fixed(ady.0 >> 1)
    }
}

// ==========================================================================
// P_PointOnLineSide (p_maputl.c lines 65-100)
// ==========================================================================

/// Determine which side of a line a point is on.
///
/// Returns `0` for the front side (right side when facing from v1 to v2)
/// and `1` for the back side.
///
/// Uses axis-aligned fast paths for horizontal/vertical lines, falling
/// back to a cross-product test for the general case.
///
/// Original C: `P_PointOnLineSide` (p_maputl.c:65-100)
pub fn p_point_on_line_side(x: Fixed, y: Fixed, line: &LineDef, vertexes: &[Vertex]) -> i32 {
    let v1 = &vertexes[line.v1];

    // Vertical line fast path (dx == 0)
    if line.dx == Fixed::ZERO {
        if x <= v1.x {
            return (line.dy > Fixed::ZERO) as i32;
        }
        return (line.dy < Fixed::ZERO) as i32;
    }

    // Horizontal line fast path (dy == 0)
    if line.dy == Fixed::ZERO {
        if y <= v1.y {
            return (line.dx < Fixed::ZERO) as i32;
        }
        return (line.dx > Fixed::ZERO) as i32;
    }

    // General case: cross-product test
    let dx = x - v1.x;
    let dy = y - v1.y;

    // left = (line.dy >> FRACBITS) * dx   (via FixedMul)
    // right = dy * (line.dx >> FRACBITS)  (via FixedMul)
    let left = Fixed(line.dy.0 >> FRACBITS).fixed_mul(dx);
    let right = dy.fixed_mul(Fixed(line.dx.0 >> FRACBITS));

    if right < left {
        0 // front side
    } else {
        1 // back side
    }
}

// ==========================================================================
// P_BoxOnLineSide (p_maputl.c lines 109-153)
// ==========================================================================

/// Test which side of a line a bounding box is on.
///
/// Returns `0` (front), `1` (back), or `-1` (box crosses the line).
/// The line is considered infinite for this test.
///
/// Uses the line's slope type to select optimized corner tests for each
/// of the four slope categories.
///
/// Original C: `P_BoxOnLineSide` (p_maputl.c:109-153)
pub fn p_box_on_line_side(tmbox: &BBox, ld: &LineDef, vertexes: &[Vertex]) -> i32 {
    let v1 = &vertexes[ld.v1];

    let (p1, p2) = match ld.slopetype {
        SlopeType::Horizontal => {
            let mut p1 = (tmbox[BOXTOP] > v1.y) as i32;
            let mut p2 = (tmbox[BOXBOTTOM] > v1.y) as i32;
            if ld.dx < Fixed::ZERO {
                p1 ^= 1;
                p2 ^= 1;
            }
            (p1, p2)
        }
        SlopeType::Vertical => {
            let mut p1 = (tmbox[BOXRIGHT] < v1.x) as i32;
            let mut p2 = (tmbox[BOXLEFT] < v1.x) as i32;
            if ld.dy < Fixed::ZERO {
                p1 ^= 1;
                p2 ^= 1;
            }
            (p1, p2)
        }
        SlopeType::Positive => {
            let p1 = p_point_on_line_side(tmbox[BOXLEFT], tmbox[BOXTOP], ld, vertexes);
            let p2 = p_point_on_line_side(tmbox[BOXRIGHT], tmbox[BOXBOTTOM], ld, vertexes);
            (p1, p2)
        }
        SlopeType::Negative => {
            let p1 = p_point_on_line_side(tmbox[BOXRIGHT], tmbox[BOXTOP], ld, vertexes);
            let p2 = p_point_on_line_side(tmbox[BOXLEFT], tmbox[BOXBOTTOM], ld, vertexes);
            (p1, p2)
        }
    };

    if p1 == p2 {
        p1
    } else {
        -1
    }
}

// ==========================================================================
// P_PointOnDivlineSide (p_maputl.c lines 160-203)
// ==========================================================================

/// Determine which side of a divline a point is on.
///
/// Returns `0` for the front side and `1` for the back side.
/// Similar to [`p_point_on_line_side`] but operates on a [`Divline`]
/// instead of a [`LineDef`], and uses a sign-bit optimization plus
/// `>>8` precision shifts (instead of `>>FRACBITS`) for the general case.
///
/// Original C: `P_PointOnDivlineSide` (p_maputl.c:160-203)
pub fn p_point_on_divline_side(x: Fixed, y: Fixed, line: &Divline) -> i32 {
    // Vertical divline fast path (dx == 0)
    if line.dx == Fixed::ZERO {
        if x <= line.x {
            return (line.dy > Fixed::ZERO) as i32;
        }
        return (line.dy < Fixed::ZERO) as i32;
    }

    // Horizontal divline fast path (dy == 0)
    if line.dy == Fixed::ZERO {
        if y <= line.y {
            return (line.dx < Fixed::ZERO) as i32;
        }
        return (line.dx > Fixed::ZERO) as i32;
    }

    let dx = x - line.x;
    let dy = y - line.y;

    // Quick rejection using sign-bit XOR (p_maputl.c lines 190-195).
    // If the XOR of all four sign bits has odd parity, we can determine
    // the side from just the sign of (line.dy ^ dx).
    if (line.dy.0 ^ line.dx.0 ^ dx.0 ^ dy.0) < 0 {
        if (line.dy.0 ^ dx.0) < 0 {
            return 1; // left side is negative → back
        }
        return 0; // front
    }

    // General case with >>8 precision (NOT >>FRACBITS).
    // This provides more precision than P_PointOnLineSide's >>16 shifts.
    let left = Fixed(line.dy.0 >> 8).fixed_mul(Fixed(dx.0 >> 8));
    let right = Fixed(dy.0 >> 8).fixed_mul(Fixed(line.dx.0 >> 8));

    if right < left {
        0 // front side
    } else {
        1 // back side
    }
}

// ==========================================================================
// P_MakeDivline (p_maputl.c lines 210-219)
// ==========================================================================

/// Convert a [`LineDef`] into a [`Divline`].
///
/// Copies the line's start vertex position and direction deltas into
/// a parametric divline representation.
///
/// Original C: `P_MakeDivline` (p_maputl.c:210-219)
pub fn p_make_divline(li: &LineDef, vertexes: &[Vertex]) -> Divline {
    let v1 = &vertexes[li.v1];
    Divline {
        x: v1.x,
        y: v1.y,
        dx: li.dx,
        dy: li.dy,
    }
}

// ==========================================================================
// P_InterceptVector (p_maputl.c lines 230-285)
// ==========================================================================

/// Compute the fractional intercept point along the first divline (`v2`)
/// where the second divline (`v1`) crosses it.
///
/// Uses `>>8` precision shifts (not `>>16`) for the intermediate
/// calculations to maintain accuracy. Returns `Fixed::ZERO` if the
/// lines are parallel (denominator is zero).
///
/// Only the `#if 1` (active) branch from the original C is translated;
/// the `#else` float debug branch is not included.
///
/// Original C: `P_InterceptVector` (p_maputl.c:230-252)
pub fn p_intercept_vector(v2: &Divline, v1: &Divline) -> Fixed {
    let den = Fixed(v1.dy.0 >> 8).fixed_mul(Fixed(v2.dx.0))
        - Fixed(v1.dx.0 >> 8).fixed_mul(Fixed(v2.dy.0));

    if den == Fixed::ZERO {
        return Fixed::ZERO; // parallel lines
    }

    let num = Fixed((v1.x.0 - v2.x.0) >> 8).fixed_mul(v1.dy)
        + Fixed((v2.y.0 - v1.y.0) >> 8).fixed_mul(v1.dx);

    num.fixed_div(den)
}

// ==========================================================================
// P_LineOpening (p_maputl.c lines 300-332)
// ==========================================================================

/// Compute the vertical opening through a two-sided line.
///
/// Sets the global variables [`opentop`], [`openbottom`], [`openrange`],
/// and [`lowfloor`]. For single-sided lines, sets `openrange = 0` and
/// returns immediately.
///
/// # Safety
///
/// Compute the opening through a two-sided line, storing results in
/// `state.opentop`, `state.openbottom`, `state.openrange`, `state.lowfloor`.
///
/// Original C: `P_LineOpening` (p_maputl.c:300-332)
pub fn p_line_opening(state: &mut MapUtilState, linedef: &LineDef, sectors: &[Sector]) {
    // Single-sided line — no opening
    if linedef.sidenum[1] == -1 {
        state.openrange = Fixed::ZERO;
        return;
    }

    let front = &sectors[linedef
        .frontsector
        .expect("two-sided line must have frontsector")];
    let back = &sectors[linedef
        .backsector
        .expect("two-sided line must have backsector")];

    // Top of opening: minimum of both ceilings
    if front.ceilingheight < back.ceilingheight {
        state.opentop = front.ceilingheight;
    } else {
        state.opentop = back.ceilingheight;
    }

    // Bottom of opening and lowest floor
    if front.floorheight > back.floorheight {
        state.openbottom = front.floorheight;
        state.lowfloor = back.floorheight;
    } else {
        state.openbottom = back.floorheight;
        state.lowfloor = front.floorheight;
    }

    state.openrange = state.opentop - state.openbottom;
}

// ==========================================================================
// P_UnsetThingPosition (p_maputl.c lines 347-386)
// ==========================================================================

/// Unlink a thing from sector thinglist and blockmap lists.
///
/// Called before each position change to remove the thing from the
/// spatial data structures. After the move, [`p_set_thing_position`]
/// re-links it at the new location.
///
/// # Parameters
///
/// - `thing_idx`: Arena index of the mobj to unlink.
/// - `mobjs`: Mutable mobj arena (linked-list pointers are updated).
/// - `sectors`: Mutable sector array (thinglist head may change).
/// - `subsectors`: Subsector array for sector lookup.
/// - `blocklinks`: Blockmap thing heads (may change if thing is head).
/// - `bmaporgx`, `bmaporgy`: Blockmap origin in fixed-point.
/// - `bmapwidth`, `bmapheight`: Blockmap dimensions in cells.
///
/// Original C: `P_UnsetThingPosition` (p_maputl.c:347-386)
pub fn p_unset_thing_position(
    thing_idx: usize,
    mobjs: &mut [MapObject],
    sectors: &mut [Sector],
    subsectors: &[Subsector],
    blocklinks: &mut [Option<usize>],
    bmaporgx: Fixed,
    bmaporgy: Fixed,
    bmapwidth: i32,
    bmapheight: i32,
) {
    // Copy fields we need to avoid borrow conflicts during linked-list updates
    let flags = mobjs[thing_idx].flags;
    let snext = mobjs[thing_idx].snext;
    let sprev = mobjs[thing_idx].sprev;
    let bnext = mobjs[thing_idx].bnext;
    let bprev = mobjs[thing_idx].bprev;
    let subsector_opt = mobjs[thing_idx].subsector;
    let x = mobjs[thing_idx].x;
    let y = mobjs[thing_idx].y;

    // Unlink from sector thinglist
    if !flags.contains(MobjFlags::MF_NOSECTOR) {
        if let Some(snext_idx) = snext {
            mobjs[snext_idx].sprev = sprev;
        }
        if let Some(sprev_idx) = sprev {
            mobjs[sprev_idx].snext = snext;
        } else if let Some(ss_idx) = subsector_opt {
            // Thing was head of the sector's thinglist
            let sec_idx = subsectors[ss_idx].sector;
            sectors[sec_idx].thinglist = snext;
        }
    }

    // Unlink from blockmap
    if !flags.contains(MobjFlags::MF_NOBLOCKMAP) {
        if let Some(bnext_idx) = bnext {
            mobjs[bnext_idx].bprev = bprev;
        }
        if let Some(bprev_idx) = bprev {
            mobjs[bprev_idx].bnext = bnext;
        } else {
            // Thing was head of the blockmap chain — update blocklinks
            let blockx = (x.0 - bmaporgx.0) >> MAPBLOCKSHIFT;
            let blocky = (y.0 - bmaporgy.0) >> MAPBLOCKSHIFT;
            if blockx >= 0 && blockx < bmapwidth && blocky >= 0 && blocky < bmapheight {
                let idx = (blocky * bmapwidth + blockx) as usize;
                blocklinks[idx] = bnext;
            }
        }
    }
}

// ==========================================================================
// P_SetThingPosition (p_maputl.c lines 395-450)
// ==========================================================================

/// Link a thing into subsector, sector thinglist, and blockmap based on
/// its `(x, y)` position. Sets `thing.subsector` to the correct subsector.
///
/// # Parameters
///
/// - `thing_idx`: Arena index of the mobj to link.
/// - `mobjs`: Mutable mobj arena.
/// - `sectors`: Mutable sector array (thinglist head may change).
/// - `subsectors`: Subsector array for sector lookup.
/// - `blocklinks`: Blockmap thing heads (may change).
/// - `bmaporgx`, `bmaporgy`: Blockmap origin in fixed-point.
/// - `bmapwidth`, `bmapheight`: Blockmap dimensions in cells.
/// - `point_in_subsector`: Function returning the subsector index for a
///   given (x, y) position (equivalent to `R_PointInSubsector`).
///
/// Original C: `P_SetThingPosition` (p_maputl.c:395-450)
pub fn p_set_thing_position(
    thing_idx: usize,
    mobjs: &mut [MapObject],
    sectors: &mut [Sector],
    subsectors: &[Subsector],
    blocklinks: &mut [Option<usize>],
    bmaporgx: Fixed,
    bmaporgy: Fixed,
    bmapwidth: i32,
    bmapheight: i32,
    point_in_subsector: impl Fn(Fixed, Fixed) -> usize,
) {
    let x = mobjs[thing_idx].x;
    let y = mobjs[thing_idx].y;
    let flags = mobjs[thing_idx].flags;

    // Link into subsector
    let ss_idx = point_in_subsector(x, y);
    mobjs[thing_idx].subsector = Some(ss_idx);

    // Link into sector thinglist (insert at head)
    if !flags.contains(MobjFlags::MF_NOSECTOR) {
        let sec_idx = subsectors[ss_idx].sector;

        mobjs[thing_idx].sprev = None;
        let old_head = sectors[sec_idx].thinglist;
        mobjs[thing_idx].snext = old_head;

        if let Some(old_head_idx) = old_head {
            mobjs[old_head_idx].sprev = Some(thing_idx);
        }

        sectors[sec_idx].thinglist = Some(thing_idx);
    }

    // Link into blockmap (insert at head)
    if !flags.contains(MobjFlags::MF_NOBLOCKMAP) {
        let blockx = (x.0 - bmaporgx.0) >> MAPBLOCKSHIFT;
        let blocky = (y.0 - bmaporgy.0) >> MAPBLOCKSHIFT;

        if blockx >= 0 && blockx < bmapwidth && blocky >= 0 && blocky < bmapheight {
            let idx = (blocky * bmapwidth + blockx) as usize;

            mobjs[thing_idx].bprev = None;
            let old_head = blocklinks[idx];
            mobjs[thing_idx].bnext = old_head;

            if let Some(old_head_idx) = old_head {
                mobjs[old_head_idx].bprev = Some(thing_idx);
            }

            blocklinks[idx] = Some(thing_idx);
        } else {
            // Thing is off the map
            mobjs[thing_idx].bnext = None;
            mobjs[thing_idx].bprev = None;
        }
    }
}

// ==========================================================================
// P_BlockLinesIterator (p_maputl.c lines 471-506)
// ==========================================================================

/// Iterate over all lines in a single blockmap cell.
///
/// Uses `validcount` to prevent processing the same line twice when it
/// spans multiple blockmap cells. Returns `false` if `func` returns
/// `false` for any line (early termination), otherwise returns `true`.
///
/// The callback receives `(line_index, line_data_copy)`. The line data
/// is passed by value (Copy) because the iterator holds a mutable borrow
/// on `lines` for `validcount` updates.
///
/// Original C: `P_BlockLinesIterator` (p_maputl.c:471-506)
pub fn p_block_lines_iterator<F>(
    x: i32,
    y: i32,
    blockmap: &[i16],
    blockmaplump: &[i16],
    lines: &mut [LineDef],
    bmapwidth: i32,
    bmapheight: i32,
    validcount: i32,
    func: &mut F,
) -> bool
where
    F: FnMut(usize, LineDef) -> bool,
{
    // Bounds check
    if x < 0 || y < 0 || x >= bmapwidth || y >= bmapheight {
        return true;
    }

    let table_idx = (y * bmapwidth + x) as usize;
    let offset = blockmap[table_idx] as usize;

    // Walk the line list until the -1 sentinel
    let mut list_idx = offset;
    while blockmaplump[list_idx] != -1 {
        let line_num = blockmaplump[list_idx] as usize;
        list_idx += 1;

        // Skip lines already checked this frame
        if lines[line_num].validcount == validcount {
            continue;
        }
        lines[line_num].validcount = validcount;

        // Copy line data (LineDef is Copy) to pass to callback
        let ld = lines[line_num];
        if !func(line_num, ld) {
            return false;
        }
    }

    true
}

// ==========================================================================
// P_BlockThingsIterator (p_maputl.c lines 512-537)
// ==========================================================================

/// Iterate over all things in a single blockmap cell.
///
/// Walks the linked list of things in the specified blockmap cell via
/// `bnext` pointers. Returns `false` if `func` returns `false` for any
/// thing (early termination), otherwise returns `true`.
///
/// Original C: `P_BlockThingsIterator` (p_maputl.c:512-537)
pub fn p_block_things_iterator<F>(
    x: i32,
    y: i32,
    blocklinks: &[Option<usize>],
    mobjs: &[MapObject],
    bmapwidth: i32,
    bmapheight: i32,
    func: &mut F,
) -> bool
where
    F: FnMut(usize, &MapObject) -> bool,
{
    // Bounds check
    if x < 0 || y < 0 || x >= bmapwidth || y >= bmapheight {
        return true;
    }

    let idx = (y * bmapwidth + x) as usize;
    let mut mobj_opt = blocklinks[idx];

    while let Some(mobj_idx) = mobj_opt {
        // Read next pointer before calling callback (in case callback modifies it)
        let next = mobjs[mobj_idx].bnext;
        if !func(mobj_idx, &mobjs[mobj_idx]) {
            return false;
        }
        mobj_opt = next;
    }

    true
}

// ==========================================================================
// PIT_AddLineIntercepts (p_maputl.c lines 561-608)
// ==========================================================================

/// Check if a line crosses the current trace and add it to the intercept
/// buffer if so.
///
/// Uses a precision-based method selection: if the trace is long (any
/// component > 16 * FRACUNIT), uses [`p_point_on_divline_side`] for
/// better accuracy; otherwise uses [`p_point_on_line_side`].
///
/// If `earlyout` is set and the intercept is before FRACUNIT on a
/// single-sided line, returns `false` to stop traversal immediately.
///
/// Original C: `PIT_AddLineIntercepts` (p_maputl.c:561-608)
pub fn pit_add_line_intercepts(
    state: &mut MapUtilState,
    line_idx: usize,
    ld: &LineDef,
    vertexes: &[Vertex],
) -> bool {
    let v1 = &vertexes[ld.v1];
    let v2 = &vertexes[ld.v2];

    let trace_local = state.trace;

    // Choose side-test method based on trace magnitude to avoid
    // precision problems (p_maputl.c lines 570-582)
    let (s1, s2) = if trace_local.dx.0 > FRACUNIT * 16
        || trace_local.dy.0 > FRACUNIT * 16
        || trace_local.dx.0 < -(FRACUNIT * 16)
        || trace_local.dy.0 < -(FRACUNIT * 16)
    {
        // Long trace: test line endpoints against the trace divline
        (
            p_point_on_divline_side(v1.x, v1.y, &trace_local),
            p_point_on_divline_side(v2.x, v2.y, &trace_local),
        )
    } else {
        // Short trace: test trace endpoints against the line
        (
            p_point_on_line_side(trace_local.x, trace_local.y, ld, vertexes),
            p_point_on_line_side(
                trace_local.x + trace_local.dx,
                trace_local.y + trace_local.dy,
                ld,
                vertexes,
            ),
        )
    };

    // Line isn't crossed — both endpoints on same side
    if s1 == s2 {
        return true;
    }

    // Hit the line — compute fractional intercept
    let dl = p_make_divline(ld, vertexes);
    let frac = p_intercept_vector(&trace_local, &dl);

    // Behind source — ignore
    if frac.0 < 0 {
        return true;
    }

    // Early-out: solid line hit before reaching the target
    if state.earlyout && frac.0 < FRACUNIT && ld.backsector.is_none() {
        return false; // stop checking
    }

    // Add to intercept buffer
    if state.intercept_p < MAXINTERCEPTS {
        state.intercepts[state.intercept_p] = Intercept {
            frac,
            is_a_line: true,
            d: InterceptData::Line(line_idx),
        };
        state.intercept_p += 1;
    }

    true // continue
}

// ==========================================================================
// PIT_AddThingIntercepts (p_maputl.c lines 616-674)
// ==========================================================================

/// Check if a thing's bounding box crosses the current trace and add
/// it to the intercept buffer if so.
///
/// Selects corner pairs for the cross-check based on the trace direction
/// sign (positive slope vs negative slope).
///
/// Original C: `PIT_AddThingIntercepts` (p_maputl.c:616-674)
pub fn pit_add_thing_intercepts(
    state: &mut MapUtilState,
    thing_idx: usize,
    thing: &MapObject,
) -> bool {
    let trace_local = state.trace;

    // Determine corner pair based on trace direction sign
    // (p_maputl.c lines 632-650)
    let tracepositive = (trace_local.dx.0 ^ trace_local.dy.0) > 0;

    let (x1, y1, x2, y2) = if tracepositive {
        (
            thing.x - thing.radius, // left
            thing.y + thing.radius, // top
            thing.x + thing.radius, // right
            thing.y - thing.radius, // bottom
        )
    } else {
        (
            thing.x - thing.radius, // left
            thing.y - thing.radius, // bottom
            thing.x + thing.radius, // right
            thing.y + thing.radius, // top
        )
    };

    // Test corner endpoints against the trace divline
    let s1 = p_point_on_divline_side(x1, y1, &trace_local);
    let s2 = p_point_on_divline_side(x2, y2, &trace_local);

    // Line isn't crossed — both corners on same side
    if s1 == s2 {
        return true;
    }

    // Build divline from corner pair and compute intercept
    let dl = Divline {
        x: x1,
        y: y1,
        dx: x2 - x1,
        dy: y2 - y1,
    };

    let frac = p_intercept_vector(&trace_local, &dl);

    // Behind source — ignore
    if frac.0 < 0 {
        return true;
    }

    // Add to intercept buffer
    if state.intercept_p < MAXINTERCEPTS {
        state.intercepts[state.intercept_p] = Intercept {
            frac,
            is_a_line: false,
            d: InterceptData::Thing(thing_idx),
        };
        state.intercept_p += 1;
    }

    true // keep going
}

// ==========================================================================
// P_TraverseIntercepts (p_maputl.c lines 682-730)
// ==========================================================================

/// Process collected intercepts in sorted order using selection sort.
///
/// Iterates through all intercepts from nearest to farthest, calling
/// `func` on each one. Uses O(n²) selection sort with MAXINT sentinels
/// to mark processed entries.
///
/// Returns `true` if all intercepts within `maxfrac` were traversed
/// successfully, or `false` if `func` returned `false`.
///
/// Original C: `P_TraverseIntercepts` (p_maputl.c:682-730)
pub fn p_traverse_intercepts(state: &mut MapUtilState, func: traverser_t, maxfrac: Fixed) -> bool {
    let mut count = state.intercept_p as i32;

    while count > 0 {
        count -= 1;

        // Find the intercept with minimum frac (nearest to trace start).
        let mut dist = Fixed(MAXINT);
        let mut in_idx: usize = 0;

        #[allow(clippy::needless_range_loop)]
        for scan in 0..state.intercept_p {
            if state.intercepts[scan].frac < dist {
                dist = state.intercepts[scan].frac;
                in_idx = scan;
            }
        }

        // Past the requested range — done
        if dist > maxfrac {
            return true;
        }

        // Call the traverser function
        if !func(&state.intercepts[in_idx]) {
            return false; // don't bother going farther
        }

        // Mark as processed by setting frac to MAXINT
        state.intercepts[in_idx].frac = Fixed(MAXINT);
    }

    true // everything was traversed
}

// ==========================================================================
// P_PathTraverse (p_maputl.c lines 742-880)
// ==========================================================================

/// Trace a line from `(x1, y1)` to `(x2, y2)` through the blockmap,
/// collecting intercepts and calling the traverser function for each.
///
/// This is the main entry point for line-of-sight checks, hitscan attacks,
/// and other ray-casting operations. It:
///
/// 1. Resets the intercept buffer and increments `validcount`.
/// 2. Nudges the start point off blockmap boundaries to avoid precision issues.
/// 3. Steps through blockmap cells along the ray using a DDA-style algorithm.
/// 4. At each cell, optionally adds line intercepts (PT_ADDLINES) and/or
///    thing intercepts (PT_ADDTHINGS).
/// 5. After traversal, processes all collected intercepts in sorted order
///    via [`p_traverse_intercepts`].
///
/// The 64-step safety limit prevents infinite loops from floating-point
/// rounding errors.
///
/// Original C: `P_PathTraverse` (p_maputl.c:742-880)
#[allow(clippy::too_many_arguments)]
pub fn p_path_traverse(
    state: &mut MapUtilState,
    mut x1: Fixed,
    mut y1: Fixed,
    x2: Fixed,
    y2: Fixed,
    flags: i32,
    trav: traverser_t,
    // Level data references:
    blockmap: &[i16],
    blockmaplump: &[i16],
    lines: &mut [LineDef],
    vertexes: &[Vertex],
    mobjs: &[MapObject],
    blocklinks: &[Option<usize>],
    bmaporgx: Fixed,
    bmaporgy: Fixed,
    bmapwidth: i32,
    bmapheight: i32,
    validcount: &mut i32,
) -> bool {
    // Set early-out flag from flags
    state.earlyout = (flags & PT_EARLYOUT) != 0;

    // Increment validation counter to mark a new traversal pass
    *validcount += 1;

    // Reset intercept buffer
    state.intercept_p = 0;

    // Nudge start point off blockmap boundaries to avoid precision
    // issues with exact boundary alignment (p_maputl.c lines 777-781)
    if ((x1.0 - bmaporgx.0) & (MAPBLOCKSIZE - 1)) == 0 {
        x1 = Fixed(x1.0 + FRACUNIT);
    }
    if ((y1.0 - bmaporgy.0) & (MAPBLOCKSIZE - 1)) == 0 {
        y1 = Fixed(y1.0 + FRACUNIT);
    }

    // Set the trace line
    state.trace.x = x1;
    state.trace.y = y1;
    state.trace.dx = x2 - x1;
    state.trace.dy = y2 - y1;

    // Convert to blockmap-relative coordinates
    let adj_x1 = x1.0 - bmaporgx.0;
    let adj_y1 = y1.0 - bmaporgy.0;
    let xt1 = adj_x1 >> MAPBLOCKSHIFT;
    let yt1 = adj_y1 >> MAPBLOCKSHIFT;

    let adj_x2 = x2.0 - bmaporgx.0;
    let adj_y2 = y2.0 - bmaporgy.0;
    let xt2 = adj_x2 >> MAPBLOCKSHIFT;
    let yt2 = adj_y2 >> MAPBLOCKSHIFT;

    // Calculate step direction and initial partial/intercept values
    // for the X axis (p_maputl.c lines 798-815)
    let mapxstep: i32;
    let partial_x: i32;
    let ystep: Fixed;

    if xt2 > xt1 {
        mapxstep = 1;
        partial_x = FRACUNIT - ((adj_x1 >> MAPBTOFRAC) & (FRACUNIT - 1));
        let abs_dx = (adj_x2 - adj_x1).abs();
        ystep = if abs_dx != 0 {
            Fixed(adj_y2 - adj_y1).fixed_div(Fixed(abs_dx))
        } else {
            Fixed(256 * FRACUNIT)
        };
    } else if xt2 < xt1 {
        mapxstep = -1;
        partial_x = (adj_x1 >> MAPBTOFRAC) & (FRACUNIT - 1);
        let abs_dx = (adj_x2 - adj_x1).abs();
        ystep = if abs_dx != 0 {
            Fixed(adj_y2 - adj_y1).fixed_div(Fixed(abs_dx))
        } else {
            Fixed(256 * FRACUNIT)
        };
    } else {
        mapxstep = 0;
        partial_x = FRACUNIT;
        ystep = Fixed(256 * FRACUNIT);
    }

    let mut yintercept: i32 = (adj_y1 >> MAPBTOFRAC) + Fixed(partial_x).fixed_mul(ystep).0;

    // Calculate step direction and initial partial/intercept values
    // for the Y axis (p_maputl.c lines 820-837)
    let mapystep: i32;
    let partial_y: i32;
    let xstep: Fixed;

    if yt2 > yt1 {
        mapystep = 1;
        partial_y = FRACUNIT - ((adj_y1 >> MAPBTOFRAC) & (FRACUNIT - 1));
        let abs_dy = (adj_y2 - adj_y1).abs();
        xstep = if abs_dy != 0 {
            Fixed(adj_x2 - adj_x1).fixed_div(Fixed(abs_dy))
        } else {
            Fixed(256 * FRACUNIT)
        };
    } else if yt2 < yt1 {
        mapystep = -1;
        partial_y = (adj_y1 >> MAPBTOFRAC) & (FRACUNIT - 1);
        let abs_dy = (adj_y2 - adj_y1).abs();
        xstep = if abs_dy != 0 {
            Fixed(adj_x2 - adj_x1).fixed_div(Fixed(abs_dy))
        } else {
            Fixed(256 * FRACUNIT)
        };
    } else {
        mapystep = 0;
        partial_y = FRACUNIT;
        xstep = Fixed(256 * FRACUNIT);
    }

    let mut xintercept: i32 = (adj_x1 >> MAPBTOFRAC) + Fixed(partial_y).fixed_mul(xstep).0;

    // Step through map blocks (64-step safety limit)
    let mut mapx = xt1;
    let mut mapy = yt1;

    for _count in 0..64 {
        // Add line intercepts if requested
        if (flags & PT_ADDLINES) != 0 {
            let verts = vertexes;
            let st = &mut *state;
            let mut line_func = |line_idx: usize, ld: LineDef| -> bool {
                pit_add_line_intercepts(st, line_idx, &ld, verts)
            };
            if !p_block_lines_iterator(
                mapx,
                mapy,
                blockmap,
                blockmaplump,
                lines,
                bmapwidth,
                bmapheight,
                *validcount,
                &mut line_func,
            ) {
                return false; // early out
            }
        }

        // Add thing intercepts if requested
        if (flags & PT_ADDTHINGS) != 0 {
            let st = &mut *state;
            let mut thing_func = |thing_idx: usize, thing: &MapObject| -> bool {
                pit_add_thing_intercepts(st, thing_idx, thing)
            };
            if !p_block_things_iterator(
                mapx,
                mapy,
                blocklinks,
                mobjs,
                bmapwidth,
                bmapheight,
                &mut thing_func,
            ) {
                return false; // early out
            }
        }

        // Check if we've reached the destination block
        if mapx == xt2 && mapy == yt2 {
            break;
        }

        // Step to the next block using DDA algorithm
        if (yintercept >> FRACBITS) == mapy {
            yintercept += ystep.0;
            mapx += mapxstep;
        } else if (xintercept >> FRACBITS) == mapx {
            xintercept += xstep.0;
            mapy += mapystep;
        }
    }

    // Go through the sorted intercept list
    p_traverse_intercepts(state, trav, Fixed(FRACUNIT))
}
