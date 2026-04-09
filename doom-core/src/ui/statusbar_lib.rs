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
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

//! The status bar widget code.
//!
//! Provides low-level status bar widget primitives: number display,
//! percentage display, multiple icon selector, and binary icon toggle.
//! These are the building blocks used by `statusbar.rs` (ST_Stuff).
//!
//! Translated from `linuxdoom-1.10/st_lib.c` (293 lines) and
//! `linuxdoom-1.10/st_lib.h` (227 lines).
//!
//! ## Widget Types
//!
//! - [`StNumber`] — Right-justified multi-digit number display
//! - [`StPercent`] — Number with appended percent sign
//! - [`StMultIcon`] — Selectable icon from an indexed set
//! - [`StBinIcon`] — On/off toggle icon
//!
//! ## Screen Buffers
//!
//! The status bar uses screen [`BG`] (4) as the background source for
//! erasing widget areas, and screen [`FG`] (0) as the draw target.
//! **Note:** This differs from the HUD library (`hud_lib.rs`) which
//! uses screen 1 as its background buffer.

use crate::types::doomdef::{SCREENHEIGHT, SCREENWIDTH};
use crate::util::swap;
use crate::video::video::VideoState;
use doom_wad::PurgeTag;

/// Background screen number for status bar widgets.
///
/// The status bar background is stored on screen 4. Widget erase
/// operations copy from this buffer to restore the background behind
/// a widget before redrawing.
///
/// **IMPORTANT:** This is 4 (`screens[4]`), NOT 1 (which is used by
/// HUD widgets in `hud_lib.rs`).
pub const BG: usize = 4;

/// Foreground screen number (primary display surface).
///
/// All visible widget drawing is rendered to screen 0, which is the
/// buffer presented to the player.
pub const FG: usize = 0;

/// Status bar height in pixels (`32 * SCREEN_MUL` where `SCREEN_MUL = 1`).
const ST_HEIGHT: i32 = 32;

/// Status bar Y position on screen (top edge of the status bar).
/// Computed as `SCREENHEIGHT - ST_HEIGHT` = 200 - 32 = 168.
const ST_Y: i32 = SCREENHEIGHT - ST_HEIGHT;

/// Status bar width in pixels (equal to screen width).
/// Defined here for completeness, matching the original `ST_WIDTH` from
/// `st_stuff.h`. Used indirectly via [`VideoState::copy_rect`] for
/// background copy operations across the full status bar width.
#[allow(dead_code)]
const ST_WIDTH: i32 = SCREENWIDTH;

/// Type alias for raw WAD patch data stored as owned byte vectors.
///
/// Patches are stored in the WAD file format with the following header:
/// - Bytes 0–1: `width` (`i16`, little-endian)
/// - Bytes 2–3: `height` (`i16`, little-endian)
/// - Bytes 4–5: `leftoffset` (`i16`, little-endian)
/// - Bytes 6–7: `topoffset` (`i16`, little-endian)
/// - Bytes 8+: column offsets and pixel data
///
/// The [`VideoState::draw_patch`] method accepts `&[u8]` slices in this
/// format directly.
pub type PatchData = Vec<u8>;

// ---------------------------------------------------------------------------
// Patch header field extraction helpers
// ---------------------------------------------------------------------------
// These correspond to accessing `patch->width`, `patch->height`, etc.
// through the `SHORT()` macro in the original C code. We parse the
// little-endian fields from the raw byte buffer rather than constructing
// a full `Patch` struct, since `VideoState::draw_patch` operates on raw
// bytes directly (matching the C `patch_t*` pointer pattern).
// ---------------------------------------------------------------------------

/// Extracts the width field from raw patch header bytes.
///
/// Equivalent to `SHORT(patch->width)` in the original C code.
fn patch_width(data: &[u8]) -> i32 {
    if data.len() >= 2 {
        swap::short(i16::from_ne_bytes([data[0], data[1]])) as i32
    } else {
        0
    }
}

/// Extracts the height field from raw patch header bytes.
///
/// Equivalent to `SHORT(patch->height)` in the original C code.
fn patch_height(data: &[u8]) -> i32 {
    if data.len() >= 4 {
        swap::short(i16::from_ne_bytes([data[2], data[3]])) as i32
    } else {
        0
    }
}

/// Extracts the leftoffset field from raw patch header bytes.
///
/// Equivalent to `SHORT(patch->leftoffset)` in the original C code.
fn patch_leftoffset(data: &[u8]) -> i32 {
    if data.len() >= 6 {
        swap::short(i16::from_ne_bytes([data[4], data[5]])) as i32
    } else {
        0
    }
}

/// Extracts the topoffset field from raw patch header bytes.
///
/// Equivalent to `SHORT(patch->topoffset)` in the original C code.
fn patch_topoffset(data: &[u8]) -> i32 {
    if data.len() >= 8 {
        swap::short(i16::from_ne_bytes([data[6], data[7]])) as i32
    } else {
        0
    }
}

// ===========================================================================
// StNumber — Numeric widget (right-justified number display)
// ===========================================================================

/// A status bar number widget.
///
/// Displays a right-justified multi-digit number using WAD digit patches
/// (the `STTNUM0`–`STTNUM9` font graphics). Supports negative numbers
/// via the STTMINUS patch, and respects a configurable digit width.
///
/// Corresponds to `st_number_t` in the original `st_lib.h`.
///
/// ## Right-Justification
///
/// The `x` coordinate specifies the **right** edge of the number display
/// area. Digits are drawn right-to-left, each shifted left by the digit
/// patch width.
///
/// ## Special Value: 1994
///
/// The value 1994 is a Jaguar DOOM easter egg and is intentionally **not
/// drawn**. This behavior is preserved from the original engine.
pub struct StNumber {
    /// X position (upper-right corner — number is right-justified from here).
    pub x: i32,
    /// Y position (top edge of the digit patches).
    pub y: i32,
    /// Maximum number of digits to display.
    pub width: i32,
    /// Previously drawn number value (for external change detection).
    pub oldnum: i32,
    /// Current number value to display.
    /// In the original C code this was `int* num` (pointer to shared state).
    /// In Rust, the caller updates this field directly before calling
    /// [`update`](StNumber::update).
    pub num: i32,
    /// Whether this widget is active and should be drawn.
    /// In the original C code this was `boolean* on` (pointer to shared state).
    /// In Rust, the caller sets this field directly.
    pub on: bool,
    /// Digit patches for 0–9. Index `n` contains the raw patch data for
    /// digit `n`. These are typically loaded from WAD lumps `STTNUM0`
    /// through `STTNUM9`.
    pub p: [PatchData; 10],
    /// User data field (unused by the widget library itself; available
    /// for the caller to store auxiliary information).
    pub data: i32,
}

impl StNumber {
    /// Creates a new number widget.
    ///
    /// Corresponds to `STlib_initNum` in the original `st_lib.c`.
    ///
    /// # Arguments
    ///
    /// * `x` — Right-edge X coordinate (right-justified)
    /// * `y` — Y coordinate (top edge)
    /// * `patches` — Slice of digit patches (should contain 10 entries for
    ///   digits 0–9). Entries beyond 10 are ignored; missing entries default
    ///   to empty patch data.
    /// * `num` — Initial number value to display
    /// * `on` — Whether the widget starts in the active (drawing) state
    /// * `width` — Maximum number of digits (e.g., 3 for a 3-digit counter)
    pub fn new(x: i32, y: i32, patches: &[PatchData], num: i32, on: bool, width: i32) -> Self {
        let mut p: [PatchData; 10] = Default::default();
        for (i, patch) in patches.iter().enumerate().take(10) {
            p[i] = patch.clone();
        }
        Self {
            x,
            y,
            width,
            oldnum: 0,
            num,
            on,
            p,
            data: 0,
        }
    }

    /// Updates and redraws the number widget.
    ///
    /// Corresponds to `STlib_updateNum` in the original `st_lib.c`.
    /// When the widget is active (`on == true`), this draws the current
    /// number value using the digit patches.
    ///
    /// The `_refresh` parameter is accepted for API consistency with the
    /// original C interface, but is unused by the number widget — drawing
    /// is unconditional when the widget is on.
    ///
    /// # Arguments
    ///
    /// * `_refresh` — Whether a full screen refresh is occurring (unused
    ///   for number widgets; present for interface consistency)
    /// * `sttminus` — Raw patch data for the STTMINUS (minus sign) glyph,
    ///   loaded by [`stlib_init`]
    /// * `video` — Mutable reference to the video state for drawing
    pub fn update(&mut self, _refresh: bool, sttminus: &[u8], video: &mut VideoState) {
        if self.on {
            self.draw_num(sttminus, video);
        }
    }

    /// Internal: draws the number right-to-left using digit patches.
    ///
    /// Corresponds to `STlib_drawNum` in the original `st_lib.c`.
    ///
    /// Behavior:
    /// 1. Clamps negative numbers to the displayable range based on `width`
    /// 2. Skips drawing entirely for the special value 1994 (Jaguar DOOM
    ///    easter egg)
    /// 3. Draws zero explicitly as a single '0' digit
    /// 4. Draws remaining digits right-to-left
    /// 5. Prepends the minus sign for negative numbers
    fn draw_num(&mut self, sttminus: &[u8], video: &mut VideoState) {
        let mut numdigits = self.width;
        let mut num = self.num;
        let w = patch_width(&self.p[0]);

        // Record the value we are drawing for external change tracking.
        self.oldnum = self.num;

        let neg = num < 0;

        if neg {
            // Clamp negative values to fit within the digit width.
            // 2-digit display: min -9;  3-digit display: min -99.
            if numdigits == 2 && num < -9 {
                num = -9;
            } else if numdigits == 3 && num < -99 {
                num = -99;
            }
            num = -num;
        }

        // Special case: the value 1994 is a Jaguar DOOM reference and is
        // intentionally not drawn (original behavior preserved).
        if num == 1994 {
            return;
        }

        let mut x = self.x;

        // In the special case of zero, explicitly draw the '0' digit.
        if num == 0 {
            video.draw_patch(x - w, self.y, FG, &self.p[0]);
        }

        // Draw digits right-to-left.
        while num != 0 && numdigits > 0 {
            numdigits -= 1;
            x -= w;
            video.draw_patch(x, self.y, FG, &self.p[(num % 10) as usize]);
            num /= 10;
        }

        // Draw the minus sign if the number was negative.
        if neg && !sttminus.is_empty() {
            video.draw_patch(x - 8, self.y, FG, sttminus);
        }
    }
}

// ===========================================================================
// StPercent — Percentage widget (number + percent sign)
// ===========================================================================

/// A status bar percentage widget.
///
/// Extends [`StNumber`] by drawing a percent sign ('%') patch after the
/// numeric display. Used for health and armor percentage readouts.
///
/// Corresponds to `st_percent_t` in the original `st_lib.h`.
pub struct StPercent {
    /// The underlying number widget providing the numeric display.
    pub n: StNumber,
    /// The percent sign ('%') patch data (typically loaded from the WAD
    /// lump `STTPRCNT`).
    pub p: PatchData,
}

impl StPercent {
    /// Creates a new percentage widget.
    ///
    /// Corresponds to `STlib_initPercent` in the original `st_lib.c`.
    /// The underlying number widget is initialized with a width of 3
    /// (matching the original behavior).
    ///
    /// # Arguments
    ///
    /// * `x` — Right-edge X coordinate for the number portion
    /// * `y` — Y coordinate
    /// * `patches` — Slice of 10 digit patches (0–9)
    /// * `num` — Initial numeric value
    /// * `on` — Whether the widget starts active
    /// * `percent` — The percent sign patch data
    pub fn new(
        x: i32,
        y: i32,
        patches: &[PatchData],
        num: i32,
        on: bool,
        percent: PatchData,
    ) -> Self {
        Self {
            n: StNumber::new(x, y, patches, num, on, 3),
            p: percent,
        }
    }

    /// Updates and redraws the percentage widget.
    ///
    /// Corresponds to `STlib_updatePercent` in the original `st_lib.c`.
    /// When `refresh` is true and the widget is on, draws the percent
    /// sign patch, then delegates to [`StNumber::update`] for the numeric
    /// portion.
    ///
    /// # Arguments
    ///
    /// * `refresh` — Whether a full screen refresh is occurring (triggers
    ///   percent sign redraw)
    /// * `sttminus` — Raw patch data for the minus sign
    /// * `video` — Mutable reference to the video state
    pub fn update(&mut self, refresh: bool, sttminus: &[u8], video: &mut VideoState) {
        if refresh && self.n.on {
            video.draw_patch(self.n.x, self.n.y, FG, &self.p);
        }
        self.n.update(refresh, sttminus, video);
    }
}

// ===========================================================================
// StMultIcon — Multiple icon selector widget
// ===========================================================================

/// A status bar multiple-icon widget.
///
/// Displays one icon from a set of icon patches, selected by an integer
/// index. Used for the face sprite selector (STFST\*) and key card
/// indicators. Negative index values indicate that no icon should be
/// displayed.
///
/// Corresponds to `st_multicon_t` in the original `st_lib.h`.
pub struct StMultIcon {
    /// X position for icon placement.
    pub x: i32,
    /// Y position for icon placement.
    pub y: i32,
    /// Previously displayed icon index (-1 means no icon was drawn).
    pub oldinum: i32,
    /// Current icon index to display (-1 means hide / no icon).
    /// In the original C code this was `int* inum` (pointer to shared
    /// state). In Rust, the caller updates this field directly.
    pub inum: i32,
    /// Whether this widget is active and should be drawn.
    pub on: bool,
    /// List of available icon patches. Index `n` holds the raw patch data
    /// for icon `n`.
    pub p: Vec<PatchData>,
    /// User data field (available for the caller).
    pub data: i32,
}

impl StMultIcon {
    /// Creates a new multiple-icon widget.
    ///
    /// Corresponds to `STlib_initMultIcon` in the original `st_lib.c`.
    /// The `oldinum` is initialized to -1 (no previous icon drawn).
    ///
    /// # Arguments
    ///
    /// * `x` — X coordinate for icon placement
    /// * `y` — Y coordinate for icon placement
    /// * `patches` — Vector of icon patches
    /// * `inum` — Initial icon index (-1 for none)
    /// * `on` — Whether the widget starts active
    pub fn new(x: i32, y: i32, patches: Vec<PatchData>, inum: i32, on: bool) -> Self {
        Self {
            x,
            y,
            oldinum: -1,
            inum,
            on,
            p: patches,
            data: 0,
        }
    }

    /// Updates and redraws the multiple-icon widget.
    ///
    /// Corresponds to `STlib_updateMultIcon` in the original `st_lib.c`.
    /// When the icon index has changed (or `refresh` is true) and the
    /// current index is valid (not -1):
    ///
    /// 1. Erases the old icon area by copying from the background buffer
    ///    (screen [`BG`]) to the foreground (screen [`FG`])
    /// 2. Draws the new icon patch at the widget position
    ///
    /// # Arguments
    ///
    /// * `refresh` — Whether a full screen refresh is occurring
    /// * `video` — Mutable reference to the video state
    pub fn update(&mut self, refresh: bool, video: &mut VideoState) {
        if !self.on {
            return;
        }

        if (self.oldinum != self.inum || refresh) && self.inum != -1 {
            // Erase the old icon by copying from background buffer.
            if self.oldinum != -1 && (self.oldinum as usize) < self.p.len() {
                let old_patch = &self.p[self.oldinum as usize];
                if !old_patch.is_empty() {
                    let px = self.x - patch_leftoffset(old_patch);
                    let py = self.y - patch_topoffset(old_patch);
                    let pw = patch_width(old_patch);
                    let ph = patch_height(old_patch);

                    // Safety check from original: y - ST_Y must be >= 0.
                    if py - ST_Y >= 0 {
                        video.copy_rect(px, py - ST_Y, BG, pw, ph, px, py, FG);
                    }
                }
            }

            // Draw the new icon.
            if (self.inum as usize) < self.p.len() && !self.p[self.inum as usize].is_empty() {
                video.draw_patch(self.x, self.y, FG, &self.p[self.inum as usize]);
            }

            self.oldinum = self.inum;
        }
    }
}

// ===========================================================================
// StBinIcon — Binary (on/off) icon widget
// ===========================================================================

/// A status bar binary icon widget.
///
/// Displays or hides a single icon based on a boolean value. When the
/// value is `true`, the icon patch is drawn; when `false`, the icon area
/// is erased by copying from the background buffer. Used for weapon
/// availability indicators in the arms display.
///
/// Corresponds to `st_binicon_t` in the original `st_lib.h`.
pub struct StBinIcon {
    /// X position for icon placement.
    pub x: i32,
    /// Y position for icon placement.
    pub y: i32,
    /// Previously displayed boolean state.
    pub oldval: bool,
    /// Current boolean value (`true` = show icon, `false` = hide).
    /// In the original C code this was `boolean* val` (pointer to shared
    /// state). In Rust, the caller updates this field directly.
    pub val: bool,
    /// Whether this widget is active and should be drawn.
    pub on: bool,
    /// The icon patch data (displayed when `val` is `true`).
    pub p: PatchData,
    /// User data field (available for the caller).
    pub data: i32,
}

impl StBinIcon {
    /// Creates a new binary icon widget.
    ///
    /// Corresponds to `STlib_initBinIcon` in the original `st_lib.c`.
    /// The `oldval` is initialized to `false` (matching the original
    /// `oldval = 0` behavior).
    ///
    /// # Arguments
    ///
    /// * `x` — X coordinate for icon placement
    /// * `y` — Y coordinate for icon placement
    /// * `patch` — The icon patch data
    /// * `val` — Initial boolean value
    /// * `on` — Whether the widget starts active
    pub fn new(x: i32, y: i32, patch: PatchData, val: bool, on: bool) -> Self {
        Self {
            x,
            y,
            oldval: false,
            val,
            on,
            p: patch,
            data: 0,
        }
    }

    /// Updates and redraws the binary icon widget.
    ///
    /// Corresponds to `STlib_updateBinIcon` in the original `st_lib.c`.
    /// When the value has changed (or `refresh` is true):
    ///
    /// - If `val` is `true`: draws the icon patch
    /// - If `val` is `false`: erases the icon area by copying from the
    ///   background buffer (screen [`BG`]) to foreground (screen [`FG`])
    ///
    /// # Arguments
    ///
    /// * `refresh` — Whether a full screen refresh is occurring
    /// * `video` — Mutable reference to the video state
    pub fn update(&mut self, refresh: bool, video: &mut VideoState) {
        if !self.on {
            return;
        }

        if self.oldval != self.val || refresh {
            let px = self.x - patch_leftoffset(&self.p);
            let py = self.y - patch_topoffset(&self.p);
            let pw = patch_width(&self.p);
            let ph = patch_height(&self.p);

            // Safety check from original: y - ST_Y must be >= 0.
            if py - ST_Y >= 0 {
                if self.val {
                    video.draw_patch(self.x, self.y, FG, &self.p);
                } else {
                    video.copy_rect(px, py - ST_Y, BG, pw, ph, px, py, FG);
                }
            }

            self.oldval = self.val;
        }
    }
}

// ===========================================================================
// Module initialization
// ===========================================================================

/// Initializes the status bar widget library by loading the STTMINUS
/// (minus sign) patch from the WAD file.
///
/// Corresponds to `STlib_init` in the original `st_lib.c`. The STTMINUS
/// patch is used by [`StNumber::draw_num`] to display negative frag counts
/// on the status bar.
///
/// This replaces the original C call:
/// ```c
/// sttminus = (patch_t *) W_CacheLumpName("STTMINUS", PU_STATIC);
/// ```
///
/// # Arguments
///
/// * `wad` — Mutable reference to a WAD provider for loading lump data
///
/// # Returns
///
/// The STTMINUS patch data as an owned byte vector ([`PatchData`]).
/// The caller should store this and pass it to widget update methods.
pub fn stlib_init(wad: &mut impl doom_wad::WadProvider) -> PatchData {
    wad.cache_lump_name("STTMINUS", PurgeTag::Static).to_vec()
}
