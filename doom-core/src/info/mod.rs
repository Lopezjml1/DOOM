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

//! Massive data tables defining all game objects, state machines, sprites, and sounds.
//!
//! Translated from `linuxdoom-1.10/info.c`, `linuxdoom-1.10/info.h`,
//! `linuxdoom-1.10/sounds.c`, and `linuxdoom-1.10/sounds.h`.
//!
//! This module is the "database" of the DOOM engine. It contains:
//!
//! - [`sprites`] — Sprite name identifiers and 4-char lookup table (138 entries)
//! - [`sounds`] — Sound effect and music info enums and tables (109 SFX, 68 music)
//! - [`states`] — State machine table: animation frames, actions, transitions (967 entries)
//! - [`mobjinfo`] — Map object metadata: health, speed, size, sounds, flags (137 entries)
//!
//! These tables are primarily **static data** — very large arrays of struct literals
//! that must be reproduced exactly from the C source for behavioral parity.
//!
//! ## Dependencies
//! - Depends on: `crate::types` (Fixed, Angle, MobjFlags, ActionFn)
//! - Depended on by: `crate::play` (AI, spawning, damage), `crate::ui` (status bar,
//!   intermission), `crate::game` (initialization)

// ---------------------------------------------------------------------------
// Child module declarations.
//
// Dependency chain (informational — Rust resolves modules independently):
//   sprites and sounds are foundational (no intra-info dependencies),
//   states depends on sprites (SpriteNum),
//   mobjinfo depends on states (StateNum) and sounds (SfxEnum).
// ---------------------------------------------------------------------------

pub mod mobjinfo;
pub mod sounds;
pub mod sprites;
pub mod states;

// ---------------------------------------------------------------------------
// Convenience re-exports for ergonomic access by downstream modules.
//
// These allow other crates and modules to write:
//   use doom_core::info::StateNum;
// instead of the longer:
//   use doom_core::info::states::StateNum;
// ---------------------------------------------------------------------------

// Sprite identifiers and name lookup table.
pub use sprites::{SpriteNum, NUMSPRITES, SPRITE_NAMES};

// Sound effect and music metadata.
pub use sounds::{MusicEnum, MusicInfo, SfxEnum, SfxInfo, NUMMUSIC, NUMSFX, S_MUSIC, S_SFX};

// State machine table — animation frames, actions, transitions.
pub use states::{ActionFnId, State, StateNum, NUMSTATES, STATES};

// Map object type metadata — health, speed, collision, sounds, flags.
pub use mobjinfo::{MobjInfo, MobjType, MOBJINFO, NUMMOBJTYPES};
