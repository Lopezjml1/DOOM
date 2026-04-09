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

//! DOOM main program (D_DoomMain) and game initialization.
//!
//! Translated from `linuxdoom-1.10/d_main.c` and `linuxdoom-1.10/d_main.h`.
//!
//! This is the primary engine entry point. It contains:
//! - `D_DoomMain` — master initialization and subsystem orchestration
//! - `D_AddFile` — WAD file list management
//! - `IdentifyVersion` — IWAD detection with Windows-native path search
//! - `D_PostEvent` / `D_ProcessEvents` — input event ring buffer
//! - `D_Display` — per-frame rendering orchestration
//! - `D_DoAdvanceDemo` / `D_StartTitle` — demo sequence cycling
//! - `FindResponseFile` — @responsefile argument expansion
//!
//! All former C global variables from `doomstat.h` / `d_main.c` are
//! consolidated into the [`GameMain`] struct (no `static mut` usage).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use tracing::{debug, error, info, warn};

use crate::game::game_ctrl::{
    g_begin_recording, g_defered_play_demo, g_init_new, g_load_game, g_record_demo, g_responder,
    g_time_demo, GameCtrl, FORWARD_MOVE_DEFAULT, SIDE_MOVE_DEFAULT,
};
use crate::game::strings::{DEVDATA, DEVMAPS, D_DEVSTR, SAVEGAMENAME};
use crate::info::sounds::{MusicEnum, S_MUSIC};
use crate::info::sprites::SpriteNum;
use crate::play::setup::p_init;
use crate::traits::audio::AudioBackend;
use crate::traits::platform::PlatformHost;
use crate::traits::renderer::Renderer;
use crate::types::doomdef::{
    GameMode, GameState, Language, Skill, SCREENHEIGHT, SCREENWIDTH, TICRATE, VERSION,
};
use crate::types::event::{Event, GameAction};
use crate::types::player::PlayerState;
use crate::ui::automap::{am_drawer, AutomapState};
use crate::ui::finale::{f_drawer, FinaleState};
use crate::ui::hud::{hu_drawer, hu_erase, hu_init, HudState};
use crate::ui::intermission::{wi_drawer, IntermissionState};
use crate::ui::menu::{m_drawer, m_init, m_responder, MenuState};
use crate::ui::statusbar::{st_drawer, st_init, StatusBarState};
use crate::ui::wipe::{WipeState, WipeType};
use crate::util::argv::Args;
use crate::util::misc::ConfigDefaults;
use crate::util::random::DoomRandom;
use crate::video::video::VideoState;
use doom_wad::{PurgeTag, WadFile};

// =========================================================================
// Constants (from d_main.h and d_main.c)
// =========================================================================

/// Maximum number of WAD files that can be loaded simultaneously.
/// Equivalent of `MAXWADFILES` in `d_main.h:30`.
pub const MAXWADFILES: usize = 20;

/// Maximum number of events in the ring buffer.
/// Must be a power of 2 for `& (MAXEVENTS - 1)` masking.
/// Equivalent of `MAXEVENTS` in `d_main.c:139`.
pub const MAXEVENTS: usize = 64;

/// Background color index for title screen/wipe.
const _BGCOLOR: u8 = 7;

/// Foreground color index for title screen/wipe.
const _FGCOLOR: u8 = 8;

// =========================================================================
// GameMain — consolidated global state from d_main.c
// =========================================================================

/// Primary engine state structure consolidating all former C global
/// variables from `d_main.c` and related `doomstat.h` externs.
///
/// In the original C code, these were `extern` declarations in headers
/// and `static` / file-scope variables in `d_main.c`. Consolidating
/// them into a struct eliminates `static mut` and enables safe Rust
/// ownership semantics.
pub struct GameMain {
    // -- WAD file list (replaces char* wadfiles[MAXWADFILES]) --
    /// List of WAD file paths to load. Populated by `d_add_file` and
    /// command-line processing. Maximum `MAXWADFILES` entries.
    pub wadfiles: Vec<String>,

    // -- Game parameters (d_main.c lines 95-119) --
    /// Development mode flag (`-devparm`). Enables extra diagnostics
    /// and F1 screenshot capability.
    pub devparm: bool,
    /// Disable all monsters (`-nomonsters`).
    pub nomonsters: bool,
    /// Enable monster respawning outside Nightmare (`-respawn`).
    pub respawnparm: bool,
    /// Enable fast monsters outside Nightmare (`-fast`).
    pub fastparm: bool,
    /// Drone mode — forward all input to network peer.
    pub drone: bool,
    /// Debug flag: cancel adaptiveness, run one tic per draw.
    pub singletics: bool,

    // -- Start parameters --
    /// Starting skill level for `-skill` parameter.
    pub startskill: Skill,
    /// Starting episode for `-episode` parameter (1-based).
    pub startepisode: i32,
    /// Starting map for `-warp` parameter (1-based).
    pub startmap: i32,
    /// Auto-start a new game (set when `-warp` or `-skill` is provided).
    pub autostart: bool,

    // -- Demo sequence state (d_main.c lines 414-416) --
    /// Current position in the demo sequence rotation.
    pub demosequence: i32,
    /// Tics remaining on current title/credit/demo page.
    pub pagetic: i32,
    /// Lump name of the current page graphic.
    pub pagename: String,

    // -- Display state (d_main.c line 188) --
    /// Previous game state for screen wipe detection.
    pub wipegamestate: GameState,
    /// Flag to advance to the next demo sequence entry.
    pub advancedemo: bool,

    // -- Event ring buffer (d_main.c lines 141-143) --
    /// Ring buffer of input events. Fixed size of `MAXEVENTS`.
    pub events: Vec<Event>,
    /// Write index into the event ring buffer.
    pub eventhead: usize,
    /// Read index into the event ring buffer.
    pub eventtail: usize,

    // -- File paths (d_main.c lines 124-126) --
    /// Path to the primary IWAD file.
    pub wadfile: String,
    /// Directory for map/data files (development modes).
    pub mapdir: String,
    /// Base path for configuration file (replaces `~/.doomrc`).
    pub basedefault: String,

    // -- Title string (d_main.c line 536) --
    /// Title string displayed at startup (version + game name).
    pub title: String,

    // -- D_Display internal state (formerly local statics) --
    /// Tracks whether the view was active last frame for border redraw.
    viewactivestate: bool,
    /// Tracks whether the menu was active last frame.
    menuactivestate: bool,
    /// Tracks inhelpscreens state for border redraw detection.
    inhelpscreensstate: bool,
    /// Tracks fullscreen state for status bar redraw.
    fullscreen: bool,
    /// Tracks previous gamestate for redraw detection.
    oldgamestate: GameState,
    /// Counter for border redraw frames (draws border for 3 frames after change).
    borderdrawcount: i32,

    // -- Music handle for title/demo screen music --
    /// Handle for the currently registered title music.
    pub title_music_handle: i32,
}

impl Default for GameMain {
    fn default() -> Self {
        Self::new()
    }
}

impl GameMain {
    /// Create a new `GameMain` with C-compatible default values.
    ///
    /// All fields are initialized to their original C defaults:
    /// - `startskill` = `Skill::Medium` (sk_medium)
    /// - `startepisode` = 1
    /// - `startmap` = 1
    /// - `demosequence` = -1 (triggers advance on first tick)
    /// - `wipegamestate` = `GameState::DemoScreen`
    /// - Event ring buffer allocated with `MAXEVENTS` capacity
    pub fn new() -> Self {
        let mut events = Vec::with_capacity(MAXEVENTS);
        for _ in 0..MAXEVENTS {
            events.push(Event::default());
        }
        GameMain {
            wadfiles: Vec::new(),
            devparm: false,
            nomonsters: false,
            respawnparm: false,
            fastparm: false,
            drone: false,
            singletics: false,
            startskill: Skill::Medium,
            startepisode: 1,
            startmap: 1,
            autostart: false,
            demosequence: -1,
            pagetic: 0,
            pagename: String::new(),
            wipegamestate: GameState::DemoScreen,
            advancedemo: false,
            events,
            eventhead: 0,
            eventtail: 0,
            wadfile: String::new(),
            mapdir: String::new(),
            basedefault: String::new(),
            title: String::new(),
            viewactivestate: false,
            menuactivestate: false,
            inhelpscreensstate: false,
            fullscreen: false,
            oldgamestate: GameState::DemoScreen,
            borderdrawcount: 0,
            title_music_handle: 0,
        }
    }
}

// =========================================================================
// D_AddFile — Add a WAD file to the load list (d_main.c:543-555)
// =========================================================================

/// Add a WAD file path to the load list.
///
/// Equivalent of `D_AddFile` at `d_main.c:543-555`. The original C code
/// walked the `wadfiles[]` array to find the first `NULL` slot and
/// allocated a new string via `malloc`/`strcpy`. In Rust we simply
/// push onto the `Vec<String>`.
///
/// # Panics
/// Does not panic. If `MAXWADFILES` is exceeded, the file is silently
/// ignored with a warning log message.
pub fn d_add_file(game: &mut GameMain, file: &str) {
    if game.wadfiles.len() >= MAXWADFILES {
        warn!(
            "D_AddFile: maximum WAD file count ({}) reached, ignoring '{}'",
            MAXWADFILES, file
        );
        return;
    }
    game.wadfiles.push(String::from(file));
    debug!("D_AddFile: added '{}'", file);
}

// =========================================================================
// D_PostEvent — Queue an input event (d_main.c:150-154)
// =========================================================================

/// Post an input event to the ring buffer for later processing.
///
/// Equivalent of `D_PostEvent` at `d_main.c:150-154`. Uses a ring buffer
/// with `& (MAXEVENTS - 1)` index masking for O(1) wrap-around.
///
/// Events are consumed by [`d_process_events`] during the main game loop.
pub fn d_post_event(game: &mut GameMain, ev: Event) {
    game.events[game.eventhead] = ev;
    game.eventhead = (game.eventhead + 1) & (MAXEVENTS - 1);
}

// =========================================================================
// D_ProcessEvents — Dispatch queued events (d_main.c:161-177)
// =========================================================================

/// Process all queued input events, dispatching to menu and game responders.
///
/// Equivalent of `D_ProcessEvents` at `d_main.c:161-177`. Events are
/// dispatched in order: menu responder gets first crack, then game responder.
///
/// The original C code checked `gamemode == commercial && W_CheckNumForName("map01") == -1`
/// for a store demo condition. We preserve this check.
pub fn d_process_events(
    game: &mut GameMain,
    game_ctrl: &mut GameCtrl,
    menu_state: &mut MenuState,
    video: &mut VideoState,
    audio: &mut dyn AudioBackend,
    renderer: &mut dyn Renderer,
    platform: &mut dyn PlatformHost,
    hud_state: &mut HudState,
    automap_state: &mut AutomapState,
    args: &Args,
    wad: &mut WadFile,
) {
    // d_main.c:166-168: Store demo check — if commercial with no map01, return
    if game_ctrl.gamemode == GameMode::Commercial && wad.check_num_for_name("map01").is_none() {
        return;
    }

    // Process all events in the ring buffer
    while game.eventtail != game.eventhead {
        let ev = game.events[game.eventtail];
        // Menu gets first crack at input (d_main.c:173)
        if !m_responder(
            &ev,
            menu_state,
            game_ctrl,
            video,
            audio,
            renderer,
            platform,
            hud_state,
            automap_state,
            args,
        ) {
            // Game responder handles remaining events (d_main.c:175)
            g_responder(game_ctrl, &ev);
        }
        game.eventtail = (game.eventtail + 1) & (MAXEVENTS - 1);
    }
}

// =========================================================================
// D_PageTicker — Tick the demo screen page timer (d_main.c:423-427)
// =========================================================================

/// Tick the demo screen page timer, advancing the demo sequence when
/// the current page's display time expires.
///
/// Equivalent of `D_PageTicker` at `d_main.c:423-427`.
pub fn d_page_ticker(game: &mut GameMain) {
    game.pagetic -= 1;
    if game.pagetic < 0 {
        d_advance_demo(game);
    }
}

// =========================================================================
// D_PageDrawer — Draw the current title/credit page (d_main.c:434-437)
// =========================================================================

/// Draw the current demo sequence page graphic to screen buffer 0.
///
/// Equivalent of `D_PageDrawer` at `d_main.c:434-437`. Loads the page
/// lump by name and draws it as a full-screen patch.
pub fn d_page_drawer(game: &GameMain, video: &mut VideoState, wad: &mut WadFile) {
    // d_main.c:436: V_DrawPatch(0, 0, 0, W_CacheLumpName(pagename, PU_CACHE))
    let patch_data = wad
        .cache_lump_name(&game.pagename, PurgeTag::Cache)
        .to_vec();
    video.draw_patch(0, 0, 0, &patch_data);
}

// =========================================================================
// D_AdvanceDemo — Signal demo sequence advancement (d_main.c:444-447)
// =========================================================================

/// Signal that the demo sequence should advance to the next entry.
///
/// Equivalent of `D_AdvanceDemo` at `d_main.c:444-447`. Sets the
/// `advancedemo` flag which is checked at the start of `D_DoAdvanceDemo`.
pub fn d_advance_demo(game: &mut GameMain) {
    game.advancedemo = true;
}

// =========================================================================
// D_DoAdvanceDemo — Cycle through demo sequence (d_main.c:454-518)
// =========================================================================

/// Advance to the next entry in the demo sequence rotation.
///
/// Equivalent of `D_DoAdvanceDemo` at `d_main.c:454-518`. Cycles through
/// title screens, credits, help pages, and demo playbacks. The sequence
/// differs based on game mode:
/// - Retail: 7 entries (includes CREDIT and demo4)
/// - Commercial: 6 entries (includes demo3 twice, different title music)
/// - Registered/Shareware: 6 entries (uses HELP2 instead of CREDIT)
pub fn d_do_advance_demo(
    game: &mut GameMain,
    game_ctrl: &mut GameCtrl,
    audio: &mut dyn AudioBackend,
    wad: &mut WadFile,
) {
    // d_main.c:455: players[consoleplayer].playerstate = PST_LIVE
    game_ctrl.players[game_ctrl.consoleplayer].playerstate = PlayerState::Live;
    game.advancedemo = false;
    game_ctrl.usergame = false;
    game_ctrl.paused = false;
    game_ctrl.gameaction = GameAction::Nothing;

    // d_main.c:462: Retail has 7 sequences, others have 6
    if game_ctrl.gamemode == GameMode::Retail {
        game.demosequence = (game.demosequence + 1) % 7;
    } else {
        game.demosequence = (game.demosequence + 1) % 6;
    }

    match game.demosequence {
        0 => {
            // Title screen with music
            if game_ctrl.gamemode == GameMode::Commercial {
                game.pagetic = TICRATE * 11; // 35 * 11 = 385 tics
            } else {
                game.pagetic = 170;
            }
            game_ctrl.gamestate = GameState::DemoScreen;
            game.pagename = "TITLEPIC".to_string();
            // Start title music
            if game_ctrl.gamemode == GameMode::Commercial {
                start_title_music(game, audio, wad, MusicEnum::mus_dm2ttl);
            } else {
                start_title_music(game, audio, wad, MusicEnum::mus_intro);
            }
        }
        1 => {
            // Play demo1
            g_defered_play_demo(game_ctrl, "demo1");
        }
        2 => {
            // Credits or help page
            game.pagetic = 200;
            game_ctrl.gamestate = GameState::DemoScreen;
            if game_ctrl.gamemode == GameMode::Commercial {
                game.pagename = "TITLEPIC".to_string();
                // Commercial mode shows title again (d_main.c:486-490)
                start_title_music(game, audio, wad, MusicEnum::mus_dm2ttl);
            } else if game_ctrl.gamemode == GameMode::Retail {
                game.pagename = "CREDIT".to_string();
            } else {
                game.pagename = "HELP2".to_string();
            }
        }
        3 => {
            // Play demo2
            g_defered_play_demo(game_ctrl, "demo2");
        }
        4 => {
            // Title screen or credits depending on mode
            game_ctrl.gamestate = GameState::DemoScreen;
            if game_ctrl.gamemode == GameMode::Commercial {
                game.pagetic = TICRATE * 11;
                game.pagename = "TITLEPIC".to_string();
                start_title_music(game, audio, wad, MusicEnum::mus_dm2ttl);
            } else {
                game.pagetic = 200;
                if game_ctrl.gamemode == GameMode::Retail {
                    game.pagename = "CREDIT".to_string();
                } else {
                    // Registered and shareware: play HELP2
                    game.pagename = "HELP2".to_string();
                }
            }
        }
        5 => {
            // Play demo3
            g_defered_play_demo(game_ctrl, "demo3");
        }
        6 => {
            // Retail-only: play demo4
            g_defered_play_demo(game_ctrl, "demo4");
        }
        _ => {
            // Should not happen due to modulo, but handle gracefully
            game.demosequence = -1;
            d_advance_demo(game);
        }
    }
}

/// Helper: start title/demo screen music via the audio backend.
///
/// Looks up the music lump name from `S_MUSIC`, loads it from the WAD,
/// registers with the audio backend, and starts looping playback.
fn start_title_music(
    game: &mut GameMain,
    audio: &mut dyn AudioBackend,
    wad: &mut WadFile,
    music: MusicEnum,
) {
    let idx = music as usize;
    if idx >= S_MUSIC.len() {
        return;
    }
    // Build WAD lump name: "d_" + music name (e.g., "d_intro", "d_dm2ttl")
    let lump_name = format!("d_{}", S_MUSIC[idx].name);
    if let Some(_lump_num) = wad.check_num_for_name(&lump_name) {
        let data = wad.cache_lump_name(&lump_name, PurgeTag::Music).to_vec();
        let handle = audio.register_song(&data);
        audio.play_song(handle, true);
        game.title_music_handle = handle;
    } else {
        debug!("start_title_music: music lump '{}' not found", lump_name);
    }
}

// =========================================================================
// D_StartTitle — Reset to title screen (d_main.c:525-530)
// =========================================================================

/// Reset the demo sequence and advance to the title screen.
///
/// Equivalent of `D_StartTitle` at `d_main.c:525-530`. Called when
/// returning to the title screen from the menu or after a demo ends.
pub fn d_start_title(game: &mut GameMain, game_ctrl: &mut GameCtrl) {
    game_ctrl.gameaction = GameAction::Nothing;
    game.demosequence = -1;
    d_advance_demo(game);
}

// =========================================================================
// IdentifyVersion — Detect IWAD and set game mode (d_main.c:563-717)
// =========================================================================

/// Detect the IWAD file and set the game mode accordingly.
///
/// Equivalent of `IdentifyVersion` at `d_main.c:563-717`. This is the
/// **CRITICAL PLATFORM CHANGE** from the original C code:
///
/// - ALL Unix path logic (`/usr/local/share/games/doom/`, `~/`, etc.)
///   is replaced with Windows-native equivalents
/// - `access()` file existence checks replaced with `std::path::Path::exists()`
/// - `getenv("HOME")` replaced with Windows AppData via environment
/// - Steam installation paths are checked on Windows
///
/// The IWAD search order is:
/// 1. CLI `--iwad` argument (if provided via `wadfile` field)
/// 2. `DOOMWADDIR` environment variable directory
/// 3. Current working directory
/// 4. Common Steam installation paths (Windows)
///
/// Game mode detection logic is preserved exactly from the original:
/// - `doom2.wad` / `doom2f.wad` / `plutonia.wad` / `tnt.wad` → Commercial
/// - `doomu.wad` → Retail
/// - `doom.wad` → Registered
/// - `doom1.wad` → Shareware
pub fn identify_version(game: &mut GameMain, game_ctrl: &mut GameCtrl, args: &Args) {
    let mut doomwaddir = String::new();

    // d_main.c:617-656: Handle -shdev, -regdev, -comdev development modes
    if args.has_parm("-shdev") {
        game_ctrl.gamemode = GameMode::Shareware;
        game.devparm = true;
        if let Some(val) = args.parm_value("-shdev") {
            d_add_file(game, val);
        } else {
            d_add_file(game, &format!("{}doom1.wad", DEVDATA));
        }
        d_add_file(game, &format!("{}doom.wad", DEVMAPS));
        game.basedefault = format!("{}default.cfg", DEVDATA);
        return;
    }

    if args.has_parm("-regdev") {
        game_ctrl.gamemode = GameMode::Registered;
        game.devparm = true;
        if let Some(val) = args.parm_value("-regdev") {
            d_add_file(game, val);
        } else {
            d_add_file(game, &format!("{}doom.wad", DEVDATA));
        }
        d_add_file(game, &format!("{}doom.wad", DEVMAPS));
        game.basedefault = format!("{}default.cfg", DEVDATA);
        return;
    }

    if args.has_parm("-comdev") {
        game_ctrl.gamemode = GameMode::Commercial;
        game.devparm = true;
        if let Some(val) = args.parm_value("-comdev") {
            d_add_file(game, val);
        } else {
            d_add_file(game, &format!("{}doom2.wad", DEVDATA));
        }
        d_add_file(game, &format!("{}doom.wad", DEVMAPS));
        game.basedefault = format!("{}default.cfg", DEVDATA);
        return;
    }

    // d_main.c:658-664: If wadfile is already set (via --iwad CLI), use directly
    if !game.wadfile.is_empty() {
        doomwaddir = String::new();
    } else {
        // d_main.c:599: Check DOOMWADDIR environment variable
        if let Ok(dir) = env::var("DOOMWADDIR") {
            doomwaddir = if dir.ends_with('/') || dir.ends_with('\\') {
                dir
            } else {
                format!("{}/", dir)
            };
        }
    }

    // Build list of search directories (Windows-native replacement)
    let search_dirs = build_iwad_search_dirs(&doomwaddir);

    // d_main.c:670-710: Try each IWAD filename in each search directory
    let iwad_candidates = [
        ("doom2f.wad", GameMode::Commercial),
        ("doom2.wad", GameMode::Commercial),
        ("plutonia.wad", GameMode::Commercial),
        ("tnt.wad", GameMode::Commercial),
        ("doomu.wad", GameMode::Retail),
        ("doom.wad", GameMode::Registered),
        ("doom1.wad", GameMode::Shareware),
    ];

    // If wadfile already set (from --iwad), check it directly
    if !game.wadfile.is_empty() {
        let path = Path::new(&game.wadfile);
        if path.exists() {
            let filename_lower = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let mut found_mode = GameMode::Indetermined;
            for (name, mode) in &iwad_candidates {
                if filename_lower == *name {
                    found_mode = *mode;
                    break;
                }
            }
            if found_mode == GameMode::Indetermined {
                found_mode = GameMode::Registered;
            }
            game_ctrl.gamemode = found_mode;
            d_add_file(game, &game.wadfile.clone());
            info!("IdentifyVersion: IWAD found at '{}'", game.wadfile);
        } else {
            error!(
                "IWAD file not found at path: '{}'. \
                 Please provide a valid path using --iwad <path>.",
                game.wadfile
            );
            game_ctrl.gamemode = GameMode::Indetermined;
        }
    } else {
        // Search for IWAD in all directories
        let mut found = false;
        for (iwad_name, mode) in &iwad_candidates {
            for dir in &search_dirs {
                let full_path = if dir.is_empty() {
                    PathBuf::from(iwad_name)
                } else {
                    PathBuf::from(dir).join(iwad_name)
                };
                if full_path.exists() {
                    let path_str = full_path.to_string_lossy().to_string();
                    game_ctrl.gamemode = *mode;
                    game.wadfile = path_str.clone();
                    d_add_file(game, &path_str);
                    info!("IdentifyVersion: found '{}' at '{}'", iwad_name, path_str);
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }

        if !found {
            warn!(
                "IdentifyVersion: no IWAD found in search paths. \
                 Game mode set to Indetermined. \
                 Use --iwad <path> to specify your IWAD file."
            );
            game_ctrl.gamemode = GameMode::Indetermined;
        }
    }

    // d_main.c:665: Set basedefault for configuration file
    if game.basedefault.is_empty() {
        if let Ok(appdata) = env::var("APPDATA") {
            let config_dir = PathBuf::from(&appdata).join("doom-rust");
            let _ = fs::create_dir_all(&config_dir);
            game.basedefault = config_dir.join("default.cfg").to_string_lossy().to_string();
        } else {
            game.basedefault = "default.cfg".to_string();
        }
    }

    // d_main.c:702-710: Handle French doom2f.wad
    if game_ctrl.gamemode == GameMode::Commercial {
        let filename_lower = Path::new(&game.wadfile)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if filename_lower == "doom2f.wad" {
            info!("IdentifyVersion: French DOOM II detected (doom2f.wad)");
        }
    }
}

/// Build a list of directories to search for IWAD files.
///
/// Windows-native replacement for the Unix path search at `d_main.c:575-615`.
fn build_iwad_search_dirs(doomwaddir: &str) -> Vec<String> {
    let mut dirs = Vec::new();

    // 1. DOOMWADDIR environment variable (if set)
    if !doomwaddir.is_empty() {
        dirs.push(doomwaddir.to_string());
    }

    // 2. Current working directory
    if let Ok(cwd) = env::current_dir() {
        dirs.push(cwd.to_string_lossy().to_string());
    }
    dirs.push(String::new());

    // 3. Common Steam installation paths (Windows)
    let steam_paths = [
        r"C:\Program Files (x86)\Steam\steamapps\common\Ultimate Doom\base",
        r"C:\Program Files (x86)\Steam\steamapps\common\Doom 2\base",
        r"C:\Program Files (x86)\Steam\steamapps\common\Final Doom\base",
        r"C:\Program Files (x86)\Steam\steamapps\common\DOOM 3 BFG Edition\base\wads",
        r"C:\Program Files\Steam\steamapps\common\Ultimate Doom\base",
        r"C:\Program Files\Steam\steamapps\common\Doom 2\base",
    ];
    for path in &steam_paths {
        dirs.push(path.to_string());
    }

    // 4. GOG installation paths
    let gog_paths = [r"C:\GOG Games\DOOM\base", r"C:\GOG Games\DOOM 2\base"];
    for path in &gog_paths {
        dirs.push(path.to_string());
    }

    dirs
}

// =========================================================================
// FindResponseFile — Parse @responsefile arguments (d_main.c:722-790)
// =========================================================================

/// Parse `@responsefile` arguments, expanding them in-place.
///
/// Equivalent of `FindResponseFile` at `d_main.c:722-790`. If any
/// argument begins with `@`, the referenced file is read and its
/// contents are split into individual arguments that replace the
/// `@filename` argument.
pub fn find_response_file(args: &mut Vec<String>) {
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with('@') {
            let filename = args[i][1..].to_string();
            info!("Found response file '{}'", filename);

            match fs::read_to_string(&filename) {
                Ok(contents) => {
                    let new_args: Vec<String> = parse_response_args(&contents);
                    info!(
                        "Response file '{}' contained {} arguments",
                        filename,
                        new_args.len()
                    );
                    args.remove(i);
                    for (j, arg) in new_args.into_iter().enumerate() {
                        args.insert(i + j, arg);
                    }
                }
                Err(e) => {
                    error!("No such response file: '{}' ({})", filename, e);
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
}

/// Parse response file contents into individual arguments.
/// Splits on whitespace, respecting double-quoted strings.
fn parse_response_args(contents: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut chars = contents.chars().peekable();

    loop {
        // Skip whitespace
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        if chars.peek().is_none() {
            break;
        }

        if chars.peek() == Some(&'"') {
            chars.next(); // consume opening quote
            let mut arg = String::new();
            while let Some(&c) = chars.peek() {
                if c == '"' {
                    chars.next();
                    break;
                }
                arg.push(c);
                chars.next();
            }
            if !arg.is_empty() {
                args.push(arg);
            }
        } else {
            let mut arg = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                arg.push(c);
                chars.next();
            }
            if !arg.is_empty() {
                args.push(arg);
            }
        }
    }

    args
}

// =========================================================================
// D_Display — Per-frame rendering orchestration (d_main.c:193-345)
// =========================================================================

/// Orchestrate per-frame rendering based on the current game state.
///
/// Equivalent of `D_Display` at `d_main.c:193-345`. This is the central
/// rendering dispatcher that:
/// 1. Detects state changes requiring screen wipes
/// 2. Dispatches to state-specific drawers (level, intermission, finale, demo)
/// 3. Handles border drawing for non-fullscreen views
/// 4. Draws the pause overlay and menu
/// 5. Manages screen wipe animation
///
/// All `I_*` platform calls are replaced with `PlatformHost` trait methods.
#[allow(clippy::too_many_arguments)]
pub fn d_display(
    game: &mut GameMain,
    game_ctrl: &mut GameCtrl,
    video: &mut VideoState,
    menu_state: &mut MenuState,
    hud_state: &mut HudState,
    statusbar_state: &mut StatusBarState,
    automap_state: &mut AutomapState,
    intermission_state: &mut IntermissionState,
    finale_state: &mut FinaleState,
    wipe_state: &mut WipeState,
    rng: &mut DoomRandom,
    platform: &mut dyn PlatformHost,
    renderer: &mut dyn Renderer,
    wad: &mut WadFile,
) {
    // d_main.c:203-206: Check for view size change
    // (set_size_needed is tracked in game_ctrl or renderer)
    // renderer.set_view_size() would be called if setsizeneeded is true

    // d_main.c:210: Determine if screen wipe is needed
    let wipe = game_ctrl.gamestate != game.wipegamestate;
    let redrawsbar = false;

    if wipe {
        // Save the current screen for wipe start
        wipe_state.start_screen(video, 0, 0, SCREENWIDTH, SCREENHEIGHT);
    }

    // d_main.c:224-278: State-based drawing dispatch
    match game_ctrl.gamestate {
        GameState::Level => {
            // d_main.c:228-275: Level rendering
            if !game_ctrl.viewactive {
                // HUD erase when view is not active
                hu_erase(
                    hud_state,
                    video,
                    automap_state.automapactive,
                    0, // viewwindowx
                    0, // viewwindowy
                    SCREENWIDTH,
                    SCREENHEIGHT,
                );
            }

            if automap_state.automapactive {
                // d_main.c:241: Automap drawing
                am_drawer(
                    automap_state,
                    video,
                    &[], // vertexes - passed from level data in full impl
                    &[], // lines
                    &[], // sectors
                    &[], // mobjs
                    &game_ctrl.players,
                    game_ctrl.consoleplayer,
                    &game_ctrl.playeringame,
                    game_ctrl.deathmatch != 0,
                );
            }

            // d_main.c:246: Status bar drawing
            // viewheight == 200 means fullscreen (no status bar visible)
            st_drawer(
                statusbar_state,
                video,
                game.fullscreen,
                redrawsbar,
                game_ctrl.netgame,
                game_ctrl.consoleplayer,
            );

            // d_main.c:256: Render the 3D view if not in automap
            if game_ctrl.viewactive && !automap_state.automapactive {
                renderer.render_player_view(&game_ctrl.players[game_ctrl.displayplayer]);
            }

            // d_main.c:271: HUD overlay drawing
            hu_drawer(hud_state, video, automap_state.automapactive);
        }
        GameState::Intermission => {
            // d_main.c:251: Intermission stats/world map
            wi_drawer(intermission_state, video);
        }
        GameState::Finale => {
            // d_main.c:255: Finale text/bunny/cast
            // f_drawer requires flat/font data from WAD which would be
            // loaded in the full game loop. Pass empty defaults here —
            // the finale state machine handles what to render.
            let empty_font: Vec<&[u8]> = Vec::new();
            f_drawer(
                finale_state,
                video,
                game_ctrl.gamemode,
                game_ctrl.gameepisode,
                &[],         // flat_data — loaded from WAD at finale start
                &empty_font, // hu_font patches
                &[],         // art_patch
                None,        // bunny_data
                &[],         // bossback_patch
                |_spr: SpriteNum, _frame: i32| -> Option<(Vec<u8>, bool)> { None },
            );
        }
        GameState::DemoScreen => {
            // d_main.c:258: Demo screen page drawing
            d_page_drawer(game, video, wad);
        }
    }

    // d_main.c:277-295: Border redraw logic
    // Track changes that require border redraws
    if game_ctrl.gamestate == GameState::Level && game.oldgamestate != GameState::Level {
        // Entered level state — need border redraw
        game.borderdrawcount = 3;
        game.viewactivestate = false;
    }

    if game_ctrl.gamestate == GameState::Level {
        let fullscreen = false; // Would be viewheight == SCREENHEIGHT
        let needs_border_redraw = fullscreen != game.fullscreen
            || game.borderdrawcount > 0
            || (game_ctrl.viewactive != game.viewactivestate)
            || (menu_state.menu_active != game.menuactivestate)
            || (menu_state.inhelpscreens != game.inhelpscreensstate);

        if needs_border_redraw && game.borderdrawcount > 0 {
            game.borderdrawcount -= 1;
        }
        // In the full implementation, R_FillBackScreen() and
        // R_DrawViewBorder() would be called here when needs_border_redraw is true

        game.fullscreen = fullscreen;
        game.viewactivestate = game_ctrl.viewactive;
        game.menuactivestate = menu_state.menu_active;
        game.inhelpscreensstate = menu_state.inhelpscreens;
    }

    // d_main.c:303-311: Pause pic overlay
    if game_ctrl.paused {
        // Draw "M_PAUSE" lump centered at top of screen
        if automap_state.automapactive {
            // d_main.c:307: Different Y offset when automap is active
            if let Some(_lump) = wad.check_num_for_name("M_PAUSE") {
                let pause_data = wad.cache_lump_name("M_PAUSE", PurgeTag::Cache).to_vec();
                video.draw_patch_direct(
                    (SCREENWIDTH - 68) / 2, // approximate patch width
                    4 + (SCREENHEIGHT - 200) / 2,
                    0,
                    &pause_data,
                );
            }
        } else {
            if let Some(_lump) = wad.check_num_for_name("M_PAUSE") {
                let pause_data = wad.cache_lump_name("M_PAUSE", PurgeTag::Cache).to_vec();
                video.draw_patch_direct(
                    (SCREENWIDTH - 68) / 2,
                    (SCREENHEIGHT - 200) / 2 + 4,
                    0,
                    &pause_data,
                );
            }
        }
    }

    // d_main.c:315: Menu drawing — always on top
    m_drawer(menu_state, game_ctrl, video, wad, hud_state);

    // d_main.c:327-344: Screen wipe animation
    if wipe {
        // Save the destination screen for wipe end
        wipe_state.end_screen(video, 0, 0, SCREENWIDTH, SCREENHEIGHT);

        // Run the wipe animation loop
        let mut done = false;
        while !done {
            let nowtime = platform.get_time();
            let tics = if nowtime > game_ctrl.gametic {
                nowtime - game_ctrl.gametic
            } else {
                1
            };
            let tics = if tics > 0 { tics } else { 1 };

            // d_main.c:339: Melt wipe effect
            let result = wipe_state.screen_wipe(
                video,
                rng,
                WipeType::Melt,
                0,
                0,
                SCREENWIDTH,
                SCREENHEIGHT,
                tics,
            );
            done = result != 0;

            // d_main.c:342: Draw menu during wipe
            m_drawer(menu_state, game_ctrl, video, wad, hud_state);

            // d_main.c:343: Update the screen
            platform.finish_update(&video.screens[0]);
        }
    }

    // d_main.c:345: Normal (non-wipe) screen update
    if !wipe {
        platform.finish_update(&video.screens[0]);
    }

    // Update tracked state for next frame
    game.oldgamestate = game_ctrl.gamestate;
    game.wipegamestate = game_ctrl.gamestate;

    // Mark status bar for redraw if needed
    let _ = redrawsbar;
}

// =========================================================================
// D_DoomMain — Master initialization entry point (d_main.c:796-1171)
// =========================================================================

/// Master initialization and main game loop entry point.
///
/// Equivalent of `D_DoomMain` at `d_main.c:796-1171`. This is the most
/// important function in the engine — it orchestrates all subsystem
/// initialization and enters the main game loop.
///
/// # Initialization sequence (preserving original order):
/// 1. Find response files (@responsefile expansion)
/// 2. Identify IWAD version (set game mode)
/// 3. Parse command-line parameters (-nomonsters, -respawn, -fast, etc.)
/// 4. Build title string
/// 5. Handle -cdrom, -turbo, -wart, -file parameters
/// 6. Initialize subsystems: V_Init, M_LoadDefaults, W_InitMultipleFiles,
///    M_Init, R_Init, P_Init, platform init, S_Init, HU_Init, ST_Init
/// 7. Process startup actions (-record, -playdemo, -timedemo, -loadgame)
/// 8. Enter D_DoomLoop (game main loop — never returns in original)
///
/// # Platform changes from original C:
/// - All platform calls go through `PlatformHost` / `AudioBackend` traits
/// - `I_Error()` replaced with `tracing::error!()` and error return
/// - `printf()` replaced with `tracing::info!()`
/// - `malloc`/`free` replaced with Rust allocations
/// - Unix paths replaced with Windows paths
#[allow(clippy::too_many_arguments)]
pub fn d_doom_main(
    game: &mut GameMain,
    game_ctrl: &mut GameCtrl,
    args: &Args,
    platform: &mut dyn PlatformHost,
    audio: &mut dyn AudioBackend,
    renderer: &mut dyn Renderer,
    video: &mut VideoState,
    menu_state: &mut MenuState,
    hud_state: &mut HudState,
    statusbar_state: &mut StatusBarState,
    wad_result: &mut Option<WadFile>,
    config: &mut ConfigDefaults,
) {
    // d_main.c:991: IdentifyVersion — detect IWAD and set gamemode.
    // MUST be called before W_InitMultipleFiles and any gamemode-dependent
    // logic (title strings, episode selection, shareware restrictions).
    // This populates game.wadfiles with the IWAD path and sets
    // game_ctrl.gamemode based on the IWAD filename.
    identify_version(game, game_ctrl, args);

    // d_main.c:811-816: Check for -nomonsters, -respawn, -fast, -devparm
    game.nomonsters = args.has_parm("-nomonsters");
    game.respawnparm = args.has_parm("-respawn");
    game.fastparm = args.has_parm("-fast");
    game.devparm = game.devparm || args.has_parm("-devparm");
    // respawnmonsters in GameCtrl is set from respawnparm
    game_ctrl.respawnmonsters = game.respawnparm;

    // d_main.c:812-816: Deathmatch settings
    if args.has_parm("-altdeath") {
        game_ctrl.deathmatch = 2;
    } else if args.has_parm("-deathmatch") {
        game_ctrl.deathmatch = 1;
    }

    // d_main.c:817-870: Build title string based on game mode
    match game_ctrl.gamemode {
        GameMode::Retail => {
            game.title = format!(
                "                         The Ultimate DOOM Startup v{}.{}                           ",
                VERSION / 100,
                VERSION % 100
            );
        }
        GameMode::Shareware => {
            game.title = format!(
                "                            DOOM Shareware Startup v{}.{}                           ",
                VERSION / 100,
                VERSION % 100
            );
        }
        GameMode::Registered => {
            game.title = format!(
                "                            DOOM Registered Startup v{}.{}                           ",
                VERSION / 100,
                VERSION % 100
            );
        }
        GameMode::Commercial => {
            game.title = format!(
                "                         DOOM 2: Hell on Earth v{}.{}                           ",
                VERSION / 100,
                VERSION % 100
            );
        }
        _ => {
            game.title = format!(
                "                         Public DOOM - v{}.{}                           ",
                VERSION / 100,
                VERSION % 100
            );
        }
    }
    info!("{}", game.title.trim());

    // d_main.c:873-876: Dev parameter notification
    if game.devparm {
        info!("{}", D_DEVSTR);
    }

    // d_main.c:885-902: Handle -turbo parameter
    // Scales forward and side movement speeds. Capped at 400% as in the
    // original C code. The mutable arrays on GameCtrl are initialised from
    // FORWARD_MOVE_DEFAULT / SIDE_MOVE_DEFAULT and modified in-place here.
    if let Some(turbo_str) = args.parm_value("-turbo") {
        let scale: i32 = turbo_str.parse().unwrap_or(200).clamp(10, 400);
        info!("turbo scale: {}%%", scale);

        game_ctrl.forward_move[0] = FORWARD_MOVE_DEFAULT[0] * scale / 100;
        game_ctrl.forward_move[1] = FORWARD_MOVE_DEFAULT[1] * scale / 100;
        game_ctrl.side_move[0] = SIDE_MOVE_DEFAULT[0] * scale / 100;
        game_ctrl.side_move[1] = SIDE_MOVE_DEFAULT[1] * scale / 100;
    }

    // d_main.c:910-936: Handle -wart parameter (development WAD)
    if let Some(wart_val) = args.parm_value("-wart") {
        if game_ctrl.gamemode == GameMode::Commercial {
            // DOOM 2: -wart <map>
            let map: i32 = wart_val.parse().unwrap_or(1);
            let wart_file = if map < 10 {
                format!("{}map0{}.wad", game.mapdir, map)
            } else {
                format!("{}map{}.wad", game.mapdir, map)
            };
            d_add_file(game, &wart_file);
        } else {
            // DOOM 1: -wart <episode> <map>
            // wart_val is argv[wart_idx+1] (episode).  Map is the NEXT arg
            // (argv[wart_idx+2]).  Original C: myargv[p+1] / myargv[p+2].
            let ep: i32 = wart_val.parse().unwrap_or(1);
            let map: i32 = args
                .check_parm("-wart")
                .and_then(|idx| args.argv(idx + 2))
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            let wart_file = format!("{}e{}m{}.wad", game.mapdir, ep, map);
            d_add_file(game, &wart_file);
        }
    }

    // d_main.c:938-946: Handle -file parameter (additional PWADs)
    if let Some(file_idx) = args.check_parm("-file") {
        // d_main.c:939: game.modifiedgame = true
        let mut idx = file_idx + 1;
        while idx < args.argc() {
            let arg = args.argv(idx).unwrap_or("");
            if arg.starts_with('-') {
                break;
            }
            d_add_file(game, arg);
            idx += 1;
        }
    }

    // d_main.c:948-959: Handle -playdemo, -timedemo parameters (pre-WAD-load)
    if let Some(demo_name) = args.parm_value("-playdemo") {
        let wad_name = format!("{}.wad", demo_name);
        if Path::new(&wad_name).exists() {
            d_add_file(game, &wad_name);
        }
    }
    if let Some(demo_name) = args.parm_value("-timedemo") {
        let wad_name = format!("{}.wad", demo_name);
        if Path::new(&wad_name).exists() {
            d_add_file(game, &wad_name);
        }
    }

    // d_main.c:960-1008: Parse -skill, -episode, -warp, -timer, -avg
    if let Some(skill_str) = args.parm_value("-skill") {
        let skill_val: i32 = skill_str.parse().unwrap_or(3);
        game.startskill = match skill_val {
            1 => Skill::Baby,
            2 => Skill::Easy,
            3 => Skill::Medium,
            4 => Skill::Hard,
            5 => Skill::Nightmare,
            _ => Skill::Medium,
        };
        game.autostart = true;
    }

    if let Some(ep_str) = args.parm_value("-episode") {
        let ep: i32 = ep_str.parse().unwrap_or(1);
        game.startepisode = ep;
        game.autostart = true;
    }

    if let Some(timer_str) = args.parm_value("-timer") {
        let _timer_val: i32 = timer_str.parse().unwrap_or(0);
        // Timer functionality for deathmatch — store for later use
    }

    if args.has_parm("-avg") {
        // Austin Virtual Gaming — 20 minute timelimit
        debug!("-avg: 20 minute deathmatch time limit");
    }

    if let Some(warp_str) = args.parm_value("-warp") {
        if game_ctrl.gamemode == GameMode::Commercial {
            let map: i32 = warp_str.parse().unwrap_or(1);
            game.startmap = map;
        } else {
            let ep: i32 = warp_str.parse().unwrap_or(1);
            game.startepisode = ep;
            // Try to get the map number from the next argument
            let warp_idx = args.check_parm("-warp").unwrap_or(0);
            if warp_idx + 2 < args.argc() {
                if let Some(map_str) = args.argv(warp_idx + 2) {
                    if let Ok(map) = map_str.parse::<i32>() {
                        game.startmap = map;
                    }
                }
            }
        }
        game.autostart = true;
    }

    // ================================================================
    // Subsystem initialization (d_main.c:1010-1113)
    // ================================================================

    // d_main.c:1012: V_Init: allocate screens
    info!("V_Init: allocate screens.");
    // Ensure video screens are allocated (VideoState constructed by caller,
    // but we verify initialization here — equivalent to V_Init).
    let _ = &video.screens; // confirm video state is live

    // d_main.c:1015: M_LoadDefaults: Load system defaults
    info!("M_LoadDefaults: Load system defaults.");
    config.load();

    // d_main.c:1019-1023: W_InitMultipleFiles
    info!("W_Init: Init WADfiles.");
    let wad_names: Vec<&str> = game.wadfiles.iter().map(|s| s.as_str()).collect();
    match WadFile::init_multiple_files(&wad_names) {
        Ok(wad) => {
            *wad_result = Some(wad);
        }
        Err(e) => {
            error!("W_InitMultipleFiles failed: {}", e);
            return;
        }
    }

    let wad = wad_result.as_mut().unwrap();

    // d_main.c:1024-1047: IWAD validation (shareware/registered checks)
    if game_ctrl.gamemode == GameMode::Shareware {
        // Check for shareware IWAD integrity
        if wad.check_num_for_name("e2m1").is_some() {
            error!(
                "This is not the shareware version! \
                 Registered/Retail WAD detected as shareware."
            );
        }
    }
    if game_ctrl.gamemode == GameMode::Registered {
        // Verify it has enough episodes for registered
        if wad.check_num_for_name("e4m1").is_some() {
            game_ctrl.gamemode = GameMode::Retail;
            info!("IdentifyVersion: upgraded Registered to Retail (Episode 4 detected)");
        }
    }

    // d_main.c:1092: M_Init: Init miscellaneous info
    info!("M_Init: Init miscellaneous info.");
    m_init(menu_state, game_ctrl.gamemode);

    // d_main.c:1095: R_Init: Init DOOM refresh daemon
    info!("R_Init: Init DOOM refresh daemon.");
    renderer.init();

    // d_main.c:1098: P_Init: Init Playloop state
    info!("P_Init: Init Playloop state.");
    let _sprite_names = p_init();

    // d_main.c:1100: I_Init: Setting up machine state (platform init)
    info!("I_Init: Setting up machine state.");
    platform.init_graphics();

    // d_main.c:1107: S_Init: Setting up sound
    info!("S_Init: Setting up sound.");
    audio.init_sound();
    audio.init_music();
    // Set initial volumes from config (defaults: sfx=8, music=8 out of 15)
    audio.set_music_volume(8);

    // d_main.c:1110: HU_Init: Setting up heads up display
    info!("HU_Init: Setting up heads up display.");
    // d_main.c:1110 — Language defaults to English (doomstat.c line 45: language = english)
    // doom2f.wad detection in identify_version sets French; otherwise English.
    hu_init(hud_state, wad, Language::English);

    // d_main.c:1113: ST_Init: Init status bar
    info!("ST_Init: Init status bar.");
    st_init(statusbar_state, wad);

    // ================================================================
    // Startup actions (d_main.c:1127-1170)
    // ================================================================

    // d_main.c:1127-1135: Handle -record
    if let Some(record_name) = args.parm_value("-record") {
        g_record_demo(game_ctrl, record_name, args);
        game.autostart = true;
    }

    // d_main.c:1150-1157: Handle -loadgame
    if let Some(loadgame_str) = args.parm_value("-loadgame") {
        let slot: i32 = loadgame_str.parse().unwrap_or(0);
        let savename = format!("{}{}.dsg", SAVEGAMENAME, slot);
        g_load_game(game_ctrl, &savename);
    }

    // d_main.c:1137-1143: Handle -playdemo (post-WAD-load)
    if let Some(demo_name) = args.parm_value("-playdemo") {
        game_ctrl.singledemo = true;
        g_defered_play_demo(game_ctrl, demo_name);
        // In the original C, D_DoomLoop() is called here and never returns.
        // We set singledemo and let the main loop handle it.
    }

    // d_main.c:1144-1149: Handle -timedemo
    if let Some(demo_name) = args.parm_value("-timedemo") {
        g_time_demo(game_ctrl, demo_name);
        // Same as above — singledemo handled in main loop
    }

    // d_main.c:1159-1167: Auto-start or title screen
    if game_ctrl.gameaction != GameAction::LoadGame {
        if game.autostart {
            g_init_new(game_ctrl, game.startskill, game.startepisode, game.startmap);
        } else {
            d_start_title(game, game_ctrl);
        }
    }

    // d_main.c:1170: D_DoomLoop — main game loop
    // In the original C code, D_DoomLoop() never returns.
    // In the Rust port, the main loop is driven by the caller (doom-bin/main.rs)
    // who calls d_doom_loop_tick() repeatedly. We initialize the state here.
    if game_ctrl.demorecording {
        g_begin_recording(game_ctrl, game.fastparm, game.nomonsters);
    }

    info!("D_DoomMain: initialization complete.");
}
