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

//! Translated from linuxdoom-1.10/doomdef.h and doomdef.c
//!
//! Internally used data structures for virtually everything,
//! key definitions, lots of other stuff.
//!
//! This module contains the central engine definitions that nearly every other
//! module depends on: game version, screen dimensions, timing, game mode and
//! mission enumerations, skill levels, key card types, weapon types, ammunition
//! types, power-up types and durations, keyboard scan-code constants, and the
//! weapon info table mapping weapons to their animation states.
//!
//! # Constants Behavioral Contract
//!
//! The constants defined here (`VERSION`, `SCREENWIDTH`, `SCREENHEIGHT`,
//! `TICRATE`, `MAXPLAYERS`) are critical engine parameters. Changing them
//! would break demo playback compatibility, savegame compatibility, and
//! networking protocol compatibility with the original engine.

use crate::info::states::StateNum;

// =============================================================================
// Global parameters/defines
// =============================================================================

/// Engine version identifier: DOOM 1.10.
pub const VERSION: i32 = 110;

/// Base render buffer width before any scaling factor.
pub const BASE_WIDTH: i32 = 320;

/// Screen scaling multiplier. Drawing of status bar, menus, etc. is tied to the
/// scale implied by the graphics; changing this would require rewriting all UI
/// drawing code.
pub const SCREEN_MUL: i32 = 1;

/// Native render buffer width in pixels.
///
/// The software renderer draws into a 320-pixel-wide buffer. The platform
/// backend scales this to the actual window size.
pub const SCREENWIDTH: i32 = 320;

/// Native render buffer height in pixels.
///
/// The software renderer draws into a 200-pixel-tall buffer (320×200, matching
/// the original VGA Mode 13h resolution).
pub const SCREENHEIGHT: i32 = 200;

/// Maximum number of simultaneous players in a multiplayer game.
pub const MAXPLAYERS: usize = 4;

/// Game simulation ticks per second. The entire game loop is driven at this
/// fixed rate: physics, AI, input sampling, and rendering all synchronize to
/// 35 tics/second.
pub const TICRATE: i32 = 35;

// =============================================================================
// Game mode handling — identify IWAD version
// =============================================================================

/// Identifies the IWAD type to handle IWAD-dependent animations, level counts,
/// and feature gating.
///
/// Translated from C `GameMode_t` in doomdef.h lines 38-47.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GameMode {
    /// DOOM 1 shareware — Episode 1 only, 9 maps.
    Shareware = 0,
    /// DOOM 1 registered — Episodes 1-3, 27 maps.
    Registered = 1,
    /// DOOM 2 retail — 1 episode, 34 maps.
    Commercial = 2,
    /// DOOM 1 retail (Ultimate DOOM) — Episodes 1-4, 36 maps.
    Retail = 3,
    /// No IWAD found; game mode not yet determined.
    Indetermined = 4,
}

impl Default for GameMode {
    /// Returns [`GameMode::Indetermined`] — the initial state before IWAD
    /// identification.
    fn default() -> Self {
        GameMode::Indetermined
    }
}

// =============================================================================
// Mission packs
// =============================================================================

/// Identifies the specific game mission / expansion pack.
///
/// Translated from C `GameMission_t` in doomdef.h lines 51-59.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GameMission {
    /// Original DOOM (1993).
    Doom = 0,
    /// DOOM II: Hell on Earth (1994).
    Doom2 = 1,
    /// TNT: Evilution mission pack (Final DOOM, 1996).
    PackTnt = 2,
    /// The Plutonia Experiment mission pack (Final DOOM, 1996).
    PackPlut = 3,
    /// No mission identified.
    None = 4,
}

// =============================================================================
// Language selection
// =============================================================================

/// Language for software localization of in-game text.
///
/// Translated from C `Language_t` in doomdef.h lines 63-70.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Language {
    /// English text strings.
    English = 0,
    /// French text strings.
    French = 1,
    /// German text strings.
    German = 2,
    /// Unknown / not determined.
    Unknown = 3,
}

// =============================================================================
// Game state
// =============================================================================

/// The current high-level state of the game engine, controlling which
/// subsystems (ticker, drawer, responder) are active.
///
/// Translated from C `gamestate_t` in doomdef.h lines 127-133.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GameState {
    /// Playing a level — gameplay simulation is active.
    Level = 0,
    /// Intermission screen between levels — stats display.
    Intermission = 1,
    /// Finale sequence — end-of-episode text, bunny scroll, or cast call.
    Finale = 2,
    /// Demo screen / title sequence — attract mode.
    DemoScreen = 3,
}

// =============================================================================
// Difficulty / skill settings / filters
// =============================================================================

/// Map Thing Flag: thing appears on skill levels 1 & 2 (Baby / Easy).
pub const MTF_EASY: i32 = 1;

/// Map Thing Flag: thing appears on skill level 3 (Medium / Hurt Me Plenty).
pub const MTF_NORMAL: i32 = 2;

/// Map Thing Flag: thing appears on skill levels 4 & 5 (Hard / Nightmare).
pub const MTF_HARD: i32 = 4;

/// Map Thing Flag: deaf monster — does not react to sound, only to sight.
pub const MTF_AMBUSH: i32 = 8;

/// Difficulty / skill level selection.
///
/// Translated from C `skill_t` in doomdef.h lines 148-154.
/// Derives `PartialOrd` and `Ord` because skill comparisons are used
/// throughout the game logic for difficulty-based branching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum Skill {
    /// "I'm too young to die" — double ammo pickups, half damage taken.
    Baby = 0,
    /// "Hey, not too rough" — normal ammo, normal damage.
    Easy = 1,
    /// "Hurt me plenty" — default difficulty.
    Medium = 2,
    /// "Ultra-Violence" — more and tougher monsters.
    Hard = 3,
    /// Nightmare — fast monsters, respawning enemies, no saving.
    Nightmare = 4,
}

// =============================================================================
// Key cards
// =============================================================================

/// Key card / skull key types used for locked doors and switches.
///
/// Translated from C `card_t` in doomdef.h lines 162-173.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Card {
    /// Blue keycard.
    BlueCard = 0,
    /// Yellow keycard.
    YellowCard = 1,
    /// Red keycard.
    RedCard = 2,
    /// Blue skull key.
    BlueSkull = 3,
    /// Yellow skull key.
    YellowSkull = 4,
    /// Red skull key.
    RedSkull = 5,
}

/// Total number of key card / skull key types.
pub const NUMCARDS: usize = 6;

// =============================================================================
// Weapons
// =============================================================================

/// Weapon type identifiers.
///
/// Translated from C `weapontype_t` in doomdef.h lines 180-197.
/// Note: `NoChange` is a sentinel value indicating no pending weapon change;
/// it is NOT included in the `NUMWEAPONS` count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum WeaponType {
    /// Bare fists (or berserk punch).
    Fist = 0,
    /// Pistol — starting weapon.
    Pistol = 1,
    /// Pump-action shotgun.
    Shotgun = 2,
    /// Chaingun (rapid-fire).
    Chaingun = 3,
    /// Rocket launcher.
    Missile = 4,
    /// Plasma rifle.
    Plasma = 5,
    /// BFG 9000.
    Bfg = 6,
    /// Chainsaw.
    Chainsaw = 7,
    /// Super shotgun (DOOM II only).
    SuperShotgun = 8,
    /// Sentinel: no pending weapon change. Not a real weapon.
    NoChange = 9,
}

/// Number of actual weapon types (excludes `NoChange` sentinel).
/// Equals `wp_supershotgun + 1` in the original C.
pub const NUMWEAPONS: usize = 9;

// =============================================================================
// Ammunition
// =============================================================================

/// Ammunition type identifiers.
///
/// Translated from C `ammotype_t` in doomdef.h lines 200-210.
/// `NoAmmo` is a sentinel for weapons that consume no ammunition (fist,
/// chainsaw); it is NOT included in the `NUMAMMO` count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum AmmoType {
    /// Bullets — used by pistol and chaingun.
    Clip = 0,
    /// Shells — used by shotgun and super shotgun.
    Shell = 1,
    /// Energy cells — used by plasma rifle and BFG.
    Cell = 2,
    /// Rockets — used by rocket launcher.
    Missile = 3,
    /// No ammo consumed — fist, chainsaw.
    NoAmmo = 4,
}

/// Number of actual ammunition types (excludes `NoAmmo` sentinel).
pub const NUMAMMO: usize = 4;

// =============================================================================
// Power-up artifacts
// =============================================================================

/// Power-up types that can be active on a player.
///
/// Translated from C `powertype_t` in doomdef.h lines 214-224.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PowerType {
    /// Invulnerability sphere — immune to all damage.
    Invulnerability = 0,
    /// Berserk pack — enhanced punch damage + health boost.
    Strength = 1,
    /// Partial invisibility — monsters have reduced aim accuracy.
    Invisibility = 2,
    /// Radiation suit — immune to floor damage (nukage, lava).
    IronFeet = 3,
    /// Computer area map — reveals full automap.
    AllMap = 4,
    /// Light amplification visor — full brightness rendering.
    InfraRed = 5,
}

/// Number of power-up types.
pub const NUMPOWERS: usize = 6;

// =============================================================================
// Power-up durations
// =============================================================================
// How many tics until a power-up expires, assuming TICRATE = 35 tics/second.
// Translated from C `powerduration_t` enum in doomdef.h lines 233-240.

/// Invulnerability duration: 30 seconds = 1050 tics.
pub const INVULNTICS: i32 = 30 * TICRATE;

/// Invisibility (partial) duration: 60 seconds = 2100 tics.
pub const INVISTICS: i32 = 60 * TICRATE;

/// Infrared / light-amp visor duration: 120 seconds = 4200 tics.
pub const INFRATICS: i32 = 120 * TICRATE;

/// Radiation suit (iron feet) duration: 60 seconds = 2100 tics.
pub const IRONTICS: i32 = 60 * TICRATE;

// =============================================================================
// DOOM keyboard definitions
// =============================================================================
// This is the stuff configured by Setup.Exe.
// Most key data are simple ASCII (upper-cased).
// Translated from doomdef.h lines 250-280.

/// Right arrow key scan code.
pub const KEY_RIGHTARROW: i32 = 0xae;
/// Left arrow key scan code.
pub const KEY_LEFTARROW: i32 = 0xac;
/// Up arrow key scan code.
pub const KEY_UPARROW: i32 = 0xad;
/// Down arrow key scan code.
pub const KEY_DOWNARROW: i32 = 0xaf;
/// Escape key.
pub const KEY_ESCAPE: i32 = 27;
/// Enter / Return key.
pub const KEY_ENTER: i32 = 13;
/// Tab key.
pub const KEY_TAB: i32 = 9;

/// Function key F1.
pub const KEY_F1: i32 = 0x80 + 0x3b;
/// Function key F2.
pub const KEY_F2: i32 = 0x80 + 0x3c;
/// Function key F3.
pub const KEY_F3: i32 = 0x80 + 0x3d;
/// Function key F4.
pub const KEY_F4: i32 = 0x80 + 0x3e;
/// Function key F5.
pub const KEY_F5: i32 = 0x80 + 0x3f;
/// Function key F6.
pub const KEY_F6: i32 = 0x80 + 0x40;
/// Function key F7.
pub const KEY_F7: i32 = 0x80 + 0x41;
/// Function key F8.
pub const KEY_F8: i32 = 0x80 + 0x42;
/// Function key F9.
pub const KEY_F9: i32 = 0x80 + 0x43;
/// Function key F10.
pub const KEY_F10: i32 = 0x80 + 0x44;
/// Function key F11.
pub const KEY_F11: i32 = 0x80 + 0x57;
/// Function key F12.
pub const KEY_F12: i32 = 0x80 + 0x58;

/// Backspace / Delete key.
pub const KEY_BACKSPACE: i32 = 127;
/// Pause key.
pub const KEY_PAUSE: i32 = 0xff;

/// Equals sign key ('=').
pub const KEY_EQUALS: i32 = 0x3d;
/// Minus / hyphen key ('-').
pub const KEY_MINUS: i32 = 0x2d;

/// Right Shift key.
pub const KEY_RSHIFT: i32 = 0x80 + 0x36;
/// Right Control key.
pub const KEY_RCTRL: i32 = 0x80 + 0x1d;
/// Right Alt key.
pub const KEY_RALT: i32 = 0x80 + 0x38;
/// Left Alt key (aliased to Right Alt in the original engine).
pub const KEY_LALT: i32 = KEY_RALT;

// =============================================================================
// Range check flag
// =============================================================================

/// When true, enables parameter validation / range-checking debugging code
/// throughout the engine. Translated from `#define RANGECHECK` in doomdef.h.
pub const RANGECHECK: bool = true;

// =============================================================================
// Weapon info — sprite frames, ammunition use
// =============================================================================
// Translated from linuxdoom-1.10/d_items.h (struct) and d_items.c (table).
// Placed here in doomdef.rs because it depends on AmmoType and WeaponType
// which are defined above, and on StateNum from the info module.

/// Weapon animation and resource information.
///
/// Each entry describes the ammunition type and the animation state indices
/// for a single weapon. Translated from C `weaponinfo_t` in d_items.h.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponInfo {
    /// Ammunition type consumed by this weapon.
    pub ammo: AmmoType,
    /// State entered when raising (selecting) this weapon.
    pub upstate: StateNum,
    /// State entered when lowering (deselecting) this weapon.
    pub downstate: StateNum,
    /// State when weapon is ready and idle.
    pub readystate: StateNum,
    /// State entered when the fire button is pressed.
    pub atkstate: StateNum,
    /// State for the muzzle flash overlay (or `S_NULL` if none).
    pub flashstate: StateNum,
}

/// Static weapon information table indexed by weapon type ordinal.
///
/// Contains exactly `NUMWEAPONS` (9) entries, one per real weapon type
/// (fist through super shotgun). The `NoChange` sentinel weapon type
/// is not represented.
///
/// Translated from C `weaponinfo[NUMWEAPONS]` in d_items.c.
pub static WEAPONINFO: [WeaponInfo; NUMWEAPONS] = [
    // wp_fist (Fist)
    WeaponInfo {
        ammo: AmmoType::NoAmmo,
        upstate: StateNum::S_PUNCHUP,
        downstate: StateNum::S_PUNCHDOWN,
        readystate: StateNum::S_PUNCH,
        atkstate: StateNum::S_PUNCH1,
        flashstate: StateNum::S_NULL,
    },
    // wp_pistol (Pistol)
    WeaponInfo {
        ammo: AmmoType::Clip,
        upstate: StateNum::S_PISTOLUP,
        downstate: StateNum::S_PISTOLDOWN,
        readystate: StateNum::S_PISTOL,
        atkstate: StateNum::S_PISTOL1,
        flashstate: StateNum::S_PISTOLFLASH,
    },
    // wp_shotgun (Shotgun)
    WeaponInfo {
        ammo: AmmoType::Shell,
        upstate: StateNum::S_SGUNUP,
        downstate: StateNum::S_SGUNDOWN,
        readystate: StateNum::S_SGUN,
        atkstate: StateNum::S_SGUN1,
        flashstate: StateNum::S_SGUNFLASH1,
    },
    // wp_chaingun (Chaingun)
    WeaponInfo {
        ammo: AmmoType::Clip,
        upstate: StateNum::S_CHAINUP,
        downstate: StateNum::S_CHAINDOWN,
        readystate: StateNum::S_CHAIN,
        atkstate: StateNum::S_CHAIN1,
        flashstate: StateNum::S_CHAINFLASH1,
    },
    // wp_missile (Rocket Launcher)
    WeaponInfo {
        ammo: AmmoType::Missile,
        upstate: StateNum::S_MISSILEUP,
        downstate: StateNum::S_MISSILEDOWN,
        readystate: StateNum::S_MISSILE,
        atkstate: StateNum::S_MISSILE1,
        flashstate: StateNum::S_MISSILEFLASH1,
    },
    // wp_plasma (Plasma Rifle)
    WeaponInfo {
        ammo: AmmoType::Cell,
        upstate: StateNum::S_PLASMAUP,
        downstate: StateNum::S_PLASMADOWN,
        readystate: StateNum::S_PLASMA,
        atkstate: StateNum::S_PLASMA1,
        flashstate: StateNum::S_PLASMAFLASH1,
    },
    // wp_bfg (BFG 9000)
    WeaponInfo {
        ammo: AmmoType::Cell,
        upstate: StateNum::S_BFGUP,
        downstate: StateNum::S_BFGDOWN,
        readystate: StateNum::S_BFG,
        atkstate: StateNum::S_BFG1,
        flashstate: StateNum::S_BFGFLASH1,
    },
    // wp_chainsaw (Chainsaw)
    WeaponInfo {
        ammo: AmmoType::NoAmmo,
        upstate: StateNum::S_SAWUP,
        downstate: StateNum::S_SAWDOWN,
        readystate: StateNum::S_SAW,
        atkstate: StateNum::S_SAW1,
        flashstate: StateNum::S_NULL,
    },
    // wp_supershotgun (Super Shotgun — DOOM II)
    WeaponInfo {
        ammo: AmmoType::Shell,
        upstate: StateNum::S_DSGUNUP,
        downstate: StateNum::S_DSGUNDOWN,
        readystate: StateNum::S_DSGUN,
        atkstate: StateNum::S_DSGUN1,
        flashstate: StateNum::S_DSGUNFLASH1,
    },
];
