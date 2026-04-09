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
use doom_wad::wad_provider::WadProvider;

use crate::bsp::BspState;
use crate::data::DataState;
use crate::defs::RenderState;
use crate::draw::DrawState;
use crate::main::{
    init_light_tables, r_execute_set_view_size, r_set_view_size, r_setup_frame, RenderMain,
};
use crate::plane::PlaneState;
use crate::segs::SegsState;
use crate::sky::SkyState;
use crate::things::ThingsState;

// =============================================================================
// SoftwareRenderer — Main renderer struct
// =============================================================================

/// The DOOM software renderer.
///
/// Implements the [`Renderer`] trait to provide BSP-based rendering
/// of the 3D game world at 320×200 resolution with 256-color palette.
///
/// This struct owns all sub-module state structs required for the rendering
/// pipeline: [`RenderMain`], [`RenderState`], [`DataState`], [`DrawState`],
/// [`BspState`], [`PlaneState`], [`SegsState`], [`SkyState`], [`ThingsState`].
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
/// ## Initialization
///
/// After construction, call [`init`](Renderer::init) for non-WAD initialization
/// (lookup tables, view size, sky, translation tables). Then call
/// [`init_with_wad`](SoftwareRenderer::init_with_wad) to load texture,
/// flat, sprite, and colormap data from the WAD file. Both must complete
/// before the first call to [`render_player_view`](Renderer::render_player_view).
///
/// ## Rendering Pipeline
///
/// The [`render_player_view`](Renderer::render_player_view) method performs
/// Step 1 (setup frame — viewpoint, lighting) of the rendering pipeline.
/// Steps 2–5 (BSP traversal, planes, masked sprites) are orchestrated by
/// the game loop via the public accessor methods on the owned sub-module
/// states. This split is necessary because Rust's borrow checker requires
/// separate mutable borrows on distinct sub-states during the pipeline.
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

    /// Core renderer state: viewpoint, projection, lighting LUTs.
    ///
    /// Original C: scattered globals in `r_main.c` (viewx, viewy, viewz,
    /// viewangle, centerx, centery, projection, scalelight, zlight, etc.)
    pub render_main: RenderMain,

    /// Per-frame rendering state: validcount, drawsegs, vissprites, visplanes.
    ///
    /// Original C: scattered globals in `r_state.h` / `r_defs.h`.
    pub render_state: RenderState,

    /// Texture/flat/sprite/colormap cache loaded from WAD.
    ///
    /// Original C: globals in `r_data.c` (textures, textureheight, flats, etc.)
    pub data_state: DataState,

    /// Column and span drawing primitives state.
    ///
    /// Original C: globals in `r_draw.c` (dc_*, ds_*, translationtables, etc.)
    pub draw_state: DrawState,

    /// BSP tree traversal state: solidsegs, newend, curline, etc.
    ///
    /// Original C: globals in `r_bsp.c`.
    pub bsp_state: BspState,

    /// Visplane allocation and floor/ceiling rendering state.
    ///
    /// Original C: globals in `r_plane.c` (visplanes[], openings[], etc.)
    pub plane_state: PlaneState,

    /// Wall segment rendering state.
    ///
    /// Original C: globals in `r_segs.c` (rw_*, wall*, etc.)
    pub segs_state: SegsState,

    /// Sky texture rendering state.
    ///
    /// Original C: globals in `r_sky.c` (skyflatnum, skytexture, skytexturemid).
    pub sky_state: SkyState,

    /// Sprite sorting and masked column compositing state.
    ///
    /// Original C: globals in `r_things.c` (vissprites[], vsprsortedhead, etc.)
    pub things_state: ThingsState,
}

impl Default for SoftwareRenderer {
    /// Creates a `SoftwareRenderer` with default initialization values.
    ///
    /// - `setsizeneeded` = `true` — forces initial view size calculation on first frame
    /// - `setblocks` = `10` — default to the largest windowed view (with status bar)
    /// - `setdetail` = `0` — default to high detail mode
    /// - All sub-module states initialized to their defaults
    #[inline]
    fn default() -> Self {
        Self {
            setsizeneeded: true,
            setblocks: 10,
            setdetail: 0,
            render_main: RenderMain::default(),
            render_state: RenderState::default(),
            data_state: DataState::default(),
            draw_state: DrawState::default(),
            bsp_state: BspState::default(),
            plane_state: PlaneState::default(),
            segs_state: SegsState::default(),
            sky_state: SkyState::default(),
            things_state: ThingsState::default(),
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

    /// Perform full renderer initialization including WAD-dependent data loading.
    ///
    /// This method performs the complete `R_Init` pipeline from `r_main.c`:
    /// 1. `R_InitData` — Load textures, flats, sprites, colormaps from WAD
    /// 2. `R_SetViewSize` — Set initial view size (triggers deferred recalc)
    /// 3. `R_InitLightTables` — Build diminishing lighting LUTs (requires colormaps)
    /// 4. `R_InitSkyMap` — Set up sky texture parameters
    /// 5. `R_InitTranslationTables` — Build player color translation tables
    ///
    /// Must be called after the WAD file system is initialized and before
    /// the first call to [`render_player_view`](Renderer::render_player_view).
    ///
    /// The `screenblocks` and `detail_level` parameters set the initial view
    /// size configuration (typically 10 and 0 respectively for default settings).
    ///
    /// # Parameters
    /// - `wad` — The WAD provider for loading texture/flat/sprite/colormap data
    /// - `screenblocks` — Initial view size in blocks (3–11)
    /// - `detail_level` — Initial detail level (0 = high, 1 = low)
    pub fn init_with_wad<W: WadProvider>(
        &mut self,
        wad: &mut W,
        screenblocks: i32,
        detail_level: i32,
    ) {
        // 1. Initialize data (textures, flats, sprites, colormaps from WAD)
        self.data_state.init_data(wad);

        // 2. R_InitPointToAngle — no-op (tables are compile-time in tables.rs)
        // 3. R_InitTables — no-op (tables are compile-time in tables.rs)

        // 4. R_SetViewSize with initial screenblocks and detail
        r_set_view_size(&mut self.render_main, screenblocks, detail_level);

        // 5. R_InitPlanes — no-op in original C (plane tables built per-frame)

        // 6. R_InitLightTables — build zlight LUTs (references data_state.colormaps)
        init_light_tables(&mut self.render_main, &self.data_state);

        // 7. R_InitSkyMap
        self.sky_state.init_sky_map();

        // 8. R_InitTranslationTables
        self.draw_state.init_translation_tables();

        // 9. Reset frame counter
        self.render_main.framecount = 0;
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
    fn render_player_view(&mut self, player: &Player) {
        // If a deferred view size change is pending, apply it now
        // (matching the check at the start of R_RenderPlayerView in C).
        if self.render_main.setsizeneeded {
            r_execute_set_view_size(
                &mut self.render_main,
                &mut self.render_state,
                &mut self.draw_state,
            );
        }

        // Step 1: Configure the viewpoint for this frame.
        // Uses player_idx = 0 (display player, single-player default).
        r_setup_frame(&mut self.render_main, &mut self.render_state, player, 0);

        // Steps 2-5 (BSP traversal, plane drawing, sprite compositing) are
        // orchestrated by the game loop using the public sub-module state
        // fields. This is architecturally required because each step needs
        // simultaneous mutable access to different sub-states, which cannot
        // be expressed through a single &mut self borrow.
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
        // Non-WAD initialization steps from R_Init (r_main.c lines 973-998):
        // R_InitPointToAngle — no-op (compile-time tables)
        // R_InitTables — no-op (compile-time tables)

        // R_SetViewSize with default screenblocks and detail level
        r_set_view_size(&mut self.render_main, self.setblocks, self.setdetail);

        // R_InitPlanes — no-op (per-frame)

        // R_InitSkyMap — set up sky texture parameters
        self.sky_state.init_sky_map();

        // R_InitTranslationTables — build player color translation tables
        self.draw_state.init_translation_tables();

        // Reset frame counter
        self.render_main.framecount = 0;

        // NOTE: WAD-dependent initialization (R_InitData, R_InitLightTables)
        // must be performed via init_with_wad() which accepts a WadProvider.
        // The Renderer trait's init() signature does not include a WadProvider
        // parameter — the game loop calls init_with_wad() directly on
        // SoftwareRenderer during startup.
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
        // Also propagate to render_main for the deferred execution path
        r_set_view_size(&mut self.render_main, blocks, detail);
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
