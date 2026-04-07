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

//! Translated from linuxdoom-1.10/i_video.c — SDL2 window creation and
//! event pump management.
//!
//! Replaces the X11/Xlib window initialisation (`XCreateWindow`,
//! `XSelectInput`, MIT-SHM setup) and the X11 event handling
//! (`XNextEvent` / `XPending`) with an SDL2 window and event pump.
//!
//! [`WindowManager`] is responsible for:
//! - Creating the SDL2 video subsystem and opening a resizable window at
//!   the specified scale (default 3×, so 960×600 for DOOM's 320×200).
//! - Providing access to the [`sdl2::EventPump`] so the platform layer
//!   can poll input events and translate them to DOOM [`Event`]s.
//! - Managing the mouse grab state for in-game capture.
//!
//! The window itself is eventually transferred to [`super::video::VideoOutput`]
//! which converts it into an SDL2 canvas for rendering.

use sdl2::video::Window;
use sdl2::{EventPump, Sdl, VideoSubsystem};
use thiserror::Error;
use tracing::info;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Native DOOM framebuffer width.
pub const SCREENWIDTH: u32 = 320;

/// Native DOOM framebuffer height.
pub const SCREENHEIGHT: u32 = 200;

/// Default window scale factor (3× = 960×600).
pub const DEFAULT_SCALE: u32 = 3;

/// Window title.
pub const WINDOW_TITLE: &str = "DOOM";

// ---------------------------------------------------------------------------
// WindowError
// ---------------------------------------------------------------------------

/// Errors that can occur during window creation.
#[derive(Debug, Error)]
pub enum WindowError {
    /// SDL2 video subsystem initialisation failed.
    #[error("SDL2 video init failed: {0}")]
    SdlInit(String),

    /// Window creation failed.
    #[error("Window creation failed: {0}")]
    WindowCreation(String),

    /// Event pump creation failed.
    #[error("Event pump creation failed: {0}")]
    EventPump(String),
}

// ---------------------------------------------------------------------------
// WindowManager
// ---------------------------------------------------------------------------

/// Manages the SDL2 window lifecycle and event pump.
///
/// After construction the window can be transferred to a
/// [`super::video::VideoOutput`] via [`WindowManager::take_window`], which
/// converts it into a rendering canvas. The event pump and video subsystem
/// remain owned by this manager for the lifetime of the application.
pub struct WindowManager {
    /// The SDL2 window, wrapped in [`Option`] so it can be transferred
    /// to the video output layer via [`Self::take_window`].
    window: Option<Window>,

    /// SDL2 event pump — the only way to receive input events.
    event_pump: EventPump,

    /// SDL2 video subsystem handle (must outlive the window).
    video_subsystem: VideoSubsystem,

    /// Current scale factor.
    scale: u32,

    /// Whether the mouse is grabbed (confined to the window).
    grab_mouse: bool,
}

impl WindowManager {
    /// Create a new SDL2 window and event pump.
    ///
    /// `scale` controls the window size multiplier. `None` or `Some(0)`
    /// defaults to [`DEFAULT_SCALE`] (3×).
    ///
    /// # Errors
    ///
    /// Returns [`WindowError`] if the video subsystem, window, or event
    /// pump cannot be initialised.
    pub fn new(sdl_context: &Sdl, scale: Option<u32>) -> Result<Self, WindowError> {
        let scale = match scale {
            Some(s) if s > 0 => s,
            _ => DEFAULT_SCALE,
        };

        let video_subsystem = sdl_context.video().map_err(WindowError::SdlInit)?;

        let width = SCREENWIDTH * scale;
        let height = SCREENHEIGHT * scale;

        let window = video_subsystem
            .window(WINDOW_TITLE, width, height)
            .position_centered()
            .resizable()
            .build()
            .map_err(|e| WindowError::WindowCreation(e.to_string()))?;

        let event_pump = sdl_context.event_pump().map_err(WindowError::EventPump)?;

        info!(
            "Window created: \"{}\" {}×{} ({}× scale)",
            WINDOW_TITLE, width, height, scale
        );

        Ok(Self {
            window: Some(window),
            event_pump,
            video_subsystem,
            scale,
            grab_mouse: false,
        })
    }

    /// Borrow the SDL2 window.
    ///
    /// # Panics
    ///
    /// Panics if the window has already been transferred via
    /// [`Self::take_window`].
    pub fn window(&self) -> &Window {
        self.window.as_ref().expect("Window already taken")
    }

    /// Mutably borrow the SDL2 window.
    ///
    /// # Panics
    ///
    /// Panics if the window has already been transferred via
    /// [`Self::take_window`].
    pub fn window_mut(&mut self) -> &mut Window {
        self.window.as_mut().expect("Window already taken")
    }

    /// Transfer ownership of the SDL2 window to the caller.
    ///
    /// This is used to pass the window to
    /// [`super::video::VideoOutput::new`] which converts it into a
    /// rendering canvas. After this call, [`Self::window`] and
    /// [`Self::window_mut`] will panic.
    ///
    /// Returns `None` if the window has already been taken.
    pub fn take_window(&mut self) -> Option<Window> {
        let w = self.window.take();
        if w.is_some() {
            info!("Window transferred to video output");
        }
        w
    }

    /// Get a mutable reference to the SDL2 event pump for polling events.
    pub fn event_pump(&mut self) -> &mut EventPump {
        &mut self.event_pump
    }

    /// Get a reference to the SDL2 video subsystem.
    pub fn video_subsystem(&self) -> &VideoSubsystem {
        &self.video_subsystem
    }

    /// Set the mouse grab state.
    ///
    /// When grabbed, the mouse cursor is confined to the window and hidden,
    /// providing the in-game mouse look experience.
    pub fn set_grab_mouse(&mut self, grab: bool) {
        self.grab_mouse = grab;
        if let Some(ref mut window) = self.window {
            window.set_grab(grab);
            self.video_subsystem.sdl().mouse().show_cursor(!grab);
        }
    }

    /// Returns `true` if the mouse is currently grabbed.
    pub fn grab_mouse(&self) -> bool {
        self.grab_mouse
    }

    /// Get the current window scale factor.
    pub fn scale(&self) -> u32 {
        self.scale
    }
}
