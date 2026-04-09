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
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

//! # UI Module — User Interface and Presentation
//!
//! Contains all user interface and presentation code for DOOM:
//!
//! - [`menu`] — In-game menu system (New Game, Options, Load/Save, Quit)
//! - [`hud`] — Heads-up display (messages, chat, level titles)
//! - [`hud_lib`] — HUD widget primitives (text line, scrolling text, input)
//! - [`statusbar`] — Status bar (health, ammo, face, keys, armor)
//! - [`statusbar_lib`] — Status bar widget primitives (number, percent, icon)
//! - [`intermission`] — Intermission screens (stats, world map)
//! - [`automap`] — Automap overlay (map geometry, zoom, pan)
//! - [`finale`] — End-of-episode sequences (text scroll, bunny, cast call)
//! - [`wipe`] — Screen wipe transitions (melt effect)
//!
//! ## Architecture
//!
//! Each UI subsystem follows the DOOM Responder/Ticker/Drawer pattern:
//! - **Responder**: Processes input events, returns true if consumed
//! - **Ticker**: Advances animation state once per game tic (35 Hz)
//! - **Drawer**: Renders the current state to the screen buffer
//!
//! The game loop in `doom-core::game` dispatches to these functions
//! in the appropriate order each frame.
//!
//! ## Dependencies
//!
//! Depends on: `types/`, `info/`, `util/`, `video/`
//! Depended on by: `game/` (dispatches to UI responders/tickers/drawers)
//!
//! ## Original Source Files
//!
//! Translated from the following linuxdoom-1.10 files:
//! - m_menu.c/h, hu_stuff.c/h, hu_lib.c/h
//! - st_stuff.c/h, st_lib.c/h, wi_stuff.c/h
//! - am_map.c/h, f_finale.c/h, f_wipe.c/h

// ─── Sub-module declarations ─────────────────────────────────────
//
// Ordered: widget primitives first (used by the composite modules),
// then composite UI modules, then standalone UI subsystems.

/// HUD widget library primitives — text line, scrolling text, input text.
///
/// Provides [`HuTextLine`], [`HuScrollText`], and [`HuInputText`] widget
/// types used by the HUD module (`hud`) for rendering on-screen messages
/// and chat text.
///
/// Translated from `linuxdoom-1.10/hu_lib.c` and `hu_lib.h`.
pub mod hud_lib;

/// Status bar widget library primitives — number, percent, multi-icon, binary icon.
///
/// Provides [`StNumber`], [`StPercent`], [`StMultIcon`], and [`StBinIcon`]
/// widget types used by the status bar module (`statusbar`) for rendering
/// health, ammo, armor, keys, and weapon availability indicators.
///
/// Translated from `linuxdoom-1.10/st_lib.c` and `st_lib.h`.
pub mod statusbar_lib;

/// Heads-up display management — messages, chat, level titles.
///
/// Provides [`HudState`] and constants [`HU_FONTSTART`], [`HU_FONTEND`],
/// [`HU_FONTSIZE`] for HUD font access and message rendering.
///
/// Translated from `linuxdoom-1.10/hu_stuff.c` and `hu_stuff.h`.
pub mod hud;

/// DOOM status bar — health, ammo, face, keys, armor.
///
/// Provides [`StatusBarState`] and dimension constants [`ST_HEIGHT`],
/// [`ST_WIDTH`], [`ST_Y`] for status bar rendering and layout.
///
/// Translated from `linuxdoom-1.10/st_stuff.c` and `st_stuff.h`.
pub mod statusbar;

/// In-game menu system — New Game, Options, Load/Save, Quit.
///
/// Provides [`MenuState`] for game loop menu dispatching and state
/// management.
///
/// Translated from `linuxdoom-1.10/m_menu.c` and `m_menu.h`.
pub mod menu;

/// Intermission screens — stats, world map, level transition display.
///
/// Provides [`IntermissionState`] for between-level stats display and
/// world map animation.
///
/// Translated from `linuxdoom-1.10/wi_stuff.c` and `wi_stuff.h`.
pub mod intermission;

/// Automap overlay — map geometry, zoom, pan, follow mode.
///
/// Provides [`AutomapState`] and message protocol constants
/// [`AM_MSGHEADER`], [`AM_MSGENTERED`], [`AM_MSGEXITED`] used for
/// communication between the automap and status bar subsystems.
///
/// Translated from `linuxdoom-1.10/am_map.c` and `am_map.h`.
pub mod automap;

/// Game ending sequences — text scroll, bunny scroll, cast call.
///
/// Provides [`FinaleState`] for end-of-episode text scroll, bunny
/// scroll (Episode 3), and DOOM II cast call sequence management.
///
/// Translated from `linuxdoom-1.10/f_finale.c` and `f_finale.h`.
pub mod finale;

/// Screen wipe transitions — color crossfade and iconic melt effect.
///
/// Provides [`WipeState`] for wipe animation state and [`WipeType`]
/// enum for selecting between `ColorXForm` and `Melt` wipe effects.
///
/// Translated from `linuxdoom-1.10/f_wipe.c` and `f_wipe.h`.
pub mod wipe;

// ─── Convenience re-exports ──────────────────────────────────────
//
// Re-export the most commonly-used types from each sub-module for
// ergonomic access. Consumers can use `doom_core::ui::MenuState`
// instead of `doom_core::ui::menu::MenuState`.

// Widget types from hud_lib
pub use hud_lib::{HuInputText, HuScrollText, HuTextLine};

// Widget types from statusbar_lib
pub use statusbar_lib::{StBinIcon, StMultIcon, StNumber, StPercent};

// State structs from each UI subsystem
pub use automap::AutomapState;
pub use finale::FinaleState;
pub use hud::HudState;
pub use intermission::IntermissionState;
pub use menu::MenuState;
pub use statusbar::StatusBarState;
pub use wipe::{WipeState, WipeType};

// Key constants from hud (font character range)
pub use hud::{HU_FONTEND, HU_FONTSIZE, HU_FONTSTART};

// Automap message protocol constants
pub use automap::{AM_MSGENTERED, AM_MSGEXITED, AM_MSGHEADER};

// Status bar dimension constants
pub use statusbar::{ST_HEIGHT, ST_WIDTH, ST_Y};
