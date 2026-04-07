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

//! Platform abstraction traits for DOOM engine.
//!
//! Defines the trait interfaces that decouple `doom-core` game logic from
//! platform-specific implementations. The `doom-platform-win` and
//! `doom-render-soft` crates provide concrete implementations.

pub mod audio;
pub mod platform;
pub mod wad;

pub use audio::AudioBackend;
pub use platform::PlatformHost;
pub use wad::WadProvider;
