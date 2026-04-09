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

//! Translated from linuxdoom-1.10/m_misc.c and linuxdoom-1.10/m_misc.h
//!
//! Miscellaneous utilities: binary file I/O, configuration defaults
//! management, PCX screenshot capture, and text drawing.
//!
//! Platform-specific changes: Unix file I/O (`open`/`read`/`write`/`close`/
//! `fstat`) replaced with `std::fs`, Unix paths replaced with platform-neutral
//! approach, `Z_Malloc` replaced with `Vec<u8>` allocations, `access()` replaced
//! with `std::path::Path::exists()`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use tracing::{error, info};

use crate::game::strings::{
    HUSTR_CHATMACRO0, HUSTR_CHATMACRO1, HUSTR_CHATMACRO2, HUSTR_CHATMACRO3, HUSTR_CHATMACRO4,
    HUSTR_CHATMACRO5, HUSTR_CHATMACRO6, HUSTR_CHATMACRO7, HUSTR_CHATMACRO8, HUSTR_CHATMACRO9,
};
use crate::types::doomdef::{
    KEY_DOWNARROW, KEY_LEFTARROW, KEY_RALT, KEY_RCTRL, KEY_RIGHTARROW, KEY_RSHIFT, KEY_UPARROW,
    SCREENHEIGHT, SCREENWIDTH,
};

// =============================================================================
// HUD font constants — used by draw_text
// =============================================================================
// These match HU_FONTSTART / HU_FONTEND / HU_FONTSIZE from hu_stuff.h.

/// First printable character in the HUD font — `'!'` (ASCII 33).
const HU_FONTSTART: u8 = b'!';

/// Last printable character in the HUD font — `'_'` (ASCII 95).
const HU_FONTEND: u8 = b'_';

/// Number of characters in the HUD font.
const HU_FONTSIZE: usize = (HU_FONTEND - HU_FONTSTART + 1) as usize;

// =============================================================================
// M_DrawText — Draw text on screen using HUD font
// =============================================================================

/// Draw text on screen using the HUD font.
///
/// Equivalent to C: `int M_DrawText(int x, int y, boolean direct, char* string)`
/// (m_misc.c lines 69-100)
///
/// Returns the final X coordinate after drawing all characters. `HU_Init` must
/// have been called to initialise the font before invoking this function.
///
/// # Parameters
///
/// * `x` — Starting horizontal pixel position.
/// * `y` — Vertical pixel position.
/// * `_direct` — Whether to use direct screen drawing. Preserved for behavioral
///   parity but has no distinct behavior under the SDL2 backend (both direct and
///   buffered drawing target the same framebuffer).
/// * `string` — The text string to draw.
/// * `font_char_width` — Callback that returns `Some(width)` for a valid HUD
///   font character index (0 .. `HU_FONTSIZE`-1), or `None` if the index is
///   out of range or the font is not yet loaded.
/// * `draw_char_fn` — Callback to draw a single character at `(x, y)` using the
///   given font index. The `bool` argument mirrors the `direct` parameter.
pub fn draw_text(
    mut x: i32,
    y: i32,
    direct: bool,
    string: &str,
    font_char_width: &dyn Fn(usize) -> Option<i32>,
    draw_char_fn: &mut dyn FnMut(i32, i32, bool, usize),
) -> i32 {
    for ch in string.bytes() {
        // Convert to uppercase and compute font index, matching C:
        //   c = toupper(*string) - HU_FONTSTART;
        let upper = ch.to_ascii_uppercase();
        let c = upper as i32 - HU_FONTSTART as i32;

        // Characters outside the font range get a fixed-width space.
        if c < 0 || c >= HU_FONTSIZE as i32 {
            x += 4;
            continue;
        }

        let idx = c as usize;

        // Retrieve the character width from the font. If the callback cannot
        // provide a width (font not yet loaded, missing glyph), fall back to
        // the default space width.
        let w = match font_char_width(idx) {
            Some(w) => w,
            None => {
                x += 4;
                continue;
            }
        };

        // Stop if the character would overflow the screen width.
        if x + w > SCREENWIDTH {
            break;
        }

        // Draw the character patch via the provided callback.
        draw_char_fn(x, y, direct, idx);

        x += w;
    }

    x
}

// =============================================================================
// M_WriteFile — Write binary data to file
// =============================================================================

/// Write binary data to a file.
///
/// Equivalent to C: `boolean M_WriteFile(char const* name, void* source, int length)`
/// (m_misc.c lines 112-133)
///
/// Returns `true` on success, `false` on failure. Replaces Unix
/// `open`/`write`/`close` with `std::fs::write`. On failure an error is logged
/// via `tracing::error!` (replacing the original silent boolean return).
pub fn write_file(name: &str, source: &[u8]) -> bool {
    match fs::write(name, source) {
        Ok(()) => true,
        Err(e) => {
            error!("M_WriteFile: Couldn't write file {}: {}", name, e);
            false
        }
    }
}

// =============================================================================
// M_ReadFile — Read entire file into memory
// =============================================================================

/// Read an entire file into memory.
///
/// Equivalent to C: `int M_ReadFile(char const* name, byte** buffer)`
/// (m_misc.c lines 139-163)
///
/// Returns the file contents as a `Vec<u8>`. Replaces Unix
/// `open`/`fstat`/`read`/`close` and `Z_Malloc` with `std::fs::read`.
///
/// # Panics
///
/// Panics if the file cannot be read, matching the original C behaviour which
/// calls `I_Error` (terminates the program) on failure.
pub fn read_file(name: &str) -> Vec<u8> {
    match fs::read(name) {
        Ok(data) => data,
        Err(e) => {
            // The original C code calls I_Error on failure (terminates the
            // program). A panic is the Rust equivalent.
            panic!("Couldn't read file {}: {}", name, e);
        }
    }
}

/// Read an entire file into memory, returning a `Result` instead of panicking.
///
/// This is a Rust-idiomatic alternative to [`read_file`] for callers that
/// prefer explicit error handling without program termination.
pub fn try_read_file(name: &str) -> io::Result<Vec<u8>> {
    let mut file = fs::File::open(name)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    Ok(buffer)
}

// =============================================================================
// Configuration defaults system
// =============================================================================
// Translated from m_misc.c lines 168-403: default_t struct, defaults[] array,
// M_SaveDefaults, and M_LoadDefaults.

/// A configuration default value — either an integer or a string.
///
/// In the original C code, string values are stored as `int` casts of `char*`
/// pointers, with the determination based on the magnitude of `defaultvalue`
/// (values outside the range −0xFFF..0xFFF are treated as string pointers).
/// In Rust we use an explicit enum for type safety.
#[derive(Debug, Clone)]
pub enum DefaultValue {
    /// Integer configuration value.
    Int(i32),
    /// String configuration value.
    Str(String),
}

/// A single configuration default entry.
///
/// Equivalent to C:
/// ```c
/// typedef struct {
///     char*  name;
///     int*   location;
///     int    defaultvalue;
///     int    scantranslate;
///     int    untranslated;
/// } default_t;
/// ```
/// (m_misc.c lines 225-232)
///
/// The `scantranslate` and `untranslated` fields are omitted — PC scan-code
/// translation is not needed in the SDL2 backend.
#[derive(Debug, Clone)]
pub struct DefaultEntry {
    /// Configuration key name (e.g. `"mouse_sensitivity"`, `"key_right"`).
    pub name: String,
    /// Current value of this configuration entry.
    pub value: DefaultValue,
    /// Default value used when no configuration file is present.
    pub default: DefaultValue,
}

/// Configuration defaults manager.
///
/// Replaces the C globals: `default_t defaults[]`, `int numdefaults`,
/// `char* defaultfile`.
///
/// This struct owns all configuration state and provides load/save
/// functionality using a tab-separated text file format compatible with the
/// original DOOM config format.
#[derive(Debug, Clone)]
pub struct ConfigDefaults {
    /// All configuration entries.
    pub entries: Vec<DefaultEntry>,
    /// Path to the configuration file.
    pub default_file: String,
}

impl ConfigDefaults {
    /// Load configuration defaults from a file.
    ///
    /// Equivalent to C: `void M_LoadDefaults(void)` (m_misc.c lines 340-403)
    ///
    /// First sets all entries to their default values, then attempts to read
    /// and parse the configuration file. If the file does not exist the
    /// defaults are used silently.
    ///
    /// Config file format: `name<whitespace>value`
    /// - Integer values: plain decimal numbers or hex with `0x` prefix.
    /// - String values: enclosed in double quotes (`"value"`).
    pub fn load(&mut self) {
        // Set all entries to their default values.
        for entry in &mut self.entries {
            entry.value = entry.default.clone();
        }

        // Attempt to read the config file.
        let content = match fs::read_to_string(&self.default_file) {
            Ok(c) => c,
            Err(_) => {
                // File doesn't exist yet — use defaults.
                return;
            }
        };

        info!("M_LoadDefaults: loading config from {}", self.default_file);

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // The original C uses `fscanf(f, "%79s %[^\n]\n", def, strparm)`:
            // first whitespace-delimited token is the key name, the remainder
            // of the line is the value.
            let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
            if parts.len() != 2 {
                continue;
            }
            let name = parts[0].trim();
            let value_str = parts[1].trim();

            // Find the matching entry and update its value.
            for entry in &mut self.entries {
                if entry.name == name {
                    if value_str.starts_with('"')
                        && value_str.ends_with('"')
                        && value_str.len() >= 2
                    {
                        // String value — strip surrounding double-quotes.
                        let s = &value_str[1..value_str.len() - 1];
                        entry.value = DefaultValue::Str(s.to_string());
                    } else if let Some(hex_digits) = value_str
                        .strip_prefix("0x")
                        .or_else(|| value_str.strip_prefix("0X"))
                    {
                        // Hexadecimal integer value.
                        if let Ok(v) = i32::from_str_radix(hex_digits, 16) {
                            entry.value = DefaultValue::Int(v);
                        }
                    } else {
                        // Decimal integer value.
                        if let Ok(v) = value_str.parse::<i32>() {
                            entry.value = DefaultValue::Int(v);
                        }
                    }
                    break;
                }
            }
        }
    }

    /// Save configuration defaults to a file.
    ///
    /// Equivalent to C: `void M_SaveDefaults(void)` (m_misc.c lines 308-332)
    ///
    /// Writes each configuration entry as a tab-separated line:
    /// - Integer entries: `name\t\tvalue\n`
    /// - String entries:  `name\t\t"value"\n`
    ///
    /// If the file cannot be created the function returns silently, matching
    /// the original C behaviour ("can't write the file, but don't complain").
    pub fn save(&self) {
        let mut f = match fs::File::create(&self.default_file) {
            Ok(f) => f,
            Err(_) => return,
        };

        for entry in &self.entries {
            match &entry.value {
                DefaultValue::Int(v) => {
                    let _ = writeln!(f, "{}\t\t{}", entry.name, v);
                }
                DefaultValue::Str(s) => {
                    let _ = writeln!(f, "{}\t\t\"{}\"", entry.name, s);
                }
            }
        }
    }

    /// Build the default configuration entries.
    ///
    /// Matches the C `defaults[]` array from m_misc.c lines 234-299.
    ///
    /// Linux-specific entries (`sndserver`, `mb_used` under `#ifdef SNDSERV`;
    /// `mousedev`, `mousetype` under `#ifdef LINUX`) are omitted as they are
    /// not applicable to the Windows 11 SDL2 platform backend. Key bindings
    /// use the `NORMALUNIX` default values.
    pub fn build_defaults() -> Self {
        /// Helper: create an integer default entry.
        fn int_entry(name: &str, default: i32) -> DefaultEntry {
            DefaultEntry {
                name: name.to_string(),
                value: DefaultValue::Int(default),
                default: DefaultValue::Int(default),
            }
        }

        /// Helper: create a string default entry.
        fn str_entry(name: &str, default: &str) -> DefaultEntry {
            DefaultEntry {
                name: name.to_string(),
                value: DefaultValue::Str(default.to_string()),
                default: DefaultValue::Str(default.to_string()),
            }
        }

        let entries = vec![
            // --- General settings (m_misc.c lines 236-239) ---
            int_entry("mouse_sensitivity", 5),
            int_entry("sfx_volume", 8),
            int_entry("music_volume", 8),
            int_entry("show_messages", 1),
            // --- Key bindings — NORMALUNIX path (m_misc.c lines 243-253) ---
            int_entry("key_right", KEY_RIGHTARROW),
            int_entry("key_left", KEY_LEFTARROW),
            int_entry("key_up", KEY_UPARROW),
            int_entry("key_down", KEY_DOWNARROW),
            int_entry("key_strafeleft", b',' as i32),
            int_entry("key_straferight", b'.' as i32),
            int_entry("key_fire", KEY_RCTRL),
            int_entry("key_use", b' ' as i32),
            int_entry("key_strafe", KEY_RALT),
            int_entry("key_speed", KEY_RSHIFT),
            // --- SNDSERV entries OMITTED (sndserver, mb_used) — Linux-only ---
            // --- LINUX entries OMITTED (mousedev, mousetype) — Linux-only ---
            // --- Mouse settings (m_misc.c lines 268-271) ---
            int_entry("use_mouse", 1),
            int_entry("mouseb_fire", 0),
            int_entry("mouseb_strafe", 1),
            int_entry("mouseb_forward", 2),
            // --- Joystick settings (m_misc.c lines 273-277) ---
            int_entry("use_joystick", 0),
            int_entry("joyb_fire", 0),
            int_entry("joyb_strafe", 1),
            int_entry("joyb_use", 3),
            int_entry("joyb_speed", 2),
            // --- Screen settings (m_misc.c lines 279-280) ---
            int_entry("screenblocks", 9),
            int_entry("detaillevel", 0),
            // --- Sound channels (m_misc.c line 282) ---
            int_entry("snd_channels", 3),
            // --- Gamma correction (m_misc.c line 286) ---
            int_entry("usegamma", 0),
            // --- Chat macros (m_misc.c lines 288-297) ---
            str_entry("chatmacro0", HUSTR_CHATMACRO0),
            str_entry("chatmacro1", HUSTR_CHATMACRO1),
            str_entry("chatmacro2", HUSTR_CHATMACRO2),
            str_entry("chatmacro3", HUSTR_CHATMACRO3),
            str_entry("chatmacro4", HUSTR_CHATMACRO4),
            str_entry("chatmacro5", HUSTR_CHATMACRO5),
            str_entry("chatmacro6", HUSTR_CHATMACRO6),
            str_entry("chatmacro7", HUSTR_CHATMACRO7),
            str_entry("chatmacro8", HUSTR_CHATMACRO8),
            str_entry("chatmacro9", HUSTR_CHATMACRO9),
        ];

        ConfigDefaults {
            entries,
            default_file: String::from("default.cfg"),
        }
    }
}

// =============================================================================
// PCX Screenshot support
// =============================================================================
// Translated from m_misc.c lines 411-532: pcx_t header struct, WritePCXfile,
// and M_ScreenShot.

/// Size of the PCX file header in bytes (fixed by the PCX specification).
const PCX_HEADER_SIZE: usize = 128;

/// Write a PCX screenshot file.
///
/// Equivalent to C: `WritePCXfile(char* filename, byte* data, int width, int
/// height, byte* palette)` (m_misc.c lines 441-497)
///
/// Encodes the provided palettized pixel data using PCX Run-Length Encoding
/// and writes the result to the specified file together with a 256-colour
/// palette.
///
/// # Parameters
///
/// * `filename` — Output file path.
/// * `data` — Raw palettized pixel data (`width × height` bytes).
/// * `width` — Image width in pixels.
/// * `height` — Image height in pixels.
/// * `palette` — 256-colour RGB palette (768 bytes: R, G, B for each entry).
pub fn write_pcx_file(filename: &str, data: &[u8], width: i32, height: i32, palette: &[u8]) {
    let w = width as u16;
    let h = height as u16;

    // Pre-allocate: header + worst-case pixel data (2× for all-escaped) + palette.
    let max_size = PCX_HEADER_SIZE + (width as usize * height as usize * 2) + 769;
    let mut pcx: Vec<u8> = Vec::with_capacity(max_size);

    // ---- Build PCX header (128 bytes) ----
    //
    // Byte layout matches the C `pcx_t` struct (m_misc.c lines 411-435):
    //   Offset  0: manufacturer  (u8)  = 0x0A — PCX magic
    //   Offset  1: version       (u8)  = 5    — 256-colour
    //   Offset  2: encoding      (u8)  = 1    — RLE
    //   Offset  3: bits_per_pixel(u8)  = 8    — 8-bit indexed
    //   Offset  4: xmin          (u16 LE) = 0
    //   Offset  6: ymin          (u16 LE) = 0
    //   Offset  8: xmax          (u16 LE) = width − 1
    //   Offset 10: ymax          (u16 LE) = height − 1
    //   Offset 12: hres          (u16 LE) = width
    //   Offset 14: vres          (u16 LE) = height
    //   Offset 16: palette[48]   (zero — EGA palette not used)
    //   Offset 64: reserved      (u8)  = 0
    //   Offset 65: color_planes  (u8)  = 1 — chunky image
    //   Offset 66: bytes_per_line(u16 LE) = width
    //   Offset 68: palette_type  (u16 LE) = 2 — colour (not greyscale)
    //   Offset 70: filler[58]    (zero)
    pcx.push(0x0A); // manufacturer
    pcx.push(5); // version
    pcx.push(1); // encoding
    pcx.push(8); // bits_per_pixel
    pcx.extend_from_slice(&0u16.to_le_bytes()); // xmin
    pcx.extend_from_slice(&0u16.to_le_bytes()); // ymin
    pcx.extend_from_slice(&(w.wrapping_sub(1)).to_le_bytes()); // xmax
    pcx.extend_from_slice(&(h.wrapping_sub(1)).to_le_bytes()); // ymax
    pcx.extend_from_slice(&w.to_le_bytes()); // hres
    pcx.extend_from_slice(&h.to_le_bytes()); // vres
    pcx.extend_from_slice(&[0u8; 48]); // EGA palette (zeroed)
    pcx.push(0); // reserved
    pcx.push(1); // color_planes
    pcx.extend_from_slice(&w.to_le_bytes()); // bytes_per_line
    pcx.extend_from_slice(&2u16.to_le_bytes()); // palette_type
    pcx.extend_from_slice(&[0u8; 58]); // filler

    debug_assert_eq!(pcx.len(), PCX_HEADER_SIZE);

    // ---- RLE-encode pixel data (m_misc.c lines 476-485) ----
    //
    // PCX RLE rule: bytes in the range 0xC0..=0xFF are reserved as run-length
    // count markers. When such a byte appears as raw pixel data it must be
    // escaped with a run-length-of-1 prefix (0xC1).
    let pixel_count = (width as usize * height as usize).min(data.len());
    for &byte in &data[..pixel_count] {
        if (byte & 0xC0) != 0xC0 {
            // Value < 0xC0 — safe to write directly.
            pcx.push(byte);
        } else {
            // Value >= 0xC0 — must escape with run-length = 1 prefix.
            pcx.push(0xC1);
            pcx.push(byte);
        }
    }

    // ---- Append VGA palette (m_misc.c lines 488-490) ----
    pcx.push(0x0C); // Palette ID marker byte.

    // Ensure exactly 768 palette bytes follow the marker.
    let mut pal_buf = [0u8; 768];
    let copy_len = palette.len().min(768);
    pal_buf[..copy_len].copy_from_slice(&palette[..copy_len]);
    pcx.extend_from_slice(&pal_buf);

    // ---- Write output file ----
    write_file(filename, &pcx);
}

/// Take a screenshot and save it as DOOMxx.pcx.
///
/// Equivalent to C: `void M_ScreenShot(void)` (m_misc.c lines 503-532)
///
/// Searches for the first available filename from `DOOM00.pcx` through
/// `DOOM99.pcx` and writes the provided screen data as a PCX file.
///
/// # Parameters
///
/// * `screen_data` — The current screen pixel data (`SCREENWIDTH × SCREENHEIGHT`
///   bytes of palettized pixel data). The caller is responsible for invoking the
///   platform's `I_ReadScreen` equivalent to fill this buffer before calling
///   this function.
/// * `palette` — The current 256-colour palette (768 bytes: R, G, B triples),
///   typically obtained from the `PLAYPAL` WAD lump.
///
/// # Returns
///
/// `Some(filename)` on success, or `None` if all 100 filename slots are
/// already occupied.
pub fn screenshot(screen_data: &[u8], palette: &[u8]) -> Option<String> {
    // Find the first available filename: DOOM00.pcx .. DOOM99.pcx.
    // Matches C: access(lbmname, 0) == -1  ⇒  Path::exists() == false.
    let mut filename = String::new();
    let mut found = false;

    for i in 0..100_u32 {
        filename = format!("DOOM{:02}.pcx", i);
        if !Path::new(&filename).exists() {
            found = true;
            break;
        }
    }

    if !found {
        error!("M_ScreenShot: Couldn't create a PCX (all DOOM00-DOOM99.pcx exist)");
        return None;
    }

    // Write the PCX file using the current screen data and palette.
    write_pcx_file(&filename, screen_data, SCREENWIDTH, SCREENHEIGHT, palette);

    info!("M_ScreenShot: wrote {}", filename);
    Some(filename)
}
