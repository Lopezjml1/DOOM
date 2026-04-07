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

//! Windows 11 platform backend for DOOM using SDL2.
//!
//! Translated from `linuxdoom-1.10/i_*.c` files and `sndserv/*.c`.
//!
//! This crate implements the platform-specific layer that bridges the
//! deterministic game logic in `doom-core` with the host operating
//! system.  It provides:
//!
//! - **Window management** ([`window::WindowManager`]) — SDL2 window
//!   creation, event pump, and mouse grab.
//! - **Video output** ([`video::VideoOutput`]) — palettized 320×200
//!   framebuffer → ARGB8888 streaming texture → SDL2 canvas.
//! - **Audio** ([`audio::SdlAudioBackend`]) — 8-channel SFX mixing
//!   via SDL2's callback-based audio device.
//! - **Timer** ([`timer::Timer`]) — high-resolution timing at 35 Hz.
//! - **Input** ([`input`]) — SDL2 keyboard/mouse → DOOM event translation.
//! - **Filesystem** ([`filesystem`]) — Windows IWAD discovery and
//!   config/save directory management.
//!
//! # Architecture
//!
//! The primary exported type is [`SdlPlatform`], which implements the
//! [`doom_core::traits::PlatformHost`] trait.  This struct owns the SDL2
//! context and all platform subsystems.  The audio backend is exposed
//! separately as [`audio::SdlAudioBackend`] implementing
//! [`doom_core::traits::AudioBackend`].
//!
//! ```text
//!  doom-bin
//!    │
//!    ├── SdlPlatform  ← implements PlatformHost
//!    │     ├── sdl2::Sdl context
//!    │     ├── WindowManager (event pump, window)
//!    │     ├── VideoOutput   (canvas, palette, texture)
//!    │     ├── Timer         (Instant-based 35 Hz timing)
//!    │     ├── InputState    (mouse tracking, button state)
//!    │     └── event_queue   (Vec<Event> per-tic buffer)
//!    │
//!    └── SdlAudioBackend ← implements AudioBackend
//!          └── SDL2 AudioDevice with DoomAudioCallback
//! ```

pub mod audio;
pub mod filesystem;
pub mod input;
pub mod timer;
pub mod video;
pub mod window;

// Re-export the primary types for convenient access by doom-bin.
pub use audio::SdlAudioBackend;

use doom_core::traits::PlatformHost;
use doom_core::types::event::Event;
use doom_core::types::ticcmd::TicCmd;
use input::InputState;
use timer::Timer;
use tracing::{debug, error, info, warn};
use video::VideoOutput;
use window::WindowManager;

/// Screen width in pixels (matches `SCREENWIDTH` from `doomdef.h`).
const SCREENWIDTH: u32 = 320;

/// Screen height in pixels (matches `SCREENHEIGHT` from `doomdef.h`).
const SCREENHEIGHT: u32 = 200;

/// Total screen buffer size in bytes (320 × 200 = 64,000).
const SCREEN_SIZE: usize = (SCREENWIDTH * SCREENHEIGHT) as usize;

/// SDL2-based platform host for Windows 11.
///
/// Implements [`PlatformHost`] by orchestrating the SDL2 window manager,
/// video output, timer, and input subsystems.  This is the concrete
/// platform backend that `doom-bin` constructs at startup and passes
/// to `doom-core`'s game loop.
///
/// # Construction
///
/// Call [`SdlPlatform::new()`] to initialize the SDL2 library and create
/// the timer.  Graphics are **not** started until [`PlatformHost::init_graphics()`]
/// is called by the game's initialization sequence.
///
/// # Lifecycle
///
/// 1. `SdlPlatform::new()` — SDL2 init, timer start
/// 2. `init_graphics()` — window + video output creation
/// 3. Game loop: `start_frame()` → `start_tic()` → `finish_update()` repeated
/// 4. `shutdown_graphics()` — window teardown
/// 5. `quit()` or `error()` — process exit
///
/// # Thread Safety
///
/// `SdlPlatform` is `!Send` and `!Sync` because SDL2 contexts are
/// thread-local.  All access must occur on the main thread, which matches
/// DOOM's single-threaded game loop architecture.
pub struct SdlPlatform {
    /// The root SDL2 context.  Must outlive all SDL2 subsystems.
    /// Stored as `Option` to allow controlled shutdown ordering.
    sdl_context: Option<sdl2::Sdl>,

    /// SDL2 window and event pump manager.
    /// `None` before `init_graphics()` is called or after
    /// `shutdown_graphics()`.
    window_manager: Option<WindowManager>,

    /// SDL2 canvas/texture video output pipeline.
    /// `None` before `init_graphics()` or after `shutdown_graphics()`.
    video_output: Option<VideoOutput>,

    /// High-resolution monotonic timer (35 tics/second).
    /// Created at construction time and never replaced.
    timer: Timer,

    /// Persistent mouse and keyboard tracking state.
    input_state: InputState,

    /// Per-tic event buffer.  Filled by `start_tic()`, consumed by
    /// the game loop's event dispatcher (`D_ProcessEvents`).
    event_queue: Vec<Event>,

    /// Window scale factor (default 3×).  Used when creating the
    /// SDL2 window in `init_graphics()`.
    window_scale: Option<u32>,
}

impl SdlPlatform {
    /// Create a new SDL2 platform host.
    ///
    /// Initializes the SDL2 library and starts the monotonic timer.
    /// Graphics are NOT started here — call [`PlatformHost::init_graphics()`]
    /// to create the window and video output.
    ///
    /// # Errors
    ///
    /// Returns an error string if SDL2 initialization fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use doom_platform_win::SdlPlatform;
    ///
    /// let platform = SdlPlatform::new(None).expect("SDL2 init failed");
    /// ```
    pub fn new(window_scale: Option<u32>) -> Result<Self, String> {
        let sdl_context = sdl2::init().map_err(|e| format!("Failed to initialize SDL2: {e}"))?;

        info!("SDL2 initialized successfully");

        let timer = Timer::new();

        Ok(Self {
            sdl_context: Some(sdl_context),
            window_manager: None,
            video_output: None,
            timer,
            input_state: InputState::new(),
            event_queue: Vec::with_capacity(64),
            window_scale,
        })
    }

    /// Returns a reference to the per-tic event queue.
    ///
    /// The game loop reads events from this queue after calling
    /// [`PlatformHost::start_tic()`].  The queue is cleared at the
    /// beginning of each `start_tic()` call.
    #[inline]
    pub fn event_queue(&self) -> &[Event] {
        &self.event_queue
    }

    /// Drains and returns all queued events, leaving the internal
    /// buffer empty.
    ///
    /// This is the primary interface for `doom-core`'s game loop to
    /// consume input events after `start_tic()` has populated the queue.
    #[inline]
    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.event_queue)
    }

    /// Returns a reference to the SDL2 context, if still active.
    ///
    /// Useful for creating additional SDL2 subsystems (e.g., audio)
    /// from the same context.
    pub fn sdl_context(&self) -> Option<&sdl2::Sdl> {
        self.sdl_context.as_ref()
    }

    /// Returns a mutable reference to the window manager, if graphics
    /// have been initialized.
    pub fn window_manager_mut(&mut self) -> Option<&mut WindowManager> {
        self.window_manager.as_mut()
    }
}

// =============================================================================
// PlatformHost trait implementation
// =============================================================================

impl PlatformHost for SdlPlatform {
    /// Returns the current time in game tics (35 tics per second).
    ///
    /// Delegates to [`Timer::get_time()`] which computes:
    /// `elapsed_micros * TICRATE / 1_000_000`.
    ///
    /// Equivalent of `I_GetTime()` from `i_system.c:88–100`.
    #[inline]
    fn get_time(&self) -> i32 {
        self.timer.get_time()
    }

    /// Called before processing any tics in a frame.
    ///
    /// Equivalent of `I_StartFrame()` from `i_system.c`.
    /// The original Linux implementation was a no-op.  In the SDL2
    /// backend, this is also a no-op — all input polling occurs in
    /// [`start_tic()`].
    ///
    /// [`start_tic()`]: PlatformHost::start_tic
    fn start_frame(&mut self) {
        // No-op, matching the original i_system.c implementation.
        // Joystick polling (if added later) would go here.
    }

    /// Polls SDL2 input events and translates them to DOOM events.
    ///
    /// Equivalent of `I_StartTic()` from `i_video.c:309–338`.
    /// Drains the SDL2 event pump and converts each event into a DOOM
    /// [`Event`] using [`input::process_sdl_events()`].
    ///
    /// The resulting events are stored in the internal event queue,
    /// accessible via [`event_queue()`] or [`drain_events()`].
    ///
    /// [`event_queue()`]: SdlPlatform::event_queue
    /// [`drain_events()`]: SdlPlatform::drain_events
    fn start_tic(&mut self) {
        // Clear the previous tic's events.
        self.event_queue.clear();

        // Collect SDL2 events from the event pump.
        if let Some(ref mut wm) = self.window_manager {
            let sdl_events: Vec<sdl2::event::Event> = wm.event_pump().poll_iter().collect();

            // Translate SDL2 events to DOOM events.
            let doom_events = input::process_sdl_events(&sdl_events, &mut self.input_state);

            self.event_queue.extend(doom_events);

            debug!(
                "start_tic: polled {} SDL2 events, produced {} DOOM events",
                sdl_events.len(),
                self.event_queue.len()
            );
        } else {
            // Graphics not initialized yet — no events to poll.
            debug!("start_tic: window_manager not available, no events polled");
        }
    }

    /// Initialize the SDL2 window and video output pipeline.
    ///
    /// Equivalent of `I_InitGraphics()` from `i_video.c`.
    /// Creates the SDL2 window via [`WindowManager`], transfers the
    /// window to [`VideoOutput`] for canvas/texture rendering.
    ///
    /// # Panics
    ///
    /// Logs an error and returns early if the SDL2 context is unavailable
    /// or if window/video creation fails.  Does not panic to allow the
    /// game to attempt graceful degradation or exit.
    fn init_graphics(&mut self) {
        let sdl = match self.sdl_context {
            Some(ref ctx) => ctx,
            None => {
                error!("init_graphics: SDL2 context not available");
                return;
            }
        };

        // Create the window manager with the configured scale factor.
        let mut wm = match WindowManager::new(sdl, self.window_scale) {
            Ok(wm) => wm,
            Err(e) => {
                error!("init_graphics: failed to create window: {e:?}");
                return;
            }
        };

        // Transfer the window to VideoOutput which converts it into
        // a hardware-accelerated canvas for rendering.
        let window = match wm.take_window() {
            Some(w) => w,
            None => {
                error!("init_graphics: window already taken from WindowManager");
                return;
            }
        };

        let vo = match VideoOutput::new(window) {
            Ok(vo) => vo,
            Err(e) => {
                error!("init_graphics: failed to create video output: {e}");
                return;
            }
        };

        self.window_manager = Some(wm);
        self.video_output = Some(vo);

        info!(
            "Graphics initialized: {}×{} (scale: {}×)",
            SCREENWIDTH,
            SCREENHEIGHT,
            self.window_scale.unwrap_or(3)
        );
    }

    /// Shut down the SDL2 display subsystem.
    ///
    /// Equivalent of `I_ShutdownGraphics()` from `i_video.c`.
    /// Drops the video output and window manager, releasing all
    /// SDL2 display resources.
    fn shutdown_graphics(&mut self) {
        // Drop video output first (it holds the canvas/window).
        if self.video_output.take().is_some() {
            info!("Video output shut down");
        }
        // Drop window manager (holds event pump and video subsystem).
        if self.window_manager.take().is_some() {
            info!("Window manager shut down");
        }
    }

    /// Present the rendered framebuffer to the SDL2 window.
    ///
    /// Equivalent of `I_FinishUpdate()` from `i_video.c:352–521`.
    /// Delegates to [`VideoOutput::finish_update()`] which converts the
    /// palettized 320×200 framebuffer to ARGB8888 and presents it via
    /// the SDL2 canvas.
    ///
    /// If graphics have not been initialized, the call is silently ignored.
    fn finish_update(&mut self, screen: &[u8]) {
        if let Some(ref mut vo) = self.video_output {
            vo.finish_update(screen);
        } else {
            warn!("finish_update called before init_graphics");
        }
    }

    /// Set the 256-color palette for framebuffer conversion.
    ///
    /// Equivalent of `I_SetPalette()` from `i_video.c`.
    /// Delegates to [`VideoOutput::set_palette()`] which stores the
    /// RGB palette for use during [`finish_update()`].
    ///
    /// [`finish_update()`]: PlatformHost::finish_update
    fn set_palette(&mut self, palette: &[u8]) {
        if let Some(ref mut vo) = self.video_output {
            vo.set_palette(palette);
        } else {
            warn!("set_palette called before init_graphics");
        }
    }

    /// Read back the current screen contents.
    ///
    /// Equivalent of `I_ReadScreen()` from `i_video.c:527–530`.
    ///
    /// **Note**: The original `I_ReadScreen` copies from the internal
    /// `screens[0]` buffer.  In the Rust architecture, the screen buffer
    /// is owned by `doom-core`'s `VideoState`, not by the platform layer.
    /// This implementation zero-fills the buffer because the platform
    /// layer does not have access to the game's screen memory.  Game code
    /// should read `VideoState::screens[0]` directly for screen capture.
    fn read_screen(&self, buffer: &mut [u8]) {
        // The original I_ReadScreen copies screens[0] into the provided
        // buffer.  Since the platform layer does not own the game's
        // screen buffer (it lives in doom-core's VideoState), we
        // zero-fill as the correct safe default.  The game code reads
        // its own screens[0] directly rather than calling I_ReadScreen.
        let len = SCREEN_SIZE.min(buffer.len());
        buffer[..len].fill(0);
        debug!(
            "read_screen: zero-filled {} bytes (screen data owned by VideoState)",
            len
        );
    }

    /// Return a default (empty) tic command.
    ///
    /// Equivalent of `I_BaseTiccmd()` from `i_system.c`.
    /// Returns a zeroed [`TicCmd`] — the game loop modifies it with
    /// input from the event queue.
    #[inline]
    fn base_ticcmd(&self) -> TicCmd {
        TicCmd::default()
    }

    /// Perform a clean exit from the application.
    ///
    /// Equivalent of `I_Quit()` from `i_system.c`.
    /// Shuts down graphics, logs the exit, and terminates the process
    /// with exit code 0.
    fn quit(&self) {
        info!("I_Quit: shutting down DOOM");
        // Note: In a full implementation, the caller would invoke
        // D_QuitNetGame(), I_ShutdownSound(), I_ShutdownMusic(),
        // and M_SaveDefaults() before calling this method.
        // shutdown_graphics() requires &mut self, so the caller should
        // invoke it separately before calling quit().
        std::process::exit(0);
    }

    /// Report a fatal error and terminate the application.
    ///
    /// Equivalent of `I_Error()` from `i_system.c`.
    /// Logs the error message via `tracing::error!` and terminates
    /// the process with exit code 1.
    ///
    /// This function never returns (`-> !`).
    fn error(&self, msg: &str) -> ! {
        error!("I_Error: {}", msg);
        eprintln!("Error: {msg}");
        std::process::exit(1);
    }
}
