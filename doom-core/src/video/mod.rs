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
//! Provides the 5 framebuffer screens (320×200×8-bit palettized), 2D drawing
//! primitives for patches and pixel blocks, dirty rectangle tracking, and
//! gamma correction lookup tables.
//!
//! This module operates exclusively on raw pixel buffers and does NOT interact
//! with any platform-specific display APIs. Actual screen presentation to the
//! user is handled by the platform crate (`doom-platform-win`) via traits.
//!
//! Translated from linuxdoom-1.10/v_video.c and linuxdoom-1.10/v_video.h.

// The child module is intentionally named `video` inside the parent `video`
// module to match the original C source file naming convention (`v_video.c`).
#[allow(clippy::module_inception)]
pub mod video;

// Convenience re-exports for ergonomic access.
//
// These allow consuming code to write:
//   `use doom_core::video::VideoState;`
// instead of:
//   `use doom_core::video::video::VideoState;`
pub use video::VideoState;
pub use video::CENTERY;
pub use video::GAMMATABLE;
pub use video::NUM_SCREENS;
