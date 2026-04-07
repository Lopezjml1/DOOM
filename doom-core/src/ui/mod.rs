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

//! UI / presentation subsystem modules.
//!
//! Contains all user-interface elements for the DOOM engine including
//! the status bar, HUD, menus, intermission screens, automap, finale,
//! and screen wipe transitions.

pub mod statusbar_lib;

// Convenience re-exports for status bar widget types.
pub use statusbar_lib::{StBinIcon, StMultIcon, StNumber, StPercent};
