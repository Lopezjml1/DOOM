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

//! Core type definitions for the DOOM engine.
//!
//! This module contains all fundamental data types, enums, constants, and structures
//! used throughout the engine. Every other module depends on types defined here.
//!
//! Translated from the shared C header files in `linuxdoom-1.10/`:
//! `doomdef.h`, `doomtype.h`, `m_fixed.h`, `tables.h`, `d_ticcmd.h`, `d_event.h`,
//! `d_player.h`, `p_mobj.h`, `d_think.h`, `doomdata.h`, `r_defs.h`, `d_net.h`.

// =============================================================================
// Child module declarations — ordered from foundational to dependent
// =============================================================================

/// Fixed-point 16.16 arithmetic newtype and operations.
/// Translated from `linuxdoom-1.10/m_fixed.h` and `m_fixed.c`.
pub mod fixed;

/// Binary Angle Measurement (BAM) newtype and angle constants.
/// Translated from `linuxdoom-1.10/tables.h` (angle portion).
pub mod angle;

/// Precomputed trigonometric lookup tables (finesine, finetangent, tantoangle).
/// Translated from `linuxdoom-1.10/tables.c` and `tables.h`.
pub mod tables;

/// Central engine definitions: game modes, skills, weapon/ammo/power enums, constants.
/// Translated from `linuxdoom-1.10/doomdef.h` and `doomdef.c`.
pub mod doomdef;

/// Basic type aliases (Byte) and min/max constants.
/// Translated from `linuxdoom-1.10/doomtype.h`.
pub mod doomtype;

/// Per-tick input command structure.
/// Translated from `linuxdoom-1.10/d_ticcmd.h`.
pub mod ticcmd;

/// Input event types, game actions, and button code definitions.
/// Translated from `linuxdoom-1.10/d_event.h`.
pub mod event;

/// Thinker linked list management and action function dispatch.
/// Translated from `linuxdoom-1.10/d_think.h`.
pub mod thinker;

/// Map Object (mobj) entity system: position, state, flags, AI.
/// Translated from `linuxdoom-1.10/p_mobj.h`.
pub mod mobj;

/// Map geometry structures for WAD format and runtime representation.
/// Translated from `linuxdoom-1.10/doomdata.h` and `r_defs.h`.
pub mod map_data;

/// Player state: health, armor, weapons, ammo, powers, intermission data.
/// Translated from `linuxdoom-1.10/d_player.h`.
pub mod player;

/// Network protocol structures: DoomCom, DoomData, command codes.
/// Translated from `linuxdoom-1.10/d_net.h`.
pub mod net;

// =============================================================================
// Convenience re-exports — most commonly used types for ergonomic access
// =============================================================================

// Fixed-point arithmetic
pub use fixed::Fixed;

// Angle type
pub use angle::Angle;

// Core engine enums from doomdef
pub use doomdef::{AmmoType, Card, PowerType, WeaponType};
pub use doomdef::{GameMission, GameMode, GameState, Language, Skill};

// Core engine constants from doomdef
pub use doomdef::{MAXPLAYERS, SCREENHEIGHT, SCREENWIDTH, TICRATE, VERSION};

// Basic type aliases
pub use doomtype::Byte;

// Per-tick command
pub use ticcmd::TicCmd;

// Event system
pub use event::{Event, EventType, GameAction};

// Thinker system
pub use thinker::Thinker;

// Map object entity
pub use mobj::MapObject;

// Player state
pub use player::Player;

// Map geometry (most commonly used runtime types)
pub use map_data::{LineDef, MapThing, Node, Sector, Seg, SideDef, Subsector, Vertex};

// Network protocol
pub use net::{DoomCom, DoomData};
