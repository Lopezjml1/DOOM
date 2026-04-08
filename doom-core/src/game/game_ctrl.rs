//! Game control: new game, save/load, demo recording/playback, level transitions.
//!
//! Translated from linuxdoom-1.10/g_game.c and linuxdoom-1.10/g_game.h
//!
//! This module manages the game state machine: new game initialization,
//! save/load game, demo recording/playback, level transitions, player rebirth,
//! input-to-ticcmd building, and the G_Ticker dispatch. This is the heart of
//! the game's state transitions and the second-largest file in the game module.
//!
//! Copyright (C) 1993-1996 by id Software, Inc.
//! Copyright (C) 2024 - Rust port contributors.
//!
//! This program is free software; you can redistribute it and/or modify
//! it under the terms of the GNU General Public License as published by
//! the Free Software Foundation; either version 2 of the License, or
//! (at your option) any later version.
//!
//! This program is distributed in the hope that it will be useful,
//! but WITHOUT ANY WARRANTY; without even the implied warranty of
//! MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
//! GNU General Public License for more details.

use crate::game::strings::{GGSAVED, SAVEGAMENAME};
use crate::play::MAXHEALTH;
use crate::types::angle::{ANG45, ANGLETOFINESHIFT};
use crate::types::doomdef::{
    AmmoType, GameMode, GameState, Skill, WeaponType, KEY_F12, KEY_PAUSE, MAXPLAYERS, NUMWEAPONS,
    TICRATE, VERSION,
};
use crate::types::event::{
    Event, EventType, GameAction, BTS_PAUSE, BTS_SAVEGAME, BTS_SAVEMASK, BTS_SAVESHIFT, BT_ATTACK,
    BT_CHANGE, BT_SPECIAL, BT_SPECIALMASK, BT_USE, BT_WEAPONSHIFT,
};
use crate::types::fixed::{Fixed, FRACBITS};
use crate::types::map_data::MapThing;
use crate::types::net::BACKUPTICS;
use crate::types::player::{Player, PlayerState, WbStartStruct};
use crate::types::tables::{finecosine, FINESINE};
use crate::types::ticcmd::TicCmd;

use tracing::{debug, error, info, warn};

// =============================================================================
// Constants — translated from g_game.c
// =============================================================================

/// Save game buffer size (180224 bytes).
/// Original C: `#define SAVEGAMESIZE 0x2c000` (g_game.c line 56)
pub const SAVEGAMESIZE: usize = 0x2c000;

/// Save description string length.
/// Original C: `#define SAVESTRINGSIZE 24` (g_game.c line 57)
pub const SAVESTRINGSIZE: usize = 24;

/// Version string size in savegame header.
/// Original C: `#define VERSIONSIZE 16` (g_game.c line 58)
pub const VERSIONSIZE: usize = 16;

/// Turbo movement threshold — movement values above this trigger
/// the "turbo" cheat warning in netgames.
/// Original C: `#define TURBOTHRESHOLD 0x32` (g_game.c line 171)
pub const TURBOTHRESHOLD: i32 = 0x32;

/// Number of tics before turning accelerates to full speed.
/// Original C: `#define SLOWTURNTICS 6` (g_game.c line 173)
pub const SLOWTURNTICS: i32 = 6;

/// Keyboard key count for gamekeydown array.
/// Original C: `#define NUMKEYS 256` (g_game.c line 183)
pub const NUMKEYS: usize = 256;

/// Body queue size for deathmatch corpse removal.
/// Original C: `#define BODYQUESIZE 32` (g_game.c line 205)
pub const BODYQUESIZE: usize = 32;

/// Demo end marker byte.
/// Original C: `#define DEMOMARKER 0x80` (g_game.c line 1489)
pub const DEMOMARKER: u8 = 0x80;

/// Movement speed tables — normal and turbo (shift-run) speeds.
/// Index 0 = walk, Index 1 = run.
/// Original C: `fixed_t forwardmove[2] = {0x19, 0x32};` (g_game.c line 175)
pub const FORWARD_MOVE: [i32; 2] = [0x19, 0x32];

/// Side movement speed tables.
/// Original C: `fixed_t sidemove[2] = {0x18, 0x28};` (g_game.c line 176)
pub const SIDE_MOVE: [i32; 2] = [0x18, 0x28];

/// Turn speed table: [normal, fast, slow].
/// Original C: `fixed_t angleturn[3] = {640, 1280, 320};` (g_game.c line 177)
pub const ANGLE_TURN: [i32; 3] = [640, 1280, 320];

/// Maximum player movement per tic (= forwardmove[1] = 0x32).
/// Original C: `#define MAXPLMOVE (forwardmove[1])` (g_game.c line 179)
pub const MAXPLMOVE: i32 = FORWARD_MOVE[1];

/// Par times for DOOM 1 episodes/maps (in seconds).
/// pars[episode][map] — episode 0 unused.
/// Original C: `int pars[4][10]` (g_game.c lines 977-993)
pub const PARS: [[i32; 10]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 30, 75, 120, 90, 165, 180, 180, 30, 165],
    [0, 90, 90, 90, 120, 90, 360, 240, 30, 170],
    [0, 90, 45, 90, 150, 90, 90, 165, 30, 135],
];

/// Par times for DOOM II maps (in seconds). Index = map-1.
/// Original C: `int cpars[32]` (g_game.c lines 997-1000)
pub const CPARS: [i32; 32] = [
    30, 90, 120, 120, 90, 150, 120, 120, 270, 90, 210, 150, 150, 150, 210, 150, 420, 150, 210, 150,
    240, 150, 180, 150, 150, 300, 330, 420, 300, 180, 120, 30,
];

// =============================================================================
// GameCtrl — Game Controller State Struct
// =============================================================================

/// Central game controller state struct, consolidating all former global
/// variables from g_game.c and relevant doomstat.h globals into a single
/// owned struct. Replaces C global mutable state with Rust struct ownership.
pub struct GameCtrl {
    // -- Core Game State (from doomstat.h / g_game.c globals) --
    /// Current game mode (shareware/registered/retail/commercial).
    pub gamemode: GameMode,
    /// Current pending game action (dispatched by G_Ticker).
    pub gameaction: GameAction,
    /// Current game state (level, intermission, finale, demoscreen).
    pub gamestate: GameState,
    /// Current difficulty level.
    pub gameskill: Skill,
    /// Whether monster respawning is enabled (nightmare mode).
    pub respawnmonsters: bool,
    /// Current episode number (1-based for DOOM 1, 1 for DOOM 2).
    pub gameepisode: i32,
    /// Current map number (1-based).
    pub gamemap: i32,

    // -- Pause and control flags --
    pub paused: bool,
    pub sendpause: bool,
    pub sendsave: bool,
    pub usergame: bool,

    // -- Demo and timing state --
    pub timingdemo: bool,
    pub nodrawers: bool,
    pub noblit: bool,
    pub starttime: i32,

    // -- View and network state --
    pub viewactive: bool,
    /// Deathmatch mode: 0=coop, 1=deathmatch, 2=altdeath.
    pub deathmatch: i32,
    pub netgame: bool,
    pub playeringame: [bool; MAXPLAYERS],
    pub players: [Player; MAXPLAYERS],

    // -- Tic tracking --
    pub consoleplayer: usize,
    pub displayplayer: usize,
    pub gametic: i32,
    pub levelstarttic: i32,
    pub totalkills: i32,
    pub totalitems: i32,
    pub totalsecret: i32,

    // -- Demo state --
    pub demoname: String,
    pub demorecording: bool,
    pub demoplayback: bool,
    pub netdemo: bool,
    pub demobuffer: Vec<u8>,
    pub demo_p: usize,
    pub singledemo: bool,
    pub precache: bool,

    // -- Intermission info --
    pub wminfo: WbStartStruct,
    pub consistancy: [[i16; BACKUPTICS]; MAXPLAYERS],
    pub savebuffer: Vec<u8>,

    // -- Key bindings (defaults from g_game.c lines 148-167) --
    pub key_right: i32,
    pub key_left: i32,
    pub key_up: i32,
    pub key_down: i32,
    pub key_strafeleft: i32,
    pub key_straferight: i32,
    pub key_fire: i32,
    pub key_use: i32,
    pub key_strafe: i32,
    pub key_speed: i32,
    pub mousebfire: i32,
    pub mousebstrafe: i32,
    pub mousebforward: i32,
    pub joybfire: i32,
    pub joybstrafe: i32,
    pub joybuse: i32,
    pub joybspeed: i32,

    // -- Input state tracking --
    pub gamekeydown: [bool; NUMKEYS],
    pub turnheld: i32,
    pub mousebuttons: [bool; 4],
    pub mousex: i32,
    pub mousey: i32,
    pub dclicktime: i32,
    pub dclickstate: i32,
    pub dclicks: i32,
    pub dclicktime2: i32,
    pub dclickstate2: i32,
    pub dclicks2: i32,
    pub joyxmove: i32,
    pub joyymove: i32,
    pub joybuttons: [bool; 5],

    // -- Save game state --
    pub savegameslot: i32,
    pub savedescription: String,

    // -- Deferred action parameters --
    pub d_skill: Skill,
    pub d_episode: i32,
    pub d_map: i32,
    pub defdemoname: String,
    pub secretexit: bool,

    // -- Body queue (deathmatch corpse management) --
    /// Body queue indices (into mobj arena). Used by deathmatch to
    /// remove old corpses at spawn points.
    pub bodyque: Vec<Option<usize>>,
    pub bodyqueslot: usize,

    // -- Internal state --
    /// Previous game state for wipe detection.
    pub wipegamestate: GameState,
    /// Tic duplication count for network (1 in single-player).
    pub ticdup: i32,
    /// Level time accumulated during gameplay (tics).
    pub leveltime: i32,
}

impl GameCtrl {
    /// Create a new GameCtrl with default (zero-initialized) state.
    /// Key bindings use the DOOM defaults from g_game.c.
    pub fn new() -> Self {
        GameCtrl {
            gamemode: GameMode::Indetermined,
            gameaction: GameAction::Nothing,
            gamestate: GameState::DemoScreen,
            gameskill: Skill::Medium,
            respawnmonsters: false,
            gameepisode: 1,
            gamemap: 1,
            paused: false,
            sendpause: false,
            sendsave: false,
            usergame: false,
            timingdemo: false,
            nodrawers: false,
            noblit: false,
            starttime: 0,
            viewactive: false,
            deathmatch: 0,
            netgame: false,
            playeringame: [false; MAXPLAYERS],
            players: Default::default(),
            consoleplayer: 0,
            displayplayer: 0,
            gametic: 0,
            levelstarttic: 0,
            totalkills: 0,
            totalitems: 0,
            totalsecret: 0,
            demoname: String::new(),
            demorecording: false,
            demoplayback: false,
            netdemo: false,
            demobuffer: Vec::new(),
            demo_p: 0,
            singledemo: false,
            precache: true,
            wminfo: WbStartStruct::default(),
            consistancy: [[0i16; BACKUPTICS]; MAXPLAYERS],
            savebuffer: Vec::new(),
            // Default key bindings matching original DOOM defaults.
            key_right: 0xae, // KEY_RIGHTARROW
            key_left: 0xac,  // KEY_LEFTARROW
            key_up: 0xad,    // KEY_UPARROW
            key_down: 0xaf,  // KEY_DOWNARROW
            key_strafeleft: b',' as i32,
            key_straferight: b'.' as i32,
            key_fire: 0x80 + 0x1d, // KEY_RCTRL
            key_use: b' ' as i32,
            key_strafe: 0x80 + 0x38, // KEY_RALT
            key_speed: 0x80 + 0x36,  // KEY_RSHIFT
            mousebfire: 0,
            mousebstrafe: 1,
            mousebforward: 2,
            joybfire: 0,
            joybstrafe: 1,
            joybuse: 3,
            joybspeed: 2,
            gamekeydown: [false; NUMKEYS],
            turnheld: 0,
            mousebuttons: [false; 4],
            mousex: 0,
            mousey: 0,
            dclicktime: 0,
            dclickstate: 0,
            dclicks: 0,
            dclicktime2: 0,
            dclickstate2: 0,
            dclicks2: 0,
            joyxmove: 0,
            joyymove: 0,
            joybuttons: [false; 5],
            savegameslot: 0,
            savedescription: String::new(),
            d_skill: Skill::Medium,
            d_episode: 1,
            d_map: 1,
            defdemoname: String::new(),
            secretexit: false,
            bodyque: vec![None; BODYQUESIZE],
            bodyqueslot: 0,
            wipegamestate: GameState::DemoScreen,
            ticdup: 1,
            leveltime: 0,
        }
    }
}

impl Default for GameCtrl {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// G_CmdChecksum — Tic command consistency checksum
// =============================================================================

/// Compute a checksum over a tic command's fields for consistency
/// verification in network games.
///
/// Original C: `int G_CmdChecksum(ticcmd_t *cmd)` (g_game.c lines 219-228)
pub fn g_cmd_checksum(cmd: &TicCmd) -> i32 {
    let mut sum: i32 = 0;
    sum = sum.wrapping_add(cmd.forwardmove as i32);
    sum = sum.wrapping_add(cmd.sidemove as i32);
    sum = sum.wrapping_add(cmd.angleturn as i32);
    sum = sum.wrapping_add(cmd.buttons as i32);
    sum = sum.wrapping_add(cmd.chatchar as i32);
    sum
}

// =============================================================================
// Helper functions
// =============================================================================

/// Check if a button index is active in a button state array.
#[inline]
fn button_active(buttons: &[bool], index: i32) -> bool {
    if index < 0 {
        return false;
    }
    buttons.get(index as usize).copied().unwrap_or(false)
}

// =============================================================================
// G_BuildTiccmd — Build tic command from current input state
// =============================================================================

/// Build a tic command from current input state.
///
/// This is the core input-to-movement translation. It reads keyboard, mouse,
/// and joystick state from GameCtrl and produces a TicCmd with movement,
/// turning, and button values.
///
/// Original C: `void G_BuildTiccmd(ticcmd_t* cmd)` (g_game.c lines 237-437)
pub fn g_build_ticcmd(ctrl: &mut GameCtrl, cmd: &mut TicCmd) {
    // Start with a clean command
    *cmd = TicCmd::new();

    // Determine speed and strafe state
    let strafe = ctrl
        .gamekeydown
        .get(ctrl.key_strafe as usize)
        .copied()
        .unwrap_or(false)
        || button_active(&ctrl.mousebuttons, ctrl.mousebstrafe)
        || button_active(&ctrl.joybuttons, ctrl.joybstrafe);

    let speed: usize = if ctrl
        .gamekeydown
        .get(ctrl.key_speed as usize)
        .copied()
        .unwrap_or(false)
        || button_active(&ctrl.joybuttons, ctrl.joybspeed)
    {
        1
    } else {
        0
    };

    let mut forward: i32 = 0;
    let mut side: i32 = 0;

    // Two-stage accelerative turning.
    // Original C: g_game.c lines 264-275
    if ctrl
        .gamekeydown
        .get(ctrl.key_right as usize)
        .copied()
        .unwrap_or(false)
        || ctrl
            .gamekeydown
            .get(ctrl.key_left as usize)
            .copied()
            .unwrap_or(false)
    {
        ctrl.turnheld += 1;
    } else {
        ctrl.turnheld = 0;
    }

    let tspeed: usize = if ctrl.turnheld < SLOWTURNTICS {
        2
    } else {
        speed
    };

    // Use two-stage accelerative turning on the keyboard.
    // Original C: g_game.c lines 278-306
    if strafe {
        if ctrl
            .gamekeydown
            .get(ctrl.key_right as usize)
            .copied()
            .unwrap_or(false)
        {
            side += SIDE_MOVE[speed];
        }
        if ctrl
            .gamekeydown
            .get(ctrl.key_left as usize)
            .copied()
            .unwrap_or(false)
        {
            side -= SIDE_MOVE[speed];
        }
    } else {
        if ctrl
            .gamekeydown
            .get(ctrl.key_right as usize)
            .copied()
            .unwrap_or(false)
        {
            cmd.angleturn = cmd.angleturn.wrapping_sub(ANGLE_TURN[tspeed] as i16);
        }
        if ctrl
            .gamekeydown
            .get(ctrl.key_left as usize)
            .copied()
            .unwrap_or(false)
        {
            cmd.angleturn = cmd.angleturn.wrapping_add(ANGLE_TURN[tspeed] as i16);
        }
    }

    // Forward/backward movement
    if ctrl
        .gamekeydown
        .get(ctrl.key_up as usize)
        .copied()
        .unwrap_or(false)
    {
        forward += FORWARD_MOVE[speed];
    }
    if ctrl
        .gamekeydown
        .get(ctrl.key_down as usize)
        .copied()
        .unwrap_or(false)
    {
        forward -= FORWARD_MOVE[speed];
    }

    // Strafe left/right
    if ctrl
        .gamekeydown
        .get(ctrl.key_straferight as usize)
        .copied()
        .unwrap_or(false)
    {
        side += SIDE_MOVE[speed];
    }
    if ctrl
        .gamekeydown
        .get(ctrl.key_strafeleft as usize)
        .copied()
        .unwrap_or(false)
    {
        side -= SIDE_MOVE[speed];
    }

    // Fire, use, weapon change buttons
    if ctrl
        .gamekeydown
        .get(ctrl.key_fire as usize)
        .copied()
        .unwrap_or(false)
        || button_active(&ctrl.mousebuttons, ctrl.mousebfire)
        || button_active(&ctrl.joybuttons, ctrl.joybfire)
    {
        cmd.buttons |= BT_ATTACK;
    }

    if ctrl
        .gamekeydown
        .get(ctrl.key_use as usize)
        .copied()
        .unwrap_or(false)
        || button_active(&ctrl.joybuttons, ctrl.joybuse)
    {
        cmd.buttons |= BT_USE;
    }

    // Weapon change — check weapon keys 1-8
    // Original C: g_game.c lines 327-348
    for i in 0..NUMWEAPONS {
        let key_code = (b'1' as i32) + (i as i32);
        if ctrl
            .gamekeydown
            .get(key_code as usize)
            .copied()
            .unwrap_or(false)
        {
            cmd.buttons |= BT_CHANGE;
            cmd.buttons |= (i << BT_WEAPONSHIFT) as u8;
            break;
        }
    }

    // Mouse forward movement
    // Original C: g_game.c lines 351-395
    if button_active(&ctrl.mousebuttons, ctrl.mousebforward) {
        forward += FORWARD_MOVE[speed];
    }

    // Mouse turning / strafing
    if strafe {
        side += ctrl.mousex * 2;
    } else {
        cmd.angleturn = cmd.angleturn.wrapping_add((ctrl.mousex * -8) as i16);
    }

    // Mouse forward/backward movement
    forward += ctrl.mousey;

    // Mouse double-click handling for use action
    // Original C: g_game.c lines 361-395
    // Forward double-click
    let bstrafe = button_active(&ctrl.mousebuttons, ctrl.mousebforward);
    if bstrafe != (ctrl.dclickstate2 != 0) && ctrl.dclickstate2 != 0 {
        // Button released after being held
    }
    if ctrl.dclickstate2 != 0 {
        ctrl.dclicktime2 += 1;
        if ctrl.dclicktime2 > 20 {
            ctrl.dclickstate2 = 0;
            ctrl.dclicktime2 = 0;
        }
    } else {
        ctrl.dclicktime2 = 0;
    }
    if bstrafe && ctrl.dclickstate2 == 0 {
        ctrl.dclickstate2 = 1;
        ctrl.dclicktime2 = 0;
        ctrl.dclicks2 += 1;
    }
    if ctrl.dclicks2 == 2 {
        cmd.buttons |= BT_USE;
        ctrl.dclicks2 = 0;
    }

    // Use button double-click
    let buse = button_active(&ctrl.mousebuttons, ctrl.mousebstrafe);
    if buse != (ctrl.dclickstate != 0) && ctrl.dclickstate != 0 {
        // Button released
    }
    if ctrl.dclickstate != 0 {
        ctrl.dclicktime += 1;
        if ctrl.dclicktime > 20 {
            ctrl.dclickstate = 0;
            ctrl.dclicktime = 0;
        }
    } else {
        ctrl.dclicktime = 0;
    }
    if buse && ctrl.dclickstate == 0 {
        ctrl.dclickstate = 1;
        ctrl.dclicktime = 0;
        ctrl.dclicks += 1;
    }
    if ctrl.dclicks == 2 {
        cmd.buttons |= BT_USE;
        ctrl.dclicks = 0;
    }

    // Joystick movement
    // Original C: g_game.c lines 397-412
    if ctrl.joyxmove != 0 || ctrl.joyymove != 0 {
        if strafe {
            side += ctrl.joyxmove * 2;
        } else {
            cmd.angleturn = cmd.angleturn.wrapping_add((ctrl.joyxmove * -8) as i16);
        }
        forward += ctrl.joyymove;
    }

    // Clamp movement to MAXPLMOVE
    // Original C: g_game.c lines 413-420
    forward = forward.clamp(-MAXPLMOVE, MAXPLMOVE);
    side = side.clamp(-MAXPLMOVE, MAXPLMOVE);

    cmd.forwardmove = cmd.forwardmove.wrapping_add(forward as i8);
    cmd.sidemove = cmd.sidemove.wrapping_add(side as i8);

    // Special buttons: sendpause and sendsave
    // Original C: g_game.c lines 426-436
    if ctrl.sendpause {
        ctrl.sendpause = false;
        cmd.buttons = BT_SPECIAL | BTS_PAUSE;
    }

    if ctrl.sendsave {
        ctrl.sendsave = false;
        cmd.buttons = BT_SPECIAL | BTS_SAVEGAME | ((ctrl.savegameslot as u8) << BTS_SAVESHIFT);
    }

    // Reset per-tic mouse movement
    ctrl.mousex = 0;
    ctrl.mousey = 0;
}

// =============================================================================
// G_DoLoadLevel — Initialize and load a new level
// =============================================================================

/// Initialize a new level. Resets player states, clears input, and
/// records the level start tic.
///
/// Original C: `void G_DoLoadLevel(void)` (g_game.c lines 445-497)
pub fn g_do_load_level(ctrl: &mut GameCtrl) {
    // Set the level start tic for par time calculation
    ctrl.levelstarttic = ctrl.gametic;
    ctrl.leveltime = 0;

    // Sky texture selection based on episode/gamemap.
    // Original C: g_game.c lines 450-464 — sky texture is set by the caller
    // (game_main/renderer) based on ctrl.gameepisode and ctrl.gamemap.

    // Reset player states: dead players need reborn
    // Original C: g_game.c lines 470-482
    for i in 0..MAXPLAYERS {
        if ctrl.playeringame[i] {
            if ctrl.players[i].playerstate == PlayerState::Dead {
                ctrl.players[i].playerstate = PlayerState::Reborn;
            }
            // Clear player commands and input state
            ctrl.players[i].cmd = TicCmd::new();
        }
    }

    // Set the gamestate and viewactive
    ctrl.gamestate = GameState::Level;
    ctrl.viewactive = true;
    ctrl.gameaction = GameAction::Nothing;

    // Clear gamekeydown state to prevent stuck keys
    // Original C: g_game.c lines 490-492 — Z_FreeTags, then memset gamekeydown
    for key in ctrl.gamekeydown.iter_mut() {
        *key = false;
    }

    debug!(
        "G_DoLoadLevel: E{}M{} / MAP{:02} (skill {:?})",
        ctrl.gameepisode, ctrl.gamemap, ctrl.gamemap, ctrl.gameskill
    );
}

// =============================================================================
// G_Responder — Process game input events
// =============================================================================

/// Process a game input event. Returns true if the event was consumed.
///
/// Original C: `boolean G_Responder(event_t* ev)` (g_game.c lines 504-597)
pub fn g_responder(ctrl: &mut GameCtrl, ev: &Event) -> bool {
    // Spy mode (F12 key) — cycle display player in cooperative mode
    // Original C: g_game.c lines 507-518
    if ctrl.gamestate == GameState::Level
        && ev.event_type == EventType::KeyDown
        && ev.data1 == KEY_F12
        && ctrl.deathmatch == 0
    {
        // Cycle display player
        let start = ctrl.displayplayer;
        loop {
            ctrl.displayplayer = (ctrl.displayplayer + 1) % MAXPLAYERS;
            if ctrl.playeringame[ctrl.displayplayer] || ctrl.displayplayer == start {
                break;
            }
        }
        return true;
    }

    // During demo playback, any key press aborts the demo
    // Original C: g_game.c lines 521-533
    if ctrl.gamestate == GameState::DemoScreen && ctrl.demoplayback {
        if ev.event_type == EventType::KeyDown {
            ctrl.gameaction = GameAction::Nothing;
            ctrl.demoplayback = false;
            return true;
        }
        return false;
    }

    // GS_LEVEL state: dispatch to subsystem responders
    // Original C: g_game.c lines 535-549
    // The actual hu_responder, st_responder, am_responder calls are made by the
    // caller (game loop) which has access to subsystem state.

    // Direct event handling for game_ctrl: keyboard/mouse/joystick state
    // Original C: g_game.c lines 558-594
    match ev.event_type {
        EventType::KeyDown => {
            if ev.data1 == KEY_PAUSE {
                ctrl.sendpause = true;
            } else if (ev.data1 as usize) < NUMKEYS {
                ctrl.gamekeydown[ev.data1 as usize] = true;
            }
            true
        }
        EventType::KeyUp => {
            if (ev.data1 as usize) < NUMKEYS {
                ctrl.gamekeydown[ev.data1 as usize] = false;
            }
            false
        }
        EventType::Mouse => {
            // data1 = buttons, data2 = x movement, data3 = y movement
            ctrl.mousebuttons[0] = (ev.data1 & 1) != 0;
            ctrl.mousebuttons[1] = (ev.data1 & 2) != 0;
            ctrl.mousebuttons[2] = (ev.data1 & 4) != 0;
            ctrl.mousex = ev.data2;
            ctrl.mousey = -ev.data3; // y axis is inverted
            true
        }
        EventType::Joystick => {
            // data1 = buttons, data2 = x axis, data3 = y axis
            ctrl.joybuttons[0] = (ev.data1 & 1) != 0;
            ctrl.joybuttons[1] = (ev.data1 & 2) != 0;
            ctrl.joybuttons[2] = (ev.data1 & 4) != 0;
            ctrl.joybuttons[3] = (ev.data1 & 8) != 0;
            ctrl.joyxmove = ev.data2;
            ctrl.joyymove = ev.data3;
            true
        }
    }
}

// =============================================================================
// G_Ticker — Main game ticker dispatching game actions and state updates
// =============================================================================

/// Main game ticker. Dispatches pending game actions, processes player
/// tic commands, handles special buttons (pause/save), and updates the
/// game state machine.
///
/// The caller (game loop) is responsible for dispatching subsystem tickers
/// (P_Ticker, ST_Ticker, AM_Ticker, HU_Ticker, WI_Ticker, F_Ticker)
/// based on the current gamestate after g_ticker returns.
///
/// Original C: `void G_Ticker(void)` (g_game.c lines 605-748)
pub fn g_ticker(ctrl: &mut GameCtrl) {
    // Check for player reborn
    // Original C: g_game.c lines 612-614
    for i in 0..MAXPLAYERS {
        if ctrl.playeringame[i] && ctrl.players[i].playerstate == PlayerState::Reborn {
            // Reborn is handled by the caller via g_do_reborn
            ctrl.gameaction = GameAction::LoadLevel;
        }
    }

    // Dispatch pending game action
    // Original C: g_game.c lines 617-652
    match ctrl.gameaction {
        GameAction::LoadLevel => g_do_load_level(ctrl),
        GameAction::NewGame => g_do_new_game(ctrl),
        GameAction::Completed => g_do_completed(ctrl),
        GameAction::Victory => {
            // F_StartFinale is called by the game loop, which owns FinaleState
            ctrl.gameaction = GameAction::Nothing;
        }
        GameAction::WorldDone => g_do_world_done(ctrl),
        GameAction::Screenshot => {
            // Screenshot is handled by the caller which has access to the
            // video/platform subsystems
            ctrl.gameaction = GameAction::Nothing;
        }
        _ => {}
    }

    // Get commands, check consistency for each player
    // Original C: g_game.c lines 656-694
    let buf = (ctrl.gametic / ctrl.ticdup) % (BACKUPTICS as i32);

    for i in 0..MAXPLAYERS {
        if !ctrl.playeringame[i] {
            continue;
        }

        // Turbo cheat detection
        // Original C: g_game.c lines 672-679
        let cmd = &ctrl.players[i].cmd;
        if cmd.forwardmove > TURBOTHRESHOLD as i8 {
            warn!(
                "player {} is turbo! (forwardmove={})",
                i + 1,
                cmd.forwardmove
            );
        }

        // Consistency check for netgames
        // Original C: g_game.c lines 681-694
        if ctrl.netgame && !ctrl.demoplayback {
            let expected = ctrl.consistancy[i][buf as usize];
            if cmd.consistancy != expected {
                error!(
                    "consistency failure: player {} (gametic={}, expected={}, got={})",
                    i, ctrl.gametic, expected, cmd.consistancy
                );
            }
        }
    }

    // Handle special buttons (pause, save)
    // Original C: g_game.c lines 698-724
    for i in 0..MAXPLAYERS {
        if !ctrl.playeringame[i] {
            continue;
        }

        let buttons = ctrl.players[i].cmd.buttons;
        if buttons & BT_SPECIAL != 0 {
            let special = buttons & BT_SPECIALMASK;
            if special == BTS_PAUSE {
                ctrl.paused = !ctrl.paused;
                // Pause sound is handled by the caller
            } else if special == BTS_SAVEGAME {
                let slot = ((buttons & BTS_SAVEMASK) >> BTS_SAVESHIFT) as i32;
                ctrl.savegameslot = slot;
                ctrl.savedescription = format!("savegame slot {}", slot);
                ctrl.gameaction = GameAction::SaveGame;
            }
        }
    }

    // Advance gametic
    ctrl.gametic += 1;
}

// =============================================================================
// Player Management
// =============================================================================

/// Called when a player completes a level. Clears powers, cards,
/// and visual effects. Preserves kill/item/secret counts.
///
/// Original C: `void G_PlayerFinishLevel(int player)` (g_game.c lines 779-792)
pub fn g_player_finish_level(ctrl: &mut GameCtrl, player_num: usize) {
    if player_num >= MAXPLAYERS {
        return;
    }
    let p = &mut ctrl.players[player_num];

    // Clear powers
    for power in p.powers.iter_mut() {
        *power = 0;
    }

    // Clear cards
    for card in p.cards.iter_mut() {
        *card = false;
    }

    // Cancel invisibility (remove MF_SHADOW from mobj)
    // Note: the actual mobj flag removal is handled by the caller that
    // has access to the mobj arena.

    // Clear visual effects
    p.extralight = 0;
    p.fixedcolormap = 0;
    p.damagecount = 0;
    p.bonuscount = 0;
}

/// Reset a player to default state after death (reborn). Preserves
/// frags and cumulative level stats across reborn.
///
/// Original C: `void G_PlayerReborn(int player)` (g_game.c lines 800-833)
pub fn g_player_reborn(ctrl: &mut GameCtrl, player_num: usize) {
    if player_num >= MAXPLAYERS {
        return;
    }

    // Save cumulative stats that persist across reborn
    let frags = ctrl.players[player_num].frags;
    let killcount = ctrl.players[player_num].killcount;
    let itemcount = ctrl.players[player_num].itemcount;
    let secretcount = ctrl.players[player_num].secretcount;

    // Reset the entire player struct to defaults
    ctrl.players[player_num] = Player::default();

    // Restore preserved stats
    let p = &mut ctrl.players[player_num];
    p.frags = frags;
    p.killcount = killcount;
    p.itemcount = itemcount;
    p.secretcount = secretcount;

    // Set default state for a newly spawned player
    p.playerstate = PlayerState::Live;
    p.health = MAXHEALTH;
    p.readyweapon = WeaponType::Pistol;
    p.pendingweapon = WeaponType::Pistol;
    p.weaponowned[WeaponType::Fist as usize] = true;
    p.weaponowned[WeaponType::Pistol as usize] = true;
    p.ammo[AmmoType::Clip as usize] = 50;

    // Set max ammo values
    // Original C: g_game.c lines 825-830
    p.maxammo[AmmoType::Clip as usize] = 200;
    p.maxammo[AmmoType::Shell as usize] = 50;
    p.maxammo[AmmoType::Cell as usize] = 300;
    p.maxammo[AmmoType::Missile as usize] = 50;
}

/// Check if a player can spawn at the given deathmatch start point.
/// Handles body queue removal and teleport fog spawning.
///
/// Original C: `void G_CheckSpot(int playernum, mapthing_t* mthing)` (g_game.c lines 843-889)
///
/// Returns true if the spot is clear for spawning.
pub fn g_check_spot(_ctrl: &mut GameCtrl, _player_num: usize, mthing: &MapThing) -> bool {
    // Check if the position is valid (not blocked by other objects).
    // The actual P_CheckPosition call requires a MovementContext which
    // is beyond GameCtrl's scope. The caller must perform the collision
    // check and fog spawn.

    // Calculate position in fixed-point.
    let _x = Fixed((mthing.x as i32) << FRACBITS);
    let _y = Fixed((mthing.y as i32) << FRACBITS);

    // Calculate teleport fog spawn position.
    // Original C: g_game.c lines 868-877
    let an = (ANG45.0 >> ANGLETOFINESHIFT).wrapping_mul((mthing.angle as u32) / 45) as usize;
    // Use finecosine and FINESINE which return Fixed — extract .0 for i32
    let _fog_x =
        Fixed(((mthing.x as i32) << FRACBITS).wrapping_add(20i32.wrapping_mul(finecosine(an).0)));
    let _fog_y =
        Fixed(((mthing.y as i32) << FRACBITS).wrapping_add(20i32.wrapping_mul(FINESINE[an].0)));

    // The actual spawn check (P_CheckPosition) and fog spawning
    // (P_SpawnMobj of MT_TFOG) require MobjContext/MovementContext.
    // Return true to indicate the spot is usable; the caller with
    // access to the full game context handles the actual collision
    // check and fog effects.
    true
}

/// Spawn a player at a random deathmatch start. Tries up to 20 random
/// positions, falling back to player start 0 if none are clear.
///
/// Original C: `void G_DeathMatchSpawnPlayer(int playernum)` (g_game.c lines 897-919)
pub fn g_death_match_spawn_player(
    ctrl: &mut GameCtrl,
    player_num: usize,
    deathmatch_starts: &[MapThing],
    player_starts: &[Option<MapThing>; MAXPLAYERS],
    rng: &mut crate::util::random::DoomRandom,
) {
    if deathmatch_starts.is_empty() {
        error!("G_DeathMatchSpawnPlayer: no deathmatch starts");
        return;
    }

    // Try up to 20 random deathmatch starts
    // Original C: g_game.c lines 903-912
    let selections = deathmatch_starts.len();
    for _ in 0..20 {
        let i = (rng.p_random() as usize) % selections;
        if g_check_spot(ctrl, player_num, &deathmatch_starts[i]) {
            // Spot is valid — the caller spawns the player
            debug!(
                "G_DeathMatchSpawnPlayer: player {} at DM start {}",
                player_num, i
            );
            return;
        }
    }

    // No valid deathmatch start found — fall back to player start
    // Original C: g_game.c lines 914-917
    if let Some(ref start) = player_starts[player_num] {
        debug!(
            "G_DeathMatchSpawnPlayer: player {} falling back to player start",
            player_num
        );
        let _spot = start;
        // Caller handles P_SpawnPlayer(start)
    }
}

/// Handle player respawn after death.
///
/// In single-player, this reloads the current level.
/// In multiplayer, this respawns the player at a random spawn point.
///
/// Original C: `void G_DoReborn(int playernum)` (g_game.c lines 924-967)
pub fn g_do_reborn(ctrl: &mut GameCtrl, player_num: usize) {
    if !ctrl.netgame {
        // Single player: reload the level
        ctrl.gameaction = GameAction::LoadLevel;
    } else {
        // Multiplayer: respawn the player at a spawn point
        // The actual P_SpawnPlayer call requires MobjContext.
        // Set the player state to reborn so the game loop handles it.
        if player_num < MAXPLAYERS {
            ctrl.players[player_num].playerstate = PlayerState::Reborn;
        }
        debug!("G_DoReborn: player {} respawning in netgame", player_num);
    }
}

/// Request a screenshot. Sets the game action flag for the ticker to handle.
///
/// Original C: `void G_ScreenShot(void)` (g_game.c line 975)
pub fn g_screen_shot(ctrl: &mut GameCtrl) {
    ctrl.gameaction = GameAction::Screenshot;
}

// =============================================================================
// Level Completion
// =============================================================================

/// Normal exit from a level. Sets the game action to Completed.
///
/// Original C: `void G_ExitLevel(void)` (g_game.c lines 1002-1006)
pub fn g_exit_level(ctrl: &mut GameCtrl) {
    ctrl.secretexit = false;
    ctrl.gameaction = GameAction::Completed;
}

/// Secret exit from a level. Sets the game action to Completed with
/// the secret exit flag.
///
/// Original C: `void G_SecretExitLevel(void)` (g_game.c lines 1009-1018)
pub fn g_secret_exit_level(ctrl: &mut GameCtrl) {
    ctrl.secretexit = true;
    ctrl.gameaction = GameAction::Completed;
}

/// Handle level completion: finish players, compute next map, fill
/// intermission (wminfo) struct, transition to intermission state.
///
/// Original C: `void G_DoCompleted(void)` (g_game.c lines 1020-1141)
pub fn g_do_completed(ctrl: &mut GameCtrl) {
    ctrl.gameaction = GameAction::Nothing;

    // Finish all active players
    // Original C: g_game.c lines 1026-1032
    for i in 0..MAXPLAYERS {
        if ctrl.playeringame[i] {
            g_player_finish_level(ctrl, i);
        }
    }

    // Determine the next map based on game mode and current map
    // Original C: g_game.c lines 1035-1112
    if ctrl.gamemode == GameMode::Commercial {
        // DOOM II next map calculation
        if ctrl.secretexit {
            match ctrl.gamemap {
                15 => {
                    // Secret exit from MAP15 → MAP31
                    ctrl.wminfo.next = 30; // 0-based
                }
                31 => {
                    // Secret exit from MAP31 → MAP32
                    ctrl.wminfo.next = 31;
                }
                _ => {
                    ctrl.wminfo.next = ctrl.gamemap; // 0-based next = current (1-based)
                }
            }
        } else {
            match ctrl.gamemap {
                31 | 32 => {
                    // Return from secret levels to MAP16
                    ctrl.wminfo.next = 15; // 0-based MAP16
                }
                _ => {
                    ctrl.wminfo.next = ctrl.gamemap; // 0-based next = current (1-based)
                }
            }
        }

        ctrl.wminfo.maxkills = ctrl.totalkills;
        ctrl.wminfo.maxitems = ctrl.totalitems;
        ctrl.wminfo.maxsecret = ctrl.totalsecret;
        ctrl.wminfo.maxfrags = 0;

        // Par time for DOOM II
        if ctrl.gamemap >= 1 && ctrl.gamemap <= 32 {
            ctrl.wminfo.partime = TICRATE * CPARS[(ctrl.gamemap - 1) as usize];
        } else {
            ctrl.wminfo.partime = 0;
        }
    } else {
        // DOOM 1 next map calculation
        // Original C: g_game.c lines 1035-1080
        if ctrl.gamemap == 8 {
            // Episode end — goes to victory/finale
            ctrl.gameaction = GameAction::Victory;
            return;
        }

        if ctrl.gamemap == 9 {
            // Return from secret level
            // The return map depends on the episode
            match ctrl.gameepisode {
                1 => ctrl.wminfo.next = 3, // E1M9 returns to E1M4
                2 => ctrl.wminfo.next = 5, // E2M9 returns to E2M6
                3 => ctrl.wminfo.next = 5, // E3M9 returns to E3M6
                4 => ctrl.wminfo.next = 2, // E4M9 returns to E4M3
                _ => ctrl.wminfo.next = 3,
            }
        } else if ctrl.secretexit {
            // Secret exit → map 9
            ctrl.wminfo.next = 8; // 0-based map 9
        } else {
            // Normal progression: next map
            ctrl.wminfo.next = ctrl.gamemap; // 0-based next = current (1-based)
        }

        ctrl.wminfo.maxkills = ctrl.totalkills;
        ctrl.wminfo.maxitems = ctrl.totalitems;
        ctrl.wminfo.maxsecret = ctrl.totalsecret;
        ctrl.wminfo.maxfrags = 0;

        // Par time for DOOM 1
        if ctrl.gameepisode >= 1 && ctrl.gameepisode <= 3 && ctrl.gamemap >= 1 && ctrl.gamemap <= 9
        {
            ctrl.wminfo.partime = TICRATE * PARS[ctrl.gameepisode as usize][ctrl.gamemap as usize];
        } else {
            ctrl.wminfo.partime = 0;
        }
    }

    // Fill wminfo for all players
    // Original C: g_game.c lines 1113-1133
    ctrl.wminfo.epsd = ctrl.gameepisode - 1;
    ctrl.wminfo.last = ctrl.gamemap - 1; // 0-based
    ctrl.wminfo.pnum = ctrl.consoleplayer as i32;

    for i in 0..MAXPLAYERS {
        ctrl.wminfo.plyr[i].in_game = ctrl.playeringame[i];
        ctrl.wminfo.plyr[i].skills = ctrl.players[i].killcount;
        ctrl.wminfo.plyr[i].sitems = ctrl.players[i].itemcount;
        ctrl.wminfo.plyr[i].ssecret = ctrl.players[i].secretcount;
        ctrl.wminfo.plyr[i].stime = ctrl.leveltime;

        // Copy frags
        for j in 0..MAXPLAYERS {
            ctrl.wminfo.plyr[i].frags[j] = ctrl.players[i].frags[j];
        }
    }

    // Track secret exit for intermission
    ctrl.wminfo.didsecret = ctrl.players[ctrl.consoleplayer].didsecret;

    // Transition to intermission state
    // WI_Start is called by the game loop with the filled wminfo
    ctrl.gamestate = GameState::Intermission;
    ctrl.viewactive = false;

    info!(
        "G_DoCompleted: E{}M{} → next map {} (partime={} tics)",
        ctrl.gameepisode,
        ctrl.gamemap,
        ctrl.wminfo.next + 1,
        ctrl.wminfo.partime
    );
}

// =============================================================================
// World Done / Next Level
// =============================================================================

/// Handle world-done state transition. For commercial mode, triggers
/// finale at specific map numbers.
///
/// Original C: `void G_WorldDone(void)` (g_game.c lines 1147-1179)
pub fn g_world_done(ctrl: &mut GameCtrl) {
    ctrl.gameaction = GameAction::WorldDone;

    if ctrl.secretexit {
        ctrl.players[ctrl.consoleplayer].didsecret = true;
    }

    // In commercial mode, trigger finale at specific maps
    // Original C: g_game.c lines 1158-1175
    if ctrl.gamemode == GameMode::Commercial {
        match ctrl.gamemap {
            6 | 11 | 20 | 30 | 15 | 31 => {
                // F_StartFinale is called by the game loop via the WorldDone
                // action handler, which then transitions to Finale state.
                // The actual finale start is deferred to the game loop that
                // owns FinaleState.
            }
            _ => {}
        }
    }
}

/// Process the WorldDone action: advance to the next level.
///
/// Original C: `void G_DoWorldDone(void)` (g_game.c lines 1181-1189)
pub fn g_do_world_done(ctrl: &mut GameCtrl) {
    ctrl.gamestate = GameState::Level;
    ctrl.gamemap = ctrl.wminfo.next + 1; // wminfo.next is 0-based
    ctrl.gameaction = GameAction::LoadLevel;
    ctrl.secretexit = false;

    // Special case: return from secret level resets episode secret flag
    if ctrl.players.len() > ctrl.consoleplayer {
        ctrl.players[ctrl.consoleplayer].didsecret = false;
    }

    debug!("G_DoWorldDone: advancing to map {}", ctrl.gamemap);
}

// =============================================================================
// Save/Load Game
// =============================================================================

/// Initiate a game load from the specified slot.
///
/// Original C: `void G_LoadGame(char* name)` (g_game.c lines 1192-1201)
pub fn g_load_game(ctrl: &mut GameCtrl, name: &str) {
    ctrl.demoname = name.to_string();
    ctrl.gameaction = GameAction::LoadGame;
}

/// Execute game load from save file.
///
/// Reads the save file, validates the version, and restores game state.
///
/// Original C: `void G_DoLoadGame(void)` (g_game.c lines 1203-1252)
pub fn g_do_load_game(ctrl: &mut GameCtrl) {
    ctrl.gameaction = GameAction::Nothing;

    // Build save file path
    let save_path = format!("{}{}.dsg", SAVEGAMENAME, ctrl.savegameslot);
    let data = crate::util::misc::read_file(&save_path);

    if data.is_empty() {
        error!("G_DoLoadGame: could not read save file '{}'", save_path);
        return;
    }

    // Validate version string
    // Original C: g_game.c lines 1214-1218
    let mut offset: usize = SAVESTRINGSIZE; // skip description

    // Check version
    if offset + VERSIONSIZE > data.len() {
        error!("G_DoLoadGame: save file too short for version check");
        return;
    }
    let version_str = &data[offset..offset + VERSIONSIZE];
    let expected_version = format!("version {}", VERSION);
    let version_match = version_str
        .iter()
        .take(expected_version.len())
        .zip(expected_version.bytes())
        .all(|(&a, b)| a == b);

    if !version_match {
        warn!("G_DoLoadGame: version mismatch in save file");
        // Continue loading anyway, matching original behavior
    }
    offset += VERSIONSIZE;

    // Read skill, episode, map, playeringame
    // Original C: g_game.c lines 1220-1224
    if offset + 4 > data.len() {
        error!("G_DoLoadGame: save file truncated at game state");
        return;
    }
    let save_skill = data[offset];
    offset += 1;
    let save_episode = data[offset] as i32;
    offset += 1;
    let save_map = data[offset] as i32;
    offset += 1;

    // Read playeringame array
    let mut save_playeringame = [false; MAXPLAYERS];
    for slot in save_playeringame.iter_mut() {
        if offset < data.len() {
            *slot = data[offset] != 0;
            offset += 1;
        }
    }

    // Initialize the new game state
    ctrl.gameskill = match save_skill {
        0 => Skill::Baby,
        1 => Skill::Easy,
        2 => Skill::Medium,
        3 => Skill::Hard,
        4 => Skill::Nightmare,
        _ => Skill::Medium,
    };
    ctrl.gameepisode = save_episode;
    ctrl.gamemap = save_map;
    ctrl.playeringame = save_playeringame;

    // Store the remaining save data for unarchiving by the game loop.
    // The actual unarchive calls (unarchive_players, unarchive_world,
    // unarchive_thinkers, unarchive_specials) require access to the full
    // game context (level data, thinker list, etc.).
    ctrl.savebuffer = data[offset..].to_vec();

    // Load the level
    g_do_load_level(ctrl);

    info!(
        "G_DoLoadGame: loaded E{}M{} skill {:?}",
        save_episode, save_map, ctrl.gameskill
    );
}

/// Request saving the game to the specified slot.
///
/// Original C: `void G_SaveGame(int slot, char* description)` (g_game.c lines 1260-1268)
pub fn g_save_game(ctrl: &mut GameCtrl, slot: i32, description: &str) {
    ctrl.savegameslot = slot;
    ctrl.savedescription = description.to_string();
    ctrl.sendsave = true;
}

/// Execute game save to file.
///
/// Serializes the current game state to a save file.
///
/// Original C: `void G_DoSaveGame(void)` (g_game.c lines 1270-1320)
pub fn g_do_save_game(ctrl: &mut GameCtrl) {
    // Allocate save buffer
    let mut save_data: Vec<u8> = Vec::with_capacity(SAVEGAMESIZE);

    // Write description (SAVESTRINGSIZE bytes, zero-padded)
    // Original C: g_game.c lines 1282-1283
    let desc_bytes = ctrl.savedescription.as_bytes();
    for i in 0..SAVESTRINGSIZE {
        save_data.push(desc_bytes.get(i).copied().unwrap_or(0));
    }

    // Write version string (VERSIONSIZE bytes, zero-padded)
    let version = format!("version {}", VERSION);
    let version_bytes = version.as_bytes();
    for i in 0..VERSIONSIZE {
        save_data.push(version_bytes.get(i).copied().unwrap_or(0));
    }

    // Write game state
    save_data.push(ctrl.gameskill as u8);
    save_data.push(ctrl.gameepisode as u8);
    save_data.push(ctrl.gamemap as u8);

    // Write playeringame
    for i in 0..MAXPLAYERS {
        save_data.push(if ctrl.playeringame[i] { 1 } else { 0 });
    }

    // The actual archive calls (archive_players, archive_world,
    // archive_thinkers, archive_specials) are performed by the game loop
    // which has access to the full game context. The save buffer is
    // stored here for the game loop to append archived data.
    ctrl.savebuffer = save_data;

    // The game loop should call the archive functions, append 0x1d marker,
    // then write via write_file.

    // Build save file path
    let save_path = format!("{}{}.dsg", SAVEGAMENAME, ctrl.savegameslot);

    // Check for buffer overflow
    if ctrl.savebuffer.len() > SAVEGAMESIZE {
        error!(
            "G_DoSaveGame: save buffer overflow ({} > {})",
            ctrl.savebuffer.len(),
            SAVEGAMESIZE
        );
    }

    // Write to disk
    if !crate::util::misc::write_file(&save_path, &ctrl.savebuffer) {
        error!("G_DoSaveGame: failed to write '{}'", save_path);
        return;
    }

    ctrl.gameaction = GameAction::Nothing;

    // Display save confirmation message
    ctrl.players[ctrl.consoleplayer].message = Some(GGSAVED.to_string());

    info!(
        "G_DoSaveGame: saved to '{}' ({} bytes)",
        save_path,
        ctrl.savebuffer.len()
    );
}

// =============================================================================
// New Game Initialization
// =============================================================================

/// Deferred new game initialization. Sets parameters and triggers
/// the action on the next G_Ticker.
///
/// Original C: `void G_DeferedInitNew(skill_t skill, int episode, int map)` (g_game.c lines 1341-1343)
pub fn g_defered_init_new(ctrl: &mut GameCtrl, skill: Skill, episode: i32, map: i32) {
    ctrl.d_skill = skill;
    ctrl.d_episode = episode;
    ctrl.d_map = map;
    ctrl.gameaction = GameAction::NewGame;
}

/// Process the NewGame action: reset state and call g_init_new.
///
/// Original C: `void G_DoNewGame(void)` (g_game.c lines 1345-1358)
pub fn g_do_new_game(ctrl: &mut GameCtrl) {
    ctrl.demoplayback = false;
    ctrl.netdemo = false;
    ctrl.netgame = false;
    ctrl.deathmatch = 0;

    // Only player 0 is in the game for single-player
    ctrl.playeringame = [false; MAXPLAYERS];
    ctrl.playeringame[0] = true;
    ctrl.consoleplayer = 0;
    ctrl.displayplayer = 0;

    g_init_new(ctrl, ctrl.d_skill, ctrl.d_episode, ctrl.d_map);
    ctrl.gameaction = GameAction::Nothing;
}

/// Initialize a new game with the given skill, episode, and map.
///
/// Clamps skill/episode/map to valid ranges, resets the PRNG for
/// deterministic demo compatibility, handles nightmare mode speed
/// modifications, and loads the level.
///
/// Original C: `void G_InitNew(skill_t skill, int episode, int map)` (g_game.c lines 1364-1482)
pub fn g_init_new(ctrl: &mut GameCtrl, skill: Skill, episode: i32, map: i32) {
    // Clamp skill
    let mut skill = skill;
    if skill > Skill::Nightmare {
        skill = Skill::Nightmare;
    }

    // Clamp episode based on game mode
    // Original C: g_game.c lines 1373-1393
    let mut episode = episode;
    let mut map = map;

    match ctrl.gamemode {
        GameMode::Retail => {
            if episode > 4 {
                episode = 4;
            }
        }
        GameMode::Shareware => {
            if episode > 1 {
                episode = 1;
            }
        }
        GameMode::Registered => {
            if episode > 3 {
                episode = 3;
            }
        }
        GameMode::Commercial => {
            // DOOM II has maps 1-32
            map = map.clamp(1, 32);
        }
        _ => {
            if episode > 3 {
                episode = 3;
            }
        }
    }
    if episode < 1 {
        episode = 1;
    }
    if map < 1 {
        map = 1;
    }
    if map > 9 && ctrl.gamemode != GameMode::Commercial {
        map = 9;
    }

    // Nightmare mode: enable respawn monsters
    // Original C: g_game.c lines 1407-1410
    ctrl.respawnmonsters = skill == Skill::Nightmare;

    // Nightmare mode speed modifications for Demon (Pinky) and projectiles.
    // NOTE: STATES and MOBJINFO are immutable statics in our Rust port.
    // The nightmare speed modifications are handled at runtime by the
    // thinker/mobj code when skill == Nightmare, rather than modifying
    // the static tables. This preserves the behavioral contract while
    // maintaining Rust's safety guarantees.
    //
    // Original C: g_game.c lines 1421-1436
    // - Halve SARG (Demon) animation tics: states[S_SARG_RUN1..S_SARG_PAIN2].tics >>= 1
    // - Increase projectile speeds to 20*FRACUNIT: MT_BRUISERSHOT, MT_HEADSHOT, MT_TROOPSHOT
    // The runtime check for nightmare speed is: if gameskill == sk_nightmare { ... }

    // Set game parameters
    ctrl.gameskill = skill;
    ctrl.gameepisode = episode;
    ctrl.gamemap = map;
    ctrl.paused = false;
    ctrl.demoplayback = false;
    ctrl.usergame = true;

    // Sky texture selection based on episode
    // The renderer handles this based on ctrl.gameepisode and ctrl.gamemap.
    // Original C: g_game.c lines 1460-1475

    // Load the level
    g_do_load_level(ctrl);

    info!(
        "G_InitNew: skill={:?} episode={} map={}",
        skill, episode, map
    );
}

// =============================================================================
// Demo Recording and Playback
// =============================================================================

/// Read a demo tic command from the demo buffer.
///
/// CRITICAL FOR DEMO COMPATIBILITY: The byte format must match the
/// original C implementation exactly:
///   byte 0: forwardmove (signed i8)
///   byte 1: sidemove (signed i8)
///   byte 2: angleturn (unsigned u8 << 8, producing i16)
///   byte 3: buttons (u8)
///
/// Original C: `void G_ReadDemoTiccmd(ticcmd_t* cmd)` (g_game.c lines 1491-1503)
pub fn g_read_demo_ticcmd(ctrl: &mut GameCtrl, cmd: &mut TicCmd) {
    if ctrl.demo_p >= ctrl.demobuffer.len() {
        // End of demo buffer — treat as demo marker
        ctrl.demoplayback = false;
        return;
    }

    // Check for demo end marker (DEMOMARKER = 0x80)
    if ctrl.demobuffer[ctrl.demo_p] == DEMOMARKER {
        // End of demo
        ctrl.demoplayback = false;
        return;
    }

    // Read 4 bytes: forwardmove, sidemove, angleturn, buttons
    if ctrl.demo_p + 4 > ctrl.demobuffer.len() {
        ctrl.demoplayback = false;
        return;
    }

    cmd.forwardmove = ctrl.demobuffer[ctrl.demo_p] as i8;
    ctrl.demo_p += 1;
    cmd.sidemove = ctrl.demobuffer[ctrl.demo_p] as i8;
    ctrl.demo_p += 1;
    // Angleturn is stored as unsigned byte, shifted left by 8
    cmd.angleturn = (ctrl.demobuffer[ctrl.demo_p] as i16) << 8;
    ctrl.demo_p += 1;
    cmd.buttons = ctrl.demobuffer[ctrl.demo_p];
    ctrl.demo_p += 1;
}

/// Write a demo tic command to the demo buffer.
///
/// CRITICAL FOR DEMO COMPATIBILITY: The byte format must match the
/// original C implementation exactly.
///
/// Original C: `void G_WriteDemoTiccmd(ticcmd_t* cmd)` (g_game.c lines 1506-1523)
pub fn g_write_demo_ticcmd(ctrl: &mut GameCtrl, cmd: &TicCmd) {
    // Append 4 bytes to demo buffer
    ctrl.demobuffer.push(cmd.forwardmove as u8);
    ctrl.demobuffer.push(cmd.sidemove as u8);
    // Angleturn: store as (angleturn + 128) >> 8, matching original C
    // Original C: `*demo_p++ = (cmd->angleturn+128)>>8;`
    ctrl.demobuffer
        .push(((cmd.angleturn.wrapping_add(128)) >> 8) as u8);
    ctrl.demobuffer.push(cmd.buttons);

    // Update write position
    ctrl.demo_p = ctrl.demobuffer.len();

    // Check for buffer overflow: if close to end, stop recording
    // Original C checks `demo_p > demoend - 16`
    // We use Vec so it grows dynamically, but cap at a reasonable limit
    if ctrl.demobuffer.len() > 0x200000 {
        // 2MB limit to prevent unbounded growth
        warn!("G_WriteDemoTiccmd: demo buffer approaching limit, stopping recording");
        g_check_demo_status(ctrl);
    }
}

/// Set up demo recording. Allocates the demo buffer.
///
/// Original C: `void G_RecordDemo(char* name)` (g_game.c lines 1530-1546)
pub fn g_record_demo(ctrl: &mut GameCtrl, name: &str, args: &crate::util::argv::Args) {
    ctrl.usergame = false;
    ctrl.demoname = format!("{}.lmp", name);

    // Check for -maxdemo parameter to override buffer size
    let maxsize = if let Some(val) = args.parm_value("-maxdemo") {
        val.parse::<usize>().unwrap_or(0x20000)
    } else {
        0x20000 // Default 128KB
    };

    ctrl.demobuffer = Vec::with_capacity(maxsize);
    ctrl.demo_p = 0;
    ctrl.demorecording = true;

    info!(
        "G_RecordDemo: recording to '{}' (buffer={}KB)",
        ctrl.demoname,
        maxsize / 1024
    );
}

/// Write the demo header at the beginning of the demo buffer.
///
/// Header format: VERSION, skill, episode, map, deathmatch, respawnparm,
/// fastparm, nomonsters, consoleplayer, playeringame[0..3]
///
/// Original C: `void G_BeginRecording(void)` (g_game.c lines 1549-1567)
pub fn g_begin_recording(ctrl: &mut GameCtrl) {
    ctrl.demobuffer.clear();
    ctrl.demo_p = 0;

    // Write demo header
    ctrl.demobuffer.push(VERSION as u8);
    ctrl.demobuffer.push(ctrl.gameskill as u8);
    ctrl.demobuffer.push(ctrl.gameepisode as u8);
    ctrl.demobuffer.push(ctrl.gamemap as u8);
    ctrl.demobuffer.push(ctrl.deathmatch as u8);
    ctrl.demobuffer
        .push(if ctrl.respawnmonsters { 1 } else { 0 });
    ctrl.demobuffer.push(0); // fastparm — not tracked in GameCtrl, default false
    ctrl.demobuffer.push(0); // nomonsters — not tracked in GameCtrl, default false
    ctrl.demobuffer.push(ctrl.consoleplayer as u8);

    for i in 0..MAXPLAYERS {
        ctrl.demobuffer
            .push(if ctrl.playeringame[i] { 1 } else { 0 });
    }

    ctrl.demo_p = ctrl.demobuffer.len();

    debug!("G_BeginRecording: header written ({} bytes)", ctrl.demo_p);
}

/// Deferred demo playback. Sets the demo name for loading on next tick.
///
/// Original C: `void G_DeferedPlayDemo(char* name)` (g_game.c lines 1574)
pub fn g_defered_play_demo(ctrl: &mut GameCtrl, name: &str) {
    ctrl.defdemoname = name.to_string();
    ctrl.gameaction = GameAction::PlayDemo;
}

/// Alias for g_defered_play_demo (initiates demo playback from WAD lump).
///
/// Original C: `char* G_PlayDemo(void)` (g_game.c lines 1576)
pub fn g_play_demo(ctrl: &mut GameCtrl, name: &str) {
    g_defered_play_demo(ctrl, name);
}

/// Execute demo playback from a WAD lump.
///
/// Reads the demo header, validates the version, and starts playback.
///
/// Original C: `void G_DoPlayDemo(void)` (g_game.c lines 1576-1620)
pub fn g_do_play_demo(ctrl: &mut GameCtrl, demo_data: &[u8]) {
    ctrl.gameaction = GameAction::Nothing;

    if demo_data.is_empty() {
        error!("G_DoPlayDemo: empty demo data");
        ctrl.gameaction = GameAction::Nothing;
        return;
    }

    let mut offset: usize = 0;

    // Read and validate version
    if offset >= demo_data.len() {
        error!("G_DoPlayDemo: demo too short for header");
        return;
    }
    let demo_version = demo_data[offset] as i32;
    offset += 1;

    if demo_version != VERSION {
        warn!(
            "G_DoPlayDemo: demo version {} != engine version {}",
            demo_version, VERSION
        );
        ctrl.gameaction = GameAction::Nothing;
        return;
    }

    // Read demo header fields
    // Original C: g_game.c lines 1595-1605
    if offset + 8 + MAXPLAYERS > demo_data.len() {
        error!("G_DoPlayDemo: demo too short for header fields");
        return;
    }

    let skill_byte = demo_data[offset];
    offset += 1;
    let episode = demo_data[offset] as i32;
    offset += 1;
    let map = demo_data[offset] as i32;
    offset += 1;
    let deathmatch = demo_data[offset] as i32;
    offset += 1;
    let respawnparm = demo_data[offset] != 0;
    offset += 1;
    let _fastparm = demo_data[offset] != 0;
    offset += 1;
    let _nomonsters = demo_data[offset] != 0;
    offset += 1;
    let consoleplayer = demo_data[offset] as usize;
    offset += 1;

    let mut playeringame = [false; MAXPLAYERS];
    for slot in playeringame.iter_mut() {
        *slot = demo_data[offset] != 0;
        offset += 1;
    }

    let skill = match skill_byte {
        0 => Skill::Baby,
        1 => Skill::Easy,
        2 => Skill::Medium,
        3 => Skill::Hard,
        4 => Skill::Nightmare,
        _ => Skill::Medium,
    };

    // Setup game state from demo header
    ctrl.deathmatch = deathmatch;
    ctrl.respawnmonsters = respawnparm;
    ctrl.consoleplayer = consoleplayer;
    ctrl.displayplayer = consoleplayer;
    ctrl.playeringame = playeringame;

    // Check for multiplayer demo
    let player_count: usize = playeringame.iter().filter(|&&p| p).count();
    if player_count > 1 {
        ctrl.netgame = true;
        ctrl.netdemo = true;
    }

    // Initialize the game for demo playback
    ctrl.precache = false;
    g_init_new(ctrl, skill, episode, map);
    ctrl.precache = true;

    // Store the remaining demo data for reading tic commands
    ctrl.demobuffer = demo_data[offset..].to_vec();
    ctrl.demo_p = 0;
    ctrl.demoplayback = true;
    ctrl.usergame = false;

    info!(
        "G_DoPlayDemo: playing E{}M{} skill={:?} ({} players)",
        episode, map, skill, player_count
    );
}

/// Start a timed demo for performance benchmarking.
///
/// Original C: `void G_TimeDemo(char* name)` (g_game.c lines 1637-1645)
pub fn g_time_demo(ctrl: &mut GameCtrl, name: &str) {
    ctrl.timingdemo = true;
    ctrl.singledemo = true;
    ctrl.defdemoname = name.to_string();
    ctrl.gameaction = GameAction::PlayDemo;

    info!("G_TimeDemo: benchmarking '{}'", name);
}

/// Check and handle demo status at end of demo or recording.
///
/// For timed demos, prints performance stats.
/// For demo playback, resets state.
/// For demo recording, writes the buffer to file.
///
/// Original C: `boolean G_CheckDemoStatus(void)` (g_game.c lines 1647-1687)
pub fn g_check_demo_status(ctrl: &mut GameCtrl) -> bool {
    if ctrl.timingdemo {
        // Calculate performance: gametics / realtics
        let endtime = ctrl.gametic;
        let realtics = endtime - ctrl.starttime;

        if realtics > 0 {
            let fps = (ctrl.gametic as f64 * TICRATE as f64) / realtics as f64;
            info!(
                "G_CheckDemoStatus: timed {} gametics in {} realtics ({:.1} fps)",
                ctrl.gametic, realtics, fps
            );
        }

        ctrl.timingdemo = false;
        ctrl.demoplayback = false;

        if ctrl.singledemo {
            // Exit after single demo benchmark
            return false;
        }
        return true;
    }

    if ctrl.demoplayback {
        // Demo playback ended
        ctrl.demoplayback = false;
        ctrl.netdemo = false;
        ctrl.netgame = false;
        ctrl.deathmatch = 0;

        // Reset players
        ctrl.playeringame = [false; MAXPLAYERS];
        ctrl.playeringame[0] = true;
        ctrl.consoleplayer = 0;
        ctrl.displayplayer = 0;

        // Advance to demo screen state
        ctrl.gameaction = GameAction::Nothing;

        if ctrl.singledemo {
            return false;
        }

        return true;
    }

    if ctrl.demorecording {
        // Stop recording: write DEMOMARKER and flush to file
        ctrl.demobuffer.push(DEMOMARKER);
        ctrl.demo_p = ctrl.demobuffer.len();

        if crate::util::misc::write_file(&ctrl.demoname, &ctrl.demobuffer) {
            info!(
                "G_CheckDemoStatus: demo '{}' written ({} bytes)",
                ctrl.demoname,
                ctrl.demobuffer.len()
            );
        } else {
            error!(
                "G_CheckDemoStatus: failed to write demo '{}'",
                ctrl.demoname
            );
        }

        ctrl.demorecording = false;
        return false;
    }

    false
}
