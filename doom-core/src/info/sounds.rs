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

//! Translated from linuxdoom-1.10/sounds.h and linuxdoom-1.10/sounds.c
//!
//! Sound effect and music information tables.
//! Created by the sound utility written by Dave Taylor.
//! Kept as a sample, DOOM2 sounds. Frozen.
//!
//! Contains:
//! - [`SfxEnum`] — Enumeration of all sound effect identifiers (109 entries)
//! - [`MusicEnum`] — Enumeration of all music track identifiers (68 entries)
//! - [`SfxInfo`] — Metadata struct for each sound effect
//! - [`MusicInfo`] — Metadata struct for each music track
//! - [`S_SFX`] — Complete sound effect info table
//! - [`S_MUSIC`] — Complete music info table

// ---------------------------------------------------------------------------
// Music track identifiers (sounds.h lines 99-170)
// ---------------------------------------------------------------------------

/// Identifiers for all music tracks in game.
///
/// Matches the C `musicenum_t` exactly. 68 entries (mus_None=0 through
/// mus_dm2int=67). The sentinel `NUMMUSIC` is provided as a separate
/// constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
#[allow(non_camel_case_types)]
pub enum MusicEnum {
    mus_None = 0,
    mus_e1m1 = 1,
    mus_e1m2 = 2,
    mus_e1m3 = 3,
    mus_e1m4 = 4,
    mus_e1m5 = 5,
    mus_e1m6 = 6,
    mus_e1m7 = 7,
    mus_e1m8 = 8,
    mus_e1m9 = 9,
    mus_e2m1 = 10,
    mus_e2m2 = 11,
    mus_e2m3 = 12,
    mus_e2m4 = 13,
    mus_e2m5 = 14,
    mus_e2m6 = 15,
    mus_e2m7 = 16,
    mus_e2m8 = 17,
    mus_e2m9 = 18,
    mus_e3m1 = 19,
    mus_e3m2 = 20,
    mus_e3m3 = 21,
    mus_e3m4 = 22,
    mus_e3m5 = 23,
    mus_e3m6 = 24,
    mus_e3m7 = 25,
    mus_e3m8 = 26,
    mus_e3m9 = 27,
    mus_inter = 28,
    mus_intro = 29,
    mus_bunny = 30,
    mus_victor = 31,
    mus_introa = 32,
    mus_runnin = 33,
    mus_stalks = 34,
    mus_countd = 35,
    mus_betwee = 36,
    mus_doom = 37,
    mus_the_da = 38,
    mus_shawn = 39,
    mus_ddtblu = 40,
    mus_in_cit = 41,
    mus_dead = 42,
    mus_stlks2 = 43,
    mus_theda2 = 44,
    mus_doom2 = 45,
    mus_ddtbl2 = 46,
    mus_runni2 = 47,
    mus_dead2 = 48,
    mus_stlks3 = 49,
    mus_romero = 50,
    mus_shawn2 = 51,
    mus_messag = 52,
    mus_count2 = 53,
    mus_ddtbl3 = 54,
    mus_ampie = 55,
    mus_theda3 = 56,
    mus_adrian = 57,
    mus_messg2 = 58,
    mus_romer2 = 59,
    mus_tense = 60,
    mus_shawn3 = 61,
    mus_openin = 62,
    mus_evil = 63,
    mus_ultima = 64,
    mus_read_m = 65,
    mus_dm2ttl = 66,
    mus_dm2int = 67,
}

/// Total number of music tracks (sentinel value of the C enum).
pub const NUMMUSIC: usize = 68;

// ---------------------------------------------------------------------------
// Sound effect identifiers (sounds.h lines 177-289)
// ---------------------------------------------------------------------------

/// Identifiers for all sound effects in game.
///
/// Matches the C `sfxenum_t` exactly. 109 entries (sfx_None=0 through
/// sfx_radio=108). The sentinel `NUMSFX` is provided as a separate
/// constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
#[allow(non_camel_case_types)]
pub enum SfxEnum {
    sfx_None = 0,
    sfx_pistol = 1,
    sfx_shotgn = 2,
    sfx_sgcock = 3,
    sfx_dshtgn = 4,
    sfx_dbopn = 5,
    sfx_dbcls = 6,
    sfx_dbload = 7,
    sfx_plasma = 8,
    sfx_bfg = 9,
    sfx_sawup = 10,
    sfx_sawidl = 11,
    sfx_sawful = 12,
    sfx_sawhit = 13,
    sfx_rlaunc = 14,
    sfx_rxplod = 15,
    sfx_firsht = 16,
    sfx_firxpl = 17,
    sfx_pstart = 18,
    sfx_pstop = 19,
    sfx_doropn = 20,
    sfx_dorcls = 21,
    sfx_stnmov = 22,
    sfx_swtchn = 23,
    sfx_swtchx = 24,
    sfx_plpain = 25,
    sfx_dmpain = 26,
    sfx_popain = 27,
    sfx_vipain = 28,
    sfx_mnpain = 29,
    sfx_pepain = 30,
    sfx_slop = 31,
    sfx_itemup = 32,
    sfx_wpnup = 33,
    sfx_oof = 34,
    sfx_telept = 35,
    sfx_posit1 = 36,
    sfx_posit2 = 37,
    sfx_posit3 = 38,
    sfx_bgsit1 = 39,
    sfx_bgsit2 = 40,
    sfx_sgtsit = 41,
    sfx_cacsit = 42,
    sfx_brssit = 43,
    sfx_cybsit = 44,
    sfx_spisit = 45,
    sfx_bspsit = 46,
    sfx_kntsit = 47,
    sfx_vilsit = 48,
    sfx_mansit = 49,
    sfx_pesit = 50,
    sfx_sklatk = 51,
    sfx_sgtatk = 52,
    sfx_skepch = 53,
    sfx_vilatk = 54,
    sfx_claw = 55,
    sfx_skeswg = 56,
    sfx_pldeth = 57,
    sfx_pdiehi = 58,
    sfx_podth1 = 59,
    sfx_podth2 = 60,
    sfx_podth3 = 61,
    sfx_bgdth1 = 62,
    sfx_bgdth2 = 63,
    sfx_sgtdth = 64,
    sfx_cacdth = 65,
    sfx_skldth = 66,
    sfx_brsdth = 67,
    sfx_cybdth = 68,
    sfx_spidth = 69,
    sfx_bspdth = 70,
    sfx_vildth = 71,
    sfx_kntdth = 72,
    sfx_pedth = 73,
    sfx_skedth = 74,
    sfx_posact = 75,
    sfx_bgact = 76,
    sfx_dmact = 77,
    sfx_bspact = 78,
    sfx_bspwlk = 79,
    sfx_vilact = 80,
    sfx_noway = 81,
    sfx_barexp = 82,
    sfx_punch = 83,
    sfx_hoof = 84,
    sfx_metal = 85,
    sfx_chgun = 86,
    sfx_tink = 87,
    sfx_bdopn = 88,
    sfx_bdcls = 89,
    sfx_itmbk = 90,
    sfx_flame = 91,
    sfx_flamst = 92,
    sfx_getpow = 93,
    sfx_bospit = 94,
    sfx_boscub = 95,
    sfx_bossit = 96,
    sfx_bospn = 97,
    sfx_bosdth = 98,
    sfx_manatk = 99,
    sfx_mandth = 100,
    sfx_sssit = 101,
    sfx_ssdth = 102,
    sfx_keenpn = 103,
    sfx_keendt = 104,
    sfx_skeact = 105,
    sfx_skesit = 106,
    sfx_skeatk = 107,
    sfx_radio = 108,
}

/// Total number of sound effects (sentinel value of the C enum).
pub const NUMSFX: usize = 109;

// ---------------------------------------------------------------------------
// SfxInfo struct (sounds.h lines 30-62)
// ---------------------------------------------------------------------------

/// Sound effect metadata.
///
/// Rust translation of the C `sfxinfo_struct`. Each entry describes a single
/// sound effect: its WAD lump name, scheduling priority, optional linkage
/// to another effect, and runtime state fields.
#[derive(Debug, Clone)]
pub struct SfxInfo {
    /// Up to 6-character name (lump name prefix, e.g. "pistol" → DS prefix
    /// added at load time).
    pub name: &'static str,
    /// Sfx singularity — when `true`, only one instance of this sound can
    /// play at a time (used for ambient/positional monster sounds).
    pub singularity: bool,
    /// Sfx priority (higher = more important). Used to decide which sound
    /// to evict when all channels are full.
    pub priority: i32,
    /// Referenced sound if a link (`Some(SfxEnum)`) — the linked sound's
    /// data is reused with modified pitch/volume. `None` if not linked.
    pub link: Option<SfxEnum>,
    /// Pitch adjustment if a link (-1 = not linked / no override).
    pub pitch: i32,
    /// Volume adjustment if a link (-1 = not linked / no override).
    pub volume: i32,
    /// Sound data handle (loaded at runtime, starts as `None`).
    pub data: Option<usize>,
    /// Usefulness counter for cache eviction decisions.
    /// Checked every second: 0 → decrement, -1 → evict, >0 → in use.
    pub usefulness: i32,
    /// Lump number of sfx (assigned at runtime).
    pub lumpnum: i32,
}

// ---------------------------------------------------------------------------
// MusicInfo struct (sounds.h lines 70-84)
// ---------------------------------------------------------------------------

/// Music track metadata.
///
/// Rust translation of the C `musicinfo_t`. Each entry describes a single
/// music track by its WAD lump name and runtime state.
#[derive(Debug, Clone)]
pub struct MusicInfo {
    /// Up to 6-character name (lump name, e.g. "e1m1" → D_ prefix added
    /// at load time).
    pub name: &'static str,
    /// Lump number of music (assigned at runtime).
    pub lumpnum: i32,
    /// Music data handle (loaded at runtime).
    pub data: Option<usize>,
    /// Music handle once registered with the audio backend.
    pub handle: i32,
}

// ---------------------------------------------------------------------------
// S_MUSIC — Complete music info table (sounds.c lines 37-107)
// ---------------------------------------------------------------------------

/// The complete set of music track metadata.
///
/// 68 entries indexed by [`MusicEnum`] values. Entry 0 (`mus_None`) is a
/// dummy. All fields other than `name` are zero-initialized; `lumpnum`,
/// `data`, and `handle` are assigned at runtime.
pub static S_MUSIC: [MusicInfo; NUMMUSIC] = [
    // mus_None — dummy entry
    MusicInfo {
        name: "",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    // Episode 1
    MusicInfo {
        name: "e1m1",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e1m2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e1m3",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e1m4",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e1m5",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e1m6",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e1m7",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e1m8",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e1m9",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    // Episode 2
    MusicInfo {
        name: "e2m1",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e2m2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e2m3",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e2m4",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e2m5",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e2m6",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e2m7",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e2m8",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e2m9",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    // Episode 3
    MusicInfo {
        name: "e3m1",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e3m2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e3m3",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e3m4",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e3m5",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e3m6",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e3m7",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e3m8",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "e3m9",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    // Intermission / special
    MusicInfo {
        name: "inter",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "intro",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "bunny",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "victor",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "introa",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    // DOOM II music
    MusicInfo {
        name: "runnin",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "stalks",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "countd",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "betwee",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "doom",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "the_da",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "shawn",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "ddtblu",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "in_cit",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "dead",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "stlks2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "theda2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "doom2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "ddtbl2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "runni2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "dead2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "stlks3",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "romero",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "shawn2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "messag",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "count2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "ddtbl3",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "ampie",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "theda3",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "adrian",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "messg2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "romer2",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "tense",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "shawn3",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "openin",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "evil",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "ultima",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "read_m",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "dm2ttl",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
    MusicInfo {
        name: "dm2int",
        lumpnum: 0,
        data: None,
        handle: 0,
    },
];

// ---------------------------------------------------------------------------
// S_SFX — Complete sound effect info table (sounds.c lines 114-227)
// ---------------------------------------------------------------------------

/// The complete set of sound effect metadata.
///
/// 109 entries indexed by [`SfxEnum`] values. Entry 0 (`sfx_None`) is a
/// required dummy. All runtime fields (`data`, `usefulness`, `lumpnum`) are
/// zero-initialized and assigned during sound system startup.
///
/// **Special case**: `sfx_chgun` (index 86) is linked to `sfx_pistol`
/// with `pitch=150, volume=0`. All other entries are unlinked with
/// `link=None, pitch=-1, volume=-1`.
pub static S_SFX: [SfxInfo; NUMSFX] = [
    // 0: sfx_None — dummy entry (required for odd engine reasons)
    SfxInfo {
        name: "none",
        singularity: false,
        priority: 0,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 1: sfx_pistol
    SfxInfo {
        name: "pistol",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 2: sfx_shotgn
    SfxInfo {
        name: "shotgn",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 3: sfx_sgcock
    SfxInfo {
        name: "sgcock",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 4: sfx_dshtgn
    SfxInfo {
        name: "dshtgn",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 5: sfx_dbopn
    SfxInfo {
        name: "dbopn",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 6: sfx_dbcls
    SfxInfo {
        name: "dbcls",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 7: sfx_dbload
    SfxInfo {
        name: "dbload",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 8: sfx_plasma
    SfxInfo {
        name: "plasma",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 9: sfx_bfg
    SfxInfo {
        name: "bfg",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 10: sfx_sawup
    SfxInfo {
        name: "sawup",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 11: sfx_sawidl
    SfxInfo {
        name: "sawidl",
        singularity: false,
        priority: 118,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 12: sfx_sawful
    SfxInfo {
        name: "sawful",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 13: sfx_sawhit
    SfxInfo {
        name: "sawhit",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 14: sfx_rlaunc
    SfxInfo {
        name: "rlaunc",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 15: sfx_rxplod
    SfxInfo {
        name: "rxplod",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 16: sfx_firsht
    SfxInfo {
        name: "firsht",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 17: sfx_firxpl
    SfxInfo {
        name: "firxpl",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 18: sfx_pstart
    SfxInfo {
        name: "pstart",
        singularity: false,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 19: sfx_pstop
    SfxInfo {
        name: "pstop",
        singularity: false,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 20: sfx_doropn
    SfxInfo {
        name: "doropn",
        singularity: false,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 21: sfx_dorcls
    SfxInfo {
        name: "dorcls",
        singularity: false,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 22: sfx_stnmov
    SfxInfo {
        name: "stnmov",
        singularity: false,
        priority: 119,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 23: sfx_swtchn
    SfxInfo {
        name: "swtchn",
        singularity: false,
        priority: 78,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 24: sfx_swtchx
    SfxInfo {
        name: "swtchx",
        singularity: false,
        priority: 78,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 25: sfx_plpain
    SfxInfo {
        name: "plpain",
        singularity: false,
        priority: 96,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 26: sfx_dmpain
    SfxInfo {
        name: "dmpain",
        singularity: false,
        priority: 96,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 27: sfx_popain
    SfxInfo {
        name: "popain",
        singularity: false,
        priority: 96,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 28: sfx_vipain
    SfxInfo {
        name: "vipain",
        singularity: false,
        priority: 96,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 29: sfx_mnpain
    SfxInfo {
        name: "mnpain",
        singularity: false,
        priority: 96,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 30: sfx_pepain
    SfxInfo {
        name: "pepain",
        singularity: false,
        priority: 96,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 31: sfx_slop
    SfxInfo {
        name: "slop",
        singularity: false,
        priority: 78,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 32: sfx_itemup
    SfxInfo {
        name: "itemup",
        singularity: true,
        priority: 78,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 33: sfx_wpnup
    SfxInfo {
        name: "wpnup",
        singularity: true,
        priority: 78,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 34: sfx_oof
    SfxInfo {
        name: "oof",
        singularity: false,
        priority: 96,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 35: sfx_telept
    SfxInfo {
        name: "telept",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 36: sfx_posit1
    SfxInfo {
        name: "posit1",
        singularity: true,
        priority: 98,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 37: sfx_posit2
    SfxInfo {
        name: "posit2",
        singularity: true,
        priority: 98,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 38: sfx_posit3
    SfxInfo {
        name: "posit3",
        singularity: true,
        priority: 98,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 39: sfx_bgsit1
    SfxInfo {
        name: "bgsit1",
        singularity: true,
        priority: 98,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 40: sfx_bgsit2
    SfxInfo {
        name: "bgsit2",
        singularity: true,
        priority: 98,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 41: sfx_sgtsit
    SfxInfo {
        name: "sgtsit",
        singularity: true,
        priority: 98,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 42: sfx_cacsit
    SfxInfo {
        name: "cacsit",
        singularity: true,
        priority: 98,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 43: sfx_brssit
    SfxInfo {
        name: "brssit",
        singularity: true,
        priority: 94,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 44: sfx_cybsit
    SfxInfo {
        name: "cybsit",
        singularity: true,
        priority: 92,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 45: sfx_spisit
    SfxInfo {
        name: "spisit",
        singularity: true,
        priority: 90,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 46: sfx_bspsit
    SfxInfo {
        name: "bspsit",
        singularity: true,
        priority: 90,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 47: sfx_kntsit
    SfxInfo {
        name: "kntsit",
        singularity: true,
        priority: 90,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 48: sfx_vilsit
    SfxInfo {
        name: "vilsit",
        singularity: true,
        priority: 90,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 49: sfx_mansit
    SfxInfo {
        name: "mansit",
        singularity: true,
        priority: 90,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 50: sfx_pesit
    SfxInfo {
        name: "pesit",
        singularity: true,
        priority: 90,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 51: sfx_sklatk
    SfxInfo {
        name: "sklatk",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 52: sfx_sgtatk
    SfxInfo {
        name: "sgtatk",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 53: sfx_skepch
    SfxInfo {
        name: "skepch",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 54: sfx_vilatk
    SfxInfo {
        name: "vilatk",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 55: sfx_claw
    SfxInfo {
        name: "claw",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 56: sfx_skeswg
    SfxInfo {
        name: "skeswg",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 57: sfx_pldeth
    SfxInfo {
        name: "pldeth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 58: sfx_pdiehi
    SfxInfo {
        name: "pdiehi",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 59: sfx_podth1
    SfxInfo {
        name: "podth1",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 60: sfx_podth2
    SfxInfo {
        name: "podth2",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 61: sfx_podth3
    SfxInfo {
        name: "podth3",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 62: sfx_bgdth1
    SfxInfo {
        name: "bgdth1",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 63: sfx_bgdth2
    SfxInfo {
        name: "bgdth2",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 64: sfx_sgtdth
    SfxInfo {
        name: "sgtdth",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 65: sfx_cacdth
    SfxInfo {
        name: "cacdth",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 66: sfx_skldth
    SfxInfo {
        name: "skldth",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 67: sfx_brsdth
    SfxInfo {
        name: "brsdth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 68: sfx_cybdth
    SfxInfo {
        name: "cybdth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 69: sfx_spidth
    SfxInfo {
        name: "spidth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 70: sfx_bspdth
    SfxInfo {
        name: "bspdth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 71: sfx_vildth
    SfxInfo {
        name: "vildth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 72: sfx_kntdth
    SfxInfo {
        name: "kntdth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 73: sfx_pedth
    SfxInfo {
        name: "pedth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 74: sfx_skedth
    SfxInfo {
        name: "skedth",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 75: sfx_posact
    SfxInfo {
        name: "posact",
        singularity: true,
        priority: 120,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 76: sfx_bgact
    SfxInfo {
        name: "bgact",
        singularity: true,
        priority: 120,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 77: sfx_dmact
    SfxInfo {
        name: "dmact",
        singularity: true,
        priority: 120,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 78: sfx_bspact
    SfxInfo {
        name: "bspact",
        singularity: true,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 79: sfx_bspwlk
    SfxInfo {
        name: "bspwlk",
        singularity: true,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 80: sfx_vilact
    SfxInfo {
        name: "vilact",
        singularity: true,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 81: sfx_noway
    SfxInfo {
        name: "noway",
        singularity: false,
        priority: 78,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 82: sfx_barexp
    SfxInfo {
        name: "barexp",
        singularity: false,
        priority: 60,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 83: sfx_punch
    SfxInfo {
        name: "punch",
        singularity: false,
        priority: 64,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 84: sfx_hoof
    SfxInfo {
        name: "hoof",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 85: sfx_metal
    SfxInfo {
        name: "metal",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 86: sfx_chgun — LINKED to sfx_pistol with pitch=150, volume=0
    SfxInfo {
        name: "chgun",
        singularity: false,
        priority: 64,
        link: Some(SfxEnum::sfx_pistol),
        pitch: 150,
        volume: 0,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 87: sfx_tink
    SfxInfo {
        name: "tink",
        singularity: false,
        priority: 60,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 88: sfx_bdopn
    SfxInfo {
        name: "bdopn",
        singularity: false,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 89: sfx_bdcls
    SfxInfo {
        name: "bdcls",
        singularity: false,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 90: sfx_itmbk
    SfxInfo {
        name: "itmbk",
        singularity: false,
        priority: 100,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 91: sfx_flame
    SfxInfo {
        name: "flame",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 92: sfx_flamst
    SfxInfo {
        name: "flamst",
        singularity: false,
        priority: 32,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 93: sfx_getpow
    SfxInfo {
        name: "getpow",
        singularity: false,
        priority: 60,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 94: sfx_bospit
    SfxInfo {
        name: "bospit",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 95: sfx_boscub
    SfxInfo {
        name: "boscub",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 96: sfx_bossit
    SfxInfo {
        name: "bossit",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 97: sfx_bospn
    SfxInfo {
        name: "bospn",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 98: sfx_bosdth
    SfxInfo {
        name: "bosdth",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 99: sfx_manatk
    SfxInfo {
        name: "manatk",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 100: sfx_mandth
    SfxInfo {
        name: "mandth",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 101: sfx_sssit
    SfxInfo {
        name: "sssit",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 102: sfx_ssdth
    SfxInfo {
        name: "ssdth",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 103: sfx_keenpn
    SfxInfo {
        name: "keenpn",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 104: sfx_keendt
    SfxInfo {
        name: "keendt",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 105: sfx_skeact
    SfxInfo {
        name: "skeact",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 106: sfx_skesit
    SfxInfo {
        name: "skesit",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 107: sfx_skeatk
    SfxInfo {
        name: "skeatk",
        singularity: false,
        priority: 70,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
    // 108: sfx_radio
    SfxInfo {
        name: "radio",
        singularity: false,
        priority: 60,
        link: None,
        pitch: -1,
        volume: -1,
        data: None,
        usefulness: 0,
        lumpnum: 0,
    },
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_music_enum_count() {
        assert_eq!(NUMMUSIC, 68);
        assert_eq!(MusicEnum::mus_dm2int as usize, 67);
    }

    #[test]
    fn test_sfx_enum_count() {
        assert_eq!(NUMSFX, 109);
        assert_eq!(SfxEnum::sfx_radio as usize, 108);
    }

    #[test]
    fn test_s_music_table_size() {
        assert_eq!(S_MUSIC.len(), NUMMUSIC);
    }

    #[test]
    fn test_s_sfx_table_size() {
        assert_eq!(S_SFX.len(), NUMSFX);
    }

    #[test]
    fn test_s_music_names() {
        assert_eq!(S_MUSIC[MusicEnum::mus_None as usize].name, "");
        assert_eq!(S_MUSIC[MusicEnum::mus_e1m1 as usize].name, "e1m1");
        assert_eq!(S_MUSIC[MusicEnum::mus_e3m9 as usize].name, "e3m9");
        assert_eq!(S_MUSIC[MusicEnum::mus_inter as usize].name, "inter");
        assert_eq!(S_MUSIC[MusicEnum::mus_runnin as usize].name, "runnin");
        assert_eq!(S_MUSIC[MusicEnum::mus_dm2ttl as usize].name, "dm2ttl");
        assert_eq!(S_MUSIC[MusicEnum::mus_dm2int as usize].name, "dm2int");
    }

    #[test]
    fn test_s_sfx_names() {
        assert_eq!(S_SFX[SfxEnum::sfx_None as usize].name, "none");
        assert_eq!(S_SFX[SfxEnum::sfx_pistol as usize].name, "pistol");
        assert_eq!(S_SFX[SfxEnum::sfx_shotgn as usize].name, "shotgn");
        assert_eq!(S_SFX[SfxEnum::sfx_plasma as usize].name, "plasma");
        assert_eq!(S_SFX[SfxEnum::sfx_bfg as usize].name, "bfg");
        assert_eq!(S_SFX[SfxEnum::sfx_chgun as usize].name, "chgun");
        assert_eq!(S_SFX[SfxEnum::sfx_radio as usize].name, "radio");
    }

    #[test]
    fn test_sfx_priorities() {
        assert_eq!(S_SFX[SfxEnum::sfx_None as usize].priority, 0);
        assert_eq!(S_SFX[SfxEnum::sfx_pistol as usize].priority, 64);
        assert_eq!(S_SFX[SfxEnum::sfx_sawidl as usize].priority, 118);
        assert_eq!(S_SFX[SfxEnum::sfx_stnmov as usize].priority, 119);
        assert_eq!(S_SFX[SfxEnum::sfx_pstart as usize].priority, 100);
        assert_eq!(S_SFX[SfxEnum::sfx_telept as usize].priority, 32);
        assert_eq!(S_SFX[SfxEnum::sfx_posact as usize].priority, 120);
        assert_eq!(S_SFX[SfxEnum::sfx_barexp as usize].priority, 60);
    }

    #[test]
    fn test_sfx_singularity_true() {
        // All entries that should have singularity = true
        let singular = [
            SfxEnum::sfx_itemup,
            SfxEnum::sfx_wpnup,
            SfxEnum::sfx_posit1,
            SfxEnum::sfx_posit2,
            SfxEnum::sfx_posit3,
            SfxEnum::sfx_bgsit1,
            SfxEnum::sfx_bgsit2,
            SfxEnum::sfx_sgtsit,
            SfxEnum::sfx_cacsit,
            SfxEnum::sfx_brssit,
            SfxEnum::sfx_cybsit,
            SfxEnum::sfx_spisit,
            SfxEnum::sfx_bspsit,
            SfxEnum::sfx_kntsit,
            SfxEnum::sfx_vilsit,
            SfxEnum::sfx_mansit,
            SfxEnum::sfx_pesit,
            SfxEnum::sfx_posact,
            SfxEnum::sfx_bgact,
            SfxEnum::sfx_dmact,
            SfxEnum::sfx_bspact,
            SfxEnum::sfx_bspwlk,
            SfxEnum::sfx_vilact,
        ];
        for sfx in &singular {
            assert!(
                S_SFX[*sfx as usize].singularity,
                "Expected singularity=true for {:?}",
                sfx
            );
        }
    }

    #[test]
    fn test_sfx_chgun_linked_to_pistol() {
        let chgun = &S_SFX[SfxEnum::sfx_chgun as usize];
        assert_eq!(chgun.name, "chgun");
        assert_eq!(chgun.link, Some(SfxEnum::sfx_pistol));
        assert_eq!(chgun.pitch, 150);
        assert_eq!(chgun.volume, 0);
    }

    #[test]
    fn test_sfx_non_linked_entries() {
        // Verify non-linked entries have link=None, pitch=-1, volume=-1
        for (i, sfx) in S_SFX.iter().enumerate().take(NUMSFX) {
            if i == SfxEnum::sfx_chgun as usize {
                continue; // sfx_chgun is the only linked entry
            }
            assert_eq!(
                sfx.link, None,
                "Entry {} ({}) should have link=None",
                i, sfx.name
            );
            assert_eq!(
                sfx.pitch, -1,
                "Entry {} ({}) should have pitch=-1",
                i, sfx.name
            );
            assert_eq!(
                sfx.volume, -1,
                "Entry {} ({}) should have volume=-1",
                i, sfx.name
            );
        }
    }

    #[test]
    fn test_music_runtime_fields_zero() {
        for m in S_MUSIC.iter().take(NUMMUSIC) {
            assert_eq!(m.lumpnum, 0);
            assert!(m.data.is_none());
            assert_eq!(m.handle, 0);
        }
    }
}
