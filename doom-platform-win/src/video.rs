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

//! Video output — SDL2 Canvas and Texture for 320×200 → window scaling.
//!
//! Translated from `linuxdoom-1.10/i_video.c` (lines 352-585).
//! Replaces X11 XImage/MIT-SHM pixel blitting with SDL2 texture streaming
//! and XStoreColors palette management with in-memory RGB palette mapping.
//!
//! ## Rendering Pipeline
//! 1. Game renders to `screens[0]` (320×200 palettized framebuffer, 1 byte/pixel)
//! 2. [`VideoOutput::finish_update`] maps each palette index to RGB using current
//!    palette lookup table
//! 3. RGB pixels written to SDL2 streaming texture via [`sdl2::render::Texture::update`]
//! 4. Texture presented to window via SDL2 [`Canvas`] with hardware scaling
//!
//! ## Key Design Decisions
//! - **Palette indirection**: The 256-entry `palette_rgb` lookup table converts
//!   8-bit palette indices to ARGB8888 in a single array lookup per pixel.
//!   Gamma correction is applied upstream by `doom-core`'s `v_video` module
//!   *before* calling [`VideoOutput::set_palette`], so this module receives
//!   already-corrected RGB values — matching the division of responsibility
//!   in the original C code where `UploadNewPalette` applied `gammatable`.
//! - **Per-frame texture creation**: A new SDL2 streaming texture is allocated
//!   each frame.  At 320×200 × 4 bytes × 35 fps ≈ 9 MB/s throughput, this is
//!   negligible overhead and avoids self-referential lifetime issues with
//!   SDL2's `Texture<'a>` being tied to the `TextureCreator`.
//! - **No manual pixel scaling**: The original `i_video.c` contained elaborate
//!   2×, 3×, and 4× pixel-doubling loops (lines 376-481).  SDL2's
//!   [`Canvas::set_logical_size`] handles arbitrary upscaling to the window
//!   dimensions automatically, eliminating all manual scaling code.
//!
//! ## AAP References
//! - IR-08: MIT-SHM replaced by SDL2 texture streaming — no shared memory needed
//! - §0.2.4: Replaces X11 XImage/MIT-SHM and XStoreColors

use sdl2::pixels::PixelFormatEnum;
use sdl2::render::{Canvas, TextureCreator};
use sdl2::video::{Window, WindowContext};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Constants — match linuxdoom-1.10/doomdef.h
// ---------------------------------------------------------------------------

/// Native render width in pixels (SCREENWIDTH from doomdef.h).
pub const SCREENWIDTH: u32 = 320;

/// Native render height in pixels (SCREENHEIGHT from doomdef.h).
pub const SCREENHEIGHT: u32 = 200;

/// Total number of pixels in the DOOM framebuffer (320 × 200 = 64 000).
const SCREEN_SIZE: usize = (SCREENWIDTH as usize) * (SCREENHEIGHT as usize);

/// Size of a raw palette in bytes: 256 colours × 3 channels (R, G, B) = 768.
const PALETTE_SIZE: usize = 256 * 3;

/// Bytes per pixel in the ARGB8888 pixel format used by the streaming texture.
const BYTES_PER_PIXEL: usize = 4;

// ---------------------------------------------------------------------------
// VideoOutput — SDL2 rendering surface
// ---------------------------------------------------------------------------

/// Video output surface backed by an SDL2 [`Canvas`] and streaming texture.
///
/// Owns the SDL2 [`Canvas<Window>`] and a [`TextureCreator`] used to build
/// the per-frame streaming texture.  Maintains a palette lookup table
/// (`palette_rgb`) and a reusable ARGB pixel buffer (`rgb_buffer`) to
/// avoid per-frame allocation.
///
/// ## Replaces
/// - `XColor colors[256]` array from `i_video.c:536` → [`palette_rgb`](Self::palette_rgb)
/// - `XShmPutImage` / `XPutImage` blitting → [`finish_update`](Self::finish_update)
/// - `XStoreColors` palette upload → [`set_palette`](Self::set_palette)
/// - `I_ReadScreen` memcpy → [`read_screen`](Self::read_screen)
pub struct VideoOutput {
    /// SDL2 rendering canvas (owns the window).
    canvas: Canvas<Window>,

    /// Factory for creating streaming textures.  Must outlive every
    /// [`Texture`] it creates, which is satisfied because textures are
    /// local to [`finish_update`](Self::finish_update).
    texture_creator: TextureCreator<WindowContext>,

    /// Current 256-colour palette mapped to ARGB8888.
    ///
    /// Each entry is a packed `u32`: `0xFF_RR_GG_BB` (alpha = 0xFF, fully
    /// opaque).  Initialised to opaque black before the first `PLAYPAL`
    /// lump is loaded.
    palette_rgb: [u32; 256],

    /// Reusable byte buffer for the ARGB8888 framebuffer.
    ///
    /// Length is always `SCREEN_SIZE * BYTES_PER_PIXEL` (256 000 bytes).
    /// Using `Vec<u8>` instead of `Vec<u32>` avoids the need for any
    /// `unsafe` reinterpretation when passing to [`Texture::update`].
    rgb_buffer: Vec<u8>,
}

impl VideoOutput {
    /// Create a new [`VideoOutput`] by converting the given SDL2 [`Window`]
    /// into a hardware-accelerated canvas.
    ///
    /// The canvas is configured with:
    /// - V-Sync enabled (`.present_vsync()`) to prevent screen tearing
    /// - Logical size set to 320×200 so SDL2 handles upscaling automatically
    ///
    /// # Errors
    ///
    /// Returns a human-readable error string if the canvas or texture
    /// creator cannot be initialised.
    pub fn new(window: Window) -> Result<Self, String> {
        // Build the hardware-accelerated canvas from the SDL2 window.
        // `present_vsync()` synchronises presentation with the display
        // refresh rate, eliminating tearing without manual timing.
        // Replaces X11 `XCreateGC` / `XMapWindow` setup from i_video.c.
        let mut canvas = window
            .into_canvas()
            .present_vsync()
            .build()
            .map_err(|e| format!("Failed to create SDL2 canvas: {e}"))?;

        // Set the logical rendering size to DOOM's native resolution.
        // SDL2 automatically scales the 320×200 output to fill the window,
        // letterboxing if needed.  This replaces the manual 2×/3×/4×
        // pixel-doubling loops from i_video.c:376-481.
        canvas
            .set_logical_size(SCREENWIDTH, SCREENHEIGHT)
            .map_err(|e| format!("Failed to set logical size: {e}"))?;

        let texture_creator = canvas.texture_creator();

        info!(
            "Video output initialised: {}×{} logical, canvas ready",
            SCREENWIDTH, SCREENHEIGHT
        );

        Ok(Self {
            canvas,
            texture_creator,
            // Opaque black — the game will call set_palette() with PLAYPAL
            // data before the first frame is rendered.
            palette_rgb: [0xFF_00_00_00; 256],
            rgb_buffer: vec![0u8; SCREEN_SIZE * BYTES_PER_PIXEL],
        })
    }

    /// Set the 256-colour palette used to convert palettized framebuffer
    /// data to ARGB8888.
    ///
    /// `palette` must contain at least [`PALETTE_SIZE`] bytes (768): three
    /// bytes (R, G, B) per entry, matching the PLAYPAL lump format from
    /// the WAD file.
    ///
    /// Gamma correction is expected to have been applied by the caller
    /// (`doom-core`'s `v_video` module) before invoking this method — this
    /// mirrors the original architecture where `UploadNewPalette`
    /// (`i_video.c:538`) applied `gammatable[usegamma]` inline.
    ///
    /// If the provided slice is shorter than [`PALETTE_SIZE`] bytes the
    /// palette is *not* updated and a warning is logged.
    pub fn set_palette(&mut self, palette: &[u8]) {
        if palette.len() < PALETTE_SIZE {
            warn!(
                "Palette buffer too small: expected at least {} bytes, got {} — palette not updated",
                PALETTE_SIZE,
                palette.len()
            );
            return;
        }

        // Map each RGB triplet to a packed ARGB8888 u32.
        // Layout: 0xAA_RR_GG_BB  with alpha = 0xFF (fully opaque).
        // This replaces the X11 XColor construction in
        // UploadNewPalette (i_video.c:563-571).
        for i in 0..256 {
            let base = i * 3;
            let r = palette[base] as u32;
            let g = palette[base + 1] as u32;
            let b = palette[base + 2] as u32;
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
    /// This is the **performance-critical path** — called 35 times per
    /// second during gameplay.  The hot loop is a single pass over 64 000
    /// pixels with one palette lookup and one 4-byte copy per pixel.
    ///
    /// If the screen buffer is too small the frame is silently skipped
    /// (a warning is logged).
    ///
    /// **Replaces**: `I_FinishUpdate()` from `i_video.c` lines 352-521.
    pub fn finish_update(&mut self, screen: &[u8]) {
        if screen.len() < SCREEN_SIZE {
            warn!(
                "Screen buffer too small for finish_update: \
                 expected {} bytes, got {} — frame skipped",
                SCREEN_SIZE,
                screen.len()
            );
            return;
        }

        // -----------------------------------------------------------------
        // Step 1: Convert palettized pixels (1 byte each) to ARGB8888
        //         (4 bytes each) using the palette lookup table.
        // -----------------------------------------------------------------
        for (i, &pixel_index) in screen[..SCREEN_SIZE].iter().enumerate() {
            let argb = self.palette_rgb[pixel_index as usize];
            let offset = i * BYTES_PER_PIXEL;
            // Write the u32 pixel value into the byte buffer using native
            // endianness.  SDL2's ARGB8888 pixel format interprets data as
            // 32-bit words in the host's native byte order, so
            // `to_ne_bytes()` is the correct serialisation.
            self.rgb_buffer[offset..offset + BYTES_PER_PIXEL].copy_from_slice(&argb.to_ne_bytes());
        }

        // -----------------------------------------------------------------
        // Step 2: Create a streaming texture and upload the pixel data.
        // -----------------------------------------------------------------
        let mut texture = match self.texture_creator.create_texture_streaming(
            PixelFormatEnum::ARGB8888,
            SCREENWIDTH,
            SCREENHEIGHT,
        ) {
            Ok(tex) => tex,
            Err(e) => {
                warn!("Failed to create streaming texture: {e} — frame skipped");
                return;
            }
        };

        let pitch = SCREENWIDTH as usize * BYTES_PER_PIXEL;
        if let Err(e) = texture.update(None, &self.rgb_buffer, pitch) {
            warn!("Failed to update streaming texture: {e} — frame skipped");
            return;
        }

        // -----------------------------------------------------------------
        // Step 3: Clear the canvas, copy the texture, and present.
        //
        // `canvas.clear()` fills the back-buffer with the current draw
        // colour (black by default), ensuring no stale content is visible
        // in letterbox bands when the window aspect ratio doesn't match
        // 320×200.  `canvas.present()` swaps the front and back buffers.
        // -----------------------------------------------------------------
        self.canvas.clear();

        if let Err(e) = self.canvas.copy(&texture, None, None) {
            warn!("Failed to copy texture to canvas: {e} — frame skipped");
            return;
        }

        self.canvas.present();
    }

    /// Read back the palettized screen buffer.
    ///
    /// This is the Rust equivalent of `I_ReadScreen()` from
    /// `i_video.c:527-530`, which simply copies the raw palettized
    /// framebuffer (`screens[0]`) into the caller's destination buffer.
    /// The data is copied as-is (no ARGB conversion); the screenshot
    /// routine uses this to capture the current frame state.
    ///
    /// Copies `min(screen.len(), buffer.len(), SCREEN_SIZE)` bytes.
    pub fn read_screen(&self, screen: &[u8], buffer: &mut [u8]) {
        let len = SCREEN_SIZE.min(screen.len()).min(buffer.len());
        buffer[..len].copy_from_slice(&screen[..len]);
    }

    /// Get a shared reference to the underlying SDL2 canvas.
    ///
    /// Useful for querying window properties (title, size, display index)
    /// from other modules in the `doom-platform-win` crate.
    pub fn canvas(&self) -> &Canvas<Window> {
        &self.canvas
    }

    /// Get a mutable reference to the underlying SDL2 canvas.
    ///
    /// Exposed for cases where other platform modules need direct canvas
    /// access (e.g. changing the draw colour for debug overlays).
    pub fn canvas_mut(&mut self) -> &mut Canvas<Window> {
        &mut self.canvas
    }
}
