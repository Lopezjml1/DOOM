// Copyright (C) 1993-1996 by id Software, Inc.
// Copyright (C) 2024 by the DOOM Rust port contributors.
//
// This source is available for distribution and/or modification
// only under the terms of the GNU General Public License v2 as
// published by the Free Software Foundation. All rights reserved.
//
// This source is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License
// for more details.

//! Translated from `linuxdoom-1.10/d_event.h`
//!
//! Input event types, game actions, and button code definitions.
//!
//! This module defines the event system used for input handling (keyboard,
//! mouse, joystick), game actions that drive the main loop state machine,
//! and button/action code bitmask constants used in tic commands.
//!
//! The event system is the primary interface between the platform layer
//! (`doom-platform-win`) and the deterministic game logic (`doom-core`).

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of events that can be queued in the event buffer.
///
/// Corresponds to `#define MAXEVENTS 64` in `d_event.h` line 108.
pub const MAXEVENTS: usize = 64;

// ---------------------------------------------------------------------------
// EventType — Input event classification
// ---------------------------------------------------------------------------

/// Input event types corresponding to the C `evtype_t` enum.
///
/// Each variant maps 1:1 to the original enumeration values so that any
/// integer-based dispatch logic in the engine translates directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EventType {
    /// A key was pressed (`ev_keydown`).
    KeyDown = 0,
    /// A key was released (`ev_keyup`).
    KeyUp = 1,
    /// Mouse movement or button event (`ev_mouse`).
    Mouse = 2,
    /// Joystick movement or button event (`ev_joystick`).
    Joystick = 3,
}

// ---------------------------------------------------------------------------
// Event — Input event payload
// ---------------------------------------------------------------------------

/// A single input event passed from the platform layer into the game logic.
///
/// Translated from the C `event_t` struct (`d_event.h` lines 44-50).
///
/// The interpretation of `data1`, `data2`, and `data3` depends on the
/// `event_type`:
///
/// | EventType   | data1              | data2            | data3            |
/// |-------------|--------------------|------------------|------------------|
/// | KeyDown     | key code           | 0                | 0                |
/// | KeyUp       | key code           | 0                | 0                |
/// | Mouse       | button bitmask     | x movement       | y movement       |
/// | Joystick    | button bitmask     | x movement       | y movement       |
#[derive(Debug, Clone, Copy)]
pub struct Event {
    /// The type of input event.
    pub event_type: EventType,
    /// Key code, or mouse/joystick button bitmask.
    pub data1: i32,
    /// Mouse or joystick horizontal (x) movement.
    pub data2: i32,
    /// Mouse or joystick vertical (y) movement.
    pub data3: i32,
}

impl Default for Event {
    /// Returns a default event representing a no-op key-down with all data
    /// fields zeroed, matching the zero-initialized `event_t` in C.
    #[inline]
    fn default() -> Self {
        Self {
            event_type: EventType::KeyDown,
            data1: 0,
            data2: 0,
            data3: 0,
        }
    }
}

impl Event {
    /// Creates a new `Event` with the given type and data fields.
    #[inline]
    pub const fn new(event_type: EventType, data1: i32, data2: i32, data3: i32) -> Self {
        Self {
            event_type,
            data1,
            data2,
            data3,
        }
    }
}

// ---------------------------------------------------------------------------
// GameAction — High-level game state transitions
// ---------------------------------------------------------------------------

/// Game actions that drive the main loop state machine.
///
/// Translated from the C `gameaction_t` enum (`d_event.h` lines 53-65).
/// The game loop inspects the current action each tic and performs the
/// corresponding state transition (load level, save game, play demo, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum GameAction {
    /// No pending action (`ga_nothing`).
    Nothing = 0,
    /// Load a level (`ga_loadlevel`).
    LoadLevel = 1,
    /// Start a new game (`ga_newgame`).
    NewGame = 2,
    /// Load a saved game (`ga_loadgame`).
    LoadGame = 3,
    /// Save the current game (`ga_savegame`).
    SaveGame = 4,
    /// Begin demo playback (`ga_playdemo`).
    PlayDemo = 5,
    /// Level completed — proceed to intermission (`ga_completed`).
    Completed = 6,
    /// Ultimate victory — show finale (`ga_victory`).
    Victory = 7,
    /// Inter-level world transition done (`ga_worlddone`).
    WorldDone = 8,
    /// Take a screenshot (`ga_screenshot`).
    Screenshot = 9,
}

impl Default for GameAction {
    /// Returns `GameAction::Nothing`, matching the C convention of
    /// initialising `gameaction` to `ga_nothing`.
    #[inline]
    fn default() -> Self {
        Self::Nothing
    }
}

// ---------------------------------------------------------------------------
// Button / action code constants
// ---------------------------------------------------------------------------
//
// In the original C source these are declared as members of the
// `buttoncode_t` enum, but they are actually used as bitmask flags and
// shift amounts applied to the `buttons` field of `ticcmd_t`.  Plain
// constants are more appropriate in Rust because the values mix button
// flags with special codes and shift counts.

/// Press "Fire" button.
///
/// Corresponds to `BT_ATTACK = 1` in `d_event.h` line 75.
pub const BT_ATTACK: u8 = 1;

/// Use button — open doors, activate switches.
///
/// Corresponds to `BT_USE = 2` in `d_event.h` line 77.
pub const BT_USE: u8 = 2;

/// Flag indicating game events (not really buttons).
///
/// When this bit is set in the `buttons` byte, the lower bits encode
/// a special command rather than a player action.
///
/// Corresponds to `BT_SPECIAL = 128` in `d_event.h` line 80.
pub const BT_SPECIAL: u8 = 128;

/// Mask applied when `BT_SPECIAL` is set to extract the special command.
///
/// Corresponds to `BT_SPECIALMASK = 3` in `d_event.h` line 81.
pub const BT_SPECIALMASK: u8 = 3;

/// Flag indicating a weapon change is pending.
///
/// If set, the next 3 bits (masked by `BT_WEAPONMASK`) hold the weapon
/// number.
///
/// Corresponds to `BT_CHANGE = 4` in `d_event.h` line 85.
pub const BT_CHANGE: u8 = 4;

/// 3-bit mask for extracting the weapon number from the button byte.
///
/// `BT_WEAPONMASK = 8 + 16 + 32 = 56 = 0x38`.
///
/// Corresponds to `BT_WEAPONMASK = (8+16+32)` in `d_event.h` line 87.
pub const BT_WEAPONMASK: u8 = 8 + 16 + 32;

/// Bit shift count to align the weapon number within the button byte.
///
/// Corresponds to `BT_WEAPONSHIFT = 3` in `d_event.h` line 88.
pub const BT_WEAPONSHIFT: u8 = 3;

/// Pause the game (special button command).
///
/// Corresponds to `BTS_PAUSE = 1` in `d_event.h` line 91.
pub const BTS_PAUSE: u8 = 1;

/// Save the game at each console (special button command).
///
/// Corresponds to `BTS_SAVEGAME = 2` in `d_event.h` line 93.
pub const BTS_SAVEGAME: u8 = 2;

/// Mask for extracting the save-game slot number from the second byte.
///
/// `BTS_SAVEMASK = 4 + 8 + 16 = 28 = 0x1C`.
///
/// Corresponds to `BTS_SAVEMASK = (4+8+16)` in `d_event.h` line 97.
pub const BTS_SAVEMASK: u8 = 4 + 8 + 16;

/// Bit shift count for the save-game slot number.
///
/// Corresponds to `BTS_SAVESHIFT = 2` in `d_event.h` line 98.
pub const BTS_SAVESHIFT: u8 = 2;

// ---------------------------------------------------------------------------
// Unit-testable assertions (compile-time sanity checks)
// ---------------------------------------------------------------------------

// Verify button code constant values at compile time.
const _: () = assert!(BT_WEAPONMASK == 56, "BT_WEAPONMASK must equal 56");
const _: () = assert!(BTS_SAVEMASK == 28, "BTS_SAVEMASK must equal 28");
const _: () = assert!(MAXEVENTS == 64, "MAXEVENTS must equal 64");
