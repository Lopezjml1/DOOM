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

//! DOOM executable entry point — translated from `linuxdoom-1.10/i_main.c`.
//!
//! The original C entry point simply set `myargc`/`myargv` globals and called
//! `D_DoomMain()`. This Rust version is more explicit:
//!
//! 1. Initialize structured logging via `tracing-subscriber`
//! 2. Parse command-line arguments via `clap` (`Cli` struct from `cli.rs`)
//! 3. Validate the IWAD file path with clear user-facing diagnostics
//! 4. Construct the SDL2 platform host (`doom-platform-win`)
//! 5. Initialize the WAD provider (`doom-wad`), loading IWAD and optional PWADs
//! 6. Construct the software renderer (`doom-render-soft`)
//! 7. Initialize the audio backend (`doom-platform-win`)
//! 8. Construct all game state objects and call `d_doom_main` from `doom-core`
//! 9. Enter the main game loop (D_DoomLoop equivalent)
//! 10. Handle top-level errors with clear user-facing diagnostics
//!
//! ## Original C Entry Point (i_main.c, 46 lines)
//!
//! ```c
//! int main(int argc, char** argv) {
//!     myargc = argc;
//!     myargv = argv;
//!     D_DoomMain();
//!     return 0;
//! }
//! ```
//!
//! The Rust version is more elaborate because CLI parsing, structured logging,
//! platform initialization, WAD loading, renderer construction, and audio
//! setup are all made explicit in the entry point rather than being hidden
//! behind global variables and implicit initialization sequences.

mod cli;

use clap::Parser;
use cli::Cli;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use std::process;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Binary entry point — the Rust equivalent of `i_main.c:main()`.
///
/// This function is intentionally small: it parses CLI arguments, initializes
/// logging, and delegates all game startup logic to [`run()`]. The delegation
/// pattern enables clean error handling with `Result` — any error returned
/// by `run()` is reported to the user via both `tracing::error!()` and
/// `eprintln!()` before terminating with exit code 1.
///
/// # Exit Codes
///
/// - `0` — Normal exit (game quit via menu or Escape)
/// - `1` — Fatal error during initialization or gameplay
fn main() {
    // Step 1: Parse CLI arguments (replaces myargc/myargv + M_CheckParm)
    let cli = Cli::parse();

    // Step 2: Initialize tracing/logging (replaces printf/fprintf)
    init_logging(&cli);

    // Step 3: Log startup banner
    info!("DOOM Rust Port — translated from id Software DOOM 1.10");
    info!("Using SDL2 platform backend");

    // Step 4: Run the game, catch and report errors
    if let Err(e) = run(cli) {
        error!("Fatal error: {}", e);
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Logging initialization
// ---------------------------------------------------------------------------

/// Initialize structured logging via `tracing-subscriber`.
///
/// If `--verbose` is passed, sets the default log level to `DEBUG`.
/// Otherwise, defaults to `INFO` level.
/// The `RUST_LOG` environment variable overrides both settings, allowing
/// fine-grained control such as `RUST_LOG=doom_core=debug,doom_wad=trace`.
///
/// Replaces all `printf()` and `fprintf(stderr, ...)` calls in the original
/// C codebase with structured, filterable diagnostic output.
fn init_logging(cli: &Cli) {
    let default_filter = if cli.verbose { "debug" } else { "info" };

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}

// ---------------------------------------------------------------------------
// Main game startup and loop
// ---------------------------------------------------------------------------

/// Execute the DOOM startup sequence and enter the main game loop.
///
/// This is the Rust equivalent of the original `i_main.c` `main()` function,
/// expanded to make all initialization steps explicit and leveraging Rust's
/// error handling via `Result`:
///
/// 1. Validate the IWAD file path (deterministic startup diagnostics)
/// 2. Construct the SDL2 platform host
/// 3. Construct the software renderer
/// 4. Initialize the audio backend (non-fatal on failure)
/// 5. Build all game state objects
/// 6. Call `d_doom_main` for engine initialization (loads WAD internally)
/// 7. Extract loaded WAD and enter the main game loop (D_DoomLoop)
///
/// # Errors
///
/// Returns a boxed error on any fatal failure during initialization:
/// - IWAD file not found
/// - SDL2 platform initialization failure
/// - WAD file parsing failure
///
/// Audio initialization failure is non-fatal — the game continues without
/// audio and logs a warning.
fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    // -----------------------------------------------------------------------
    // Step 1: Validate IWAD path
    // Per AAP §0.8.2: "Deterministic startup path with clear diagnostics"
    // -----------------------------------------------------------------------
    let iwad_path = &cli.iwad;
    if !std::path::Path::new(iwad_path).exists() {
        return Err(format!(
            "IWAD file not found at path: {}\n\
             Please provide a valid path using --iwad <path>.\n\
             Common Steam locations:\n  \
             C:\\Program Files (x86)\\Steam\\steamapps\\common\\Ultimate Doom\\base\\DOOM.WAD\n  \
             C:\\Program Files (x86)\\Steam\\steamapps\\common\\Doom 2\\base\\DOOM2.WAD",
            iwad_path
        )
        .into());
    }
    info!("IWAD: {}", iwad_path);

    // Log PWAD files if any
    for pwad in &cli.pwad {
        info!("PWAD: {}", pwad);
    }

    // Log warp target if specified
    if let Some(ref warp) = cli.warp {
        info!("Warp target: {:?}", warp);
    }

    info!("Skill level: {}", cli.skill);

    // -----------------------------------------------------------------------
    // Step 2: Construct SDL2 platform host
    // Replaces i_main.c setting argc/argv and the implicit platform init.
    // SdlPlatform::new() initializes the SDL2 context and timer subsystem.
    // Graphics are NOT started here — init_graphics() is called later by
    // d_doom_main during engine initialization.
    // -----------------------------------------------------------------------
    info!("Initializing SDL2 platform...");
    let mut platform = doom_platform_win::SdlPlatform::new(None)
        .map_err(|e| format!("Failed to initialize platform: {}", e))?;
    info!("SDL2 platform initialized successfully");

    // -----------------------------------------------------------------------
    // Step 3: Construct software renderer
    // (WAD loading is handled by d_doom_main via identify_version →
    //  d_add_file → W_InitMultipleFiles, matching the original C flow.)
    // -----------------------------------------------------------------------
    info!("Initializing software renderer...");
    let mut renderer = doom_render_soft::SoftwareRenderer::new();
    info!("Software renderer initialized");

    // -----------------------------------------------------------------------
    // Step 4: Initialize audio backend
    // Audio initialization failure is non-fatal — the game continues without
    // audio and logs a warning. This matches behavior of systems where no
    // audio device is available.
    // -----------------------------------------------------------------------
    info!("Initializing audio backend...");
    let audio_result = platform.create_audio_backend();
    let mut audio_backend = match audio_result {
        Ok(audio) => {
            info!("Audio backend initialized successfully");
            Some(audio)
        }
        Err(e) => {
            warn!(
                "Audio initialization failed: {}. Continuing without audio.",
                e
            );
            None
        }
    };

    // -----------------------------------------------------------------------
    // Step 5: Build all game state objects
    // Per AAP §0.7.5: "Global state consolidated into structs passed by
    // mutable reference" — main.rs creates all state objects and passes
    // them down the call chain.
    // -----------------------------------------------------------------------

    // Build the Args struct from the CLI arguments for legacy -param support.
    // The Cli struct handles --iwad/--pwad/--warp/--skill via clap,
    // but d_doom_main also checks for original-style parameters like
    // -nomonsters, -respawn, -fast, -devparm, -turbo, -file, etc.
    let raw_args: Vec<String> = build_legacy_args(&cli);
    let args = doom_core::util::argv::Args::new(raw_args.iter().map(|s| s.as_str()));

    // Game main state — consolidates d_main.c global variables
    let mut game = doom_core::game::game_main::GameMain::new();

    // Game control state — consolidates g_game.c global variables
    let mut game_ctrl = doom_core::game::game_ctrl::GameCtrl::new();

    // Video state — screen buffers, gamma tables (v_video.c)
    let mut video = doom_core::video::video::VideoState::new();

    // UI subsystem states
    let mut menu_state = doom_core::ui::menu::MenuState::new();
    let mut hud_state = doom_core::ui::hud::HudState::new();
    let mut statusbar_state = doom_core::ui::statusbar::StatusBarState::new();
    let mut automap_state = doom_core::ui::automap::AutomapState::new();
    let mut intermission_state = doom_core::ui::intermission::IntermissionState::new();
    let mut finale_state = doom_core::ui::finale::FinaleState::default();
    let mut wipe_state = doom_core::ui::wipe::WipeState::new();

    // Network state (single-player stub — AAP §0.3.2 defers networking)
    let mut net_state = doom_core::game::game_net::NetState::new();

    // Deterministic PRNG — m_random.c rndtable[256]
    let mut rng = doom_core::util::random::DoomRandom::default();

    // Configuration defaults (m_misc.c default_t table)
    let mut config = doom_core::util::misc::ConfigDefaults::build_defaults();

    // WAD is populated by d_doom_main via identify_version → d_add_file →
    // W_InitMultipleFiles. Passed as Option<WadFile> so d_doom_main can
    // move ownership into it.
    let mut wad_option: Option<doom_wad::WadFile> = None;

    // -----------------------------------------------------------------------
    // Step 6: Create a no-op audio backend for the case where audio failed.
    // d_doom_main requires &mut dyn AudioBackend, so we provide a no-op
    // implementation when audio is unavailable.
    // -----------------------------------------------------------------------
    let mut noop_audio = NoopAudioBackend;
    let audio_ref: &mut dyn doom_core::traits::audio::AudioBackend =
        if let Some(ref mut ab) = audio_backend {
            ab
        } else {
            &mut noop_audio
        };

    // -----------------------------------------------------------------------
    // Step 7: Call d_doom_main for engine initialization
    // This is the equivalent of the original:
    //   myargc = argc;
    //   myargv = argv;
    //   D_DoomMain();
    //
    // In the Rust port, D_DoomMain receives all dependencies explicitly
    // rather than accessing them through globals. It performs all engine
    // initialization (IWAD detection, subsystem init, level loading) but
    // returns control to the caller instead of entering an infinite loop.
    // -----------------------------------------------------------------------
    info!("Entering D_DoomMain...");

    doom_core::game::game_main::d_doom_main(
        &mut game,
        &mut game_ctrl,
        &args,
        &mut platform,
        audio_ref,
        &mut renderer,
        &mut video,
        &mut menu_state,
        &mut hud_state,
        &mut statusbar_state,
        &mut wad_option,
        &mut config,
    );

    info!("D_DoomMain: initialization complete — all subsystems ready");

    // -----------------------------------------------------------------------
    // Step 8: Enter the main game loop (D_DoomLoop equivalent)
    //
    // In the original C code, D_DoomLoop() was called at the end of
    // D_DoomMain and never returned (infinite loop).  In the Rust port the
    // loop lives in doom_core::game::game_loop::d_doom_loop and has the
    // same divergent return type (-> !).
    //
    // All required state objects were constructed above and are passed by
    // mutable reference.  The WAD is extracted from the Option populated
    // by d_doom_main.
    // -----------------------------------------------------------------------
    let wad = wad_option
        .as_mut()
        .expect("WAD should have been loaded by d_doom_main via identify_version");

    // Re-create audio_ref since the first borrow ended with d_doom_main.
    let mut noop_audio2 = NoopAudioBackend;
    let audio_ref2: &mut dyn doom_core::traits::audio::AudioBackend =
        if let Some(ref mut ab) = audio_backend {
            ab
        } else {
            &mut noop_audio2
        };

    info!("Entering D_DoomLoop...");

    // d_doom_loop returns `-> !` — it never returns.
    doom_core::game::game_loop::d_doom_loop(
        &mut game,
        &mut game_ctrl,
        &mut net_state,
        &mut platform,
        audio_ref2,
        &mut renderer,
        &mut menu_state,
        &mut video,
        &mut hud_state,
        &mut statusbar_state,
        &mut automap_state,
        &mut intermission_state,
        &mut finale_state,
        &mut wipe_state,
        &mut rng,
        &args,
        wad,
    );
    // d_doom_loop never returns — the line below is unreachable.
}

// ---------------------------------------------------------------------------
// Legacy argument builder
// ---------------------------------------------------------------------------

/// Build a legacy-style argument vector from the structured `Cli` arguments.
///
/// The `doom-core` game initialization (`d_doom_main`) uses the `Args` struct
/// to check for original-style DOOM command-line parameters like `-nomonsters`,
/// `-respawn`, `-fast`, `-skill`, `-episode`, `-warp`, etc. This function
/// translates the clap-parsed `Cli` struct into the traditional argument
/// vector format expected by `Args::new()`.
///
/// # Arguments
///
/// * `cli` — The parsed CLI arguments from clap
///
/// # Returns
///
/// A `Vec<String>` representing the legacy-style argument list, starting
/// with the program name `"doom-rust"`.
fn build_legacy_args(cli: &Cli) -> Vec<String> {
    let mut args: Vec<String> = vec!["doom-rust".to_string()];

    // -iwad <path> — always present (required by clap)
    args.push("-iwad".to_string());
    args.push(cli.iwad.clone());

    // -file <pwad...> — optional PWAD files
    if !cli.pwad.is_empty() {
        args.push("-file".to_string());
        for pwad in &cli.pwad {
            args.push(pwad.clone());
        }
    }

    // -warp <episode> <map> or -warp <map>
    if let Some(ref warp) = cli.warp {
        args.push("-warp".to_string());
        for val in warp {
            args.push(val.to_string());
        }
    }

    // -skill <1-5>
    if cli.skill != 3 {
        args.push("-skill".to_string());
        args.push(cli.skill.to_string());
    }

    // -devparm if verbose mode (enables development parameter diagnostics)
    if cli.verbose {
        args.push("-devparm".to_string());
    }

    args
}

// ---------------------------------------------------------------------------
// No-op audio backend for when audio initialization fails
// ---------------------------------------------------------------------------

/// A no-operation audio backend used when SDL2 audio initialization fails.
///
/// All methods are no-ops that silently succeed. This allows the game to
/// run without audio rather than crashing when no audio device is available.
/// Matches the behavior of the original DOOM when SNDSERV was not found
/// or `/dev/dsp` could not be opened — the game continued without sound.
struct NoopAudioBackend;

impl doom_core::traits::audio::AudioBackend for NoopAudioBackend {
    fn init_sound(&mut self) {
        // No-op: audio unavailable
    }

    fn shutdown_sound(&mut self) {
        // No-op: audio unavailable
    }

    fn start_sound(&mut self, _id: i32, _vol: i32, _sep: i32, _pitch: i32, _priority: i32) -> i32 {
        // No-op: return channel 0 (no actual playback)
        0
    }

    fn stop_sound(&mut self, _handle: i32) {
        // No-op: audio unavailable
    }

    fn sound_is_playing(&self, _handle: i32) -> bool {
        // No-op: nothing is ever playing
        false
    }

    fn update_sound(&mut self) {
        // No-op: audio unavailable
    }

    fn update_sound_params(&mut self, _handle: i32, _vol: i32, _sep: i32, _pitch: i32) {
        // No-op: audio unavailable
    }

    fn submit_sound(&mut self) {
        // No-op: audio unavailable
    }

    fn init_music(&mut self) {
        // No-op: audio unavailable
    }

    fn shutdown_music(&mut self) {
        // No-op: audio unavailable
    }

    fn set_music_volume(&mut self, _volume: i32) {
        // No-op: audio unavailable
    }

    fn pause_song(&mut self, _handle: i32) {
        // No-op: audio unavailable
    }

    fn resume_song(&mut self, _handle: i32) {
        // No-op: audio unavailable
    }

    fn register_song(&mut self, _data: &[u8]) -> i32 {
        // No-op: return handle 0
        0
    }

    fn play_song(&mut self, _handle: i32, _looping: bool) {
        // No-op: audio unavailable
    }

    fn stop_song(&mut self, _handle: i32) {
        // No-op: audio unavailable
    }

    fn unregister_song(&mut self, _handle: i32) {
        // No-op: audio unavailable
    }
}
