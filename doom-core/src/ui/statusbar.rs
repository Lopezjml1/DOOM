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

//! Status bar code — face/direction indicator animation, palette indicators.
//!
//! Translated from linuxdoom-1.10/st_stuff.c and st_stuff.h.

// Many positional constants from the original C code are defined for reference
// even when they are only used indirectly through the widget creation functions.
#![allow(dead_code)]
//!
//! Implements the full DOOM status bar including health/armor/ammo numbers,
//! weapon availability indicators, key icons, face sprite animation with
//! directional awareness and damage reactions, palette shift effects (red pain,
//! gold bonus, green radiation suit), and cheat code handling.

use crate::game::strings::{
    STSTR_BEHOLD, STSTR_BEHOLDX, STSTR_CHOPPERS, STSTR_CLEV, STSTR_DQDOFF, STSTR_DQDON,
    STSTR_FAADDED, STSTR_KFAADDED, STSTR_MUS, STSTR_NCOFF, STSTR_NCON, STSTR_NOMUS,
};
#[allow(unused_imports)]
use crate::info::sounds::{MusicEnum, SfxEnum, NUMMUSIC, S_MUSIC};
#[allow(unused_imports)]
use crate::types::angle::{Angle, ANG180, ANG45};
use crate::types::doomdef::{
    AmmoType, Card, GameMode, PowerType, Skill, WeaponType, MAXPLAYERS, NUMAMMO, NUMCARDS,
    NUMWEAPONS, SCREENHEIGHT, SCREENWIDTH, SCREEN_MUL, TICRATE,
};
use crate::types::event::{Event, EventType};
#[allow(unused_imports)]
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::Patch;
use crate::types::player::{CheatFlags, Player};
use crate::types::tables::point_to_angle2;
use crate::ui::automap::{AutomapState, AM_MSGENTERED, AM_MSGEXITED, AM_MSGHEADER};
#[allow(unused_imports)]
use crate::ui::statusbar_lib::{
    stlib_init, PatchData, StBinIcon, StMultIcon, StNumber, StPercent, BG, FG,
};
use crate::util::cheat::{scramble, CheatSeq};
use crate::util::random::DoomRandom;
use crate::video::video::VideoState;
use doom_wad::{PurgeTag, WadProvider};

// ============================================================================
// Public constants (from st_stuff.h)
// ============================================================================

/// Height of the status bar area in pixels.
pub const ST_HEIGHT: i32 = 32 * SCREEN_MUL;

/// Width of the status bar area in pixels (full screen width).
pub const ST_WIDTH: i32 = SCREENWIDTH;

/// Y position of the top of the status bar on screen.
pub const ST_Y: i32 = SCREENHEIGHT - ST_HEIGHT;

// ============================================================================
// Public enums (from st_stuff.h)
// ============================================================================

/// Status bar display state — determines whether to show automap or
/// first-person status bar layout.
///
/// Original C: `st_stateenum_t` (st_stuff.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StStateEnum {
    /// Automap display is active.
    AutomapState,
    /// Normal first-person gameplay display.
    FirstPersonState,
}

/// Status bar chat sub-state for multiplayer chat input.
///
/// Original C: `st_chatstateenum_t` (st_stuff.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StChatStateEnum {
    /// Chat input start state.
    StartChatState,
    /// Waiting for destination player selection.
    WaitDestState,
    /// Receiving chat text input.
    GetChatState,
}

/// Context for face direction animation computation.
///
/// Provides mobj position/angle data that the status bar needs for
/// directional face awareness but doesn't own directly. In the original C,
/// these were accessed via `plyr->mo->x/y/angle` and `plyr->attacker->x/y`.
/// In the Rust port, the game loop passes this data explicitly.
pub struct FaceContext {
    /// Player mobj X position (fixed-point).
    pub player_x: Fixed,
    /// Player mobj Y position (fixed-point).
    pub player_y: Fixed,
    /// Player mobj facing angle (BAM).
    pub player_angle: Angle,
    /// Attacker mobj X position (fixed-point), if attacker exists.
    pub attacker_x: Option<Fixed>,
    /// Attacker mobj Y position (fixed-point), if attacker exists.
    pub attacker_y: Option<Fixed>,
}

// ============================================================================
// Internal constants — Palette indices (st_stuff.c lines 68-80)
// ============================================================================

/// Starting index of red pain palettes in the PLAYPAL lump.
const STARTREDPALS: i32 = 1;
/// Starting index of bonus (gold) palettes.
const STARTBONUSPALS: i32 = 9;
/// Number of red pain palette levels.
const NUMREDPALS: i32 = 8;
/// Number of bonus (gold pickup) palette levels.
const NUMBONUSPALS: i32 = 4;
/// Palette index for radiation suit (green tint).
const RADIATIONPAL: i32 = 13;

// ============================================================================
// Internal constants — Face animation (st_stuff.c lines 82-120)
// ============================================================================

/// Threshold for random face changes (0..255 range, lower = less likely).
const ST_FACEPROBABILITY: i32 = 96;
/// Number of pain level tiers for face animation.
const ST_NUMPAINFACES: i32 = 5;
/// Number of straight-ahead face frames per pain level.
const ST_NUMSTRAIGHTFACES: i32 = 3;
/// Number of turn (left/right) face frames per pain level.
const ST_NUMTURNFACES: i32 = 2;
/// Number of special face frames per pain level (ouch, evil grin, rampage).
const ST_NUMSPECIALFACES: i32 = 3;
/// Stride: total frames per pain level (straight + turn + special).
const ST_FACESTRIDE: i32 = ST_NUMSTRAIGHTFACES + ST_NUMTURNFACES + ST_NUMSPECIALFACES;
/// Number of extra faces beyond the pain-level matrix (god, dead).
const ST_NUMEXTRAFACES: i32 = 2;
/// Total number of face frames across all pain levels plus extras.
pub const ST_NUMFACES: i32 = ST_FACESTRIDE * ST_NUMPAINFACES + ST_NUMEXTRAFACES;

/// Offset within a pain level to the turn frames.
const ST_TURNOFFSET: i32 = ST_NUMSTRAIGHTFACES;
/// Offset within a pain level to the ouch frame.
const ST_OUCHOFFSET: i32 = ST_TURNOFFSET + ST_NUMTURNFACES;
/// Offset within a pain level to the evil grin frame.
const ST_EVILGRINOFFSET: i32 = ST_OUCHOFFSET + 1;
/// Offset within a pain level to the rampage frame.
const ST_RAMPAGEOFFSET: i32 = ST_EVILGRINOFFSET + 1;

/// Index of the god-mode face (gold eyes, invulnerable).
const ST_GODFACE: i32 = ST_NUMPAINFACES * ST_FACESTRIDE;
/// Index of the dead face (last face).
const ST_DEADFACE: i32 = ST_GODFACE + 1;

/// Tics the evil grin face stays visible after weapon pickup.
const ST_EVILGRINCOUNT: i32 = 2 * TICRATE;
/// Tics a straight-ahead face holds before random change.
const ST_STRAIGHTFACECOUNT: i32 = TICRATE / 2;
/// Tics a turn face holds.
const ST_TURNCOUNT: i32 = TICRATE;
/// Tics the ouch face holds.
const ST_OUCHCOUNT: i32 = TICRATE;
/// Tics of sustained fire before showing rampage face.
const ST_RAMPAGEDELAY: i32 = 2 * TICRATE;

/// Damage threshold for triggering the ouch face (health drop >= this).
const ST_MUCHPAIN: i32 = 20;

// ============================================================================
// Internal constants — Widget positions (st_stuff.c lines 124-265)
// ============================================================================

/// X coordinate of the status bar left edge.
const ST_X: i32 = 0;

// --- Current ammo (large) ---
const ST_AMMOWIDTH: i32 = 3;
const ST_AMMOX: i32 = 44;
const ST_AMMOY: i32 = 171;

// --- Health (large, with percent) ---
const ST_HEALTHWIDTH: i32 = 3;
const ST_HEALTHX: i32 = 90;
const ST_HEALTHY: i32 = 171;

// --- Arms area ---
const ST_ARMSX: i32 = 111;
const ST_ARMSY: i32 = 172;
const ST_ARMSBGX: i32 = 104;
const ST_ARMSBGY: i32 = 168;
const ST_ARMSXSPACE: i32 = 12;
const ST_ARMSYSPACE: i32 = 10;

// --- Frags (deathmatch) ---
const ST_FRAGSX: i32 = 138;
const ST_FRAGSY: i32 = 171;
const ST_FRAGSWIDTH: i32 = 2;

// --- Armor (large, with percent) ---
const ST_ARMORWIDTH: i32 = 3;
const ST_ARMORX: i32 = 221;
const ST_ARMORY: i32 = 171;

// --- Key icons (3 slots) ---
const ST_KEY0WIDTH: i32 = 8;
const ST_KEY0HEIGHT: i32 = 5;
const ST_KEY0X: i32 = 239;
const ST_KEY0Y: i32 = 171;
const ST_KEY1WIDTH: i32 = ST_KEY0WIDTH;
const ST_KEY1X: i32 = 239;
const ST_KEY1Y: i32 = 181;
const ST_KEY2WIDTH: i32 = ST_KEY0WIDTH;
const ST_KEY2X: i32 = 239;
const ST_KEY2Y: i32 = 191;

// --- Ammo counts (small, right side) ---
const ST_AMMO0WIDTH: i32 = 3;
const ST_AMMO0HEIGHT: i32 = 6;
const ST_AMMO0X: i32 = 288;
const ST_AMMO0Y: i32 = 173;
const ST_AMMO1WIDTH: i32 = ST_AMMO0WIDTH;
const ST_AMMO1X: i32 = 288;
const ST_AMMO1Y: i32 = 179;
const ST_AMMO2WIDTH: i32 = ST_AMMO0WIDTH;
const ST_AMMO2X: i32 = 288;
const ST_AMMO2Y: i32 = 185;
const ST_AMMO3WIDTH: i32 = ST_AMMO0WIDTH;
const ST_AMMO3X: i32 = 288;
const ST_AMMO3Y: i32 = 191;

// --- Max ammo (small, right side) ---
const ST_MAXAMMO0WIDTH: i32 = 3;
const ST_MAXAMMO0HEIGHT: i32 = 6;
const ST_MAXAMMO0X: i32 = 314;
const ST_MAXAMMO0Y: i32 = 173;
const ST_MAXAMMO1WIDTH: i32 = ST_MAXAMMO0WIDTH;
const ST_MAXAMMO1X: i32 = 314;
const ST_MAXAMMO1Y: i32 = 179;
const ST_MAXAMMO2WIDTH: i32 = ST_MAXAMMO0WIDTH;
const ST_MAXAMMO2X: i32 = 314;
const ST_MAXAMMO2Y: i32 = 185;
const ST_MAXAMMO3WIDTH: i32 = ST_MAXAMMO0WIDTH;
const ST_MAXAMMO3X: i32 = 314;
const ST_MAXAMMO3Y: i32 = 191;

// --- Weapon number positions in the Arms area ---
const ST_WEAPON0X: i32 = 111;
const ST_WEAPON0Y: i32 = 172;
const ST_WEAPON1X: i32 = 123;
const ST_WEAPON1Y: i32 = 172;
const ST_WEAPON2X: i32 = 135;
const ST_WEAPON2Y: i32 = 172;
const ST_WEAPON3X: i32 = 111;
const ST_WEAPON3Y: i32 = 182;
const ST_WEAPON4X: i32 = 123;
const ST_WEAPON4Y: i32 = 182;
const ST_WEAPON5X: i32 = 135;
const ST_WEAPON5Y: i32 = 182;

// --- Face sprite position ---
const ST_FACESX: i32 = 143;
const ST_FACESY: i32 = 168;

// ============================================================================
// Power duration constants for idbehold cheats (from p_inter.c)
// ============================================================================

/// Tics of invulnerability.
const INVULNTICS: i32 = 30 * TICRATE;
/// Tics of invisibility (partial invisibility).
const INVISTICS: i32 = 60 * TICRATE;
/// Tics of infra-red (light amplification).
const INFRATICS: i32 = 120 * TICRATE;
/// Tics of iron-feet (radiation suit).
const IRONTICS: i32 = 60 * TICRATE;

// ============================================================================
// Weapon → ammo mapping (from d_items.c)
// ============================================================================

/// Returns the ammo type for a given weapon.
///
/// Equivalent to `weaponinfo[wp].ammo` from the C `d_items.c` table.
fn weapon_ammo(weapon: WeaponType) -> AmmoType {
    match weapon {
        WeaponType::Fist => AmmoType::NoAmmo,
        WeaponType::Pistol => AmmoType::Clip,
        WeaponType::Shotgun => AmmoType::Shell,
        WeaponType::Chaingun => AmmoType::Clip,
        WeaponType::Missile => AmmoType::Missile,
        WeaponType::Plasma => AmmoType::Cell,
        WeaponType::Bfg => AmmoType::Cell,
        WeaponType::Chainsaw => AmmoType::NoAmmo,
        WeaponType::SuperShotgun => AmmoType::Shell,
        WeaponType::NoChange => AmmoType::NoAmmo,
    }
}

/// Maximum ammo capacities for each ammo type, matching the C `maxammo[]`.
const MAXAMMO_VALUES: [i32; NUMAMMO] = [200, 50, 300, 50];

// ============================================================================
// Patch header helpers — extract width/height from raw patch bytes
// ============================================================================

/// Read the 16-bit width from raw patch data (little-endian at offset 0).
///
/// The raw bytes follow the same binary layout as the [`Patch`] struct:
/// `[width: i16, height: i16, leftoffset: i16, topoffset: i16, ...]`.
/// The status bar widget system works with raw `PatchData` (Vec<u8>) rather
/// than the parsed [`Patch`] struct, so we extract the header field directly.
fn patch_width(data: &[u8]) -> i16 {
    if data.len() >= 2 {
        i16::from_le_bytes([data[0], data[1]])
    } else {
        0
    }
}

/// Read the 16-bit height from raw patch data (little-endian at offset 2).
/// See [`patch_width`] for layout details matching [`Patch`] struct fields.
fn patch_height(data: &[u8]) -> i16 {
    if data.len() >= 4 {
        i16::from_le_bytes([data[2], data[3]])
    } else {
        0
    }
}

/// Parse raw WAD patch bytes into a minimal [`Patch`] header.
///
/// Used for dimension queries where the full [`Patch`] type is convenient.
/// Fields [`Patch::width`] and [`Patch::height`] are the primary outputs.
#[allow(dead_code)]
fn parse_patch_header(data: &[u8]) -> Patch {
    Patch {
        width: patch_width(data),
        height: patch_height(data),
        leftoffset: if data.len() >= 6 {
            i16::from_le_bytes([data[4], data[5]])
        } else {
            0
        },
        topoffset: if data.len() >= 8 {
            i16::from_le_bytes([data[6], data[7]])
        } else {
            0
        },
        columnofs: Vec::new(),
    }
}

// ============================================================================
// StatusBarState — all formerly-global status bar state
// ============================================================================

/// Complete status bar state, replacing all `static` globals from st_stuff.c.
///
/// Per AAP §0.7.5, NO `static mut` is used — all state is consolidated here
/// and passed by mutable reference through the call chain.
pub struct StatusBarState {
    // --- Player tracking ---
    /// Index of the player this status bar displays (usually `consoleplayer`).
    pub plyr: usize,

    // --- Initialization flags ---
    /// True if this is the first rendering pass after a level start.
    pub st_firsttime: bool,
    /// True until the very first ST_Start call; triggers PLAYPAL lump load.
    pub veryfirsttime: bool,

    // --- Display state ---
    /// Current status bar display mode (automap vs first-person).
    st_gamestate: StStateEnum,
    /// True when the status bar is visible (always true during gameplay).
    st_statusbaron: bool,
    /// True when not in deathmatch mode (controls arms vs frags display).
    st_notdeathmatch: bool,
    /// True if the Arms section should display.
    st_armson: bool,
    /// True if the frags counter should display (deathmatch mode).
    st_fragson: bool,

    // --- Palette ---
    /// PLAYPAL lump number for palette switching.
    pub lu_palette: i32,
    /// Current active palette index (0 = normal, 1-8 = red pain, etc.).
    st_palette: i32,

    // --- Timing ---
    /// Status bar animation clock, incremented each tick.
    pub st_clock: i32,
    /// Countdown for the current player message display.
    pub st_msgcounter: i32,

    // --- Chat ---
    /// Current chat state.
    pub st_chat: StChatStateEnum,
    /// Previous chat active state for transition detection.
    st_oldchat: bool,
    /// Chat cursor blink state.
    st_cursoron: bool,

    // --- Health and damage tracking ---
    /// Previous health value for face animation delta detection.
    pub st_oldhealth: i32,

    // --- Face animation ---
    /// Countdown timer for current face expression.
    pub st_facecount: i32,
    /// Current face sprite index into the `faces` array.
    pub st_faceindex: i32,
    /// Previous face index for change detection.
    pub old_faces_index: i32,

    // --- Weapon tracking ---
    /// Whether the attack button was held down last tick (for rampage detection).
    pub last_attackdown: i32,
    /// Current face animation priority (higher priority wins).
    pub priority: i32,
    /// Previous weapon ownership state for evil-grin detection on pickup.
    pub old_weapons_owned: [bool; NUMWEAPONS],

    // --- Key display ---
    /// Key icon indices for the 3 key slots (-1 = none, 0-5 = card/skull).
    keyboxes: [i32; 3],

    // --- Random number generator for face animations ---
    rng: DoomRandom,

    // --- Status bar stopped ---
    st_stopped: bool,

    // --- Widgets ---
    /// Current ammo (large number, center-left).
    pub w_ready: StNumber,
    /// Ammo counts for each ammo type (small numbers, right side).
    pub w_ammo: [StNumber; 4],
    /// Max ammo for each ammo type (small numbers, right side).
    pub w_maxammo: [StNumber; 4],
    /// Health (large number with percent sign).
    pub w_health: StPercent,
    /// Armor (large number with percent sign).
    pub w_armor: StPercent,
    /// Weapon availability indicators (6 weapons, multi-icon: gray/yellow digit).
    pub w_arms: [StMultIcon; 6],
    /// Frags counter (deathmatch mode, replaces arms area).
    pub w_frags: StNumber,
    /// Face sprite selector.
    pub w_faces: StMultIcon,
    /// Key icons (3 slots, each showing one of 6 key patches or none).
    pub w_keyboxes: [StMultIcon; 3],

    // --- Loaded patches ---
    /// Large digit patches (0-9) from STTNUM0..STTNUM9.
    pub tallnum: [PatchData; 10],
    /// Percent sign patch (STTPRCNT).
    pub tallpercent: PatchData,
    /// Small digit patches (0-9) from STYSNUM0..STYSNUM9.
    pub shortnum: [PatchData; 10],
    /// Face animation patches (ST_NUMFACES total).
    pub faces: Vec<PatchData>,
    /// Face background patch for multiplayer color.
    pub faceback: PatchData,
    /// Arms area background patch.
    pub armsbg: PatchData,
    /// Main status bar background patch (STBAR).
    sbar: PatchData,
    /// Key icon patches (STKEYS0-5).
    keys_patches: [PatchData; NUMCARDS],
    /// Arms yellow digit patches (STGNUM2-STGNUM7) for weapon-owned state.
    arms_patches: [PatchData; 6],
    /// STTMINUS patch for negative number display.
    sttminus: PatchData,

    // --- Cheat sequences ---
    cheat_mus: CheatSeq,
    cheat_god: CheatSeq,
    cheat_ammo: CheatSeq,
    cheat_ammonokey: CheatSeq,
    cheat_noclip: CheatSeq,
    cheat_commercial_noclip: CheatSeq,
    cheat_powerup: [CheatSeq; 7],
    cheat_choppers: CheatSeq,
    cheat_clev: CheatSeq,
    cheat_mypos: CheatSeq,

    // --- Pending actions for caller to process ---
    /// Pending music change from idmus cheat: Some(music_index).
    pub pending_music_change: Option<usize>,
    /// Pending level change from idclev cheat: Some((episode, map)).
    pub pending_level_change: Option<(i32, i32)>,
}

// ============================================================================
// Cheat sequence builders
// ============================================================================

/// Build the "idmus" + 2-param cheat sequence (scrambled bytes).
fn make_cheat_mus() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'm'),
        scramble(b'u'),
        scramble(b's'),
        1,
        0,
        0,
        0xff,
    ])
}

/// Build the "iddqd" god-mode cheat sequence.
fn make_cheat_god() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'd'),
        scramble(b'q'),
        scramble(b'd'),
        0xff,
    ])
}

/// Build the "idkfa" keys + full ammo cheat sequence.
fn make_cheat_ammo() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'k'),
        scramble(b'f'),
        scramble(b'a'),
        0xff,
    ])
}

/// Build the "idfa" full ammo (no keys) cheat sequence.
fn make_cheat_ammonokey() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'f'),
        scramble(b'a'),
        0xff,
    ])
}

/// Build the "idspispopd" no-clip cheat for DOOM 1 (shareware/registered/retail).
fn make_cheat_noclip() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b's'),
        scramble(b'p'),
        scramble(b'i'),
        scramble(b's'),
        scramble(b'p'),
        scramble(b'o'),
        scramble(b'p'),
        scramble(b'd'),
        0xff,
    ])
}

/// Build the "idclip" no-clip cheat for DOOM II (commercial).
fn make_cheat_commercial_noclip() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'c'),
        scramble(b'l'),
        scramble(b'i'),
        scramble(b'p'),
        0xff,
    ])
}

/// Build the 7 "idbehold" powerup cheat sequences.
///
/// Index 0-5: `idbeholdv`, `idbeholds`, `idbeholdi`, `idbeholdr`, `idbeholda`, `idbeholdl`
/// Index 6: `idbehold` (the general help/query prompt).
fn make_cheat_powerup() -> [CheatSeq; 7] {
    let base = [
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'b'),
        scramble(b'e'),
        scramble(b'h'),
        scramble(b'o'),
        scramble(b'l'),
        scramble(b'd'),
    ];
    let suffixes: [u8; 6] = [b'v', b's', b'i', b'r', b'a', b'l'];

    let mut result: [CheatSeq; 7] = std::array::from_fn(|_| CheatSeq::new(&[0xff]));

    for i in 0..6 {
        let mut seq = base.to_vec();
        seq.push(scramble(suffixes[i]));
        seq.push(0xff);
        result[i] = CheatSeq::new(&seq);
    }

    // Index 6: bare "idbehold" without suffix — shows help message
    let mut base_seq = base.to_vec();
    base_seq.push(0xff);
    result[6] = CheatSeq::new(&base_seq);

    result
}

/// Build the "idchoppers" chainsaw cheat sequence.
fn make_cheat_choppers() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'c'),
        scramble(b'h'),
        scramble(b'o'),
        scramble(b'p'),
        scramble(b'p'),
        scramble(b'e'),
        scramble(b'r'),
        scramble(b's'),
        0xff,
    ])
}

/// Build the "idclev" + 2-param level warp cheat sequence.
fn make_cheat_clev() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'c'),
        scramble(b'l'),
        scramble(b'e'),
        scramble(b'v'),
        1,
        0,
        0,
        0xff,
    ])
}

/// Build the "idmypos" position display cheat sequence.
fn make_cheat_mypos() -> CheatSeq {
    CheatSeq::new(&[
        scramble(b'i'),
        scramble(b'd'),
        scramble(b'm'),
        scramble(b'y'),
        scramble(b'p'),
        scramble(b'o'),
        scramble(b's'),
        0xff,
    ])
}

// ============================================================================
// StatusBarState implementation
// ============================================================================

/// Create a default empty `StNumber` widget for array initialization.
fn empty_st_number() -> StNumber {
    StNumber::new(0, 0, &[], 0, false, 3)
}

/// Create a default empty `StMultIcon` widget for array initialization.
fn empty_st_multicon() -> StMultIcon {
    StMultIcon::new(0, 0, vec![], -1, false)
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBarState {
    /// Creates a new `StatusBarState` with all default values.
    ///
    /// This initializes the state to a clean starting point. The caller
    /// must subsequently call [`st_init`] (one-time setup) and [`st_start`]
    /// (per-level initialization) before using the status bar.
    pub fn new() -> Self {
        let empty_patch: PatchData = Vec::new();
        let empty_patches_10: [PatchData; 10] = std::array::from_fn(|_| Vec::new());
        let empty_keys: [PatchData; NUMCARDS] = std::array::from_fn(|_| Vec::new());

        StatusBarState {
            plyr: 0,
            st_firsttime: true,
            veryfirsttime: true,
            st_gamestate: StStateEnum::FirstPersonState,
            st_statusbaron: true,
            st_notdeathmatch: true,
            st_armson: true,
            st_fragson: false,
            lu_palette: 0,
            st_palette: 0,
            st_clock: 0,
            st_msgcounter: 0,
            st_chat: StChatStateEnum::StartChatState,
            st_oldchat: false,
            st_cursoron: false,
            st_oldhealth: -1,
            st_facecount: 0,
            st_faceindex: 0,
            old_faces_index: -1,
            last_attackdown: -1,
            priority: 0,
            old_weapons_owned: [false; NUMWEAPONS],
            keyboxes: [-1; 3],
            rng: DoomRandom::new(),
            st_stopped: true,

            w_ready: empty_st_number(),
            w_ammo: std::array::from_fn(|_| empty_st_number()),
            w_maxammo: std::array::from_fn(|_| empty_st_number()),
            w_health: StPercent::new(0, 0, &[], 0, false, Vec::new()),
            w_armor: StPercent::new(0, 0, &[], 0, false, Vec::new()),
            w_arms: std::array::from_fn(|_| empty_st_multicon()),
            w_frags: empty_st_number(),
            w_faces: empty_st_multicon(),
            w_keyboxes: std::array::from_fn(|_| empty_st_multicon()),

            tallnum: empty_patches_10.clone(),
            tallpercent: empty_patch.clone(),
            shortnum: empty_patches_10,
            faces: Vec::new(),
            faceback: empty_patch.clone(),
            armsbg: empty_patch.clone(),
            sbar: empty_patch.clone(),
            keys_patches: empty_keys,
            arms_patches: std::array::from_fn(|_| Vec::new()),
            sttminus: empty_patch,

            cheat_mus: make_cheat_mus(),
            cheat_god: make_cheat_god(),
            cheat_ammo: make_cheat_ammo(),
            cheat_ammonokey: make_cheat_ammonokey(),
            cheat_noclip: make_cheat_noclip(),
            cheat_commercial_noclip: make_cheat_commercial_noclip(),
            cheat_powerup: make_cheat_powerup(),
            cheat_choppers: make_cheat_choppers(),
            cheat_clev: make_cheat_clev(),
            cheat_mypos: make_cheat_mypos(),

            pending_music_change: None,
            pending_level_change: None,
        }
    }
}

// ============================================================================
// Face animation — pain offset calculation
// ============================================================================

/// Calculate the face pain-level offset based on player health.
///
/// Maps the player's current health (0-100+) to a pain level (0-4),
/// where 0 is the most healthy and 4 is nearly dead.
///
/// Original C: `ST_calcPainOffset` (st_stuff.c).
fn st_calc_pain_offset(health: i32) -> i32 {
    let health = if health > 100 { 100 } else { health };

    // The original C formula:
    //   ST_FACESTRIDE * (((100 - health) * ST_NUMPAINFACES) / 101)
    // This maps health 100->0 to pain_level 0->4.
    ST_FACESTRIDE * (((100 - health) * ST_NUMPAINFACES) / 101)
}

// ============================================================================
// Face animation state machine
// ============================================================================

/// Update the face animation widget based on player state.
///
/// This is the heart of the status bar face animation. It uses a priority-based
/// system to select the most appropriate face expression:
///
/// - Priority 10: Player is dead → dead face
/// - Priority 9: (unused)
/// - Priority 8: Evil grin → just picked up a weapon
/// - Priority 7: Ouch face → took significant damage
/// - Priority 6: Rampage / look direction → sustained fire or hit from a direction
/// - Priority 5: Evil grin (alternate check)
/// - Priority 4: God mode → invulnerable gold face
/// - Priority 0: Idle → random looking around, blinking
///
/// Original C: `ST_updateFaceWidget` (st_stuff.c lines ~600-800).
fn st_update_face_widget(
    state: &mut StatusBarState,
    player: &Player,
    face_ctx: Option<&FaceContext>,
) {
    let health = player.health;

    // --- Priority 10: Player dead ---
    if state.priority < 10 && player.health <= 0 {
        state.priority = 10; // dead, not going to change
        state.st_faceindex = ST_DEADFACE;
        state.st_facecount = 1;
    }

    // --- Priority 9: God-mode face (highest survivable) ---
    if state.priority < 9 && player.cheats & CheatFlags::CF_GODMODE.bits() != 0 {
        state.priority = 4;
        state.st_faceindex = ST_GODFACE;
        state.st_facecount = 1;
    }

    // --- Priority 8: Ouch face — took significant damage this tic ---
    if state.priority < 8 {
        // Check for significant health loss since last tick
        if player.damagecount != 0 && state.st_oldhealth - health > ST_MUCHPAIN {
            state.priority = 8;
            state.st_facecount = ST_OUCHCOUNT;
            state.st_faceindex = st_calc_pain_offset(health) + ST_OUCHOFFSET;
        }
    }

    // --- Priority 7: Being attacked — directional face ---
    // Original C: Uses R_PointToAngle2 and ANG180 to determine attacker
    // direction relative to player facing angle.
    if state.priority < 7 && player.damagecount != 0 {
        if let Some(attacker_idx) = player.attacker {
            if let Some(mobj_idx) = player.mobj {
                if attacker_idx != mobj_idx {
                    // Different attacker — compute direction.
                    state.priority = 7;
                    state.st_facecount = ST_TURNCOUNT;

                    // Use FaceContext for directional computation when
                    // mobj position data is available from the game loop.
                    let leftright: i32 = if let Some(ctx) = face_ctx {
                        if let (Some(ax), Some(ay)) = (ctx.attacker_x, ctx.attacker_y) {
                            // Compute angle from player to attacker
                            // using R_PointToAngle2 equivalent.
                            let badguyangle = point_to_angle2(ctx.player_x, ctx.player_y, ax, ay);
                            let player_ang = ctx.player_angle;

                            // Determine if attacker is to left or right.
                            // Original C: diff > ANG180 means leftside.
                            let diff = if badguyangle.0 > player_ang.0 {
                                Angle(badguyangle.0.wrapping_sub(player_ang.0))
                            } else {
                                Angle(player_ang.0.wrapping_sub(badguyangle.0))
                            };
                            if diff.0 > ANG180.0 {
                                1
                            } else {
                                0
                            }
                        } else {
                            // No attacker position — fall back to random
                            (state.rng.m_random() % 2) as i32
                        }
                    } else {
                        // No face context — fall back to random direction
                        (state.rng.m_random() % 2) as i32
                    };

                    let pain = st_calc_pain_offset(health);
                    state.st_faceindex = pain + ST_TURNOFFSET + leftright;
                } else {
                    // Self-damage — straight ouch face
                    state.priority = 7;
                    state.st_facecount = ST_TURNCOUNT;
                    state.st_faceindex = st_calc_pain_offset(health) + ST_OUCHOFFSET;
                }
            }
        }
    }

    // --- Priority 6: Rampage face — sustained attack button press ---
    if state.priority < 6 {
        if player.attackdown != 0 {
            if state.last_attackdown == -1 {
                state.last_attackdown = ST_RAMPAGEDELAY;
            } else {
                state.last_attackdown -= 1;
                if state.last_attackdown == 0 {
                    state.priority = 5;
                    state.st_faceindex = st_calc_pain_offset(health) + ST_RAMPAGEOFFSET;
                    state.st_facecount = 1;
                    state.last_attackdown = 1;
                }
            }
        } else {
            state.last_attackdown = -1;
        }
    }

    // --- Priority 5: Evil grin — just picked up a new weapon ---
    if state.priority < 5 {
        // Check if the player has gained any new weapons since last tic.
        let mut new_weapon_found = false;
        for i in 0..NUMWEAPONS {
            if player.weaponowned[i] && !state.old_weapons_owned[i] {
                new_weapon_found = true;
                break;
            }
        }
        if new_weapon_found {
            state.priority = 5;
            state.st_faceindex = st_calc_pain_offset(health) + ST_EVILGRINOFFSET;
            state.st_facecount = ST_EVILGRINCOUNT;
        }
    }

    // --- Priority 0: Idle — random face selection ---
    if state.priority == 0 && state.st_facecount == 0 {
        // Time to select a new random face.
        let random_val = state.rng.m_random() as i32;
        if random_val < ST_FACEPROBABILITY {
            // Mostly show straight-ahead face (with random variant)
            state.st_faceindex = st_calc_pain_offset(health) + (random_val % ST_NUMSTRAIGHTFACES);
        } else {
            // Occasionally look left or right
            let turn = random_val % ST_NUMTURNFACES;
            state.st_faceindex = st_calc_pain_offset(health) + ST_TURNOFFSET + turn;
        }
        state.st_facecount = ST_STRAIGHTFACECOUNT;
        state.priority = 0;
    }

    // Decrement face count timer.
    if state.st_facecount > 0 {
        state.st_facecount -= 1;
    }

    // When count reaches 0, drop priority so a new face can be chosen.
    if state.st_facecount == 0 {
        state.priority = 0;
    }
}

// ============================================================================
// Palette calculation
// ============================================================================

/// Calculate the active palette index based on player damage, bonuses,
/// and power-ups.
///
/// Returns the palette index (0 = normal, 1-8 = red pain, 9-12 = gold
/// bonus, 13 = green radiation suit).
///
/// Original C: `ST_doPaletteStuff` (st_stuff.c lines ~1000-1052).
pub fn st_calc_palette(_state: &StatusBarState, player: &Player) -> i32 {
    let cnt = player.damagecount;

    if cnt != 0 {
        // Red damage palette — intensity proportional to damage count.
        // The original formula: palette = (cnt + 7) >> 3
        let mut palette = (cnt + 7) >> 3;
        if palette > NUMREDPALS {
            palette = NUMREDPALS;
        }
        palette + STARTREDPALS - 1
    } else if player.bonuscount != 0 {
        // Gold bonus palette — intensity proportional to bonus count.
        let mut palette = (player.bonuscount + 7) >> 3;
        if palette > NUMBONUSPALS {
            palette = NUMBONUSPALS;
        }
        palette + STARTBONUSPALS - 1
    } else if player.powers[PowerType::IronFeet as usize] > 4 * 32
        || (player.powers[PowerType::IronFeet as usize] & 8) != 0
    {
        // Green radiation suit palette — active when power-up timer is
        // above threshold or flashing (bit 3 toggling).
        RADIATIONPAL
    } else {
        // Normal palette.
        0
    }
}

// ============================================================================
// Internal helper — refresh status bar background
// ============================================================================

/// Draw the status bar background graphics onto the BG screen buffer,
/// then copy to the foreground.
///
/// Original C: `ST_refreshBackground` (st_stuff.c lines ~499-512).
fn st_refresh_background(
    state: &StatusBarState,
    video: &mut VideoState,
    netgame: bool,
    _consoleplayer: usize,
) {
    if state.st_statusbaron {
        // Draw the main status bar background at the top of screen BG.
        video.draw_patch(ST_X, 0, BG, &state.sbar);

        // In netgame, draw face background with player color.
        if netgame {
            video.draw_patch(ST_FACESX, 0, BG, &state.faceback);
        }

        // Copy the status bar region from BG to FG at the correct Y position.
        video.copy_rect(ST_X, 0, BG, ST_WIDTH, ST_HEIGHT, ST_X, ST_Y, FG);
    }
}

// ============================================================================
// Widget value update — copies player data into widgets
// ============================================================================

/// Update all widget values from the current player state.
///
/// This replaces the C pointer-based binding where widgets held pointers
/// directly into the player struct. In Rust, we copy values into widgets
/// before each rendering pass.
///
/// Original C: `ST_updateWidgets` (st_stuff.c lines ~924-996).
fn st_update_widgets(state: &mut StatusBarState, player: &Player, face_ctx: Option<&FaceContext>) {
    // --- Current ammo for ready weapon ---
    let ammo_type = weapon_ammo(player.readyweapon);
    if ammo_type == AmmoType::NoAmmo {
        state.w_ready.num = 1994; // "1994" displayed for ammo-less weapons (id signature)
    } else {
        state.w_ready.num = player.ammo[ammo_type as usize];
    }

    // --- Health and armor ---
    state.w_health.n.num = player.health;
    state.w_armor.n.num = player.armorpoints;

    // --- Arms / weapon ownership (for non-deathmatch) ---
    for i in 0..6 {
        // Arms widgets show weapons 2-7 (index i maps to weapon i+2)
        let weapon_idx = i + 2;
        if weapon_idx < NUMWEAPONS {
            state.w_arms[i].inum = if player.weaponowned[weapon_idx] { 1 } else { 0 };
        }
    }

    // --- Frags (deathmatch) ---
    if state.st_fragson {
        let mut frag_count: i32 = 0;
        for i in 0..MAXPLAYERS {
            frag_count += player.frags[i];
        }
        // Subtract self-frags (deaths) per deathmatch convention.
        frag_count -= player.frags[state.plyr];
        state.w_frags.num = frag_count;
    }

    // --- Key icons ---
    // Card enum: BlueCard=0, YellowCard=1, RedCard=2, BlueSkull=3..=RedSkull=5.
    // Each of the 3 key slots corresponds to a color (blue=0, yellow=1, red=2).
    // Skull key display takes priority over card key when both are held.
    let card_base = Card::BlueCard as usize; // 0
    let skull_base = Card::BlueSkull as usize; // 3
    for i in 0..3 {
        let card_idx = card_base + i;
        let skull_idx = skull_base + i;
        state.keyboxes[i] = if player.cards[card_idx] {
            card_idx as i32
        } else {
            -1
        };
        if player.cards[skull_idx] {
            state.keyboxes[i] = skull_idx as i32;
        }
        state.w_keyboxes[i].inum = state.keyboxes[i];
    }

    // --- Ammo counts (all types, small numbers on right side) ---
    for i in 0..NUMAMMO {
        state.w_ammo[i].num = player.ammo[i];
        state.w_maxammo[i].num = player.maxammo[i];
    }

    // --- Face widget ---
    state.w_faces.inum = state.st_faceindex;

    // --- Update face animation state machine ---
    st_update_face_widget(state, player, face_ctx);

    // --- Track old health for next tick's delta detection ---
    state.st_oldhealth = player.health;

    // --- Track weapon ownership for evil-grin detection ---
    for i in 0..NUMWEAPONS {
        state.old_weapons_owned[i] = player.weaponowned[i];
    }
}

// ============================================================================
// Widget drawing — renders all widgets to the screen
// ============================================================================

/// Draw all status bar widgets by calling their update methods.
///
/// Original C: `ST_drawWidgets` (st_stuff.c lines ~1054-1090).
fn st_draw_widgets(state: &mut StatusBarState, video: &mut VideoState, refresh: bool) {
    let sttminus = state.sttminus.clone();

    // Ready weapon ammo (large number).
    state.w_ready.on = state.st_statusbaron;
    state.w_ready.update(refresh, &sttminus, video);

    // Health (large number with percent).
    state.w_health.n.on = state.st_statusbaron;
    state.w_health.update(refresh, &sttminus, video);

    // Armor (large number with percent).
    state.w_armor.n.on = state.st_statusbaron;
    state.w_armor.update(refresh, &sttminus, video);

    // Arms (weapons 2-7, not in deathmatch).
    for i in 0..6 {
        state.w_arms[i].on = state.st_armson;
        state.w_arms[i].update(refresh, video);
    }

    // Face sprite.
    state.w_faces.on = state.st_statusbaron;
    state.w_faces.update(refresh, video);

    // Key icons.
    for i in 0..3 {
        state.w_keyboxes[i].on = state.st_statusbaron;
        state.w_keyboxes[i].update(refresh, video);
    }

    // Frags (deathmatch only).
    state.w_frags.on = state.st_fragson;
    state.w_frags.update(refresh, &sttminus, video);

    // Ammo counts (small, all 4 types).
    for i in 0..NUMAMMO {
        state.w_ammo[i].on = state.st_statusbaron;
        state.w_ammo[i].update(refresh, &sttminus, video);
    }

    // Max ammo (small, all 4 types).
    for i in 0..NUMAMMO {
        state.w_maxammo[i].on = state.st_statusbaron;
        state.w_maxammo[i].update(refresh, &sttminus, video);
    }
}

// ============================================================================
// Graphics loading — load all status bar patches from WAD
// ============================================================================

/// Load all status bar graphic patches from the WAD file.
///
/// Loads digit patches (large and small), face sprites, key icons,
/// arms background, and the main status bar background.
///
/// Original C: `ST_loadGraphics` (st_stuff.c lines ~1124-1199).
fn st_load_graphics(state: &mut StatusBarState, wad: &mut impl WadProvider, consoleplayer: usize) {
    // --- Large digit patches (STTNUM0 through STTNUM9) ---
    for i in 0..10 {
        let name = format!("STTNUM{}", i);
        state.tallnum[i] = wad.cache_lump_name(&name, PurgeTag::Static).to_vec();
    }

    // --- Small digit patches (STYSNUM0 through STYSNUM9) ---
    for i in 0..10 {
        let name = format!("STYSNUM{}", i);
        state.shortnum[i] = wad.cache_lump_name(&name, PurgeTag::Static).to_vec();
    }

    // --- Percent sign patch ---
    state.tallpercent = wad.cache_lump_name("STTPRCNT", PurgeTag::Static).to_vec();

    // --- Key icon patches (STKEYS0 through STKEYS5) ---
    for i in 0..NUMCARDS {
        let name = format!("STKEYS{}", i);
        state.keys_patches[i] = wad.cache_lump_name(&name, PurgeTag::Static).to_vec();
    }

    // --- Arms area background patch ---
    state.armsbg = wad.cache_lump_name("STARMS", PurgeTag::Static).to_vec();

    // --- Arms yellow digit patches (STGNUM2 through STGNUM7) ---
    for i in 0..6 {
        let name = format!("STGNUM{}", i + 2);
        state.arms_patches[i] = wad.cache_lump_name(&name, PurgeTag::Static).to_vec();
    }

    // --- Face sprites ---
    // Load all face animation frames. The naming convention is:
    //   STFST<pain><frame>  — straight faces
    //   STFTR<pain><frame>  — turning faces
    //   STFOUCH<pain>       — ouch face
    //   STFEVL<pain>        — evil grin
    //   STFKILL<pain>       — rampage face
    //   STFGOD0             — god mode face
    //   STFDEAD0            — dead face
    state.faces = Vec::with_capacity(ST_NUMFACES as usize);

    for pain_level in 0..ST_NUMPAINFACES {
        // Straight-ahead faces
        for frame in 0..ST_NUMSTRAIGHTFACES {
            let name = format!("STFST{}{}", pain_level, frame);
            state
                .faces
                .push(wad.cache_lump_name(&name, PurgeTag::Static).to_vec());
        }
        // Turning faces
        for frame in 0..ST_NUMTURNFACES {
            let name = format!("STFTR{}{}", pain_level, frame);
            state
                .faces
                .push(wad.cache_lump_name(&name, PurgeTag::Static).to_vec());
        }
        // Ouch face
        {
            let name = format!("STFOUCH{}", pain_level);
            state
                .faces
                .push(wad.cache_lump_name(&name, PurgeTag::Static).to_vec());
        }
        // Evil grin
        {
            let name = format!("STFEVL{}", pain_level);
            state
                .faces
                .push(wad.cache_lump_name(&name, PurgeTag::Static).to_vec());
        }
        // Rampage (kill) face
        {
            let name = format!("STFKILL{}", pain_level);
            state
                .faces
                .push(wad.cache_lump_name(&name, PurgeTag::Static).to_vec());
        }
    }

    // God mode face
    state
        .faces
        .push(wad.cache_lump_name("STFGOD0", PurgeTag::Static).to_vec());
    // Dead face
    state
        .faces
        .push(wad.cache_lump_name("STFDEAD0", PurgeTag::Static).to_vec());

    // --- Face background (player color in multiplayer) ---
    let faceback_name = format!("STFB{}", consoleplayer);
    state.faceback = wad
        .cache_lump_name(&faceback_name, PurgeTag::Static)
        .to_vec();

    // --- Main status bar background ---
    state.sbar = wad.cache_lump_name("STBAR", PurgeTag::Static).to_vec();
}

// ============================================================================
// Widget creation — initialize all widget instances with positions and patches
// ============================================================================

/// Create and initialize all status bar widget instances.
///
/// Each widget is configured with its screen position, digit/icon patches,
/// initial value, and visibility flag. This must be called after
/// [`st_load_graphics`] so that patches are available.
///
/// Original C: `ST_createWidgets` (st_stuff.c lines ~1282-1438).
fn st_create_widgets(state: &mut StatusBarState) {
    // --- Ready weapon ammo (large number) ---
    state.w_ready = StNumber::new(
        ST_AMMOX,
        ST_AMMOY,
        &state.tallnum,
        0,    // num (updated each tick)
        true, // on
        ST_AMMOWIDTH,
    );

    // --- Health (large number with percent) ---
    state.w_health = StPercent::new(
        ST_HEALTHX,
        ST_HEALTHY,
        &state.tallnum,
        0,    // num (updated each tick)
        true, // on
        state.tallpercent.clone(),
    );

    // --- Armor (large number with percent) ---
    state.w_armor = StPercent::new(
        ST_ARMORX,
        ST_ARMORY,
        &state.tallnum,
        0,    // num (updated each tick)
        true, // on
        state.tallpercent.clone(),
    );

    // --- Arms (weapon ownership indicators, 6 widgets) ---
    let weapon_x = [
        ST_WEAPON0X,
        ST_WEAPON1X,
        ST_WEAPON2X,
        ST_WEAPON3X,
        ST_WEAPON4X,
        ST_WEAPON5X,
    ];
    let weapon_y = [
        ST_WEAPON0Y,
        ST_WEAPON1Y,
        ST_WEAPON2Y,
        ST_WEAPON3Y,
        ST_WEAPON4Y,
        ST_WEAPON5Y,
    ];

    for i in 0..6 {
        // Each arms widget has 2 patches: [0] = gray (not owned), [1] = yellow (owned)
        let patches = vec![
            state.shortnum[i + 2].clone(), // Gray digit for weapon (i+2)
            state.arms_patches[i].clone(), // Yellow digit for weapon (i+2)
        ];
        state.w_arms[i] = StMultIcon::new(
            weapon_x[i],
            weapon_y[i],
            patches,
            0,    // inum (updated each tick: 0=not owned, 1=owned)
            true, // on (if not deathmatch)
        );
    }

    // --- Frags (deathmatch) ---
    state.w_frags = StNumber::new(
        ST_FRAGSX,
        ST_FRAGSY,
        &state.tallnum,
        0,     // num (updated each tick)
        false, // off by default (enabled in deathmatch)
        ST_FRAGSWIDTH,
    );

    // --- Face sprite ---
    state.w_faces = StMultIcon::new(
        ST_FACESX,
        ST_FACESY,
        state.faces.clone(),
        0,    // inum = current face index (updated each tick)
        true, // on
    );

    // --- Key icons (3 slots) ---
    let key_x = [ST_KEY0X, ST_KEY1X, ST_KEY2X];
    let key_y = [ST_KEY0Y, ST_KEY1Y, ST_KEY2Y];
    for i in 0..3 {
        state.w_keyboxes[i] = StMultIcon::new(
            key_x[i],
            key_y[i],
            state.keys_patches.to_vec(),
            -1,   // inum = -1 (no key) initially
            true, // on
        );
    }

    // --- Ammo counts (small, 4 types) ---
    let ammo_x = [ST_AMMO0X, ST_AMMO1X, ST_AMMO2X, ST_AMMO3X];
    let ammo_y = [ST_AMMO0Y, ST_AMMO1Y, ST_AMMO2Y, ST_AMMO3Y];
    let ammo_w = [ST_AMMO0WIDTH, ST_AMMO1WIDTH, ST_AMMO2WIDTH, ST_AMMO3WIDTH];
    for i in 0..NUMAMMO {
        state.w_ammo[i] = StNumber::new(
            ammo_x[i],
            ammo_y[i],
            &state.shortnum,
            0,    // num (updated each tick)
            true, // on
            ammo_w[i],
        );
    }

    // --- Max ammo (small, 4 types) ---
    let maxammo_x = [ST_MAXAMMO0X, ST_MAXAMMO1X, ST_MAXAMMO2X, ST_MAXAMMO3X];
    let maxammo_y = [ST_MAXAMMO0Y, ST_MAXAMMO1Y, ST_MAXAMMO2Y, ST_MAXAMMO3Y];
    let maxammo_w = [
        ST_MAXAMMO0WIDTH,
        ST_MAXAMMO1WIDTH,
        ST_MAXAMMO2WIDTH,
        ST_MAXAMMO3WIDTH,
    ];
    for i in 0..NUMAMMO {
        state.w_maxammo[i] = StNumber::new(
            maxammo_x[i],
            maxammo_y[i],
            &state.shortnum,
            0,    // num (updated each tick)
            true, // on
            maxammo_w[i],
        );
    }
}

// ============================================================================
// Public API — st_responder
// ============================================================================

/// Process an input event for the status bar.
///
/// Handles automap state transitions (AM_MSGHEADER protocol) and all
/// cheat code sequences. Returns `true` if the event was consumed.
///
/// # Cheat codes processed
///
/// - `idmus##` — Change music track
/// - `iddqd` — Toggle god mode
/// - `idkfa` — Keys + full ammo + all weapons
/// - `idfa` — Full ammo + all weapons (no keys)
/// - `idspispopd` / `idclip` — Toggle no-clip mode
/// - `idbehold[vsiralm]` — Toggle power-ups
/// - `idchoppers` — Give chainsaw + invulnerability
/// - `idclev##` — Level warp
/// - `idmypos` — Display coordinates
///
/// Original C: `ST_Responder` (st_stuff.c lines ~517-725).
pub fn st_responder(
    state: &mut StatusBarState,
    ev: &Event,
    player: &mut Player,
    gamemode: GameMode,
    _gameskill: Skill,
    _gamemap: i32,
    _gameepisode: i32,
    netgame: bool,
    _deathmatch: bool,
    _automapstate: &AutomapState,
) -> bool {
    // Clear pending actions from previous tick.
    state.pending_music_change = None;
    state.pending_level_change = None;

    // --- Automap state message protocol ---
    // The automap sends messages via the event data fields to inform the
    // status bar of mode transitions.
    if ev.event_type == EventType::KeyUp {
        // Check for automap state messages encoded in the key value.
        if (ev.data1 & (0xffff0000u32 as i32)) == AM_MSGHEADER {
            let msg = ev.data1;
            if msg == AM_MSGENTERED {
                state.st_gamestate = StStateEnum::AutomapState;
                state.st_firsttime = true;
            } else if msg == AM_MSGEXITED {
                state.st_gamestate = StStateEnum::FirstPersonState;
                state.st_firsttime = true;
            }
            return false;
        }
    }

    // Only process cheat codes on key-down events.
    if ev.event_type != EventType::KeyDown {
        return false;
    }

    // Don't process cheats in netgame (anti-cheat).
    if netgame {
        return false;
    }

    let key = (ev.data1 & 0xff) as u8;

    // -----------------------------------------------------------------------
    // idmus## — Change music
    // -----------------------------------------------------------------------
    if state.cheat_mus.check_cheat(key) {
        let buf = state.cheat_mus.get_param();
        player.message = Some(STSTR_MUS.to_string());

        // Parse the 2-digit music number.
        let d0 = if !buf.is_empty() {
            (buf[0] as i32).wrapping_sub(b'0' as i32)
        } else {
            0
        };
        let d1 = if buf.len() > 1 {
            (buf[1] as i32).wrapping_sub(b'0' as i32)
        } else {
            0
        };

        let musnum = if gamemode == GameMode::Commercial {
            // DOOM II: music index = mus_runnin + (tens * 10 + ones) - 1
            MusicEnum::mus_runnin as i32 + d0 * 10 + d1 - 1
        } else {
            // DOOM I: music index = mus_e1m1 + (episode * 9 + map) - 1
            MusicEnum::mus_e1m1 as i32 + d0 * 9 + d1 - 1
        };

        if musnum > 0 && (musnum as usize) < NUMMUSIC {
            state.pending_music_change = Some(musnum as usize);
        } else {
            player.message = Some(STSTR_NOMUS.to_string());
        }
        return true;
    }

    // -----------------------------------------------------------------------
    // idchoppers — Give chainsaw + invulnerability
    // -----------------------------------------------------------------------
    if state.cheat_choppers.check_cheat(key) {
        player.weaponowned[WeaponType::Chainsaw as usize] = true;
        player.powers[PowerType::Invulnerability as usize] = 1; // Will toggle on next tick
        player.message = Some(STSTR_CHOPPERS.to_string());
        return true;
    }

    // -----------------------------------------------------------------------
    // iddqd — Toggle god mode
    // -----------------------------------------------------------------------
    if state.cheat_god.check_cheat(key) {
        // Toggle CF_GODMODE flag.
        player.cheats ^= CheatFlags::CF_GODMODE.bits();
        if player.cheats & CheatFlags::CF_GODMODE.bits() != 0 {
            // God mode ON: restore health to 100.
            player.health = 100;
            player.message = Some(STSTR_DQDON.to_string());
        } else {
            player.message = Some(STSTR_DQDOFF.to_string());
        }
        return true;
    }

    // -----------------------------------------------------------------------
    // idkfa — Full ammo + keys + all weapons
    // -----------------------------------------------------------------------
    if state.cheat_ammo.check_cheat(key) {
        // Give all weapons.
        for i in 0..NUMWEAPONS {
            player.weaponowned[i] = true;
        }
        // Max out ammo.
        for (i, &max_val) in MAXAMMO_VALUES.iter().enumerate() {
            player.ammo[i] = max_val;
        }
        // Give all keys (blue/yellow/red cards and skulls).
        // Card enum defines BlueCard=0 through RedSkull=5, total NUMCARDS=6.
        for i in 0..NUMCARDS {
            player.cards[i] = true;
        }
        player.message = Some(STSTR_KFAADDED.to_string());
        return true;
    }

    // -----------------------------------------------------------------------
    // idfa — Full ammo + all weapons (no keys)
    // -----------------------------------------------------------------------
    if state.cheat_ammonokey.check_cheat(key) {
        for i in 0..NUMWEAPONS {
            player.weaponowned[i] = true;
        }
        for (i, &max_val) in MAXAMMO_VALUES.iter().enumerate() {
            player.ammo[i] = max_val;
        }
        player.message = Some(STSTR_FAADDED.to_string());
        return true;
    }

    // -----------------------------------------------------------------------
    // idspispopd / idclip — Toggle no-clip
    // -----------------------------------------------------------------------
    // Check the appropriate noclip cheat based on game mode.
    let noclip_triggered = if gamemode == GameMode::Commercial {
        state.cheat_commercial_noclip.check_cheat(key)
    } else {
        state.cheat_noclip.check_cheat(key)
    };

    if noclip_triggered {
        player.cheats ^= CheatFlags::CF_NOCLIP.bits();
        if player.cheats & CheatFlags::CF_NOCLIP.bits() != 0 {
            player.message = Some(STSTR_NCON.to_string());
        } else {
            player.message = Some(STSTR_NCOFF.to_string());
        }
        return true;
    }

    // -----------------------------------------------------------------------
    // idbehold[vsiralm] — Toggle power-ups (6 types)
    // -----------------------------------------------------------------------
    for i in 0..6 {
        if state.cheat_powerup[i].check_cheat(key) {
            if player.powers[i] == 0 {
                // Power not active — give it with appropriate duration.
                match i {
                    0 => player.powers[i] = INVULNTICS, // Invulnerability
                    1 => {
                        // Strength (berserk)
                        player.powers[i] = 1;
                        // Berserk also heals to 100 (simplified GiveBody)
                        if player.health < 100 {
                            player.health = 100;
                        }
                    }
                    2 => player.powers[i] = INVISTICS, // Invisibility
                    3 => player.powers[i] = IRONTICS,  // Iron feet (rad suit)
                    4 => player.powers[i] = 1,         // Allmap (permanent)
                    5 => player.powers[i] = INFRATICS, // InfraRed (lite-amp)
                    _ => {}
                }
            } else if i != PowerType::Strength as usize {
                // Power is active and not berserk — set to 1 (will expire next tick).
                player.powers[i] = 1;
            } else {
                // Berserk — remove it.
                player.powers[i] = 0;
            }
            player.message = Some(STSTR_BEHOLDX.to_string());
            return true;
        }
    }

    // idbehold (bare, no suffix) — show the help message.
    if state.cheat_powerup[6].check_cheat(key) {
        player.message = Some(STSTR_BEHOLD.to_string());
        return true;
    }

    // -----------------------------------------------------------------------
    // idclev## — Level warp
    // -----------------------------------------------------------------------
    if state.cheat_clev.check_cheat(key) {
        let buf = state.cheat_clev.get_param();

        let d0 = if !buf.is_empty() {
            (buf[0] as i32).wrapping_sub(b'0' as i32)
        } else {
            0
        };
        let d1 = if buf.len() > 1 {
            (buf[1] as i32).wrapping_sub(b'0' as i32)
        } else {
            0
        };

        let (epsd, map) = if gamemode == GameMode::Commercial {
            (1, d0 * 10 + d1)
        } else {
            (d0, d1)
        };

        // Validate the level numbers (range checks per game mode).
        // Original C: checks registered (epsd <= 3, map <= 9),
        // shareware (epsd <= 1, map <= 9), commercial (map <= 34).
        let valid = match gamemode {
            GameMode::Commercial => (1..=34).contains(&map),
            GameMode::Shareware => (1..=1).contains(&epsd) && (1..=9).contains(&map),
            GameMode::Registered => (1..=3).contains(&epsd) && (1..=9).contains(&map),
            _ => (1..=4).contains(&epsd) && (1..=9).contains(&map),
        };

        if valid {
            player.message = Some(STSTR_CLEV.to_string());
            state.pending_level_change = Some((epsd, map));
        }
        return true;
    }

    // -----------------------------------------------------------------------
    // idmypos — Show player position (x, y, angle)
    // -----------------------------------------------------------------------
    if state.cheat_mypos.check_cheat(key) {
        // Display the player's current map coordinates and facing angle.
        // C: sprintf(buf, "ang=0x%x;x,y=(0x%x,0x%x)",
        //     players[consoleplayer].mo->angle,
        //     players[consoleplayer].mo->x,
        //     players[consoleplayer].mo->y);
        // Uses cached mobj position fields on the Player struct.
        let msg = format!(
            "ang=0x{:x};x,y=(0x{:x},0x{:x})",
            player.mo_angle, player.mo_x.0, player.mo_y.0,
        );
        player.message = Some(msg);
        return true;
    }

    false
}

// ============================================================================
// Public API — st_ticker
// ============================================================================

/// Per-tick status bar update.
///
/// Updates the face animation, widget values, and message counter.
/// Called once per game tic (35 times per second) by the main game loop.
///
/// Original C: `ST_Ticker` (st_stuff.c lines ~997-1000).
pub fn st_ticker(state: &mut StatusBarState, player: &Player, face_ctx: Option<&FaceContext>) {
    // Increment animation clock.
    state.st_clock += 1;

    // Decrement message display counter.
    if state.st_msgcounter > 0 {
        state.st_msgcounter -= 1;
    }

    // Update all widget values from current player state.
    st_update_widgets(state, player, face_ctx);
}

// ============================================================================
// Public API — st_drawer
// ============================================================================

/// Draw the status bar to the screen.
///
/// Handles both full refresh (entire status bar background + widgets)
/// and partial refresh (only changed widgets).
///
/// Original C: `ST_Drawer` (st_stuff.c lines ~1092-1122).
pub fn st_drawer(
    state: &mut StatusBarState,
    video: &mut VideoState,
    fullscreen: bool,
    refresh: bool,
    netgame: bool,
    consoleplayer: usize,
) {
    state.st_statusbaron = !fullscreen || state.st_gamestate == StStateEnum::AutomapState;
    state.st_notdeathmatch = true; // Always true for single-player.

    // Determine display flags.
    state.st_armson = state.st_statusbaron && state.st_notdeathmatch;
    state.st_fragson = false; // Deathmatch frags disabled in single-player.

    let do_refresh = state.st_firsttime || refresh;

    // Draw background on first time or when explicitly refreshed.
    if do_refresh {
        st_refresh_background(state, video, netgame, consoleplayer);
    }

    // Draw all widgets.
    st_draw_widgets(state, video, do_refresh);

    // Clear first-time flag after initial render.
    state.st_firsttime = false;
}

// ============================================================================
// Public API — st_start
// ============================================================================

/// Per-level status bar initialization.
///
/// Called at the start of each level to reset the status bar state,
/// load graphics, and create widget instances. The caller should
/// provide the player index to display.
///
/// Original C: `ST_Start` (st_stuff.c lines ~1441-1455).
pub fn st_start(
    state: &mut StatusBarState,
    wad: &mut impl WadProvider,
    consoleplayer: usize,
    deathmatch: bool,
) {
    // ST_Stop equivalent: if not already stopped, mark as stopped.
    if !state.st_stopped {
        state.st_stopped = true;
    }

    // Initialize status bar data.
    st_init_data(state, consoleplayer, deathmatch);

    // Load all graphic patches.
    st_load_graphics(state, wad, consoleplayer);

    // Create/initialize all widgets.
    st_create_widgets(state);

    // Mark as running.
    state.st_stopped = false;
    state.st_firsttime = true;
}

/// Reset status bar data to initial values for a new level.
///
/// Original C: `ST_initData` (st_stuff.c lines ~1255-1278).
fn st_init_data(state: &mut StatusBarState, consoleplayer: usize, deathmatch: bool) {
    state.plyr = consoleplayer;
    state.st_firsttime = true;
    state.st_clock = 0;
    state.st_msgcounter = 0;
    state.st_chat = StChatStateEnum::StartChatState;
    state.st_gamestate = StStateEnum::FirstPersonState;
    state.st_statusbaron = true;
    state.st_oldchat = false;
    state.st_cursoron = false;
    state.st_oldhealth = -1;
    state.st_facecount = 0;
    state.st_faceindex = 0;
    state.old_faces_index = -1;
    state.last_attackdown = -1;
    state.priority = 0;
    state.st_palette = 0;

    // Display flags.
    state.st_notdeathmatch = !deathmatch;
    state.st_armson = state.st_statusbaron && state.st_notdeathmatch;
    state.st_fragson = deathmatch;

    // Reset key display.
    for i in 0..3 {
        state.keyboxes[i] = -1;
    }

    // Reset weapon ownership tracking.
    for i in 0..NUMWEAPONS {
        state.old_weapons_owned[i] = false;
    }

    // Reset cheat sequences.
    state.cheat_mus = make_cheat_mus();
    state.cheat_god = make_cheat_god();
    state.cheat_ammo = make_cheat_ammo();
    state.cheat_ammonokey = make_cheat_ammonokey();
    state.cheat_noclip = make_cheat_noclip();
    state.cheat_commercial_noclip = make_cheat_commercial_noclip();
    state.cheat_powerup = make_cheat_powerup();
    state.cheat_choppers = make_cheat_choppers();
    state.cheat_clev = make_cheat_clev();
    state.cheat_mypos = make_cheat_mypos();
}

// ============================================================================
// Public API — st_init
// ============================================================================

/// One-time status bar initialization.
///
/// Loads the PLAYPAL lump number and initializes the st_lib module.
/// Called once at engine startup, before any calls to [`st_start`].
///
/// Original C: `ST_Init` (st_stuff.c lines ~1466-1471).
pub fn st_init(state: &mut StatusBarState, wad: &mut impl WadProvider) {
    // Cache the PLAYPAL lump number for palette switching.
    state.lu_palette = match wad.get_num_for_name("PLAYPAL") {
        Ok(num) => num as i32,
        Err(_) => 0,
    };

    // Initialize the st_lib module (loads the STTMINUS patch).
    state.sttminus = stlib_init(wad);

    // Clear veryfirsttime so we know init has been called.
    state.veryfirsttime = false;
}
