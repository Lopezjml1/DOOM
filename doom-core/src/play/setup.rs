//! Map loading and level setup — translated from linuxdoom-1.10/p_setup.c
//!
//! Loads map geometry from WAD lumps (THINGS, LINEDEFS, SIDEDEFS, VERTEXES,
//! SEGS, SSECTORS, NODES, SECTORS, REJECT, BLOCKMAP), builds cross-references,
//! and initializes the play simulation for a new level.

// Copyright (C) 1993-1996 by id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

use byteorder::{ByteOrder, LittleEndian};
use tracing::{debug, error, info, warn};

use crate::info::sprites::SPRITE_NAMES;
use doom_wad::PurgeTag;

use crate::traits::wad::WadProvider;
use crate::types::angle::Angle;
use crate::types::doomdef::{GameMode, Skill, MAXPLAYERS};
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::map_data::{
    LineDef, LineFlags, MapLump, MapThing, Node, Sector, Seg, SideDef, SlopeType, Subsector,
    Vertex, NF_SUBSECTOR,
};
use crate::util::bbox::{add_to_box, clear_box, BBox, BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of deathmatch spawn points (from p_setup.c line 109).
pub const MAX_DEATHMATCH_STARTS: usize = 10;

/// Blockmap unit shift: FRACBITS + 7 = 23.
/// Each blockmap cell is 128 map units (1 << 7) wide, in fixed-point that is
/// 128 << 16 = 1 << 23.
const MAPBLOCKSHIFT: i32 = FRACBITS + 7;

/// Maximum radius used for sector blockbox padding (32 map units in fixed-point).
/// Equivalent to `32 * FRACUNIT` = `32 * 65536` = 2_097_152.
const MAXRADIUS: Fixed = Fixed(32 * FRACUNIT);

/// DOOM II monster type numbers that must be filtered in non-commercial mode.
/// If game mode is not `Commercial`, things with these DoomEd numbers are
/// skipped during `load_things`.
const DOOM2_MONSTER_TYPES: [i16; 10] = [
    68, // Arachnotron
    64, // Archvile
    88, // Boss Brain
    89, // Boss Shooter
    69, // Hell Knight
    67, // Mancubus
    71, // Pain Elemental
    65, // Former Human Commando (Heavy Weapon Dude)
    66, // Revenant
    84, // Wolf SS
];

// ---------------------------------------------------------------------------
// LevelData — owns all loaded map geometry
// ---------------------------------------------------------------------------

/// Container for all map data loaded from WAD lumps.
///
/// In the original C source these were global arrays and counters
/// (`numvertexes`, `vertexes`, `numsegs`, `segs`, etc.).  In the Rust port
/// each array is a `Vec` owned by this struct, and cross-references between
/// arrays use `usize` indices instead of raw pointers.
pub struct LevelData {
    // ---- Geometry ----
    /// Runtime vertex positions (Fixed-point).
    pub vertexes: Vec<Vertex>,
    /// Wall segments referencing vertices and linedefs.
    pub segs: Vec<Seg>,
    /// Sector definitions (heights, lighting, specials).
    pub sectors: Vec<Sector>,
    /// BSP leaf nodes referencing contiguous seg runs.
    pub subsectors: Vec<Subsector>,
    /// BSP internal nodes for spatial partitioning.
    pub nodes: Vec<Node>,
    /// Line definitions connecting two vertices with optional two-sidedness.
    pub lines: Vec<LineDef>,
    /// Side definitions (textures, sector reference).
    pub sides: Vec<SideDef>,

    // ---- Blockmap ----
    /// Raw blockmap lump data stored as `i16` words.
    /// Layout: `[orgx, orgy, width, height, offsets..., block lists...]`.
    pub blockmap_lump: Vec<i16>,
    /// Index into `blockmap_lump` where per-block offset table begins (always 4).
    pub blockmap_offset: usize,
    /// Blockmap origin X in fixed-point.
    pub bmap_orgx: Fixed,
    /// Blockmap origin Y in fixed-point.
    pub bmap_orgy: Fixed,
    /// Blockmap width in blocks.
    pub bmap_width: i32,
    /// Blockmap height in blocks.
    pub bmap_height: i32,
    /// Per-block linked-list head for map objects (mobj arena indices).
    /// Initialised to `None` (empty chains) on level load.
    pub block_links: Vec<Option<usize>>,

    // ---- Reject table ----
    /// Raw REJECT lump bytes used for fast line-of-sight rejection.
    pub reject_matrix: Vec<u8>,

    // ---- Spawn points ----
    /// Deathmatch spawn locations (up to [`MAX_DEATHMATCH_STARTS`]).
    pub deathmatch_starts: Vec<MapThing>,
    /// Player start positions, indexed by player number (0..MAXPLAYERS).
    pub player_starts: [Option<MapThing>; 4],
}

impl LevelData {
    /// Create a new, empty `LevelData`.  All vectors are empty and the
    /// blockmap fields are zeroed.  Call the `load_*` helpers followed by
    /// [`setup_level`] to populate.
    pub fn new() -> Self {
        Self {
            vertexes: Vec::new(),
            segs: Vec::new(),
            sectors: Vec::new(),
            subsectors: Vec::new(),
            nodes: Vec::new(),
            lines: Vec::new(),
            sides: Vec::new(),
            blockmap_lump: Vec::new(),
            blockmap_offset: 0,
            bmap_orgx: Fixed::ZERO,
            bmap_orgy: Fixed::ZERO,
            bmap_width: 0,
            bmap_height: 0,
            block_links: Vec::new(),
            reject_matrix: Vec::new(),
            deathmatch_starts: Vec::new(),
            player_starts: [None; 4],
        }
    }
}

impl Default for LevelData {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// WAD lump loading helpers
// ---------------------------------------------------------------------------

/// Extract a NUL-padded 8-byte name from a WAD lump record and return it as
/// an upper-case `String`.  Bytes after the first NUL are ignored.
fn read_lump_name(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).to_uppercase()
}

// ---------------------------------------------------------------------------
// P_LoadVertexes  (p_setup.c lines 122-152)
// ---------------------------------------------------------------------------

/// Load vertex data from the VERTEXES WAD lump.
///
/// Each vertex record is 4 bytes: two little-endian `i16` values (x, y) that
/// are promoted to 16.16 fixed-point by left-shifting by [`FRACBITS`].
fn load_vertexes(wad: &dyn WadProvider, lump: usize) -> Vec<Vertex> {
    // Use lump_length for count calculation (faithful to original C pattern:
    // numvertexes = W_LumpLength(lump) / sizeof(mapvertex_t)).
    let expected_size = wad.lump_length(lump);
    let data = wad.read_lump(lump);
    debug_assert_eq!(data.len(), expected_size, "vertex lump size mismatch");
    let count = data.len() / 4;
    let mut verts = Vec::with_capacity(count);

    for i in 0..count {
        let off = i * 4;
        let x = LittleEndian::read_i16(&data[off..]) as i32;
        let y = LittleEndian::read_i16(&data[off + 2..]) as i32;
        // from_int converts map-unit integers to 16.16 fixed-point.
        verts.push(Vertex {
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
        });
    }

    debug!("P_LoadVertexes: loaded {} vertexes", count);
    verts
}

// ---------------------------------------------------------------------------
// P_LoadSegs  (p_setup.c lines 159-196)
// ---------------------------------------------------------------------------

/// Load segment data from the SEGS WAD lump.
///
/// Each seg record is 12 bytes. Cross-references to vertices, linedefs and
/// sidedefs are stored as `usize` indices.
fn load_segs(wad: &dyn WadProvider, lump: usize, lines: &[LineDef], sides: &[SideDef]) -> Vec<Seg> {
    let data = wad.read_lump(lump);
    let count = data.len() / 12;
    let mut segs = Vec::with_capacity(count);

    for i in 0..count {
        let off = i * 12;
        let v1 = LittleEndian::read_i16(&data[off..]) as usize;
        let v2 = LittleEndian::read_i16(&data[off + 2..]) as usize;
        let angle_raw = LittleEndian::read_i16(&data[off + 4..]) as u16;
        let linedef_idx = LittleEndian::read_i16(&data[off + 6..]) as usize;
        let side = LittleEndian::read_i16(&data[off + 8..]) as usize;
        let offset_raw = LittleEndian::read_i16(&data[off + 10..]) as i32;

        let ld = &lines[linedef_idx];
        let sidedef_idx = ld.sidenum[side] as usize;
        let frontsector = sides[sidedef_idx].sector;

        // Check two-sided flag using raw bitwise operation on the i16 flags
        // field, matching the original C: `linedef->flags & ML_TWOSIDED`.
        let backsector = if (ld.flags & LineFlags::ML_TWOSIDED.bits()) != 0 {
            let back_side = ld.sidenum[side ^ 1];
            if back_side >= 0 {
                Some(sides[back_side as usize].sector)
            } else {
                None
            }
        } else {
            None
        };

        segs.push(Seg {
            v1,
            v2,
            offset: Fixed::new(offset_raw << FRACBITS),
            angle: Angle((angle_raw as u32) << 16),
            sidedef: sidedef_idx,
            linedef: linedef_idx,
            frontsector,
            backsector,
        });
    }

    debug!("P_LoadSegs: loaded {} segs", count);
    segs
}

// ---------------------------------------------------------------------------
// P_LoadSubsectors  (p_setup.c lines 202-224)
// ---------------------------------------------------------------------------

/// Load subsector data from the SSECTORS WAD lump.
///
/// Each subsector record is 4 bytes: `numsegs` (i16) and `firstseg` (i16).
fn load_subsectors(wad: &dyn WadProvider, lump: usize) -> Vec<Subsector> {
    let data = wad.read_lump(lump);
    let count = data.len() / 4;
    let mut subs = Vec::with_capacity(count);

    for i in 0..count {
        let off = i * 4;
        let numlines = LittleEndian::read_i16(&data[off..]);
        let firstline = LittleEndian::read_i16(&data[off + 2..]);
        subs.push(Subsector {
            sector: 0, // resolved later in p_group_lines
            numlines,
            firstline,
        });
    }

    debug!("P_LoadSubsectors: loaded {} subsectors", count);
    subs
}

// ---------------------------------------------------------------------------
// P_LoadSectors  (p_setup.c lines 231-258)
// ---------------------------------------------------------------------------

/// Load sector data from the SECTORS WAD lump.
///
/// Each sector record is 26 bytes. Heights are converted from map units (i16)
/// to 16.16 fixed-point.  Floor and ceiling texture names are stored as raw
/// indices to be resolved by the renderer later (flat lookup is deferred).
fn load_sectors(wad: &dyn WadProvider, lump: usize) -> Vec<Sector> {
    let data = wad.read_lump(lump);
    let record_size = 26;
    let count = data.len() / record_size;
    let mut sectors = Vec::with_capacity(count);

    for i in 0..count {
        let off = i * record_size;
        let floorheight = LittleEndian::read_i16(&data[off..]) as i32;
        let ceilingheight = LittleEndian::read_i16(&data[off + 2..]) as i32;

        // Floor and ceiling flat names (8 bytes each) — stored as raw name
        // strings. In the original C these were passed to R_FlatNumForName;
        // here we store index 0 as a placeholder to be resolved by the
        // renderer during R_PrecacheLevel.
        let _floor_name = read_lump_name(&data[off + 4..off + 12]);
        let _ceil_name = read_lump_name(&data[off + 12..off + 20]);

        let lightlevel = LittleEndian::read_i16(&data[off + 20..]);
        let special = LittleEndian::read_i16(&data[off + 22..]);
        let tag = LittleEndian::read_i16(&data[off + 24..]);

        // Construct sector with parsed values; remaining fields default.
        // floorpic / ceilingpic will be resolved by the renderer via
        // R_FlatNumForName when it initialises flat lookups.
        sectors.push(Sector {
            floorheight: Fixed::new(floorheight << FRACBITS),
            ceilingheight: Fixed::new(ceilingheight << FRACBITS),
            floorpic: 0,
            ceilingpic: 0,
            lightlevel,
            special,
            tag,
            thinglist: None,
            specialdata: None,
            soundtarget: None,
            ..Sector::default()
        });
    }

    debug!("P_LoadSectors: loaded {} sectors", count);
    sectors
}

// ---------------------------------------------------------------------------
// P_LoadNodes  (p_setup.c lines 264-295)
// ---------------------------------------------------------------------------

/// Load BSP node data from the NODES WAD lump.
///
/// Each node record is 28 bytes. Partition line coordinates and child bounding
/// boxes are converted to 16.16 fixed-point.  Child references are stored as
/// raw `u16` values where the high bit ([`NF_SUBSECTOR`]) marks a leaf
/// (subsector index).
fn load_nodes(wad: &dyn WadProvider, lump: usize) -> Vec<Node> {
    let data = wad.read_lump(lump);
    let record_size = 28;
    let count = data.len() / record_size;
    let mut nodes = Vec::with_capacity(count);

    for i in 0..count {
        let base = i * record_size;
        let x = LittleEndian::read_i16(&data[base..]) as i32;
        let y = LittleEndian::read_i16(&data[base + 2..]) as i32;
        let dx = LittleEndian::read_i16(&data[base + 4..]) as i32;
        let dy = LittleEndian::read_i16(&data[base + 6..]) as i32;

        // Two child bounding boxes (right=0, left=1), each with 4 values
        // (top, bottom, left, right) as i16.
        let mut bbox = [[Fixed::ZERO; 4]; 2];
        for (child_idx, child_bbox) in bbox.iter_mut().enumerate() {
            for (coord_idx, coord_val) in child_bbox.iter_mut().enumerate() {
                let off = base + 8 + child_idx * 8 + coord_idx * 2;
                let val = LittleEndian::read_i16(&data[off..]) as i32;
                *coord_val = Fixed::new(val << FRACBITS);
            }
        }

        // Child references: high bit = NF_SUBSECTOR flag.
        // When NF_SUBSECTOR is set, the lower bits are a subsector index;
        // otherwise they are a child node index.
        let children_off = base + 24;
        let child0 = LittleEndian::read_u16(&data[children_off..]);
        let child1 = LittleEndian::read_u16(&data[children_off + 2..]);

        // Validate subsector references for diagnostics.
        let _c0_is_leaf = (child0 & NF_SUBSECTOR) != 0;
        let _c1_is_leaf = (child1 & NF_SUBSECTOR) != 0;

        nodes.push(Node {
            x: Fixed::new(x << FRACBITS),
            y: Fixed::new(y << FRACBITS),
            dx: Fixed::new(dx << FRACBITS),
            dy: Fixed::new(dy << FRACBITS),
            bbox,
            children: [child0, child1],
        });
    }

    debug!("P_LoadNodes: loaded {} nodes", count);
    nodes
}

// ---------------------------------------------------------------------------
// P_LoadThings  (p_setup.c lines 301-350)
// ---------------------------------------------------------------------------

/// Load thing spawn records from the THINGS WAD lump.
///
/// Returns a vector of [`MapThing`] entries that should later be passed to
/// `P_SpawnMapThing`.  Deathmatch start points (type 11) and player start
/// points (types 1–4) are extracted into the provided `LevelData`.
///
/// # IMPORTANT — Preserved Bug (demo compatibility)
///
/// The original C code at line 337 uses `break` instead of `continue` when
/// filtering DOOM II–only monsters in non-commercial game modes.  This means
/// the loader **stops processing all remaining things** after the first
/// filtered monster.  This behaviour is intentionally preserved for demo
/// compatibility.
fn load_things(
    wad: &dyn WadProvider,
    lump: usize,
    game_mode: GameMode,
    level_data: &mut LevelData,
) -> Vec<MapThing> {
    let data = wad.read_lump(lump);
    let record_size = 10;
    let num_things = data.len() / record_size;
    let mut spawnable = Vec::with_capacity(num_things);

    for i in 0..num_things {
        let off = i * record_size;
        let x = LittleEndian::read_i16(&data[off..]);
        let y = LittleEndian::read_i16(&data[off + 2..]);
        let angle = LittleEndian::read_i16(&data[off + 4..]);
        let type_ = LittleEndian::read_i16(&data[off + 6..]);
        let options = LittleEndian::read_i16(&data[off + 8..]);

        let mt = MapThing {
            x,
            y,
            angle,
            type_,
            options,
        };

        // ---- DOOM II monster filter ----
        // In non-commercial modes, DOOM II–exclusive monsters must be
        // filtered out.  The original C code uses `break` here instead of
        // `continue`, which is a bug that we MUST preserve for demo
        // compatibility — it stops loading ALL remaining things after the
        // first filtered monster.
        if game_mode != GameMode::Commercial {
            let mut spawn = true;
            for &doom2_type in &DOOM2_MONSTER_TYPES {
                if type_ == doom2_type {
                    spawn = false;
                    break;
                }
            }
            if !spawn {
                // BUG PRESERVED: original uses `break` not `continue`.
                // This halts processing of ALL remaining things.
                break;
            }
        }

        // Categorise the thing.
        if type_ == 11 {
            // Deathmatch start.
            if level_data.deathmatch_starts.len() < MAX_DEATHMATCH_STARTS {
                level_data.deathmatch_starts.push(mt);
            } else {
                warn!(
                    "P_LoadThings: too many deathmatch starts (max {})",
                    MAX_DEATHMATCH_STARTS
                );
            }
        } else if (1..=4).contains(&type_) {
            // Player start (players 1–4, zero-indexed).
            let idx = (type_ - 1) as usize;
            if idx < MAXPLAYERS {
                level_data.player_starts[idx] = Some(mt);
            }
        }

        // All things (including DM/player starts) are returned for
        // P_SpawnMapThing processing by the caller.
        spawnable.push(mt);
    }

    debug!(
        "P_LoadThings: processed {}/{} things ({} DM starts, {} player starts)",
        spawnable.len(),
        num_things,
        level_data.deathmatch_starts.len(),
        level_data
            .player_starts
            .iter()
            .filter(|s| s.is_some())
            .count()
    );
    spawnable
}

// ---------------------------------------------------------------------------
// P_LoadLineDefs  (p_setup.c lines 357-432)
// ---------------------------------------------------------------------------

/// Load linedef data from the LINEDEFS WAD lump.
///
/// Each record is 14 bytes.  Slope type is computed from `dx`/`dy` using
/// fixed-point division, matching the original classification logic exactly.
fn load_linedefs(
    wad: &dyn WadProvider,
    lump: usize,
    vertexes: &[Vertex],
    sides: &[SideDef],
) -> Vec<LineDef> {
    let data = wad.read_lump(lump);
    let record_size = 14;
    let count = data.len() / record_size;
    let mut lines = Vec::with_capacity(count);

    for i in 0..count {
        let off = i * record_size;
        let v1_idx = LittleEndian::read_i16(&data[off..]) as usize;
        let v2_idx = LittleEndian::read_i16(&data[off + 2..]) as usize;
        let flags_raw = LittleEndian::read_i16(&data[off + 4..]);
        let special = LittleEndian::read_i16(&data[off + 6..]);
        let tag = LittleEndian::read_i16(&data[off + 8..]);
        let sidenum0 = LittleEndian::read_i16(&data[off + 10..]);
        let sidenum1 = LittleEndian::read_i16(&data[off + 12..]);

        let v1 = &vertexes[v1_idx];
        let v2 = &vertexes[v2_idx];

        let dx = Fixed::new(v2.x.raw() - v1.x.raw());
        let dy = Fixed::new(v2.y.raw() - v1.y.raw());

        // Compute slope type (p_setup.c lines 404-416).
        let slopetype = if dx.raw() == 0 {
            SlopeType::Vertical
        } else if dy.raw() == 0 {
            SlopeType::Horizontal
        } else {
            let slope = dy.fixed_div(dx);
            if slope.raw() > 0 {
                SlopeType::Positive
            } else {
                SlopeType::Negative
            }
        };

        // Compute bounding box from vertex coordinates.
        let mut bbox: BBox = [Fixed::ZERO; 4];
        if v1.x.raw() < v2.x.raw() {
            bbox[BOXLEFT] = v1.x;
            bbox[BOXRIGHT] = v2.x;
        } else {
            bbox[BOXLEFT] = v2.x;
            bbox[BOXRIGHT] = v1.x;
        }
        if v1.y.raw() < v2.y.raw() {
            bbox[BOXBOTTOM] = v1.y;
            bbox[BOXTOP] = v2.y;
        } else {
            bbox[BOXBOTTOM] = v2.y;
            bbox[BOXTOP] = v1.y;
        }

        // Resolve front and back sectors from sidenum indices.
        let frontsector = if sidenum0 >= 0 {
            Some(sides[sidenum0 as usize].sector)
        } else {
            None
        };
        let backsector = if sidenum1 >= 0 {
            Some(sides[sidenum1 as usize].sector)
        } else {
            None
        };

        lines.push(LineDef {
            v1: v1_idx,
            v2: v2_idx,
            dx,
            dy,
            flags: flags_raw,
            special,
            tag,
            sidenum: [sidenum0, sidenum1],
            bbox,
            slopetype,
            frontsector,
            backsector,
            validcount: 0,
            specialdata: None,
        });
    }

    debug!("P_LoadLineDefs: loaded {} linedefs", count);
    lines
}

// ---------------------------------------------------------------------------
// P_LoadSideDefs  (p_setup.c lines 438-463)
// ---------------------------------------------------------------------------

/// Load sidedef data from the SIDEDEFS WAD lump.
///
/// Each record is 30 bytes.  Texture names are stored as raw indices to be
/// resolved by the renderer (R_TextureNumForName); during loading we store
/// zero as a placeholder value.
fn load_sidedefs(wad: &dyn WadProvider, lump: usize) -> Vec<SideDef> {
    let data = wad.read_lump(lump);
    let record_size = 30;
    let count = data.len() / record_size;
    let mut sides = Vec::with_capacity(count);

    for i in 0..count {
        let off = i * record_size;
        let textureoffset = LittleEndian::read_i16(&data[off..]) as i32;
        let rowoffset = LittleEndian::read_i16(&data[off + 2..]) as i32;

        // Texture names (8 bytes each) — stored temporarily as names.
        // In the original C these were passed to R_TextureNumForName;
        // here they will be resolved when the renderer initialises.
        let _top_name = read_lump_name(&data[off + 4..off + 12]);
        let _bottom_name = read_lump_name(&data[off + 12..off + 20]);
        let _mid_name = read_lump_name(&data[off + 20..off + 28]);

        let sector_idx = LittleEndian::read_i16(&data[off + 28..]) as usize;

        sides.push(SideDef {
            textureoffset: Fixed::new(textureoffset << FRACBITS),
            rowoffset: Fixed::new(rowoffset << FRACBITS),
            toptexture: 0,    // resolved by renderer
            bottomtexture: 0, // resolved by renderer
            midtexture: 0,    // resolved by renderer
            sector: sector_idx,
        });
    }

    debug!("P_LoadSideDefs: loaded {} sidedefs", count);
    sides
}

// ---------------------------------------------------------------------------
// P_LoadBlockMap  (p_setup.c lines 469-490)
// ---------------------------------------------------------------------------

/// Load and parse the BLOCKMAP WAD lump into `LevelData` blockmap fields.
///
/// The lump begins with a 4-word header (orgx, orgy, width, height) followed
/// by per-block offset words and terminator-separated block lists.
///
/// Uses [`WadProvider::cache_lump_num`] to cache the blockmap data with
/// [`PurgeTag::Level`], matching the original C behaviour of
/// `W_CacheLumpNum(lump+ML_BLOCKMAP, PU_LEVEL)` (p_setup.c line 641).
fn load_blockmap(wad: &mut dyn WadProvider, lump: usize, level: &mut LevelData) {
    let raw = wad.cache_lump_num(lump, PurgeTag::Level);
    let word_count = raw.len() / 2;
    let mut blockmap_lump = Vec::with_capacity(word_count);

    for i in 0..word_count {
        let val = LittleEndian::read_i16(&raw[i * 2..]);
        blockmap_lump.push(val);
    }

    if blockmap_lump.len() < 4 {
        error!(
            "P_LoadBlockMap: blockmap lump too small ({} words)",
            blockmap_lump.len()
        );
        return;
    }

    level.bmap_orgx = Fixed::new((blockmap_lump[0] as i32) << FRACBITS);
    level.bmap_orgy = Fixed::new((blockmap_lump[1] as i32) << FRACBITS);
    level.bmap_width = blockmap_lump[2] as i32;
    level.bmap_height = blockmap_lump[3] as i32;

    // Offset table starts after the 4-word header.
    level.blockmap_offset = 4;

    // Allocate block links (mobj chain heads, one per blockmap cell).
    let num_blocks = (level.bmap_width as usize) * (level.bmap_height as usize);
    level.block_links = vec![None; num_blocks];

    level.blockmap_lump = blockmap_lump;

    debug!(
        "P_LoadBlockMap: {}x{} blocks, origin ({}, {})",
        level.bmap_width,
        level.bmap_height,
        level.bmap_orgx.raw() >> FRACBITS,
        level.bmap_orgy.raw() >> FRACBITS
    );
}

// ---------------------------------------------------------------------------
// P_LoadReject  (implicit in p_setup.c P_SetupLevel around line 664)
// ---------------------------------------------------------------------------

/// Load the REJECT table from the WAD lump.
///
/// The reject table is a bit matrix used for fast line-of-sight rejection
/// between sectors.  It is stored as raw bytes.
fn load_reject(wad: &dyn WadProvider, lump: usize) -> Vec<u8> {
    let data = wad.read_lump(lump);
    debug!("P_LoadReject: loaded {} bytes", data.len());
    data
}

// ---------------------------------------------------------------------------
// P_GroupLines  (p_setup.c lines 499-577)
// ---------------------------------------------------------------------------

/// Build cross-references between sectors, subsectors, linedefs, and the
/// blockmap.
///
/// This function:
/// 1. Resolves each subsector's `sector` field by looking up
///    `segs[subsector.firstline].frontsector`.
/// 2. Counts lines touching each sector (front and back).
/// 3. Builds `sector.lines` (indices into `lines`).
/// 4. Computes each sector's bounding box and `soundorg` (center of bbox).
/// 5. Computes each sector's `blockbox` relative to the blockmap grid,
///    padded by [`MAXRADIUS`].
fn p_group_lines(level: &mut LevelData) {
    // ---- Step 1: resolve subsector → sector mapping ----
    for i in 0..level.subsectors.len() {
        let first_seg = level.subsectors[i].firstline as usize;
        if first_seg < level.segs.len() {
            let seg = &level.segs[first_seg];
            level.subsectors[i].sector = seg.frontsector;
        } else {
            warn!(
                "P_GroupLines: subsector {} firstline {} out of range (num segs = {})",
                i,
                first_seg,
                level.segs.len()
            );
        }
    }

    // ---- Step 2: count lines per sector ----
    let num_sectors = level.sectors.len();
    let mut line_counts = vec![0usize; num_sectors];

    for line in &level.lines {
        if let Some(fs) = line.frontsector {
            if fs < num_sectors {
                line_counts[fs] += 1;
            }
        }
        if let Some(bs) = line.backsector {
            if bs < num_sectors {
                line_counts[bs] += 1;
            }
        }
    }

    // ---- Step 3: allocate and populate sector.lines ----
    // Pre-allocate per-sector line lists.
    for (si, sector) in level.sectors.iter_mut().enumerate() {
        sector.lines = Vec::with_capacity(line_counts[si]);
        sector.linecount = line_counts[si] as i32;
    }

    for (li, line) in level.lines.iter().enumerate() {
        if let Some(fs) = line.frontsector {
            if fs < num_sectors {
                level.sectors[fs].lines.push(li);
            }
        }
        if let Some(bs) = line.backsector {
            if bs < num_sectors {
                level.sectors[bs].lines.push(li);
            }
        }
    }

    // ---- Step 4: compute sector bounding boxes and sound origins ----
    for si in 0..num_sectors {
        let mut bbox: BBox = [Fixed::ZERO; 4];
        clear_box(&mut bbox);

        // Gather vertex indices from sector's lines (clone to avoid borrow).
        let line_indices: Vec<usize> = level.sectors[si].lines.clone();
        for &li in &line_indices {
            let line = &level.lines[li];
            let v1 = &level.vertexes[line.v1];
            let v2 = &level.vertexes[line.v2];
            add_to_box(&mut bbox, v1.x, v1.y);
            add_to_box(&mut bbox, v2.x, v2.y);
        }

        // Sound origin is the center of the bounding box.
        // Update x/y in place; thinker and z keep their default values.
        level.sectors[si].soundorg.x = Fixed::new((bbox[BOXLEFT].raw() + bbox[BOXRIGHT].raw()) / 2);
        level.sectors[si].soundorg.y = Fixed::new((bbox[BOXTOP].raw() + bbox[BOXBOTTOM].raw()) / 2);

        // ---- Step 5: compute sector blockbox ----
        // Block coordinates are (map_coord - blockmap_origin) >> MAPBLOCKSHIFT.
        // Padded outward by MAXRADIUS on each side.
        let block_top =
            ((bbox[BOXTOP].raw() - level.bmap_orgy.raw()) + MAXRADIUS.raw()) >> MAPBLOCKSHIFT;
        let block_bottom =
            ((bbox[BOXBOTTOM].raw() - level.bmap_orgy.raw()) - MAXRADIUS.raw()) >> MAPBLOCKSHIFT;
        let block_right =
            ((bbox[BOXRIGHT].raw() - level.bmap_orgx.raw()) + MAXRADIUS.raw()) >> MAPBLOCKSHIFT;
        let block_left =
            ((bbox[BOXLEFT].raw() - level.bmap_orgx.raw()) - MAXRADIUS.raw()) >> MAPBLOCKSHIFT;

        // Clamp to valid blockmap range.
        let bw = level.bmap_width;
        let bh = level.bmap_height;

        level.sectors[si].blockbox[BOXTOP] = if block_top >= bh { bh - 1 } else { block_top };
        level.sectors[si].blockbox[BOXBOTTOM] = if block_bottom < 0 { 0 } else { block_bottom };
        level.sectors[si].blockbox[BOXRIGHT] = if block_right >= bw {
            bw - 1
        } else {
            block_right
        };
        level.sectors[si].blockbox[BOXLEFT] = if block_left < 0 { 0 } else { block_left };
    }

    debug!(
        "P_GroupLines: grouped {} lines into {} sectors",
        level.lines.len(),
        num_sectors
    );
}

// ---------------------------------------------------------------------------
// P_SetupLevel  (p_setup.c lines 583-693)
// ---------------------------------------------------------------------------

/// Load a complete map from the WAD file system and return the populated
/// [`LevelData`] along with a vector of [`MapThing`] records that the caller
/// must pass to `P_SpawnMapThing` for object instantiation.
///
/// # Arguments
///
/// * `episode` — Episode number (1-based, ignored for DOOM II commercial).
/// * `map` — Map number (1-based).
/// * `skill` — Difficulty setting.
/// * `game_mode` — Determines lump naming convention and monster filtering.
/// * `wad` — WAD file provider for lump access.
///
/// # Returns
///
/// A tuple of `(LevelData, Vec<MapThing>)`.  The caller is responsible for
/// iterating the `MapThing` vector and calling `P_SpawnMapThing` on each
/// entry, as well as invoking `P_InitThinkers` before loading and
/// `P_SpawnSpecials` + `R_PrecacheLevel` after loading.
pub fn setup_level(
    episode: i32,
    map: i32,
    _playermask: i32,
    skill: Skill,
    game_mode: GameMode,
    wad: &mut dyn WadProvider,
) -> (LevelData, Vec<MapThing>) {
    info!(
        "P_SetupLevel: loading E{}M{} / MAP{:02} (skill {:?})",
        episode, map, map, skill
    );

    let mut level = LevelData::new();

    // ---- Build lump name ----
    // Commercial DOOM (DOOM II, TNT, Plutonia): "MAPxx"
    // Shareware / Registered / Retail DOOM: "ExMy"
    let lump_name = if game_mode == GameMode::Commercial {
        format!("MAP{:02}", map)
    } else {
        format!("E{}M{}", episode, map)
    };

    let lump_num = match wad.get_num_for_name(&lump_name) {
        Ok(n) => n,
        Err(e) => {
            error!("P_SetupLevel: cannot find map lump '{}': {}", lump_name, e);
            return (level, Vec::new());
        }
    };

    debug!("P_SetupLevel: base lump '{}' = {}", lump_name, lump_num);

    // Cache the map label lump itself (the marker lump at the base offset).
    // In the original C engine, lumps accessed via W_CacheLumpName stay in the
    // lump cache with tag PU_LEVEL so they are freed on the next level load.
    // The label lump is typically zero-length but caching it validates that the
    // WAD provider has the lump available and warms the cache for subsequent
    // offset-based accesses.
    let _label_data = wad.cache_lump_name(&lump_name, PurgeTag::Level);

    // ---- Load map lumps in the correct order (p_setup.c lines 641-665) ----
    // ORDER MATTERS — sidedefs reference sectors, linedefs reference sidedefs
    // and vertexes, segs reference linedefs and sides, etc.

    // 1. Blockmap (ML_BLOCKMAP)
    load_blockmap(wad, lump_num + MapLump::Blockmap as usize, &mut level);

    // 2. Vertexes (ML_VERTEXES)
    level.vertexes = load_vertexes(wad, lump_num + MapLump::Vertexes as usize);

    // 3. Sectors (ML_SECTORS)
    level.sectors = load_sectors(wad, lump_num + MapLump::Sectors as usize);

    // 4. Sidedefs (ML_SIDEDEFS)
    level.sides = load_sidedefs(wad, lump_num + MapLump::SideDefs as usize);

    // 5. Linedefs (ML_LINEDEFS)
    level.lines = load_linedefs(
        wad,
        lump_num + MapLump::LineDefs as usize,
        &level.vertexes,
        &level.sides,
    );

    // 6. Subsectors (ML_SSECTORS)
    level.subsectors = load_subsectors(wad, lump_num + MapLump::SSectors as usize);

    // 7. Nodes (ML_NODES)
    level.nodes = load_nodes(wad, lump_num + MapLump::Nodes as usize);

    // 8. Segs (ML_SEGS)
    level.segs = load_segs(
        wad,
        lump_num + MapLump::Segs as usize,
        &level.lines,
        &level.sides,
    );

    // 9. Reject table (ML_REJECT)
    level.reject_matrix = load_reject(wad, lump_num + MapLump::Reject as usize);

    // 10. Group lines into sectors and build sector bounding boxes.
    p_group_lines(&mut level);

    // 11. Things (ML_THINGS)
    let things = load_things(
        wad,
        lump_num + MapLump::Things as usize,
        game_mode,
        &mut level,
    );

    info!(
        "P_SetupLevel: loaded {} verts, {} segs, {} sectors, {} subsectors, \
         {} nodes, {} lines, {} sides",
        level.vertexes.len(),
        level.segs.len(),
        level.sectors.len(),
        level.subsectors.len(),
        level.nodes.len(),
        level.lines.len(),
        level.sides.len(),
    );

    (level, things)
}

// ---------------------------------------------------------------------------
// P_Init  (p_setup.c lines 700-705)
// ---------------------------------------------------------------------------

/// One-time play subsystem initialisation.
///
/// In the original C code this called:
/// - `P_InitSwitchList()` — initialise switch texture pairs
/// - `P_InitPicAnims()` — initialise animated flat/texture definitions
/// - `R_InitSprites(sprnames)` — initialise sprite frame lookups
///
/// Because those subsystems live in separate modules (`play::spec`,
/// `play::spec`, and `doom-render-soft`) that are not direct dependencies of
/// this module, `p_init` returns the sprite name table so that the
/// orchestrating code can call each initialiser in the correct order.
///
/// # Returns
///
/// A static reference to the [`SPRITE_NAMES`] table that should be passed to
/// `R_InitSprites`.
pub fn p_init() -> &'static [&'static str] {
    info!("P_Init: play subsystem initialising");
    &SPRITE_NAMES
}
