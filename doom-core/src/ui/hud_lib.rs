// Copyright (C) 1993-1996 by id Software, Inc.
// Copyright (C) 2024 DOOM Rust Contributors.
//
// This program is free software; you can redistribute it and/or
// modify it under the terms of the GNU General Public License
// as published by the Free Software Foundation; either version 2
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

//! Heads-up text and input code — widget library.
//!
//! Translated from `linuxdoom-1.10/hu_lib.c` (354 lines) and
//! `linuxdoom-1.10/hu_lib.h` (197 lines).
//!
//! Provides low-level HUD widget primitives: text line, scrolling text,
//! and input text line widgets. These are the building blocks used by the
//! HUD module (`hu_stuff`) for rendering on-screen messages and chat text.
//!
//! # Widget Hierarchy
//!
//! - [`HuTextLine`] — Single line of text (base widget)
//! - [`HuScrollText`] — Scrolling multi-line message area (contains `HuTextLine`s)
//! - [`HuInputText`] — Interactive text entry with cursor (wraps a `HuTextLine`)
//!
//! # Font Convention
//!
//! Font glyphs are stored as raw WAD patch byte arrays (`Vec<u8>`),
//! indexed by `(character_code - start_char)`. The start character is
//! typically `b'!'` (33), covering ASCII printable characters through
//! `b'_'` (95). The [`Patch`] struct documents the WAD patch header
//! layout (width, height, offsets).

use crate::types::doomdef::{KEY_BACKSPACE, KEY_ENTER, KEY_ESCAPE, SCREENHEIGHT, SCREENWIDTH};
use crate::types::map_data::Patch;
use crate::util::swap;
use crate::video::video::VideoState;

// ─── Constants ───────────────────────────────────────────────────

/// Background screen number (source for erase copies).
pub const BG: usize = 1;

/// Foreground screen number (rendering target).
pub const FG: usize = 0;

/// Character code that triggers character erasure (backspace).
pub const HU_CHARERASE: u8 = KEY_BACKSPACE as u8;

/// Maximum number of lines in a scrolling text widget.
pub const HU_MAXLINES: usize = 4;

/// Maximum length of a single text line (excluding null terminator).
pub const HU_MAXLINELENGTH: usize = 80;

// ─── Patch dimension helpers ─────────────────────────────────────

/// Extract the width from raw WAD patch byte data (little-endian `i16` at offset 0).
///
/// This mirrors the `SHORT(patch->width)` pattern from the original C code.
/// The raw byte layout matches the [`Patch`] struct's `width` field.
fn patch_width(data: &[u8]) -> i32 {
    if data.len() < 2 {
        return 0;
    }
    swap::short(i16::from_ne_bytes([data[0], data[1]])) as i32
}

/// Extract the height from raw WAD patch byte data (little-endian `i16` at offset 2).
///
/// This mirrors the `SHORT(patch->height)` pattern from the original C code.
/// The raw byte layout matches the [`Patch`] struct's `height` field.
fn patch_height(data: &[u8]) -> i32 {
    if data.len() < 4 {
        return 0;
    }
    swap::short(i16::from_ne_bytes([data[2], data[3]])) as i32
}

/// Retrieve the width of a glyph from a [`Patch`] struct reference.
///
/// Applies the endian swap to the `Patch.width` field, returning the
/// width in pixels. Useful when callers have parsed Patch structs
/// rather than raw byte data.
#[inline]
pub fn glyph_width_from_patch(p: &Patch) -> i32 {
    swap::short(p.width) as i32
}

/// Retrieve the height of a glyph from a [`Patch`] struct reference.
///
/// Applies the endian swap to the `Patch.height` field, returning the
/// height in pixels.
#[inline]
pub fn glyph_height_from_patch(p: &Patch) -> i32 {
    swap::short(p.height) as i32
}

// ─── HuTextLine ──────────────────────────────────────────────────

/// A single line of HUD text — the fundamental widget.
///
/// Stores the text content, position, font reference, and update state.
/// Font glyphs are stored as raw WAD patch byte arrays, one per character
/// in the font set starting at character code `sc`.
pub struct HuTextLine {
    /// Left-justified X position on screen.
    pub x: i32,
    /// Y position on screen.
    pub y: i32,
    /// Font patch data array — one `Vec<u8>` per glyph, indexed by `(ch - sc)`.
    pub f: Vec<Vec<u8>>,
    /// Start character code of the font (first printable glyph, typically `b'!'`).
    pub sc: i32,
    /// Line of text (null-terminated byte buffer).
    pub l: [u8; HU_MAXLINELENGTH + 1],
    /// Current line length (number of characters before the null terminator).
    pub len: usize,
    /// Update counter: nonzero means the line needs redrawing.
    /// Set to 4 on content changes, decremented each frame by erase.
    pub needs_update: i32,
    /// Per-line tracking of the last automap state, used by erase to detect
    /// automap transitions and force redraws. Replaces the C function-local
    /// `static boolean lastautomapactive`.
    last_automap_active: bool,
}

impl Default for HuTextLine {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            f: Vec::new(),
            sc: 0,
            l: [0u8; HU_MAXLINELENGTH + 1],
            len: 0,
            needs_update: 0,
            last_automap_active: true,
        }
    }
}

// ─── HuScrollText ────────────────────────────────────────────────

/// A scrolling text window composed of multiple [`HuTextLine`] instances.
///
/// Used for the message display area at the top of the screen. New messages
/// are added to the current line, and lines scroll upward when full.
#[derive(Default)]
pub struct HuScrollText {
    /// Text lines in the scrolling window (up to [`HU_MAXLINES`]).
    pub lines: Vec<HuTextLine>,
    /// Number of active lines in the window.
    pub h: i32,
    /// Index of the current (most recently written) line.
    pub cl: i32,
    /// Whether the widget is currently visible and should be drawn/updated.
    pub on: bool,
    /// Previous visibility state, used to detect on→off transitions for erase.
    pub last_on: bool,
}

// ─── HuInputText ─────────────────────────────────────────────────

/// An input text line widget with cursor and left-margin support.
///
/// Used for the chat input line. Characters before the left margin (`lm`)
/// are treated as a fixed prefix that cannot be deleted.
#[derive(Default)]
pub struct HuInputText {
    /// The underlying text line widget.
    pub l: HuTextLine,
    /// Left margin — characters at indices `< lm` cannot be deleted.
    pub lm: usize,
    /// Whether the widget is currently visible and should be drawn/updated.
    pub on: bool,
    /// Previous visibility state, used to detect on→off transitions for erase.
    pub last_on: bool,
}

// ═══════════════════════════════════════════════════════════════════
//  TextLine widget functions
// ═══════════════════════════════════════════════════════════════════

/// Library initialisation — no-op, matches the original `HUlib_init`.
///
/// The original C function body is empty; this preserves the call site
/// contract so that `D_DoomMain` can invoke it during startup.
pub fn hulib_init() {
    // Intentionally empty: the original C function is a no-op placeholder.
}

/// Clear the contents of a text line, resetting it to an empty string.
///
/// Sets `needs_update` to 1 (C boolean `true`) so the line is redrawn.
pub fn hulib_clear_text_line(t: &mut HuTextLine) {
    t.len = 0;
    t.l[0] = 0;
    t.needs_update = 1; // C `true` == 1
}

/// Initialise a text line widget with position, font, and start character.
///
/// The font data is cloned so each text line owns its own copy of the glyph
/// set. `sc` is the ASCII code of the first character in the font (typically
/// `b'!'` = 33).
pub fn hulib_init_text_line(t: &mut HuTextLine, x: i32, y: i32, f: &[Vec<u8>], sc: i32) {
    t.x = x;
    t.y = y;
    t.f = f.to_vec();
    t.sc = sc;
    hulib_clear_text_line(t);
}

/// Append a character to the text line.
///
/// Returns `true` if the character was added, `false` if the line is full
/// (length equals [`HU_MAXLINELENGTH`]).
pub fn hulib_add_char_to_text_line(t: &mut HuTextLine, ch: u8) -> bool {
    if t.len >= HU_MAXLINELENGTH {
        return false;
    }
    t.l[t.len] = ch;
    t.len += 1;
    t.l[t.len] = 0; // null-terminate
    t.needs_update = 4;
    true
}

/// Delete the last character from the text line.
///
/// Returns `true` if a character was removed, `false` if the line was
/// already empty. Sets `needs_update` to 4 (matching the original C code).
pub fn hulib_del_char_from_text_line(t: &mut HuTextLine) -> bool {
    if t.len == 0 {
        return false;
    }
    t.len -= 1;
    t.l[t.len] = 0;
    t.needs_update = 4;
    true
}

/// Draw the text line on-screen, optionally with a blinking cursor.
///
/// Characters are upper-cased before rendering. Spaces advance the X cursor
/// by 4 pixels; printable characters in the font range `[sc..=b'_']` are
/// drawn using their font patch glyph and advance by the glyph width.
///
/// The `draw_cursor` flag appends an underscore glyph (`b'_'`) at the end
/// of the line to indicate the text insertion point.
///
/// Faithfully reproduces the original `HUlib_drawTextLine` behaviour:
/// - `c = toupper(l->l[i])`
/// - Space or out-of-range: `x += 4`
/// - In range: `x += SHORT(patch->width)` (no extra +1)
/// - Cursor: underscore glyph at the current X if space permits
pub fn hulib_draw_text_line(l: &HuTextLine, draw_cursor: bool, video: &mut VideoState) {
    // Safety: verify y is within screen bounds.
    if l.y < 0 || l.y >= SCREENHEIGHT {
        return;
    }

    let mut x = l.x;
    let y = l.y;

    for i in 0..l.len {
        // toupper equivalent for ASCII
        let c = (l.l[i] as char).to_ascii_uppercase() as u8;

        if c != b' ' && (c as i32) >= l.sc && c <= b'_' {
            let idx = (c as i32 - l.sc) as usize;
            if idx < l.f.len() {
                let w = patch_width(&l.f[idx]);
                if x + w > SCREENWIDTH {
                    break;
                }
                video.draw_patch_direct(x, y, FG, &l.f[idx]);
                x += w;
            } else {
                // Character outside available font glyphs — treat as space.
                x += 4;
                if x >= SCREENWIDTH {
                    break;
                }
            }
        } else {
            // Space or character outside the drawable range.
            x += 4;
            if x >= SCREENWIDTH {
                break;
            }
        }
    }

    // Draw the cursor (underscore glyph) at the current position.
    if draw_cursor && l.len < HU_MAXLINELENGTH {
        let cursor_idx = (b'_' as i32 - l.sc) as usize;
        if cursor_idx < l.f.len() {
            let cw = patch_width(&l.f[cursor_idx]);
            if x + cw <= SCREENWIDTH {
                video.draw_patch_direct(x, y, FG, &l.f[cursor_idx]);
            }
        }
    }
}

/// Erase the text line's background by copying from the background screen
/// to the foreground screen.
///
/// This implements the `R_VideoErase` logic from the original engine. When
/// the view window does not fill the full screen width (`viewwindowx > 0`),
/// the HUD area around the view window must be re-blitted from the stored
/// background. An automap state change forces a full redraw.
///
/// The renderer globals `automapactive`, `viewwindowx`, `viewwindowy`,
/// `viewwidth`, and `viewheight` are passed as parameters because they are
/// not module-level state in the Rust port.
pub fn hulib_erase_text_line(
    l: &mut HuTextLine,
    video: &mut VideoState,
    automapactive: bool,
    viewwindowx: i32,
    viewwindowy: i32,
    viewwidth: i32,
    viewheight: i32,
) {
    // Detect automap state transitions and force redraw.
    if l.last_automap_active != automapactive {
        l.needs_update = 4;
        l.last_automap_active = automapactive;
    }

    // Erase by copying from screen BG to screen FG when the view window
    // is not full-width and the line needs updating.
    if viewwindowx != 0 && l.needs_update != 0 {
        // Compute the line height from the first font glyph.
        let lh = if !l.f.is_empty() {
            patch_height(&l.f[0]) + 1
        } else {
            1
        };

        let mut y = l.y;
        let mut yoffset = y * SCREENWIDTH;

        while y < l.y + lh {
            let ofs = yoffset as usize;
            if y < viewwindowy || y >= viewwindowy + viewheight {
                // Outside the view window — erase the entire row.
                video_erase(video, ofs, SCREENWIDTH as usize);
            } else {
                // Inside the view window — erase only the left and right columns.
                video_erase(video, ofs, viewwindowx as usize);
                let right_ofs = ofs + (viewwindowx + viewwidth) as usize;
                video_erase(video, right_ofs, viewwindowx as usize);
            }
            y += 1;
            yoffset += SCREENWIDTH;
        }
    }

    // Decrement the update counter.
    if l.needs_update > 0 {
        l.needs_update -= 1;
    }
}

/// Copy `count` bytes from screen BG to screen FG at the given byte offset.
///
/// Equivalent to the original `R_VideoErase(ofs, count)` which performs
/// `memcpy(screens[0]+ofs, screens[1]+ofs, count)`.
fn video_erase(video: &mut VideoState, ofs: usize, count: usize) {
    if count == 0 {
        return;
    }
    let fg_len = video.screens[FG].len();
    let bg_len = video.screens[BG].len();
    if ofs >= fg_len || ofs >= bg_len {
        return;
    }
    // Clamp count to avoid out-of-bounds.
    let actual_count = count.min(fg_len - ofs).min(bg_len - ofs);
    if actual_count == 0 {
        return;
    }

    // Use split_at_mut on the screens array to borrow FG and BG
    // simultaneously without conflicting mutable references.
    // split_at_mut(BG) where BG=1 gives:
    //   fg_screens = &mut [screens[0]]
    //   bg_screens = &mut [screens[1], screens[2], ...]
    let (fg_screens, bg_screens) = video.screens.split_at_mut(BG);
    fg_screens[FG][ofs..ofs + actual_count]
        .copy_from_slice(&bg_screens[0][ofs..ofs + actual_count]);
}

// ═══════════════════════════════════════════════════════════════════
//  ScrollText widget functions
// ═══════════════════════════════════════════════════════════════════

/// Initialise a scrolling text widget.
///
/// Creates `h` text lines stacked vertically upward from (`x`, `y`). Each
/// successive line is positioned at `y - i * (font_height + 1)`, so that
/// the most recent message appears at the bottom and older messages scroll
/// upward. Matches the original `HUlib_initSText`.
pub fn hulib_init_stext(
    s: &mut HuScrollText,
    x: i32,
    y: i32,
    h: i32,
    font: &[Vec<u8>],
    startchar: i32,
    on: bool,
) {
    s.h = h;
    s.on = on;
    s.last_on = true; // Original C sets laston = true
    s.cl = 0;

    // Determine the pixel height of each text row from the first font glyph.
    let font_h = if !font.is_empty() {
        patch_height(&font[0])
    } else {
        0
    };

    // Allocate and initialise each text line.
    s.lines = Vec::with_capacity(h as usize);
    for i in 0..h {
        let mut line = HuTextLine::default();
        hulib_init_text_line(&mut line, x, y - i * (font_h + 1), font, startchar);
        s.lines.push(line);
    }
}

/// Advance to the next line in the scrolling text, wrapping at the end.
///
/// All lines are marked as needing update so the entire widget redraws.
/// Matches the original `HUlib_addLineToSText`.
pub fn hulib_add_line_to_stext(s: &mut HuScrollText) {
    // Advance the current line index, wrapping at h.
    s.cl += 1;
    if s.cl == s.h {
        s.cl = 0;
    }

    // Clear the newly current line.
    if let Some(line) = s.lines.get_mut(s.cl as usize) {
        hulib_clear_text_line(line);
    }

    // Mark all lines as needing update.
    for line in &mut s.lines {
        line.needs_update = 4;
    }
}

/// Add a message to the scrolling text, optionally with a prefix string.
///
/// A new line is started, the optional prefix is prepended character by
/// character, and then the message characters are appended. Matches the
/// original `HUlib_addMessageToSText`.
pub fn hulib_add_message_to_stext(s: &mut HuScrollText, prefix: Option<&str>, msg: &str) {
    hulib_add_line_to_stext(s);

    let cl = s.cl as usize;

    // Add prefix characters if provided.
    if let Some(pfx) = prefix {
        for &b in pfx.as_bytes() {
            if let Some(line) = s.lines.get_mut(cl) {
                hulib_add_char_to_text_line(line, b);
            }
        }
    }

    // Add message characters.
    for &b in msg.as_bytes() {
        if let Some(line) = s.lines.get_mut(cl) {
            hulib_add_char_to_text_line(line, b);
        }
    }
}

/// Draw all visible lines of the scrolling text widget.
///
/// Lines are drawn in order from the current line backwards (wrapping),
/// rendering the most recent message first. Matches `HUlib_drawSText`.
pub fn hulib_draw_stext(s: &HuScrollText, video: &mut VideoState) {
    if !s.on {
        return;
    }

    for i in 0..s.h {
        let mut idx = s.cl - i;
        if idx < 0 {
            idx += s.h;
        }
        if let Some(line) = s.lines.get(idx as usize) {
            hulib_draw_text_line(line, false, video);
        }
    }
}

/// Erase the scrolling text widget's background.
///
/// When the widget transitions from visible to hidden, all lines are forced
/// to update so the background is restored. Matches `HUlib_eraseSText`.
pub fn hulib_erase_stext(
    s: &mut HuScrollText,
    video: &mut VideoState,
    automapactive: bool,
    viewwindowx: i32,
    viewwindowy: i32,
    viewwidth: i32,
    viewheight: i32,
) {
    for i in 0..s.h as usize {
        if s.last_on && !s.on {
            if let Some(line) = s.lines.get_mut(i) {
                line.needs_update = 4;
            }
        }
        if let Some(line) = s.lines.get_mut(i) {
            hulib_erase_text_line(
                line,
                video,
                automapactive,
                viewwindowx,
                viewwindowy,
                viewwidth,
                viewheight,
            );
        }
    }
    s.last_on = s.on;
}

// ═══════════════════════════════════════════════════════════════════
//  InputText widget functions
// ═══════════════════════════════════════════════════════════════════

/// Initialise an input text widget at the given position with a font.
///
/// Matches the original `HUlib_initIText`.
pub fn hulib_init_itext(
    it: &mut HuInputText,
    x: i32,
    y: i32,
    font: &[Vec<u8>],
    startchar: i32,
    on: bool,
) {
    it.lm = 0;
    it.on = on;
    it.last_on = false;
    hulib_init_text_line(&mut it.l, x, y, font, startchar);
}

/// Delete a character from the input text, respecting the left margin.
///
/// Characters at or before the left margin (`lm`) cannot be deleted. This
/// prevents the user from erasing a prefix string. Matches
/// `HUlib_delCharFromIText`.
pub fn hulib_del_char_from_itext(it: &mut HuInputText) {
    if it.l.len > it.lm {
        hulib_del_char_from_text_line(&mut it.l);
    }
}

/// Erase all user-entered characters back to the left margin.
///
/// Repeatedly deletes characters until only the prefix (characters before
/// `lm`) remains. Matches `HUlib_eraseLineFromIText`.
pub fn hulib_erase_line_from_itext(it: &mut HuInputText) {
    while it.l.len > it.lm {
        hulib_del_char_from_text_line(&mut it.l);
    }
}

/// Reset the input text widget to its initial empty state.
///
/// Clears the left margin and all text content.
/// Matches `HUlib_resetIText`.
pub fn hulib_reset_itext(it: &mut HuInputText) {
    it.lm = 0;
    hulib_clear_text_line(&mut it.l);
}

/// Set a fixed prefix on the input text and lock the left margin after it.
///
/// The prefix characters are added to the text line, and `lm` is set to
/// the resulting length so that subsequent delete operations cannot remove
/// the prefix. Matches `HUlib_addPrefixToIText`.
pub fn hulib_add_prefix_to_itext(it: &mut HuInputText, s: &str) {
    for &b in s.as_bytes() {
        hulib_add_char_to_text_line(&mut it.l, b);
    }
    it.lm = it.l.len;
}

/// Process a key press in the input text widget.
///
/// Handles printable characters (ASCII `b' '` through `b'_'`), backspace,
/// and enter. Returns `true` if the key was consumed, `false` otherwise.
///
/// Faithfully reproduces the original `HUlib_keyInIText` logic:
/// - Printable chars (`b' '`..=`b'_'`): appended to the line, returns `true`.
/// - [`KEY_BACKSPACE`]: deletes a character (respecting left margin), returns `true`.
/// - [`KEY_ENTER`]: consumed but takes no action (caller handles enter semantics),
///   returns `true`.
/// - [`KEY_ESCAPE`] and all other keys: not consumed, returns `false`.
///   Escape handling is performed by the caller (`hu_stuff`), not this widget.
pub fn hulib_key_in_itext(it: &mut HuInputText, ch: u8) -> bool {
    if (b' '..=b'_').contains(&ch) {
        hulib_add_char_to_text_line(&mut it.l, ch);
    } else if ch == KEY_BACKSPACE as u8 {
        hulib_del_char_from_itext(it);
    } else if ch == KEY_ESCAPE as u8 {
        // Escape is not consumed by this widget — the caller (hu_stuff)
        // handles escape to close the chat input.
        return false;
    } else if ch != KEY_ENTER as u8 {
        return false; // did not eat key
    }
    true // ate the key
}

/// Draw the input text widget with a cursor.
///
/// Renders the underlying text line with the cursor enabled. Only draws
/// when the widget is visible (`on == true`). Matches `HUlib_drawIText`.
pub fn hulib_draw_itext(it: &HuInputText, video: &mut VideoState) {
    if !it.on {
        return;
    }
    hulib_draw_text_line(&it.l, true, video);
}

/// Erase the input text widget's background.
///
/// When the widget transitions from visible to hidden, forces a full
/// redraw to restore the background. Matches `HUlib_eraseIText`.
pub fn hulib_erase_itext(
    it: &mut HuInputText,
    video: &mut VideoState,
    automapactive: bool,
    viewwindowx: i32,
    viewwindowy: i32,
    viewwidth: i32,
    viewheight: i32,
) {
    if it.last_on && !it.on {
        it.l.needs_update = 4;
    }
    hulib_erase_text_line(
        &mut it.l,
        video,
        automapactive,
        viewwindowx,
        viewwindowy,
        viewwidth,
        viewheight,
    );
    it.last_on = it.on;
}
