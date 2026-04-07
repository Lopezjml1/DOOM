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

//! Translated from linuxdoom-1.10/i_video.c — video output subsystem.
//!
//! Replaces the X11 XImage / MIT-SHM rendering pipeline with SDL2-based
//! texture streaming. The DOOM engine renders into a palettized 320×200
//! byte buffer; this module converts that buffer through a 256-entry
//! palette lookup into ARGB8888 pixel data, uploads it to an SDL2
//! streaming texture, and presents it to the window canvas.
//!
//! # Rendering pipeline
//!
//! 1. DOOM core writes palette indices (0-255) into a 320×200 `[u8]`
//!    framebuffer (`screens[0]`).
//! 2. [`VideoOutput::set_palette`] builds a 256-entry `[u32; 256]`
//!    lookup table mapping each palette index to an ARGB8888 colour.
//! 3. [`VideoOutput::finish_update`] converts the palettized buffer
//!    to ARGB via the lookup table, uploads to a streaming texture,
//!    and copies the texture to the SDL2 canvas.
//! 4. SDL2 handles scaling from 320×200 to the actual window size.

use sdl2::pixels::PixelFormatEnum;
use sdl2::render::{Canvas, TextureCreator};
use sdl2::video::{Window, WindowContext};
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Constants — match linuxdoom-1.10/doomdef.h
// ---------------------------------------------------------------------------

/// Native render width in pixels.
pub const SCREENWIDTH: usize = 320;

/// Native render height in pixels.
pub const SCREENHEIGHT: usize = 200;

/// Total number of pixels in the DOOM framebuffer.
const SCREEN_SIZE: usize = SCREENWIDTH * SCREENHEIGHT;

/// Size of a raw palette in bytes (256 colours × 3 channels).
const PALETTE_SIZE: usize = 256 * 3;

// ---------------------------------------------------------------------------
// VideoOutput — SDL2 rendering surface
// ---------------------------------------------------------------------------

/// Video output surface backed by an SDL2 canvas and streaming texture.
///
/// Owns the SDL2 [`Canvas<Window>`] and a [`TextureCreator`] used to build
/// the per-frame streaming texture. Maintains a palette lookup table
/// (`palette_rgb`) and a reusable ARGB pixel buffer (`rgb_buffer`) to
/// avoid per-frame allocation.
pub struct VideoOutput {
    /// SDL2 rendering canvas (owns the window).
    canvas: Canvas<Window>,

    /// Factory for creating streaming textures.
    texture_creator: TextureCreator<WindowContext>,

    /// Current palette: 256 ARGB8888 values (0xAA_RR_GG_BB, alpha=0xFF).
    palette_rgb: [u32; 256],

    /// Reusable ARGB pixel buffer (SCREEN_SIZE entries).
    rgb_buffer: Vec<u32>,
}

impl VideoOutput {
    /// Create a new [`VideoOutput`] by converting the given SDL2 [`Window`]
    /// into a hardware-accelerated canvas.
    ///
    /// The canvas is configured with:
    /// - V-Sync enabled (`.present_vsync()`)
    /// - Logical size set to 320×200 so SDL2 handles upscaling automatically
    ///
    /// # Errors
    ///
    /// Returns a human-readable error string if the canvas or texture
    /// creator cannot be initialised.
    pub fn new(window: Window) -> Result<Self, String> {
        let mut canvas = window
            .into_canvas()
            .present_vsync()
            .build()
            .map_err(|e| format!("Failed to create SDL2 canvas: {e}"))?;

        canvas
            .set_logical_size(SCREENWIDTH as u32, SCREENHEIGHT as u32)
            .map_err(|e| format!("Failed to set logical size: {e}"))?;

        let texture_creator = canvas.texture_creator();

        info!(
            "Video output initialised: {}×{} logical, canvas ready",
            SCREENWIDTH, SCREENHEIGHT
        );

        Ok(Self {
            canvas,
            texture_creator,
            palette_rgb: [0xFF_00_00_00; 256], // opaque black default
            rgb_buffer: vec![0xFF_00_00_00; SCREEN_SIZE],
        })
    }

    /// Set the 256-colour palette used to convert palettized framebuffer
    /// data to ARGB8888.
    ///
    /// `palette` must contain at least [`PALETTE_SIZE`] bytes (768): three
    /// bytes (R, G, B) per entry, matching the PLAYPAL lump format from
    /// the WAD file. Gamma correction is expected to have been applied by
    /// the caller (V_Video) before invoking this method.
    ///
    /// # Panics
    ///
    /// Panics if `palette.len() < PALETTE_SIZE`.
    pub fn set_palette(&mut self, palette: &[u8]) {
        assert!(
            palette.len() >= PALETTE_SIZE,
            "Palette buffer too small: expected {} bytes, got {}",
            PALETTE_SIZE,
            palette.len()
        );
        for i in 0..256 {
            let r = palette[i * 3] as u32;
            let g = palette[i * 3 + 1] as u32;
            let b = palette[i * 3 + 2] as u32;
            self.palette_rgb[i] = 0xFF_00_00_00 | (r << 16) | (g << 8) | b;
        }
        debug!(
            "Palette updated (first entry: #{:06X})",
            self.palette_rgb[0] & 0x00_FF_FF_FF
        );
    }

    /// Blit the palettized DOOM framebuffer to the window.
    ///
    /// `screen` is the 320×200 palettized byte buffer (`screens[0]`).
    /// Each byte is a palette index (0-255) that is converted to ARGB8888
    /// through the current palette lookup table, uploaded to a streaming
    /// SDL2 texture, and presented to the canvas.
    ///
    /// # Panics
    ///
    /// Panics if `screen.len() < SCREEN_SIZE`.
    pub fn finish_update(&mut self, screen: &[u8]) {
        assert!(
            screen.len() >= SCREEN_SIZE,
            "Screen buffer too small: expected {} bytes, got {}",
            SCREEN_SIZE,
            screen.len()
        );

        // --- Convert palettized pixels to ARGB8888 ---
        for (dst, &src) in self.rgb_buffer[..SCREEN_SIZE]
            .iter_mut()
            .zip(&screen[..SCREEN_SIZE])
        {
            *dst = self.palette_rgb[src as usize];
        }

        // --- Upload to streaming texture and present ---
        //
        // A new streaming texture is created per frame. For 320×200 at 35
        // fps this is negligible overhead and avoids self-referential
        // lifetime issues with SDL2's Texture<'a> tied to TextureCreator.
        let mut texture = self
            .texture_creator
            .create_texture_streaming(
                PixelFormatEnum::ARGB8888,
                SCREENWIDTH as u32,
                SCREENHEIGHT as u32,
            )
            .expect("Failed to create streaming texture");

        // SAFETY: Reinterpreting &[u32] as &[u8] is safe because:
        //   - u32 has no padding bytes
        //   - u8 alignment (1) divides u32 alignment (4)
        //   - The data is fully initialised and we only read it
        let pixel_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                self.rgb_buffer.as_ptr() as *const u8,
                SCREEN_SIZE * std::mem::size_of::<u32>(),
            )
        };

        let pitch = SCREENWIDTH * std::mem::size_of::<u32>();
        texture
            .update(None, pixel_bytes, pitch)
            .expect("Failed to update streaming texture");

        self.canvas
            .copy(&texture, None, None)
            .expect("Failed to copy texture to canvas");

        self.canvas.present();
    }

    /// Read back the palettized screen buffer.
    ///
    /// This is the Rust equivalent of the original C `I_ReadScreen` which
    /// simply copies the raw framebuffer. The caller passes the current
    /// palettized screen and a destination buffer; the data is copied as-is
    /// (no ARGB conversion). Used by the screenshot routine.
    ///
    /// Copies `min(screen.len(), buffer.len(), SCREEN_SIZE)` bytes.
    pub fn read_screen(screen: &[u8], buffer: &mut [u8]) {
        let len = SCREEN_SIZE.min(screen.len()).min(buffer.len());
        buffer[..len].copy_from_slice(&screen[..len]);
    }

    /// Get a reference to the underlying SDL2 canvas.
    pub fn canvas(&self) -> &Canvas<Window> {
        &self.canvas
    }

    /// Get a mutable reference to the underlying SDL2 canvas.
    pub fn canvas_mut(&mut self) -> &mut Canvas<Window> {
        &mut self.canvas
    }
}
