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

//! Renderer trait definition — software rendering abstraction.
//!
//! Translated from `linuxdoom-1.10/r_main.h`.
//!
//! Defines the interface for the software renderer that `doom-render-soft`
//! implements. The trait decouples `doom-core` game logic from the specific
//! rendering implementation, enabling future rendering backends (e.g.,
//! hardware-accelerated) without modifying core game code.
//!
//! ## Original C Interface (r_main.h)
//!
//! The original C codebase declared the following public functions:
//! - `R_Init()` — Initialize the renderer (load textures, build lookup tables)
//! - `R_RenderPlayerView(player_t* player)` — Render the 3D view for a player
//! - `R_SetViewSize(int blocks, int detail)` — Set screen size and detail level
//!
//! These are translated into the three methods of the [`Renderer`] trait below.
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

use crate::types::player::Player;

/// Platform-independent renderer interface.
///
/// Abstracts the software rendering pipeline so that `doom-core` game logic
/// can issue render commands without depending on the specific renderer
/// implementation. The `doom-render-soft` crate provides the concrete
/// implementation of this trait.
///
/// ## Design Notes
///
/// - Uses `&mut self` for all methods since rendering mutates internal state
///   (viewpoint, visplane arrays, sprite lists, column drawing state, etc.).
/// - The `render_player_view` method takes a `&Player` reference rather than
///   a raw pointer, matching Rust's ownership model while preserving the
///   original C function's read-only relationship with the player struct.
/// - The trait is object-safe (no generic methods) to enable dynamic dispatch
///   via `dyn Renderer` if needed.
///
/// ## Method Receiver Conventions
///
/// - `&mut self` for all methods: rendering modifies internal lookup tables,
///   visplane arrays, sprite sort buffers, and column drawing state.
pub trait Renderer {
    /// Initialize the renderer subsystem.
    ///
    /// Equivalent of `R_Init()` from `r_main.h` / `r_main.c` (lines 1002-1037).
    ///
    /// Performs all one-time renderer initialization:
    /// - `R_InitData()` — load textures, flats, sprites, and colormaps from WAD
    /// - `R_InitPointToAngle()` — build point-to-angle lookup table
    /// - `R_InitTables()` — build trigonometric lookup tables
    /// - `R_SetViewSize(screenblocks, detailLevel)` — initial view window setup
    /// - `R_InitPlanes()` — initialize visplane system
    /// - `R_InitLightTables()` — build diminishing lighting lookup tables
    /// - `R_InitSkyMap()` — configure sky texture rendering
    /// - `R_InitTranslationTables()` — build player color translation tables
    ///
    /// Must be called after the WAD file system is initialized and before
    /// the first call to [`render_player_view`](Self::render_player_view).
    fn init(&mut self);

    /// Render the 3D first-person view for the given player.
    ///
    /// Equivalent of `R_RenderPlayerView(player_t* player)` from `r_main.h` /
    /// `r_main.c` (lines 1046-1094).
    ///
    /// This is the main rendering entry point, called once per frame from
    /// `D_Display`. It performs the complete rendering pipeline:
    /// 1. `R_SetupFrame(player)` — set viewpoint from player position/angle
    /// 2. `R_ClearClipSegs()` — reset wall clipping arrays
    /// 3. `R_ClearDrawSegs()` — reset draw segment list
    /// 4. `R_ClearPlanes()` — reset visplane array
    /// 5. `R_ClearSprites()` — reset sprite sort buffer
    /// 6. `R_RenderBSPNode(numnodes - 1)` — traverse BSP tree, render walls and flats
    /// 7. `R_DrawPlanes()` — render floor/ceiling visplanes
    /// 8. `R_DrawMasked()` — render sprites and masked (transparent) textures
    ///
    /// Output is written to `screens[0]` (the primary display buffer).
    ///
    /// # Parameters
    ///
    /// - `player`: Reference to the player whose viewpoint is being rendered.
    ///   The renderer reads the player's position (`mo.x`, `mo.y`, `mo.z`),
    ///   viewing angle (`mo.angle`), and extra light (`extralight`) from this
    ///   struct.
    fn render_player_view(&mut self, player: &Player);

    /// Set the view window size and detail level.
    ///
    /// Equivalent of `R_SetViewSize(int blocks, int detail)` from `r_main.h` /
    /// `r_main.c` (lines 963-998).
    ///
    /// Called when the user changes screen size or detail level from the
    /// options menu, and once during [`init`](Self::init) for the initial
    /// setup.
    ///
    /// # Parameters
    ///
    /// - `blocks`: Screen size in blocks (3–11). At block 11 the view fills
    ///   the full screen including the status bar area. Values 3–10 produce
    ///   progressively smaller centered view windows with the status bar
    ///   visible. The view window width is `blocks * 32` pixels.
    /// - `detail`: Detail level (0 = high detail, 1 = low/blocky detail).
    ///   In low detail mode, every other column is skipped and columns are
    ///   doubled, halving the horizontal resolution for performance.
    ///
    /// This method sets a "recalc" flag that causes the actual view window
    /// recalculation to happen at the start of the next
    /// [`render_player_view`](Self::render_player_view) call via
    /// `R_ExecuteSetViewSize`.
    fn set_view_size(&mut self, blocks: i32, detail: i32);
}
