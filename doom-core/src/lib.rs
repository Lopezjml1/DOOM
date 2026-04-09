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

//! # doom-core — Deterministic Game Logic for DOOM 1.10
//!
//! This crate contains the platform-independent, deterministic game logic
//! translated from the original id Software DOOM 1.10 C source code
//! (`linuxdoom-1.10/`).
//!
//! ## Crate Organization
//!
//! - [`types`] — Core type definitions (fixed-point, angles, enums, structs)
//! - [`info`] — Massive data tables (states, sprites, mobjinfo, sounds)
//! - [`game`] — Game flow control (main loop, level transitions, demos)
//! - [`play`] — Gameplay simulation (physics, AI, map specials, thinkers)
//! - [`ui`] — User interface (menus, HUD, status bar, automap, finale)
//! - [`video`] — Video buffer management (screen buffers, patch drawing)
//! - [`util`] — Utilities (PRNG, bounding box, cheat codes, file I/O)
//! - [`traits`] — Platform abstraction traits (PlatformHost, Renderer, AudioBackend)
//!
//! ## Design Principles
//!
//! - **No platform dependencies**: All platform interaction goes through traits in [`traits`]
//! - **Deterministic**: PRNG tables, fixed-point arithmetic, and 35 tics/second timing
//!   produce identical results to the original C engine
//! - **Behavioral parity**: Every gameplay mechanic, rendering calculation, and AI routine
//!   faithfully reproduces the original DOOM 1.10 behavior
//!
//! ## Original Source
//!
//! Translated from approximately 90 C source files in `linuxdoom-1.10/`,
//! the id Software DOOM 1.10 source release (December 23, 1997).

// =============================================================================
// Module declarations — ordered from foundational to dependent
//
// The ordering reflects the logical dependency hierarchy:
//   types   → no internal deps (most foundational)
//   info    → depends on types
//   traits  → depends on types (trait signatures use core types)
//   util    → depends on types
//   video   → depends on types
//   play    → depends on types, info, util
//   ui      → depends on types, info, util, video
//   game    → depends on all above (most dependent)
// =============================================================================

/// Core type definitions: fixed-point arithmetic, angle measurement, game mode/skill
/// enums, player structures, map geometry, tic commands, events, and network protocol.
///
/// Translated from `linuxdoom-1.10/doomdef.h`, `doomtype.h`, `m_fixed.h`, `tables.h`,
/// `d_ticcmd.h`, `d_event.h`, `d_player.h`, `p_mobj.h`, `d_think.h`, `doomdata.h`,
/// `r_defs.h`, `d_net.h`.
pub mod types;

/// Massive gameplay data tables: state machine (967 entries), sprite names (138 entries),
/// map object metadata (137 entries), and sound/music definitions (109 SFX, 68 music).
///
/// Translated from `linuxdoom-1.10/info.c`, `info.h`, `sounds.c`, `sounds.h`.
pub mod info;

/// Platform abstraction traits defining the boundary between portable game logic and
/// platform-specific implementations: `PlatformHost`, `Renderer`, `AudioBackend`, `WadProvider`.
///
/// Translated from `linuxdoom-1.10/i_system.h`, `i_video.h`, `i_sound.h`, `w_wad.h`.
pub mod traits;

/// General-purpose utility modules: command-line parsing, bounding box operations,
/// cheat code detection, file I/O and configuration, deterministic PRNG, endian utilities.
///
/// Translated from `linuxdoom-1.10/m_argv.c/h`, `m_bbox.c/h`, `m_cheat.c/h`,
/// `m_misc.c/h`, `m_random.c/h`, `m_swap.c/h`.
pub mod util;

/// Video buffer management: five 320×200 framebuffer screens, 2D patch and pixel block
/// drawing primitives, dirty rectangle tracking, and gamma correction lookup tables.
///
/// Translated from `linuxdoom-1.10/v_video.c` and `v_video.h`.
pub mod video;

/// Gameplay simulation: physics, AI, collision detection, map specials (doors, floors,
/// ceilings, lifts, switches, teleports), thinker loop, line-of-sight, level setup,
/// save/load serialization.
///
/// Translated from 20 `p_*.c/h` files in `linuxdoom-1.10/`.
pub mod play;

/// User interface: in-game menus, HUD messages and chat, status bar (health/ammo/face/keys),
/// intermission statistics, automap overlay, finale sequences, screen wipe transitions.
///
/// Translated from `linuxdoom-1.10/m_menu.c/h`, `hu_stuff.c/h`, `hu_lib.c/h`,
/// `st_stuff.c/h`, `st_lib.c/h`, `wi_stuff.c/h`, `am_map.c/h`, `f_finale.c/h`,
/// `f_wipe.c/h`.
pub mod ui;

/// Game flow control: engine initialization (`D_DoomMain`), main loop (`D_DoomLoop`),
/// game state machine (new game, save/load, demos, level transitions), network tic
/// synchronization (single-player stub), and localized string tables.
///
/// Translated from `linuxdoom-1.10/d_main.c/h`, `g_game.c/h`, `d_net.c/h`,
/// `dstrings.c/h`, `d_englsh.h`, `d_french.h`.
pub mod game;

// =============================================================================
// Convenience re-exports — most commonly-used types for ergonomic downstream access
//
// These re-exports allow downstream crates to write:
//   use doom_core::Fixed;
// instead of the longer:
//   use doom_core::types::fixed::Fixed;
// =============================================================================

// Fixed-point 16.16 arithmetic newtype
pub use types::fixed::Fixed;

// Binary Angle Measurement (BAM) newtype
pub use types::angle::Angle;

// Core engine enums from doomdef
pub use types::doomdef::{GameMission, GameMode, GameState, Skill};

// Core engine constants from doomdef
pub use types::doomdef::{MAXPLAYERS, SCREENHEIGHT, SCREENWIDTH, TICRATE, VERSION};

// Per-tick input command structure
pub use types::ticcmd::TicCmd;

// Input event type
pub use types::event::Event;

// Player state structure
pub use types::player::Player;
