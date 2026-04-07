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

//! DOOM core game logic library.
//!
//! Contains deterministic gameplay, types, info tables, UI, and utilities.
//! This crate implements the platform-independent core of the DOOM engine,
//! translated from the original C source in linuxdoom-1.10/.

pub mod game;
pub mod info;
pub mod play;
pub mod traits;
pub mod types;
pub mod util;
pub mod video;
