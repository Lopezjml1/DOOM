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

// Allow `pub mod main;` — this is a renderer module named after the original
// C source file `r_main.c`, NOT a binary entry point.
#![allow(special_module_name)]

//! # doom-render-soft — Software Renderer for DOOM 1.10
//!
//! Translated from the `r_*` source files in `linuxdoom-1.10/`:
//! `r_bsp.c`, `r_data.c`, `r_draw.c`, `r_main.c`, `r_plane.c`,
//! `r_segs.c`, `r_sky.c`, `r_things.c`, and their associated headers.
//!
//! The umbrella header `r_local.h` included all renderer sub-headers.
//! This crate root serves the same role in Rust.
//!
//! ## Modules
//! - [`bsp`] — BSP tree traversal (from r_bsp.c/h)
//! - [`data`] — Texture/flat/sprite/colormap cache (from r_data.c/h)
//! - [`draw`] — Column and span drawing primitives (from r_draw.c/h + README.asm)
//! - [`main`] — Renderer entry point (from r_main.c/h) — NOT a binary entry point
//! - [`plane`] — Visplane floor/ceiling rendering (from r_plane.c/h)
//! - [`segs`] — Wall segment rendering (from r_segs.c/h)
//! - [`sky`] — Sky texture rendering (from r_sky.c/h)
//! - [`things`] — Sprite sorting and masked column compositing (from r_things.c/h)
//! - [`defs`] — Renderer-internal type definitions (from r_defs.h + r_state.h)
//!
//! ## Architecture
//!
//! This crate implements the `Renderer` trait defined in `doom_core::traits::renderer`,
//! providing the BSP-based software rendering pipeline that draws the DOOM 3D game
//! world at 320×200 resolution with 256-color palettized graphics.
//!
//! The rendering pipeline (per frame):
//! 1. `R_SetupFrame` — set viewpoint from player position/angle
//! 2. Clear clip segs, draw segs, visplanes, and visible sprites
//! 3. `R_RenderBSPNode` — traverse BSP tree front-to-back
//! 4. `R_DrawPlanes` — render floor/ceiling visplanes
//! 5. `R_DrawMasked` — sort and composite sprites with masked textures
//!
//! This crate is **platform-independent**. It writes pixels to an in-memory
//! framebuffer; actual display output is handled by `doom-platform-win`.
//! No SDL2, X11, or any platform-specific code exists in this crate.

// =============================================================================
// Module declarations — all 9 renderer subsystems
// =============================================================================
//
// These correspond to the includes from the original C umbrella header
// r_local.h: r_bsp.h, r_data.h, r_draw.h, r_main.h, r_plane.h,
// r_segs.h, r_sky.h, r_things.h, and r_defs.h/r_state.h.

/// BSP tree traversal for front-to-back subsector rendering.
///
/// Translated from: `linuxdoom-1.10/r_bsp.c` and `r_bsp.h`.
/// Key functions: `R_RenderBSPNode`, `R_Subsector`, `R_AddLine`.
pub mod bsp;

/// Texture, flat, sprite, and colormap cache loading from WAD lumps.
///
/// Translated from: `linuxdoom-1.10/r_data.c` and `r_data.h`.
/// Key functions: `R_InitTextures`, `R_InitFlats`, `R_InitSpriteLumps`,
/// `R_InitColormaps`, `R_GetColumn`.
pub mod data;

/// Renderer-internal type definitions and consolidated global state.
///
/// Translated from: `linuxdoom-1.10/r_defs.h` and `r_state.h`.
/// Re-exports shared map types from `doom_core` and defines `RenderState`.
pub mod defs;

/// Column and span drawing primitives — the renderer inner loops.
///
/// Translated from: `linuxdoom-1.10/r_draw.c`, `r_draw.h`, and `README.asm`.
/// Key functions: `R_DrawColumn`, `R_DrawSpan`, `R_DrawFuzzColumn`,
/// `R_InitBuffer`, `R_FillBackScreen`.
pub mod draw;

/// Renderer entry point, viewpoint setup, and lighting LUT initialization.
///
/// Translated from: `linuxdoom-1.10/r_main.c` and `r_main.h`.
/// Key functions: `R_RenderPlayerView`, `R_SetupFrame`, `R_Init`,
/// `R_SetViewSize`, `R_ExecuteSetViewSize`.
///
/// **NOTE**: This is a renderer module, NOT a binary entry point. The file
/// is named `main.rs` to match the original `r_main.c` convention.
pub mod main;

/// Visplane allocation and floor/ceiling span rendering.
///
/// Translated from: `linuxdoom-1.10/r_plane.c` and `r_plane.h`.
/// Key functions: `R_FindPlane`, `R_MakeSpans`, `R_DrawPlanes`,
/// `R_ClearPlanes`.
pub mod plane;

/// Wall segment rendering and texture mapping.
///
/// Translated from: `linuxdoom-1.10/r_segs.c` and `r_segs.h`.
/// Key functions: `R_RenderSegLoop`, `R_StoreWallRange`.
pub mod segs;

/// Sky texture rendering constants and initialization.
///
/// Translated from: `linuxdoom-1.10/r_sky.c` and `r_sky.h`.
/// Key constants: `SKYFLATNAME`, `ANGLETOSKYSHIFT`.
/// Key struct: `SkyState` (manages `skyflatnum`, `skytexture`, `skytexturemid`).
pub mod sky;

/// Sprite sorting and masked column compositing.
///
/// Translated from: `linuxdoom-1.10/r_things.c` and `r_things.h`.
/// Key functions: `R_DrawMasked`, `R_ProjectSprite`, `R_DrawVisSprite`,
/// `R_SortVisSprites`.
pub mod things;

// =============================================================================
// Imports
// =============================================================================

use doom_core::traits::renderer::Renderer;
use doom_core::types::player::Player;

// =============================================================================
// SoftwareRenderer — Main renderer struct
// =============================================================================

/// The DOOM software renderer.
///
/// Implements the [`Renderer`] trait to provide BSP-based rendering
/// of the 3D game world at 320×200 resolution with 256-color palette.
///
/// This struct holds the deferred view-size change state. The actual
/// rendering pipeline state is distributed across the sub-module state
/// structs ([`defs::RenderState`], [`draw::DrawState`], [`data::DataState`],
/// [`bsp::BspState`], [`plane::PlaneState`], [`segs::SegsState`],
/// [`sky::SkyState`], [`things::ThingsState`], [`main::RenderMain`]).
///
/// ## View Size Changes
///
/// View size changes are deferred rather than applied immediately. When
/// [`set_view_size`](Renderer::set_view_size) is called, the `setsizeneeded`
/// flag is set and the pending `setblocks`/`setdetail` values are stored.
/// The actual recalculation happens at the start of the next frame via
/// `R_ExecuteSetViewSize` in the [`main`] module.
///
/// This matches the original C behavior in `r_main.c` where `R_SetViewSize`
/// simply sets global variables and the deferred flag:
/// ```c
/// void R_SetViewSize(int blocks, int detail) {
///     setsizeneeded = true;
///     setblocks = blocks;
///     setdetail = detail;
/// }
/// ```
///
/// ## Original C References
///
/// - `r_main.c` lines 299-305: `R_SetViewSize` (deferred flag pattern)
/// - `r_main.c` lines 255-297: `R_ExecuteSetViewSize` (actual recalculation)
/// - `r_main.h` lines 157-165: `R_RenderPlayerView`, `R_Init`, `R_SetViewSize`
pub struct SoftwareRenderer {
    /// View size has changed — flag to trigger `R_ExecuteSetViewSize`.
    ///
    /// When `true`, the next call to `render_player_view` (or the game loop)
    /// should invoke `R_ExecuteSetViewSize` before rendering to apply the
    /// pending `setblocks` and `setdetail` values.
    ///
    /// Original C: `boolean setsizeneeded;` (`r_main.c` line 235)
    pub setsizeneeded: bool,

    /// Pending view size in screen blocks (3–11).
    ///
    /// Block values map to the view window size:
    /// - 3 = smallest view window
    /// - 10 = largest view window with status bar
    /// - 11 = full-screen view (no status bar)
    ///
    /// Original C: `int setblocks;` (`r_main.c` line 237)
    pub setblocks: i32,

    /// Pending detail level (0 = high detail, 1 = low/blocky detail).
    ///
    /// In low detail mode, each column is drawn twice as wide, halving
    /// the horizontal resolution for a performance boost on slow hardware.
    ///
    /// Original C: `int setdetail;` (`r_main.c` line 238)
    pub setdetail: i32,
}

impl Default for SoftwareRenderer {
    /// Creates a `SoftwareRenderer` with default initialization values.
    ///
    /// - `setsizeneeded` = `true` — forces initial view size calculation on first frame
    /// - `setblocks` = `10` — default to the largest windowed view (with status bar)
    /// - `setdetail` = `0` — default to high detail mode
    #[inline]
    fn default() -> Self {
        Self {
            setsizeneeded: true,
            setblocks: 10,
            setdetail: 0,
        }
    }
}

impl SoftwareRenderer {
    /// Creates a new `SoftwareRenderer` with default initialization values.
    ///
    /// Identical to [`Default::default()`]. The `setsizeneeded` flag is set
    /// to `true` so that view size calculation occurs before the first frame.
    ///
    /// # Example
    /// ```ignore
    /// let renderer = SoftwareRenderer::new();
    /// assert!(renderer.setsizeneeded);
    /// assert_eq!(renderer.setblocks, 10);
    /// assert_eq!(renderer.setdetail, 0);
    /// ```
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
}

// =============================================================================
// Renderer trait implementation
// =============================================================================

impl Renderer for SoftwareRenderer {
    /// Renders the 3D game world from the given player's viewpoint.
    ///
    /// This method orchestrates the full software rendering pipeline for a
    /// single frame. The pipeline (matching `R_RenderPlayerView` in `r_main.c`
    /// lines 1000-1026) proceeds through these stages:
    ///
    /// 1. **Setup frame** — `R_SetupFrame(player)`: set the viewpoint position,
    ///    angle, and height from the player's map object.
    /// 2. **Clear buffers** — `R_ClearClipSegs`, `R_ClearDrawSegs`,
    ///    `R_ClearPlanes`, `R_ClearSprites`: reset all per-frame rendering buffers.
    /// 3. **BSP traversal** — `R_RenderBSPNode(numnodes - 1)`: traverse the BSP
    ///    tree front-to-back, rendering wall segments and recording visplanes
    ///    and visible sprites.
    /// 4. **Floor/ceiling** — `R_DrawPlanes`: render all recorded visplanes
    ///    (floor and ceiling spans).
    /// 5. **Sprites** — `R_DrawMasked`: sort visible sprites by distance and
    ///    composite them with masked wall textures.
    ///
    /// # Parameters
    /// - `player` — The player whose viewpoint is used for rendering. The
    ///   renderer reads the player's position (`mobj.x`, `mobj.y`, `viewz`),
    ///   orientation (`mobj.angle`), and lighting state (`extralight`,
    ///   `fixedcolormap`) to configure the 3D perspective.
    ///
    /// # Original C Reference
    /// `r_main.c` lines 1000-1026: `R_RenderPlayerView(player_t* player)`
    fn render_player_view(&mut self, _player: &Player) {
        // The full rendering pipeline will be wired through the `main` module's
        // `r_render_player_view` function once all sub-module state structs
        // (RenderMain, BspState, PlaneState, SegsState, ThingsState, DrawState,
        // DataState, SkyState) are connected via the game loop in doom-bin.
        //
        // At this architectural level, SoftwareRenderer acts as the trait
        // adapter — the actual rendering work is delegated to module-level
        // functions that operate on the full renderer state graph.
        todo!("R_RenderPlayerView: wire through main::r_render_player_view when sub-modules are connected")
    }

    /// Initializes the software renderer subsystems.
    ///
    /// Must be called once during game startup (from `D_DoomMain`) before
    /// any rendering occurs. Initializes all renderer subsystems in order:
    ///
    /// 1. `R_InitData` — Load textures, flats, sprites, colormaps from WAD
    /// 2. `R_InitPointToAngle` — Build point-to-angle lookup tables
    /// 3. `R_InitTables` — Build trigonometric rendering tables
    /// 4. `R_SetViewSize` — Set initial view size (triggers deferred recalc)
    /// 5. `R_InitPlanes` — Initialize visplane rendering
    /// 6. `R_InitLightTables` — Build diminishing lighting LUTs
    /// 7. `R_InitSkyMap` — Set up sky texture parameters
    /// 8. `R_InitTranslationTables` — Build player color translation tables
    ///
    /// # Original C Reference
    /// `r_main.c` lines 973-998: `R_Init(void)`
    fn init(&mut self) {
        // The full initialization pipeline will be wired through the `main`
        // module's `r_init` function once all sub-module state structs and
        // the WAD provider are connected via the game loop in doom-bin.
        todo!("R_Init: wire through main::r_init when sub-modules are connected")
    }

    /// Sets the pending view size and detail level for deferred application.
    ///
    /// This method does NOT immediately recalculate the view parameters.
    /// Instead, it sets the `setsizeneeded` flag and stores the pending
    /// `blocks` and `detail` values. The actual recalculation occurs at
    /// the start of the next frame via `R_ExecuteSetViewSize` in the
    /// [`main`] module.
    ///
    /// This deferred pattern matches the original C behavior exactly —
    /// `R_SetViewSize` in `r_main.c` lines 299-305 simply stores values
    /// and sets the flag, while `R_ExecuteSetViewSize` (lines 255-297)
    /// performs the actual recalculation when polled.
    ///
    /// # Parameters
    /// - `blocks` — View size in screen blocks (3–11). Values 3–10 show the
    ///   status bar; value 11 is full-screen rendering without the status bar.
    /// - `detail` — Detail level: 0 = high detail (normal), 1 = low detail
    ///   (blocky mode where each column is drawn twice as wide).
    ///
    /// # Original C Reference
    /// `r_main.c` lines 299-305:
    /// ```c
    /// void R_SetViewSize(int blocks, int detail) {
    ///     setsizeneeded = true;
    ///     setblocks = blocks;
    ///     setdetail = detail;
    /// }
    /// ```
    fn set_view_size(&mut self, blocks: i32, detail: i32) {
        self.setsizeneeded = true;
        self.setblocks = blocks;
        self.setdetail = detail;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_software_renderer_new_defaults() {
        let renderer = SoftwareRenderer::new();
        // setsizeneeded should be true to trigger initial view size calculation
        assert!(renderer.setsizeneeded);
        // Default view blocks: 10 (largest windowed view with status bar)
        assert_eq!(renderer.setblocks, 10);
        // Default detail: 0 (high detail)
        assert_eq!(renderer.setdetail, 0);
    }

    #[test]
    fn test_software_renderer_default_matches_new() {
        let r_new = SoftwareRenderer::new();
        let r_default = SoftwareRenderer::default();
        assert_eq!(r_new.setsizeneeded, r_default.setsizeneeded);
        assert_eq!(r_new.setblocks, r_default.setblocks);
        assert_eq!(r_new.setdetail, r_default.setdetail);
    }

    #[test]
    fn test_set_view_size_sets_flag_and_values() {
        let mut renderer = SoftwareRenderer::new();
        // Clear the flag first
        renderer.setsizeneeded = false;
        renderer.setblocks = 0;
        renderer.setdetail = 0;

        // Call set_view_size via the Renderer trait
        Renderer::set_view_size(&mut renderer, 7, 1);

        assert!(renderer.setsizeneeded);
        assert_eq!(renderer.setblocks, 7);
        assert_eq!(renderer.setdetail, 1);
    }

    #[test]
    fn test_set_view_size_minimum_blocks() {
        let mut renderer = SoftwareRenderer::new();
        renderer.setsizeneeded = false;

        Renderer::set_view_size(&mut renderer, 3, 0);

        assert!(renderer.setsizeneeded);
        assert_eq!(renderer.setblocks, 3);
        assert_eq!(renderer.setdetail, 0);
    }

    #[test]
    fn test_set_view_size_maximum_blocks() {
        let mut renderer = SoftwareRenderer::new();
        renderer.setsizeneeded = false;

        // blocks=11 is full-screen (no status bar)
        Renderer::set_view_size(&mut renderer, 11, 0);

        assert!(renderer.setsizeneeded);
        assert_eq!(renderer.setblocks, 11);
        assert_eq!(renderer.setdetail, 0);
    }

    #[test]
    fn test_set_view_size_low_detail() {
        let mut renderer = SoftwareRenderer::new();
        renderer.setsizeneeded = false;

        Renderer::set_view_size(&mut renderer, 10, 1);

        assert!(renderer.setsizeneeded);
        assert_eq!(renderer.setblocks, 10);
        assert_eq!(renderer.setdetail, 1);
    }

    #[test]
    fn test_set_view_size_overwrites_previous() {
        let mut renderer = SoftwareRenderer::new();

        // First call
        Renderer::set_view_size(&mut renderer, 5, 0);
        assert_eq!(renderer.setblocks, 5);
        assert_eq!(renderer.setdetail, 0);

        // Second call should overwrite
        Renderer::set_view_size(&mut renderer, 8, 1);
        assert_eq!(renderer.setblocks, 8);
        assert_eq!(renderer.setdetail, 1);
        assert!(renderer.setsizeneeded);
    }

    #[test]
    fn test_renderer_is_object_safe() {
        // Verify that SoftwareRenderer can be used as a trait object
        fn _accepts_renderer(_r: &dyn Renderer) {}
        let renderer = SoftwareRenderer::new();
        _accepts_renderer(&renderer);
    }

    #[test]
    fn test_struct_fields_are_public() {
        // Verify all three fields are publicly accessible
        let mut renderer = SoftwareRenderer::new();
        renderer.setsizeneeded = false;
        renderer.setblocks = 42;
        renderer.setdetail = 1;
        assert!(!renderer.setsizeneeded);
        assert_eq!(renderer.setblocks, 42);
        assert_eq!(renderer.setdetail, 1);
    }
}
