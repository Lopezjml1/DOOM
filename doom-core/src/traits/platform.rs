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

//! PlatformHost trait definition — window, input, timing, and lifecycle abstraction.
//!
//! Translated from `linuxdoom-1.10/i_system.h` and `linuxdoom-1.10/i_video.h`.
//! Consolidates system services (timing, lifecycle, error handling) and
//! video display (initialization, frame presentation, palette management)
//! into a single trait.
//!
//! ## Original C Interfaces
//!
//! **i_system.h** — System services:
//! - `I_GetTime()` — current time in tics (replaces `gettimeofday`)
//! - `I_StartFrame()` — pre-frame processing (joystick polling)
//! - `I_StartTic()` — per-tic input polling
//! - `I_BaseTiccmd()` — base (empty) tic command
//! - `I_Quit()` — clean application exit
//! - `I_Error()` — fatal error exit with message
//!
//! **i_video.h** — Video display:
//! - `I_InitGraphics()` — display initialization (replaces X11 init)
//! - `I_ShutdownGraphics()` — display shutdown
//! - `I_FinishUpdate()` — present framebuffer (replaces MIT-SHM/XImage blit)
//! - `I_SetPalette()` — set 256-color palette (replaces X11 colormap)
//! - `I_ReadScreen()` — read back screen contents
//!
//! ## Platform Replacements
//!
//! | C Function | Linux Implementation | Windows Replacement |
//! |---|---|---|
//! | `I_GetTime` | `gettimeofday()` | `std::time::Instant` or SDL2 timer |
//! | `I_StartTic` | XEvent polling | SDL2 `EventPump` |
//! | `I_InitGraphics` | X11 `XOpenDisplay`, `XCreateWindow` | SDL2 `Window` |
//! | `I_FinishUpdate` | MIT-SHM `XShmPutImage` | SDL2 `Canvas::present()` |
//! | `I_SetPalette` | X11 colormap allocation | SDL2 palette → texture mapping |
//! | `I_Error` | `fprintf(stderr, ...)` + `exit(-1)` | `tracing::error!()` + `std::process::exit(1)` |
//!
//! ## Functions NOT Ported to PlatformHost
//!
//! The following functions from the original C headers are intentionally excluded:
//!
//! - `I_Init()` — initialization is handled by struct construction and `init_graphics()`
//! - `I_ZoneBase()` — zone allocator replaced by Rust standard allocator (AAP §0.7.4)
//! - `I_AllocLow()` — DOS-era low memory allocation, not applicable
//! - `I_Tactile()` — force-feedback support, was a no-op in the original Linux code
//! - `I_UpdateNoBlit()` — typically a no-op in the original, not needed
//! - `I_WaitVBL()` — DOS vertical blank wait, SDL2 manages vsync internally
//! - `I_BeginRead()` / `I_EndRead()` — disk activity icon, minor visual feature

use crate::types::ticcmd::TicCmd;

/// Platform-independent host interface for window management, input,
/// timing, and lifecycle operations.
///
/// This is the primary platform abstraction trait, consolidating the
/// interfaces from `i_system.h` and `i_video.h` into a single trait.
/// Implementations provide the bridge between portable game logic
/// and the host operating system.
///
/// ## Design Notes
///
/// The original C codebase used free functions (`I_GetTime()`, `I_FinishUpdate()`,
/// etc.) that directly called X11/Xlib and `gettimeofday()`. In the Rust port,
/// these become trait methods that can be implemented by any platform backend.
///
/// The `doom-platform-win` crate implements this trait using SDL2 on Windows 11.
/// The trait architecture enables future platform backends (Linux/SDL2, macOS,
/// etc.) without modifying `doom-core`.
///
/// ## Method Receiver Conventions
///
/// - `&self` for read-only queries: [`get_time`], [`read_screen`], [`base_ticcmd`],
///   [`quit`], [`error`]
/// - `&mut self` for state-mutating operations: [`start_frame`], [`start_tic`],
///   [`init_graphics`], [`shutdown_graphics`], [`finish_update`], [`set_palette`]
///
/// [`get_time`]: PlatformHost::get_time
/// [`read_screen`]: PlatformHost::read_screen
/// [`base_ticcmd`]: PlatformHost::base_ticcmd
/// [`quit`]: PlatformHost::quit
/// [`error`]: PlatformHost::error
/// [`start_frame`]: PlatformHost::start_frame
/// [`start_tic`]: PlatformHost::start_tic
/// [`init_graphics`]: PlatformHost::init_graphics
/// [`shutdown_graphics`]: PlatformHost::shutdown_graphics
/// [`finish_update`]: PlatformHost::finish_update
/// [`set_palette`]: PlatformHost::set_palette
pub trait PlatformHost {
    // ===== Timing =====

    /// Returns the current time in tics (35 tics per second).
    ///
    /// Equivalent of `I_GetTime()` from `i_system.h` line 45.
    /// Called by `D_DoomLoop` and `TryRunTics` to drive the game simulation.
    ///
    /// The original Linux implementation uses `gettimeofday()` and computes:
    /// ```text
    /// newtics = (tp.tv_sec - basetime) * TICRATE
    ///         + tp.tv_usec * TICRATE / 1_000_000;
    /// ```
    /// where `TICRATE = 35`. The Windows implementation should use
    /// `std::time::Instant` or an SDL2 high-resolution timer to achieve
    /// equivalent precision.
    ///
    /// # Returns
    ///
    /// Current time expressed as an integer tic count since engine startup.
    /// The value monotonically increases at approximately 35 tics per second.
    fn get_time(&self) -> i32;

    // ===== Frame Lifecycle =====

    /// Called before processing any tics in a frame.
    ///
    /// Equivalent of `I_StartFrame()` from `i_system.h` line 56.
    /// Called by `D_DoomLoop` just after displaying a frame.
    /// Time-consuming synchronous operations (e.g., joystick reading) are
    /// performed here. Implementations may post events via `D_PostEvent`.
    ///
    /// The original Linux implementation is a no-op (empty function body).
    /// Implementations should perform any per-frame polling that is too
    /// expensive for per-tic processing.
    fn start_frame(&mut self);

    /// Called before processing each tic in a frame — polls input.
    ///
    /// Equivalent of `I_StartTic()` from `i_system.h` line 64.
    /// Quick synchronous operations: keyboard and mouse input polling.
    /// Posts input events to the game event queue via `D_PostEvent`.
    ///
    /// The original Linux implementation calls `XNextEvent` in a loop
    /// to drain the X11 event queue, translating X11 key symbols to
    /// DOOM key codes and posting `ev_keydown`/`ev_keyup`/`ev_mouse`
    /// events. The Windows SDL2 implementation should call
    /// `event_pump.poll_iter()` and perform equivalent translation.
    fn start_tic(&mut self);

    // ===== Graphics Subsystem =====

    /// Initialize the display/graphics subsystem.
    ///
    /// Equivalent of `I_InitGraphics()` from `i_video.h` line 37.
    /// Called by `D_DoomMain` to create the game window and set up
    /// the rendering surface.
    ///
    /// The original creates an X11 window (320×200 or scaled by the
    /// `-multiply` parameter) with MIT-SHM shared memory for fast
    /// blitting. The Windows SDL2 implementation should create an
    /// SDL2 window with a streaming texture for the 320×200
    /// palettized framebuffer.
    ///
    /// This method must be called before [`finish_update`], [`set_palette`],
    /// or [`read_screen`] are invoked.
    ///
    /// [`finish_update`]: PlatformHost::finish_update
    /// [`set_palette`]: PlatformHost::set_palette
    /// [`read_screen`]: PlatformHost::read_screen
    fn init_graphics(&mut self);

    /// Shut down the display/graphics subsystem.
    ///
    /// Equivalent of `I_ShutdownGraphics()` from `i_video.h` line 40.
    /// Releases display resources, destroys the window, and frees
    /// associated memory. Called during [`quit`] and on fatal error.
    ///
    /// [`quit`]: PlatformHost::quit
    fn shutdown_graphics(&mut self);

    /// Present the rendered framebuffer to the display.
    ///
    /// Equivalent of `I_FinishUpdate()` from `i_video.h` line 46.
    /// Called after the renderer has finished writing to the screen buffer
    /// (`screens[0]`). Copies the 320×200 palettized framebuffer to the
    /// display surface with appropriate scaling.
    ///
    /// # Parameters
    ///
    /// - `screen`: The 320×200 (`SCREENWIDTH` × `SCREENHEIGHT`) palettized
    ///   framebuffer as a byte slice. Each byte is a palette index (0–255).
    ///   Total size: 64,000 bytes (320 × 200).
    ///
    /// The original Linux implementation uses MIT-SHM `XShmPutImage` for
    /// zero-copy display, with a fallback to `XPutImage`. The `multiply`
    /// variable controls pixel scaling (1×, 2×, 3×, or 4×).
    /// The Windows SDL2 implementation should map palette indices to RGB
    /// via the current palette, write to an SDL2 streaming texture, and
    /// call `Canvas::present()`.
    fn finish_update(&mut self, screen: &[u8]);

    /// Set the 256-color palette.
    ///
    /// Equivalent of `I_SetPalette(byte* palette)` from `i_video.h` line 43.
    /// Called when the palette changes (e.g., pain flash, radiation suit
    /// green tint, invulnerability grayscale, berserk red tint, menu
    /// focus transitions).
    ///
    /// # Parameters
    ///
    /// - `palette`: Exactly 768 bytes (256 × 3) of RGB color data.
    ///   Each entry is 3 consecutive bytes: red, green, blue (0–255 each).
    ///   The palette data comes from the `PLAYPAL` lump in the WAD file,
    ///   which contains 14 palettes of 768 bytes each. The game selects
    ///   the active palette based on player damage, powerup status, etc.
    ///
    /// The original Linux implementation allocates X11 colormap entries
    /// and updates the display colormap. The SDL2 implementation should
    /// store the palette for use during [`finish_update`] when converting
    /// palettized pixels to RGB.
    ///
    /// [`finish_update`]: PlatformHost::finish_update
    fn set_palette(&mut self, palette: &[u8]);

    /// Read back the current screen contents into a buffer.
    ///
    /// Equivalent of `I_ReadScreen(byte* scr)` from `i_video.h` line 51.
    /// Used for screen wipe transitions (capturing the "before" frame)
    /// and for taking screenshots.
    ///
    /// # Parameters
    ///
    /// - `buffer`: Mutable byte slice to receive the screen data.
    ///   Must be at least `SCREENWIDTH` × `SCREENHEIGHT` (64,000) bytes.
    ///   Receives palettized pixel data — one byte per pixel, where each
    ///   byte is a palette index (0–255).
    ///
    /// The original implementation copies from the internal framebuffer
    /// (`screens[0]`) directly into the provided buffer.
    fn read_screen(&self, buffer: &mut [u8]);

    // ===== Input =====

    /// Return a base (empty/default) tic command.
    ///
    /// Equivalent of `I_BaseTiccmd()` from `i_system.h` line 74.
    /// Returns a zeroed [`TicCmd`] that will be modified by the game loop
    /// with input from event processing.
    ///
    /// The original C implementation maintains a static zero-initialized
    /// `ticcmd_t` and returns a pointer to it. In Rust, we return a
    /// [`TicCmd`] by value (8 bytes, trivially copyable) to avoid
    /// lifetime management issues with static references.
    ///
    /// # Returns
    ///
    /// A default-initialized [`TicCmd`] with all fields set to zero,
    /// representing "no input" for this tic.
    fn base_ticcmd(&self) -> TicCmd;

    // ===== Lifecycle =====

    /// Perform a clean exit from the application.
    ///
    /// Equivalent of `I_Quit()` from `i_system.h` line 79.
    /// Called by `M_Responder` when the user selects "Quit" from the menu.
    ///
    /// The original implementation performs the following shutdown sequence:
    /// 1. `D_QuitNetGame()` — disconnect from any network game
    /// 2. `I_ShutdownSound()` — release audio resources
    /// 3. `I_ShutdownMusic()` — stop music playback
    /// 4. `M_SaveDefaults()` — write configuration to disk
    /// 5. `I_ShutdownGraphics()` — destroy the display window
    /// 6. `exit(0)` — terminate the process
    ///
    /// Implementations should perform equivalent cleanup and then
    /// call `std::process::exit(0)` to terminate cleanly.
    fn quit(&self);

    /// Report a fatal error and terminate the application.
    ///
    /// Equivalent of `I_Error(char *error, ...)` from `i_system.h` line 89.
    /// This is a diverging function — it never returns.
    ///
    /// # Parameters
    ///
    /// - `msg`: Human-readable error message string describing the fatal
    ///   condition. The original C function accepts `printf`-style format
    ///   strings with variadic arguments; in Rust, callers should use
    ///   `format!()` to compose the message before passing it.
    ///
    /// The original C implementation writes the message to `stderr` via
    /// `fprintf`, then calls `exit(-1)`. The Rust implementation should
    /// log via `tracing::error!()` (or `eprintln!` as a fallback) and
    /// call `std::process::exit(1)`.
    ///
    /// # Divergence
    ///
    /// The `-> !` return type indicates this function never returns.
    /// Implementations must ensure the process terminates (e.g., via
    /// `std::process::exit`) rather than panicking, to allow orderly
    /// shutdown of platform resources.
    fn error(&self, msg: &str) -> !;
}
