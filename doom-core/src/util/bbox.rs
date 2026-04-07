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

//! Translated from linuxdoom-1.10/m_bbox.c and linuxdoom-1.10/m_bbox.h
//!
//! Bounding box utilities. Used by BSP traversal and collision detection.
//! A bounding box is represented as `[Fixed; 4]` indexed by [`BOXTOP`],
//! [`BOXBOTTOM`], [`BOXLEFT`], [`BOXRIGHT`] constants.
//!
//! The original C code stores bounding boxes as `fixed_t box[4]` and
//! accesses them via an anonymous enum (`BOXTOP=0, BOXBOTTOM=1, BOXLEFT=2,
//! BOXRIGHT=3`). This Rust translation preserves the exact same layout and
//! index semantics.

use crate::types::fixed::Fixed;

// ---------------------------------------------------------------------------
// Box index constants (from m_bbox.h lines 32-38)
// ---------------------------------------------------------------------------
// These indices MUST match the original C enum values exactly. They are used
// throughout the engine to index into `[Fixed; 4]` bounding box arrays.

/// Index for the top (maximum Y) coordinate of a bounding box.
/// Original C: `BOXTOP = 0` in `m_bbox.h`
pub const BOXTOP: usize = 0;

/// Index for the bottom (minimum Y) coordinate of a bounding box.
/// Original C: `BOXBOTTOM = 1` in `m_bbox.h`
pub const BOXBOTTOM: usize = 1;

/// Index for the left (minimum X) coordinate of a bounding box.
/// Original C: `BOXLEFT = 2` in `m_bbox.h`
pub const BOXLEFT: usize = 2;

/// Index for the right (maximum X) coordinate of a bounding box.
/// Original C: `BOXRIGHT = 3` in `m_bbox.h`
pub const BOXRIGHT: usize = 3;

// ---------------------------------------------------------------------------
// BBox type alias
// ---------------------------------------------------------------------------

/// Bounding box as `[Fixed; 4]` indexed by [`BOXTOP`], [`BOXBOTTOM`],
/// [`BOXLEFT`], [`BOXRIGHT`].
///
/// This is a simple array alias — no heap allocation is required.
/// The layout matches the original C `fixed_t box[4]` exactly.
pub type BBox = [Fixed; 4];

// ---------------------------------------------------------------------------
// M_ClearBox (from m_bbox.c lines 39-43)
// ---------------------------------------------------------------------------

/// Reset bounding box to extremes so that any point will expand it.
///
/// Equivalent to C: `M_ClearBox(fixed_t *box)`
///
/// Sets TOP and RIGHT to `i32::MIN` (C `MININT` from `<values.h>`) — any
/// real coordinate is guaranteed to be greater, so the first point added
/// will always expand these boundaries.
///
/// Sets BOTTOM and LEFT to `i32::MAX` (C `MAXINT` from `<values.h>`) — any
/// real coordinate is guaranteed to be lesser, so the first point added
/// will always shrink these boundaries.
///
/// # Examples
///
/// ```
/// use doom_core::util::bbox::{clear_box, BBox, BOXTOP, BOXBOTTOM, BOXLEFT, BOXRIGHT};
/// use doom_core::types::fixed::Fixed;
///
/// let mut bbox: BBox = [Fixed(0); 4];
/// clear_box(&mut bbox);
/// assert_eq!(bbox[BOXTOP], Fixed(i32::MIN));
/// assert_eq!(bbox[BOXRIGHT], Fixed(i32::MIN));
/// assert_eq!(bbox[BOXBOTTOM], Fixed(i32::MAX));
/// assert_eq!(bbox[BOXLEFT], Fixed(i32::MAX));
/// ```
pub fn clear_box(bbox: &mut BBox) {
    bbox[BOXTOP] = Fixed(i32::MIN); // MININT
    bbox[BOXRIGHT] = Fixed(i32::MIN); // MININT
    bbox[BOXBOTTOM] = Fixed(i32::MAX); // MAXINT
    bbox[BOXLEFT] = Fixed(i32::MAX); // MAXINT
}

// ---------------------------------------------------------------------------
// M_AddToBox (from m_bbox.c lines 45-59)
// ---------------------------------------------------------------------------

/// Expand bounding box to include the given point `(x, y)`.
///
/// Equivalent to C: `M_AddToBox(fixed_t *box, fixed_t x, fixed_t y)`
///
/// For the X coordinate:
/// - If `x` is less than the current left boundary, the left boundary is
///   updated to `x`.
/// - Otherwise, if `x` is greater than the current right boundary, the
///   right boundary is updated to `x`.
///
/// For the Y coordinate:
/// - If `y` is less than the current bottom boundary, the bottom boundary
///   is updated to `y`.
/// - Otherwise, if `y` is greater than the current top boundary, the top
///   boundary is updated to `y`.
///
/// The comparison operators use `Fixed`'s derived [`PartialOrd`], which
/// compares the underlying `i32` values directly — matching the C behavior
/// of comparing `fixed_t` (int) values.
///
/// Note: The if-else chain structure is preserved exactly from the original
/// C code. Using separate `min`/`max` calls would change the behavior when
/// a point lies exactly on a boundary, so the original branching is kept.
///
/// # Examples
///
/// ```
/// use doom_core::util::bbox::{clear_box, add_to_box, BBox, BOXTOP, BOXBOTTOM, BOXLEFT, BOXRIGHT};
/// use doom_core::types::fixed::Fixed;
///
/// let mut bbox: BBox = [Fixed(0); 4];
/// clear_box(&mut bbox);
/// add_to_box(&mut bbox, Fixed(-10), Fixed(-20));
/// add_to_box(&mut bbox, Fixed(100), Fixed(200));
/// assert_eq!(bbox[BOXLEFT], Fixed(-10));
/// assert_eq!(bbox[BOXRIGHT], Fixed(100));
/// assert_eq!(bbox[BOXBOTTOM], Fixed(-20));
/// assert_eq!(bbox[BOXTOP], Fixed(200));
/// ```
pub fn add_to_box(bbox: &mut BBox, x: Fixed, y: Fixed) {
    if x < bbox[BOXLEFT] {
        bbox[BOXLEFT] = x;
    } else if x > bbox[BOXRIGHT] {
        bbox[BOXRIGHT] = x;
    }
    if y < bbox[BOXBOTTOM] {
        bbox[BOXBOTTOM] = y;
    } else if y > bbox[BOXTOP] {
        bbox[BOXTOP] = y;
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Constant values match original C enum --

    #[test]
    fn test_boxtop_index() {
        assert_eq!(BOXTOP, 0);
    }

    #[test]
    fn test_boxbottom_index() {
        assert_eq!(BOXBOTTOM, 1);
    }

    #[test]
    fn test_boxleft_index() {
        assert_eq!(BOXLEFT, 2);
    }

    #[test]
    fn test_boxright_index() {
        assert_eq!(BOXRIGHT, 3);
    }

    // -- clear_box --

    #[test]
    fn test_clear_box_sets_extremes() {
        let mut bbox: BBox = [Fixed(0); 4];
        clear_box(&mut bbox);
        // TOP and RIGHT should be MININT (most negative)
        assert_eq!(bbox[BOXTOP], Fixed(i32::MIN));
        assert_eq!(bbox[BOXRIGHT], Fixed(i32::MIN));
        // BOTTOM and LEFT should be MAXINT (most positive)
        assert_eq!(bbox[BOXBOTTOM], Fixed(i32::MAX));
        assert_eq!(bbox[BOXLEFT], Fixed(i32::MAX));
    }

    #[test]
    fn test_clear_box_idempotent() {
        let mut bbox: BBox = [Fixed(42); 4];
        clear_box(&mut bbox);
        let first = bbox;
        clear_box(&mut bbox);
        assert_eq!(bbox, first);
    }

    // -- add_to_box --

    #[test]
    fn test_add_single_point_to_cleared_box() {
        // After clearing, adding a single point sets LEFT and BOTTOM (because
        // the `if` branches fire: x < MAXINT and y < MAXINT). The `else if`
        // branches for RIGHT and TOP are NOT reached, so they remain at
        // i32::MIN. This matches the original C behavior — you need at least
        // two add_to_box calls to establish all four boundaries.
        let mut bbox: BBox = [Fixed(0); 4];
        clear_box(&mut bbox);
        add_to_box(&mut bbox, Fixed(0), Fixed(0));
        assert_eq!(bbox[BOXLEFT], Fixed(0));
        assert_eq!(bbox[BOXRIGHT], Fixed(i32::MIN)); // else-if not reached
        assert_eq!(bbox[BOXBOTTOM], Fixed(0));
        assert_eq!(bbox[BOXTOP], Fixed(i32::MIN)); // else-if not reached

        // Adding the same point again hits the else-if branches (0 > i32::MIN).
        add_to_box(&mut bbox, Fixed(0), Fixed(0));
        assert_eq!(bbox[BOXLEFT], Fixed(0));
        assert_eq!(bbox[BOXRIGHT], Fixed(0));
        assert_eq!(bbox[BOXBOTTOM], Fixed(0));
        assert_eq!(bbox[BOXTOP], Fixed(0));
    }

    #[test]
    fn test_add_point_expands_in_all_directions() {
        let mut bbox: BBox = [Fixed(0); 4];
        clear_box(&mut bbox);

        // First point: x=10 < MAXINT → LEFT=10; y=20 < MAXINT → BOTTOM=20.
        // RIGHT and TOP remain at i32::MIN (else-if not reached).
        add_to_box(&mut bbox, Fixed(10), Fixed(20));
        assert_eq!(bbox[BOXLEFT], Fixed(10));
        assert_eq!(bbox[BOXRIGHT], Fixed(i32::MIN));
        assert_eq!(bbox[BOXBOTTOM], Fixed(20));
        assert_eq!(bbox[BOXTOP], Fixed(i32::MIN));

        // Second point: x=-5 < LEFT(10) → LEFT=-5; y=-10 < BOTTOM(20) → BOTTOM=-10.
        // Still haven't hit else-if branches.
        add_to_box(&mut bbox, Fixed(-5), Fixed(-10));
        assert_eq!(bbox[BOXLEFT], Fixed(-5));
        assert_eq!(bbox[BOXRIGHT], Fixed(i32::MIN));
        assert_eq!(bbox[BOXBOTTOM], Fixed(-10));
        assert_eq!(bbox[BOXTOP], Fixed(i32::MIN));

        // Third point: x=100 > LEFT(-5)? No, skip if. x=100 > RIGHT(MIN)? Yes → RIGHT=100.
        // y=200 > BOTTOM(-10)? No, skip if. y=200 > TOP(MIN)? Yes → TOP=200.
        add_to_box(&mut bbox, Fixed(100), Fixed(200));
        assert_eq!(bbox[BOXLEFT], Fixed(-5));
        assert_eq!(bbox[BOXRIGHT], Fixed(100));
        assert_eq!(bbox[BOXBOTTOM], Fixed(-10));
        assert_eq!(bbox[BOXTOP], Fixed(200));
    }

    #[test]
    fn test_add_point_inside_does_not_change_box() {
        let mut bbox: BBox = [Fixed(0); 4];
        clear_box(&mut bbox);
        add_to_box(&mut bbox, Fixed(-100), Fixed(-100));
        add_to_box(&mut bbox, Fixed(100), Fixed(100));

        let snapshot = bbox;
        // Point (0, 0) is inside the box — should not change anything.
        add_to_box(&mut bbox, Fixed(0), Fixed(0));
        assert_eq!(bbox, snapshot);
    }

    #[test]
    fn test_add_point_on_boundary_does_not_change_box() {
        let mut bbox: BBox = [Fixed(0); 4];
        clear_box(&mut bbox);
        add_to_box(&mut bbox, Fixed(-10), Fixed(-20));
        add_to_box(&mut bbox, Fixed(10), Fixed(20));

        let snapshot = bbox;
        // Points exactly on each boundary should not change the box due to
        // the if-else chain structure (strict < and > comparisons).
        add_to_box(&mut bbox, Fixed(-10), Fixed(-20));
        assert_eq!(bbox, snapshot);
        add_to_box(&mut bbox, Fixed(10), Fixed(20));
        assert_eq!(bbox, snapshot);
    }

    #[test]
    fn test_add_negative_coordinates() {
        let mut bbox: BBox = [Fixed(0); 4];
        clear_box(&mut bbox);

        // First point: -65536 < MAXINT → LEFT=-65536; -131072 < MAXINT → BOTTOM=-131072.
        // RIGHT and TOP remain at i32::MIN.
        add_to_box(&mut bbox, Fixed(-65536), Fixed(-131072));
        assert_eq!(bbox[BOXLEFT], Fixed(-65536));
        assert_eq!(bbox[BOXRIGHT], Fixed(i32::MIN));
        assert_eq!(bbox[BOXBOTTOM], Fixed(-131072));
        assert_eq!(bbox[BOXTOP], Fixed(i32::MIN));

        // Second point: -32768 < LEFT(-65536)? No. -32768 > RIGHT(MIN)? Yes → RIGHT=-32768.
        // -65536 < BOTTOM(-131072)? No. -65536 > TOP(MIN)? Yes → TOP=-65536.
        add_to_box(&mut bbox, Fixed(-32768), Fixed(-65536));
        assert_eq!(bbox[BOXLEFT], Fixed(-65536));
        assert_eq!(bbox[BOXRIGHT], Fixed(-32768));
        assert_eq!(bbox[BOXBOTTOM], Fixed(-131072));
        assert_eq!(bbox[BOXTOP], Fixed(-65536));
    }

    #[test]
    fn test_bbox_type_is_array() {
        // BBox is just a type alias for [Fixed; 4] — verify it.
        let bbox: BBox = [Fixed(1), Fixed(2), Fixed(3), Fixed(4)];
        assert_eq!(bbox.len(), 4);
        assert_eq!(bbox[BOXTOP], Fixed(1));
        assert_eq!(bbox[BOXBOTTOM], Fixed(2));
        assert_eq!(bbox[BOXLEFT], Fixed(3));
        assert_eq!(bbox[BOXRIGHT], Fixed(4));
    }
}
