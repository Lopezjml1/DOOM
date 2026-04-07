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

//! Translated from linuxdoom-1.10/v_video.c and linuxdoom-1.10/v_video.h
//!
//! Gamma correction LUT stuff.
//! Functions to draw patches (by post) directly to screen buffers.
//! Functions to blit a block to the screen buffers.
//!
//! This module operates on raw pixel buffers (320×200×8-bit palettized)
//! and does NOT interact with platform-specific display APIs.
//!
//! # Screen Buffers
//!
//! The engine uses 5 screen buffers, each `SCREENWIDTH * SCREENHEIGHT`
//! (320 × 200 = 64,000) bytes:
//!
//! - Screen 0: Primary display buffer (updated to the window by
//!   `I_FinishUpdate`)
//! - Screen 1: Extra buffer used for background storage during wipes
//! - Screens 2–4: Temporary/background buffers for wipes, menus, etc.
//!
//! # Patch Drawing
//!
//! Patches use a column-post format where each column consists of runs of
//! non-transparent pixels separated by transparent gaps. The post structure
//! is: `[topdelta:u8][length:u8][padding:u8][pixel_data:length bytes][padding:u8]`.
//! A `topdelta` of `0xFF` marks the end of a column's post list.

use crate::types::doomdef::{SCREENHEIGHT, SCREENWIDTH};

// =============================================================================
// Constants (from v_video.h)
// =============================================================================

/// Vertical center of the screen in pixels (SCREENHEIGHT / 2 = 100).
/// Used for centering calculations throughout the renderer and UI.
pub const CENTERY: i32 = SCREENHEIGHT / 2;

/// Number of screen buffers. The original engine allocates 5 screens:
/// screens[0] is the primary display buffer, screens[1..4] are used for
/// background storage, wipe transitions, and temporary drawing.
pub const NUM_SCREENS: usize = 5;

// Bounding box coordinate indices, matching m_bbox.h definitions.
// These are module-private — external code uses the VideoState methods.
const BOXTOP: usize = 0;
const BOXBOTTOM: usize = 1;
const BOXLEFT: usize = 2;
const BOXRIGHT: usize = 3;

// =============================================================================
// Gamma Correction Lookup Table (from v_video.c lines 51-133)
// =============================================================================

/// Gamma correction lookup tables — 5 levels of gamma correction, each mapping
/// 256 input palette indices to corrected output values.
///
/// Level 0 is near-identity (starts at 1, not 0 — value 128 appears twice at
/// indices 127 and 128). Level 4 is the most aggressive correction (first value
/// is 16). These values are copied verbatim from the original C source and are
/// part of the engine's behavioral contract — incorrect values would cause
/// visible palette distortion.
///
/// # Original C comment
/// "Now where did these came from?"
#[rustfmt::skip]
pub static GAMMATABLE: [[u8; 256]; 5] = [
    // Level 0 (near-identity): v_video.c lines 53-68
    [
        1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,
        17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,
        33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,
        49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,
        65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,
        81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,
        97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,
        113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,128,
        128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,
        144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,
        160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,
        176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,
        192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,
        208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,
        224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239,
        240,241,242,243,244,245,246,247,248,249,250,251,252,253,254,255,
    ],
    // Level 1: v_video.c lines 70-84
    [
        2,4,5,7,8,10,11,12,14,15,16,18,19,20,21,23,24,25,26,27,29,30,31,
        32,33,34,36,37,38,39,40,41,42,44,45,46,47,48,49,50,51,52,54,55,
        56,57,58,59,60,61,62,63,64,65,66,67,69,70,71,72,73,74,75,76,77,
        78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,
        99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,
        115,116,117,118,119,120,121,122,123,124,125,126,127,128,129,129,
        130,131,132,133,134,135,136,137,138,139,140,141,142,143,144,145,
        146,147,148,148,149,150,151,152,153,154,155,156,157,158,159,160,
        161,162,163,163,164,165,166,167,168,169,170,171,172,173,174,175,
        175,176,177,178,179,180,181,182,183,184,185,186,186,187,188,189,
        190,191,192,193,194,195,196,196,197,198,199,200,201,202,203,204,
        205,205,206,207,208,209,210,211,212,213,214,214,215,216,217,218,
        219,220,221,222,222,223,224,225,226,227,228,229,230,230,231,232,
        233,234,235,236,237,237,238,239,240,241,242,243,244,245,245,246,
        247,248,249,250,251,252,252,253,254,255,
    ],
    // Level 2: v_video.c lines 86-100
    [
        4,7,9,11,13,15,17,19,21,22,24,26,27,29,30,32,33,35,36,38,39,40,42,
        43,45,46,47,48,50,51,52,54,55,56,57,59,60,61,62,63,65,66,67,68,69,
        70,72,73,74,75,76,77,78,79,80,82,83,84,85,86,87,88,89,90,91,92,93,
        94,95,96,97,98,100,101,102,103,104,105,106,107,108,109,110,111,112,
        113,114,114,115,116,117,118,119,120,121,122,123,124,125,126,127,128,
        129,130,131,132,133,133,134,135,136,137,138,139,140,141,142,143,144,
        144,145,146,147,148,149,150,151,152,153,153,154,155,156,157,158,159,
        160,160,161,162,163,164,165,166,166,167,168,169,170,171,172,172,173,
        174,175,176,177,178,178,179,180,181,182,183,183,184,185,186,187,188,
        188,189,190,191,192,193,193,194,195,196,197,197,198,199,200,201,201,
        202,203,204,205,206,206,207,208,209,210,210,211,212,213,213,214,215,
        216,217,217,218,219,220,221,221,222,223,224,224,225,226,227,228,228,
        229,230,231,231,232,233,234,235,235,236,237,238,238,239,240,241,241,
        242,243,244,244,245,246,247,247,248,249,250,251,251,252,253,254,254,
        255,
    ],
    // Level 3: v_video.c lines 102-116
    [
        8,12,16,19,22,24,27,29,31,34,36,38,40,41,43,45,47,49,50,52,53,55,
        57,58,60,61,63,64,65,67,68,70,71,72,74,75,76,77,79,80,81,82,84,85,
        86,87,88,90,91,92,93,94,95,96,98,99,100,101,102,103,104,105,106,107,
        108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,
        125,126,127,128,129,130,131,132,133,134,135,135,136,137,138,139,140,
        141,142,143,143,144,145,146,147,148,149,150,150,151,152,153,154,155,
        155,156,157,158,159,160,160,161,162,163,164,165,165,166,167,168,169,
        169,170,171,172,173,173,174,175,176,176,177,178,179,180,180,181,182,
        183,183,184,185,186,186,187,188,189,189,190,191,192,192,193,194,195,
        195,196,197,197,198,199,200,200,201,202,202,203,204,205,205,206,207,
        207,208,209,210,210,211,212,212,213,214,214,215,216,216,217,218,219,
        219,220,221,221,222,223,223,224,225,225,226,227,227,228,229,229,230,
        231,231,232,233,233,234,235,235,236,237,237,238,238,239,240,240,241,
        242,242,243,244,244,245,246,246,247,247,248,249,249,250,251,251,252,
        253,253,254,254,255,
    ],
    // Level 4 (most aggressive): v_video.c lines 118-132
    [
        16,23,28,32,36,39,42,45,48,50,53,55,57,60,62,64,66,68,69,71,73,75,76,
        78,80,81,83,84,86,87,89,90,92,93,94,96,97,98,100,101,102,103,105,106,
        107,108,109,110,112,113,114,115,116,117,118,119,120,121,122,123,124,
        125,126,128,128,129,130,131,132,133,134,135,136,137,138,139,140,141,
        142,143,143,144,145,146,147,148,149,150,150,151,152,153,154,155,155,
        156,157,158,159,159,160,161,162,163,163,164,165,166,166,167,168,169,
        169,170,171,172,172,173,174,175,175,176,177,177,178,179,180,180,181,
        182,182,183,184,184,185,186,187,187,188,189,189,190,191,191,192,193,
        193,194,195,195,196,196,197,198,198,199,200,200,201,202,202,203,203,
        204,205,205,206,207,207,208,208,209,210,210,211,211,212,213,213,214,
        214,215,216,216,217,217,218,219,219,220,220,221,221,222,223,223,224,
        224,225,225,226,227,227,228,228,229,229,230,230,231,232,232,233,233,
        234,234,235,235,236,236,237,237,238,239,239,240,240,241,241,242,242,
        243,243,244,244,245,245,246,246,247,247,248,248,249,249,250,250,251,
        251,252,252,253,254,254,255,255,
    ],
];

// =============================================================================
// VideoState — consolidates global state from v_video.c/h
// =============================================================================

/// Video buffer management state.
///
/// Consolidates the global variables from `v_video.c/h` (`screens[5]`,
/// `dirtybox[4]`, `usegamma`) into a single owned struct, per the Rust port's
/// global-state-elimination strategy (AAP §0.7.5).
///
/// # Usage
///
/// ```ignore
/// let mut video = VideoState::new();
/// video.clear_dirty_box();
/// // Draw a patch to screen 0...
/// video.draw_patch(100, 50, 0, &patch_data);
/// ```
pub struct VideoState {
    /// Five screen buffers, each `SCREENWIDTH × SCREENHEIGHT` (320 × 200 =
    /// 64,000) bytes of palettized pixel data.
    ///
    /// - `screens[0]`: Primary display buffer (presented to the window by the
    ///   platform backend's `finish_update`)
    /// - `screens[1]`: Extra buffer used for background storage during wipes
    /// - `screens[2..4]`: Temporary buffers for wipes, menus, etc.
    pub screens: [Vec<u8>; NUM_SCREENS],

    /// Dirty box coordinates `[BOXTOP, BOXBOTTOM, BOXLEFT, BOXRIGHT]`.
    ///
    /// Tracks the bounding rectangle of modified screen regions for partial
    /// update optimization. Cleared to "nothing dirty" via
    /// [`clear_dirty_box`](Self::clear_dirty_box).
    pub dirtybox: [i32; 4],

    /// Current gamma correction level (0–4), used as an index into
    /// [`GAMMATABLE`].
    pub usegamma: i32,
}

impl VideoState {
    /// Initialize the video subsystem — allocates all 5 screen buffers.
    ///
    /// Equivalent to `V_Init` in `v_video.c` (lines 482–493). The original C
    /// code allocates only 4 screens via `I_AllocLow(SCREENWIDTH*SCREENHEIGHT*4)`
    /// with the 5th screen allocated later in `R_Init`. In Rust we allocate all
    /// 5 up front for simplicity — this is a minimal, justified deviation that
    /// does not change behavior since `screens[4]` is always expected to exist.
    pub fn new() -> Self {
        let screen_size = (SCREENWIDTH * SCREENHEIGHT) as usize;
        VideoState {
            screens: [
                vec![0u8; screen_size],
                vec![0u8; screen_size],
                vec![0u8; screen_size],
                vec![0u8; screen_size],
                vec![0u8; screen_size],
            ],
            dirtybox: [0; 4],
            usegamma: 0,
        }
    }

    /// Clear the dirty box to the "nothing dirty" state.
    ///
    /// Equivalent to calling `M_ClearBox(dirtybox)` in the original C code.
    /// Sets `BOXTOP`/`BOXRIGHT` to `i32::MIN` and `BOXBOTTOM`/`BOXLEFT` to
    /// `i32::MAX` so that the first `mark_rect` call will properly initialize
    /// the bounds.
    pub fn clear_dirty_box(&mut self) {
        self.dirtybox[BOXTOP] = i32::MIN;
        self.dirtybox[BOXRIGHT] = i32::MIN;
        self.dirtybox[BOXBOTTOM] = i32::MAX;
        self.dirtybox[BOXLEFT] = i32::MAX;
    }

    /// Mark a screen rectangle as dirty for partial update optimization.
    ///
    /// Equivalent to `V_MarkRect` in `v_video.c` (lines 142–151). Calls the
    /// equivalent of `M_AddToBox` twice — once for the top-left corner and
    /// once for the bottom-right corner — to expand the dirty box to encompass
    /// the given rectangle.
    ///
    /// The `M_AddToBox` logic uses `else if` (not independent `if`s) to match
    /// the original `m_bbox.c` implementation: `BOXTOP` holds the max Y,
    /// `BOXBOTTOM` holds the min Y, `BOXLEFT` holds the min X, and `BOXRIGHT`
    /// holds the max X.
    pub fn mark_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        // M_AddToBox(dirtybox, x, y)
        if x < self.dirtybox[BOXLEFT] {
            self.dirtybox[BOXLEFT] = x;
        } else if x > self.dirtybox[BOXRIGHT] {
            self.dirtybox[BOXRIGHT] = x;
        }
        if y < self.dirtybox[BOXBOTTOM] {
            self.dirtybox[BOXBOTTOM] = y;
        } else if y > self.dirtybox[BOXTOP] {
            self.dirtybox[BOXTOP] = y;
        }

        // M_AddToBox(dirtybox, x + width - 1, y + height - 1)
        let x2 = x + width - 1;
        let y2 = y + height - 1;
        if x2 < self.dirtybox[BOXLEFT] {
            self.dirtybox[BOXLEFT] = x2;
        } else if x2 > self.dirtybox[BOXRIGHT] {
            self.dirtybox[BOXRIGHT] = x2;
        }
        if y2 < self.dirtybox[BOXBOTTOM] {
            self.dirtybox[BOXBOTTOM] = y2;
        } else if y2 > self.dirtybox[BOXTOP] {
            self.dirtybox[BOXTOP] = y2;
        }
    }

    /// Copy a rectangular region from one screen buffer to another.
    ///
    /// Equivalent to `V_CopyRect` in `v_video.c` (lines 157–196). Copies
    /// `width × height` pixels from `(srcx, srcy)` on `srcscrn` to
    /// `(destx, desty)` on `destscrn`. Marks the destination rectangle as
    /// dirty.
    ///
    /// # Panics (debug only)
    ///
    /// Debug-asserts that all coordinates are within screen bounds and that
    /// screen indices are valid (matching the `#ifdef RANGECHECK` guards in
    /// the original C code).
    pub fn copy_rect(
        &mut self,
        srcx: i32,
        srcy: i32,
        srcscrn: usize,
        width: i32,
        height: i32,
        destx: i32,
        desty: i32,
        destscrn: usize,
    ) {
        // Range checking (equivalent to #ifdef RANGECHECK in C)
        debug_assert!(
            srcx >= 0 && srcx + width <= SCREENWIDTH,
            "Bad V_CopyRect srcx"
        );
        debug_assert!(
            srcy >= 0 && srcy + height <= SCREENHEIGHT,
            "Bad V_CopyRect srcy"
        );
        debug_assert!(
            destx >= 0 && destx + width <= SCREENWIDTH,
            "Bad V_CopyRect destx"
        );
        debug_assert!(
            desty >= 0 && desty + height <= SCREENHEIGHT,
            "Bad V_CopyRect desty"
        );
        debug_assert!(srcscrn < NUM_SCREENS, "Bad V_CopyRect srcscrn");
        debug_assert!(destscrn < NUM_SCREENS, "Bad V_CopyRect destscrn");

        self.mark_rect(destx, desty, width, height);

        let sw = SCREENWIDTH as usize;
        let w = width as usize;

        if srcscrn == destscrn {
            // Copy within the same buffer — use copy_within for overlapping
            // safety.
            for row in 0..height as usize {
                let src_off = (srcy as usize + row) * sw + srcx as usize;
                let dst_off = (desty as usize + row) * sw + destx as usize;
                self.screens[srcscrn].copy_within(src_off..src_off + w, dst_off);
            }
        } else {
            // Copy between different buffers. Use split_at_mut to obtain
            // simultaneous references to the source and destination Vecs
            // without any heap allocation.
            let (min_idx, max_idx) = if srcscrn < destscrn {
                (srcscrn, destscrn)
            } else {
                (destscrn, srcscrn)
            };
            let (first_half, second_half) = self.screens.split_at_mut(max_idx);
            let (src_buf, dst_buf) = if srcscrn < destscrn {
                (
                    &first_half[min_idx] as &Vec<u8>,
                    &mut second_half[0] as &mut Vec<u8>,
                )
            } else {
                (
                    &second_half[0] as &Vec<u8>,
                    &mut first_half[min_idx] as &mut Vec<u8>,
                )
            };

            for row in 0..height as usize {
                let src_off = (srcy as usize + row) * sw + srcx as usize;
                let dst_off = (desty as usize + row) * sw + destx as usize;
                dst_buf[dst_off..dst_off + w].copy_from_slice(&src_buf[src_off..src_off + w]);
            }
        }
    }

    /// Draw a column-based patch graphic to a screen buffer.
    ///
    /// Equivalent to `V_DrawPatch` in `v_video.c` (lines 203–263). This is the
    /// primary 2D drawing function used for menu graphics, HUD elements, status
    /// bar widgets, and all patch-based artwork.
    ///
    /// Patches use a column-post format where each column consists of runs of
    /// non-transparent pixels separated by transparent gaps. The raw
    /// `patch_data` bytes are parsed directly (header: `width:i16`,
    /// `height:i16`, `leftoffset:i16`, `topoffset:i16`, then
    /// `columnofs[width]:i32`).
    ///
    /// # Arguments
    ///
    /// * `x`, `y` — Screen position (adjusted by the patch's left/top offsets)
    /// * `scrn` — Target screen buffer index (0–4)
    /// * `patch_data` — Raw patch data bytes as loaded from a WAD lump
    ///
    /// # Behavior on Out-of-Bounds
    ///
    /// If the adjusted patch position exceeds screen bounds, a warning is
    /// logged and the function returns without drawing. This matches the
    /// original C behavior (comment: "No I_Error abort — what is up with
    /// TNT.WAD?").
    pub fn draw_patch(&mut self, x: i32, y: i32, scrn: usize, patch_data: &[u8]) {
        if patch_data.len() < 8 {
            tracing::warn!(
                "V_DrawPatch: patch data too small ({} bytes)",
                patch_data.len()
            );
            return;
        }

        // Parse patch header (8 bytes): width, height, leftoffset, topoffset
        let width = i16::from_le_bytes([patch_data[0], patch_data[1]]) as i32;
        let height = i16::from_le_bytes([patch_data[2], patch_data[3]]) as i32;
        let leftoffset = i16::from_le_bytes([patch_data[4], patch_data[5]]) as i32;
        let topoffset = i16::from_le_bytes([patch_data[6], patch_data[7]]) as i32;

        let x = x - leftoffset;
        let y = y - topoffset;

        // Range check (equivalent to #ifdef RANGECHECK in C)
        // Original comment: "No I_Error abort - what is up with TNT.WAD?"
        if x < 0
            || x + width > SCREENWIDTH
            || y < 0
            || y + height > SCREENHEIGHT
            || scrn >= NUM_SCREENS
        {
            tracing::warn!(
                "V_DrawPatch: bad patch at ({},{}) size {}x{} (ignored)",
                x,
                y,
                width,
                height
            );
            return;
        }

        // Only mark dirty when drawing to the primary display buffer
        if scrn == 0 {
            self.mark_rect(x, y, width, height);
        }

        let sw = SCREENWIDTH as usize;

        for col in 0..width as usize {
            // Read this column's offset from the columnofs table (starts at
            // byte 8 in the patch header, each entry is 4 bytes / i32).
            let col_ofs_pos = 8 + col * 4;
            if col_ofs_pos + 4 > patch_data.len() {
                break;
            }
            let col_ofs = i32::from_le_bytes([
                patch_data[col_ofs_pos],
                patch_data[col_ofs_pos + 1],
                patch_data[col_ofs_pos + 2],
                patch_data[col_ofs_pos + 3],
            ]) as usize;

            // Walk through the column's post list
            let mut post_offset = col_ofs;
            loop {
                if post_offset >= patch_data.len() {
                    break;
                }

                let topdelta = patch_data[post_offset];
                if topdelta == 0xFF {
                    break; // End-of-column sentinel
                }

                if post_offset + 1 >= patch_data.len() {
                    break;
                }
                let length = patch_data[post_offset + 1] as usize;

                // Source pixels start 3 bytes into the post (skip topdelta,
                // length, and one padding byte).
                let source_start = post_offset + 3;

                // Destination: start at (x + col, y + topdelta), writing
                // vertically down the column.
                let dest_start = (y as usize + topdelta as usize) * sw + (x as usize + col);

                for i in 0..length {
                    if source_start + i < patch_data.len() {
                        let dest_idx = dest_start + i * sw;
                        if dest_idx < self.screens[scrn].len() {
                            self.screens[scrn][dest_idx] = patch_data[source_start + i];
                        }
                    }
                }

                // Advance to next post: topdelta(1) + length(1) + pre-pad(1)
                // + data(length) + post-pad(1) = length + 4
                post_offset += length + 4;
            }
        }
    }

    /// Draw a horizontally flipped patch to a screen buffer.
    ///
    /// Equivalent to `V_DrawPatchFlipped` in `v_video.c` (lines 270–328).
    /// Identical to [`draw_patch`](Self::draw_patch) except that columns are
    /// read in reverse order (`columnofs[width - 1 - col]` instead of
    /// `columnofs[col]`), producing a mirror image. Used for flipping the
    /// player's face in the status bar.
    ///
    /// # Behavior on Out-of-Bounds
    ///
    /// Unlike `draw_patch` (which merely warns), the original C code calls
    /// `I_Error` for range violations. In the Rust port, an error is logged
    /// and the function returns early.
    pub fn draw_patch_flipped(&mut self, x: i32, y: i32, scrn: usize, patch_data: &[u8]) {
        if patch_data.len() < 8 {
            tracing::error!(
                "V_DrawPatchFlipped: patch data too small ({} bytes)",
                patch_data.len()
            );
            return;
        }

        // Parse patch header
        let width = i16::from_le_bytes([patch_data[0], patch_data[1]]) as i32;
        let height = i16::from_le_bytes([patch_data[2], patch_data[3]]) as i32;
        let leftoffset = i16::from_le_bytes([patch_data[4], patch_data[5]]) as i32;
        let topoffset = i16::from_le_bytes([patch_data[6], patch_data[7]]) as i32;

        let x = x - leftoffset;
        let y = y - topoffset;

        // Range check — original C calls I_Error here (stricter than draw_patch)
        if x < 0
            || x + width > SCREENWIDTH
            || y < 0
            || y + height > SCREENHEIGHT
            || scrn >= NUM_SCREENS
        {
            tracing::error!(
                "Bad V_DrawPatch in V_DrawPatchFlipped: origin ({},{})",
                x,
                y
            );
            return;
        }

        // Only mark dirty when drawing to the primary display buffer
        if scrn == 0 {
            self.mark_rect(x, y, width, height);
        }

        let sw = SCREENWIDTH as usize;
        let w = width as usize;

        for col in 0..w {
            // KEY DIFFERENCE: read columns in reverse order for horizontal flip
            let flipped_col = w - 1 - col;
            let col_ofs_pos = 8 + flipped_col * 4;
            if col_ofs_pos + 4 > patch_data.len() {
                break;
            }
            let col_ofs = i32::from_le_bytes([
                patch_data[col_ofs_pos],
                patch_data[col_ofs_pos + 1],
                patch_data[col_ofs_pos + 2],
                patch_data[col_ofs_pos + 3],
            ]) as usize;

            // Walk through the column's post list
            let mut post_offset = col_ofs;
            loop {
                if post_offset >= patch_data.len() {
                    break;
                }

                let topdelta = patch_data[post_offset];
                if topdelta == 0xFF {
                    break;
                }

                if post_offset + 1 >= patch_data.len() {
                    break;
                }
                let length = patch_data[post_offset + 1] as usize;

                let source_start = post_offset + 3;
                let dest_start = (y as usize + topdelta as usize) * sw + (x as usize + col);

                for i in 0..length {
                    if source_start + i < patch_data.len() {
                        let dest_idx = dest_start + i * sw;
                        if dest_idx < self.screens[scrn].len() {
                            self.screens[scrn][dest_idx] = patch_data[source_start + i];
                        }
                    }
                }

                post_offset += length + 4;
            }
        }
    }

    /// Draw a patch directly to the screen.
    ///
    /// Equivalent to `V_DrawPatchDirect` in `v_video.c` (lines 336–396). On
    /// the original Linux port (and all modern systems), this simply delegates
    /// to [`draw_patch`](Self::draw_patch). The commented-out code in the C
    /// source was for VGA Mode X (planar) direct screen writes on DOS — it is
    /// purely historical and not ported.
    pub fn draw_patch_direct(&mut self, x: i32, y: i32, scrn: usize, patch_data: &[u8]) {
        self.draw_patch(x, y, scrn, patch_data);
    }

    /// Draw a linear block of pixels into a screen buffer.
    ///
    /// Equivalent to `V_DrawBlock` in `v_video.c` (lines 404–436). Copies a
    /// `width × height` rectangle of raw pixel data from `src` into the
    /// specified screen buffer at `(x, y)`. The source data is stored in
    /// row-major order with `width` bytes per row.
    ///
    /// # Panics (debug only)
    ///
    /// Debug-asserts that all coordinates are within screen bounds.
    pub fn draw_block(&mut self, x: i32, y: i32, scrn: usize, width: i32, height: i32, src: &[u8]) {
        debug_assert!(x >= 0 && x + width <= SCREENWIDTH, "Bad V_DrawBlock x");
        debug_assert!(y >= 0 && y + height <= SCREENHEIGHT, "Bad V_DrawBlock y");
        debug_assert!(scrn < NUM_SCREENS, "Bad V_DrawBlock scrn");

        self.mark_rect(x, y, width, height);

        let sw = SCREENWIDTH as usize;
        let w = width as usize;
        let mut src_offset = 0;
        let mut dest_offset = y as usize * sw + x as usize;

        for _ in 0..height {
            self.screens[scrn][dest_offset..dest_offset + w]
                .copy_from_slice(&src[src_offset..src_offset + w]);
            src_offset += w;
            dest_offset += sw;
        }
    }

    /// Read a linear block of pixels from a screen buffer.
    ///
    /// Equivalent to `V_GetBlock` in `v_video.c` (lines 444–474). Copies a
    /// `width × height` rectangle of pixel data from the specified screen
    /// buffer at `(x, y)` into `dest`. The destination buffer is filled in
    /// row-major order with `width` bytes per row.
    ///
    /// # Panics (debug only)
    ///
    /// Debug-asserts that all coordinates are within screen bounds.
    pub fn get_block(&self, x: i32, y: i32, scrn: usize, width: i32, height: i32, dest: &mut [u8]) {
        debug_assert!(x >= 0 && x + width <= SCREENWIDTH, "Bad V_GetBlock x");
        debug_assert!(y >= 0 && y + height <= SCREENHEIGHT, "Bad V_GetBlock y");
        debug_assert!(scrn < NUM_SCREENS, "Bad V_GetBlock scrn");

        let sw = SCREENWIDTH as usize;
        let w = width as usize;
        let mut src_offset = y as usize * sw + x as usize;
        let mut dest_offset = 0;

        for _ in 0..height {
            dest[dest_offset..dest_offset + w]
                .copy_from_slice(&self.screens[scrn][src_offset..src_offset + w]);
            src_offset += sw;
            dest_offset += w;
        }
    }
}

impl Default for VideoState {
    /// Creates a new [`VideoState`] with all buffers zero-initialized.
    fn default() -> Self {
        Self::new()
    }
}
