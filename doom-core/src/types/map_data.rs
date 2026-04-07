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

//! Translated from linuxdoom-1.10/doomdata.h and linuxdoom-1.10/r_defs.h
//!
//! Map data structures for both WAD persistent format and runtime representation.
//!
//! This module contains two categories of structures:
//!
//! 1. **WAD-format (on-disk) structures** — Prefixed with `Map*`, these represent
//!    the binary layout of map data as stored in WAD file lumps. All fields use
//!    `i16`/`u16` types matching the original 16-bit WAD format. These are
//!    deserialized from WAD lumps during map loading (`P_SetupLevel`).
//!
//! 2. **Runtime structures** — Used during gameplay by both the play simulation
//!    (`doom-core/src/play/`) and the renderer (`doom-render-soft/`). These use
//!    `Fixed` (16.16 fixed-point) for spatial coordinates, `Angle` (BAM u32)
//!    for directions, and `usize` arena indices replacing C raw pointers.
//!
//! # Pointer-to-Index Translation
//!
//! The original C code uses raw pointers (`vertex_t*`, `sector_t*`, `line_t**`)
//! extensively. In this Rust port, all pointers are replaced with:
//! - `usize` — Arena index into the corresponding storage vector (vertexes,
//!   sectors, lines, segs, etc.)
//! - `Option<usize>` — Nullable arena index (for backsector, soundtarget, etc.)
//! - `Vec<usize>` — Dynamic array of arena indices (for sector line lists)
//!
//! # Original C Source References
//!
//! - `doomdata.h` lines 43-210: WAD-format map structures and constants
//! - `r_defs.h` lines 48-480: Runtime map structures, renderer types, and
//!   rendering data structures (drawseg, vissprite, visplane, patch, sprite)

use super::angle::Angle;
use super::doomdef::SCREENWIDTH;
use super::doomtype::Byte;
use super::fixed::Fixed;
use super::thinker::Thinker;

// =============================================================================
// Map lump ordering (doomdata.h lines 43-56)
// =============================================================================

/// Lump order in a map WAD. Each map needs a sequence of lumps to provide
/// a complete scene geometry description.
///
/// Original C: anonymous enum at doomdata.h lines 43-56
/// (`ML_LABEL`, `ML_THINGS`, `ML_LINEDEFS`, ..., `ML_BLOCKMAP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MapLump {
    /// A separator, name, ExMx or MAPxx.
    Label = 0,
    /// Monsters, items, and other things.
    Things = 1,
    /// LineDefs, from editing.
    LineDefs = 2,
    /// SideDefs, from editing.
    SideDefs = 3,
    /// Vertices, edited and BSP splits generated.
    Vertexes = 4,
    /// LineSegs, from LineDefs split by BSP.
    Segs = 5,
    /// SubSectors, list of LineSegs.
    SSectors = 6,
    /// BSP nodes.
    Nodes = 7,
    /// Sectors, from editing.
    Sectors = 8,
    /// LUT, sector-sector visibility.
    Reject = 9,
    /// LUT, motion clipping, walls/grid element.
    Blockmap = 10,
}

// =============================================================================
// WAD-format (on-disk) map structures (doomdata.h lines 60-210)
// =============================================================================

/// A single vertex as stored in a WAD lump.
///
/// Original C: `mapvertex_t` (doomdata.h lines 60-64).
/// WAD format uses 16-bit signed coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapVertex {
    /// X coordinate in map units.
    pub x: i16,
    /// Y coordinate in map units.
    pub y: i16,
}

/// A SideDef as stored in a WAD lump, defining the visual appearance of a wall
/// by setting textures and offsets.
///
/// Original C: `mapsidedef_t` (doomdata.h lines 69-78).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapSideDef {
    /// Horizontal texture offset (added to calculated texture column).
    pub textureoffset: i16,
    /// Vertical texture offset (added to calculated texture top).
    pub rowoffset: i16,
    /// Upper texture name (8 bytes, null-padded).
    pub toptexture: [u8; 8],
    /// Lower texture name (8 bytes, null-padded).
    pub bottomtexture: [u8; 8],
    /// Middle texture name (8 bytes, null-padded).
    pub midtexture: [u8; 8],
    /// Front sector index (towards viewer).
    pub sector: i16,
}

/// A LineDef as stored in a WAD lump, used for editing and as input to the
/// BSP builder.
///
/// Original C: `maplinedef_t` (doomdata.h lines 84-93).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapLineDef {
    /// First vertex index.
    pub v1: i16,
    /// Second vertex index.
    pub v2: i16,
    /// Line attribute flags (see [`LineFlags`]).
    pub flags: i16,
    /// Special action type.
    pub special: i16,
    /// Sector tag for triggering effects.
    pub tag: i16,
    /// SideDef indices. `sidenum[1]` will be -1 if one sided.
    pub sidenum: [i16; 2],
}

// =============================================================================
// LineDef attribute flags (doomdata.h lines 99-135)
// =============================================================================

bitflags::bitflags! {
    /// Line attribute flags controlling rendering, collision, and map behavior.
    ///
    /// Original C: `#define ML_BLOCKING 1` through `#define ML_MAPPED 256`
    /// (doomdata.h lines 99-135).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct LineFlags: i16 {
        /// Solid, is an obstacle.
        const ML_BLOCKING = 1;
        /// Blocks monsters only.
        const ML_BLOCKMONSTERS = 2;
        /// Backside will not be present at all if not two sided.
        const ML_TWOSIDED = 4;
        /// Upper texture unpegged.
        const ML_DONTPEGTOP = 8;
        /// Lower texture unpegged.
        const ML_DONTPEGBOTTOM = 16;
        /// In AutoMap: don't map as two sided: IT'S A SECRET!
        const ML_SECRET = 32;
        /// Sound rendering: don't let sound cross two of these.
        const ML_SOUNDBLOCK = 64;
        /// Don't draw on the automap at all.
        const ML_DONTDRAW = 128;
        /// Set if already seen, thus drawn in automap.
        const ML_MAPPED = 256;
    }
}

/// Sector definition as stored in a WAD lump.
///
/// Original C: `mapsector_t` (doomdata.h lines 141-150).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapSector {
    /// Floor height in map units.
    pub floorheight: i16,
    /// Ceiling height in map units.
    pub ceilingheight: i16,
    /// Floor texture name (8 bytes, null-padded).
    pub floorpic: [u8; 8],
    /// Ceiling texture name (8 bytes, null-padded).
    pub ceilingpic: [u8; 8],
    /// Light level (0-255).
    pub lightlevel: i16,
    /// Special sector type (damage, secret, etc.).
    pub special: i16,
    /// Sector tag for triggering effects.
    pub tag: i16,
}

/// SubSector as generated by BSP builder, stored in a WAD lump.
///
/// Original C: `mapsubsector_t` (doomdata.h lines 153-158).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapSubsector {
    /// Number of line segments.
    pub numsegs: i16,
    /// Index of first segment (segs are stored sequentially).
    pub firstseg: i16,
}

/// LineSeg as stored in a WAD lump, generated by splitting LineDefs using
/// partition lines selected by the BSP builder.
///
/// Original C: `mapseg_t` (doomdata.h lines 163-171).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapSeg {
    /// First vertex index.
    pub v1: i16,
    /// Second vertex index.
    pub v2: i16,
    /// Angle (in BAM, stored as i16 in WAD).
    pub angle: i16,
    /// LineDef index this seg belongs to.
    pub linedef: i16,
    /// Side of the LineDef (0 = front, 1 = back).
    pub side: i16,
    /// Offset along the LineDef.
    pub offset: i16,
}

// =============================================================================
// NF_SUBSECTOR constant (doomdata.h line 178)
// =============================================================================

/// Bit flag indicating a BSP node child is a subsector leaf rather than
/// another internal node. Set in the high bit of `MapNode.children[i]`.
///
/// Original C: `#define NF_SUBSECTOR 0x8000` (doomdata.h line 178).
pub const NF_SUBSECTOR: u16 = 0x8000;

/// BSP node structure as stored in a WAD lump.
///
/// Original C: `mapnode_t` (doomdata.h lines 180-196).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapNode {
    /// Partition line start X.
    pub x: i16,
    /// Partition line start Y.
    pub y: i16,
    /// Partition line delta X (x + dx = end X).
    pub dx: i16,
    /// Partition line delta Y (y + dy = end Y).
    pub dy: i16,
    /// Bounding box for each child: `bbox[0]` = right child, `bbox[1]` = left child.
    /// Each bounding box is `[top, bottom, left, right]`.
    pub bbox: [[i16; 4]; 2],
    /// Child node indices. If `NF_SUBSECTOR` bit is set, the lower bits
    /// give the subsector index; otherwise, the value is a node index.
    pub children: [u16; 2],
}

/// Thing definition as stored in a WAD lump: position, orientation, type,
/// plus skill/visibility flags and attributes.
///
/// Original C: `mapthing_t` (doomdata.h lines 203-210).
/// Note: `type` is a reserved keyword in Rust, so the field is named `type_`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapThing {
    /// X position in map units.
    pub x: i16,
    /// Y position in map units.
    pub y: i16,
    /// Facing angle in degrees (0-359).
    pub angle: i16,
    /// DoomEd thing type number. Named `type_` because `type` is a Rust keyword.
    /// Original C field: `short type;`
    pub type_: i16,
    /// Skill/visibility flags (bits: easy, medium, hard, ambush, multiplayer).
    pub options: i16,
}

// =============================================================================
// Silhouette constants (r_defs.h lines 48-55)
// =============================================================================

/// No silhouette clipping needed.
/// Original C: `#define SIL_NONE 0` (r_defs.h line 50).
pub const SIL_NONE: i32 = 0;

/// Clip sprites at the bottom silhouette only.
/// Original C: `#define SIL_BOTTOM 1` (r_defs.h line 51).
pub const SIL_BOTTOM: i32 = 1;

/// Clip sprites at the top silhouette only.
/// Original C: `#define SIL_TOP 2` (r_defs.h line 52).
pub const SIL_TOP: i32 = 2;

/// Clip sprites at both top and bottom silhouettes.
/// Original C: `#define SIL_BOTH 3` (r_defs.h line 53).
pub const SIL_BOTH: i32 = 3;

/// Maximum number of draw segments the renderer can track simultaneously.
/// Original C: `#define MAXDRAWSEGS 256` (r_defs.h line 55).
pub const MAXDRAWSEGS: usize = 256;

// =============================================================================
// Runtime map structures (r_defs.h lines 71-279)
// =============================================================================

/// Runtime vertex with fixed-point coordinates.
///
/// Transformed values are NOT buffered locally (unlike some DOOM source ports).
///
/// Original C: `vertex_t` (r_defs.h lines 71-76).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vertex {
    /// X coordinate in 16.16 fixed-point.
    pub x: Fixed,
    /// Y coordinate in 16.16 fixed-point.
    pub y: Fixed,
}

/// Degenerate map object used as a sound origin for sectors.
///
/// Each sector has a `DegenMobj` at its center for positional sound purposes.
/// The thinker field exists for compatibility with the original C layout where
/// `degenmobj_t` starts with a `thinker_t`, but is not used for thinker
/// list management.
///
/// Original C: `degenmobj_t` (r_defs.h lines 88-95).
#[derive(Debug, Clone, Copy)]
pub struct DegenMobj {
    /// Thinker node — not used for anything, exists for struct layout compatibility.
    pub thinker: Thinker,
    /// X coordinate (16.16 fixed-point).
    pub x: Fixed,
    /// Y coordinate (16.16 fixed-point).
    pub y: Fixed,
    /// Z coordinate (16.16 fixed-point).
    pub z: Fixed,
}

impl Default for DegenMobj {
    fn default() -> Self {
        Self {
            thinker: Thinker::default(),
            x: Fixed::ZERO,
            y: Fixed::ZERO,
            z: Fixed::ZERO,
        }
    }
}

/// Runtime sector record. Stores things/mobjs and is the primary container
/// for floor/ceiling/lighting state during gameplay.
///
/// Original C: `sector_t` (r_defs.h lines 101-135).
///
/// # Pointer Translation
/// - `mobj_t* soundtarget` → `Option<usize>` (arena index into mobj storage)
/// - `mobj_t* thinglist` → `Option<usize>` (head of linked list in mobj arena)
/// - `void* specialdata` → `Option<usize>` (arena index to active thinker)
/// - `struct line_s** lines` → `Vec<usize>` (indices into line array)
#[derive(Debug, Clone)]
pub struct Sector {
    /// Floor height (16.16 fixed-point).
    pub floorheight: Fixed,
    /// Ceiling height (16.16 fixed-point).
    pub ceilingheight: Fixed,
    /// Floor flat texture number.
    pub floorpic: i16,
    /// Ceiling flat texture number.
    pub ceilingpic: i16,
    /// Light level (0-255).
    pub lightlevel: i16,
    /// Special sector type (damage, secret, etc.).
    pub special: i16,
    /// Sector tag for triggering effects.
    pub tag: i16,
    /// Sound traversal marker: 0 = untraversed, 1 or 2 = sndlines - 1.
    pub soundtraversed: i32,
    /// Arena index of the thing that made a sound (or None).
    /// Original C: `mobj_t* soundtarget`.
    pub soundtarget: Option<usize>,
    /// Mapblock bounding box for height changes: [top, bottom, left, right].
    pub blockbox: [i32; 4],
    /// Origin for any sounds played by the sector (center point).
    pub soundorg: DegenMobj,
    /// Validation counter — if equal to `validcount`, already checked this frame.
    pub validcount: i32,
    /// Arena index of the head of the linked list of mobjs in this sector.
    /// Original C: `mobj_t* thinglist`.
    pub thinglist: Option<usize>,
    /// Arena index of the active thinker for reversible sector actions
    /// (doors, floors, ceilings, platforms).
    /// Original C: `void* specialdata`.
    pub specialdata: Option<usize>,
    /// Number of lines bounding this sector.
    pub linecount: i32,
    /// Arena indices of the bounding lines.
    /// Original C: `struct line_s** lines` (array of `linecount` pointers).
    pub lines: Vec<usize>,
}

impl Default for Sector {
    fn default() -> Self {
        Self {
            floorheight: Fixed::ZERO,
            ceilingheight: Fixed::ZERO,
            floorpic: 0,
            ceilingpic: 0,
            lightlevel: 0,
            special: 0,
            tag: 0,
            soundtraversed: 0,
            soundtarget: None,
            blockbox: [0; 4],
            soundorg: DegenMobj::default(),
            validcount: 0,
            thinglist: None,
            specialdata: None,
            linecount: 0,
            lines: Vec::new(),
        }
    }
}

/// Runtime SideDef with fixed-point texture offsets.
///
/// Original C: `side_t` (r_defs.h lines 144-161).
///
/// # Pointer Translation
/// - `sector_t* sector` → `usize` (arena index into sector array)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SideDef {
    /// Horizontal texture offset (added to calculated texture column).
    pub textureoffset: Fixed,
    /// Vertical texture offset (added to calculated texture top).
    pub rowoffset: Fixed,
    /// Upper texture index (resolved from WAD name).
    pub toptexture: i16,
    /// Lower texture index (resolved from WAD name).
    pub bottomtexture: i16,
    /// Middle texture index (resolved from WAD name).
    pub midtexture: i16,
    /// Arena index of the sector this SideDef faces.
    /// Original C: `sector_t* sector`.
    pub sector: usize,
}

/// Move clipping aid for LineDefs — describes the slope direction of a line.
///
/// Original C: `slopetype_t` (r_defs.h lines 168-175).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum SlopeType {
    /// Line runs horizontally (dy == 0).
    #[default]
    Horizontal = 0,
    /// Line runs vertically (dx == 0).
    Vertical = 1,
    /// Line has positive slope (dx and dy have same sign).
    Positive = 2,
    /// Line has negative slope (dx and dy have opposite signs).
    Negative = 3,
}

/// Runtime LineDef — a line segment connecting two vertices with associated
/// properties, textures, and sectors.
///
/// Original C: `line_t` (r_defs.h lines 179-215).
///
/// # Pointer Translation
/// - `vertex_t* v1/v2` → `usize` (arena indices into vertex array)
/// - `sector_t* frontsector/backsector` → `Option<usize>` (arena indices)
/// - `void* specialdata` → `Option<usize>` (arena index to active thinker)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LineDef {
    /// Arena index of the first vertex.
    /// Original C: `vertex_t* v1`.
    pub v1: usize,
    /// Arena index of the second vertex.
    /// Original C: `vertex_t* v2`.
    pub v2: usize,
    /// Precalculated delta X (v2.x - v1.x) for side checking.
    pub dx: Fixed,
    /// Precalculated delta Y (v2.y - v1.y) for side checking.
    pub dy: Fixed,
    /// Line attribute flags.
    pub flags: i16,
    /// Special action type.
    pub special: i16,
    /// Sector tag for triggering effects.
    pub tag: i16,
    /// SideDef indices. `sidenum[1]` will be -1 if one sided.
    pub sidenum: [i16; 2],
    /// Bounding box for the extent of the LineDef: [top, bottom, left, right].
    pub bbox: [Fixed; 4],
    /// Slope type to aid movement clipping.
    pub slopetype: SlopeType,
    /// Arena index of the front sector.
    /// Original C: `sector_t* frontsector`.
    pub frontsector: Option<usize>,
    /// Arena index of the back sector (None for one-sided lines).
    /// Original C: `sector_t* backsector`.
    pub backsector: Option<usize>,
    /// Validation counter — if equal to `validcount`, already checked this frame.
    pub validcount: i32,
    /// Arena index of the active thinker for reversible line actions.
    /// Original C: `void* specialdata`.
    pub specialdata: Option<usize>,
}

/// Runtime SubSector — a convex leaf node of the BSP tree, referencing a
/// sector and a contiguous range of line segments.
///
/// Original C: `subsector_t` (r_defs.h lines 227-233).
///
/// # Pointer Translation
/// - `sector_t* sector` → `usize` (arena index into sector array)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Subsector {
    /// Arena index of the sector this subsector belongs to.
    /// Original C: `sector_t* sector`.
    pub sector: usize,
    /// Number of line segments in this subsector.
    pub numlines: i16,
    /// Index of the first line segment in the segs array.
    pub firstline: i16,
}

/// Runtime line segment (Seg) — part of a LineDef after BSP splitting.
///
/// Original C: `seg_t` (r_defs.h lines 240-258).
///
/// # Pointer Translation
/// - `vertex_t* v1/v2` → `usize` (arena indices into vertex array)
/// - `side_t* sidedef` → `usize` (arena index into sidedef array)
/// - `line_t* linedef` → `usize` (arena index into linedef array)
/// - `sector_t* frontsector/backsector` → `usize` / `Option<usize>`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Seg {
    /// Arena index of the first vertex.
    pub v1: usize,
    /// Arena index of the second vertex.
    pub v2: usize,
    /// Offset along the parent LineDef (16.16 fixed-point).
    pub offset: Fixed,
    /// Direction angle of this segment (BAM).
    pub angle: Angle,
    /// Arena index of the associated SideDef.
    pub sidedef: usize,
    /// Arena index of the parent LineDef.
    pub linedef: usize,
    /// Arena index of the front sector.
    pub frontsector: usize,
    /// Arena index of the back sector (None for one-sided lines).
    pub backsector: Option<usize>,
}

/// Runtime BSP node — used for traversal during rendering and visibility checks.
///
/// Original C: `node_t` (r_defs.h lines 265-279).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Node {
    /// Partition line start X (16.16 fixed-point).
    pub x: Fixed,
    /// Partition line start Y (16.16 fixed-point).
    pub y: Fixed,
    /// Partition line delta X (16.16 fixed-point).
    pub dx: Fixed,
    /// Partition line delta Y (16.16 fixed-point).
    pub dy: Fixed,
    /// Bounding box for each child: `bbox[0]` = right, `bbox[1]` = left.
    /// Each is `[top, bottom, left, right]` in fixed-point.
    pub bbox: [[Fixed; 4]; 2],
    /// Child node indices. If `NF_SUBSECTOR` bit is set, lower bits give
    /// the subsector index; otherwise, the value is a node index.
    pub children: [u16; 2],
}

// =============================================================================
// Renderer-specific structures (r_defs.h lines 285-480)
// =============================================================================

/// A post (run of non-masked source pixels) within a column.
///
/// Posts are the fundamental building blocks of column-based sprite and patch
/// rendering. A column is a vertical strip of pixels described as a list of
/// zero or more posts, terminated by a `topdelta` value of 0xFF.
///
/// Original C: `post_t` (r_defs.h lines 285-289).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Post {
    /// Vertical offset from the top of the column. 0xFF (255) marks the
    /// end of the column's post list.
    pub topdelta: u8,
    /// Number of data bytes that follow this post header.
    pub length: u8,
}

/// A column is a list of zero or more posts, terminated by byte value 0xFF.
///
/// Original C: `typedef post_t column_t;` (r_defs.h line 292).
pub type Column = Post;

/// Light table entry type. Could be wider for >8 bit display, but DOOM uses
/// 8-bit palettized rendering with 256-entry colormaps.
///
/// Original C: `typedef byte lighttable_t;` (r_defs.h line 314).
pub type LightTable = Byte;

/// A draw segment records information about a wall segment as seen from the
/// current viewpoint, used during the rendering phase for sprite clipping.
///
/// Original C: `drawseg_t` (r_defs.h lines 322-347).
///
/// # Pointer Translation
/// - `seg_t* curline` → `usize` (arena index into seg array)
/// - `short* sprtopclip` → `Option<usize>` (offset into shared clip buffer)
/// - `short* sprbottomclip` → `Option<usize>` (offset into shared clip buffer)
/// - `short* maskedtexturecol` → `Option<usize>` (offset into shared buffer)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrawSeg {
    /// Arena index of the source seg being drawn.
    /// Original C: `seg_t* curline`.
    pub curline: usize,
    /// Left screen column (inclusive).
    pub x1: i32,
    /// Right screen column (inclusive).
    pub x2: i32,
    /// Scale at left edge (16.16 fixed-point).
    pub scale1: Fixed,
    /// Scale at right edge (16.16 fixed-point).
    pub scale2: Fixed,
    /// Scale increment per column (16.16 fixed-point).
    pub scalestep: Fixed,
    /// Silhouette type: 0=none, 1=bottom, 2=top, 3=both.
    /// See [`SIL_NONE`], [`SIL_BOTTOM`], [`SIL_TOP`], [`SIL_BOTH`].
    pub silhouette: i32,
    /// Bottom silhouette height — do not clip sprites above this.
    pub bsilheight: Fixed,
    /// Top silhouette height — do not clip sprites below this.
    pub tsilheight: Fixed,
    /// Index into the sprite top clip buffer, adjusted so that
    /// buffer\[index + x1\] gives the first value.
    /// Original C: `short* sprtopclip`.
    pub sprtopclip: Option<usize>,
    /// Index into the sprite bottom clip buffer, adjusted so that
    /// buffer\[index + x1\] gives the first value.
    /// Original C: `short* sprbottomclip`.
    pub sprbottomclip: Option<usize>,
    /// Index into the masked texture column buffer, adjusted so that
    /// buffer\[index + x1\] gives the first value.
    /// Original C: `short* maskedtexturecol`.
    pub maskedtexturecol: Option<usize>,
}

/// Patch header — a multi-column image used for sprites, UI elements, and
/// compositing textures from TEXTURE1/2 lists.
///
/// Original C: `patch_t` (r_defs.h lines 356-364).
///
/// In the C code, `columnofs[8]` is a variable-length array trick — only
/// `width` entries are actually valid, with column data following immediately
/// in memory. In Rust, we use a `Vec<i32>` to store exactly `width` column
/// offsets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Patch {
    /// Width of the patch in pixels.
    pub width: i16,
    /// Height of the patch in pixels.
    pub height: i16,
    /// Pixels to the left of the origin (used for sprite centering).
    pub leftoffset: i16,
    /// Pixels below the origin (used for sprite grounding).
    pub topoffset: i16,
    /// Column data offsets (one per pixel column). In the original C, this was
    /// `int columnofs[8]` with only `[width]` entries used.
    pub columnofs: Vec<i32>,
}

/// A visible sprite — a thing that will be drawn during the rendering refresh.
/// Represents a sprite object that is partly visible from the current viewpoint.
///
/// Original C: `vissprite_t` (r_defs.h lines 375-409).
///
/// # Pointer Translation
/// - `struct vissprite_s* prev/next` → `Option<usize>` (arena indices for
///   doubly-linked list in vissprite pool)
/// - `lighttable_t* colormap` → `Option<usize>` (index into colormap data)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VisSprite {
    /// Previous vissprite in the sorted list (arena index).
    pub prev: Option<usize>,
    /// Next vissprite in the sorted list (arena index).
    pub next: Option<usize>,
    /// Left screen column (inclusive).
    pub x1: i32,
    /// Right screen column (inclusive).
    pub x2: i32,
    /// Global X position for line side calculation (16.16 fixed-point).
    pub gx: Fixed,
    /// Global Y position for line side calculation (16.16 fixed-point).
    pub gy: Fixed,
    /// Global bottom position for silhouette clipping (16.16 fixed-point).
    pub gz: Fixed,
    /// Global top position for silhouette clipping (16.16 fixed-point).
    pub gzt: Fixed,
    /// Horizontal position of x1 in the sprite texture (16.16 fixed-point).
    pub startfrac: Fixed,
    /// Rendering scale factor (16.16 fixed-point).
    pub scale: Fixed,
    /// X inverse scale — negative if flipped (16.16 fixed-point).
    pub xiscale: Fixed,
    /// Texture mid-point for vertical positioning (16.16 fixed-point).
    pub texturemid: Fixed,
    /// Patch lump number.
    pub patch: i32,
    /// Index into colormap data for color translation, shadow draw,
    /// and maxbright frames.
    /// Original C: `lighttable_t* colormap`.
    pub colormap: Option<usize>,
    /// Flags from the source mobj (used for rendering decisions like
    /// MF_SHADOW for fuzz effect).
    pub mobjflags: i32,
}

/// Sprite frame definition — describes one animation frame of a sprite,
/// including rotation variants and horizontal flip flags.
///
/// Sprites use a special naming convention (NNNNFx or NNNNFxFx) so they
/// can be recognized by `R_InitSprites`. A sprite may have up to 8
/// rotation frames, or a single frame used for all view angles.
///
/// Original C: `spriteframe_t` (r_defs.h lines 427-440).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpriteFrame {
    /// If false, use lump\[0\] for any position (no rotation).
    /// If true, use different lumps for different view angles.
    /// Original C: `boolean rotate` (which is an enum, effectively bool).
    pub rotate: bool,
    /// Lump numbers to use for view angles 0-7.
    pub lump: [i16; 8],
    /// Flip bit (1 = flip horizontally) for view angles 0-7.
    pub flip: [u8; 8],
}

/// Sprite definition — a collection of animation frames for one sprite type.
///
/// Original C: `spritedef_t` (r_defs.h lines 448-453).
///
/// # Pointer Translation
/// - `spriteframe_t* spriteframes` → `Vec<SpriteFrame>`
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpriteDef {
    /// Number of animation frames.
    pub numframes: i32,
    /// Array of sprite frame definitions.
    /// Original C: `spriteframe_t* spriteframes`.
    pub spriteframes: Vec<SpriteFrame>,
}

/// Visplane — represents a horizontal span of floor or ceiling to be rendered.
///
/// The `top` and `bottom` arrays store per-column clip bounds. The pad bytes
/// before/after each array allow the renderer to safely access `top[minx-1]`
/// and `top[maxx+1]` without bounds-check failures, matching the original C
/// struct layout.
///
/// Original C: `visplane_t` (r_defs.h lines 460-480).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visplane {
    /// Height of this visplane (16.16 fixed-point).
    pub height: Fixed,
    /// Flat texture number.
    pub picnum: i32,
    /// Light level.
    pub lightlevel: i32,
    /// Leftmost column with data.
    pub minx: i32,
    /// Rightmost column with data.
    pub maxx: i32,
    /// Per-column top clip bounds for floor/ceiling rendering.
    /// Sized to SCREENWIDTH (320) entries.
    ///
    /// In the original C, pad bytes surround this array to allow indexing
    /// at `[minx-1]` and `[maxx+1]`. In Rust, bounds checking prevents
    /// such accesses, so the renderer must handle boundary columns explicitly.
    pub top: [u8; SCREENWIDTH as usize],
    /// Per-column bottom clip bounds for floor/ceiling rendering.
    /// Sized to SCREENWIDTH (320) entries.
    pub bottom: [u8; SCREENWIDTH as usize],
}

impl Default for Visplane {
    fn default() -> Self {
        Self {
            height: Fixed::ZERO,
            picnum: 0,
            lightlevel: 0,
            minx: 0,
            maxx: 0,
            top: [0u8; SCREENWIDTH as usize],
            bottom: [0u8; SCREENWIDTH as usize],
        }
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_lump_values() {
        assert_eq!(MapLump::Label as i32, 0);
        assert_eq!(MapLump::Things as i32, 1);
        assert_eq!(MapLump::LineDefs as i32, 2);
        assert_eq!(MapLump::SideDefs as i32, 3);
        assert_eq!(MapLump::Vertexes as i32, 4);
        assert_eq!(MapLump::Segs as i32, 5);
        assert_eq!(MapLump::SSectors as i32, 6);
        assert_eq!(MapLump::Nodes as i32, 7);
        assert_eq!(MapLump::Sectors as i32, 8);
        assert_eq!(MapLump::Reject as i32, 9);
        assert_eq!(MapLump::Blockmap as i32, 10);
    }

    #[test]
    fn test_nf_subsector_constant() {
        assert_eq!(NF_SUBSECTOR, 0x8000);
        // High bit set check
        assert_eq!(NF_SUBSECTOR & 0x8000, 0x8000);
    }

    #[test]
    fn test_silhouette_constants() {
        assert_eq!(SIL_NONE, 0);
        assert_eq!(SIL_BOTTOM, 1);
        assert_eq!(SIL_TOP, 2);
        assert_eq!(SIL_BOTH, 3);
    }

    #[test]
    fn test_maxdrawsegs() {
        assert_eq!(MAXDRAWSEGS, 256);
    }

    #[test]
    fn test_line_flags() {
        assert_eq!(LineFlags::ML_BLOCKING.bits(), 1);
        assert_eq!(LineFlags::ML_BLOCKMONSTERS.bits(), 2);
        assert_eq!(LineFlags::ML_TWOSIDED.bits(), 4);
        assert_eq!(LineFlags::ML_DONTPEGTOP.bits(), 8);
        assert_eq!(LineFlags::ML_DONTPEGBOTTOM.bits(), 16);
        assert_eq!(LineFlags::ML_SECRET.bits(), 32);
        assert_eq!(LineFlags::ML_SOUNDBLOCK.bits(), 64);
        assert_eq!(LineFlags::ML_DONTDRAW.bits(), 128);
        assert_eq!(LineFlags::ML_MAPPED.bits(), 256);

        // Test bitwise combination
        let combined = LineFlags::ML_BLOCKING | LineFlags::ML_TWOSIDED;
        assert!(combined.contains(LineFlags::ML_BLOCKING));
        assert!(combined.contains(LineFlags::ML_TWOSIDED));
        assert!(!combined.contains(LineFlags::ML_SECRET));
    }

    #[test]
    fn test_slope_type_values() {
        assert_eq!(SlopeType::Horizontal as i32, 0);
        assert_eq!(SlopeType::Vertical as i32, 1);
        assert_eq!(SlopeType::Positive as i32, 2);
        assert_eq!(SlopeType::Negative as i32, 3);
    }

    #[test]
    fn test_map_vertex_default() {
        let v = MapVertex::default();
        assert_eq!(v.x, 0);
        assert_eq!(v.y, 0);
    }

    #[test]
    fn test_map_thing_fields() {
        let thing = MapThing {
            x: 100,
            y: -200,
            angle: 90,
            type_: 1,
            options: 7,
        };
        assert_eq!(thing.x, 100);
        assert_eq!(thing.y, -200);
        assert_eq!(thing.angle, 90);
        assert_eq!(thing.type_, 1);
        assert_eq!(thing.options, 7);
    }

    #[test]
    fn test_runtime_vertex() {
        let v = Vertex {
            x: Fixed(100 << 16),
            y: Fixed(-50 << 16),
        };
        assert_eq!(v.x.0, 100 << 16);
        assert_eq!(v.y.0, -50 << 16);
    }

    #[test]
    fn test_degen_mobj_default() {
        let dm = DegenMobj::default();
        assert_eq!(dm.x, Fixed::ZERO);
        assert_eq!(dm.y, Fixed::ZERO);
        assert_eq!(dm.z, Fixed::ZERO);
    }

    #[test]
    fn test_sector_default() {
        let s = Sector::default();
        assert_eq!(s.floorheight, Fixed::ZERO);
        assert_eq!(s.ceilingheight, Fixed::ZERO);
        assert_eq!(s.floorpic, 0);
        assert_eq!(s.lightlevel, 0);
        assert!(s.soundtarget.is_none());
        assert!(s.thinglist.is_none());
        assert!(s.specialdata.is_none());
        assert_eq!(s.linecount, 0);
        assert!(s.lines.is_empty());
    }

    #[test]
    fn test_linedef_default() {
        let l = LineDef::default();
        assert_eq!(l.v1, 0);
        assert_eq!(l.v2, 0);
        assert_eq!(l.dx, Fixed::ZERO);
        assert_eq!(l.dy, Fixed::ZERO);
        assert_eq!(l.flags, 0);
        assert!(l.frontsector.is_none());
        assert!(l.backsector.is_none());
        assert!(l.specialdata.is_none());
    }

    #[test]
    fn test_seg_fields() {
        let seg = Seg {
            v1: 0,
            v2: 1,
            offset: Fixed(1024),
            angle: Angle(0x40000000),
            sidedef: 0,
            linedef: 0,
            frontsector: 0,
            backsector: Some(1),
        };
        assert_eq!(seg.v1, 0);
        assert_eq!(seg.v2, 1);
        assert_eq!(seg.offset, Fixed(1024));
        assert_eq!(seg.angle, Angle(0x40000000));
        assert_eq!(seg.backsector, Some(1));
    }

    #[test]
    fn test_node_default() {
        let n = Node::default();
        assert_eq!(n.x, Fixed::ZERO);
        assert_eq!(n.y, Fixed::ZERO);
        assert_eq!(n.children, [0u16; 2]);
    }

    #[test]
    fn test_post_default() {
        let p = Post::default();
        assert_eq!(p.topdelta, 0);
        assert_eq!(p.length, 0);
    }

    #[test]
    fn test_column_is_post() {
        // Column is a type alias for Post
        let c: Column = Post {
            topdelta: 255,
            length: 64,
        };
        assert_eq!(c.topdelta, 255);
        assert_eq!(c.length, 64);
    }

    #[test]
    fn test_drawseg_default() {
        let ds = DrawSeg::default();
        assert_eq!(ds.x1, 0);
        assert_eq!(ds.x2, 0);
        assert_eq!(ds.scale1, Fixed::ZERO);
        assert_eq!(ds.silhouette, 0);
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
        assert_eq!(vs.scale, Fixed::ZERO);
        assert!(vs.colormap.is_none());
        assert_eq!(vs.mobjflags, 0);
    }

    #[test]
    fn test_sprite_frame_default() {
        let sf = SpriteFrame::default();
        assert!(!sf.rotate);
        assert_eq!(sf.lump, [0i16; 8]);
        assert_eq!(sf.flip, [0u8; 8]);
    }

    #[test]
    fn test_sprite_def_default() {
        let sd = SpriteDef::default();
        assert_eq!(sd.numframes, 0);
        assert!(sd.spriteframes.is_empty());
    }

    #[test]
    fn test_visplane_default() {
        let vp = Visplane::default();
        assert_eq!(vp.height, Fixed::ZERO);
        assert_eq!(vp.picnum, 0);
        assert_eq!(vp.top.len(), 320);
        assert_eq!(vp.bottom.len(), 320);
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
    fn test_map_node_bbox_layout() {
        let node = MapNode {
            x: 0,
            y: 0,
            dx: 64,
            dy: 0,
            bbox: [[100, -100, -50, 50], [200, 0, 0, 100]],
            children: [0, NF_SUBSECTOR | 5],
        };
        assert_eq!(node.bbox[0][0], 100); // right child top
        assert_eq!(node.bbox[1][0], 200); // left child top
                                          // Second child is a subsector (index 5)
        assert_ne!(node.children[1] & NF_SUBSECTOR, 0);
        assert_eq!(node.children[1] & !NF_SUBSECTOR, 5);
    }

    #[test]
    fn test_subsector_default() {
        let ss = Subsector::default();
        assert_eq!(ss.sector, 0);
        assert_eq!(ss.numlines, 0);
        assert_eq!(ss.firstline, 0);
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
    fn test_map_linedef_sidenum() {
        let ld = MapLineDef {
            v1: 0,
            v2: 1,
            flags: 4, // ML_TWOSIDED
            special: 0,
            tag: 0,
            sidenum: [0, -1], // one-sided from the back
        };
        assert_eq!(ld.sidenum[0], 0);
        assert_eq!(ld.sidenum[1], -1);
    }

    #[test]
    fn test_lighttable_is_byte() {
        // LightTable is a type alias for Byte (u8)
        let lt: LightTable = 128;
        assert_eq!(lt, 128u8);
    }
}
