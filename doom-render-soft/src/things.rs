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
//! Sprite sorting and masked column compositing.
//! Stub module — will be replaced with full implementation.

use doom_core::types::fixed::FRACUNIT;

/// Maximum number of visible sprites per frame (r_things.c line 65).
///
/// Original C: `#define MAXVISSPRITES 128`
pub const MAXVISSPRITES: usize = 128;

/// Minimum Z distance for sprite rendering (r_things.h line 22).
///
/// Sprites closer than 4 fracunits are clamped to prevent division
/// by zero in perspective projection.
///
/// Original C: `#define MINZ (FRACUNIT*4)` (r_things.h line 22)
pub const MINZ: i32 = FRACUNIT * 4;

/// Base Y center for sprite rendering (r_things.h line 23).
///
/// Vertical center of the screen in pixels (200 / 2 = 100).
///
/// Original C: `#define BASEYCENTER 100`
pub const BASEYCENTER: i32 = 100;

/// Sprite sorting and rendering state.
///
/// Consolidates all formerly-global variables from `r_things.c` into a single
/// owned struct.
pub struct ThingsState {
    _placeholder: (),
}

impl ThingsState {
    /// Creates a new `ThingsState` with default values.
    pub fn new() -> Self {
        Self { _placeholder: () }
    }
}

impl Default for ThingsState {
    fn default() -> Self {
        Self::new()
    }
}
