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

//! Renderer trait definition — the rendering system platform abstraction.
//!
//! Translated from the "REFRESH" section of `linuxdoom-1.10/r_main.h` (lines 157-165).
//! Defines the interface for rendering the 3D game view from a player's perspective.
//!
//! The original C code declared these as free functions:
//! - `R_RenderPlayerView(player_t *player)` — render the 3D view
//! - `R_Init(void)` — initialize renderer subsystem
//! - `R_SetViewSize(int blocks, int detail)` — set view window size
//!
//! The software renderer implementation lives in `doom-render-soft`.
//! The trait-based approach enables potential future renderer backends
//! (e.g., hardware-accelerated) without modifying core game logic.
//!
//! ## Behavioral Parity Notes
//!
//! - `R_RenderPlayerView` is called once per frame from `D_Display` to produce
//!   the 3D first-person view. It performs BSP traversal, wall/floor/ceiling
//!   rendering, and sprite compositing into `screens[0]`.
//! - `R_SetViewSize` is called from the options menu when the user changes the
//!   screen size (blocks 3–11) or detail level (0 = high, 1 = low). It
//!   recalculates the view window dimensions and rebuilds the column/row
//!   lookup tables.
//! - `R_Init` is called once during startup by `D_DoomMain` after the WAD
//!   file system is initialized. It loads all texture definitions, flat
//!   textures, sprite definitions, and colormaps from the WAD.
//!
//! ## Design Rationale
//!
//! - The trait uses `&mut self` for all methods because rendering modifies
//!   internal state (visplanes, vissprites, lighting LUTs, etc.).
//! - `render_player_view` takes `&Player` (immutable reference) — the renderer
//!   reads player state but does not modify it.
//! - The `Player` type is from `doom-core::types::player` — no platform-specific
//!   types appear in the trait signatures.
//! - The trait is object-safe: no generic methods, no `Self` in return position.
//!   This permits dynamic dispatch via `dyn Renderer` if needed.
//!
//! ## Functions NOT in the Trait
//!
//! The following functions from `r_main.h` are renderer-internal utilities and
//! are **not** part of the external interface. They live inside `doom-render-soft`:
//! - `R_PointOnSide`, `R_PointOnSegSide`, `R_PointToAngle`, `R_PointToAngle2`,
//!   `R_PointToDist` (r_main.h:109-136)
//! - `R_ScaleFromGlobalAngle` (r_main.h:139)
//! - `R_PointInSubsector` (r_main.h:141-144)
//! - `R_AddPointToBox` (r_main.h:147-150)
//! - Lighting globals: `scalelight`, `zlight`, `extralight`, `fixedcolormap`
//! - Function pointers: `colfunc`, `basecolfunc`, `fuzzcolfunc`, `spanfunc`

use crate::types::player::Player;

/// Platform-independent renderer interface.
///
/// Abstracts the rendering of the 3D game view. The primary implementation
/// is the software renderer in `doom-render-soft`, which faithfully reproduces
/// DOOM's original BSP-based rendering with column/span drawing.
///
/// ## Original C Interface (r_main.h:157-165)
///
/// ```c
/// // Called by G_Drawer.
/// void R_RenderPlayerView (player_t *player);
///
/// // Called by startup code.
/// void R_Init (void);
///
/// // Called by M_Responder.
/// void R_SetViewSize (int blocks, int detail);
/// ```
pub trait Renderer {
    /// Render the 3D view from the given player's perspective.
    ///
    /// Equivalent of `R_RenderPlayerView(player_t *player)` from r_main.h:159.
    /// Called by `G_Drawer` (g_game.c) once per frame to render the 3D world
    /// from the player's viewpoint.
    ///
    /// The implementation performs:
    /// 1. Setup frame (viewpoint, lighting calculations)
    /// 2. BSP tree traversal to determine visible subsectors
    /// 3. Wall segment rendering (column drawing)
    /// 4. Floor/ceiling rendering (span drawing)
    /// 5. Sprite sorting and masked column compositing
    ///
    /// # Parameters
    /// - `player`: Reference to the player whose perspective to render from.
    ///   Uses `player.mobj` → position (`x`, `y`) and `player.viewz` for the
    ///   camera position, `player.mobj` → `angle` for view direction, and
    ///   `player.extralight` + `player.fixedcolormap` for lighting overrides.
    fn render_player_view(&mut self, player: &Player);

    /// Initialize the renderer subsystem.
    ///
    /// Equivalent of `R_Init(void)` from r_main.h:162.
    /// Called by startup code during engine initialization.
    ///
    /// The implementation performs:
    /// - Initialize texture, flat, and sprite caches (`R_InitData`)
    /// - Build angle-to-screen-column lookup tables (`R_InitPointToAngle`)
    /// - Initialize lighting lookup tables (`R_InitLightTables`)
    /// - Set up sky rendering (`R_InitSkyMap`)
    /// - Build player color translation tables (`R_InitTranslationTables`)
    /// - Allocate visplane, vissprite, and drawseg arrays
    ///
    /// Must be called after the WAD file system is initialized and before
    /// the first call to [`render_player_view`](Self::render_player_view).
    fn init(&mut self);

    /// Set the view window size and detail level.
    ///
    /// Equivalent of `R_SetViewSize(int blocks, int detail)` from r_main.h:165.
    /// Called by `M_Responder` (m_menu.c) when the user changes the screen size
    /// or detail level in the Options menu.
    ///
    /// # Parameters
    /// - `blocks`: Screen size in blocks (3-11, where 11 = full screen).
    ///   Controls `viewwidth` and `viewheight` relative to SCREENWIDTH/SCREENHEIGHT.
    ///   At block 11 the view fills the full screen including the status bar area.
    ///   Values 3–10 produce progressively smaller centered view windows with the
    ///   status bar visible. The view window width is `blocks * 32` pixels.
    /// - `detail`: Detail level (0 = high, 1 = low/blocky).
    ///   Controls the `detailshift` variable (r_main.h:93: "//B remove this?").
    ///   In low detail mode, every other column is duplicated instead of rendered,
    ///   halving the horizontal resolution for performance.
    ///
    /// This method sets a "recalc" flag that causes the actual view window
    /// recalculation to happen at the start of the next
    /// [`render_player_view`](Self::render_player_view) call via
    /// `R_ExecuteSetViewSize`.
    fn set_view_size(&mut self, blocks: i32, detail: i32);
}
