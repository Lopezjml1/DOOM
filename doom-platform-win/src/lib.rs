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

//! Windows 11 platform backend for DOOM using SDL2.
//!
//! Translated from linuxdoom-1.10/i_*.c files and sndserv/*.c.
//!
//! This crate implements the platform-specific layer that bridges the
//! deterministic game logic in `doom-core` with the host operating
//! system.  It provides:
//!
//! - **Window management** ([`window::WindowManager`]) — SDL2 window
//!   creation, event pump, and mouse grab.
//! - **Video output** ([`video::VideoOutput`]) — palettized 320×200
//!   framebuffer → ARGB8888 streaming texture → SDL2 canvas.
//!
//! Additional modules (audio, timer, filesystem, input) will be added
//! as subsequent implementation phases are completed.

pub mod audio;
pub mod filesystem;
pub mod timer;
pub mod video;
pub mod window;
