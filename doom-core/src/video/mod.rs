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

//! Video buffer management module.
//!
//! Translated from `linuxdoom-1.10/v_video.c` and `linuxdoom-1.10/v_video.h`.
//!
//! This module manages the 5 framebuffer screens (320×200×8-bit palettized),
//! provides 2D drawing primitives for patches and raw pixel blocks, handles
//! dirty rectangle tracking, and contains the gamma correction lookup tables.
//!
//! This module operates exclusively on raw pixel data in memory — it has NO
//! platform-specific display API interaction. Actual screen presentation is
//! handled by the platform crate via traits.

#[allow(clippy::module_inception)]
pub mod video;

// Re-export key types for ergonomic access.
pub use video::{VideoState, CENTERY, GAMMATABLE, NUM_SCREENS};
