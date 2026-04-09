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

//! Mission begin melt/wipe screen special effect.
//!
//! Translated from linuxdoom-1.10/f_wipe.c and f_wipe.h
//!
//! This module implements the screen wipe transition effects used between game
//! state changes (e.g., entering a new level). Two wipe types are provided:
//!
//! - **Color crossfade** (`WipeType::ColorXForm`): Gradually interpolates each
//!   pixel from the start screen value toward the end screen value.
//! - **Melt** (`WipeType::Melt`): The iconic DOOM screen melt where columns of
//!   the old screen slide down at varying speeds to reveal the new screen
//!   underneath.
//!
//! # Screen Wipe Package
//!
//! The wipe system captures two screen states: the "start" screen (before the
//! transition) and the "end" screen (after the transition), then animates
//! between them using one of the available wipe types. The animation operates
//! on a working buffer (`wipe_scr`) which is copied back to the display buffer
//! (`screens[0]`) after each frame.
//!
//! # Usage Flow
//!
//! 1. Call [`WipeState::start_screen`] to capture the current display (old state).
//! 2. The game renders the new state to `screens[0]`.
//! 3. Call [`WipeState::end_screen`] to capture the new display and restore old.
//! 4. Loop calling [`WipeState::screen_wipe`] until it returns 1 (complete),
//!    calling `I_FinishUpdate()` after each step to display the wipe frame.

use crate::types::doomdef::{SCREENHEIGHT, SCREENWIDTH};
use crate::util::random::DoomRandom;
use crate::video::video::VideoState;

/// Standard screen buffer size in bytes (`SCREENWIDTH * SCREENHEIGHT`).
/// Used for debug assertions to validate buffer dimensions.
const SCREEN_SIZE: usize = (SCREENWIDTH * SCREENHEIGHT) as usize;

// =============================================================================
// Wipe Type Enum (from f_wipe.h lines 30–39)
// =============================================================================

/// Screen wipe transition types.
///
/// Translated from the anonymous enum in f_wipe.h lines 30–39.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum WipeType {
    /// Simple gradual pixel change for 8-bit palettized mode.
    /// Each pixel value is incremented or decremented toward the target
    /// by `ticks` units per frame.
    ColorXForm = 0,
    /// Iconic screen melt effect — columns slide down at varying speeds,
    /// revealing the new screen underneath. Uses randomized initial column
    /// offsets and an acceleration/constant-speed pattern.
    Melt = 1,
}

/// Total number of available wipe types.
///
/// Corresponds to `wipe_NUMWIPES` in f_wipe.h line 38.
pub const WIPE_NUMWIPES: usize = 2;

// =============================================================================
// Wipe State (consolidates globals from f_wipe.c)
// =============================================================================

/// Screen wipe transition state.
///
/// Consolidates all formerly-global variables from f_wipe.c (`go`,
/// `wipe_scr_start`, `wipe_scr_end`, `wipe_scr`, `y`) into a single owned
/// struct per the Rust port's global-state-elimination strategy (AAP §0.7.5).
///
/// All screen buffers are `Vec<u8>` with `SCREENWIDTH * SCREENHEIGHT` bytes of
/// palettized pixel data. The `y` array has one `i32` entry per column for the
/// melt effect.
pub struct WipeState {
    /// Whether a wipe is currently in progress. When `false`, the next call
    /// to [`screen_wipe`](Self::screen_wipe) will initialize a new wipe.
    /// Corresponds to the `static boolean go` in f_wipe.c line 43.
    pub go: bool,

    /// Start screen buffer — captured before the game state transition.
    /// During melt wipe, this is transformed to column-major layout.
    /// Corresponds to `static byte* wipe_scr_start` in f_wipe.c line 45.
    pub wipe_scr_start: Vec<u8>,

    /// End screen buffer — captured after the game state transition.
    /// During melt wipe, this is transformed to column-major layout.
    /// Corresponds to `static byte* wipe_scr_end` in f_wipe.c line 46.
    pub wipe_scr_end: Vec<u8>,

    /// Working screen buffer — modified frame-by-frame during the wipe
    /// animation. After each step, this is copied back to `screens[0]`
    /// for display. Corresponds to `static byte* wipe_scr` in f_wipe.c
    /// line 47, which originally pointed directly to `screens[0]`.
    pub wipe_scr: Vec<u8>,

    /// Per-column melt position array. Each element tracks how far down
    /// that column has scrolled. Negative values indicate a delay before
    /// scrolling begins. Only used for [`WipeType::Melt`].
    /// Corresponds to `static int* y` in f_wipe.c line 138.
    pub y: Vec<i32>,
}

impl WipeState {
    /// Create a new wipe state with empty buffers and `go` set to `false`.
    ///
    /// Pre-allocates screen buffers with capacity for the standard screen
    /// size (`SCREENWIDTH * SCREENHEIGHT`) to avoid runtime reallocations
    /// during wipe transitions.
    pub fn new() -> Self {
        WipeState {
            go: false,
            wipe_scr_start: Vec::with_capacity(SCREEN_SIZE),
            wipe_scr_end: Vec::with_capacity(SCREEN_SIZE),
            wipe_scr: Vec::with_capacity(SCREEN_SIZE),
            y: Vec::new(),
        }
    }

    /// Capture the current display as the wipe start screen.
    ///
    /// Equivalent to `wipe_StartScreen` in f_wipe.c lines 236–246.
    /// Reads the current contents of `screens[0]` (the primary display
    /// buffer) into `wipe_scr_start`. This should be called BEFORE the
    /// game renders the new state.
    ///
    /// In the original C code, this sets `wipe_scr_start = screens[2]` and
    /// calls `I_ReadScreen(wipe_scr_start)` to copy the framebuffer. In
    /// the Rust port, we copy `screens[0]` directly into our owned Vec.
    ///
    /// Returns 0 (always succeeds).
    pub fn start_screen(
        &mut self,
        video: &VideoState,
        _x: i32,
        _y: i32,
        width: i32,
        height: i32,
    ) -> i32 {
        let size = (width * height) as usize;
        self.wipe_scr_start = video.screens[0][..size].to_vec();
        0
    }

    /// Capture the current display as the wipe end screen and restore the
    /// start screen to the display buffer.
    ///
    /// Equivalent to `wipe_EndScreen` in f_wipe.c lines 248–259.
    /// Reads the current contents of `screens[0]` (which now holds the
    /// newly-rendered game state) into `wipe_scr_end`, then restores the
    /// start screen to `screens[0]` via `V_DrawBlock` so that the wipe
    /// animation starts from the old screen.
    ///
    /// Returns 0 (always succeeds).
    pub fn end_screen(
        &mut self,
        video: &mut VideoState,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> i32 {
        let size = (width * height) as usize;
        self.wipe_scr_end = video.screens[0][..size].to_vec();
        // Restore start screen to display buffer (V_DrawBlock equivalent).
        // We clone to avoid borrow conflict with video.
        let start_copy = self.wipe_scr_start.clone();
        video.draw_block(x, y, 0, width, height, &start_copy);
        0
    }

    /// Execute one frame of the screen wipe animation.
    ///
    /// Equivalent to `wipe_ScreenWipe` in f_wipe.c lines 261–302.
    ///
    /// On the first call (when `go` is `false`), initializes the wipe by
    /// capturing the working buffer from `screens[0]` and calling the
    /// appropriate init function. On each call, executes one animation step
    /// via the do function. When the animation completes, calls the exit
    /// function and resets `go` to `false`.
    ///
    /// The working buffer is copied back to `screens[0]` after each step
    /// so the caller can display it via `I_FinishUpdate`.
    ///
    /// # Arguments
    ///
    /// * `video` — Video state for screen buffer access and dirty rect marking.
    /// * `rng` — Random number generator (used for melt column offsets on init).
    /// * `wipeno` — Which wipe type to use.
    /// * `x`, `y` — Top-left corner of the wipe region (typically 0, 0).
    /// * `width`, `height` — Dimensions of the wipe region.
    /// * `ticks` — Number of simulation ticks to advance this frame.
    ///
    /// # Returns
    ///
    /// 1 when the wipe is complete, 0 when still in progress.
    pub fn screen_wipe(
        &mut self,
        video: &mut VideoState,
        rng: &mut DoomRandom,
        wipeno: WipeType,
        _x: i32,
        _y: i32,
        width: i32,
        height: i32,
        ticks: i32,
    ) -> i32 {
        let size = (width * height) as usize;

        // Initial setup — first call for this wipe sequence
        if !self.go {
            self.go = true;
            // Capture working buffer from screens[0]
            // (In the original C: wipe_scr = screens[0])
            self.wipe_scr = video.screens[0][..size].to_vec();
            match wipeno {
                WipeType::ColorXForm => {
                    wipe_init_color_xform(self, width, height, ticks);
                }
                WipeType::Melt => {
                    wipe_init_melt(self, rng, width, height, ticks);
                }
            }
        }

        // Mark the entire wipe region as dirty for display update.
        // Equivalent to V_MarkRect(0, 0, width, height) in the original.
        video.mark_rect(0, 0, width, height);

        // Execute one animation step
        let rc = match wipeno {
            WipeType::ColorXForm => wipe_do_color_xform(self, width, height, ticks),
            WipeType::Melt => wipe_do_melt(self, width, height, ticks),
        };

        // Copy working buffer back to screens[0] for display.
        // (In the original C, wipe_scr IS screens[0], so this was implicit.)
        video.screens[0][..size].copy_from_slice(&self.wipe_scr[..size]);

        // Final cleanup if the wipe animation has completed
        if rc != 0 {
            self.go = false;
            match wipeno {
                WipeType::ColorXForm => {
                    wipe_exit_color_xform(self, width, height, ticks);
                }
                WipeType::Melt => {
                    wipe_exit_melt(self, width, height, ticks);
                }
            }
        }

        // Return !go: 1 when complete, 0 when still in progress
        i32::from(!self.go)
    }
}

impl Default for WipeState {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Internal helpers — i16 (short) buffer access
// =============================================================================
//
// The melt wipe treats screen buffers as arrays of `i16` (short) values,
// processing two bytes (two pixels) at a time. These helpers provide safe
// access to the underlying `u8` buffer as native-endian `i16` values.

/// Read an `i16` value from `buf` at the given short-element index.
///
/// The byte offset is `idx * 2`. Uses native-endian byte order since
/// the values are only shuffled (never interpreted as numbers).
#[inline]
fn read_short(buf: &[u8], idx: usize) -> i16 {
    let byte_idx = idx * 2;
    i16::from_ne_bytes([buf[byte_idx], buf[byte_idx + 1]])
}

/// Write an `i16` value to `buf` at the given short-element index.
///
/// The byte offset is `idx * 2`. Uses native-endian byte order.
#[inline]
fn write_short(buf: &mut [u8], idx: usize, val: i16) {
    let byte_idx = idx * 2;
    let bytes = val.to_ne_bytes();
    buf[byte_idx] = bytes[0];
    buf[byte_idx + 1] = bytes[1];
}

// =============================================================================
// Column-Major Transform (f_wipe.c lines 50–70)
// =============================================================================

/// Transform a buffer from row-major to column-major layout.
///
/// Translated from `wipe_shittyColMajorXform` in f_wipe.c lines 50–70.
/// The function name is preserved from the original source (snake_case
/// conversion of id Software's colorful naming).
///
/// Operates on `i16` (short) elements — `width` and `height` are in
/// short-element units, not byte units. The byte buffer must be at least
/// `width * height * 2` bytes long.
///
/// # Arguments
///
/// * `array` — Byte buffer to transform in-place.
/// * `width` — Number of `i16` columns (typically `SCREENWIDTH / 2`).
/// * `height` — Number of rows (typically `SCREENHEIGHT`).
fn wipe_shitty_col_major_xform(array: &mut [u8], width: usize, height: usize) {
    // Allocate temporary buffer for the transposed result.
    // Replaces Z_Malloc/Z_Free from the original C code.
    let count = width * height;
    let mut dest = vec![0i16; count];

    // Transpose: dest[x * height + y] = array[y * width + x]
    for y in 0..height {
        for x in 0..width {
            dest[x * height + y] = read_short(array, y * width + x);
        }
    }

    // Copy transposed data back to the original buffer.
    // Replaces memcpy(array, dest, width*height*2) from the original.
    for (i, &val) in dest.iter().enumerate() {
        write_short(array, i, val);
    }
}

// =============================================================================
// Color Crossfade Wipe (f_wipe.c lines 72–135)
// =============================================================================

/// Initialize the color crossfade wipe.
///
/// Translated from `wipe_initColorXForm` in f_wipe.c lines 72–80.
/// Copies the start screen to the working buffer as the initial state.
///
/// Returns 0.
fn wipe_init_color_xform(state: &mut WipeState, width: i32, height: i32, _ticks: i32) -> i32 {
    let size = (width * height) as usize;
    state.wipe_scr[..size].copy_from_slice(&state.wipe_scr_start[..size]);
    0
}

/// Execute one step of the color crossfade wipe.
///
/// Translated from `wipe_doColorXForm` in f_wipe.c lines 82–126.
/// For each pixel in the working buffer, moves its value toward the
/// corresponding end-screen pixel by `ticks` units. Values that overshoot
/// are clamped to the target.
///
/// Returns 1 when all pixels have reached their target (wipe complete),
/// 0 when changes are still occurring (wipe in progress).
fn wipe_do_color_xform(state: &mut WipeState, width: i32, height: i32, ticks: i32) -> i32 {
    let size = (width * height) as usize;
    let mut changed = false;

    for i in 0..size {
        let w_val = state.wipe_scr[i] as i32;
        let e_val = state.wipe_scr_end[i] as i32;

        if w_val != e_val {
            if w_val > e_val {
                // Pixel is brighter than target — decrease toward target.
                let newval = w_val - ticks;
                if newval < e_val {
                    state.wipe_scr[i] = state.wipe_scr_end[i];
                } else {
                    state.wipe_scr[i] = newval as u8;
                }
                changed = true;
            } else {
                // Pixel is darker than target — increase toward target.
                let newval = w_val + ticks;
                if newval > e_val {
                    state.wipe_scr[i] = state.wipe_scr_end[i];
                } else {
                    state.wipe_scr[i] = newval as u8;
                }
                changed = true;
            }
        }
    }

    // Return !changed: 1 when nothing changed (complete), 0 when still going.
    i32::from(!changed)
}

/// Finalize the color crossfade wipe (no-op).
///
/// Translated from `wipe_exitColorXForm` in f_wipe.c lines 128–135.
/// No cleanup required for the crossfade effect.
///
/// Returns 0.
fn wipe_exit_color_xform(_state: &mut WipeState, _width: i32, _height: i32, _ticks: i32) -> i32 {
    0
}

// =============================================================================
// Melt Wipe — the iconic DOOM effect (f_wipe.c lines 138–234)
// =============================================================================

/// Initialize the melt wipe effect.
///
/// Translated from `wipe_initMelt` in f_wipe.c lines 140–169.
///
/// 1. Copies the start screen to the working buffer.
/// 2. Applies the column-major transform to both start and end screens
///    (treating them as arrays of `i16` with `width/2` columns).
/// 3. Initializes the per-column position array `y[]` with randomized
///    initial offsets:
///    - `y[0] = -(M_Random() % 16)` — random negative offset (delay)
///    - Each subsequent column: `y[i] = y[i-1] + (M_Random() % 3) - 1`
///      (smooth variation: -1, 0, or +1 from the previous column)
///    - Clamped so that `y[i] <= 0` (never starts scrolled) and
///      `y[i] != -16` (avoids a specific stall case, set to -15 instead).
///
/// **CRITICAL**: Uses `m_random()` (not `p_random()`) to avoid perturbing
/// the gameplay-deterministic PRNG sequence. The full `width` (not `width/2`)
/// entries are initialized to match the exact number of `M_Random()` calls
/// made by the original C code, preserving the PRNG state.
///
/// Returns 0.
fn wipe_init_melt(
    state: &mut WipeState,
    rng: &mut DoomRandom,
    width: i32,
    height: i32,
    _ticks: i32,
) -> i32 {
    let w = width as usize;
    let h = height as usize;
    let size = w * h;

    // Copy start screen to working screen.
    state.wipe_scr[..size].copy_from_slice(&state.wipe_scr_start[..size]);

    // Transform both start and end screens to column-major format.
    // This makes the per-column reads in wipe_do_melt sequential in memory.
    // Width is halved because elements are i16 (2 bytes / 2 pixels each).
    wipe_shitty_col_major_xform(&mut state.wipe_scr_start, w / 2, h);
    wipe_shitty_col_major_xform(&mut state.wipe_scr_end, w / 2, h);

    // Initialize per-column scroll positions.
    // Allocate full width entries (320 for standard screen) even though
    // only width/2 (160) are used in wipe_do_melt. This preserves the
    // exact M_Random() call count from the original C code.
    state.y = vec![0i32; w];
    state.y[0] = -(rng.m_random() as i32 % 16);
    for i in 1..w {
        let r = (rng.m_random() as i32 % 3) - 1;
        state.y[i] = state.y[i - 1] + r;
        if state.y[i] > 0 {
            state.y[i] = 0;
        } else if state.y[i] == -16 {
            state.y[i] = -15;
        }
    }

    0
}

/// Execute one step of the melt wipe effect.
///
/// Translated from `wipe_doMelt` in f_wipe.c lines 171–224.
///
/// Processes `ticks` iterations of column advancement. For each of the
/// `width/2` columns (since we operate on `i16` / 2-pixel groups):
///
/// - If `y[i] < 0`: column is in its initial delay — increment `y[i]`.
/// - If `y[i] < height`: column is actively scrolling:
///   - Compute scroll speed `dy`: accelerates for `y < 16` (dy = y+1),
///     then constant speed (dy = 8).
///   - Clamp `dy` so the column doesn't exceed `height`.
///   - Copy `dy` rows from end screen (column-major) to working screen
///     (row-major) at the current melt position.
///   - Copy remaining rows from start screen (column-major) to working
///     screen (row-major) below the melt line.
///   - Advance `y[i]` by `dy`.
/// - If `y[i] >= height`: column is complete — no action needed.
///
/// Returns 1 when all columns have completed (`y[i] >= height` for all),
/// 0 when the animation is still in progress.
///
/// **CRITICAL**: The column-major → row-major indexing must match exactly:
/// - End screen (column-major): `end[i * height + row]`
/// - Working screen (row-major): `scr[row * half_width + i]`
fn wipe_do_melt(state: &mut WipeState, width: i32, height: i32, ticks: i32) -> i32 {
    let half_width = (width / 2) as usize;
    let h = height as usize;
    let mut done = true;

    let mut remaining_ticks = ticks;
    while remaining_ticks > 0 {
        remaining_ticks -= 1;

        for i in 0..half_width {
            if state.y[i] < 0 {
                // Column is in initial delay phase — advance toward 0.
                state.y[i] += 1;
                done = false;
            } else if (state.y[i] as usize) < h {
                // Column is actively scrolling.
                // Acceleration pattern: dy = y+1 for y < 16, dy = 8 otherwise.
                let mut dy: i32 = if state.y[i] < 16 { state.y[i] + 1 } else { 8 };
                // Clamp so column doesn't exceed screen height.
                if state.y[i] + dy >= height {
                    dy = height - state.y[i];
                }
                let dy_usize = dy as usize;
                let yi = state.y[i] as usize;

                // Copy dy rows from end screen (column-major) to working
                // screen (row-major). This reveals the new screen content
                // at the current melt position.
                //
                // Source: end_screen[i * height + y[i] + j] (column-major)
                // Dest:   wipe_scr[(y[i] + j) * half_width + i] (row-major)
                {
                    let s_base = i * h + yi;
                    for j in 0..dy_usize {
                        let s_idx = s_base + j;
                        let d_idx = (yi + j) * half_width + i;
                        let val = read_short(&state.wipe_scr_end, s_idx);
                        write_short(&mut state.wipe_scr, d_idx, val);
                    }
                }

                // Advance the column position.
                state.y[i] += dy;
                let new_yi = state.y[i] as usize;

                // Copy remaining rows from start screen (column-major) to
                // working screen (row-major). This shows the old screen
                // content below the melt line, sliding down.
                //
                // Source: start_screen[i * height + j] (column start, row 0)
                // Dest:   wipe_scr[(new_y[i] + j) * half_width + i] (row-major)
                {
                    let s_base = i * h;
                    let remaining = h - new_yi;
                    for j in 0..remaining {
                        let s_idx = s_base + j;
                        let d_idx = (new_yi + j) * half_width + i;
                        let val = read_short(&state.wipe_scr_start, s_idx);
                        write_short(&mut state.wipe_scr, d_idx, val);
                    }
                }

                done = false;
            }
            // else: y[i] >= height — column is complete, no action needed.
        }
    }

    i32::from(done)
}

/// Finalize the melt wipe effect.
///
/// Translated from `wipe_exitMelt` in f_wipe.c lines 226–234.
/// Releases the per-column position array. In the original C code this
/// called `Z_Free(y)`; in Rust we clear the Vec.
///
/// Returns 0.
fn wipe_exit_melt(state: &mut WipeState, _width: i32, _height: i32, _ticks: i32) -> i32 {
    state.y.clear();
    0
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard screen dimensions for tests.
    const TEST_WIDTH: i32 = SCREENWIDTH;
    const TEST_HEIGHT: i32 = SCREENHEIGHT;
    const TEST_SIZE: usize = (SCREENWIDTH * SCREENHEIGHT) as usize;

    // =========================================================================
    // WipeType enum tests
    // =========================================================================

    #[test]
    fn test_wipe_type_values() {
        assert_eq!(WipeType::ColorXForm as i32, 0);
        assert_eq!(WipeType::Melt as i32, 1);
    }

    #[test]
    fn test_wipe_numwipes() {
        assert_eq!(WIPE_NUMWIPES, 2);
    }

    #[test]
    fn test_wipe_type_copy_clone() {
        let wt = WipeType::Melt;
        let wt2 = wt; // Copy
        #[allow(clippy::clone_on_copy)]
        let wt3 = wt.clone(); // Clone (explicit to test Clone trait impl)
        assert_eq!(wt, wt2);
        assert_eq!(wt, wt3);
    }

    // =========================================================================
    // WipeState construction tests
    // =========================================================================

    #[test]
    fn test_wipe_state_new() {
        let state = WipeState::new();
        assert!(!state.go);
        assert!(state.wipe_scr_start.is_empty());
        assert!(state.wipe_scr_end.is_empty());
        assert!(state.wipe_scr.is_empty());
        assert!(state.y.is_empty());
    }

    #[test]
    fn test_wipe_state_default() {
        let state = WipeState::default();
        assert!(!state.go);
        assert!(state.wipe_scr_start.is_empty());
    }

    // =========================================================================
    // Column-major transform tests
    // =========================================================================

    #[test]
    fn test_col_major_xform_2x3() {
        // 2 columns × 3 rows of i16 values = 12 bytes
        // Row-major layout: [1,2, 3,4, 5,6] (as shorts)
        // Col-major layout: [1,3,5, 2,4,6] (as shorts)
        let mut buf = Vec::new();
        for val in &[1i16, 2, 3, 4, 5, 6] {
            buf.extend_from_slice(&val.to_ne_bytes());
        }

        wipe_shitty_col_major_xform(&mut buf, 2, 3);

        let expected = [1i16, 3, 5, 2, 4, 6];
        for (i, &exp) in expected.iter().enumerate() {
            assert_eq!(read_short(&buf, i), exp, "Mismatch at index {i}");
        }
    }

    #[test]
    fn test_col_major_xform_1x1() {
        // Single element — should be unchanged.
        let mut buf = 42i16.to_ne_bytes().to_vec();
        wipe_shitty_col_major_xform(&mut buf, 1, 1);
        assert_eq!(read_short(&buf, 0), 42);
    }

    #[test]
    fn test_col_major_xform_3x2() {
        // 3 columns × 2 rows: [10,20,30, 40,50,60]
        // Col-major: [10,40, 20,50, 30,60]
        let mut buf = Vec::new();
        for val in &[10i16, 20, 30, 40, 50, 60] {
            buf.extend_from_slice(&val.to_ne_bytes());
        }

        wipe_shitty_col_major_xform(&mut buf, 3, 2);

        let expected = [10i16, 40, 20, 50, 30, 60];
        for (i, &exp) in expected.iter().enumerate() {
            assert_eq!(read_short(&buf, i), exp, "Mismatch at index {i}");
        }
    }

    // =========================================================================
    // Color crossfade tests
    // =========================================================================

    #[test]
    fn test_color_xform_identical_screens() {
        // When start == end, the crossfade should complete immediately.
        let mut state = WipeState::new();
        state.wipe_scr_start = vec![128u8; TEST_SIZE];
        state.wipe_scr_end = vec![128u8; TEST_SIZE];
        state.wipe_scr = vec![0u8; TEST_SIZE];

        wipe_init_color_xform(&mut state, TEST_WIDTH, TEST_HEIGHT, 1);
        // After init, wipe_scr == wipe_scr_start == wipe_scr_end
        assert_eq!(state.wipe_scr, state.wipe_scr_start);

        let rc = wipe_do_color_xform(&mut state, TEST_WIDTH, TEST_HEIGHT, 1);
        assert_eq!(
            rc, 1,
            "Should complete immediately when screens are identical"
        );
    }

    #[test]
    fn test_color_xform_single_step_complete() {
        // With ticks large enough to bridge the gap in one step.
        let mut state = WipeState::new();
        state.wipe_scr_start = vec![0u8; TEST_SIZE];
        state.wipe_scr_end = vec![5u8; TEST_SIZE];
        state.wipe_scr = vec![0u8; TEST_SIZE];

        wipe_init_color_xform(&mut state, TEST_WIDTH, TEST_HEIGHT, 1);

        // ticks = 10 should bridge a gap of 5 in one step
        let rc = wipe_do_color_xform(&mut state, TEST_WIDTH, TEST_HEIGHT, 10);
        // After clamping, all pixels should equal the end value
        assert!(state.wipe_scr.iter().all(|&b| b == 5));
        // But done returns 0 because changes were made
        assert_eq!(rc, 0);

        // Second call: nothing to change, returns 1
        let rc = wipe_do_color_xform(&mut state, TEST_WIDTH, TEST_HEIGHT, 1);
        assert_eq!(rc, 1, "Should be complete on second call");
    }

    #[test]
    fn test_color_xform_gradual_decrease() {
        let mut state = WipeState::new();
        state.wipe_scr_start = vec![100u8; 4];
        state.wipe_scr_end = vec![90u8; 4];
        state.wipe_scr = vec![100u8; 4];

        // Decrease by 3 per tick
        let rc = wipe_do_color_xform(&mut state, 2, 2, 3);
        assert_eq!(rc, 0); // still in progress
        assert!(state.wipe_scr.iter().all(|&b| b == 97));

        let rc = wipe_do_color_xform(&mut state, 2, 2, 3);
        assert_eq!(rc, 0);
        assert!(state.wipe_scr.iter().all(|&b| b == 94));

        let rc = wipe_do_color_xform(&mut state, 2, 2, 3);
        assert_eq!(rc, 0);
        assert!(state.wipe_scr.iter().all(|&b| b == 91));

        // Next step would go below 90, so clamp to 90
        let rc = wipe_do_color_xform(&mut state, 2, 2, 3);
        assert_eq!(rc, 0);
        assert!(state.wipe_scr.iter().all(|&b| b == 90));

        // Now complete
        let rc = wipe_do_color_xform(&mut state, 2, 2, 3);
        assert_eq!(rc, 1);
    }

    // =========================================================================
    // Melt initialization tests
    // =========================================================================

    #[test]
    fn test_melt_init_y_array_length() {
        let mut state = WipeState::new();
        state.wipe_scr_start = vec![0u8; TEST_SIZE];
        state.wipe_scr_end = vec![0u8; TEST_SIZE];
        state.wipe_scr = vec![0u8; TEST_SIZE];
        let mut rng = DoomRandom::new();

        wipe_init_melt(&mut state, &mut rng, TEST_WIDTH, TEST_HEIGHT, 1);

        // y array should have SCREENWIDTH (320) entries
        assert_eq!(state.y.len(), SCREENWIDTH as usize);
    }

    #[test]
    fn test_melt_init_y_values_nonpositive() {
        let mut state = WipeState::new();
        state.wipe_scr_start = vec![0u8; TEST_SIZE];
        state.wipe_scr_end = vec![0u8; TEST_SIZE];
        state.wipe_scr = vec![0u8; TEST_SIZE];
        let mut rng = DoomRandom::new();

        wipe_init_melt(&mut state, &mut rng, TEST_WIDTH, TEST_HEIGHT, 1);

        // All y values should be <= 0 after initialization
        for (i, &val) in state.y.iter().enumerate() {
            assert!(val <= 0, "y[{i}] = {val} should be <= 0");
        }
    }

    #[test]
    fn test_melt_init_no_y_equals_neg16() {
        let mut state = WipeState::new();
        state.wipe_scr_start = vec![0u8; TEST_SIZE];
        state.wipe_scr_end = vec![0u8; TEST_SIZE];
        state.wipe_scr = vec![0u8; TEST_SIZE];
        let mut rng = DoomRandom::new();

        wipe_init_melt(&mut state, &mut rng, TEST_WIDTH, TEST_HEIGHT, 1);

        // No y value should be exactly -16 (clamped to -15)
        for (i, &val) in state.y.iter().enumerate() {
            assert_ne!(val, -16, "y[{i}] should not be -16 (should be -15)");
        }
    }

    #[test]
    fn test_melt_init_uses_m_random() {
        // Verify that melt init consumes exactly 321 M_Random calls
        // (1 for y[0] + 319 for y[1..319] = 320 total for a 320-wide screen).
        let mut rng1 = DoomRandom::new();
        let mut rng2 = DoomRandom::new();

        let mut state = WipeState::new();
        state.wipe_scr_start = vec![0u8; TEST_SIZE];
        state.wipe_scr_end = vec![0u8; TEST_SIZE];
        state.wipe_scr = vec![0u8; TEST_SIZE];

        wipe_init_melt(&mut state, &mut rng1, TEST_WIDTH, TEST_HEIGHT, 1);

        // Advance rng2 by 320 calls to match
        for _ in 0..SCREENWIDTH {
            rng2.m_random();
        }

        assert_eq!(
            rng1.rnd_index(),
            rng2.rnd_index(),
            "PRNG state should match after 320 M_Random calls"
        );
    }

    // =========================================================================
    // Melt do tests
    // =========================================================================

    #[test]
    fn test_melt_acceleration_pattern() {
        // Test the dy acceleration: dy = y+1 for y < 16, dy = 8 for y >= 16.
        // Use a minimal 4x4 buffer (2 columns of i16, 4 rows).
        let w = 4i32;
        let h = 4i32;
        let size = (w * h) as usize;

        let mut state = WipeState::new();
        state.wipe_scr_start = vec![0u8; size];
        state.wipe_scr_end = vec![0u8; size];
        state.wipe_scr = vec![0u8; size];
        // Set y[0] = 0 so the column starts scrolling immediately
        state.y = vec![0i32; w as usize];

        // First tick: y[0] = 0, dy = 0 + 1 = 1
        wipe_do_melt(&mut state, w, h, 1);
        assert_eq!(state.y[0], 1, "After first tick: y should be 1");

        // Second tick: y[0] = 1, dy = 1 + 1 = 2
        wipe_do_melt(&mut state, w, h, 1);
        assert_eq!(state.y[0], 3, "After second tick: y should be 3");
    }

    #[test]
    fn test_melt_completion() {
        // Verify that wipe_do_melt returns 1 (done) after all columns complete.
        let w = 4i32;
        let h = 4i32;
        let size = (w * h) as usize;

        let mut state = WipeState::new();
        state.wipe_scr_start = vec![0u8; size];
        state.wipe_scr_end = vec![0u8; size];
        state.wipe_scr = vec![0u8; size];
        // Set all columns to height (already complete)
        state.y = vec![h; w as usize];

        let rc = wipe_do_melt(&mut state, w, h, 1);
        assert_eq!(rc, 1, "All columns at height should report done");
    }

    #[test]
    fn test_melt_negative_y_delay() {
        // Columns with negative y values should delay before scrolling.
        let w = 4i32;
        let h = 4i32;
        let size = (w * h) as usize;

        let mut state = WipeState::new();
        state.wipe_scr_start = vec![0u8; size];
        state.wipe_scr_end = vec![0u8; size];
        state.wipe_scr = vec![0u8; size];
        state.y = vec![-3i32; w as usize];

        // Process 1 tick — y should go from -3 to -2
        let rc = wipe_do_melt(&mut state, w, h, 1);
        assert_eq!(rc, 0, "Wipe should not be done yet");
        assert_eq!(state.y[0], -2);

        // Process 2 more ticks — y should go from -2 to 0
        let rc = wipe_do_melt(&mut state, w, h, 2);
        assert_eq!(rc, 0);
        assert_eq!(state.y[0], 0);
    }

    // =========================================================================
    // Integration tests with VideoState
    // =========================================================================

    #[test]
    fn test_start_screen_captures_screen_zero() {
        let mut video = VideoState::new();
        // Fill screen 0 with a known pattern
        for (i, byte) in video.screens[0].iter_mut().enumerate() {
            *byte = (i % 256) as u8;
        }

        let mut state = WipeState::new();
        state.start_screen(&video, 0, 0, TEST_WIDTH, TEST_HEIGHT);

        assert_eq!(state.wipe_scr_start.len(), TEST_SIZE);
        assert_eq!(state.wipe_scr_start, video.screens[0]);
    }

    #[test]
    fn test_end_screen_captures_and_restores() {
        let mut video = VideoState::new();
        let mut state = WipeState::new();

        // Set up start screen
        for byte in video.screens[0].iter_mut() {
            *byte = 10;
        }
        state.start_screen(&video, 0, 0, TEST_WIDTH, TEST_HEIGHT);

        // "Render" new state
        for byte in video.screens[0].iter_mut() {
            *byte = 20;
        }
        state.end_screen(&mut video, 0, 0, TEST_WIDTH, TEST_HEIGHT);

        // End screen should hold the new state
        assert!(state.wipe_scr_end.iter().all(|&b| b == 20));
        // Screen 0 should be restored to start state
        assert!(video.screens[0].iter().all(|&b| b == 10));
    }

    #[test]
    fn test_screen_wipe_color_xform_complete_cycle() {
        let mut video = VideoState::new();
        let mut state = WipeState::new();
        let mut rng = DoomRandom::new();

        // Start with black screen
        state.start_screen(&video, 0, 0, TEST_WIDTH, TEST_HEIGHT);

        // "Render" white screen
        for byte in video.screens[0].iter_mut() {
            *byte = 10;
        }
        state.end_screen(&mut video, 0, 0, TEST_WIDTH, TEST_HEIGHT);

        // Run wipe until complete
        let mut iterations = 0;
        loop {
            let rc = state.screen_wipe(
                &mut video,
                &mut rng,
                WipeType::ColorXForm,
                0,
                0,
                TEST_WIDTH,
                TEST_HEIGHT,
                1,
            );
            iterations += 1;
            if rc != 0 || iterations > 300 {
                break;
            }
        }

        // After completion, screen 0 should show the end screen
        assert!(
            video.screens[0].iter().all(|&b| b == 10),
            "Screen should show end state after wipe completes"
        );
        assert!(
            iterations <= 300,
            "Wipe should complete within 300 iterations"
        );
    }

    // =========================================================================
    // Exit function tests
    // =========================================================================

    #[test]
    fn test_exit_color_xform_noop() {
        let mut state = WipeState::new();
        assert_eq!(wipe_exit_color_xform(&mut state, 0, 0, 0), 0);
    }

    #[test]
    fn test_exit_melt_clears_y() {
        let mut state = WipeState::new();
        state.y = vec![1, 2, 3, 4, 5];
        wipe_exit_melt(&mut state, 0, 0, 0);
        assert!(state.y.is_empty(), "y array should be cleared after exit");
    }

    // =========================================================================
    // Short helper tests
    // =========================================================================

    #[test]
    fn test_read_write_short_roundtrip() {
        let mut buf = vec![0u8; 8];
        write_short(&mut buf, 0, 1234);
        write_short(&mut buf, 1, -5678);
        write_short(&mut buf, 2, 0);
        write_short(&mut buf, 3, i16::MAX);

        assert_eq!(read_short(&buf, 0), 1234);
        assert_eq!(read_short(&buf, 1), -5678);
        assert_eq!(read_short(&buf, 2), 0);
        assert_eq!(read_short(&buf, 3), i16::MAX);
    }
}
