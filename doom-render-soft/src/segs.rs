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
//! Wall segment rendering and texture mapping.
//! Stub module — will be replaced with full implementation.

/// Wall segment rendering state.
///
/// Consolidates all formerly-global variables from `r_segs.c` into a single
/// owned struct.
pub struct SegsState {
    _placeholder: (),
}

impl SegsState {
    /// Creates a new `SegsState` with default values.
    pub fn new() -> Self {
        Self { _placeholder: () }
    }
}

impl Default for SegsState {
    fn default() -> Self {
        Self::new()
    }
}
