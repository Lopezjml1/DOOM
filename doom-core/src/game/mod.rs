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

//! Game flow control module: initialization, main loop, game state, networking, and strings.
//!
//! Translated from linuxdoom-1.10/d_main.h, g_game.h, d_net.h, dstrings.h
//!
//! This module aggregates the five child modules that comprise DOOM's game-level
//! orchestration layer:
//!
//! - [`game_main`] — Primary engine entry point (`D_DoomMain`), WAD file management,
//!   event processing, display rendering orchestration, and demo sequence cycling.
//!   Corresponds to `d_main.c` / `d_main.h`.
//!
//! - [`game_loop`] — The eternal game loop (`D_DoomLoop`) that drives frame I/O,
//!   tic processing, sound updates, and display rendering. Extracted from the
//!   `D_DoomLoop` function in `d_main.c`.
//!
//! - [`game_ctrl`] — Game state machine: new game initialization, save/load,
//!   demo recording/playback, level transitions, player rebirth, input-to-ticcmd
//!   building, and `G_Ticker` dispatch. Corresponds to `g_game.c` / `g_game.h`.
//!
//! - [`game_net`] — Network game communication and tic synchronization
//!   (`NetUpdate`, `TryRunTics`, `D_CheckNetGame`). Stubbed for single-player
//!   but preserves tic timing critical for demo playback. Corresponds to
//!   `d_net.c` / `d_net.h`.
//!
//! - [`strings`] — All user-visible text strings for DOOM (English and French),
//!   quit messages, save game name, and localization constants. Corresponds to
//!   `dstrings.c/h`, `d_englsh.h`, `d_french.h`.
//!
//! # Re-exports
//!
//! The primary types and the main loop entry function are re-exported at this
//! module level for convenient access:
//!
//! - [`GameMain`] — Consolidated engine state from `d_main.c`
//! - [`GameCtrl`] — Game state machine from `g_game.c`
//! - [`NetState`] — Network/tic synchronization state from `d_net.c`
//! - [`d_doom_loop`] — The main game loop function

// =========================================================================
// Child module declarations
// =========================================================================

/// Primary engine entry point and initialization.
/// See [`game_main::GameMain`] and [`game_main::d_doom_main`].
pub mod game_main;

/// The eternal game loop (`D_DoomLoop`).
/// See [`game_loop::d_doom_loop`].
pub mod game_loop;

/// Game state machine: new game, save/load, demo, level transitions.
/// See [`game_ctrl::GameCtrl`].
pub mod game_ctrl;

/// Network communication and tic synchronization (single-player stub).
/// See [`game_net::NetState`].
pub mod game_net;

/// User-visible text strings (English and French).
/// See [`strings::SAVEGAMENAME`], [`strings::ENDMSG`].
pub mod strings;

// =========================================================================
// Public re-exports for convenient access
// =========================================================================

/// Re-export of the primary engine state struct from [`game_main`].
pub use game_main::GameMain;

/// Re-export of the game state machine struct from [`game_ctrl`].
pub use game_ctrl::GameCtrl;

/// Re-export of the network/tic synchronization state from [`game_net`].
pub use game_net::NetState;

/// Re-export of the main game loop function from [`game_loop`].
pub use game_loop::d_doom_loop;
