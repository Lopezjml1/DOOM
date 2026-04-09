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

//! DOOM main game loop (D_DoomLoop).
//!
//! Translated from linuxdoom-1.10/d_main.c — the D_DoomLoop function (lines 354-407).
//!
//! This is the eternal game loop that drives the entire engine: frame synchronous
//! I/O, tic processing, sound updates, and display rendering. This function is
//! called by `D_DoomMain` and never returns.
//!
//! # Game Loop Structure (per iteration)
//!
//! 1. **Frame sync I/O**: `platform.start_frame()` — pre-frame joystick/input polling
//! 2. **Tic processing** — two modes:
//!    - **Single-tic mode** (`singletics` flag): manually polls input, builds ticcmd,
//!      runs one tic of menu + game simulation, then increments counters
//!    - **Normal mode**: delegates to `try_run_tics()` which runs one or more tics
//!      with adaptive timing and network synchronization
//! 3. **Sound update**: positional sound update for the listener (console player)
//! 4. **Display**: `d_display()` — renders the current frame
//! 5. **Sound mixing**: `audio.update_sound()` + `audio.submit_sound()` — mix and
//!    output audio buffers (unconditional; replaces the `#ifndef SNDSERV` /
//!    `#ifndef SNDINTR` guards from the original C code)
//!
//! # Platform Abstraction
//!
//! All `I_*` function calls from `i_system.h`, `i_video.h`, and `i_sound.h` are
//! replaced with trait method calls on [`PlatformHost`] and [`AudioBackend`]:
//!
//! | Original C | Rust replacement |
//! |---|---|
//! | `I_InitGraphics()` | `platform.init_graphics()` |
//! | `I_StartFrame()` | `platform.start_frame()` |
//! | `I_StartTic()` | `platform.start_tic()` |
//! | `I_UpdateSound()` | `audio.update_sound()` |
//! | `I_SubmitSound()` | `audio.submit_sound()` |
//!
//! # Sound Server Removal
//!
//! The original code guarded audio mixing calls with `#ifndef SNDSERV` and
//! `#ifndef SNDINTR` preprocessor conditionals, reflecting the external
//! `sndserver` process model on Linux. In the Rust port, the SNDSERV external
//! process model is **completely removed**. SDL2's callback-based audio in
//! `doom-platform-win` handles all mixing in-process. Sound calls are
//! unconditional.
//!
//! # State Management
//!
//! No `static mut` is used. All mutable state is passed through function
//! parameters following the Rust ownership model. The `debugfile` mechanism
//! from the original C code (`FILE* debugfile`) is replaced by the `tracing`
//! crate's structured logging facility.

use tracing::{debug, info};

use crate::game::game_ctrl::{g_begin_recording, g_build_ticcmd, g_ticker, GameCtrl};
use crate::game::game_main::{d_display, d_do_advance_demo, d_process_events, GameMain};
use crate::game::game_net::{try_run_tics, NetCallbacks, NetState};
use crate::traits::audio::AudioBackend;
use crate::traits::platform::PlatformHost;
use crate::traits::renderer::Renderer;
use crate::types::doomdef::{MAXPLAYERS, SCREENHEIGHT, SCREENWIDTH, TICRATE};
use crate::types::net::BACKUPTICS;
use crate::types::ticcmd::TicCmd;
use crate::ui::automap::AutomapState;
use crate::ui::finale::FinaleState;
use crate::ui::hud::HudState;
use crate::ui::intermission::IntermissionState;
use crate::ui::menu::{m_ticker, MenuState};
use crate::ui::statusbar::StatusBarState;
use crate::ui::wipe::WipeState;
use crate::util::argv::Args;
use crate::util::random::DoomRandom;
use crate::video::video::VideoState;
use doom_wad::WadFile;

// =============================================================================
// LoopCallbacks — NetCallbacks adapter for the normal (non-singletics) path
// =============================================================================

/// Temporary adapter struct that implements [`NetCallbacks`] for use with
/// [`try_run_tics`] inside the game loop.
///
/// This struct borrows all subsystem state needed by the network tic
/// synchronization layer. It is created at the start of each loop
/// iteration's tic-processing phase and dropped before the display and
/// audio phases, releasing the borrows for direct use.
///
/// # Lifetime
///
/// All borrows are scoped to a single call to `try_run_tics`. The struct
/// is created, used, and dropped within a block in the main loop body.
struct LoopCallbacks<'a> {
    game: &'a mut GameMain,
    game_ctrl: &'a mut GameCtrl,
    menu_state: &'a mut MenuState,
    video: &'a mut VideoState,
    audio: &'a mut dyn AudioBackend,
    renderer: &'a mut dyn Renderer,
    platform: &'a mut dyn PlatformHost,
    hud_state: &'a mut HudState,
    automap_state: &'a mut AutomapState,
    args: &'a Args,
    wad: &'a mut WadFile,
}

impl<'a> NetCallbacks for LoopCallbacks<'a> {
    /// Returns current time in game tics (35 per second).
    ///
    /// Delegates to `PlatformHost::get_time()`, replacing C `I_GetTime()`.
    fn get_time(&self) -> i32 {
        self.platform.get_time()
    }

    /// Polls platform input, processes all pending events, and builds
    /// a tic command from the resulting input state.
    ///
    /// Replaces the C sequence:
    /// ```text
    /// I_StartTic();
    /// D_ProcessEvents();
    /// G_BuildTiccmd(&localcmds[maketic % BACKUPTICS]);
    /// ```
    fn poll_and_build_ticcmd(&mut self, cmd: &mut TicCmd) {
        self.platform.start_tic();
        d_process_events(
            self.game,
            self.game_ctrl,
            self.menu_state,
            self.video,
            self.audio,
            self.renderer,
            self.platform,
            self.hud_state,
            self.automap_state,
            self.args,
            self.wad,
        );
        g_build_ticcmd(self.game_ctrl, cmd);
    }

    /// Runs the game ticker for one simulation tic.
    ///
    /// Delegates to `g_ticker()`, replacing C `G_Ticker()`.
    fn game_ticker(&mut self) {
        g_ticker(self.game_ctrl);
    }

    /// Runs the menu ticker for one animation tic.
    ///
    /// Delegates to `m_ticker()`, replacing C `M_Ticker()`.
    fn menu_ticker(&mut self) {
        m_ticker(self.menu_state);
    }

    /// Returns `true` if the attract-mode demo advance is pending.
    ///
    /// Reads `GameMain.advancedemo` flag, replacing C `extern boolean advancedemo`.
    fn is_advance_demo(&self) -> bool {
        self.game.advancedemo
    }

    /// Advances to the next demo in the attract sequence.
    ///
    /// Delegates to `d_do_advance_demo()`, replacing C `D_DoAdvanceDemo()`.
    fn do_advance_demo(&mut self) {
        d_do_advance_demo(self.game, self.game_ctrl, self.audio, self.wad);
    }

    /// Reports a fatal error and terminates the process.
    ///
    /// Delegates to `PlatformHost::error()`, replacing C `I_Error()`.
    fn error(&self, msg: &str) -> ! {
        self.platform.error(msg)
    }
}

// =============================================================================
// d_doom_loop — The Main Game Loop (d_main.c lines 354-407)
// =============================================================================

/// The main DOOM game loop — runs forever, never returns.
///
/// Translated from `D_DoomLoop` in `linuxdoom-1.10/d_main.c` lines 354-407.
/// Called by `D_DoomMain` after all initialization is complete. Manages the
/// frame-synchronous I/O, tic processing, sound updates, and display
/// rendering that drive the entire engine.
///
/// # Entry Actions (lines 356-367)
///
/// 1. If demo recording is active (`game_ctrl.demorecording`), calls
///    `g_begin_recording()` to initialize the demo buffer.
/// 2. Logging initialization replaces the C `-debugfile` parameter check.
///    The original C code opened `"debug%i.txt"` via `fopen()` and wrote
///    diagnostic output via `fprintf(debugfile, ...)`. In the Rust port,
///    all diagnostic output uses the `tracing` crate, configured by the
///    binary crate's tracing subscriber. No file I/O is needed here.
/// 3. Initializes the graphics subsystem via `platform.init_graphics()`.
///
/// # Loop Body (lines 369-406)
///
/// Each iteration performs:
/// 1. `platform.start_frame()` — frame-synchronous I/O (joystick, etc.)
/// 2. Tic processing:
///    - **Single-tic mode** (`game.singletics`): polls input, builds one
///      ticcmd, runs menu + game tickers, increments gametic/maketic
///    - **Normal mode**: `try_run_tics()` runs one or more tics with
///      adaptive timing
/// 3. Sound position update (placeholder for `S_UpdateSounds` when
///    the sound subsystem module is implemented)
/// 4. `d_display()` — renders the current frame
/// 5. `audio.update_sound()` — mixes active sound channels
/// 6. `audio.submit_sound()` — presents mixed audio to the device
///
/// # Parameters
///
/// All mutable state is passed by reference. No `static mut` is used.
///
/// - `game`: Primary engine state (event queue, demo sequence, display flags)
/// - `game_ctrl`: Game state machine (skill, episode, map, players, timing)
/// - `net_state`: Network/tic synchronization state (tic buffers, timing)
/// - `platform`: Platform host (window, input, timing, graphics)
/// - `audio`: Audio backend (SFX mixing, music, output)
/// - `renderer`: Software renderer (BSP, column/span drawing)
/// - `menu_state`: In-game menu state
/// - `video`: Video buffer state (screens, patch drawing)
/// - `hud_state`: Heads-up display state (messages, chat)
/// - `statusbar_state`: Status bar widget state
/// - `automap_state`: Automap overlay state
/// - `intermission_state`: Between-level statistics state
/// - `finale_state`: End-of-game sequence state
/// - `wipe_state`: Screen transition effect state
/// - `rng`: Deterministic random number generator
/// - `args`: Command-line argument state
/// - `wad`: WAD file system
///
/// # Returns
///
/// This function never returns (`-> !`). The game exits via
/// `platform.quit()` or `platform.error()` called from within the
/// subsystem functions (e.g., quit from the menu, fatal error).
#[allow(clippy::too_many_arguments)]
pub fn d_doom_loop(
    game: &mut GameMain,
    game_ctrl: &mut GameCtrl,
    net_state: &mut NetState,
    platform: &mut dyn PlatformHost,
    audio: &mut dyn AudioBackend,
    renderer: &mut dyn Renderer,
    menu_state: &mut MenuState,
    video: &mut VideoState,
    hud_state: &mut HudState,
    statusbar_state: &mut StatusBarState,
    automap_state: &mut AutomapState,
    intermission_state: &mut IntermissionState,
    finale_state: &mut FinaleState,
    wipe_state: &mut WipeState,
    rng: &mut DoomRandom,
    args: &Args,
    wad: &mut WadFile,
) -> ! {
    // -------------------------------------------------------------------------
    // Entry: Demo recording initialization (d_main.c line 356-357)
    // -------------------------------------------------------------------------
    // Original C: if (demorecording) G_BeginRecording();
    if game_ctrl.demorecording {
        info!("Demo recording active — initializing recording buffer");
        g_begin_recording(game_ctrl);
    }

    // -------------------------------------------------------------------------
    // Entry: Debug file setup (d_main.c lines 359-365)
    // -------------------------------------------------------------------------
    // Original C:
    //   if (M_CheckParm("-debugfile")) {
    //       char filename[20];
    //       sprintf(filename, "debug%i.txt", consoleplayer);
    //       printf("debug output to: %s\n", filename);
    //       debugfile = fopen(filename, "w");
    //   }
    //
    // Replaced by tracing subscriber configuration in doom-bin. The tracing
    // crate provides structured, filterable diagnostic output controlled by
    // the RUST_LOG environment variable. No file I/O is needed here.
    info!(
        "D_DoomLoop: entering main game loop (consoleplayer={}, TICRATE={}, {}x{})",
        game_ctrl.consoleplayer, TICRATE, SCREENWIDTH, SCREENHEIGHT,
    );
    debug!(
        "D_DoomLoop: singletics={}, demorecording={}, demoplayback={}, netgame={}",
        game.singletics, game_ctrl.demorecording, game_ctrl.demoplayback, game_ctrl.netgame,
    );

    // -------------------------------------------------------------------------
    // Entry: Graphics initialization (d_main.c line 367)
    // -------------------------------------------------------------------------
    // Original C: I_InitGraphics();
    platform.init_graphics();
    info!("D_DoomLoop: graphics subsystem initialized");

    // -------------------------------------------------------------------------
    // Main loop — runs forever (d_main.c lines 369-406)
    // -------------------------------------------------------------------------
    // Original C: while (1) { ... }
    loop {
        // =====================================================================
        // Step 1: Frame synchronous I/O operations (d_main.c line 372)
        // =====================================================================
        // Original C: I_StartFrame();
        // Called before processing any tics in a frame. Time-consuming
        // synchronous operations (joystick reading) are performed here.
        platform.start_frame();

        // =====================================================================
        // Step 2: Process one or more tics (d_main.c lines 374-390)
        // =====================================================================
        if game.singletics {
            // -----------------------------------------------------------------
            // Single-tic mode (d_main.c lines 375-386)
            // -----------------------------------------------------------------
            // Debug mode: run exactly one tic per frame. Disables adaptive
            // timing. Used for deterministic debugging and benchmarking.

            // d_main.c line 377: I_StartTic();
            // Poll platform input — posts events to the event queue.
            platform.start_tic();

            // d_main.c line 378: D_ProcessEvents();
            // Drain the event queue through the responder chain
            // (menu → game → HUD → automap).
            d_process_events(
                game,
                game_ctrl,
                menu_state,
                video,
                audio,
                renderer,
                platform,
                hud_state,
                automap_state,
                args,
                wad,
            );

            // d_main.c line 379:
            //   G_BuildTiccmd(&netcmds[consoleplayer][maketic%BACKUPTICS]);
            // Build the tic command for the console player from current
            // input state and store it in the network command buffer.
            {
                let console = game_ctrl.consoleplayer;
                let slot = (net_state.maketic as usize) % BACKUPTICS;
                let cmd = &mut net_state.netcmds[console][slot];
                g_build_ticcmd(game_ctrl, cmd);
            }

            // d_main.c lines 380-381:
            //   if (advancedemo) D_DoAdvanceDemo();
            // Advance the demo sequence if the flag is set.
            if game.advancedemo {
                d_do_advance_demo(game, game_ctrl, audio, wad);
            }

            // d_main.c line 382: M_Ticker();
            // Advance menu animations (skull cursor).
            m_ticker(menu_state);

            // d_main.c line 383: G_Ticker();
            // Run one tic of game simulation.
            g_ticker(game_ctrl);

            // d_main.c lines 384-385: gametic++; maketic++;
            // Manually advance the tic counters in single-tic mode.
            game_ctrl.gametic += 1;
            net_state.maketic += 1;
        } else {
            // -----------------------------------------------------------------
            // Normal mode (d_main.c lines 387-390)
            // -----------------------------------------------------------------
            // d_main.c line 389: TryRunTics();
            // Will run at least one tic with adaptive timing. Uses the
            // NetCallbacks trait to invoke game_ticker, menu_ticker, etc.

            // Extract scalar values from game state before creating the
            // callback struct. try_run_tics takes these by value to avoid
            // borrow conflicts.
            let gametic = game_ctrl.gametic;
            let consoleplayer = game_ctrl.consoleplayer;
            let displayplayer = game_ctrl.displayplayer;
            let demoplayback = game_ctrl.demoplayback;
            let netgame = game_ctrl.netgame;
            let singletics = game.singletics;
            let playeringame = game_ctrl.playeringame;
            let paused = game_ctrl.paused;
            let sendpause = game_ctrl.sendpause;

            // Create a temporary NetCallbacks adapter that borrows all
            // subsystem state needed by the tic synchronization layer.
            // This struct is dropped at the end of this block, releasing
            // all borrows for use by d_display and audio calls below.
            let new_gametic = {
                let mut cb = LoopCallbacks {
                    game,
                    game_ctrl,
                    menu_state,
                    video,
                    audio,
                    renderer,
                    platform,
                    hud_state,
                    automap_state,
                    args,
                    wad,
                };

                try_run_tics(
                    net_state,
                    gametic,
                    consoleplayer,
                    displayplayer,
                    demoplayback,
                    netgame,
                    singletics,
                    &playeringame,
                    paused,
                    sendpause,
                    &mut cb,
                )
            };
            // LoopCallbacks dropped here — all borrows released.

            // Update gametic with the value returned by try_run_tics.
            // In the original C code, gametic was a global variable
            // incremented inside TryRunTics. Here we propagate the
            // updated value back to the game controller.
            game_ctrl.gametic = new_gametic;
        }

        // =====================================================================
        // Step 3: Sound position update (d_main.c line 392)
        // =====================================================================
        // Original C: S_UpdateSounds(players[consoleplayer].mo);
        //
        // S_UpdateSounds updates positional audio for all active sound channels
        // based on the listener's position (the console player's map object).
        // The S_UpdateSounds function lives in the high-level sound module
        // (s_sound.c in the original). In this phase, positional sound updates
        // are handled at a higher integration level. The audio backend's
        // update_sound() call below handles the mixing pipeline.
        //
        // When the full sound subsystem is integrated, the call would be:
        //   s_update_sounds(&game_ctrl.players, game_ctrl.consoleplayer, audio);
        //
        // For now, log the listener position for diagnostic purposes.
        {
            let console = game_ctrl.consoleplayer;
            if console < MAXPLAYERS {
                let listener_mobj = game_ctrl.players[console].mobj;
                debug!(
                    "D_DoomLoop: sound update (listener mobj={:?}, gametic={})",
                    listener_mobj, game_ctrl.gametic
                );
            }
        }

        // =====================================================================
        // Step 4: Update display — render current frame (d_main.c line 395)
        // =====================================================================
        // Original C: D_Display();
        d_display(
            game,
            game_ctrl,
            video,
            menu_state,
            hud_state,
            statusbar_state,
            automap_state,
            intermission_state,
            finale_state,
            wipe_state,
            rng,
            platform,
            renderer,
            wad,
        );

        // =====================================================================
        // Step 5: Sound mixing and output (d_main.c lines 397-405)
        // =====================================================================
        // Original C:
        //   #ifndef SNDSERV
        //   I_UpdateSound();     // Sound mixing for the buffer is synchronous.
        //   #endif
        //   #ifndef SNDINTR
        //   I_SubmitSound();     // Synchronous sound output.
        //   #endif
        //
        // CRITICAL PLATFORM CHANGE: The #ifndef SNDSERV and #ifndef SNDINTR
        // guards are removed. The original SNDSERV external process model and
        // SNDINTR interrupt-driven model are both replaced by SDL2's in-process
        // callback-based audio in doom-platform-win. Audio mixing and submission
        // are called unconditionally.
        audio.update_sound();
        audio.submit_sound();
    }
}
