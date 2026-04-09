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

//! # Platform Abstraction Traits
//!
//! This module defines the trait-based platform abstraction layer — the Rust
//! equivalent of DOOM's original `i_*.h` interface headers. These traits define
//! the boundary between portable, deterministic game logic (in `doom-core`) and
//! platform-specific implementations (in `doom-platform-win`).
//!
//! ## Traits
//!
//! - [`PlatformHost`] — Window management, input, timing, lifecycle
//!   (from `i_system.h` + `i_video.h`)
//! - [`Renderer`] — 3D view rendering (from `r_main.h`)
//! - [`AudioBackend`] — Sound effects and music playback (from `i_sound.h`)
//! - [`WadProvider`] — WAD file access (re-exported from `doom-wad`)
//!
//! ## Design Principles
//!
//! - Traits use `&self`/`&mut self` (no static methods that would prevent trait objects)
//! - No platform-specific types in trait signatures — only doom-core types
//! - Traits are object-safe where practical to enable `dyn Trait` usage
//! - Each trait maps to one or more original `i_*.h` interface headers
//!
//! ## Dependency Direction
//!
//! - `doom-core::game` → calls platform methods through these traits
//! - `doom-platform-win` → implements `PlatformHost` and `AudioBackend`
//! - `doom-render-soft` → implements `Renderer`
//! - `doom-bin` → wires trait implementations together at the application level

// Submodule declarations — each contains one primary trait definition.
// All modules are `pub` because they are accessed by downstream crates
// (doom-platform-win, doom-render-soft, doom-bin).

/// Platform host trait: window management, input polling, timing, and lifecycle.
/// Translated from `linuxdoom-1.10/i_system.h` and `linuxdoom-1.10/i_video.h`.
pub mod platform;

/// Renderer trait: 3D view rendering from a player's perspective.
/// Translated from the REFRESH section of `linuxdoom-1.10/r_main.h` (lines 157-165).
pub mod renderer;

/// Audio backend trait: sound effect playback, music playback, and audio lifecycle.
/// Translated from `linuxdoom-1.10/i_sound.h`.
pub mod audio;

/// WAD provider trait: WAD file access contract (lump lookup, read, cache).
/// Re-exports `WadProvider` from the `doom-wad` crate, bridging it into the
/// `doom-core::traits` namespace. Translated from `linuxdoom-1.10/w_wad.h`.
pub mod wad;

// Convenience re-exports — allows consumers to write
// `use doom_core::traits::PlatformHost;` instead of
// `use doom_core::traits::platform::PlatformHost;`
pub use audio::AudioBackend;
pub use platform::PlatformHost;
pub use renderer::Renderer;
pub use wad::WadProvider;
