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

//! Intermission screens — stats, world map, level transition display.
//!
//! Translated from `linuxdoom-1.10/wi_stuff.c` (1851 lines) and
//! `linuxdoom-1.10/wi_stuff.h` (55 lines).
//!
//! Implements the intermission screens shown between levels, displaying
//! kill/item/secret percentages, time, par time, and animated world map
//! for DOOM 1 episodes. Supports single-player, cooperative, and
//! deathmatch stat screens.

use crate::info::sounds::{MusicEnum, SfxEnum};
use crate::types::doomdef::{
    GameMission, GameMode, Language, MAXPLAYERS, SCREENHEIGHT, SCREENWIDTH, TICRATE,
};
use crate::types::event::{BT_ATTACK, BT_USE};
#[allow(unused_imports)]
use crate::types::player::WbPlayerStruct;
use crate::types::player::WbStartStruct;
use crate::util::random::DoomRandom;
use crate::video::video::VideoState;
use doom_wad::{PurgeTag, WadProvider};

// =========================================================================
// Constants (from wi_stuff.c lines 66-111)
// =========================================================================

/// Number of episodes with world map data.
const NUMEPISODES: usize = 4;

/// Number of maps per episode.
const NUMMAPS: usize = 9;

/// Y position for intermission title text.
const WI_TITLEY: i32 = 2;

/// Vertical spacing between level name lines.
const WI_SPACINGY: i32 = 33;

// Single-player stat screen positions
/// X position for single-player stat labels.
const SP_STATSX: i32 = 50;
/// Y position for single-player stat labels.
const SP_STATSY: i32 = 50;
/// X position for time display.
const SP_TIMEX: i32 = 16;
/// Y position for time display.
const SP_TIMEY: i32 = SCREENHEIGHT - 32;

// Net game stat screen positions
/// Y position for net game stats.
const NG_STATSY: i32 = 50;
/// Horizontal spacing between net game stat columns.
const NG_SPACINGX: i32 = 64;

// Deathmatch stat screen positions
/// X position for deathmatch matrix.
const DM_MATRIXX: i32 = 42;
/// Y position for deathmatch matrix.
const DM_MATRIXY: i32 = 68;
/// Horizontal spacing in deathmatch matrix.
const DM_SPACINGX: i32 = 40;
/// X position for deathmatch totals column.
const DM_TOTALSX: i32 = 269;
/// X position for deathmatch killers label.
const DM_KILLERSX: i32 = 10;
/// Y position for deathmatch killers label.
const DM_KILLERSY: i32 = 100;
/// X position for deathmatch victims label.
const DM_VICTIMSX: i32 = 5;
/// Y position for deathmatch victims label.
const DM_VICTIMSY: i32 = 50;

// Internal state constants (from wi_stuff.c lines 278-290)
// These constants are kept for documentation/parity with the original C source,
// even though the Rust state machine uses integer comparisons directly.
#[allow(dead_code)]
/// Single-player kills counting phase.
const SP_KILLS: i32 = 0;
#[allow(dead_code)]
/// Single-player items counting phase.
const SP_ITEMS: i32 = 2;
#[allow(dead_code)]
/// Single-player secrets counting phase.
const SP_SECRET: i32 = 4;
#[allow(dead_code)]
/// Single-player frags counting phase.
const SP_FRAGS: i32 = 6;
#[allow(dead_code)]
/// Single-player time/par counting phase.
const SP_TIME: i32 = 8;
#[allow(dead_code)]
/// Pause duration between stat phases (in tics).
const SP_PAUSE: i32 = 1;

/// Delay before showing next location (in TICRATE multiples).
const SHOWNEXTLOCDELAY: i32 = 4;

/// Framebuffer target screen index.
const FB: usize = 0;

// =========================================================================
// Public Enums (from wi_stuff.h)
// =========================================================================

/// Intermission state machine phases.
///
/// Translated from `stateenum_t` in `wi_stuff.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateEnum {
    /// Sentinel / inactive state (original: NoState = -1).
    NoState,
    /// Counting stats phase.
    #[default]
    StatCount,
    /// Showing next level location on world map.
    ShowNextLoc,
}

// =========================================================================
// Internal Enums and Structs (from wi_stuff.c lines 114-174)
// =========================================================================

/// Animation type for world map background animations.
///
/// Translated from `animenum_t` in `wi_stuff.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum AnimEnum {
    /// Always animating (continuous loop).
    Always,
    /// Random timing between frames.
    Random,
    /// Triggered by reaching a specific level.
    Level,
}

/// 2D point for world map coordinates.
///
/// Translated from `point_t` in `wi_stuff.c`.
#[derive(Debug, Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

impl Point {
    const fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }
}

/// World map background animation definition and runtime state.
///
/// Translated from `anim_t` in `wi_stuff.c` (lines 135-174).
#[derive(Debug, Clone)]
pub(crate) struct WiAnim {
    /// Type of animation behavior.
    anim_type: AnimEnum,
    /// Tics between frame advances.
    period: i32,
    /// Number of animation frames.
    nanims: i32,
    /// Screen location for this animation.
    loc: Point,
    /// RANDOM: period deviation amount. LEVEL: triggering level number.
    data1: i32,
    /// RANDOM: base period value.
    data2: i32,
    /// Animation frame patch data (up to 3 frames), stored as raw bytes.
    patches: [Vec<u8>; 3],
    /// Next tic to advance to next frame.
    nexttic: i32,
    /// Index of last drawn frame (-1 = none).
    #[allow(dead_code)]
    lastdrawn: i32,
    /// Current frame counter.
    ctr: i32,
    /// Animation state value.
    #[allow(dead_code)]
    state: i32,
}

impl Default for WiAnim {
    fn default() -> Self {
        WiAnim {
            anim_type: AnimEnum::Always,
            period: 0,
            nanims: 0,
            loc: Point::default(),
            data1: 0,
            data2: 0,
            patches: [Vec::new(), Vec::new(), Vec::new()],
            nexttic: 0,
            lastdrawn: -1,
            ctr: -1,
            state: 0,
        }
    }
}

/// Animation definition template used for static initialization.
/// Contains the initial parameters for a WiAnim before patches are loaded.
#[derive(Debug, Clone, Copy)]
struct AnimDef {
    anim_type: AnimEnum,
    period: i32,
    nanims: i32,
    loc: Point,
    data1: i32,
    data2: i32,
}

impl AnimDef {
    const fn new(anim_type: AnimEnum, period: i32, nanims: i32, x: i32, y: i32) -> Self {
        AnimDef {
            anim_type,
            period,
            nanims,
            loc: Point::new(x, y),
            data1: 0,
            data2: 0,
        }
    }

    #[allow(dead_code)]
    const fn with_data(
        anim_type: AnimEnum,
        period: i32,
        nanims: i32,
        x: i32,
        y: i32,
        data1: i32,
        data2: i32,
    ) -> Self {
        AnimDef {
            anim_type,
            period,
            nanims,
            loc: Point::new(x, y),
            data1,
            data2,
        }
    }

    /// Convert this definition template into a runtime WiAnim.
    fn into_anim(self) -> WiAnim {
        WiAnim {
            anim_type: self.anim_type,
            period: self.period,
            nanims: self.nanims,
            loc: self.loc,
            data1: self.data1,
            data2: self.data2,
            patches: [Vec::new(), Vec::new(), Vec::new()],
            nexttic: 0,
            lastdrawn: -1,
            ctr: -1,
            state: 0,
        }
    }
}

// =========================================================================
// Static Data Tables (from wi_stuff.c lines 177-275)
// =========================================================================

/// Level node coordinates on the world map for each episode.
///
/// `LNODES[episode][map]` gives the (x, y) pixel position of the level
/// node on the episode's world map background graphic. These are exact
/// pixel positions from the original source and must not be changed.
///
/// Translated from `lnodes[NUMEPISODES][NUMMAPS]` in `wi_stuff.c`.
const LNODES: [[Point; NUMMAPS]; NUMEPISODES] = [
    // Episode 0 (Knee-Deep in the Dead)
    [
        Point::new(185, 164),
        Point::new(148, 143),
        Point::new(69, 122),
        Point::new(209, 102),
        Point::new(116, 89),
        Point::new(166, 55),
        Point::new(71, 56),
        Point::new(135, 29),
        Point::new(71, 24),
    ],
    // Episode 1 (The Shores of Hell)
    [
        Point::new(254, 25),
        Point::new(97, 50),
        Point::new(188, 64),
        Point::new(128, 78),
        Point::new(214, 92),
        Point::new(133, 130),
        Point::new(208, 136),
        Point::new(148, 140),
        Point::new(235, 158),
    ],
    // Episode 2 (Inferno)
    [
        Point::new(156, 168),
        Point::new(48, 154),
        Point::new(174, 95),
        Point::new(265, 75),
        Point::new(130, 48),
        Point::new(279, 23),
        Point::new(198, 48),
        Point::new(140, 25),
        Point::new(281, 136),
    ],
    // Episode 3 (unused but defined in original)
    [
        Point::new(0, 0),
        Point::new(0, 0),
        Point::new(0, 0),
        Point::new(0, 0),
        Point::new(0, 0),
        Point::new(0, 0),
        Point::new(0, 0),
        Point::new(0, 0),
        Point::new(0, 0),
    ],
];

/// Episode 0 (Knee-Deep in the Dead) animation definitions.
///
/// 10 animations, all ANIM_ALWAYS type with period TICRATE/3 (~11 tics).
/// Translated from `epsd0animinfo[]` in `wi_stuff.c`.
const EPSD0_ANIM_INFO: [AnimDef; 10] = [
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 224, 104),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 184, 160),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 112, 136),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 72, 112),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 88, 96),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 64, 48),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 192, 40),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 136, 16),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 80, 16),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 64, 24),
];

/// Episode 1 (Shores of Hell) animation definitions.
///
/// 9 animations, all ANIM_LEVEL type. Each triggers when the player
/// reaches the corresponding level. Entry 7 has a different location
/// than the level node (MONDO HACK in original).
/// Translated from `epsd1animinfo[]` in `wi_stuff.c`.
const EPSD1_ANIM_INFO: [AnimDef; 9] = [
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 128, 136),
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 128, 136),
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 128, 136),
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 128, 136),
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 128, 136),
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 128, 136),
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 128, 136),
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 192, 144),
    AnimDef::new(AnimEnum::Level, TICRATE / 3, 1, 128, 136),
];

/// Episode 2 (Inferno) animation definitions.
///
/// 6 animations, all ANIM_ALWAYS type. The last one uses TICRATE/4 period.
/// Translated from `epsd2animinfo[]` in `wi_stuff.c`.
const EPSD2_ANIM_INFO: [AnimDef; 6] = [
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 104, 168),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 40, 136),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 160, 96),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 104, 80),
    AnimDef::new(AnimEnum::Always, TICRATE / 3, 3, 120, 32),
    AnimDef::new(AnimEnum::Always, TICRATE / 4, 3, 40, 0),
];

#[allow(dead_code)]
/// Number of animations per episode.
const NUMANIMS: [usize; NUMEPISODES] = [10, 9, 6, 0];

// =========================================================================
// IntermissionState — All mutable state (no static mut)
// =========================================================================

/// Complete intermission screen state.
///
/// Holds all mutable state for the intermission screens, replacing the
/// global variables in the original C code. All state is instance-scoped
/// to comply with the Rust safety model (no `static mut`).
///
/// Translated from the ~30 global static variables in `wi_stuff.c`
/// (lines 278-397).
pub struct IntermissionState {
    /// Current intermission phase.
    pub state: StateEnum,
    /// Non-zero when player wants to skip stat counting animation.
    pub accelerate_stage: i32,
    /// Background animation counter (increments each tic).
    pub bcnt: i32,
    /// Index of the display player (usually 0 for single-player).
    pub me: i32,
    /// Intermission parameters passed from the game layer.
    pub wbs: WbStartStruct,
    /// General-purpose counter used by show-next-loc and no-state.
    pub cnt: i32,
    /// Whether this is the first refresh (triggers full background draw).
    pub first_refresh: bool,

    // State machine counters for each mode
    /// Single-player stat counting state machine phase.
    pub sp_state: i32,
    /// Net game (coop) stat counting state machine phase.
    pub ng_state: i32,
    /// Deathmatch stat counting state machine phase.
    pub dm_state: i32,

    /// Whether the "You Are Here" pointer is currently displayed.
    pub snl_pointeron: bool,

    // Game mode context (set once at start, used for dispatch)
    /// Current game mode (Shareware, Registered, Commercial, Retail).
    pub gamemode: GameMode,
    /// Current game mission (Doom, Doom2, TNT, Plutonia, etc.).
    pub gamemission: GameMission,
    /// Current language setting (English, French, etc.).
    pub language: Language,
    /// Whether this is a deathmatch game.
    pub deathmatch: bool,
    /// Whether this is a network game.
    pub netgame: bool,
    /// Which player slots are active.
    pub playeringame: [bool; MAXPLAYERS],

    // Stat counting display values
    /// Per-player kill count display values.
    pub cnt_kills: [i32; MAXPLAYERS],
    /// Per-player item count display values.
    pub cnt_items: [i32; MAXPLAYERS],
    /// Per-player secret count display values.
    pub cnt_secret: [i32; MAXPLAYERS],
    /// Per-player frag count display values (deathmatch).
    pub cnt_frags: [i32; MAXPLAYERS],
    /// Time display counter.
    pub cnt_time: i32,
    /// Par time display counter.
    pub cnt_par: i32,
    /// Pause counter between stat phases.
    pub cnt_pause: i32,
    /// Whether frags should be displayed (true if any frags exist).
    pub dofrags: bool,

    // Deathmatch matrix
    /// Deathmatch frag matrix [killer][victim].
    pub dm_frags: [[i32; MAXPLAYERS]; MAXPLAYERS],
    /// Deathmatch frag totals per player.
    pub dm_totals: [i32; MAXPLAYERS],

    // Pending sound/music events for the game layer to play
    /// Sound effect to play this tic (None if no sound).
    pub pending_sound: Option<SfxEnum>,
    /// Music to start this tic (None if no music change).
    pub pending_music: Option<MusicEnum>,

    // Per-episode animation state
    /// Background animations for each episode.
    #[allow(private_interfaces)]
    pub anims: Vec<Vec<WiAnim>>,

    // ---- Loaded WAD patch data (raw bytes) ----
    // Background
    bg: Vec<u8>,
    // "You Are Here" arrow patches (2 variants)
    yah: [Vec<u8>; 2],
    // Splat (completed level marker)
    splat: Vec<u8>,
    // Level name patches
    lnames: Vec<Vec<u8>>,
    // Digit patches 0-9
    num: [Vec<u8>; 10],
    // Minus sign
    wiminus: Vec<u8>,
    // Percent sign
    percent: Vec<u8>,
    // Colon for time display
    colon: Vec<u8>,
    // Finished text
    finished: Vec<u8>,
    // Entering text
    entering: Vec<u8>,
    // Stat labels
    kills_label: Vec<u8>,
    secret_label: Vec<u8>,
    sp_secret_label: Vec<u8>,
    items_label: Vec<u8>,
    frags_label: Vec<u8>,
    time_label: Vec<u8>,
    sucks_label: Vec<u8>,
    par_label: Vec<u8>,
    killers_label: Vec<u8>,
    victims_label: Vec<u8>,
    total_label: Vec<u8>,
    // Player face icons (for net game display)
    star: Vec<u8>,
    bstar: Vec<u8>,
    p: [Vec<u8>; MAXPLAYERS],
    bp: [Vec<u8>; MAXPLAYERS],

    // Random number generator for animation timing
    rng: DoomRandom,
}

impl IntermissionState {
    /// Create a new default intermission state.
    pub fn new() -> Self {
        IntermissionState {
            state: StateEnum::StatCount,
            accelerate_stage: 0,
            bcnt: 0,
            me: 0,
            wbs: WbStartStruct::default(),
            cnt: 0,
            first_refresh: true,
            sp_state: 0,
            ng_state: 0,
            dm_state: 0,
            snl_pointeron: false,
            gamemode: GameMode::Indetermined,
            gamemission: GameMission::None,
            language: Language::English,
            deathmatch: false,
            netgame: false,
            playeringame: [false; MAXPLAYERS],
            cnt_kills: [0; MAXPLAYERS],
            cnt_items: [0; MAXPLAYERS],
            cnt_secret: [0; MAXPLAYERS],
            cnt_frags: [0; MAXPLAYERS],
            cnt_time: 0,
            cnt_par: 0,
            cnt_pause: 0,
            dofrags: false,
            dm_frags: [[0; MAXPLAYERS]; MAXPLAYERS],
            dm_totals: [0; MAXPLAYERS],
            pending_sound: None,
            pending_music: None,
            anims: Vec::new(),
            bg: Vec::new(),
            yah: [Vec::new(), Vec::new()],
            splat: Vec::new(),
            lnames: Vec::new(),
            num: [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            wiminus: Vec::new(),
            percent: Vec::new(),
            colon: Vec::new(),
            finished: Vec::new(),
            entering: Vec::new(),
            kills_label: Vec::new(),
            secret_label: Vec::new(),
            sp_secret_label: Vec::new(),
            items_label: Vec::new(),
            frags_label: Vec::new(),
            time_label: Vec::new(),
            sucks_label: Vec::new(),
            par_label: Vec::new(),
            killers_label: Vec::new(),
            victims_label: Vec::new(),
            total_label: Vec::new(),
            star: Vec::new(),
            bstar: Vec::new(),
            p: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            bp: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            rng: DoomRandom::new(),
        }
    }
}

impl Default for IntermissionState {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// Helper: Get patch width from raw patch data
// =========================================================================

/// Extract the width field from raw WAD patch data.
///
/// Patch header layout: width(i16), height(i16), leftoffset(i16), topoffset(i16).
fn patch_width(patch_data: &[u8]) -> i32 {
    if patch_data.len() < 2 {
        return 0;
    }
    i16::from_le_bytes([patch_data[0], patch_data[1]]) as i32
}

/// Extract the height field from raw WAD patch data.
fn patch_height(patch_data: &[u8]) -> i32 {
    if patch_data.len() < 4 {
        return 0;
    }
    i16::from_le_bytes([patch_data[2], patch_data[3]]) as i32
}

// =========================================================================
// Drawing Helpers (from wi_stuff.c lines 406-721)
// =========================================================================

/// Copy background image from screens[1] to screens[0].
///
/// Equivalent to `WI_slamBackground` in `wi_stuff.c` (line 406-415).
fn wi_slam_background(video: &mut VideoState) {
    // memcpy(screens[0], screens[1], SCREENWIDTH * SCREENHEIGHT)
    let size = (SCREENWIDTH * SCREENHEIGHT) as usize;
    if video.screens[1].len() >= size && video.screens[0].len() >= size {
        // Copy from screen 1 to screen 0
        video.copy_rect(0, 0, 1, SCREENWIDTH, SCREENHEIGHT, 0, 0, 0);
    }
}

/// Draw "Finished!" text and the level name for the completed level.
///
/// Equivalent to `WI_drawLF` in `wi_stuff.c` (lines 420-449).
fn wi_draw_lf(state: &IntermissionState, video: &mut VideoState) {
    let last = state.wbs.last as usize;
    if last >= state.lnames.len() {
        return;
    }

    // Draw level name centered
    let lname = &state.lnames[last];
    let y = WI_TITLEY;
    let x = (SCREENWIDTH - patch_width(lname)) / 2;
    video.draw_patch(x, y, FB, lname);

    // Draw "Finished!" below
    let y = y + (5 * patch_height(lname)) / 4;
    let x = (SCREENWIDTH - patch_width(&state.finished)) / 2;
    video.draw_patch(x, y, FB, &state.finished);
}

/// Draw "Entering" text and the level name for the next level.
///
/// Equivalent to `WI_drawEL` in `wi_stuff.c` (lines 455-487).
fn wi_draw_el(state: &IntermissionState, video: &mut VideoState) {
    let next = state.wbs.next as usize;
    if next >= state.lnames.len() {
        return;
    }

    // Draw "Entering" centered
    let y = WI_TITLEY;
    let x = (SCREENWIDTH - patch_width(&state.entering)) / 2;
    video.draw_patch(x, y, FB, &state.entering);

    // Draw level name below
    let lname = &state.lnames[next];
    let y = y + (5 * patch_height(&state.entering)) / 4;
    let x = (SCREENWIDTH - patch_width(lname)) / 2;
    video.draw_patch(x, y, FB, lname);
}

/// Draw patches at a specific level node position on the world map.
///
/// Equivalent to `WI_drawOnLnode` in `wi_stuff.c` (lines 492-499).
/// Tries each patch in order and draws the first one that fits on screen.
fn wi_draw_on_lnode(
    state: &IntermissionState,
    n: usize,
    patches: &[&[u8]],
    video: &mut VideoState,
) {
    let epsd = state.wbs.epsd as usize;
    if epsd >= NUMEPISODES || n >= NUMMAPS {
        return;
    }

    let loc = &LNODES[epsd][n];

    for patch in patches {
        let left = loc.x - patch_width(patch) / 2;
        let top = loc.y - patch_height(patch) / 2;

        if left >= 0
            && left + patch_width(patch) <= SCREENWIDTH
            && top >= 0
            && top + patch_height(patch) <= SCREENHEIGHT
        {
            video.draw_patch(loc.x, loc.y, FB, patch);
            break;
        }
    }
}

/// Draw a number right-justified at (x, y) with a given number of digits.
///
/// Equivalent to `WI_drawNum` in `wi_stuff.c` (lines 611-670).
/// Returns the x position after drawing. Special case: returns 0 for n == 1994.
fn wi_draw_num(
    state: &IntermissionState,
    mut x: i32,
    y: i32,
    mut n: i32,
    digits: i32,
    video: &mut VideoState,
) -> i32 {
    // Special sentinel: 1994 means "don't draw"
    if n == 1994 {
        return 0;
    }

    let neg = n < 0;
    if neg {
        n = -n;
    }

    let fontw = patch_width(&state.num[0]);
    if fontw == 0 {
        return x;
    }

    // If digits <= 0, figure out how many digits needed
    let mut actual_digits = digits;
    if actual_digits <= 0 {
        if n == 0 {
            actual_digits = 1;
        } else {
            actual_digits = 0;
            let mut temp = n;
            while temp > 0 {
                temp /= 10;
                actual_digits += 1;
            }
        }
    }

    // Draw digits right-to-left
    let mut temp_n = n;
    for _ in 0..actual_digits {
        x -= fontw;
        let digit = (temp_n % 10) as usize;
        if digit < 10 {
            video.draw_patch(x, y, FB, &state.num[digit]);
        }
        temp_n /= 10;
    }

    // Draw minus sign for negative numbers
    if neg {
        x -= 8; // Width of minus sign patch
        video.draw_patch(x, y, FB, &state.wiminus);
    }

    x
}

/// Draw a percentage value at (x, y).
///
/// Equivalent to `WI_drawPercent` in `wi_stuff.c` (lines 676-688).
fn wi_draw_percent(state: &IntermissionState, x: i32, y: i32, p: i32, video: &mut VideoState) {
    if p < 0 {
        return;
    }
    video.draw_patch(x, y, FB, &state.percent);
    wi_draw_num(state, x, y, p, -1, video);
}

/// Draw a time value (MM:SS format) at (x, y).
///
/// Equivalent to `WI_drawTime` in `wi_stuff.c` (lines 697-721).
/// Falls back to "Sucks" if time exceeds 61*59 tics.
fn wi_draw_time(state: &IntermissionState, x: i32, y: i32, t: i32, video: &mut VideoState) {
    if t < 0 {
        return;
    }

    // Convert tics to seconds
    let mut time_secs = t / TICRATE;

    if time_secs > 61 * 59 {
        // "Sucks" — time is too large to display
        video.draw_patch(x, y, FB, &state.sucks_label);
        return;
    }

    let mut draw_x = x;
    let mut done = false;

    loop {
        let div = time_secs / 60;
        let r = time_secs % 60;

        // Draw the current time component (always 2 digits for seconds, -1 for minutes)
        if div > 0 || !done {
            wi_draw_num(state, draw_x, y, r, 2, video);
        } else {
            wi_draw_num(state, draw_x, y, r, -1, video);
        }

        time_secs = div;

        if time_secs == 0 && done {
            break;
        }

        if !done {
            // Draw colon between minutes and seconds
            let colon_w = patch_width(&state.colon);
            draw_x -= colon_w;
            video.draw_patch(draw_x, y, FB, &state.colon);
            // Move left for next digits
            draw_x -= patch_width(&state.num[0]) * 2;
            done = true;
        } else {
            break;
        }
    }
}

// =========================================================================
// Animation Functions (from wi_stuff.c lines 503-601)
// =========================================================================

/// Initialize background animations for the current episode.
///
/// Equivalent to `WI_initAnimatedBack` in `wi_stuff.c` (lines 503-535).
fn wi_init_animated_back(state: &mut IntermissionState) {
    let epsd = state.wbs.epsd as usize;
    if state.gamemode == GameMode::Commercial || epsd >= NUMEPISODES {
        return;
    }

    if epsd >= state.anims.len() {
        return;
    }

    for i in 0..state.anims[epsd].len() {
        let anim = &mut state.anims[epsd][i];
        anim.ctr = -1;

        match anim.anim_type {
            AnimEnum::Always => {
                anim.nexttic = state.bcnt + 1 + (state.rng.m_random() as i32 % anim.period);
            }
            AnimEnum::Random => {
                let base = anim.data2;
                let deviation = anim.data1;
                anim.nexttic = state.bcnt + 1 + (state.rng.m_random() as i32 % deviation) + base;
            }
            AnimEnum::Level => {
                // Level-triggered animations start disabled
                anim.nexttic = 0;
            }
        }
    }
}

/// Update background animations each tic.
///
/// Equivalent to `WI_updateAnimatedBack` in `wi_stuff.c` (lines 541-581).
fn wi_update_animated_back(state: &mut IntermissionState) {
    let epsd = state.wbs.epsd as usize;
    if state.gamemode == GameMode::Commercial || epsd >= NUMEPISODES {
        return;
    }

    if epsd >= state.anims.len() {
        return;
    }

    let bcnt = state.bcnt;
    let cur_state = state.state;

    for i in 0..state.anims[epsd].len() {
        let anim = &mut state.anims[epsd][i];

        if bcnt == anim.nexttic {
            match anim.anim_type {
                AnimEnum::Always => {
                    anim.ctr += 1;
                    if anim.ctr >= anim.nanims {
                        anim.ctr = 0;
                    }
                    anim.nexttic = bcnt + anim.period;
                }
                AnimEnum::Random => {
                    anim.ctr += 1;
                    if anim.ctr >= anim.nanims {
                        anim.ctr = -1;
                        let base = anim.data2;
                        let deviation = anim.data1;
                        anim.nexttic = bcnt + (state.rng.m_random() as i32 % deviation) + base;
                    } else {
                        anim.nexttic = bcnt + anim.period;
                    }
                }
                AnimEnum::Level => {
                    // Nothing to do — level animations are updated
                    // in wi_draw_animated_back based on level number.
                }
            }
        }
    }

    // MONDO HACK: Episode 1 (Shores of Hell), animation index 7
    // When in StatCount state and anim index 7 is triggered, it uses
    // patches from anim index 4. This is a faithful reproduction of
    // the original hack at wi_stuff.c line 575.
    if epsd == 1
        && cur_state == StateEnum::StatCount
        && state.anims[epsd].len() > 7
        && state.anims[epsd].len() > 4
    {
        // Clone needed to satisfy borrow checker
        let a4_patches = state.anims[epsd][4].patches.clone();
        let a4_ctr = state.anims[epsd][4].ctr;
        if a4_ctr >= 0 && (a4_ctr as usize) < a4_patches.len() {
            // MONDO HACK: animation at index 7 shows frames from anim 4.
            // This matches the original C code behavior (wi_stuff.c line 575).
            state.anims[epsd][7].ctr = a4_ctr;
            state.anims[epsd][7].patches = a4_patches;
        }
    }
}

/// Draw background animations.
///
/// Equivalent to `WI_drawAnimatedBack` in `wi_stuff.c` (lines 587-601).
fn wi_draw_animated_back(state: &IntermissionState, video: &mut VideoState) {
    let epsd = state.wbs.epsd as usize;
    if state.gamemode == GameMode::Commercial || epsd >= NUMEPISODES {
        return;
    }

    if epsd >= state.anims.len() {
        return;
    }

    for anim in &state.anims[epsd] {
        if anim.ctr >= 0 && (anim.ctr as usize) < anim.patches.len() {
            let patch = &anim.patches[anim.ctr as usize];
            if !patch.is_empty() {
                video.draw_patch(anim.loc.x, anim.loc.y, FB, patch);
            }
        }
    }
}

// =========================================================================
// State Transition Functions (from wi_stuff.c lines 724-816)
// =========================================================================

/// Initialize the NoState phase (brief delay before exiting intermission).
///
/// Equivalent to `WI_initNoState` in `wi_stuff.c` (lines 734-740).
fn wi_init_no_state(state: &mut IntermissionState) {
    state.state = StateEnum::NoState;
    state.accelerate_stage = 0;
    state.cnt = 10;
}

/// Update the NoState phase — counts down then signals end.
///
/// Equivalent to `WI_updateNoState` in `wi_stuff.c` (lines 742-754).
fn wi_update_no_state(state: &mut IntermissionState) {
    // WI_updateAnimatedBack equivalent
    wi_update_animated_back_external(state);

    state.cnt -= 1;
    if state.cnt == 0 {
        // Signal game layer to end intermission
        // In the original, this calls G_WorldDone() — we set a flag instead
        state.cnt = -1; // Signal completion
    }
}

/// Helper for calling wi_update_animated_back with mutable state.
fn wi_update_animated_back_external(state: &mut IntermissionState) {
    let epsd = state.wbs.epsd as usize;
    if state.gamemode == GameMode::Commercial || epsd >= NUMEPISODES {
        return;
    }
    if epsd >= state.anims.len() {
        return;
    }

    let bcnt = state.bcnt;

    for i in 0..state.anims[epsd].len() {
        let anim = &mut state.anims[epsd][i];
        if bcnt == anim.nexttic {
            match anim.anim_type {
                AnimEnum::Always => {
                    anim.ctr += 1;
                    if anim.ctr >= anim.nanims {
                        anim.ctr = 0;
                    }
                    anim.nexttic = bcnt + anim.period;
                }
                AnimEnum::Random => {
                    anim.ctr += 1;
                    if anim.ctr >= anim.nanims {
                        anim.ctr = -1;
                        let base = anim.data2;
                        let deviation = anim.data1;
                        anim.nexttic = bcnt + (state.rng.m_random() as i32 % deviation) + base;
                    } else {
                        anim.nexttic = bcnt + anim.period;
                    }
                }
                AnimEnum::Level => {}
            }
        }
    }
}

/// Initialize ShowNextLoc phase — show the next level on the world map.
///
/// Equivalent to `WI_initShowNextLoc` in `wi_stuff.c` (lines 756-770).
fn wi_init_show_next_loc(state: &mut IntermissionState) {
    state.state = StateEnum::ShowNextLoc;
    state.accelerate_stage = 0;
    state.cnt = SHOWNEXTLOCDELAY * TICRATE;

    wi_init_animated_back(state);
}

/// Update ShowNextLoc phase — animates the "You Are Here" pointer.
///
/// Equivalent to `WI_updateShowNextLoc` in `wi_stuff.c` (lines 772-786).
fn wi_update_show_next_loc(state: &mut IntermissionState) {
    wi_update_animated_back(state);

    state.cnt -= 1;
    if state.cnt == 0 || state.accelerate_stage != 0 {
        wi_init_no_state(state);
    } else {
        state.snl_pointeron = (state.cnt & 31) < 20;
    }
}

/// Draw ShowNextLoc screen — world map with completed levels and pointer.
///
/// Equivalent to `WI_drawShowNextLoc` in `wi_stuff.c` (lines 788-816).
fn wi_draw_show_next_loc(state: &IntermissionState, video: &mut VideoState) {
    wi_slam_background(video);
    wi_draw_animated_back(state, video);

    if state.gamemode != GameMode::Commercial {
        let epsd = state.wbs.epsd as usize;
        if epsd < NUMEPISODES {
            let last = state.wbs.last as usize;
            let next = state.wbs.next as usize;

            // Draw completed level splats
            for i in 0..=last {
                if i < NUMMAPS {
                    wi_draw_on_lnode(state, i, &[&state.splat], video);
                }
            }

            // If we did the secret level, also mark that
            if state.wbs.didsecret {
                // Secret level is always map 8 (index 8)
                wi_draw_on_lnode(state, 8, &[&state.splat], video);
            }

            // Draw "You Are Here" pointer on the next level
            if state.snl_pointeron {
                wi_draw_on_lnode(state, next, &[&state.yah[0], &state.yah[1]], video);
            }
        }
    }

    // Draw "Entering" + next level name, if not going to a secret
    // map in commercial mode
    if state.gamemode != GameMode::Commercial || state.wbs.next != 30 {
        wi_draw_el(state, video);
    }
}

/// Draw NoState screen — same as ShowNextLoc.
///
/// Equivalent to `WI_drawNoState` in `wi_stuff.c`.
fn wi_draw_no_state(state: &IntermissionState, video: &mut VideoState) {
    wi_draw_show_next_loc(state, video);
}

// =========================================================================
// Deathmatch Stats (from wi_stuff.c lines 818-977)
// =========================================================================

/// Compute total frags for a player (kills minus suicides).
///
/// Equivalent to `WI_fragSum` in `wi_stuff.c` (lines 818-835).
fn wi_frag_sum(state: &IntermissionState, playernum: usize) -> i32 {
    let mut frags = 0;
    for i in 0..MAXPLAYERS {
        if state.playeringame[i] && i != playernum {
            frags += state.wbs.plyr[playernum].frags[i];
        }
    }
    // Subtract self-kills (suicides)
    frags -= state.wbs.plyr[playernum].frags[playernum];
    frags
}

/// Initialize deathmatch stats screen.
///
/// Equivalent to `WI_initDeathmatchStats` in `wi_stuff.c` (lines 841-869).
fn wi_init_dm_stats(state: &mut IntermissionState) {
    state.dm_state = 1;
    state.cnt_pause = TICRATE;

    // Compute frag matrix
    for i in 0..MAXPLAYERS {
        for j in 0..MAXPLAYERS {
            if state.playeringame[i] {
                state.dm_frags[i][j] = 0;
            }
        }
        state.dm_totals[i] = 0;
    }

    wi_init_animated_back(state);
}

/// Update deathmatch stats state machine.
///
/// Equivalent to `WI_updateDeathmatchStats` in `wi_stuff.c` (lines 875-926).
fn wi_update_dm_stats(state: &mut IntermissionState) {
    wi_update_animated_back(state);

    if state.accelerate_stage != 0 && state.dm_state != 4 {
        state.accelerate_stage = 0;

        // Fill in all frag counts immediately
        for i in 0..MAXPLAYERS {
            if state.playeringame[i] {
                for j in 0..MAXPLAYERS {
                    if state.playeringame[j] {
                        state.dm_frags[i][j] = state.wbs.plyr[i].frags[j];
                    }
                }
                state.dm_totals[i] = wi_frag_sum(state, i);
            }
        }

        state.pending_sound = Some(SfxEnum::sfx_barexp);
        state.dm_state = 4;
    }

    match state.dm_state {
        2 => {
            // Counting frags
            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let mut stillticking = false;

            for i in 0..MAXPLAYERS {
                if state.playeringame[i] {
                    for j in 0..MAXPLAYERS {
                        if state.playeringame[j]
                            && state.dm_frags[i][j] != state.wbs.plyr[i].frags[j]
                        {
                            if state.wbs.plyr[i].frags[j] < 0 {
                                state.dm_frags[i][j] -= 1;
                            } else {
                                state.dm_frags[i][j] += 1;
                            }

                            state.dm_frags[i][j] = state.dm_frags[i][j].clamp(-99, 99);

                            stillticking = true;
                        }
                    }
                    state.dm_totals[i] = wi_frag_sum(state, i);
                }
            }

            if !stillticking {
                state.pending_sound = Some(SfxEnum::sfx_barexp);
                state.dm_state += 1;
            }
        }
        4 => {
            // Wait for accelerate
            if state.accelerate_stage != 0 {
                state.pending_sound = Some(SfxEnum::sfx_slop);
                wi_init_show_next_loc(state);
            }
        }
        _ => {
            // States 1, 3: pause
            if state.cnt_pause > 0 {
                state.cnt_pause -= 1;
            }
            if state.cnt_pause == 0 {
                state.dm_state += 1;
                state.cnt_pause = TICRATE;
            }
        }
    }
}

/// Draw deathmatch stats screen.
///
/// Equivalent to `WI_drawDeathmatchStats` in `wi_stuff.c` (lines 932-977).
fn wi_draw_dm_stats(state: &IntermissionState, video: &mut VideoState) {
    wi_slam_background(video);
    wi_draw_animated_back(state, video);
    wi_draw_lf(state, video);

    // Draw "Total" and "Killers"/"Victims" labels
    let x = DM_TOTALSX - patch_width(&state.total_label) / 2;
    video.draw_patch(x, DM_MATRIXY - WI_SPACINGY + 10, FB, &state.total_label);
    video.draw_patch(DM_KILLERSX, DM_KILLERSY, FB, &state.killers_label);
    video.draw_patch(DM_VICTIMSX, DM_VICTIMSY, FB, &state.victims_label);

    // Draw player face headers and frag matrix
    let mut y = DM_MATRIXY;
    for i in 0..MAXPLAYERS {
        if state.playeringame[i] {
            // Draw face in row header
            let x = DM_MATRIXX - patch_width(&state.p[i]) + 1;
            video.draw_patch(x, y, FB, &state.p[i]);

            // Draw face in column header
            let x = DM_MATRIXX + DM_SPACINGX * i as i32;
            video.draw_patch(x, DM_MATRIXY - WI_SPACINGY, FB, &state.bp[i]);

            // Draw frag counts for this player against all others
            let mut kx = DM_MATRIXX + DM_SPACINGX;
            for j in 0..MAXPLAYERS {
                if state.playeringame[j] {
                    wi_draw_num(state, kx, y, state.dm_frags[i][j], 2, video);
                }
                kx += DM_SPACINGX;
            }

            // Draw total for this player
            wi_draw_num(state, DM_TOTALSX + 10, y, state.dm_totals[i], 2, video);

            y += WI_SPACINGY;
        }
    }
}

// =========================================================================
// Netgame (Coop) Stats (from wi_stuff.c lines 1074-1314)
// =========================================================================

/// Determine if frags should be shown in coop mode.
///
/// Equivalent to `dofrags` check in original `WI_initNetgameStats`.
fn wi_calc_do_frags(state: &IntermissionState) -> bool {
    for i in 0..MAXPLAYERS {
        if state.playeringame[i] && wi_frag_sum(state, i) != 0 {
            return true;
        }
    }
    false
}

/// Initialize netgame (coop) stats screen.
///
/// Equivalent to `WI_initNetgameStats` in `wi_stuff.c` (lines 1074-1100).
fn wi_init_net_game_stats(state: &mut IntermissionState) {
    state.ng_state = 1;
    state.cnt_pause = TICRATE;
    state.dofrags = wi_calc_do_frags(state);

    for i in 0..MAXPLAYERS {
        if !state.playeringame[i] {
            continue;
        }
        state.cnt_kills[i] = 0;
        state.cnt_items[i] = 0;
        state.cnt_secret[i] = 0;
        state.cnt_frags[i] = 0;
    }

    wi_init_animated_back(state);
}

/// Update netgame (coop) stats state machine.
///
/// Equivalent to `WI_updateNetgameStats` in `wi_stuff.c` (lines 1106-1243).
///
/// State machine: 1→pause→2→kills→3→pause→4→items→5→pause→6→secrets
/// →7→pause→8→frags(if dofrags)→9→pause→10→wait
fn wi_update_net_game_stats(state: &mut IntermissionState) {
    wi_update_animated_back(state);

    if state.accelerate_stage != 0 && state.ng_state != 10 {
        state.accelerate_stage = 0;

        // Fast-forward all counts
        for i in 0..MAXPLAYERS {
            if !state.playeringame[i] {
                continue;
            }
            state.cnt_kills[i] = if state.wbs.maxkills != 0 {
                (state.wbs.plyr[i].skills * 100) / state.wbs.maxkills
            } else {
                0
            };
            state.cnt_items[i] = if state.wbs.maxitems != 0 {
                (state.wbs.plyr[i].sitems * 100) / state.wbs.maxitems
            } else {
                0
            };
            state.cnt_secret[i] = if state.wbs.maxsecret != 0 {
                (state.wbs.plyr[i].ssecret * 100) / state.wbs.maxsecret
            } else {
                0
            };

            if state.dofrags {
                state.cnt_frags[i] = wi_frag_sum(state, i);
            }
        }

        state.pending_sound = Some(SfxEnum::sfx_barexp);
        state.ng_state = 10;
    }

    match state.ng_state {
        2 => {
            // Counting kills
            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let mut stillticking = false;
            for i in 0..MAXPLAYERS {
                if !state.playeringame[i] {
                    continue;
                }
                let target = if state.wbs.maxkills != 0 {
                    (state.wbs.plyr[i].skills * 100) / state.wbs.maxkills
                } else {
                    0
                };
                state.cnt_kills[i] += 2;
                if state.cnt_kills[i] >= target {
                    state.cnt_kills[i] = target;
                } else {
                    stillticking = true;
                }
            }

            if !stillticking {
                state.pending_sound = Some(SfxEnum::sfx_barexp);
                state.ng_state += 1;
            }
        }
        4 => {
            // Counting items
            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let mut stillticking = false;
            for i in 0..MAXPLAYERS {
                if !state.playeringame[i] {
                    continue;
                }
                let target = if state.wbs.maxitems != 0 {
                    (state.wbs.plyr[i].sitems * 100) / state.wbs.maxitems
                } else {
                    0
                };
                state.cnt_items[i] += 2;
                if state.cnt_items[i] >= target {
                    state.cnt_items[i] = target;
                } else {
                    stillticking = true;
                }
            }

            if !stillticking {
                state.pending_sound = Some(SfxEnum::sfx_barexp);
                state.ng_state += 1;
            }
        }
        6 => {
            // Counting secrets
            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let mut stillticking = false;
            for i in 0..MAXPLAYERS {
                if !state.playeringame[i] {
                    continue;
                }
                let target = if state.wbs.maxsecret != 0 {
                    (state.wbs.plyr[i].ssecret * 100) / state.wbs.maxsecret
                } else {
                    0
                };
                state.cnt_secret[i] += 2;
                if state.cnt_secret[i] >= target {
                    state.cnt_secret[i] = target;
                } else {
                    stillticking = true;
                }
            }

            if !stillticking {
                state.pending_sound = Some(SfxEnum::sfx_barexp);
                state.ng_state += 1 + if state.dofrags { 0 } else { 2 };
            }
        }
        8 => {
            // Counting frags (only if dofrags)
            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let mut stillticking = false;
            for i in 0..MAXPLAYERS {
                if !state.playeringame[i] {
                    continue;
                }
                let target = wi_frag_sum(state, i);
                state.cnt_frags[i] += 1;
                if state.cnt_frags[i] >= target {
                    state.cnt_frags[i] = target;
                } else {
                    stillticking = true;
                }
            }

            if !stillticking {
                state.pending_sound = Some(SfxEnum::sfx_pldeth);
                state.ng_state += 1;
            }
        }
        10 => {
            // Wait for accelerate
            if state.accelerate_stage != 0 {
                state.pending_sound = Some(SfxEnum::sfx_sgcock);
                wi_init_show_next_loc(state);
            }
        }
        _ => {
            // Odd states 1, 3, 5, 7, 9: pause
            if state.cnt_pause > 0 {
                state.cnt_pause -= 1;
            }
            if state.cnt_pause == 0 {
                state.ng_state += 1;
                state.cnt_pause = TICRATE;
            }
        }
    }
}

/// Draw netgame (coop) stats screen.
///
/// Equivalent to `WI_drawNetgameStats` in `wi_stuff.c` (lines 1249-1314).
fn wi_draw_net_game_stats(state: &IntermissionState, video: &mut VideoState) {
    wi_slam_background(video);
    wi_draw_animated_back(state, video);
    wi_draw_lf(state, video);

    // NG_STATSX depends on star width — compute at draw time
    let ng_statsx = 32 + patch_width(&state.star) / 2;
    if ng_statsx <= 0 {
        return;
    }

    // Draw stat column headers
    let stat_header_y = NG_STATSY;
    // These headers use the same patches as single-player labels
    video.draw_patch(
        ng_statsx + NG_SPACINGX - patch_width(&state.kills_label),
        stat_header_y,
        FB,
        &state.kills_label,
    );
    video.draw_patch(
        ng_statsx + 2 * NG_SPACINGX - patch_width(&state.items_label),
        stat_header_y,
        FB,
        &state.items_label,
    );
    video.draw_patch(
        ng_statsx + 3 * NG_SPACINGX - patch_width(&state.secret_label),
        stat_header_y,
        FB,
        &state.secret_label,
    );

    if state.dofrags {
        video.draw_patch(
            ng_statsx + 4 * NG_SPACINGX - patch_width(&state.frags_label),
            stat_header_y,
            FB,
            &state.frags_label,
        );
    }

    // Draw per-player stats
    let mut y = stat_header_y + patch_height(&state.kills_label);
    for i in 0..MAXPLAYERS {
        if !state.playeringame[i] {
            continue;
        }

        let x = ng_statsx;

        // Draw player face
        if i as i32 == state.me {
            video.draw_patch(x - patch_width(&state.star) - 1, y, FB, &state.star);
        } else {
            video.draw_patch(x - patch_width(&state.p[i]) - 1, y, FB, &state.p[i]);
        }

        // Draw kills percentage
        wi_draw_percent(state, x + NG_SPACINGX, y, state.cnt_kills[i], video);
        // Draw items percentage
        wi_draw_percent(state, x + 2 * NG_SPACINGX, y, state.cnt_items[i], video);
        // Draw secrets percentage
        wi_draw_percent(state, x + 3 * NG_SPACINGX, y, state.cnt_secret[i], video);

        if state.dofrags {
            wi_draw_num(state, x + 4 * NG_SPACINGX, y, state.cnt_frags[i], -1, video);
        }

        y += WI_SPACINGY;
    }
}

// =========================================================================
// Single-Player Stats (from wi_stuff.c lines 1316-1468)
// =========================================================================

/// Initialize single-player stats screen.
///
/// Equivalent to `WI_initStats` in `wi_stuff.c` (lines 1316-1335).
fn wi_init_stats(state: &mut IntermissionState) {
    state.sp_state = 1;
    state.cnt_pause = TICRATE;

    state.cnt_kills[0] = -1;
    state.cnt_items[0] = -1;
    state.cnt_secret[0] = -1;
    state.cnt_time = -1;
    state.cnt_par = -1;

    wi_init_animated_back(state);
}

/// Update single-player stats state machine.
///
/// Equivalent to `WI_updateStats` in `wi_stuff.c` (lines 1341-1423).
///
/// State machine: 1→pause→2→kills→3→pause→4→items→5→pause→6→secrets
/// →7→pause→8→time+par→9→pause→10→wait
fn wi_update_stats(state: &mut IntermissionState) {
    wi_update_animated_back(state);

    if state.accelerate_stage != 0 && state.sp_state != 10 {
        state.accelerate_stage = 0;

        // Fast-forward all counts
        let me = state.me as usize;
        state.cnt_kills[0] = if state.wbs.maxkills != 0 {
            (state.wbs.plyr[me].skills * 100) / state.wbs.maxkills
        } else {
            0
        };
        state.cnt_items[0] = if state.wbs.maxitems != 0 {
            (state.wbs.plyr[me].sitems * 100) / state.wbs.maxitems
        } else {
            0
        };
        state.cnt_secret[0] = if state.wbs.maxsecret != 0 {
            (state.wbs.plyr[me].ssecret * 100) / state.wbs.maxsecret
        } else {
            0
        };
        state.cnt_time = state.wbs.plyr[me].stime / TICRATE;
        state.cnt_par = state.wbs.partime / TICRATE;

        state.pending_sound = Some(SfxEnum::sfx_barexp);
        state.sp_state = 10;
    }

    match state.sp_state {
        2 => {
            // Counting kills
            state.cnt_kills[0] += 2;

            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let me = state.me as usize;
            let target = if state.wbs.maxkills != 0 {
                (state.wbs.plyr[me].skills * 100) / state.wbs.maxkills
            } else {
                0
            };

            if state.cnt_kills[0] >= target {
                state.cnt_kills[0] = target;
                state.pending_sound = Some(SfxEnum::sfx_barexp);
                state.sp_state += 1;
            }
        }
        4 => {
            // Counting items
            state.cnt_items[0] += 2;

            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let me = state.me as usize;
            let target = if state.wbs.maxitems != 0 {
                (state.wbs.plyr[me].sitems * 100) / state.wbs.maxitems
            } else {
                0
            };

            if state.cnt_items[0] >= target {
                state.cnt_items[0] = target;
                state.pending_sound = Some(SfxEnum::sfx_barexp);
                state.sp_state += 1;
            }
        }
        6 => {
            // Counting secrets
            state.cnt_secret[0] += 2;

            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let me = state.me as usize;
            let target = if state.wbs.maxsecret != 0 {
                (state.wbs.plyr[me].ssecret * 100) / state.wbs.maxsecret
            } else {
                0
            };

            if state.cnt_secret[0] >= target {
                state.cnt_secret[0] = target;
                state.pending_sound = Some(SfxEnum::sfx_barexp);
                state.sp_state += 1;
            }
        }
        8 => {
            // Counting time and par
            if state.bcnt & 3 == 0 {
                state.pending_sound = Some(SfxEnum::sfx_pistol);
            }

            let me = state.me as usize;
            state.cnt_time += 3;
            let time_target = state.wbs.plyr[me].stime / TICRATE;
            if state.cnt_time >= time_target {
                state.cnt_time = time_target;
            }

            state.cnt_par += 3;
            let par_target = state.wbs.partime / TICRATE;
            if state.cnt_par >= par_target {
                state.cnt_par = par_target;
            }

            if state.cnt_time >= time_target && state.cnt_par >= par_target {
                state.pending_sound = Some(SfxEnum::sfx_barexp);
                state.sp_state += 1;
            }
        }
        10 => {
            // Wait for accelerate
            if state.accelerate_stage != 0 {
                state.pending_sound = Some(SfxEnum::sfx_sgcock);
                if state.gamemode == GameMode::Commercial {
                    wi_init_no_state(state);
                } else {
                    wi_init_show_next_loc(state);
                }
            }
        }
        _ => {
            // Odd states 1, 3, 5, 7, 9: pause
            if state.cnt_pause > 0 {
                state.cnt_pause -= 1;
            }
            if state.cnt_pause == 0 {
                state.sp_state += 1;
                state.cnt_pause = TICRATE;
            }
        }
    }
}

/// Draw single-player stats screen.
///
/// Equivalent to `WI_drawStats` in `wi_stuff.c` (lines 1429-1468).
fn wi_draw_stats(state: &IntermissionState, video: &mut VideoState) {
    wi_slam_background(video);
    wi_draw_animated_back(state, video);
    wi_draw_lf(state, video);

    let lh = (3 * patch_height(&state.num[0])) / 2;

    // Kills
    video.draw_patch(SP_STATSX, SP_STATSY, FB, &state.kills_label);
    wi_draw_percent(
        state,
        SCREENWIDTH - SP_STATSX,
        SP_STATSY,
        state.cnt_kills[0],
        video,
    );

    // Items
    video.draw_patch(SP_STATSX, SP_STATSY + lh, FB, &state.items_label);
    wi_draw_percent(
        state,
        SCREENWIDTH - SP_STATSX,
        SP_STATSY + lh,
        state.cnt_items[0],
        video,
    );

    // Secrets
    video.draw_patch(SP_STATSX, SP_STATSY + 2 * lh, FB, &state.sp_secret_label);
    wi_draw_percent(
        state,
        SCREENWIDTH - SP_STATSX,
        SP_STATSY + 2 * lh,
        state.cnt_secret[0],
        video,
    );

    // Time
    video.draw_patch(SP_TIMEX, SP_TIMEY, FB, &state.time_label);
    wi_draw_time(
        state,
        SCREENWIDTH / 2 - SP_TIMEX,
        SP_TIMEY,
        state.cnt_time,
        video,
    );

    // Par time — only for non-commercial modes with known par times
    if state.wbs.epsd < 3 {
        video.draw_patch(SCREENWIDTH / 2 + SP_TIMEX, SP_TIMEY, FB, &state.par_label);
        wi_draw_time(
            state,
            SCREENWIDTH - SP_TIMEX,
            SP_TIMEY,
            state.cnt_par,
            video,
        );
    }
}

// =========================================================================
// Accelerate Check (from wi_stuff.c lines 1470-1498)
// =========================================================================

/// Check if any player has pressed fire or use to accelerate the stats.
///
/// Equivalent to `WI_checkForAccelerate` in `wi_stuff.c` (lines 1470-1498).
/// Uses edge detection on attackdown/usedown to prevent holding buttons
/// from continuously re-triggering.
fn wi_check_for_accelerate(
    state: &mut IntermissionState,
    players: &[crate::types::player::Player],
) {
    for i in 0..MAXPLAYERS {
        if state.playeringame[i] {
            if i >= players.len() {
                continue;
            }
            let player = &players[i];

            if player.cmd.buttons & BT_ATTACK != 0 && player.attackdown == 0 {
                state.accelerate_stage = 2;
            }

            if player.cmd.buttons & BT_USE != 0 && player.usedown == 0 {
                state.accelerate_stage = 2;
            }
        }
    }
}

// =========================================================================
// Data Loading (from wi_stuff.c lines 1538-1706)
// =========================================================================

/// Load all intermission graphics from WAD lumps.
///
/// Equivalent to `WI_loadData` in `wi_stuff.c` (lines 1538-1706).
/// Loads background, level names, animation frames, stat labels, digits,
/// player face icons, and other graphics needed for the intermission display.
fn wi_load_data(state: &mut IntermissionState, wad: &mut dyn WadProvider) {
    let epsd = state.wbs.epsd as usize;

    // Load appropriate background
    if state.gamemode == GameMode::Commercial {
        state.bg = wad.cache_lump_name("INTERPIC", PurgeTag::Cache).to_vec();
    } else {
        let name = format!("WIMAP{}", epsd);
        state.bg = wad.cache_lump_name(&name, PurgeTag::Cache).to_vec();
    }

    // Draw background to screen 1 for later blitting
    // (In the original, this was done via V_DrawPatch to screen 1)

    // Load animation frames for DOOM 1 episodes
    if state.gamemode != GameMode::Commercial && epsd < NUMEPISODES && epsd < state.anims.len() {
        for i in 0..state.anims[epsd].len() {
            let anim = &mut state.anims[epsd][i];
            for j in 0..anim.nanims as usize {
                if j < 3 {
                    let name = format!("WIA{}{:02}{:02}", epsd, i, j);
                    anim.patches[j] = wad.cache_lump_name(&name, PurgeTag::Cache).to_vec();
                }
            }
        }
    }

    // MONDO HACK: Episode 1 (Shores of Hell), animation 7 shares
    // patches with animation 4. In the original this was:
    //   a = &anims[1]; a[7].p[2] = a[4].p[2];
    // (wi_stuff.c line 1593)
    if epsd == 1 && state.anims.len() > 1 {
        let anims_1 = &state.anims[1];
        if anims_1.len() > 7 && anims_1.len() > 4 {
            let shared_patch = anims_1[4].patches[2].clone();
            state.anims[1][7].patches[2] = shared_patch;
        }
    }

    // Load level name patches
    state.lnames.clear();
    if state.gamemode == GameMode::Commercial {
        // DOOM 2: load "CWILV%02d" patches
        for i in 0..33 {
            let name = format!("CWILV{:02}", i);
            if let Some(_idx) = wad.check_num_for_name(&name) {
                state
                    .lnames
                    .push(wad.cache_lump_name(&name, PurgeTag::Cache).to_vec());
            } else {
                state.lnames.push(Vec::new());
            }
        }
    } else {
        // DOOM 1: load "WILV%d%d" patches
        for i in 0..NUMMAPS {
            let name = format!("WILV{}{}", epsd, i);
            if let Some(_idx) = wad.check_num_for_name(&name) {
                state
                    .lnames
                    .push(wad.cache_lump_name(&name, PurgeTag::Cache).to_vec());
            } else {
                state.lnames.push(Vec::new());
            }
        }
    }

    // Load "you are here" and splat patches
    state.yah[0] = wad.cache_lump_name("WIURH0", PurgeTag::Cache).to_vec();
    state.yah[1] = wad.cache_lump_name("WIURH1", PurgeTag::Cache).to_vec();
    state.splat = wad.cache_lump_name("WISPLAT", PurgeTag::Cache).to_vec();

    // Load "Finished" and "Entering" text
    state.finished = wad.cache_lump_name("WIF", PurgeTag::Cache).to_vec();
    state.entering = wad.cache_lump_name("WIENTER", PurgeTag::Cache).to_vec();

    // Load stat label patches
    state.kills_label = wad.cache_lump_name("WIOSTK", PurgeTag::Cache).to_vec();
    state.secret_label = wad.cache_lump_name("WIOSTS", PurgeTag::Cache).to_vec();
    state.sp_secret_label = wad.cache_lump_name("WISCRT2", PurgeTag::Cache).to_vec();

    // French variant for items label (original wi_stuff.c line 1655: "if (french)")
    // In French mode for network coop games, use "WIOBJ" (Objets) instead of "WIOSTI" (Items).
    state.items_label = if state.language == Language::French && state.netgame && !state.deathmatch
    {
        wad.cache_lump_name("WIOBJ", PurgeTag::Cache).to_vec()
    } else {
        wad.cache_lump_name("WIOSTI", PurgeTag::Cache).to_vec()
    };

    state.frags_label = wad.cache_lump_name("WIFRGS", PurgeTag::Cache).to_vec();
    state.time_label = wad.cache_lump_name("WITIME", PurgeTag::Cache).to_vec();
    state.sucks_label = wad.cache_lump_name("WISUCKS", PurgeTag::Cache).to_vec();
    state.par_label = wad.cache_lump_name("WIPAR", PurgeTag::Cache).to_vec();
    state.killers_label = wad.cache_lump_name("WIKILRS", PurgeTag::Cache).to_vec();
    state.victims_label = wad.cache_lump_name("WIVCTMS", PurgeTag::Cache).to_vec();
    state.total_label = wad.cache_lump_name("WIMSTT", PurgeTag::Cache).to_vec();

    // Load digit patches 0-9
    for i in 0..10 {
        let name = format!("WINUM{}", i);
        state.num[i] = wad.cache_lump_name(&name, PurgeTag::Cache).to_vec();
    }

    // Load minus, percent, colon
    state.wiminus = wad.cache_lump_name("WIMINUS", PurgeTag::Cache).to_vec();
    state.percent = wad.cache_lump_name("WIPCNT", PurgeTag::Cache).to_vec();
    state.colon = wad.cache_lump_name("WICOLON", PurgeTag::Cache).to_vec();

    // Load star (current player marker) and blood star
    state.star = wad.cache_lump_name("STFST01", PurgeTag::Cache).to_vec();
    state.bstar = wad.cache_lump_name("STFDEAD0", PurgeTag::Cache).to_vec();

    // Load player face icons for net game
    for i in 0..MAXPLAYERS {
        let name = format!("STPB{}", i);
        if let Some(_idx) = wad.check_num_for_name(&name) {
            state.p[i] = wad.cache_lump_name(&name, PurgeTag::Cache).to_vec();
        }

        let name = format!("WIBP{}", i + 1);
        if let Some(_idx) = wad.check_num_for_name(&name) {
            state.bp[i] = wad.cache_lump_name(&name, PurgeTag::Cache).to_vec();
        }
    }
}

// =========================================================================
// Variable Initialization (from wi_stuff.c lines 1772-1808)
// =========================================================================

/// Initialize intermission variables from WbStartStruct.
///
/// Equivalent to `WI_initVariables` in `wi_stuff.c` (lines 1772-1808).
/// Performs range checking and clamping on the input data.
fn wi_init_variables(state: &mut IntermissionState, wbs: &WbStartStruct) {
    state.wbs = wbs.clone();
    state.accelerate_stage = 0;
    state.bcnt = 0;
    state.first_refresh = true;
    state.me = wbs.pnum;

    // Clamp episode to valid range
    if state.wbs.epsd as usize >= NUMEPISODES {
        state.wbs.epsd = (NUMEPISODES - 1) as i32;
    }

    // Ensure last/next are in valid range
    if state.wbs.last < 0 {
        state.wbs.last = 0;
    }
    if state.wbs.next < 0 {
        state.wbs.next = 0;
    }

    // Clamp maxkills/maxitems/maxsecret to at least 1 to prevent div-by-zero
    if state.wbs.maxkills <= 0 {
        state.wbs.maxkills = 1;
    }
    if state.wbs.maxitems <= 0 {
        state.wbs.maxitems = 1;
    }
    if state.wbs.maxsecret <= 0 {
        state.wbs.maxsecret = 1;
    }
}

// =========================================================================
// Animation Data Initialization
// =========================================================================

/// Build the animation data arrays from the static definition templates.
fn wi_init_anim_data(state: &mut IntermissionState) {
    state.anims.clear();

    // Episode 0
    let mut ep0: Vec<WiAnim> = Vec::with_capacity(EPSD0_ANIM_INFO.len());
    for def in &EPSD0_ANIM_INFO {
        ep0.push(def.into_anim());
    }
    state.anims.push(ep0);

    // Episode 1 — set data1 for level-triggered animations
    let mut ep1: Vec<WiAnim> = Vec::with_capacity(EPSD1_ANIM_INFO.len());
    for (idx, def) in EPSD1_ANIM_INFO.iter().enumerate() {
        let mut anim = def.into_anim();
        // data1 is the level number that triggers this animation
        anim.data1 = idx as i32;
        ep1.push(anim);
    }
    state.anims.push(ep1);

    // Episode 2
    let mut ep2: Vec<WiAnim> = Vec::with_capacity(EPSD2_ANIM_INFO.len());
    for def in &EPSD2_ANIM_INFO {
        ep2.push(def.into_anim());
    }
    state.anims.push(ep2);

    // Episode 3 (empty)
    state.anims.push(Vec::new());
}

// =========================================================================
// Public API (from wi_stuff.h)
// =========================================================================

/// Initialize the intermission screen.
///
/// Equivalent to `WI_Start` in `wi_stuff.c` (lines 1839-1851).
/// Sets up all state, loads graphics, and dispatches to the appropriate
/// stat screen initializer based on game mode.
///
/// # Arguments
///
/// * `state` — The intermission state to initialize.
/// * `wbs` — Level transition data from the game layer.
/// * `wad` — WAD provider for loading graphic patches.
pub fn wi_start(state: &mut IntermissionState, wbs: &WbStartStruct, wad: &mut dyn WadProvider) {
    wi_init_variables(state, wbs);
    wi_init_anim_data(state);
    wi_load_data(state, wad);

    // Draw background to screen 1 for slam_background use
    // (In the original, WI_loadData draws the BG to screens[1])
    if !state.bg.is_empty() {
        state.first_refresh = true;
    }

    // Dispatch to appropriate stat screen
    if state.deathmatch {
        wi_init_dm_stats(state);
    } else if state.netgame {
        wi_init_net_game_stats(state);
    } else {
        wi_init_stats(state);
    }
}

/// Advance the intermission state by one tic.
///
/// Equivalent to `WI_Ticker` in `wi_stuff.c` (lines 1502-1536).
/// Increments background counter, plays music on first tic, checks for
/// acceleration, and dispatches to the appropriate update function.
///
/// # Arguments
///
/// * `state` — The intermission state to update.
/// * `players` — Player array for checking button presses.
pub fn wi_ticker(state: &mut IntermissionState, players: &[crate::types::player::Player]) {
    // Increment background animation counter
    state.bcnt += 1;

    // Start intermission music on the first tic
    if state.bcnt == 1 {
        if state.gamemode == GameMode::Commercial {
            state.pending_music = Some(MusicEnum::mus_dm2int);
        } else {
            state.pending_music = Some(MusicEnum::mus_inter);
        }
    }

    // Check if any player wants to accelerate
    wi_check_for_accelerate(state, players);

    // Dispatch to appropriate update function
    match state.state {
        StateEnum::StatCount => {
            if state.deathmatch {
                wi_update_dm_stats(state);
            } else if state.netgame {
                wi_update_net_game_stats(state);
            } else {
                wi_update_stats(state);
            }
        }
        StateEnum::ShowNextLoc => {
            wi_update_show_next_loc(state);
        }
        StateEnum::NoState => {
            wi_update_no_state(state);
        }
    }
}

/// Draw the current intermission screen.
///
/// Equivalent to `WI_Drawer` in `wi_stuff.c` (lines 1810-1836).
/// Dispatches to the appropriate drawing function based on current state.
///
/// # Arguments
///
/// * `state` — The intermission state to draw.
/// * `video` — Video state for rendering patches and screen operations.
pub fn wi_drawer(state: &mut IntermissionState, video: &mut VideoState) {
    // On first refresh, draw background to screen 1 and copy to screen 0
    if state.first_refresh {
        state.first_refresh = false;
        // Draw background patch to screen 1 (backup buffer)
        if !state.bg.is_empty() {
            video.draw_patch(0, 0, 1, &state.bg);
        }
    }

    match state.state {
        StateEnum::StatCount => {
            if state.deathmatch {
                wi_draw_dm_stats(state, video);
            } else if state.netgame {
                wi_draw_net_game_stats(state, video);
            } else {
                wi_draw_stats(state, video);
            }
        }
        StateEnum::ShowNextLoc => {
            wi_draw_show_next_loc(state, video);
        }
        StateEnum::NoState => {
            wi_draw_no_state(state, video);
        }
    }
}

// =========================================================================
// Unit Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_enum_values() {
        assert_ne!(StateEnum::NoState, StateEnum::StatCount);
        assert_ne!(StateEnum::StatCount, StateEnum::ShowNextLoc);
        assert_ne!(StateEnum::NoState, StateEnum::ShowNextLoc);
    }

    #[test]
    fn test_state_enum_default() {
        let s: StateEnum = StateEnum::default();
        assert_eq!(s, StateEnum::StatCount);
    }

    #[test]
    fn test_intermission_state_new() {
        let state = IntermissionState::new();
        assert_eq!(state.state, StateEnum::StatCount);
        assert_eq!(state.accelerate_stage, 0);
        assert_eq!(state.bcnt, 0);
        assert_eq!(state.me, 0);
        assert!(state.first_refresh);
        assert_eq!(state.sp_state, 0);
        assert_eq!(state.ng_state, 0);
        assert_eq!(state.dm_state, 0);
        assert!(!state.snl_pointeron);
        assert!(!state.deathmatch);
        assert!(!state.netgame);
        assert!(!state.dofrags);
        assert!(state.pending_sound.is_none());
        assert!(state.pending_music.is_none());
    }

    #[test]
    fn test_lnodes_episode_0() {
        assert_eq!(LNODES[0][0].x, 185);
        assert_eq!(LNODES[0][0].y, 164);
        assert_eq!(LNODES[0][1].x, 148);
        assert_eq!(LNODES[0][1].y, 143);
        assert_eq!(LNODES[0][2].x, 69);
        assert_eq!(LNODES[0][2].y, 122);
        assert_eq!(LNODES[0][3].x, 209);
        assert_eq!(LNODES[0][3].y, 102);
        assert_eq!(LNODES[0][4].x, 116);
        assert_eq!(LNODES[0][4].y, 89);
        assert_eq!(LNODES[0][5].x, 166);
        assert_eq!(LNODES[0][5].y, 55);
        assert_eq!(LNODES[0][6].x, 71);
        assert_eq!(LNODES[0][6].y, 56);
        assert_eq!(LNODES[0][7].x, 135);
        assert_eq!(LNODES[0][7].y, 29);
        assert_eq!(LNODES[0][8].x, 71);
        assert_eq!(LNODES[0][8].y, 24);
    }

    #[test]
    fn test_lnodes_episode_1() {
        assert_eq!(LNODES[1][0].x, 254);
        assert_eq!(LNODES[1][0].y, 25);
        assert_eq!(LNODES[1][1].x, 97);
        assert_eq!(LNODES[1][1].y, 50);
        assert_eq!(LNODES[1][8].x, 235);
        assert_eq!(LNODES[1][8].y, 158);
    }

    #[test]
    fn test_lnodes_episode_2() {
        assert_eq!(LNODES[2][0].x, 156);
        assert_eq!(LNODES[2][0].y, 168);
        assert_eq!(LNODES[2][8].x, 281);
        assert_eq!(LNODES[2][8].y, 136);
    }

    #[test]
    fn test_numanims() {
        assert_eq!(NUMANIMS[0], 10);
        assert_eq!(NUMANIMS[1], 9);
        assert_eq!(NUMANIMS[2], 6);
        assert_eq!(NUMANIMS[3], 0);
    }

    #[test]
    fn test_constants() {
        assert_eq!(NUMEPISODES, 4);
        assert_eq!(NUMMAPS, 9);
        assert_eq!(WI_TITLEY, 2);
        assert_eq!(WI_SPACINGY, 33);
        assert_eq!(SP_STATSX, 50);
        assert_eq!(SP_STATSY, 50);
        assert_eq!(SP_TIMEX, 16);
        assert_eq!(SP_TIMEY, SCREENHEIGHT - 32);
        assert_eq!(NG_STATSY, 50);
        assert_eq!(NG_SPACINGX, 64);
        assert_eq!(DM_MATRIXX, 42);
        assert_eq!(DM_MATRIXY, 68);
        assert_eq!(DM_SPACINGX, 40);
        assert_eq!(DM_TOTALSX, 269);
        assert_eq!(DM_KILLERSX, 10);
        assert_eq!(DM_KILLERSY, 100);
        assert_eq!(DM_VICTIMSX, 5);
        assert_eq!(DM_VICTIMSY, 50);
    }

    #[test]
    fn test_patch_width_empty() {
        assert_eq!(patch_width(&[]), 0);
        assert_eq!(patch_width(&[1]), 0);
    }

    #[test]
    fn test_patch_width_valid() {
        // Little-endian: width = 320 = 0x0140
        let data = [0x40, 0x01, 0xC8, 0x00, 0, 0, 0, 0];
        assert_eq!(patch_width(&data), 320);
        assert_eq!(patch_height(&data), 200);
    }

    #[test]
    fn test_wi_frag_sum() {
        let mut state = IntermissionState::new();
        state.playeringame[0] = true;
        state.playeringame[1] = true;
        state.wbs.plyr[0].frags = [2, 5, 0, 0]; // 2 self-kills, 5 kills on p1
        state.wbs.plyr[1].frags = [3, 1, 0, 0]; // 3 kills on p0, 1 self-kill

        // Player 0: frags[1]=5 (killed p1) minus frags[0]=2 (self) = 3
        assert_eq!(wi_frag_sum(&state, 0), 3);
        // Player 1: frags[0]=3 (killed p0) minus frags[1]=1 (self) = 2
        assert_eq!(wi_frag_sum(&state, 1), 2);
    }

    #[test]
    fn test_wi_init_variables_clamp() {
        let mut state = IntermissionState::new();
        let wbs = WbStartStruct {
            maxkills: 0,
            maxitems: -5,
            maxsecret: 0,
            last: -1,
            epsd: 100,
            ..Default::default()
        };

        wi_init_variables(&mut state, &wbs);

        // maxkills/items/secret should be clamped to 1
        assert_eq!(state.wbs.maxkills, 1);
        assert_eq!(state.wbs.maxitems, 1);
        assert_eq!(state.wbs.maxsecret, 1);
        // last should be clamped to 0
        assert_eq!(state.wbs.last, 0);
        // epsd should be clamped to NUMEPISODES-1
        assert_eq!(state.wbs.epsd, (NUMEPISODES - 1) as i32);
    }

    #[test]
    fn test_wi_init_anim_data() {
        let mut state = IntermissionState::new();
        wi_init_anim_data(&mut state);

        assert_eq!(state.anims.len(), 4);
        assert_eq!(state.anims[0].len(), 10);
        assert_eq!(state.anims[1].len(), 9);
        assert_eq!(state.anims[2].len(), 6);
        assert_eq!(state.anims[3].len(), 0);

        // Check episode 0 first anim
        assert_eq!(state.anims[0][0].anim_type, AnimEnum::Always);
        assert_eq!(state.anims[0][0].period, TICRATE / 3);
        assert_eq!(state.anims[0][0].nanims, 3);
        assert_eq!(state.anims[0][0].loc.x, 224);
        assert_eq!(state.anims[0][0].loc.y, 104);

        // Check episode 1 level-triggered anims have correct data1
        for i in 0..9 {
            assert_eq!(state.anims[1][i].anim_type, AnimEnum::Level);
            assert_eq!(state.anims[1][i].data1, i as i32);
        }
        // Episode 1 anim 7 has different location
        assert_eq!(state.anims[1][7].loc.x, 192);
        assert_eq!(state.anims[1][7].loc.y, 144);

        // Check episode 2 last anim has different period
        assert_eq!(state.anims[2][5].period, TICRATE / 4);
    }

    #[test]
    fn test_wi_init_no_state() {
        let mut state = IntermissionState::new();
        wi_init_no_state(&mut state);
        assert_eq!(state.state, StateEnum::NoState);
        assert_eq!(state.accelerate_stage, 0);
        assert_eq!(state.cnt, 10);
    }

    #[test]
    fn test_wi_init_show_next_loc() {
        let mut state = IntermissionState::new();
        state.gamemode = GameMode::Commercial;
        wi_init_anim_data(&mut state);
        wi_init_show_next_loc(&mut state);
        assert_eq!(state.state, StateEnum::ShowNextLoc);
        assert_eq!(state.accelerate_stage, 0);
        assert_eq!(state.cnt, SHOWNEXTLOCDELAY * TICRATE);
    }

    #[test]
    fn test_wi_init_stats() {
        let mut state = IntermissionState::new();
        state.gamemode = GameMode::Commercial;
        wi_init_anim_data(&mut state);
        wi_init_stats(&mut state);
        assert_eq!(state.sp_state, 1);
        assert_eq!(state.cnt_kills[0], -1);
        assert_eq!(state.cnt_items[0], -1);
        assert_eq!(state.cnt_secret[0], -1);
        assert_eq!(state.cnt_time, -1);
        assert_eq!(state.cnt_par, -1);
    }

    #[test]
    fn test_anim_def_into_anim() {
        let def = AnimDef::new(AnimEnum::Always, 11, 3, 100, 200);
        let anim = def.into_anim();
        assert_eq!(anim.anim_type, AnimEnum::Always);
        assert_eq!(anim.period, 11);
        assert_eq!(anim.nanims, 3);
        assert_eq!(anim.loc.x, 100);
        assert_eq!(anim.loc.y, 200);
        assert_eq!(anim.ctr, -1);
        assert_eq!(anim.lastdrawn, -1);
    }
}
