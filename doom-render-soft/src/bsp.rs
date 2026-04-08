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
//! BSP tree traversal for front-to-back subsector rendering.
//! Stub module — will be replaced with full implementation.

/// Maximum draw segments (from r_defs.h line 55).
pub const MAXDRAWSEGS: usize = 256;

/// BSP traversal state.
///
/// Consolidates all formerly-global variables from `r_bsp.c` into a single
/// owned struct.
pub struct BspState {
    _placeholder: (),
}

impl BspState {
    /// Creates a new `BspState` with default values.
    pub fn new() -> Self {
        Self { _placeholder: () }
    }
}

impl Default for BspState {
    fn default() -> Self {
        Self::new()
    }
}
