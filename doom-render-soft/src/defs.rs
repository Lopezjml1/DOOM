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

//! Translated from linuxdoom-1.10/r_defs.h and r_state.h
//!
//! Refresh/rendering module, shared data struct definitions.
//! Re-exports shared map types from doom-core and defines renderer-internal types.
//!
//! This module serves as the canonical import hub for all doom-render-soft modules.
//! Rather than reaching directly into doom-core sub-modules, other renderer files
//! import types, constants, and the `RenderState` struct from `defs.rs`.
//!
//! # Architecture
//!
//! Many types in r_defs.h are shared between the play (game logic) and refresh
//! (render) subsystems. The canonical versions live in `doom_core::types::map_data`;
//! this module re-exports them so that renderer code has a single, consistent
//! import source.
//!
//! The `RenderState` struct collects all formerly-global renderer state variables
//! from r_state.h into a single owned struct, eliminating `static mut` usage and
//! enabling the Rust borrow checker to enforce safe state access patterns.
//!
//! # Index-Based Architecture
//!
//! The renderer uses arena-style indices (`usize`) instead of C pointers.
//! All `foo_t*` pointer fields become `Option<usize>` or `usize` indices into
//! the corresponding `Vec` in `RenderState`. This is safe, idiomatic Rust that
//! avoids lifetime complications and `unsafe` code.

// =============================================================================
// Re-exports from doom-core::types::map_data
// =============================================================================
//
// These are the canonical type definitions shared between the play (game logic)
// and refresh (rendering) subsystems. They are defined once in doom-core and
// re-exported here so renderer modules can import from a single location.

/// Re-export map geometry types: runtime vertex, sector, sidedef, linedef,
/// seg, subsector, and BSP node structures.
pub use doom_core::types::map_data::{LineDef, Node, Sector, Seg, SideDef, Subsector, Vertex};

/// Re-export renderer-specific data structures: draw segments, visible sprites,
/// visplanes, patches, posts/columns, sprite frames, and sprite definitions.
pub use doom_core::types::map_data::{
    Column, DegenMobj, DrawSeg, Patch, Post, SlopeType, SpriteDef, SpriteFrame, VisSprite, Visplane,
};

/// Re-export silhouette clipping constants used for sprite-against-wall clipping.
///
/// Original C (r_defs.h lines 50-53):
/// - `SIL_NONE  = 0` — No silhouette clipping needed.
/// - `SIL_BOTTOM = 1` — Clip sprites at the bottom silhouette only.
/// - `SIL_TOP    = 2` — Clip sprites at the top silhouette only.
/// - `SIL_BOTH   = 3` — Clip sprites at both top and bottom silhouettes.
pub use doom_core::types::map_data::{SIL_BOTH, SIL_BOTTOM, SIL_NONE, SIL_TOP};

/// Re-export maximum draw segment count.
///
/// Original C (r_defs.h line 55): `#define MAXDRAWSEGS 256`
pub use doom_core::types::map_data::MAXDRAWSEGS;

/// Re-export the light table type alias.
///
/// In the original C: `typedef byte lighttable_t;` (r_defs.h line 314).
/// A single byte representing a palette index lookup entry in colormaps.
/// Could be wider for >8 bit display, but DOOM 1.10 uses 8-bit palettized
/// rendering with 256-entry colormaps.
pub use doom_core::types::map_data::LightTable;

// =============================================================================
// Re-exports from doom-core::types::fixed
// =============================================================================
//
// Fixed-point 16.16 arithmetic — the fundamental numeric type for all spatial
// coordinates, scales, distances, and positional values in the renderer.

/// Re-export the `Fixed` newtype (i32 wrapper) for 16.16 fixed-point arithmetic.
pub use doom_core::types::fixed::Fixed;

/// Number of fractional bits in the 16.16 fixed-point format.
/// Original C: `#define FRACBITS 16` (m_fixed.h line 35).
pub use doom_core::types::fixed::FRACBITS;

/// The value representing 1.0 in 16.16 fixed-point format (= 65536).
/// Original C: `#define FRACUNIT (1<<FRACBITS)` (m_fixed.h line 36).
pub use doom_core::types::fixed::FRACUNIT;

// =============================================================================
// Re-exports from doom-core::types::angle
// =============================================================================
//
// Binary Angle Measurement (BAM) — used for viewangle, clipangle,
// rw_normalangle, seg angles, and all rotational calculations in the renderer.

/// Re-export the `Angle` newtype (u32 wrapper) for BAM angle values.
pub use doom_core::types::angle::Angle;

/// Re-export standard BAM angle constants (45°, 90°, 180°, 270°).
pub use doom_core::types::angle::{ANG180, ANG270, ANG45, ANG90};

/// Number of entries in the fine-angle lookup tables (8192).
/// Used for sizing `viewangletox` (FINEANGLES/2 entries).
pub use doom_core::types::angle::FINEANGLES;

/// Right-shift amount to convert a BAM angle to a fine-angle index.
/// `BAM >> 19` maps the 32-bit BAM range onto the 13-bit fine-angle range.
pub use doom_core::types::angle::ANGLETOFINESHIFT;

// =============================================================================
// Re-exports from doom-core::types::doomdef
// =============================================================================
//
// Screen dimension constants used for array sizing and rendering bounds.

/// Native render buffer width in pixels (320).
/// Original C: `#define SCREENWIDTH 320` (doomdef.h).
pub use doom_core::types::doomdef::SCREENWIDTH;

/// Native render buffer height in pixels (200).
/// Original C: `#define SCREENHEIGHT 200` (doomdef.h).
pub use doom_core::types::doomdef::SCREENHEIGHT;

// =============================================================================
// RenderState — Collected renderer globals from r_state.h
// =============================================================================

/// Consolidated renderer state structure collecting all formerly-global variables
/// from `r_state.h`.
///
/// In the original C code, these were `extern` global variables accessed from
/// multiple translation units. In the Rust port, they are collected into a single
/// struct that is owned by the renderer and passed by mutable reference through
/// the rendering call chain.
///
/// # Sections
///
/// The fields are organized into three logical groups matching the original
/// r_state.h layout:
///
/// 1. **Map geometry data** (loaded per level) — sprite definitions, vertexes,
///    segs, sectors, subsectors, nodes, lines, and sides.
///
/// 2. **POV (point-of-view) data** (updated per frame) — viewer position,
///    angle, and derived trigonometric values.
///
/// 3. **Rendering state** (cross-module) — clip angle, lookup tables, wall
///    rendering state, and current visplane pointers.
///
/// # Original C References
///
/// - `r_state.h` lines 41-109: All extern declarations for renderer globals
/// - `r_main.h`: `viewcos`, `viewsin` declarations
pub struct RenderState {
    // =========================================================================
    // Map geometry data — loaded per level (r_state.h lines 41-70)
    // =========================================================================
    /// Sprite definitions loaded from the WAD (indexed by sprite number).
    ///
    /// Original C: `spritedef_t* sprites` (r_state.h line 76, via `R_InitSprites`).
    pub sprites: Vec<SpriteDef>,

    /// Number of sprite definitions in the `sprites` array.
    ///
    /// Original C: `int numsprites` (r_state.h line 75).
    pub numsprites: usize,

    /// Map vertex array — runtime vertices with fixed-point coordinates.
    ///
    /// Original C: `vertex_t* vertexes` (r_state.h line 79).
    pub vertexes: Vec<Vertex>,

    /// Number of vertices in the `vertexes` array.
    ///
    /// Original C: `int numvertexes` (r_state.h line 78).
    pub numvertexes: usize,

    /// Map line segment array — segs are LineDef fragments after BSP splitting.
    ///
    /// Original C: `seg_t* segs` (r_state.h line 82).
    pub segs: Vec<Seg>,

    /// Number of segs in the `segs` array.
    ///
    /// Original C: `int numsegs` (r_state.h line 81).
    pub numsegs: usize,

    /// Map sector array — sectors define floor/ceiling height, textures, and
    /// lighting for each convex polygon in the map.
    ///
    /// Original C: `sector_t* sectors` (r_state.h line 85).
    pub sectors: Vec<Sector>,

    /// Number of sectors in the `sectors` array.
    ///
    /// Original C: `int numsectors` (r_state.h line 84).
    pub numsectors: usize,

    /// Map subsector array — BSP leaf nodes referencing sectors and seg ranges.
    ///
    /// Original C: `subsector_t* subsectors` (r_state.h line 88).
    pub subsectors: Vec<Subsector>,

    /// Number of subsectors in the `subsectors` array.
    ///
    /// Original C: `int numsubsectors` (r_state.h line 87).
    pub numsubsectors: usize,

    /// BSP node array — internal nodes of the BSP tree used for front-to-back
    /// traversal during rendering.
    ///
    /// Original C: `node_t* nodes` (r_state.h line 91).
    pub nodes: Vec<Node>,

    /// Number of BSP nodes in the `nodes` array.
    ///
    /// Original C: `int numnodes` (r_state.h line 90).
    pub numnodes: usize,

    /// Map linedef array — lines connecting vertices with associated textures,
    /// flags, and sector references.
    ///
    /// Original C: `line_t* lines` (r_state.h line 94).
    pub lines: Vec<LineDef>,

    /// Number of linedefs in the `lines` array.
    ///
    /// Original C: `int numlines` (r_state.h line 93).
    pub numlines: usize,

    /// Map sidedef array — visual appearance data for each side of a linedef.
    ///
    /// Original C: `side_t* sides` (r_state.h line 97).
    pub sides: Vec<SideDef>,

    /// Number of sidedefs in the `sides` array.
    ///
    /// Original C: `int numsides` (r_state.h line 96).
    pub numsides: usize,

    // =========================================================================
    // POV (point-of-view) data — updated per frame (r_state.h lines 78-84)
    // =========================================================================
    /// Viewer X position in 16.16 fixed-point map coordinates.
    ///
    /// Original C: `fixed_t viewx` (r_state.h line 103).
    pub viewx: i32,

    /// Viewer Y position in 16.16 fixed-point map coordinates.
    ///
    /// Original C: `fixed_t viewy` (r_state.h line 104).
    pub viewy: i32,

    /// Viewer Z position (eye height) in 16.16 fixed-point map coordinates.
    ///
    /// Original C: `fixed_t viewz` (r_state.h line 105).
    pub viewz: i32,

    /// Viewer facing direction as a BAM (Binary Angle Measurement) value.
    /// Full 32-bit range represents 360 degrees.
    ///
    /// Original C: `angle_t viewangle` (r_state.h line 107).
    pub viewangle: u32,

    /// Cosine of `viewangle` in 16.16 fixed-point.
    /// Pre-calculated each frame for use in coordinate transformations.
    ///
    /// Original C: `fixed_t viewcos` (derived from r_main.c).
    pub viewcos: i32,

    /// Sine of `viewangle` in 16.16 fixed-point.
    /// Pre-calculated each frame for use in coordinate transformations.
    ///
    /// Original C: `fixed_t viewsin` (derived from r_main.c).
    pub viewsin: i32,

    /// Index of the player whose viewpoint is being rendered, into the
    /// global `players[]` array.
    ///
    /// Original C: `player_t* viewplayer` (r_state.h line 108) — converted
    /// from pointer to index.
    pub viewplayer_idx: usize,

    // =========================================================================
    // Rendering state — cross-module (r_state.h lines 90-109)
    // =========================================================================
    /// Half-width of the field of view in BAM units. Used to determine which
    /// segs are potentially visible from the current viewpoint.
    ///
    /// Original C: `angle_t clipangle` (r_state.h line 112).
    pub clipangle: u32,

    /// Lookup table mapping fine-angle indices to screen X columns.
    /// Sized to `FINEANGLES / 2` (4096) entries. Given a fine-angle index
    /// representing the horizontal angle from the view center, this table
    /// returns the corresponding screen column.
    ///
    /// Original C: `int viewangletox[FINEANGLES/2]` (r_state.h line 114).
    pub viewangletox: Vec<i32>,

    /// Inverse lookup table mapping screen X columns to view angles.
    /// Sized to `SCREENWIDTH + 1` (321) entries. Given a screen column,
    /// returns the BAM angle from the view center to that column.
    ///
    /// Original C: `angle_t xtoviewangle[SCREENWIDTH+1]` (r_state.h line 115).
    pub xtoviewangle: Vec<u32>,

    /// Distance from the viewer to the current wall segment being rendered,
    /// in 16.16 fixed-point. Shared between `segs.rs` and `things.rs`.
    ///
    /// Original C: `fixed_t rw_distance` (r_state.h line 118).
    pub rw_distance: i32,

    /// Normal angle of the current wall segment being rendered, as a BAM value.
    /// Shared between `segs.rs` and `things.rs`.
    ///
    /// Original C: `angle_t rw_normalangle` (r_state.h line 119).
    pub rw_normalangle: u32,

    /// Angle from the viewer to the start vertex (v1) of the current wall
    /// segment, stored as a raw integer (matching original C `int rw_angle1`).
    ///
    /// Original C: `int rw_angle1` (r_state.h line 124).
    pub rw_angle1: i32,

    /// Count of subsectors rendered in the current frame (performance tracking).
    ///
    /// Original C: `int sscount` (r_state.h line 127).
    pub sscount: i32,

    /// Index into the visplane pool for the current floor visplane, or `None`
    /// if no floor plane is active for the current subsector.
    ///
    /// Original C: `visplane_t* floorplane` (r_state.h line 129).
    pub floorplane: Option<usize>,

    /// Index into the visplane pool for the current ceiling visplane, or `None`
    /// if no ceiling plane is active for the current subsector.
    ///
    /// Original C: `visplane_t* ceilingplane` (r_state.h line 130).
    pub ceilingplane: Option<usize>,
}

impl RenderState {
    /// Create a new `RenderState` with all fields initialized to safe defaults.
    ///
    /// Geometry arrays (`vertexes`, `segs`, `sectors`, etc.) start empty and are
    /// populated when a level is loaded via `P_SetupLevel`. The `viewangletox`
    /// and `xtoviewangle` lookup tables are pre-allocated to their required sizes
    /// and filled with zeros; they are properly initialized during `R_Init`.
    pub fn new() -> Self {
        Self {
            // Map geometry — empty until level load
            sprites: Vec::new(),
            numsprites: 0,
            vertexes: Vec::new(),
            numvertexes: 0,
            segs: Vec::new(),
            numsegs: 0,
            sectors: Vec::new(),
            numsectors: 0,
            subsectors: Vec::new(),
            numsubsectors: 0,
            nodes: Vec::new(),
            numnodes: 0,
            lines: Vec::new(),
            numlines: 0,
            sides: Vec::new(),
            numsides: 0,

            // POV data — zeroed until first frame
            viewx: 0,
            viewy: 0,
            viewz: 0,
            viewangle: 0,
            viewcos: 0,
            viewsin: 0,
            viewplayer_idx: 0,

            // Rendering state — lookup tables pre-allocated
            clipangle: 0,
            viewangletox: vec![0i32; (FINEANGLES / 2) as usize],
            xtoviewangle: vec![0u32; (SCREENWIDTH + 1) as usize],
            rw_distance: 0,
            rw_normalangle: 0,
            rw_angle1: 0,
            sscount: 0,
            floorplane: None,
            ceilingplane: None,
        }
    }
}

impl Default for RenderState {
    /// Returns a `RenderState` with all fields at their default values.
    ///
    /// Delegates to [`RenderState::new()`] which pre-allocates the lookup
    /// table vectors to their correct sizes.
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Re-exported constant verification
    // -------------------------------------------------------------------------

    #[test]
    fn test_silhouette_constants_match_original() {
        // r_defs.h lines 50-53: exact values
        assert_eq!(SIL_NONE, 0);
        assert_eq!(SIL_BOTTOM, 1);
        assert_eq!(SIL_TOP, 2);
        assert_eq!(SIL_BOTH, 3);
    }

    #[test]
    fn test_maxdrawsegs_matches_original() {
        // r_defs.h line 55: #define MAXDRAWSEGS 256
        assert_eq!(MAXDRAWSEGS, 256);
    }

    #[test]
    fn test_fracbits_and_fracunit() {
        // m_fixed.h lines 35-36
        assert_eq!(FRACBITS, 16);
        assert_eq!(FRACUNIT, 1 << 16);
        assert_eq!(FRACUNIT, 65536);
    }

    #[test]
    fn test_angle_constants() {
        assert_eq!(ANG45.value(), 0x20000000);
        assert_eq!(ANG90.value(), 0x40000000);
        assert_eq!(ANG180.value(), 0x80000000);
        assert_eq!(ANG270.value(), 0xc0000000);
    }

    #[test]
    fn test_fine_angle_constants() {
        assert_eq!(FINEANGLES, 8192);
        assert_eq!(ANGLETOFINESHIFT, 19);
    }

    #[test]
    fn test_screen_dimensions() {
        assert_eq!(SCREENWIDTH, 320);
        assert_eq!(SCREENHEIGHT, 200);
    }

    // -------------------------------------------------------------------------
    // LightTable type verification
    // -------------------------------------------------------------------------

    #[test]
    fn test_lighttable_is_u8() {
        // r_defs.h line 314: typedef byte lighttable_t;
        let lt: LightTable = 255;
        assert_eq!(lt, 255u8);
        assert_eq!(std::mem::size_of::<LightTable>(), 1);
    }

    // -------------------------------------------------------------------------
    // Re-exported type construction verification
    // -------------------------------------------------------------------------

    #[test]
    fn test_vertex_construction() {
        let v = Vertex {
            x: Fixed::from_int(10),
            y: Fixed::from_int(20),
        };
        assert_eq!(v.x.raw(), 10 << FRACBITS);
        assert_eq!(v.y.raw(), 20 << FRACBITS);
    }

    #[test]
    fn test_sector_default() {
        let s = Sector::default();
        assert_eq!(s.floorheight, Fixed::ZERO);
        assert_eq!(s.ceilingheight, Fixed::ZERO);
        assert_eq!(s.floorpic, 0);
        assert_eq!(s.ceilingpic, 0);
        assert_eq!(s.lightlevel, 0);
        assert_eq!(s.special, 0);
        assert_eq!(s.tag, 0);
        assert_eq!(s.soundtraversed, 0);
        assert!(s.soundtarget.is_none());
        assert_eq!(s.blockbox, [0; 4]);
        assert_eq!(s.validcount, 0);
        assert!(s.thinglist.is_none());
        assert!(s.specialdata.is_none());
        assert_eq!(s.linecount, 0);
        assert!(s.lines.is_empty());
    }

    #[test]
    fn test_sidedef_default() {
        let sd = SideDef::default();
        assert_eq!(sd.textureoffset, Fixed::ZERO);
        assert_eq!(sd.rowoffset, Fixed::ZERO);
        assert_eq!(sd.toptexture, 0);
        assert_eq!(sd.bottomtexture, 0);
        assert_eq!(sd.midtexture, 0);
        assert_eq!(sd.sector, 0);
    }

    #[test]
    fn test_linedef_default() {
        let ld = LineDef::default();
        assert_eq!(ld.v1, 0);
        assert_eq!(ld.v2, 0);
        assert_eq!(ld.dx, Fixed::ZERO);
        assert_eq!(ld.dy, Fixed::ZERO);
        assert_eq!(ld.flags, 0);
        assert_eq!(ld.special, 0);
        assert_eq!(ld.tag, 0);
        assert_eq!(ld.sidenum, [0; 2]);
        assert_eq!(ld.slopetype, SlopeType::Horizontal);
        assert!(ld.frontsector.is_none());
        assert!(ld.backsector.is_none());
        assert_eq!(ld.validcount, 0);
        assert!(ld.specialdata.is_none());
    }

    #[test]
    fn test_seg_default() {
        let seg = Seg::default();
        assert_eq!(seg.v1, 0);
        assert_eq!(seg.v2, 0);
        assert_eq!(seg.offset, Fixed::ZERO);
        assert_eq!(seg.angle, Angle::new(0));
        assert_eq!(seg.sidedef, 0);
        assert_eq!(seg.linedef, 0);
        assert_eq!(seg.frontsector, 0);
        assert!(seg.backsector.is_none());
    }

    #[test]
    fn test_subsector_default() {
        let ss = Subsector::default();
        assert_eq!(ss.sector, 0);
        assert_eq!(ss.numlines, 0);
        assert_eq!(ss.firstline, 0);
    }

    #[test]
    fn test_node_default() {
        let n = Node::default();
        assert_eq!(n.x, Fixed::ZERO);
        assert_eq!(n.y, Fixed::ZERO);
        assert_eq!(n.dx, Fixed::ZERO);
        assert_eq!(n.dy, Fixed::ZERO);
        assert_eq!(n.children, [0; 2]);
    }

    #[test]
    fn test_drawseg_default() {
        let ds = DrawSeg::default();
        assert_eq!(ds.curline, 0);
        assert_eq!(ds.x1, 0);
        assert_eq!(ds.x2, 0);
        assert_eq!(ds.scale1, Fixed::ZERO);
        assert_eq!(ds.scale2, Fixed::ZERO);
        assert_eq!(ds.scalestep, Fixed::ZERO);
        assert_eq!(ds.silhouette, 0);
        assert_eq!(ds.bsilheight, Fixed::ZERO);
        assert_eq!(ds.tsilheight, Fixed::ZERO);
        assert!(ds.sprtopclip.is_none());
        assert!(ds.sprbottomclip.is_none());
        assert!(ds.maskedtexturecol.is_none());
    }

    #[test]
    fn test_vissprite_default() {
        let vs = VisSprite::default();
        assert!(vs.prev.is_none());
        assert!(vs.next.is_none());
        assert_eq!(vs.x1, 0);
        assert_eq!(vs.x2, 0);
        assert_eq!(vs.gx, Fixed::ZERO);
        assert_eq!(vs.gy, Fixed::ZERO);
        assert_eq!(vs.gz, Fixed::ZERO);
        assert_eq!(vs.gzt, Fixed::ZERO);
        assert_eq!(vs.startfrac, Fixed::ZERO);
        assert_eq!(vs.scale, Fixed::ZERO);
        assert_eq!(vs.xiscale, Fixed::ZERO);
        assert_eq!(vs.texturemid, Fixed::ZERO);
        assert_eq!(vs.patch, 0);
        assert!(vs.colormap.is_none());
        assert_eq!(vs.mobjflags, 0);
    }

    #[test]
    fn test_visplane_default() {
        let vp = Visplane::default();
        assert_eq!(vp.height, Fixed::ZERO);
        assert_eq!(vp.picnum, 0);
        assert_eq!(vp.lightlevel, 0);
        assert_eq!(vp.minx, 0);
        assert_eq!(vp.maxx, 0);
        assert_eq!(vp.top.len(), SCREENWIDTH as usize);
        assert_eq!(vp.bottom.len(), SCREENWIDTH as usize);
    }

    #[test]
    fn test_patch_default() {
        let p = Patch::default();
        assert_eq!(p.width, 0);
        assert_eq!(p.height, 0);
        assert_eq!(p.leftoffset, 0);
        assert_eq!(p.topoffset, 0);
        assert!(p.columnofs.is_empty());
    }

    #[test]
    fn test_post_default() {
        let post = Post::default();
        assert_eq!(post.topdelta, 0);
        assert_eq!(post.length, 0);
    }

    #[test]
    fn test_column_is_post() {
        // column_t is typedef'd to post_t in the original C
        let c: Column = Post {
            topdelta: 0xff,
            length: 10,
        };
        assert_eq!(c.topdelta, 0xff);
        assert_eq!(c.length, 10);
    }

    #[test]
    fn test_spriteframe_default() {
        let sf = SpriteFrame::default();
        assert!(!sf.rotate);
        assert_eq!(sf.lump, [0; 8]);
        assert_eq!(sf.flip, [0; 8]);
    }

    #[test]
    fn test_spritedef_default() {
        let sd = SpriteDef::default();
        assert_eq!(sd.numframes, 0);
        assert!(sd.spriteframes.is_empty());
    }

    #[test]
    fn test_degenmobj_default() {
        let dm = DegenMobj::default();
        assert_eq!(dm.x, Fixed::ZERO);
        assert_eq!(dm.y, Fixed::ZERO);
        assert_eq!(dm.z, Fixed::ZERO);
    }

    #[test]
    fn test_slopetype_variants() {
        assert_eq!(SlopeType::Horizontal as i32, 0);
        assert_eq!(SlopeType::Vertical as i32, 1);
        assert_eq!(SlopeType::Positive as i32, 2);
        assert_eq!(SlopeType::Negative as i32, 3);
    }

    // -------------------------------------------------------------------------
    // Fixed-point re-export verification
    // -------------------------------------------------------------------------

    #[test]
    fn test_fixed_basic_ops() {
        let a = Fixed::new(FRACUNIT);
        let b = Fixed::from_int(1);
        assert_eq!(a, b);
        assert_eq!(a.raw(), FRACUNIT);

        // 1.0 * 1.0 = 1.0
        assert_eq!(a.fixed_mul(b), Fixed::ONE);

        // 1.0 / 1.0 = 1.0
        assert_eq!(a.fixed_div(b), Fixed::ONE);
    }

    #[test]
    fn test_fixed_zero_and_one() {
        assert_eq!(Fixed::ZERO.raw(), 0);
        assert_eq!(Fixed::ONE.raw(), FRACUNIT);
    }

    #[test]
    fn test_fixed_div2() {
        // 1.0 / 2.0 = 0.5
        let half = Fixed::new(FRACUNIT).fixed_div2(Fixed::new(FRACUNIT * 2));
        assert_eq!(half, Fixed::new(FRACUNIT / 2));
    }

    // -------------------------------------------------------------------------
    // Angle re-export verification
    // -------------------------------------------------------------------------

    #[test]
    fn test_angle_basic_ops() {
        let a = Angle::new(0x40000000);
        assert_eq!(a, ANG90);
        assert_eq!(a.value(), 0x40000000);
    }

    #[test]
    fn test_angle_to_fine() {
        // ANG90 >> 19 = 0x40000000 >> 19 = 0x800 = 2048
        let fine = ANG90.to_fine_angle();
        assert_eq!(fine, 2048);
        assert_eq!(fine, (FINEANGLES / 4) as usize);
    }

    // -------------------------------------------------------------------------
    // RenderState tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_render_state_new() {
        let state = RenderState::new();

        // Map geometry arrays should be empty
        assert!(state.sprites.is_empty());
        assert_eq!(state.numsprites, 0);
        assert!(state.vertexes.is_empty());
        assert_eq!(state.numvertexes, 0);
        assert!(state.segs.is_empty());
        assert_eq!(state.numsegs, 0);
        assert!(state.sectors.is_empty());
        assert_eq!(state.numsectors, 0);
        assert!(state.subsectors.is_empty());
        assert_eq!(state.numsubsectors, 0);
        assert!(state.nodes.is_empty());
        assert_eq!(state.numnodes, 0);
        assert!(state.lines.is_empty());
        assert_eq!(state.numlines, 0);
        assert!(state.sides.is_empty());
        assert_eq!(state.numsides, 0);

        // POV data should be zeroed
        assert_eq!(state.viewx, 0);
        assert_eq!(state.viewy, 0);
        assert_eq!(state.viewz, 0);
        assert_eq!(state.viewangle, 0);
        assert_eq!(state.viewcos, 0);
        assert_eq!(state.viewsin, 0);
        assert_eq!(state.viewplayer_idx, 0);

        // Rendering state
        assert_eq!(state.clipangle, 0);
        assert_eq!(state.rw_distance, 0);
        assert_eq!(state.rw_normalangle, 0);
        assert_eq!(state.rw_angle1, 0);
        assert_eq!(state.sscount, 0);
        assert!(state.floorplane.is_none());
        assert!(state.ceilingplane.is_none());
    }

    #[test]
    fn test_render_state_lookup_table_sizes() {
        let state = RenderState::new();

        // viewangletox should have FINEANGLES/2 = 4096 entries
        assert_eq!(state.viewangletox.len(), (FINEANGLES / 2) as usize);
        assert_eq!(state.viewangletox.len(), 4096);

        // xtoviewangle should have SCREENWIDTH+1 = 321 entries
        assert_eq!(state.xtoviewangle.len(), (SCREENWIDTH + 1) as usize);
        assert_eq!(state.xtoviewangle.len(), 321);
    }

    #[test]
    fn test_render_state_default_matches_new() {
        let from_new = RenderState::new();
        let from_default = RenderState::default();

        // Both should produce identical state
        assert_eq!(from_new.viewx, from_default.viewx);
        assert_eq!(from_new.viewy, from_default.viewy);
        assert_eq!(from_new.viewz, from_default.viewz);
        assert_eq!(from_new.viewangle, from_default.viewangle);
        assert_eq!(from_new.viewcos, from_default.viewcos);
        assert_eq!(from_new.viewsin, from_default.viewsin);
        assert_eq!(from_new.viewplayer_idx, from_default.viewplayer_idx);
        assert_eq!(from_new.clipangle, from_default.clipangle);
        assert_eq!(from_new.rw_distance, from_default.rw_distance);
        assert_eq!(from_new.rw_normalangle, from_default.rw_normalangle);
        assert_eq!(from_new.rw_angle1, from_default.rw_angle1);
        assert_eq!(from_new.sscount, from_default.sscount);
        assert_eq!(from_new.floorplane, from_default.floorplane);
        assert_eq!(from_new.ceilingplane, from_default.ceilingplane);
        assert_eq!(from_new.viewangletox.len(), from_default.viewangletox.len());
        assert_eq!(from_new.xtoviewangle.len(), from_default.xtoviewangle.len());
    }

    #[test]
    fn test_render_state_pov_mutation() {
        let mut state = RenderState::new();

        // Simulate setting up the viewpoint for a frame
        state.viewx = 100 << FRACBITS;
        state.viewy = 200 << FRACBITS;
        state.viewz = 41 << FRACBITS;
        state.viewangle = ANG90.value();
        state.viewcos = 0; // cos(90°) = 0
        state.viewsin = FRACUNIT; // sin(90°) = 1.0
        state.viewplayer_idx = 0;

        assert_eq!(state.viewx, 100 * FRACUNIT);
        assert_eq!(state.viewy, 200 * FRACUNIT);
        assert_eq!(state.viewz, 41 * FRACUNIT);
        assert_eq!(state.viewangle, 0x40000000);
        assert_eq!(state.viewcos, 0);
        assert_eq!(state.viewsin, FRACUNIT);
        assert_eq!(state.viewplayer_idx, 0);
    }

    #[test]
    fn test_render_state_visplane_tracking() {
        let mut state = RenderState::new();

        // No active visplanes initially
        assert!(state.floorplane.is_none());
        assert!(state.ceilingplane.is_none());

        // Set visplane indices (simulating subsector rendering)
        state.floorplane = Some(3);
        state.ceilingplane = Some(7);

        assert_eq!(state.floorplane, Some(3));
        assert_eq!(state.ceilingplane, Some(7));

        // Clear visplanes (between subsectors)
        state.floorplane = None;
        state.ceilingplane = None;

        assert!(state.floorplane.is_none());
        assert!(state.ceilingplane.is_none());
    }

    #[test]
    fn test_render_state_wall_rendering() {
        let mut state = RenderState::new();

        // Simulate wall segment rendering setup
        state.rw_distance = 512 << FRACBITS;
        state.rw_normalangle = ANG90.value();
        state.rw_angle1 = ANG45.value() as i32;
        state.clipangle = ANG90.value();

        assert_eq!(state.rw_distance, 512 * FRACUNIT);
        assert_eq!(state.rw_normalangle, 0x40000000);
        assert_eq!(state.rw_angle1, 0x20000000);
        assert_eq!(state.clipangle, 0x40000000);
    }

    #[test]
    fn test_render_state_geometry_loading() {
        let mut state = RenderState::new();

        // Simulate loading a small map
        state.vertexes = vec![
            Vertex {
                x: Fixed::from_int(0),
                y: Fixed::from_int(0),
            },
            Vertex {
                x: Fixed::from_int(128),
                y: Fixed::from_int(0),
            },
            Vertex {
                x: Fixed::from_int(128),
                y: Fixed::from_int(128),
            },
        ];
        state.numvertexes = 3;

        assert_eq!(state.vertexes.len(), 3);
        assert_eq!(state.numvertexes, 3);
        assert_eq!(state.vertexes[0].x, Fixed::from_int(0));
        assert_eq!(state.vertexes[1].x, Fixed::from_int(128));
        assert_eq!(state.vertexes[2].y, Fixed::from_int(128));
    }

    #[test]
    fn test_render_state_sscount() {
        let mut state = RenderState::new();
        assert_eq!(state.sscount, 0);

        // Increment during BSP traversal
        state.sscount = 42;
        assert_eq!(state.sscount, 42);
    }
}
