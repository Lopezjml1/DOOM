// Copyright (C) 1993-1996 by id Software, Inc.
// Copyright (C) 2024-2026 DOOM Rust Port Contributors
//
// This program is free software; you can redistribute it and/or
// modify it under the terms of the GNU General Public License
// as published by the Free Software Foundation; either version 2
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

//! LineOfSight/Visibility checks, uses REJECT Lookup Table.
//! Translated from linuxdoom-1.10/p_sight.c
//!
//! Determines whether an unobstructed line-of-sight exists between two map
//! objects.  Uses the REJECT lookup table for fast sector-pair rejection and
//! recursive BSP tree traversal with slope narrowing for precise checks.
//!
//! # Shared State
//!
//! [`topslope`] and [`bottomslope`] are **shared** with `map.rs`
//! (`PTR_AimTraverse` uses them for vertical auto-aim calculation).  They are
//! exposed as `pub static mut` so both modules can read/write them.
//!
//! # Behavioral Parity Notes
//!
//! * The bug on original line 78 (`if (x==node->y)` instead of
//!   `if (y==node->y)` in [`p_divline_side`]'s horizontal-partition branch)
//!   is **preserved intentionally** for exact behavioral parity.
//! * All fixed-point arithmetic uses wrapping operations matching the
//!   original C signed-integer behaviour.
//! * REJECT table byte/bit addressing and BSP traversal order are identical
//!   to the original engine.

// Static-mut globals match the original C pattern used throughout the DOOM
// engine (e.g. `maputl.rs`).  They are safe in DOOM's single-threaded
// game-loop context.
#![allow(non_upper_case_globals)]

use crate::play::maputl::Divline;
use crate::play::setup::LevelData;
use crate::types::fixed::{Fixed, FRACBITS};
use crate::types::map_data::{LineFlags, NF_SUBSECTOR};
use crate::types::mobj::MapObject;

// ==========================================================================
// Module-level state  (p_sight.c lines 39-47)
// ==========================================================================

/// Eye-Z of the looker: `t1.z + t1.height - (t1.height >> 2)` (3/4 of total
/// height above the floor).
static mut sightzstart: Fixed = Fixed(0);

/// Slope to the **top** of the target.
///
/// Shared with `map.rs` — `PTR_AimTraverse` reads/writes this value for
/// vertical auto-aim.  Original C: `fixed_t topslope;` (p_sight.c line 40).
pub static mut topslope: Fixed = Fixed(0);

/// Slope to the **bottom** of the target.
///
/// Shared with `map.rs` — `PTR_AimTraverse` reads/writes this value for
/// vertical auto-aim.  Original C: `fixed_t bottomslope;` (p_sight.c line 41).
pub static mut bottomslope: Fixed = Fixed(0);

/// Trace divline from the looker (`t1`) to the target (`t2`).
static mut strace: Divline = Divline {
    x: Fixed(0),
    y: Fixed(0),
    dx: Fixed(0),
    dy: Fixed(0),
};

/// Cached target X position, used in [`p_cross_bsp_node`] and
/// [`p_cross_subsector`] to check the target-side of partition lines.
static mut t2x: Fixed = Fixed(0);

/// Cached target Y position.
static mut t2y: Fixed = Fixed(0);

/// Sight-check statistics counters.
///
/// * `[0]` — number of checks trivially rejected by the REJECT table.
/// * `[1]` — number of checks that required full BSP traversal.
pub static mut sightcounts: [i32; 2] = [0; 2];

/// Validation frame counter.  Incremented once per [`p_check_sight`] call so
/// that [`p_cross_subsector`] can skip linedefs that have already been tested
/// within the same call (deduplication via `LineDef::validcount`).
///
/// In the original C engine this lives in `r_main.c` and is shared across
/// sight checks, path traversal, and BSP rendering.  Here it is defined as a
/// module-level static; other modules needing the same counter can import it.
pub static mut validcount: i32 = 0;

// ==========================================================================
// P_DivlineSide  (p_sight.c lines 54-99)
// ==========================================================================

/// Classify point (`x`, `y`) with respect to a partition divline.
///
/// Returns:
/// * `0` — point is on the **front** side
/// * `1` — point is on the **back** side
/// * `2` — point is **on** the divline (or within integer-truncation tolerance)
///
/// # Bug Preservation (line 78)
///
/// The horizontal-partition branch contains the original bug:
/// `if (x == node->y)` instead of `if (y == node->y)`.
/// This is preserved verbatim for behavioral parity.
fn p_divline_side(x: Fixed, y: Fixed, node: &Divline) -> i32 {
    // ---------------------------------------------------------------
    // Vertical partition (dx == 0)
    // ---------------------------------------------------------------
    if node.dx == Fixed::ZERO {
        if x == node.x {
            return 2;
        }
        if x <= node.x {
            return if node.dy > Fixed::ZERO { 1 } else { 0 };
        }
        return if node.dy < Fixed::ZERO { 1 } else { 0 };
    }

    // ---------------------------------------------------------------
    // Horizontal partition (dy == 0)
    // BUG PRESERVED: original line 78 uses `x` where `y` was intended.
    // ---------------------------------------------------------------
    if node.dy == Fixed::ZERO {
        // BUG: should be `y == node.y` — preserved for behavioral parity.
        if x == node.y {
            return 2;
        }
        if y <= node.y {
            return if node.dx < Fixed::ZERO { 1 } else { 0 };
        }
        return if node.dx > Fixed::ZERO { 1 } else { 0 };
    }

    // ---------------------------------------------------------------
    // General case — cross-product test
    // ---------------------------------------------------------------
    let dx = Fixed(x.0.wrapping_sub(node.x.0));
    let dy = Fixed(y.0.wrapping_sub(node.y.0));

    // Integer-part cross product (shift away fractional bits first to avoid
    // overflow in the multiply).
    let left: i32 = (node.dy.0 >> FRACBITS).wrapping_mul(dx.0 >> FRACBITS);
    let right: i32 = (dy.0 >> FRACBITS).wrapping_mul(node.dx.0 >> FRACBITS);

    if right < left {
        return 0; // front
    }
    if left == right {
        return 2; // on
    }
    1 // back
}

// ==========================================================================
// P_InterceptVector2  (p_sight.c lines 108-128)
// ==========================================================================

/// Compute the fractional intercept point along divline `v2` where `v1`
/// crosses it.
///
/// Uses >>8 pre-shift on the intermediate products to trade precision for
/// overflow headroom, exactly matching the original C implementation.
///
/// Returns `Fixed::ZERO` if the lines are parallel (`den == 0`).
fn p_intercept_vector2(v2: &Divline, v1: &Divline) -> Fixed {
    // den = FixedMul(v1->dy>>8, v2->dx) - FixedMul(v1->dx>>8, v2->dy)
    let den = Fixed(v1.dy.0 >> 8).fixed_mul(v2.dx) - Fixed(v1.dx.0 >> 8).fixed_mul(v2.dy);

    if den == Fixed::ZERO {
        return Fixed::ZERO; // parallel
    }

    // num = FixedMul((v1->x - v2->x)>>8, v1->dy)
    //     + FixedMul((v2->y - v1->y)>>8, v1->dx)
    let num =
        Fixed((v1.x - v2.x).0 >> 8).fixed_mul(v1.dy) + Fixed((v2.y - v1.y).0 >> 8).fixed_mul(v1.dx);

    num.fixed_div(den)
}

// ==========================================================================
// P_CrossSubsector  (p_sight.c lines 135-248)
// ==========================================================================

/// Check whether the sight trace ([`strace`]) crosses through the given
/// subsector without being fully occluded.
///
/// Iterates every seg in the subsector; for each seg whose linedef is crossed
/// by the trace, narrows the vertical slope window (`topslope` /
/// `bottomslope`) based on floor and ceiling height changes.
///
/// Returns `true` if the trace passes through the subsector unblocked.
///
/// # Safety
///
/// Accesses module-level `static mut` globals (`strace`, `t2x`, `t2y`,
/// `sightzstart`, `topslope`, `bottomslope`, `validcount`).
/// Must only be called from the single-threaded game loop.
unsafe fn p_cross_subsector(num: usize, level: &mut LevelData) -> bool {
    // Snapshot module-level strace and t2 coordinates into locals to avoid
    // creating shared references to `static mut` (Rust 2024 safety).
    let strace_local = strace;
    let t2x_local = t2x;
    let t2y_local = t2y;

    // Retrieve subsector metadata (copy to avoid overlapping borrows).
    let count = level.subsectors[num].numlines as usize;
    let first_line = level.subsectors[num].firstline as usize;

    for i in 0..count {
        let seg_idx = first_line + i;

        // -- Extract seg info (copy values to release borrow on `level.segs`) --
        let line_idx = level.segs[seg_idx].linedef;
        let frontsector_idx = level.segs[seg_idx].frontsector;
        let backsector_idx = level.segs[seg_idx].backsector;

        // -- Skip already-checked linedefs (validcount deduplication) ----------
        if level.lines[line_idx].validcount == validcount {
            continue;
        }
        level.lines[line_idx].validcount = validcount;

        // -- Get linedef vertex coordinates -----------------------------------
        let v1_idx = level.lines[line_idx].v1;
        let v2_idx = level.lines[line_idx].v2;
        let v1x = level.vertexes[v1_idx].x;
        let v1y = level.vertexes[v1_idx].y;
        let v2x = level.vertexes[v2_idx].x;
        let v2y = level.vertexes[v2_idx].y;

        // -- Check if strace crosses the linedef (vertex side test) -----------
        let s1 = p_divline_side(v1x, v1y, &strace_local);
        let s2 = p_divline_side(v2x, v2y, &strace_local);

        // If both vertices are on the same side, the trace doesn't cross.
        if s1 == s2 {
            continue;
        }

        // -- Reverse test: are strace endpoints on the same side of the seg? --
        let divl = Divline {
            x: v1x,
            y: v1y,
            dx: Fixed(v2x.0.wrapping_sub(v1x.0)),
            dy: Fixed(v2y.0.wrapping_sub(v1y.0)),
        };

        let s1 = p_divline_side(strace_local.x, strace_local.y, &divl);
        let s2 = p_divline_side(t2x_local, t2y_local, &divl);

        if s1 == s2 {
            continue;
        }

        // -- The line is crossed — determine if it blocks sight ---------------
        let line_flags = level.lines[line_idx].flags;

        // Not two-sided → solid wall → blocks sight.
        if (line_flags & LineFlags::ML_TWOSIDED.bits()) == 0 {
            return false;
        }

        // Two-sided line — compare front/back sector heights.
        let back_idx = match backsector_idx {
            Some(idx) => idx,
            None => {
                // Shouldn't happen for a two-sided linedef, but treat as solid
                // to be safe.
                return false;
            }
        };

        let front_floor = level.sectors[frontsector_idx].floorheight;
        let front_ceiling = level.sectors[frontsector_idx].ceilingheight;
        let back_floor = level.sectors[back_idx].floorheight;
        let back_ceiling = level.sectors[back_idx].ceilingheight;

        // No wall to block sight (same floor & ceiling on both sides).
        if front_floor == back_floor && front_ceiling == back_ceiling {
            continue;
        }

        // Compute opening: minimum ceiling, maximum floor.
        let opentop = if front_ceiling < back_ceiling {
            front_ceiling
        } else {
            back_ceiling
        };

        let openbottom = if front_floor > back_floor {
            front_floor
        } else {
            back_floor
        };

        // Quick test for totally closed doors.
        if openbottom >= opentop {
            return false;
        }

        // Fractional intercept along the sight trace.
        let frac = p_intercept_vector2(&strace_local, &divl);

        // Narrow the vertical slope window based on height changes.
        if front_floor != back_floor {
            let slope = (openbottom - sightzstart).fixed_div(frac);
            if slope > bottomslope {
                bottomslope = slope;
            }
        }

        if front_ceiling != back_ceiling {
            let slope = (opentop - sightzstart).fixed_div(frac);
            if slope < topslope {
                topslope = slope;
            }
        }

        // Fully occluded?
        if topslope <= bottomslope {
            return false;
        }
    }

    // Passed the subsector without obstruction.
    true
}

// ==========================================================================
// P_CrossBSPNode  (p_sight.c lines 257-290)
// ==========================================================================

/// Recursively traverse the BSP tree, checking whether the sight trace passes
/// through each visited node/subsector without obstruction.
///
/// When a leaf (subsector) is reached (indicated by the [`NF_SUBSECTOR`] flag
/// on `bspnum`), delegates to [`p_cross_subsector`].  For internal nodes, the
/// trace is checked against both children as needed: first the side
/// containing the trace origin, then (only if the target is on the opposite
/// side) the other child.
///
/// # Safety
///
/// Accesses module-level `static mut` globals (`strace`, `t2x`, `t2y`) and
/// calls [`p_cross_subsector`] which accesses additional globals.
unsafe fn p_cross_bsp_node(bspnum: i32, level: &mut LevelData) -> bool {
    // -- Leaf node (subsector) ------------------------------------------------
    if bspnum & (NF_SUBSECTOR as i32) != 0 {
        if bspnum == -1 {
            // Degenerate map with zero nodes — treat as single subsector 0.
            return p_cross_subsector(0, level);
        }
        return p_cross_subsector((bspnum & !(NF_SUBSECTOR as i32)) as usize, level);
    }

    // -- Internal node --------------------------------------------------------
    // Snapshot module-level strace and t2 coordinates into locals to avoid
    // creating shared references to `static mut` (Rust 2024 safety).
    let strace_local = strace;
    let t2x_local = t2x;
    let t2y_local = t2y;

    // Copy fields from the node to avoid holding a borrow across the
    // recursive `&mut level` calls.
    let bsp_idx = bspnum as usize;
    let bsp_x = level.nodes[bsp_idx].x;
    let bsp_y = level.nodes[bsp_idx].y;
    let bsp_dx = level.nodes[bsp_idx].dx;
    let bsp_dy = level.nodes[bsp_idx].dy;
    let children = level.nodes[bsp_idx].children;

    let node_divline = Divline {
        x: bsp_x,
        y: bsp_y,
        dx: bsp_dx,
        dy: bsp_dy,
    };

    // Decide which side the start point (strace origin) is on.
    let mut side = p_divline_side(strace_local.x, strace_local.y, &node_divline);
    if side == 2 {
        side = 0; // "on" the partition → treat as front
    }

    // Cross the starting side first.
    if !p_cross_bsp_node(children[side as usize] as i32, level) {
        return false;
    }

    // If the target is on the same side, no need to cross the partition.
    if side == p_divline_side(t2x_local, t2y_local, &node_divline) {
        return true;
    }

    // Cross the ending side.
    p_cross_bsp_node(children[(side ^ 1) as usize] as i32, level)
}

// ==========================================================================
// P_CheckSight  (p_sight.c lines 299-347) — Main entry point
// ==========================================================================

/// Determine whether an unobstructed line-of-sight exists from map object
/// `t1` (the looker) to map object `t2` (the target).
///
/// The algorithm has two phases:
///
/// 1. **REJECT table quick check** — Uses the pre-computed sector-to-sector
///    reject matrix to instantly rule out impossible LOS pairs.
/// 2. **BSP traversal** — For non-rejected pairs, traces a line from `t1`'s
///    eye position to `t2` through the BSP tree, narrowing a vertical slope
///    window at each crossed two-sided linedef.  If the window collapses
///    (`topslope <= bottomslope`), the LOS is blocked.
///
/// # Arguments
///
/// * `t1`    — the looker (origin of the sight trace)
/// * `t2`    — the target (destination of the sight trace)
/// * `level` — mutable reference to the loaded level data (needs mutable
///   access to update `LineDef::validcount` for deduplication)
///
/// # Returns
///
/// `true` if there is an unobstructed line-of-sight from `t1` to `t2`.
pub fn p_check_sight(t1: &MapObject, t2: &MapObject, level: &mut LevelData) -> bool {
    // Safety: All static-mut access is confined to DOOM's single-threaded
    // game loop.  No concurrent access is possible.
    unsafe {
        // ==================================================================
        // Phase 1 — REJECT table quick check
        // ==================================================================

        // Determine sector indices for both objects.
        let t1_sub = match t1.subsector {
            Some(idx) => idx,
            None => return false,
        };
        let t2_sub = match t2.subsector {
            Some(idx) => idx,
            None => return false,
        };

        let s1 = level.subsectors[t1_sub].sector;
        let s2 = level.subsectors[t2_sub].sector;
        let numsectors = level.sectors.len();
        let pnum: usize = s1.wrapping_mul(numsectors).wrapping_add(s2);
        let bytenum: usize = pnum >> 3;
        let bitnum: u8 = 1u8 << (pnum & 7);

        // Check reject matrix.
        if bytenum < level.reject_matrix.len() && (level.reject_matrix[bytenum] & bitnum) != 0 {
            sightcounts[0] += 1;
            return false; // can't possibly be connected
        }

        // ==================================================================
        // Phase 2 — Full BSP traversal
        // ==================================================================

        sightcounts[1] += 1;

        // Bump the validation counter to invalidate all previous line marks.
        validcount += 1;

        // Eye Z at 3/4 of the looker's height.
        sightzstart = t1.z + t1.height - Fixed(t1.height.0 >> 2);

        // Initial slope window: full vertical extent of the target.
        topslope = (t2.z + t2.height) - sightzstart;
        bottomslope = t2.z - sightzstart;

        // Set up the trace divline from t1 to t2.
        strace = Divline {
            x: t1.x,
            y: t1.y,
            dx: Fixed(t2.x.0.wrapping_sub(t1.x.0)),
            dy: Fixed(t2.y.0.wrapping_sub(t1.y.0)),
        };

        t2x = t2.x;
        t2y = t2.y;

        // Start the recursive BSP traversal from the root node.
        let num_nodes = level.nodes.len() as i32;
        p_cross_bsp_node(num_nodes - 1, level)
    }
}

// ==========================================================================
// Unit Tests
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // P_DivlineSide tests
    // -------------------------------------------------------------------

    #[test]
    fn test_divline_side_vertical_front() {
        // Vertical partition at x=0 going up (dy > 0).
        // Point to the LEFT (x < 0) should be on back side (1).
        let divl = Divline {
            x: Fixed(0),
            y: Fixed(0),
            dx: Fixed(0),
            dy: Fixed::from_int(1), // dy > 0
        };
        // x = -1.0, y = 0 → x < node.x → dy>0 → side 1 (back)
        assert_eq!(p_divline_side(Fixed::from_int(-1), Fixed(0), &divl), 1);
    }

    #[test]
    fn test_divline_side_vertical_back() {
        // Vertical partition at x=0 going up (dy > 0).
        // Point to the RIGHT (x > 0) should be on front side (0).
        let divl = Divline {
            x: Fixed(0),
            y: Fixed(0),
            dx: Fixed(0),
            dy: Fixed::from_int(1),
        };
        // x = 1.0, y = 0 → x > node.x → dy<0 is false → side 0 (front)
        assert_eq!(p_divline_side(Fixed::from_int(1), Fixed(0), &divl), 0);
    }

    #[test]
    fn test_divline_side_vertical_on() {
        // Vertical partition at x=100. Point with x=100 → on the line.
        let divl = Divline {
            x: Fixed::from_int(100),
            y: Fixed(0),
            dx: Fixed(0),
            dy: Fixed::from_int(1),
        };
        assert_eq!(
            p_divline_side(Fixed::from_int(100), Fixed::from_int(50), &divl),
            2
        );
    }

    #[test]
    fn test_divline_side_horizontal_bug_preserved() {
        // Horizontal partition at y=100 going right (dx > 0).
        // The ORIGINAL BUG compares `x == node.y` instead of `y == node.y`.
        // So a point with x == 100 (same as node.y) will be classified as
        // "on the line" (2), even if y != 100.
        let divl = Divline {
            x: Fixed(0),
            y: Fixed::from_int(100),
            dx: Fixed::from_int(1), // dx > 0
            dy: Fixed(0),
        };

        // BUG: x == node.y (100 == 100) → should return 2
        assert_eq!(
            p_divline_side(Fixed::from_int(100), Fixed::from_int(50), &divl),
            2,
            "Bug on original line 78 must return 2 when x == node.y"
        );
    }

    #[test]
    fn test_divline_side_general_front() {
        // 45-degree partition from origin going (1,1).
        // Point (0, 2) should be on front side (0) — to the "left" of the
        // direction of travel (when cross product > 0).
        let divl = Divline {
            x: Fixed(0),
            y: Fixed(0),
            dx: Fixed::from_int(1),
            dy: Fixed::from_int(1),
        };
        // left  = (dy>>16)*(dx_point>>16) = 1 * 0 = 0
        // right = (dy_point>>16)*(dx>>16) = 2 * 1 = 2
        // right > left → side 1 (back)
        assert_eq!(p_divline_side(Fixed(0), Fixed::from_int(2), &divl), 1);
    }

    #[test]
    fn test_divline_side_general_on() {
        // Point on the line.
        let divl = Divline {
            x: Fixed(0),
            y: Fixed(0),
            dx: Fixed::from_int(2),
            dy: Fixed::from_int(2),
        };
        // Point (1, 1) — on the 45° line.
        // left  = (2)*(1) = 2
        // right = (1)*(2) = 2
        // left == right → 2
        assert_eq!(
            p_divline_side(Fixed::from_int(1), Fixed::from_int(1), &divl),
            2
        );
    }

    // -------------------------------------------------------------------
    // P_InterceptVector2 tests
    // -------------------------------------------------------------------

    #[test]
    fn test_intercept_vector2_perpendicular() {
        // v2 horizontal: from (0,0) going right (+10, 0)
        // v1 vertical: from (5,-5) going up (0, +10)
        // They cross at (5, 0) which is at fraction 0.5 along v2.
        let v2 = Divline {
            x: Fixed(0),
            y: Fixed(0),
            dx: Fixed::from_int(10),
            dy: Fixed(0),
        };
        let v1 = Divline {
            x: Fixed::from_int(5),
            y: Fixed::from_int(-5),
            dx: Fixed(0),
            dy: Fixed::from_int(10),
        };
        let frac = p_intercept_vector2(&v2, &v1);
        // Should be approximately 0.5 in fixed-point = 32768
        // Due to >>8 precision loss there may be minor rounding, but for
        // these clean values the result should be exact.
        assert!(
            (frac.0 - 32768).abs() < 256,
            "Expected ~0.5 (32768), got {}",
            frac.0
        );
    }

    #[test]
    fn test_intercept_vector2_parallel() {
        // Two parallel horizontal lines.
        let v2 = Divline {
            x: Fixed(0),
            y: Fixed(0),
            dx: Fixed::from_int(10),
            dy: Fixed(0),
        };
        let v1 = Divline {
            x: Fixed(0),
            y: Fixed::from_int(5),
            dx: Fixed::from_int(10),
            dy: Fixed(0),
        };
        assert_eq!(p_intercept_vector2(&v2, &v1), Fixed::ZERO);
    }

    // -------------------------------------------------------------------
    // p_check_sight integration tests
    // -------------------------------------------------------------------

    use crate::play::setup::LevelData;
    use crate::types::map_data::{LineDef, Node, Sector, Seg, Subsector, Vertex};
    use crate::types::mobj::MapObject;

    /// Build a minimal two-subsector level with one two-sided linedef.
    ///
    /// The geometry places the linedef at x=0 (vertical wall from y=-100 to
    /// y=+100) and objects at y=10 to avoid the original DOOM bug where
    /// `p_divline_side` compares `x == node.y` instead of `y == node.y` for
    /// horizontal traces (this would cause vertex_x == strace.y = 0 to
    /// trigger when objects are at y=0).
    fn make_open_level() -> LevelData {
        let mut level = LevelData::default();

        // Two vertices forming a vertical partition line at x=0.
        level.vertexes = vec![
            Vertex {
                x: Fixed(0),
                y: Fixed(-100 * 65536),
            },
            Vertex {
                x: Fixed(0),
                y: Fixed(100 * 65536),
            },
        ];

        // Two sectors at equal heights.
        level.sectors = vec![
            Sector {
                floorheight: Fixed(0),
                ceilingheight: Fixed(128 * 65536),
                ..Sector::default()
            },
            Sector {
                floorheight: Fixed(0),
                ceilingheight: Fixed(128 * 65536),
                ..Sector::default()
            },
        ];

        // One two-sided linedef between the sectors.
        level.lines = vec![LineDef {
            v1: 0,
            v2: 1,
            flags: 4, // ML_TWOSIDED
            frontsector: Some(0),
            backsector: Some(1),
            validcount: 0,
            ..LineDef::default()
        }];

        // Two segs (one for each side of the linedef).
        level.segs = vec![
            Seg {
                v1: 0,
                v2: 1,
                linedef: 0,
                frontsector: 0,
                backsector: Some(1),
                ..Seg::default()
            },
            Seg {
                v1: 1,
                v2: 0,
                linedef: 0,
                frontsector: 1,
                backsector: Some(0),
                ..Seg::default()
            },
        ];

        // Two subsectors, one seg each.
        level.subsectors = vec![
            Subsector {
                sector: 0,
                numlines: 1,
                firstline: 0,
            },
            Subsector {
                sector: 1,
                numlines: 1,
                firstline: 1,
            },
        ];

        // One BSP node partitioning along x=0, children are subsectors.
        level.nodes = vec![Node {
            x: Fixed(0),
            y: Fixed(0),
            dx: Fixed(0),
            dy: Fixed(65536),
            children: [0x8000, 0x8001],
            ..Node::default()
        }];

        // Reject matrix: all zeros (no pairs rejected).
        // 2 sectors → 4 bits → 1 byte.
        level.reject_matrix = vec![0u8];

        level
    }

    /// Create a MapObject at the given world position (in map units).
    /// Uses y=10 by default to avoid the p_divline_side line-78 bug.
    fn make_mobj(x: i32, y: i32, z: i32, h: i32, sub: usize) -> MapObject {
        let mut mobj = MapObject::default();
        mobj.x = Fixed(x * 65536);
        mobj.y = Fixed(y * 65536);
        mobj.z = Fixed(z * 65536);
        mobj.height = Fixed(h * 65536);
        mobj.subsector = Some(sub);
        mobj
    }

    #[test]
    fn test_check_sight_open_level_clear_los() {
        // Objects at y=10 to avoid degenerate case (vertex x=0 == strace y=0).
        let mut level = make_open_level();
        let t1 = make_mobj(-64, 10, 0, 56, 0);
        let t2 = make_mobj(64, 10, 0, 56, 1);
        assert!(p_check_sight(&t1, &t2, &mut level));
    }

    #[test]
    fn test_check_sight_same_sector() {
        let mut level = make_open_level();
        let t1 = make_mobj(-64, 10, 0, 56, 0);
        let t2 = make_mobj(-32, 10, 0, 56, 0);
        assert!(p_check_sight(&t1, &t2, &mut level));
    }

    #[test]
    fn test_check_sight_rejected_by_reject_table() {
        let mut level = make_open_level();
        // Set reject bit for (sector 0 → sector 1):
        // pnum = 0*2 + 1 = 1, bytenum = 0, bitnum = (1<<1) = 0b10
        level.reject_matrix = vec![0b00000010];
        let t1 = make_mobj(-64, 10, 0, 56, 0);
        let t2 = make_mobj(64, 10, 0, 56, 1);
        assert!(!p_check_sight(&t1, &t2, &mut level));
    }

    #[test]
    fn test_check_sight_solid_wall_blocks() {
        let mut level = make_open_level();
        // Remove ML_TWOSIDED → one-sided solid wall.
        level.lines[0].flags = 0;
        let t1 = make_mobj(-64, 10, 0, 56, 0);
        let t2 = make_mobj(64, 10, 0, 56, 1);
        assert!(!p_check_sight(&t1, &t2, &mut level));
    }

    #[test]
    fn test_check_sight_closed_door_blocks() {
        let mut level = make_open_level();
        // Back sector: floor == ceiling → fully closed.
        level.sectors[1].floorheight = Fixed(128 * 65536);
        level.sectors[1].ceilingheight = Fixed(128 * 65536);
        let t1 = make_mobj(-64, 10, 0, 56, 0);
        let t2 = make_mobj(64, 10, 0, 56, 1);
        assert!(!p_check_sight(&t1, &t2, &mut level));
    }

    #[test]
    fn test_check_sight_nil_subsector_returns_false() {
        let mut level = make_open_level();
        let mut t1 = make_mobj(-64, 10, 0, 56, 0);
        let t2 = make_mobj(64, 10, 0, 56, 1);
        t1.subsector = None;
        assert!(!p_check_sight(&t1, &t2, &mut level));
    }
}
