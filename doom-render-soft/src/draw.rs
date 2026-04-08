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

//! Translated from linuxdoom-1.10/r_draw.c, r_draw.h, and README.asm.
//!
//! The actual span/column drawing functions. These are the innermost rendering
//! loops — the "hot path" of the renderer. Assembly inner loops from README.asm
//! (R_DrawColumn using self-modifying code, R_DrawSpan using composite position
//! packing with `shldl` instructions) are reimplemented as safe Rust with
//! equivalent fixed-point stepping behavior.
//!
//! Column drawers handle walls, fuzz/spectre effects, and translated (player
//! color-remapped) sprites. Span drawers handle floors and ceilings. Both
//! operate on palettized 8-bit screen buffers at 320×200 native resolution.
//!
//! Border patches conform to the [`Patch`] format and are drawn via
//! [`VideoState::draw_patch`] during [`DrawState::fill_back_screen`].

use crate::defs::{FRACBITS, SCREENHEIGHT, SCREENWIDTH};
// Patch type represents the format of border graphics (brdr_t/b/l/r, brdr_tl/tr/bl/br)
// loaded in fill_back_screen and drawn via VideoState::draw_patch.
#[allow(unused_imports)]
use crate::defs::Patch;
use doom_core::types::doomdef::GameMode;
use doom_core::video::VideoState;
use doom_wad::PurgeTag;
use doom_wad::WadProvider;

// ---------------------------------------------------------------------------
// Constants (r_draw.c:84-86, r_draw.h)
// ---------------------------------------------------------------------------

/// Maximum supported screen width for lookup table sizing (r_draw.c:84).
pub const MAXWIDTH: usize = 1120;

/// Maximum supported screen height for lookup table sizing (r_draw.c:85).
pub const MAXHEIGHT: usize = 832;

/// Status bar height in pixels (r_draw.c:86).
pub const SBARHEIGHT: usize = 32;

/// Size of the fuzz offset table (r_draw.c:259).
const FUZZTABLE: usize = 50;

/// Fuzz offset equals one screen row width (r_draw.c:260).
/// Used as vertical displacement for the spectre/invisibility shimmer effect.
const FUZZOFF: i32 = SCREENWIDTH;

// ---------------------------------------------------------------------------
// Fuzz offset table (r_draw.c:263-272)
// ---------------------------------------------------------------------------
// Exact 50 values: alternating +FUZZOFF / -FUZZOFF pattern that creates the
// characteristic shimmering spectre/partial-invisibility visual effect.
const FUZZ_OFFSET: [i32; FUZZTABLE] = [
    FUZZOFF, -FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF,
    FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, -FUZZOFF, -FUZZOFF,
    -FUZZOFF, FUZZOFF, -FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF,
    -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, -FUZZOFF, -FUZZOFF,
    -FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF, FUZZOFF, -FUZZOFF, FUZZOFF,
];

// ---------------------------------------------------------------------------
// DrawState — all mutable renderer drawing state (r_draw.c globals)
// ---------------------------------------------------------------------------

/// Holds all mutable state for the column and span drawing subsystem.
///
/// In the original C code these were file-scope globals in `r_draw.c`.
/// Consolidating them into a struct eliminates `static mut` and enables
/// the Rust borrow checker to enforce safe concurrent access.
pub struct DrawState {
    // -- Screen geometry (r_draw.c:81-85) --
    /// Index of the screen buffer used as the drawing target (r_draw.c:81).
    pub viewimage: usize,
    /// Width of the 3-D view window in pixels (r_draw.c:82).
    pub viewwidth: i32,
    /// Scaled view width accounting for detail level (r_draw.c:83).
    pub scaledviewwidth: i32,
    /// Height of the 3-D view window in pixels.
    pub viewheight: i32,
    /// X offset of the view window within the screen (r_draw.c:84).
    pub viewwindowx: i32,
    /// Y offset of the view window within the screen (r_draw.c:85).
    pub viewwindowy: i32,

    // -- Framebuffer addressing lookup tables (r_draw.c:87-88) --
    /// Row start byte-offsets into the primary screen buffer.
    /// `ylookup[row]` = `(row + viewwindowy) * SCREENWIDTH`.
    pub ylookup: [usize; MAXHEIGHT],
    /// Column pixel offsets added to a row base to find a specific pixel.
    /// `columnofs[col]` = `viewwindowx + col`.
    pub columnofs: [i32; MAXWIDTH],

    // -- Translation tables for player sprite recoloring (r_draw.c:383-384) --
    /// Three 256-byte palette translation tables (green → gray / brown / red).
    /// Total size: 3 × 256 = 768 bytes.
    pub translationtables: Vec<u8>,
    /// Byte offset into [`translationtables`](Self::translationtables) selecting
    /// the current player color remap.
    pub dc_translation: usize,

    // -- Column drawing parameters (dc_ prefix, r_draw.h:40-49) --
    /// Base index into the colormaps array for the current column's light level.
    pub dc_colormap: usize,
    /// Screen X coordinate of the column being drawn (r_draw.h:41).
    pub dc_x: i32,
    /// Top Y coordinate (inclusive) of the column (r_draw.h:42).
    pub dc_yl: i32,
    /// Bottom Y coordinate (inclusive) of the column (r_draw.h:43).
    pub dc_yh: i32,
    /// Inverse vertical scale (fixed 16.16) for texture stepping (r_draw.h:45).
    pub dc_iscale: i32,
    /// Texture vertical origin (fixed 16.16) (r_draw.h:47).
    pub dc_texturemid: i32,
    /// Source texture column pixel data (r_draw.h:49).
    pub dc_source: Vec<u8>,

    // -- Span drawing parameters (ds_ prefix, r_draw.c:500-512) --
    /// Screen Y coordinate of the span being drawn (r_draw.c:500).
    pub ds_y: i32,
    /// Left X coordinate (inclusive) of the span (r_draw.c:501).
    pub ds_x1: i32,
    /// Right X coordinate (inclusive) of the span (r_draw.c:502).
    pub ds_x2: i32,
    /// Base index into the colormaps array for the current span's light level.
    pub ds_colormap: usize,
    /// Span X texture coordinate (fixed 16.16) (r_draw.c:506).
    pub ds_xfrac: i32,
    /// Span Y texture coordinate (fixed 16.16) (r_draw.c:507).
    pub ds_yfrac: i32,
    /// Span X texture stepping (fixed 16.16) (r_draw.c:508).
    pub ds_xstep: i32,
    /// Span Y texture stepping (fixed 16.16) (r_draw.c:509).
    pub ds_ystep: i32,
    /// Source flat tile data (64×64 = 4096 bytes) (r_draw.c:512).
    pub ds_source: Vec<u8>,

    // -- Fuzz effect state (r_draw.c:274) --
    /// Current position in the [`FUZZ_OFFSET`] table; cycles 0..FUZZTABLE-1.
    fuzzpos: usize,
}

impl DrawState {
    /// Creates a new `DrawState` with all fields zeroed / empty.
    pub fn new() -> Self {
        Self {
            viewimage: 0,
            viewwidth: 0,
            scaledviewwidth: 0,
            viewheight: 0,
            viewwindowx: 0,
            viewwindowy: 0,
            ylookup: [0usize; MAXHEIGHT],
            columnofs: [0i32; MAXWIDTH],
            translationtables: Vec::new(),
            dc_translation: 0,
            dc_colormap: 0,
            dc_x: 0,
            dc_yl: 0,
            dc_yh: 0,
            dc_iscale: 0,
            dc_texturemid: 0,
            dc_source: Vec::new(),
            ds_y: 0,
            ds_x1: 0,
            ds_x2: 0,
            ds_colormap: 0,
            ds_xfrac: 0,
            ds_yfrac: 0,
            ds_xstep: 0,
            ds_ystep: 0,
            ds_source: Vec::new(),
            fuzzpos: 0,
        }
    }
}

impl Default for DrawState {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Drawing methods
// ===========================================================================

impl DrawState {
    // -----------------------------------------------------------------------
    // R_DrawColumn (r_draw.c:105-148) — primary wall texture column drawer
    // -----------------------------------------------------------------------
    // Reimplements the README.asm inner loop:
    //   dc_colormap[ dc_source[ (frac >> FRACBITS) & 127 ] ]
    // The C code does:
    //   fracstep = dc_iscale;
    //   frac = dc_texturemid + (dc_yl - centery) * fracstep;
    //   do { *dest = colormap[source[(frac>>FRACBITS)&127]]; dest+=SCREENWIDTH; frac+=fracstep; } while(count--);

    /// Draws a single vertical column of wall texture pixels to `screens[0]`.
    ///
    /// This is the core inner loop of the renderer — every wall pixel on screen
    /// passes through this function (or its low-detail variant). The fixed-point
    /// texture coordinate `frac` is stepped by `dc_iscale` per row, and the
    /// source texture index is masked to 128 for power-of-two wrapping.
    ///
    /// # Parameters
    /// - `screens` — framebuffer array; writes to `screens[0]`
    /// - `colormaps` — global colormap table (34 × 256 bytes)
    /// - `centery` — vertical centre of the view (typically `SCREENHEIGHT / 2`)
    pub fn draw_column(&mut self, screens: &mut [Vec<u8>], colormaps: &[u8], centery: i32) {
        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        // Safety: dc_yl and dc_x must be within screen bounds.
        let dc_yl = self.dc_yl as usize;
        let dc_x = self.dc_x as usize;

        if dc_yl >= MAXHEIGHT || dc_x >= MAXWIDTH {
            return;
        }

        let mut dest = self.ylookup[dc_yl].wrapping_add(self.columnofs[dc_x] as usize);

        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - centery).wrapping_mul(fracstep));

        let screen = &mut screens[0];
        let screen_len = screen.len();

        // do { ... } while (count--)  — draws count+1 pixels
        let mut remaining = count;
        loop {
            // Texture column index: (frac >> 16) & 127
            let tex_idx = ((frac >> FRACBITS) & 127) as usize;
            let src_pixel = if tex_idx < self.dc_source.len() {
                self.dc_source[tex_idx] as usize
            } else {
                0
            };
            let cm_idx = self.dc_colormap.wrapping_add(src_pixel);
            if dest < screen_len {
                screen[dest] = if cm_idx < colormaps.len() {
                    colormaps[cm_idx]
                } else {
                    0
                };
            }

            dest = dest.wrapping_add(SCREENWIDTH as usize);
            frac = frac.wrapping_add(fracstep);

            if remaining == 0 {
                break;
            }
            remaining -= 1;
        }
    }

    // -----------------------------------------------------------------------
    // R_DrawColumnLow (r_draw.c:211-253) — low-detail (blocky) column drawer
    // -----------------------------------------------------------------------

    /// Draws a wall column in low-detail (blocky) mode, writing two adjacent
    /// pixels per row to create a doubled-width column.
    pub fn draw_column_low(&mut self, screens: &mut [Vec<u8>], colormaps: &[u8], centery: i32) {
        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        // In low-detail mode the x coordinate is doubled (r_draw.c:219).
        let dc_x_doubled = (self.dc_x << 1) as usize;
        let dc_yl = self.dc_yl as usize;

        if dc_yl >= MAXHEIGHT || dc_x_doubled >= MAXWIDTH {
            return;
        }

        // Primary destination and secondary (one pixel right).
        let base = self.ylookup[dc_yl];
        let mut dest = base.wrapping_add(self.columnofs[dc_x_doubled] as usize);
        let mut dest2 = base.wrapping_add(if dc_x_doubled + 1 < MAXWIDTH {
            self.columnofs[dc_x_doubled + 1] as usize
        } else {
            self.columnofs[dc_x_doubled] as usize
        });

        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - centery).wrapping_mul(fracstep));

        let screen = &mut screens[0];
        let screen_len = screen.len();

        let mut remaining = count;
        loop {
            let tex_idx = ((frac >> FRACBITS) & 127) as usize;
            let src_pixel = if tex_idx < self.dc_source.len() {
                self.dc_source[tex_idx] as usize
            } else {
                0
            };
            let cm_idx = self.dc_colormap.wrapping_add(src_pixel);
            let pixel = if cm_idx < colormaps.len() {
                colormaps[cm_idx]
            } else {
                0
            };

            if dest < screen_len {
                screen[dest] = pixel;
            }
            if dest2 < screen_len {
                screen[dest2] = pixel;
            }

            dest = dest.wrapping_add(SCREENWIDTH as usize);
            dest2 = dest2.wrapping_add(SCREENWIDTH as usize);
            frac = frac.wrapping_add(fracstep);

            if remaining == 0 {
                break;
            }
            remaining -= 1;
        }
    }
}

// ===========================================================================
// Fuzz and translated column drawers
// ===========================================================================

impl DrawState {
    // -----------------------------------------------------------------------
    // R_DrawFuzzColumn (r_draw.c:285-368) — spectre / partial invisibility
    // -----------------------------------------------------------------------
    // The fuzz effect reads the pixel at (dest ± SCREENWIDTH) from the
    // framebuffer itself, then maps it through colormap #6 (a dark map).
    // This creates the shimmering, partially-transparent spectre look.
    // The dc_yl/dc_yh are clamped to [1, viewheight-2] to ensure the
    // vertical offset never reads outside the screen buffer.

    /// Draws a fuzz (spectre / partial-invisibility) column.
    ///
    /// Unlike normal columns, this ignores the texture source entirely.
    /// Instead it reads adjacent pixels from the framebuffer and darkens
    /// them through colormap #6, producing the classic shimmer effect.
    pub fn draw_fuzz_column(&mut self, screens: &mut [Vec<u8>], colormaps: &[u8]) {
        // Clamp to avoid reading outside screen bounds (r_draw.c:296-300).
        if self.dc_yl <= 0 {
            self.dc_yl = 1;
        }
        if self.dc_yh >= self.viewheight - 1 {
            self.dc_yh = self.viewheight - 2;
        }

        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let dc_yl = self.dc_yl as usize;
        let dc_x = self.dc_x as usize;

        if dc_yl >= MAXHEIGHT || dc_x >= MAXWIDTH {
            return;
        }

        let mut dest = self.ylookup[dc_yl].wrapping_add(self.columnofs[dc_x] as usize);

        let screen = &mut screens[0];
        let screen_len = screen.len();

        // Colormap #6 base offset — dark colormap for the fuzz effect.
        let dark_cm_base: usize = 6 * 256;

        let mut remaining = count;
        loop {
            // Read a pixel at (dest + fuzz_offset) from the screen, then
            // darken it through colormap 6 (r_draw.c:338).
            let fuzz_offset = FUZZ_OFFSET[self.fuzzpos];
            let source_idx = (dest as i64 + fuzz_offset as i64) as usize;
            let source_pixel = if source_idx < screen_len {
                screen[source_idx] as usize
            } else {
                0
            };
            let cm_idx = dark_cm_base + source_pixel;
            if dest < screen_len {
                screen[dest] = if cm_idx < colormaps.len() {
                    colormaps[cm_idx]
                } else {
                    0
                };
            }

            // Advance fuzz position, wrapping at FUZZTABLE (r_draw.c:341).
            self.fuzzpos += 1;
            if self.fuzzpos >= FUZZTABLE {
                self.fuzzpos = 0;
            }

            dest = dest.wrapping_add(SCREENWIDTH as usize);

            if remaining == 0 {
                break;
            }
            remaining -= 1;
        }
    }

    // -----------------------------------------------------------------------
    // R_DrawFuzzColumnLow — low-detail fuzz column
    // -----------------------------------------------------------------------

    /// Low-detail (blocky) variant of [`draw_fuzz_column`](Self::draw_fuzz_column).
    /// Writes two adjacent pixels per row.
    pub fn draw_fuzz_column_low(&mut self, screens: &mut [Vec<u8>], colormaps: &[u8]) {
        if self.dc_yl <= 0 {
            self.dc_yl = 1;
        }
        if self.dc_yh >= self.viewheight - 1 {
            self.dc_yh = self.viewheight - 2;
        }

        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let dc_x_doubled = (self.dc_x << 1) as usize;
        let dc_yl = self.dc_yl as usize;

        if dc_yl >= MAXHEIGHT || dc_x_doubled >= MAXWIDTH {
            return;
        }

        let base = self.ylookup[dc_yl];
        let mut dest = base.wrapping_add(self.columnofs[dc_x_doubled] as usize);
        let mut dest2 = base.wrapping_add(if dc_x_doubled + 1 < MAXWIDTH {
            self.columnofs[dc_x_doubled + 1] as usize
        } else {
            self.columnofs[dc_x_doubled] as usize
        });

        let screen = &mut screens[0];
        let screen_len = screen.len();
        let dark_cm_base: usize = 6 * 256;

        let mut remaining = count;
        loop {
            let fuzz_offset = FUZZ_OFFSET[self.fuzzpos];
            let source_idx = (dest as i64 + fuzz_offset as i64) as usize;
            let source_pixel = if source_idx < screen_len {
                screen[source_idx] as usize
            } else {
                0
            };
            let cm_idx = dark_cm_base + source_pixel;
            let pixel = if cm_idx < colormaps.len() {
                colormaps[cm_idx]
            } else {
                0
            };

            if dest < screen_len {
                screen[dest] = pixel;
            }
            if dest2 < screen_len {
                screen[dest2] = pixel;
            }

            self.fuzzpos += 1;
            if self.fuzzpos >= FUZZTABLE {
                self.fuzzpos = 0;
            }

            dest = dest.wrapping_add(SCREENWIDTH as usize);
            dest2 = dest2.wrapping_add(SCREENWIDTH as usize);

            if remaining == 0 {
                break;
            }
            remaining -= 1;
        }
    }

    // -----------------------------------------------------------------------
    // R_DrawTranslatedColumn (r_draw.c:385-447) — player sprite recoloring
    // -----------------------------------------------------------------------
    // Same structure as draw_column but with an extra indirection through the
    // translation table: colormap[ translationtable[ source[frac>>FRACBITS] ] ]
    // Note: NO &127 mask on the source index (unlike draw_column).

    /// Draws a wall column with palette translation applied (player sprite
    /// recoloring). The green colour ramp (0x70-0x7f) is remapped to the
    /// player's team colour via [`translationtables`](Self::translationtables).
    pub fn draw_translated_column(
        &mut self,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
        centery: i32,
    ) {
        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let dc_yl = self.dc_yl as usize;
        let dc_x = self.dc_x as usize;

        if dc_yl >= MAXHEIGHT || dc_x >= MAXWIDTH {
            return;
        }

        let mut dest = self.ylookup[dc_yl].wrapping_add(self.columnofs[dc_x] as usize);

        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - centery).wrapping_mul(fracstep));

        let screen = &mut screens[0];
        let screen_len = screen.len();

        let mut remaining = count;
        loop {
            // Note: no &127 mask here — translated columns come from sprites
            // which have explicit post lengths (r_draw.c:432).
            let tex_idx = (frac >> FRACBITS) as usize & 0xFF;
            let src_pixel = if tex_idx < self.dc_source.len() {
                self.dc_source[tex_idx]
            } else {
                0
            };

            // Apply colour translation, then colormap (r_draw.c:433-434).
            let translated =
                if self.dc_translation + (src_pixel as usize) < self.translationtables.len() {
                    self.translationtables[self.dc_translation + src_pixel as usize] as usize
                } else {
                    src_pixel as usize
                };

            let cm_idx = self.dc_colormap.wrapping_add(translated);
            if dest < screen_len {
                screen[dest] = if cm_idx < colormaps.len() {
                    colormaps[cm_idx]
                } else {
                    0
                };
            }

            dest = dest.wrapping_add(SCREENWIDTH as usize);
            frac = frac.wrapping_add(fracstep);

            if remaining == 0 {
                break;
            }
            remaining -= 1;
        }
    }

    // -----------------------------------------------------------------------
    // R_DrawTranslatedColumnLow — low-detail translated column
    // -----------------------------------------------------------------------

    /// Low-detail (blocky) variant of
    /// [`draw_translated_column`](Self::draw_translated_column).
    pub fn draw_translated_column_low(
        &mut self,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
        centery: i32,
    ) {
        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        let dc_x_doubled = (self.dc_x << 1) as usize;
        let dc_yl = self.dc_yl as usize;

        if dc_yl >= MAXHEIGHT || dc_x_doubled >= MAXWIDTH {
            return;
        }

        let base = self.ylookup[dc_yl];
        let mut dest = base.wrapping_add(self.columnofs[dc_x_doubled] as usize);
        let mut dest2 = base.wrapping_add(if dc_x_doubled + 1 < MAXWIDTH {
            self.columnofs[dc_x_doubled + 1] as usize
        } else {
            self.columnofs[dc_x_doubled] as usize
        });

        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - centery).wrapping_mul(fracstep));

        let screen = &mut screens[0];
        let screen_len = screen.len();

        let mut remaining = count;
        loop {
            let tex_idx = (frac >> FRACBITS) as usize & 0xFF;
            let src_pixel = if tex_idx < self.dc_source.len() {
                self.dc_source[tex_idx]
            } else {
                0
            };

            let translated =
                if self.dc_translation + (src_pixel as usize) < self.translationtables.len() {
                    self.translationtables[self.dc_translation + src_pixel as usize] as usize
                } else {
                    src_pixel as usize
                };

            let cm_idx = self.dc_colormap.wrapping_add(translated);
            let pixel = if cm_idx < colormaps.len() {
                colormaps[cm_idx]
            } else {
                0
            };

            if dest < screen_len {
                screen[dest] = pixel;
            }
            if dest2 < screen_len {
                screen[dest2] = pixel;
            }

            dest = dest.wrapping_add(SCREENWIDTH as usize);
            dest2 = dest2.wrapping_add(SCREENWIDTH as usize);
            frac = frac.wrapping_add(fracstep);

            if remaining == 0 {
                break;
            }
            remaining -= 1;
        }
    }
}

// ===========================================================================
// Span drawers (floor / ceiling)
// ===========================================================================

impl DrawState {
    // -----------------------------------------------------------------------
    // R_DrawSpan (r_draw.c:520-563) — primary floor/ceiling span drawer
    // -----------------------------------------------------------------------
    // Reimplements the README.asm inner loop. The "spot" calculation combines
    // 6 bits of Y-fraction and 6 bits of X-fraction into a 12-bit index for
    // the 64×64 flat tile:
    //   spot = ((yfrac >> (16-6)) & (63*64)) + ((xfrac >> 16) & 63)
    // This is equivalent to the assembly's shldl/andl composite packing.

    /// Draws a horizontal span of floor or ceiling pixels to `screens[0]`.
    ///
    /// This is the second core inner loop of the renderer. The 64×64 flat
    /// texture is addressed by combining 6 bits each from the fixed-point
    /// X and Y texture coordinates into a 12-bit tile index.
    pub fn draw_span(&mut self, screens: &mut [Vec<u8>], colormaps: &[u8]) {
        let count = self.ds_x2 - self.ds_x1;
        if count < 0 {
            return;
        }

        let ds_y = self.ds_y as usize;
        let ds_x1 = self.ds_x1 as usize;

        if ds_y >= MAXHEIGHT || ds_x1 >= MAXWIDTH {
            return;
        }

        let mut dest = self.ylookup[ds_y].wrapping_add(self.columnofs[ds_x1] as usize);

        let mut xfrac = self.ds_xfrac;
        let mut yfrac = self.ds_yfrac;

        let screen = &mut screens[0];
        let screen_len = screen.len();

        // do { ... } while (count--)  — draws count+1 pixels
        let mut remaining = count;
        loop {
            // 64×64 flat tile index: 6 bits Y × 64 + 6 bits X (r_draw.c:546).
            let spot = (((yfrac >> (16 - 6)) & (63 * 64)) + ((xfrac >> 16) & 63)) as usize;

            let src_pixel = if spot < self.ds_source.len() {
                self.ds_source[spot] as usize
            } else {
                0
            };

            let cm_idx = self.ds_colormap.wrapping_add(src_pixel);
            if dest < screen_len {
                screen[dest] = if cm_idx < colormaps.len() {
                    colormaps[cm_idx]
                } else {
                    0
                };
            }

            dest = dest.wrapping_add(1);
            xfrac = xfrac.wrapping_add(self.ds_xstep);
            yfrac = yfrac.wrapping_add(self.ds_ystep);

            if remaining == 0 {
                break;
            }
            remaining -= 1;
        }
    }

    // -----------------------------------------------------------------------
    // R_DrawSpanLow (r_draw.c:643-686) — low-detail floor/ceiling span
    // -----------------------------------------------------------------------

    /// Low-detail (blocky) variant of [`draw_span`](Self::draw_span).
    /// Coordinates are doubled and each pixel is written twice.
    pub fn draw_span_low(&mut self, screens: &mut [Vec<u8>], colormaps: &[u8]) {
        // Double the x coordinates for low-detail mode (r_draw.c:659-660).
        let ds_x1 = (self.ds_x1 << 1) as usize;
        let ds_x2 = (self.ds_x2 << 1) as usize;

        let count = ds_x2 as i32 - ds_x1 as i32;
        if count < 0 {
            return;
        }

        let ds_y = self.ds_y as usize;
        if ds_y >= MAXHEIGHT || ds_x1 >= MAXWIDTH {
            return;
        }

        let mut dest = self.ylookup[ds_y].wrapping_add(self.columnofs[ds_x1] as usize);

        let mut xfrac = self.ds_xfrac;
        let mut yfrac = self.ds_yfrac;

        let screen = &mut screens[0];
        let screen_len = screen.len();

        let mut remaining = count;
        loop {
            let spot = (((yfrac >> (16 - 6)) & (63 * 64)) + ((xfrac >> 16) & 63)) as usize;

            let src_pixel = if spot < self.ds_source.len() {
                self.ds_source[spot] as usize
            } else {
                0
            };

            let cm_idx = self.ds_colormap.wrapping_add(src_pixel);
            let pixel = if cm_idx < colormaps.len() {
                colormaps[cm_idx]
            } else {
                0
            };

            // Write the pixel twice for low-detail doubling (r_draw.c:681-682).
            if dest < screen_len {
                screen[dest] = pixel;
            }
            if dest + 1 < screen_len {
                screen[dest + 1] = pixel;
            }

            dest = dest.wrapping_add(2);
            xfrac = xfrac.wrapping_add(self.ds_xstep);
            yfrac = yfrac.wrapping_add(self.ds_ystep);

            if remaining == 0 {
                break;
            }
            remaining -= 1;
        }
    }
}

// ===========================================================================
// Initialisation and utility functions
// ===========================================================================

impl DrawState {
    // -----------------------------------------------------------------------
    // R_InitBuffer (r_draw.c:696-720) — framebuffer addressing setup
    // -----------------------------------------------------------------------

    /// Sets up the `ylookup` and `columnofs` lookup tables for the given view
    /// window dimensions. Must be called whenever the view size changes.
    ///
    /// `ylookup[row]` stores the byte-offset of the start of that row in the
    /// primary screen buffer. `columnofs[col]` stores the column offset
    /// (including `viewwindowx`).
    pub fn init_buffer(&mut self, width: i32, height: i32) {
        self.viewwindowx = (SCREENWIDTH - width) >> 1;

        // Build column offset table: columnofs[i] = viewwindowx + i
        for i in 0..width as usize {
            if i < MAXWIDTH {
                self.columnofs[i] = self.viewwindowx + i as i32;
            }
        }

        // Determine vertical window position.
        // Full-width (320) means the view fills the screen above the status bar
        // and viewwindowy is 0. Otherwise center the view vertically in the
        // area above the status bar.
        if width == SCREENWIDTH {
            self.viewwindowy = 0;
        } else {
            self.viewwindowy = (SCREENHEIGHT - SBARHEIGHT as i32 - height) >> 1;
        }

        // Build row offset table: ylookup[i] = (i + viewwindowy) * SCREENWIDTH
        for i in 0..height as usize {
            if i < MAXHEIGHT {
                self.ylookup[i] = ((i as i32 + self.viewwindowy) * SCREENWIDTH) as usize;
            }
        }
    }

    // -----------------------------------------------------------------------
    // R_InitTranslationTables (r_draw.c:459-483) — player colour remapping
    // -----------------------------------------------------------------------

    /// Creates three 256-byte palette translation tables used for remapping
    /// the green player-sprite colour ramp (palette indices 0x70-0x7f) to
    /// alternate team colours:
    ///
    /// - Table 0: green → gray  (0x60-0x6f)
    /// - Table 1: green → brown (0x40-0x4f)
    /// - Table 2: green → red   (0x20-0x2f)
    ///
    /// All other palette indices map to themselves (identity).
    pub fn init_translation_tables(&mut self) {
        // Allocate 3 × 256 bytes.
        self.translationtables = vec![0u8; 256 * 3];

        for table_idx in 0..3usize {
            let table_base = table_idx * 256;
            for i in 0..256usize {
                if (0x70..=0x7f).contains(&i) {
                    // Remap the green ramp to the target colour ramp.
                    let ramp_offset = i & 0x0f;
                    let target_base = match table_idx {
                        0 => 0x60, // gray
                        1 => 0x40, // brown
                        2 => 0x20, // red
                        _ => 0x70, // unreachable, but identity if hit
                    };
                    self.translationtables[table_base + i] = (target_base + ramp_offset) as u8;
                } else {
                    // Identity mapping for all non-green indices.
                    self.translationtables[table_base + i] = i as u8;
                }
            }
        }
    }
}

// ===========================================================================
// Border / back-screen management
// ===========================================================================

impl DrawState {
    // -----------------------------------------------------------------------
    // R_FillBackScreen (r_draw.c:731-811) — tiled border background
    // -----------------------------------------------------------------------
    // Fills screens[1] with a tiled flat texture and draws border edge/corner
    // patches around the 3-D view window. The flat is FLOOR7_2 for shareware /
    // registered DOOM and GRNROCK for commercial DOOM II.

    /// Tiles the back screen (`video.screens[1]`) with a flat texture and draws
    /// border patches around the 3-D view window.
    ///
    /// Uses [`Patch`]-format border lumps (`brdr_t/b/l/r`, `brdr_tl/tr/bl/br`)
    /// drawn via [`VideoState::draw_patch`]. The flat texture is selected by
    /// game mode: `FLOOR7_2` for DOOM, `GRNROCK` for DOOM II.
    pub fn fill_back_screen(
        &mut self,
        video: &mut VideoState,
        gamemode: GameMode,
        wad: &mut dyn WadProvider,
    ) {
        // If the view window fills the full width, there is no border to draw.
        if self.scaledviewwidth == SCREENWIDTH {
            return;
        }

        // Select the background flat by game mode (r_draw.c:745-746).
        let flat_name = if gamemode == GameMode::Commercial {
            "GRNROCK"
        } else {
            "FLOOR7_2"
        };

        // Load the flat data (64×64 = 4096 bytes). We copy to a Vec because
        // the WadProvider borrow is released before we write to screens.
        let flat_data = wad.cache_lump_name(flat_name, PurgeTag::Cache).to_vec();

        // Tile the flat over the entire back screen area above the status bar
        // (r_draw.c:756-771).
        let fill_height = (SCREENHEIGHT - SBARHEIGHT as i32) as usize;
        let sw = SCREENWIDTH as usize;
        let screen1 = &mut video.screens[1];

        for y in 0..fill_height {
            // Source row within the 64×64 flat tile.
            let src_row_start = (y & 63) << 6;
            let dest_row_start = y * sw;

            for x in 0..sw {
                let src_col = x & 63;
                let src_idx = src_row_start + src_col;
                let dest_idx = dest_row_start + x;
                if dest_idx < screen1.len() && src_idx < flat_data.len() {
                    screen1[dest_idx] = flat_data[src_idx];
                }
            }
        }

        // --- Border edge patches (every 8 pixels along each edge) ---

        // Top edge (r_draw.c:793-795).
        let patch_data = wad.cache_lump_name("brdr_t", PurgeTag::Cache).to_vec();
        let mut x = 0;
        while x < self.scaledviewwidth {
            video.draw_patch(self.viewwindowx + x, self.viewwindowy - 8, 1, &patch_data);
            x += 8;
        }

        // Bottom edge (r_draw.c:797-799).
        let patch_data = wad.cache_lump_name("brdr_b", PurgeTag::Cache).to_vec();
        let mut x = 0;
        while x < self.scaledviewwidth {
            video.draw_patch(
                self.viewwindowx + x,
                self.viewwindowy + self.viewheight,
                1,
                &patch_data,
            );
            x += 8;
        }

        // Left edge (r_draw.c:801-803).
        let patch_data = wad.cache_lump_name("brdr_l", PurgeTag::Cache).to_vec();
        let mut y = 0;
        while y < self.viewheight {
            video.draw_patch(self.viewwindowx - 8, self.viewwindowy + y, 1, &patch_data);
            y += 8;
        }

        // Right edge (r_draw.c:805-807).
        let patch_data = wad.cache_lump_name("brdr_r", PurgeTag::Cache).to_vec();
        let mut y = 0;
        while y < self.viewheight {
            video.draw_patch(
                self.viewwindowx + self.scaledviewwidth,
                self.viewwindowy + y,
                1,
                &patch_data,
            );
            y += 8;
        }

        // --- Corner patches ---

        // Top-left corner (r_draw.c:809).
        let patch_data = wad.cache_lump_name("brdr_tl", PurgeTag::Cache).to_vec();
        video.draw_patch(self.viewwindowx - 8, self.viewwindowy - 8, 1, &patch_data);

        // Top-right corner (r_draw.c:812).
        let patch_data = wad.cache_lump_name("brdr_tr", PurgeTag::Cache).to_vec();
        video.draw_patch(
            self.viewwindowx + self.scaledviewwidth,
            self.viewwindowy - 8,
            1,
            &patch_data,
        );

        // Bottom-left corner (r_draw.c:815).
        let patch_data = wad.cache_lump_name("brdr_bl", PurgeTag::Cache).to_vec();
        video.draw_patch(
            self.viewwindowx - 8,
            self.viewwindowy + self.viewheight,
            1,
            &patch_data,
        );

        // Bottom-right corner (r_draw.c:818).
        let patch_data = wad.cache_lump_name("brdr_br", PurgeTag::Cache).to_vec();
        video.draw_patch(
            self.viewwindowx + self.scaledviewwidth,
            self.viewwindowy + self.viewheight,
            1,
            &patch_data,
        );
    }

    // -----------------------------------------------------------------------
    // R_VideoErase (r_draw.c:817-828) — restore background behind view
    // -----------------------------------------------------------------------

    /// Copies `count` bytes from `screens[1]` to `screens[0]` at byte-offset
    /// `ofs`. Used to restore the tiled background behind a shrunk view window.
    pub fn video_erase(&self, screens: &mut [Vec<u8>], ofs: usize, count: usize) {
        if screens.len() < 2 {
            return;
        }

        // We need to read from screens[1] and write to screens[0].
        // Split the slice to satisfy the borrow checker.
        let (front, back) = screens.split_at_mut(1);
        let screen0 = &mut front[0];
        let screen1 = &back[0]; // immutable borrow of screens[1]

        let end = ofs.saturating_add(count);
        let copy_end = end.min(screen0.len()).min(screen1.len());
        let copy_start = ofs.min(copy_end);

        screen0[copy_start..copy_end].copy_from_slice(&screen1[copy_start..copy_end]);
    }

    // -----------------------------------------------------------------------
    // R_DrawViewBorder (r_draw.c:843-875) — copy border to front screen
    // -----------------------------------------------------------------------

    /// Draws the view border by copying the tiled background from the back
    /// screen (`screens[1]`) to the front screen (`screens[0]`) around the
    /// edges of the 3-D view window. Called each frame when the view window
    /// is smaller than full-screen.
    pub fn draw_view_border(&self, video: &mut VideoState) {
        // No border needed if the view fills the full width (r_draw.c:855).
        if self.scaledviewwidth == SCREENWIDTH {
            return;
        }

        let sw = SCREENWIDTH as usize;
        let top = ((SCREENHEIGHT - SBARHEIGHT as i32 - self.viewheight) / 2) as usize;
        let side = ((SCREENWIDTH - self.scaledviewwidth) / 2) as usize;

        // --- Copy border strips from screens[1] to screens[0] ---
        // We split the screens array so we can read [1] and write [0].
        let (front, back) = video.screens.split_at_mut(1);
        let screen0 = &mut front[0];
        let screen1 = &back[0];

        let screen_len = screen0.len().min(screen1.len());
        let view_w = self.scaledviewwidth as usize;
        let view_h = self.viewheight as usize;

        // Top strip: from row 0 to top of the view window (r_draw.c:860-861).
        {
            let end = (top * sw + side).min(screen_len);
            screen0[..end].copy_from_slice(&screen1[..end]);
        }

        // Bottom strip: from bottom of view window to bottom of border area.
        {
            let ofs = ((top + view_h) * sw).saturating_sub(side);
            let end = ((SCREENHEIGHT - SBARHEIGHT as i32) as usize * sw).min(screen_len);
            let ofs = ofs.min(end);
            screen0[ofs..end].copy_from_slice(&screen1[ofs..end]);
        }

        // Side strips: one row at a time for the height of the view window
        // (r_draw.c:866-873).
        for row in 0..view_h {
            let y = top + row;
            let row_start = y * sw;

            // Left side strip.
            let left_start = row_start;
            let left_end = (row_start + side).min(screen_len);
            let left_start = left_start.min(left_end);
            screen0[left_start..left_end].copy_from_slice(&screen1[left_start..left_end]);

            // Right side strip.
            let right_start = (row_start + side + view_w).min(screen_len);
            let right_end = (row_start + sw).min(screen_len);
            let right_start = right_start.min(right_end);
            screen0[right_start..right_end].copy_from_slice(&screen1[right_start..right_end]);
        }

        // Mark the entire border region as dirty so V_UpdateNoBlit copies it
        // to the display (r_draw.c:875).
        video.mark_rect(0, 0, SCREENWIDTH, SCREENHEIGHT - SBARHEIGHT as i32);
    }
}
