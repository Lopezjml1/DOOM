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

//! Translated from linuxdoom-1.10/r_plane.c and r_plane.h
//!
//! Visplane allocation and floor/ceiling span rendering.
//! Stub module — will be replaced with full implementation.

/// Maximum number of visplanes per frame (r_plane.h line 38).
///
/// Original C: `#define MAXVISPLANES 128`
pub const MAXVISPLANES: usize = 128;

/// Maximum number of screen openings per frame (r_plane.h line 41).
///
/// Sized as `SCREENWIDTH * 64` = `320 * 64` = 20480.
/// Original C: `#define MAXOPENINGS SCREENWIDTH*64`
pub const MAXOPENINGS: usize = 320 * 64;

/// Visplane rendering state.
///
/// Consolidates all formerly-global variables from `r_plane.c` into a single
/// owned struct.
pub struct PlaneState {
    _placeholder: (),
}

impl PlaneState {
    /// Creates a new `PlaneState` with default values.
    pub fn new() -> Self {
        Self { _placeholder: () }
    }
}

impl Default for PlaneState {
    fn default() -> Self {
        Self::new()
    }
}
