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

//! Translated from linuxdoom-1.10/r_sky.c and r_sky.h
//!
//! DOOM Sky rendering constants and initialization.
//!
//! The DOOM sky is a texture map like any wall, wrapping around. A 1024 columns
//! equal 360 degrees. The default sky map is 256 columns and repeats 4 times
//! on a 320-pixel wide screen.
//!
//! This module manages sky texture state and provides initialization. The actual
//! per-column sky drawing is performed in `plane.rs` (`R_DrawPlanes` sky branch),
//! which uses the angle calculation `(viewangle + xtoviewangle[x]) >> ANGLETOSKYSHIFT`
//! to index into the sky texture column.
//!
//! Sky textures are ALWAYS drawn full-bright (using `colormaps[0]`), which is
//! handled by the plane rendering code, not here.

use doom_core::types::fixed::FRACUNIT;

// ---------------------------------------------------------------------------
// Constants (from r_sky.h lines 32-35)
// ---------------------------------------------------------------------------

/// Name of the flat used to mark sky ceilings in maps.
///
/// When a sector's ceiling flat matches this name (resolved to `skyflatnum`
/// during texture initialization in `data.rs`), the renderer draws the sky
/// texture instead of a normal ceiling flat.
///
/// Original C (r_sky.h line 32):
/// ```c
/// #define SKYFLATNAME  "F_SKY1"
/// ```
pub const SKYFLATNAME: &str = "F_SKY1";

/// Shift value to convert a 32-bit BAM (Binary Angle Measurement) angle to a
/// sky texture column index.
///
/// Given the `angle_t` range `0..0xFFFFFFFF`, shifting right by 22 produces
/// values in the range `0..1023`. Since the default sky texture is 256 pixels
/// wide, this means the sky wraps exactly 4 times across a full 360-degree
/// rotation (`1024 / 256 = 4`), which matches the original DOOM behavior.
///
/// Used in `plane.rs` (`R_DrawPlanes`) as:
/// ```text
/// column = (viewangle + xtoviewangle[x]) >> ANGLETOSKYSHIFT
/// ```
///
/// Original C (r_sky.h line 35):
/// ```c
/// #define ANGLETOSKYSHIFT  22
/// ```
pub const ANGLETOSKYSHIFT: u32 = 22;

// ---------------------------------------------------------------------------
// SkyState — Mutable sky rendering state
// ---------------------------------------------------------------------------

/// Sky rendering state, consolidating the three global variables from
/// the original C source (`skyflatnum`, `skytexture`, `skytexturemid`).
///
/// In the original C code, these were bare `int` globals defined in `r_sky.c`
/// and declared `extern` in `r_sky.h`. In the Rust port, they are grouped into
/// this struct and passed by mutable reference where needed, eliminating
/// `static mut` usage.
///
/// # Lifecycle
///
/// 1. `SkyState::new()` — creates initial state with default values.
/// 2. During texture initialization (`R_InitData` in `data.rs`), `skyflatnum`
///    is set to the flat number for `"F_SKY1"`.
/// 3. Per-episode setup sets `skytexture` to the appropriate sky texture number.
/// 4. `init_sky_map()` is called whenever the view size changes, resetting
///    `skytexturemid` to center the sky vertically.
pub struct SkyState {
    /// Flat number for the `F_SKY1` flat.
    ///
    /// Set during texture initialization (`R_InitData`) when the flat lump
    /// directory is built. Any sector whose `ceilingpic` matches this value
    /// will have its ceiling rendered as sky instead of a normal flat.
    ///
    /// Original C: `int skyflatnum;` (r_sky.c line 47)
    pub skyflatnum: i32,

    /// Texture number for the current sky texture.
    ///
    /// Set per episode/map (e.g., `SKY1` for episode 1, `SKY2` for episode 2,
    /// `SKY3` for episode 3). The sky texture is a standard wall texture looked
    /// up via `R_TextureNumForName`.
    ///
    /// Original C: `int skytexture;` (r_sky.c line 48)
    pub skytexture: i32,

    /// Vertical offset for sky rendering in 16.16 fixed-point.
    ///
    /// Set to `100 * FRACUNIT` (= 6,553,600), which positions the sky texture
    /// so its vertical center aligns with the screen center — the horizon at
    /// `y = 100` in the 320×200 display.
    ///
    /// Original C: `int skytexturemid;` (r_sky.c line 49)
    pub skytexturemid: i32,
}

impl SkyState {
    /// Creates a new `SkyState` with default initialization values.
    ///
    /// - `skyflatnum` is set to `0` (will be resolved later during `R_InitData`
    ///   when the `F_SKY1` flat is found in the WAD lump directory).
    /// - `skytexture` is set to `0` (will be set per episode/map during game
    ///   setup).
    /// - `skytexturemid` is set to `100 * FRACUNIT` (= 6,553,600), centering
    ///   the sky texture vertically at the screen horizon.
    #[inline]
    pub fn new() -> Self {
        Self {
            skyflatnum: 0,
            skytexture: 0,
            skytexturemid: 100 * FRACUNIT,
        }
    }

    /// Initializes the sky map. Called whenever the view size changes.
    ///
    /// Sets `skytexturemid` to `100 * FRACUNIT` (= 6,553,600) to vertically
    /// center the sky texture at the screen horizon (`y = 100` in the 320×200
    /// display).
    ///
    /// The original C source had a commented-out line that would also set
    /// `skyflatnum` via `R_FlatNumForName(SKYFLATNAME)`, but that initialization
    /// is done elsewhere during `R_InitData`.
    ///
    /// Original C (r_sky.c lines 57-61):
    /// ```c
    /// void R_InitSkyMap(void) {
    ///     // skyflatnum = R_FlatNumForName(SKYFLATNAME);
    ///     skytexturemid = 100 * FRACUNIT;
    /// }
    /// ```
    #[inline]
    pub fn init_sky_map(&mut self) {
        self.skytexturemid = 100 * FRACUNIT;
    }
}

impl Default for SkyState {
    /// Provides default `SkyState` values, identical to [`SkyState::new()`].
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skyflatname_value() {
        assert_eq!(SKYFLATNAME, "F_SKY1");
    }

    #[test]
    fn test_angletoskyshift_value() {
        // Must match the original C #define ANGLETOSKYSHIFT 22
        assert_eq!(ANGLETOSKYSHIFT, 22);
    }

    #[test]
    fn test_angletoskyshift_produces_correct_range() {
        // A full rotation (0xFFFFFFFF) shifted right by 22 should give 1023
        let max_angle: u32 = 0xFFFF_FFFF;
        let max_col = max_angle >> ANGLETOSKYSHIFT;
        assert_eq!(max_col, 1023);

        // Half rotation should give ~512
        let half_angle: u32 = 0x8000_0000;
        let half_col = half_angle >> ANGLETOSKYSHIFT;
        assert_eq!(half_col, 512);
    }

    #[test]
    fn test_skystate_new_defaults() {
        let sky = SkyState::new();
        assert_eq!(sky.skyflatnum, 0);
        assert_eq!(sky.skytexture, 0);
        // 100 * FRACUNIT = 100 * 65536 = 6_553_600
        assert_eq!(sky.skytexturemid, 6_553_600);
    }

    #[test]
    fn test_skystate_default_matches_new() {
        let sky_new = SkyState::new();
        let sky_default = SkyState::default();
        assert_eq!(sky_new.skyflatnum, sky_default.skyflatnum);
        assert_eq!(sky_new.skytexture, sky_default.skytexture);
        assert_eq!(sky_new.skytexturemid, sky_default.skytexturemid);
    }

    #[test]
    fn test_init_sky_map_resets_skytexturemid() {
        let mut sky = SkyState::new();

        // Simulate someone changing skytexturemid to a different value
        sky.skytexturemid = 0;
        assert_eq!(sky.skytexturemid, 0);

        // init_sky_map should reset it to 100 * FRACUNIT
        sky.init_sky_map();
        assert_eq!(sky.skytexturemid, 100 * FRACUNIT);
        assert_eq!(sky.skytexturemid, 6_553_600);
    }

    #[test]
    fn test_init_sky_map_preserves_other_fields() {
        let mut sky = SkyState::new();

        // Set skyflatnum and skytexture to non-default values
        sky.skyflatnum = 42;
        sky.skytexture = 7;
        sky.skytexturemid = 0;

        // init_sky_map should only affect skytexturemid
        sky.init_sky_map();
        assert_eq!(sky.skyflatnum, 42);
        assert_eq!(sky.skytexture, 7);
        assert_eq!(sky.skytexturemid, 100 * FRACUNIT);
    }

    #[test]
    fn test_fracunit_value() {
        // Verify FRACUNIT from doom_core is 65536 (1 << 16)
        assert_eq!(FRACUNIT, 65536);
        assert_eq!(FRACUNIT, 1 << 16);
    }

    #[test]
    fn test_skytexturemid_calculation() {
        // 100 * FRACUNIT must equal 100 * 65536 = 6_553_600
        // This centers the sky at y=100 in the 320x200 display
        let expected = 100_i32 * 65536_i32;
        assert_eq!(100 * FRACUNIT, expected);
        assert_eq!(SkyState::new().skytexturemid, expected);
    }
}
