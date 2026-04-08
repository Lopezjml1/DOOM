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

//! Translated from linuxdoom-1.10/r_data.c and r_data.h
//!
//! Texture/flat/sprite/colormap initialization and caching.
//! Manages all graphics data used by the renderer.
//!
//! # Overview
//!
//! DOOM graphics for walls and sprites are stored in vertical runs of opaque
//! pixels (posts). A column is composed of zero or more posts, a patch or
//! sprite is composed of zero or more columns.
//!
//! This module handles:
//! - Loading wall textures composed from patches (TEXTURE1/TEXTURE2 + PNAMES)
//! - Loading floor/ceiling flats (F_START..F_END lump range)
//! - Pre-caching sprite dimensions (S_START..S_END lump range)
//! - Loading colormaps for diminishing lighting (COLORMAP lump)
//! - Runtime column retrieval for the wall rendering hot path
//!
//! # Texture Composition
//!
//! DOOM textures are built from one or more patches. The PNAMES lump maps
//! patch indices to lump names. TEXTURE1/TEXTURE2 lumps define how patches
//! are combined. Single-patch columns are served directly from the WAD lump;
//! multi-patch columns are composited into a cached buffer.

use std::io::Cursor;

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};

use crate::defs::{
    LightTable, Patch, Post, RenderState, Sector, SideDef, SpriteDef, SpriteFrame, FRACBITS,
    FRACUNIT,
};
use crate::sky::SkyState;
use doom_wad::types::PurgeTag;
use doom_wad::WadProvider;

// =============================================================================
// Helper functions
// =============================================================================

/// Convert an 8-byte null-padded WAD lump name to an uppercase String.
///
/// WAD names are up to 8 ASCII characters, null-padded. This function
/// extracts the significant characters and converts to uppercase for
/// case-insensitive matching.
fn lump_name_to_string(name: &[u8]) -> String {
    let len = name
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name.len().min(8));
    String::from_utf8_lossy(&name[..len]).to_uppercase()
}

/// Case-insensitive comparison of an 8-byte WAD name with a Rust string.
///
/// Handles null-padding in the WAD name and compares up to 8 characters
/// case-insensitively, matching the original `strncasecmp` behavior from
/// the C source (r_data.c line 705).
fn names_equal(wad_name: &[u8; 8], search_name: &str) -> bool {
    let wad_len = wad_name.iter().position(|&b| b == 0).unwrap_or(8);
    let search_bytes = search_name.as_bytes();
    let search_len = search_bytes.len().min(8);

    if wad_len != search_len {
        return false;
    }

    for i in 0..wad_len {
        if !wad_name[i].eq_ignore_ascii_case(&search_bytes[i]) {
            return false;
        }
    }
    true
}

// =============================================================================
// Internal types (from r_data.c lines 62-125)
// =============================================================================

/// A patch reference in a WAD map texture definition (mappatch_t).
///
/// Binary layout in the WAD: 10 bytes (5 × i16 LE).
/// Original C: r_data.c lines 69-76.
#[allow(dead_code)]
struct MapPatch {
    originx: i16,
    originy: i16,
    patch: i16,
    stepdir: i16,  // Unused in DOOM
    colormap: i16, // Unused in DOOM
}

/// A resolved patch within a texture (texpatch_t).
///
/// Runtime representation after resolving the patch index from PNAMES
/// to a lump number. Original C: r_data.c lines 99-107.
struct TexPatch {
    /// Horizontal origin in the texture (can be negative).
    originx: i32,
    /// Vertical origin in the texture (can be negative).
    originy: i32,
    /// Resolved WAD lump number for this patch.
    patch: i32,
}

/// A texture composed of one or more patches (texture_t).
///
/// Runtime representation of a wall texture definition loaded from
/// TEXTURE1/TEXTURE2 lumps. Original C: r_data.c lines 113-125.
struct Texture {
    /// 8-byte null-padded texture name (uppercase ASCII).
    name: [u8; 8],
    /// Width of the texture in pixels.
    width: i16,
    /// Height of the texture in pixels.
    height: i16,
    /// Number of patches composing this texture.
    patch_count: i16,
    /// The patches that make up this texture.
    patches: Vec<TexPatch>,
}

// =============================================================================
// DataState — Public renderer data cache
// =============================================================================

/// Renderer data state: texture, flat, sprite, and colormap caches.
///
/// Consolidates all formerly-global variables from r_data.c (lines 129-162)
/// and the precache counters (lines 739-741) into a single owned struct.
///
/// All data is loaded during `init_data()` and accessed during rendering.
/// The `get_column()` method is the hot-path entry point called per-column
/// during wall rendering.
pub struct DataState {
    // ---- Flat lump range (r_data.c lines 129-131) ----
    /// First flat lump index in the WAD directory.
    pub firstflat: i32,
    /// Last flat lump index in the WAD directory.
    pub lastflat: i32,
    /// Number of flat lumps (lastflat - firstflat + 1).
    pub numflats: i32,

    // ---- Patch lump range (r_data.c lines 133-135) ----
    /// First patch lump index (may be 0 if not tracked via markers).
    pub firstpatch: i32,
    /// Last patch lump index.
    pub lastpatch: i32,
    /// Number of patch lumps.
    pub numpatches: i32,

    // ---- Sprite lump range (r_data.c lines 137-139) ----
    /// First sprite lump index (S_START + 1).
    pub firstspritelump: i32,
    /// Last sprite lump index (S_END - 1).
    pub lastspritelump: i32,
    /// Number of sprite lumps.
    pub numspritelumps: i32,

    // ---- Texture data (r_data.c lines 141-151) ----
    /// Total number of textures from TEXTURE1 + TEXTURE2.
    pub numtextures: i32,
    /// Texture definitions (private — accessed via get_column).
    textures: Vec<Texture>,
    /// Per-texture width bitmask for fast column wrapping (power-of-2 - 1).
    pub texturewidthmask: Vec<i32>,
    /// Per-texture height in 16.16 fixed-point (height << FRACBITS).
    pub textureheight: Vec<i32>,
    /// Per-texture composite buffer size in bytes.
    texturecompositesize: Vec<i32>,
    /// Per-texture, per-column: lump number for direct access, or -1 for composite.
    texturecolumnlump: Vec<Vec<i16>>,
    /// Per-texture, per-column: byte offset into lump or composite buffer.
    texturecolumnofs: Vec<Vec<u16>>,
    /// Per-texture: cached composite texture data (None = not yet generated).
    texturecomposite: Vec<Option<Vec<u8>>>,

    // ---- Animation translation tables (r_data.c lines 154-155) ----
    /// Flat animation remapping: flattranslation[original] = animated_frame.
    pub flattranslation: Vec<i32>,
    /// Texture animation remapping: texturetranslation[original] = animated_frame.
    pub texturetranslation: Vec<i32>,

    // ---- Sprite dimensions pre-cached from patch headers (r_data.c lines 158-160) ----
    /// Sprite widths in 16.16 fixed-point.
    pub spritewidth: Vec<i32>,
    /// Sprite horizontal offsets (leftoffset) in 16.16 fixed-point.
    pub spriteoffset: Vec<i32>,
    /// Sprite vertical offsets (topoffset) in 16.16 fixed-point.
    pub spritetopoffset: Vec<i32>,

    // ---- Colormaps (r_data.c line 162) ----
    /// Colormap table: 32 brightness levels × 256 palette entries = 8192+ bytes.
    /// Used for diminishing lighting during rendering.
    /// Type uses LightTable (= u8) to match original lighttable_t.
    pub colormaps: Vec<LightTable>,

    // ---- Precache memory counters (r_data.c lines 739-741) ----
    /// Total memory used by precached flat lumps.
    pub flatmemory: i32,
    /// Total memory used by precached texture lumps.
    pub texturememory: i32,
    /// Total memory used by precached sprite lumps.
    pub spritememory: i32,

    // ---- Runtime column cache (Rust-specific) ----
    /// Cached WAD lump data for single-patch column access in get_column().
    /// Indexed by lump number. Replaces the original C's reliance on
    /// the zone memory cache (W_CacheLumpNum) during rendering.
    column_lump_cache: Vec<Option<Vec<u8>>>,
}

impl Default for DataState {
    fn default() -> Self {
        Self::new()
    }
}

impl DataState {
    /// Create a new `DataState` with all fields initialized to safe defaults.
    ///
    /// All arrays start empty; they are populated during `init_data()`.
    pub fn new() -> Self {
        Self {
            firstflat: 0,
            lastflat: 0,
            numflats: 0,
            firstpatch: 0,
            lastpatch: 0,
            numpatches: 0,
            firstspritelump: 0,
            lastspritelump: 0,
            numspritelumps: 0,
            numtextures: 0,
            textures: Vec::new(),
            texturewidthmask: Vec::new(),
            textureheight: Vec::new(),
            texturecompositesize: Vec::new(),
            texturecolumnlump: Vec::new(),
            texturecolumnofs: Vec::new(),
            texturecomposite: Vec::new(),
            flattranslation: Vec::new(),
            texturetranslation: Vec::new(),
            spritewidth: Vec::new(),
            spriteoffset: Vec::new(),
            spritetopoffset: Vec::new(),
            colormaps: Vec::new(),
            flatmemory: 0,
            texturememory: 0,
            spritememory: 0,
            column_lump_cache: Vec::new(),
        }
    }

    // =========================================================================
    // Internal helpers
    // =========================================================================

    /// Draw a column of pixels from a patch post list into a composite cache.
    ///
    /// Clips the column vertically against [0, cacheheight) and copies pixel
    /// data from the post list into the destination cache buffer.
    ///
    /// Translated from R_DrawColumnInCache (r_data.c lines 184-218).
    ///
    /// # Arguments
    /// * `column_data` — Raw bytes starting at the first post of a patch column.
    /// * `cache` — Destination buffer for this column in the composite texture.
    ///   Must be at least `cacheheight` bytes long.
    /// * `originy` — Vertical origin of the patch within the texture.
    /// * `cacheheight` — Height of the texture (clipping boundary).
    fn draw_column_in_cache(column_data: &[u8], cache: &mut [u8], originy: i32, cacheheight: i32) {
        let mut ofs: usize = 0;

        // Iterate through Post entries until sentinel topdelta (0xff)
        while ofs < column_data.len() {
            // Parse the post header (post_t / Post struct)
            let post: Post = Post {
                topdelta: column_data[ofs],
                length: if ofs + 1 < column_data.len() {
                    column_data[ofs + 1]
                } else {
                    0
                },
            };

            if post.topdelta == 0xff {
                break;
            }

            // Safety: ensure we read a valid length byte
            if ofs + 1 >= column_data.len() {
                break;
            }

            let length = post.length as i32;
            // Source pixel data starts 3 bytes into the post:
            // [topdelta(1) | length(1) | pre_pad(1) | data(length) | post_pad(1)]
            let source_start = ofs + 3;

            let mut count = length;
            let mut position = originy + post.topdelta as i32;

            // Clip top: if position is negative, skip pixels above the cache
            // NOTE: The original C code does NOT adjust the source pointer here.
            // This matches the original behavior exactly (r_data.c line 207).
            if position < 0 {
                count += position; // position is negative, so this reduces count
                position = 0;
            }

            // Clip bottom: don't write past the cache boundary
            if position + count > cacheheight {
                count = cacheheight - position;
            }

            // Copy pixel data if any remains after clipping
            if count > 0 {
                let dest_start = position as usize;
                let src_end = source_start + count as usize;
                if src_end <= column_data.len() && dest_start + count as usize <= cache.len() {
                    cache[dest_start..dest_start + count as usize]
                        .copy_from_slice(&column_data[source_start..src_end]);
                }
            }

            // Advance to the next post:
            // topdelta(1) + length(1) + pre_pad(1) + data(length) + post_pad(1) = length + 4
            ofs += length as usize + 4;
        }
    }

    /// Build a composite texture from multiple patches.
    ///
    /// Allocates a buffer of `texturecompositesize` bytes and composites all
    /// multi-patch columns by calling `draw_column_in_cache` for each patch
    /// column that covers a multi-patch position (collump == -1).
    ///
    /// Translated from R_GenerateComposite (r_data.c lines 228-289).
    fn generate_composite(&mut self, texnum: usize, wad: &mut impl WadProvider) {
        let size = self.texturecompositesize[texnum] as usize;
        if size == 0 {
            self.texturecomposite[texnum] = Some(Vec::new());
            return;
        }

        let tex_width = self.textures[texnum].width as i32;
        let tex_height = self.textures[texnum].height as i32;
        let patch_count = self.textures[texnum].patch_count as usize;

        // Copy patch info to local storage to avoid borrow conflicts with self
        let patches: Vec<(i32, i32, i32)> = (0..patch_count)
            .map(|i| {
                let p = &self.textures[texnum].patches[i];
                (p.originx, p.originy, p.patch)
            })
            .collect();

        let mut block = vec![0u8; size];

        for &(originx, originy, patch_lump) in &patches {
            // Read the full patch lump data
            let realpatch_data = wad.read_lump(patch_lump as usize);
            if realpatch_data.len() < 8 {
                continue;
            }

            // Parse patch header: width is at offset 0 (i16 LE)
            let patch_width = LittleEndian::read_i16(&realpatch_data[0..2]) as i32;

            let x1 = originx;
            let x2 = (x1 + patch_width).min(tex_width);
            let x_start = x1.max(0);

            for x in x_start..x2 {
                let xi = x as usize;

                // Skip columns that are single-patch (served directly from WAD lump)
                if self.texturecolumnlump[texnum][xi] >= 0 {
                    continue;
                }

                // Get column data offset from patch columnofs table
                let col_in_patch = (x - x1) as usize;
                let colofs_byte = 8 + col_in_patch * 4;
                if colofs_byte + 4 > realpatch_data.len() {
                    continue;
                }
                let col_offset = LittleEndian::read_i32(&realpatch_data[colofs_byte..]) as usize;

                if col_offset >= realpatch_data.len() {
                    continue;
                }

                // Destination offset within the composite block
                let block_ofs = self.texturecolumnofs[texnum][xi] as usize;
                let block_end = (block_ofs + tex_height as usize).min(block.len());
                if block_ofs >= block.len() {
                    continue;
                }

                Self::draw_column_in_cache(
                    &realpatch_data[col_offset..],
                    &mut block[block_ofs..block_end],
                    originy,
                    tex_height,
                );
            }
        }

        self.texturecomposite[texnum] = Some(block);
    }

    /// Analyze a texture to build the column lookup tables.
    ///
    /// For each column in the texture:
    /// - If exactly one patch covers it: `collump` = patch lump number,
    ///   `colofs` = byte offset into lump (past post header, +3).
    /// - If multiple patches cover it: `collump` = -1,
    ///   `colofs` = byte offset into the composite buffer.
    ///
    /// Also calculates `texturecompositesize` for multi-patch textures.
    ///
    /// Translated from R_GenerateLookup (r_data.c lines 296-374).
    fn generate_lookup(&mut self, texnum: usize, wad: &mut impl WadProvider) {
        // Reset composite state
        self.texturecomposite[texnum] = None;
        self.texturecompositesize[texnum] = 0;

        let tex_width = self.textures[texnum].width as usize;
        let tex_height = self.textures[texnum].height as i32;
        let patch_count = self.textures[texnum].patch_count as usize;

        // Per-column patch count tracking
        let mut column_patch_count = vec![0u8; tex_width];

        // Copy patch info to avoid borrow conflicts
        let patches: Vec<(i32, i32, i32)> = (0..patch_count)
            .map(|i| {
                let p = &self.textures[texnum].patches[i];
                (p.originx, p.originy, p.patch)
            })
            .collect();

        // First pass: determine which patch covers each column
        for &(originx, _originy, patch_lump) in &patches {
            let realpatch_data = wad.read_lump(patch_lump as usize);
            if realpatch_data.len() < 8 {
                continue;
            }

            let patch_width = LittleEndian::read_i16(&realpatch_data[0..2]) as i32;

            let x1 = originx;
            let x2 = (x1 + patch_width).min(tex_width as i32);
            let x_start = x1.max(0);

            for x in x_start..x2 {
                let xi = x as usize;

                // Saturating increment to prevent overflow on pathological data
                column_patch_count[xi] = column_patch_count[xi].saturating_add(1);

                // Record this patch's lump and column offset.
                // For multi-patch columns, these get overwritten and then replaced
                // in the second pass. For single-patch columns, the last (only)
                // patch's values persist.
                self.texturecolumnlump[texnum][xi] = patch_lump as i16;

                let col_in_patch = (x - x1) as usize;
                let colofs_byte = 8 + col_in_patch * 4;
                if colofs_byte + 4 <= realpatch_data.len() {
                    let patch_colofs = LittleEndian::read_i32(&realpatch_data[colofs_byte..]);
                    // +3 skips past the post header (topdelta + length + pre-pad)
                    // to point directly at pixel data
                    self.texturecolumnofs[texnum][xi] = (patch_colofs + 3) as u16;
                }
            }
        }

        // Second pass: handle multi-patch columns and detect missing patches
        for (x, &count) in column_patch_count.iter().enumerate().take(tex_width) {
            if count == 0 {
                // Column has no patch coverage — warn and bail out
                let name = lump_name_to_string(&self.textures[texnum].name);
                tracing::warn!("R_GenerateLookup: column without a patch ({})", name);
                return;
            }

            if count > 1 {
                // Multi-patch column: mark for compositing
                self.texturecolumnlump[texnum][x] = -1;
                self.texturecolumnofs[texnum][x] = self.texturecompositesize[texnum] as u16;

                if self.texturecompositesize[texnum] > 0x10000 - tex_height {
                    panic!("R_GenerateLookup: texture {} is >64k", texnum);
                }

                self.texturecompositesize[texnum] += tex_height;
            }
        }
    }

    // =========================================================================
    // Public column access — HOT PATH
    // =========================================================================

    /// Retrieve a column of pixel data from a texture.
    ///
    /// This is the renderer's hot-path entry point, called once per column
    /// during wall segment rendering. The column index wraps automatically
    /// via `texturewidthmask` (power-of-2 modulo).
    ///
    /// For single-patch columns, returns a slice into cached lump data (offset
    /// past the post header to raw pixel data). For multi-patch columns, returns
    /// a slice into the pre-composited texture buffer.
    ///
    /// Translated from R_GetColumn (r_data.c lines 382-401).
    ///
    /// # Arguments
    /// * `tex` — Texture index (0..numtextures).
    /// * `col` — Column index (automatically wrapped to texture width).
    ///
    /// # Returns
    /// A byte slice of pixel data for the column, at least `textureheight[tex]`
    /// pixels long for fully-covered columns.
    pub fn get_column(&mut self, tex: usize, col: i32) -> &[u8] {
        // Wrap column index using the power-of-2 bitmask
        let col_idx = (col & self.texturewidthmask[tex]) as usize;
        let lump = self.texturecolumnlump[tex][col_idx];
        let ofs = self.texturecolumnofs[tex][col_idx] as usize;

        if lump > 0 {
            // Single-patch column: return directly from cached lump data
            let lump_idx = lump as usize;
            if let Some(ref data) = self.column_lump_cache[lump_idx] {
                if ofs < data.len() {
                    return &data[ofs..];
                }
            }
            // Fallback: return empty slice if lump data unavailable
            return &[];
        }

        // Multi-patch column: return from composite texture buffer
        if let Some(ref composite) = self.texturecomposite[tex] {
            if ofs < composite.len() {
                return &composite[ofs..];
            }
        }

        // Composite not generated — should not happen after init
        &[]
    }

    // =========================================================================
    // Initialization functions
    // =========================================================================

    /// Load and resolve all wall textures from PNAMES, TEXTURE1, and TEXTURE2.
    ///
    /// This is the largest init function. It:
    /// 1. Reads PNAMES to build a patch name → lump number lookup table.
    /// 2. Reads TEXTURE1 (required) and TEXTURE2 (optional, commercial DOOM).
    /// 3. Parses each texture definition (maptexture_t + mappatch_t).
    /// 4. Builds the column directory via `generate_lookup` for all textures.
    /// 5. Pre-generates composites for multi-patch textures.
    /// 6. Pre-caches all referenced patch lumps for single-patch column access.
    /// 7. Creates the texture animation translation table (identity mapping).
    ///
    /// Translated from R_InitTextures (r_data.c lines 411-574).
    pub fn init_textures(&mut self, wad: &mut impl WadProvider) {
        tracing::info!("R_InitTextures: loading textures...");

        // --- Step 1: Load PNAMES lump (patch name → lump number mapping) ---
        let pnames_lump = wad
            .get_num_for_name("PNAMES")
            .expect("R_InitTextures: PNAMES lump not found");
        let pnames_data = wad.read_lump(pnames_lump);
        if pnames_data.len() < 4 {
            panic!("R_InitTextures: PNAMES lump too short");
        }

        // First 4 bytes: number of map patches (i32 LE)
        let mut cursor = Cursor::new(pnames_data.as_slice());
        let nummappatches = cursor
            .read_i32::<LittleEndian>()
            .expect("R_InitTextures: failed to read PNAMES count");

        self.numpatches = nummappatches;

        // Build the patchlookup table: index → lump number
        let mut patchlookup = Vec::with_capacity(nummappatches as usize);
        for i in 0..nummappatches as usize {
            let name_offset = 4 + i * 8;
            if name_offset + 8 > pnames_data.len() {
                patchlookup.push(-1i32);
                continue;
            }
            let name = lump_name_to_string(&pnames_data[name_offset..name_offset + 8]);
            let lump_num = wad.check_num_for_name(&name);
            patchlookup.push(lump_num.map(|n| n as i32).unwrap_or(-1));
        }

        // --- Step 2: Load TEXTURE1 (required) ---
        let tex1_lump = wad
            .get_num_for_name("TEXTURE1")
            .expect("R_InitTextures: TEXTURE1 lump not found");
        let maptex1 = wad.read_lump(tex1_lump);
        if maptex1.len() < 4 {
            panic!("R_InitTextures: TEXTURE1 lump too short");
        }
        let numtextures1 = LittleEndian::read_i32(&maptex1[0..4]);
        let maxoff1 = maptex1.len() as i32;

        // --- Step 3: Load TEXTURE2 (optional — present in commercial DOOM) ---
        let (maptex2, numtextures2, maxoff2) = match wad.check_num_for_name("TEXTURE2") {
            Some(lump) => {
                let data = wad.read_lump(lump);
                if data.len() < 4 {
                    (None, 0i32, 0i32)
                } else {
                    let count = LittleEndian::read_i32(&data[0..4]);
                    let maxoff = data.len() as i32;
                    (Some(data), count, maxoff)
                }
            }
            None => (None, 0i32, 0i32),
        };

        self.numtextures = numtextures1 + numtextures2;
        let num = self.numtextures as usize;

        // --- Step 4: Allocate all texture arrays ---
        self.textures = Vec::with_capacity(num);
        self.texturecolumnlump = Vec::with_capacity(num);
        self.texturecolumnofs = Vec::with_capacity(num);
        self.texturecomposite = vec![None; num];
        self.texturecompositesize = vec![0i32; num];
        self.texturewidthmask = vec![0i32; num];
        self.textureheight = vec![0i32; num];

        // --- Step 5: Parse each texture definition ---
        let mut current_dir: &[u8] = &maptex1;
        let mut current_maxoff = maxoff1;

        for i in 0..num {
            // Switch to TEXTURE2 data when we exhaust TEXTURE1 entries
            if i == numtextures1 as usize {
                if let Some(ref tex2) = maptex2 {
                    current_dir = tex2.as_slice();
                    current_maxoff = maxoff2;
                }
            }

            // Read the directory offset for this texture.
            // Directory starts at byte 4 in the lump; each entry is 4 bytes (i32 LE).
            let dir_idx = if i < numtextures1 as usize {
                i
            } else {
                i - numtextures1 as usize
            };
            let dir_byte = 4 + dir_idx * 4;
            if dir_byte + 4 > current_dir.len() {
                panic!("R_InitTextures: bad texture directory at index {}", i);
            }
            let offset = LittleEndian::read_i32(&current_dir[dir_byte..]) as usize;

            if offset as i32 > current_maxoff {
                panic!("R_InitTextures: bad texture directory");
            }

            // Parse maptexture_t at this offset:
            //   name[8]            bytes 0-7
            //   masked (i32)       bytes 8-11   (boolean stored as int, unused)
            //   width (i16)        bytes 12-13
            //   height (i16)       bytes 14-15
            //   columndirectory    bytes 16-19  (OBSOLETE, unused)
            //   patchcount (i16)   bytes 20-21
            //   patches[]          bytes 22+
            if offset + 22 > current_dir.len() {
                panic!("R_InitTextures: texture {} data truncated", i);
            }

            let mut name = [0u8; 8];
            name.copy_from_slice(&current_dir[offset..offset + 8]);

            let width = LittleEndian::read_i16(&current_dir[offset + 12..]);
            let height = LittleEndian::read_i16(&current_dir[offset + 14..]);
            let patchcount = LittleEndian::read_i16(&current_dir[offset + 20..]);

            // Parse each mappatch_t (10 bytes each):
            //   originx (i16)    bytes 0-1
            //   originy (i16)    bytes 2-3
            //   patch   (i16)    bytes 4-5    (index into PNAMES)
            //   stepdir (i16)    bytes 6-7    (unused)
            //   colormap (i16)   bytes 8-9    (unused)
            let mut patches = Vec::with_capacity(patchcount as usize);
            for j in 0..patchcount as usize {
                let pp = offset + 22 + j * 10;
                if pp + 10 > current_dir.len() {
                    break;
                }
                let originx = LittleEndian::read_i16(&current_dir[pp..]);
                let originy = LittleEndian::read_i16(&current_dir[pp + 2..]);
                let patch_idx = LittleEndian::read_i16(&current_dir[pp + 4..]);

                // Resolve patch index to lump number via patchlookup
                let patch_lump = if (patch_idx as usize) < patchlookup.len() {
                    patchlookup[patch_idx as usize]
                } else {
                    -1
                };

                if patch_lump == -1 {
                    tracing::warn!(
                        "R_InitTextures: missing patch {} in texture {}",
                        patch_idx,
                        lump_name_to_string(&name)
                    );
                }

                patches.push(TexPatch {
                    originx: originx as i32,
                    originy: originy as i32,
                    patch: patch_lump,
                });
            }

            self.textures.push(Texture {
                name,
                width,
                height,
                patch_count: patchcount,
                patches,
            });

            // Allocate per-column arrays (width entries each)
            self.texturecolumnlump.push(vec![0i16; width as usize]);
            self.texturecolumnofs.push(vec![0u16; width as usize]);

            // Calculate texturewidthmask: (next power of 2) - 1
            // This enables fast column wrapping via `col & mask`.
            let mut j = 1i32;
            while j * 2 <= width as i32 {
                j <<= 1;
            }
            self.texturewidthmask[i] = j - 1;

            // Store texture height in 16.16 fixed-point for texture pegging
            self.textureheight[i] = (height as i32) << FRACBITS;
        }

        // --- Step 6: Build column lookup tables for all textures ---
        for i in 0..num {
            self.generate_lookup(i, wad);
        }

        // --- Step 7: Pre-generate composites for multi-patch textures ---
        for i in 0..num {
            if self.texturecompositesize[i] > 0 {
                self.generate_composite(i, wad);
            }
        }

        // --- Step 8: Pre-cache all patch lumps for single-patch column access ---
        // Find the maximum lump number referenced by single-patch columns
        let mut max_lump: usize = 0;
        for i in 0..num {
            let w = self.textures[i].width as usize;
            for x in 0..w {
                let lump = self.texturecolumnlump[i][x];
                if lump > 0 && (lump as usize) > max_lump {
                    max_lump = lump as usize;
                }
            }
        }
        self.column_lump_cache = vec![None; max_lump + 1];

        for i in 0..num {
            let w = self.textures[i].width as usize;
            for x in 0..w {
                let lump = self.texturecolumnlump[i][x];
                if lump > 0 {
                    let lump_idx = lump as usize;
                    if self.column_lump_cache[lump_idx].is_none() {
                        self.column_lump_cache[lump_idx] = Some(wad.read_lump(lump_idx));
                    }
                }
            }
        }

        // --- Step 9: Create texture translation table (identity mapping) ---
        // texturetranslation[i] = i initially; game logic updates it each tic
        // for animated textures. Extra entry at the end for safety.
        self.texturetranslation = (0..=self.numtextures).collect();

        tracing::info!("R_InitTextures: loaded {} textures", self.numtextures);
    }

    /// Initialize the flat lump range from F_START/F_END markers.
    ///
    /// Flats are stored between the F_START and F_END marker lumps in the WAD.
    /// Each flat is a raw 64×64 pixel block (4096 bytes) with no header.
    ///
    /// Also creates the flat animation translation table (identity mapping).
    ///
    /// Translated from R_InitFlats (r_data.c lines 581-594).
    pub fn init_flats(&mut self, wad: &mut impl WadProvider) {
        tracing::info!("R_InitFlats: loading flat range...");

        let f_start = wad
            .get_num_for_name("F_START")
            .expect("R_InitFlats: F_START not found");
        let f_end = wad
            .get_num_for_name("F_END")
            .expect("R_InitFlats: F_END not found");

        self.firstflat = (f_start + 1) as i32;
        self.lastflat = (f_end - 1) as i32;
        self.numflats = self.lastflat - self.firstflat + 1;

        // Create flat translation table (identity mapping).
        // Extra entry at the end for safety, matching original behavior.
        self.flattranslation = (0..=self.numflats).collect();

        tracing::info!("R_InitFlats: {} flats loaded", self.numflats);
    }

    /// Pre-cache sprite lump dimensions from S_START/S_END range.
    ///
    /// Reads the patch header (width, leftoffset, topoffset) from each sprite
    /// lump and stores the values in fixed-point (shifted left by FRACBITS)
    /// for quick access during sprite projection.
    ///
    /// Translated from R_InitSpriteLumps (r_data.c lines 603-626).
    pub fn init_sprite_lumps(&mut self, wad: &mut impl WadProvider) {
        tracing::info!("R_InitSpriteLumps: caching sprite dimensions...");

        let s_start = wad
            .get_num_for_name("S_START")
            .expect("R_InitSpriteLumps: S_START not found");
        let s_end = wad
            .get_num_for_name("S_END")
            .expect("R_InitSpriteLumps: S_END not found");

        self.firstspritelump = (s_start + 1) as i32;
        self.lastspritelump = (s_end - 1) as i32;
        self.numspritelumps = self.lastspritelump - self.firstspritelump + 1;

        let num = self.numspritelumps as usize;
        self.spritewidth = vec![0i32; num];
        self.spriteoffset = vec![0i32; num];
        self.spritetopoffset = vec![0i32; num];

        for i in 0..num {
            // Print progress dot every 64 lumps (matching original behavior)
            if (i & 63) == 0 {
                tracing::debug!(".");
            }

            // Read the patch header from the sprite lump into a Patch struct.
            // patch_t layout: width(i16) height(i16) leftoffset(i16) topoffset(i16) columnofs(...)
            let lump_data = wad.read_lump(self.firstspritelump as usize + i);
            if lump_data.len() < 8 {
                continue;
            }

            let patch: Patch = Patch {
                width: LittleEndian::read_i16(&lump_data[0..2]),
                height: LittleEndian::read_i16(&lump_data[2..4]),
                leftoffset: LittleEndian::read_i16(&lump_data[4..6]),
                topoffset: LittleEndian::read_i16(&lump_data[6..8]),
                columnofs: Vec::new(), // Not needed for dimension caching
            };

            // Store dimensions in 16.16 fixed-point (multiply by FRACUNIT)
            self.spritewidth[i] = patch.width as i32 * FRACUNIT;
            self.spriteoffset[i] = patch.leftoffset as i32 * FRACUNIT;
            self.spritetopoffset[i] = patch.topoffset as i32 * FRACUNIT;
        }

        tracing::info!(
            "R_InitSpriteLumps: {} sprite lumps cached",
            self.numspritelumps
        );
    }

    /// Load the COLORMAP lump for diminishing lighting.
    ///
    /// The COLORMAP lump contains 32 brightness levels × 256 palette entries
    /// = 8192 bytes, plus an inverted colormap (256 bytes) for the invulnerability
    /// powerup, totaling 8448 bytes.
    ///
    /// The original C code aligned the buffer to a 256-byte boundary for
    /// assembly-optimized column drawing. In Rust, this alignment is not needed
    /// since we use safe indexed access.
    ///
    /// Translated from R_InitColormaps (r_data.c lines 633-644).
    pub fn init_colormaps(&mut self, wad: &mut impl WadProvider) {
        tracing::info!("R_InitColormaps: loading colormaps...");

        let colormap_lump = wad
            .get_num_for_name("COLORMAP")
            .expect("R_InitColormaps: COLORMAP lump not found");

        let length = wad.lump_length(colormap_lump);
        self.colormaps = wad.read_lump(colormap_lump);

        tracing::info!("R_InitColormaps: loaded {} bytes of colormap data", length);
    }

    /// Master initialization: call all data initialization functions in order.
    ///
    /// This is the single entry point for renderer data loading. It calls
    /// `init_textures`, `init_flats`, `init_sprite_lumps`, and `init_colormaps`
    /// in the correct order.
    ///
    /// Translated from R_InitData (r_data.c lines 654-664).
    pub fn init_data(&mut self, wad: &mut impl WadProvider) {
        tracing::info!("R_InitData: initializing renderer data...");

        self.init_textures(wad);
        self.init_flats(wad);
        self.init_sprite_lumps(wad);
        self.init_colormaps(wad);

        tracing::info!("R_InitData: complete.");
    }

    // =========================================================================
    // Name lookup functions
    // =========================================================================

    /// Look up a flat by name and return its index relative to `firstflat`.
    ///
    /// Panics if the flat is not found in the WAD lump directory.
    ///
    /// Translated from R_FlatNumForName (r_data.c lines 672-686).
    pub fn flat_num_for_name(&self, name: &str, wad: &impl WadProvider) -> i32 {
        match wad.check_num_for_name(name) {
            Some(lump_num) => {
                let flat_idx = lump_num as i32 - self.firstflat;
                if flat_idx < 0 || flat_idx >= self.numflats {
                    panic!("R_FlatNumForName: {} not found", name);
                }
                flat_idx
            }
            None => {
                panic!("R_FlatNumForName: {} not found", name);
            }
        }
    }

    /// Look up a texture by name, returning its index or -1 if not found.
    ///
    /// Returns 0 for the "NoTexture" marker (name starting with '-').
    /// Performs a case-insensitive search through all texture definitions.
    ///
    /// Translated from R_CheckTextureNumForName (r_data.c lines 696-709).
    pub fn check_texture_num_for_name(&self, name: &str) -> i32 {
        // "NoTexture" marker: a dash character indicates no texture assigned
        // to this sidedef slot. Return texture 0 as the fallback.
        // Original C returns 0 here (r_data.c line 703).
        if name.is_empty() || name.as_bytes()[0] == b'-' {
            return 0;
        }

        // Case-insensitive linear search through all textures
        for (i, tex) in self.textures.iter().enumerate() {
            if names_equal(&tex.name, name) {
                return i as i32;
            }
        }

        // Not found
        -1
    }

    /// Look up a texture by name, panicking if not found.
    ///
    /// Wraps `check_texture_num_for_name` and panics (I_Error equivalent)
    /// if the texture is not in the TEXTURE1/TEXTURE2 definitions.
    ///
    /// Translated from R_TextureNumForName (r_data.c lines 718-730).
    pub fn texture_num_for_name(&self, name: &str) -> i32 {
        let result = self.check_texture_num_for_name(name);
        if result == -1 {
            panic!("R_TextureNumForName: {} not found", name);
        }
        result
    }

    // =========================================================================
    // Level precaching
    // =========================================================================

    /// Pre-cache all graphics used by the current level.
    ///
    /// Scans all sectors for used flats, all sidedefs for used textures,
    /// and (if provided) all active sprites. Forces each used lump into
    /// the WAD cache to avoid hitching during gameplay.
    ///
    /// The sky texture is always precached regardless of sector references.
    ///
    /// Translated from R_PrecacheLevel (r_data.c lines 743-845).
    ///
    /// # Arguments
    /// * `wad` — WAD lump provider for cache operations.
    /// * `render_state` — Current map geometry (sectors, sides, sprites).
    /// * `sky` — Sky rendering state (provides sky texture number).
    /// * `demoplayback` — If true, skip precaching (demo mode optimization).
    /// * `active_sprites` — Boolean table indexed by sprite number;
    ///   `true` means the sprite is used by a visible thinker.
    pub fn precache_level(
        &mut self,
        wad: &mut impl WadProvider,
        render_state: &RenderState,
        sky: &SkyState,
        demoplayback: bool,
        active_sprites: &[bool],
    ) {
        if demoplayback {
            return;
        }

        // --- Precache flats ---
        {
            let num = self.numflats as usize;
            let mut hitlist = vec![false; num];

            for i in 0..render_state.numsectors {
                let sector: &Sector = &render_state.sectors[i];
                let floor = sector.floorpic as usize;
                let ceiling = sector.ceilingpic as usize;
                if floor < num {
                    hitlist[floor] = true;
                }
                if ceiling < num {
                    hitlist[ceiling] = true;
                }
            }

            self.flatmemory = 0;
            for (i, &hit) in hitlist.iter().enumerate() {
                if hit {
                    let lump_idx = self.firstflat as usize + i;
                    let _ = wad.cache_lump_num(lump_idx, PurgeTag::Cache);
                    self.flatmemory += wad.lump_length(lump_idx) as i32;
                }
            }
        }

        // --- Precache textures ---
        {
            let num = self.numtextures as usize;
            let mut hitlist = vec![false; num];

            // Scan all sidedefs for referenced textures
            for i in 0..render_state.numsides {
                let side: &SideDef = &render_state.sides[i];
                let top = side.toptexture as usize;
                let mid = side.midtexture as usize;
                let bot = side.bottomtexture as usize;
                if top < num {
                    hitlist[top] = true;
                }
                if mid < num {
                    hitlist[mid] = true;
                }
                if bot < num {
                    hitlist[bot] = true;
                }
            }

            // Always precache the sky texture
            let sky_tex = sky.skytexture as usize;
            if sky_tex < num {
                hitlist[sky_tex] = true;
            }

            self.texturememory = 0;
            for (i, &hit) in hitlist.iter().enumerate() {
                if !hit {
                    continue;
                }

                // Precache all patch lumps used by this texture
                let tex_width = self.textures[i].width as usize;
                for x in 0..tex_width {
                    let lump = self.texturecolumnlump[i][x];
                    if lump > 0 {
                        let lump_idx = lump as usize;
                        let _ = wad.cache_lump_num(lump_idx, PurgeTag::Cache);
                        self.texturememory += wad.lump_length(lump_idx) as i32;
                    }
                }

                // If the texture has a composite, count its memory
                if let Some(ref composite) = self.texturecomposite[i] {
                    self.texturememory += composite.len() as i32;
                }
            }
        }

        // --- Precache sprites ---
        {
            let num_sprites = render_state.numsprites;
            let num_sprite_lumps = self.numspritelumps as usize;

            let mut hitlist = vec![false; num_sprite_lumps];

            // Mark sprite lumps used by active thinkers
            for (sprite_idx, &is_active) in active_sprites.iter().enumerate() {
                if !is_active || sprite_idx >= num_sprites {
                    continue;
                }
                let sprdef: &SpriteDef = &render_state.sprites[sprite_idx];
                for f in 0..sprdef.numframes as usize {
                    if f >= sprdef.spriteframes.len() {
                        break;
                    }
                    let sf: &SpriteFrame = &sprdef.spriteframes[f];
                    for lump in &sf.lump {
                        let lump_idx = *lump as usize;
                        if lump_idx < num_sprite_lumps {
                            hitlist[lump_idx] = true;
                        }
                    }
                }
            }

            self.spritememory = 0;
            for (i, &hit) in hitlist.iter().enumerate() {
                if hit {
                    let lump_idx = self.firstspritelump as usize + i;
                    let _ = wad.cache_lump_num(lump_idx, PurgeTag::Cache);
                    self.spritememory += wad.lump_length(lump_idx) as i32;
                }
            }
        }

        tracing::info!(
            "R_PrecacheLevel: flats={}K textures={}K sprites={}K",
            self.flatmemory / 1024,
            self.texturememory / 1024,
            self.spritememory / 1024,
        );
    }
}

// =============================================================================
// Internal unit tests
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draw_column_in_cache_simple() {
        let column_data: Vec<u8> = vec![2, 3, 0, 10, 20, 30, 0, 0xff];
        let mut cache = vec![0u8; 8];
        DataState::draw_column_in_cache(&column_data, &mut cache, 0, 8);
        assert_eq!(cache, &[0, 0, 10, 20, 30, 0, 0, 0]);
    }

    #[test]
    fn test_draw_column_in_cache_originy() {
        let column_data: Vec<u8> = vec![0, 2, 0, 50, 60, 0, 0xff];
        let mut cache = vec![0u8; 8];
        DataState::draw_column_in_cache(&column_data, &mut cache, 3, 8);
        assert_eq!(cache, &[0, 0, 0, 50, 60, 0, 0, 0]);
    }

    #[test]
    fn test_draw_column_in_cache_clip_top() {
        let column_data: Vec<u8> = vec![0, 4, 0, 10, 20, 30, 40, 0, 0xff];
        let mut cache = vec![0u8; 4];
        DataState::draw_column_in_cache(&column_data, &mut cache, -2, 4);
        assert_eq!(cache, &[10, 20, 0, 0]);
    }

    #[test]
    fn test_draw_column_in_cache_clip_bottom() {
        let column_data: Vec<u8> = vec![0, 4, 0, 10, 20, 30, 40, 0, 0xff];
        let mut cache = vec![0u8; 3];
        DataState::draw_column_in_cache(&column_data, &mut cache, 0, 3);
        assert_eq!(cache, &[10, 20, 30]);
    }

    #[test]
    fn test_names_equal_basic() {
        let name: [u8; 8] = [b'S', b'T', b'A', b'R', b'T', 0, 0, 0];
        assert!(names_equal(&name, "START"));
        assert!(names_equal(&name, "start"));
        assert!(names_equal(&name, "Start"));
        assert!(!names_equal(&name, "STOP"));
        assert!(!names_equal(&name, "STARTING"));
    }

    #[test]
    fn test_names_equal_full_length() {
        let name: [u8; 8] = [b'A', b'A', b'S', b'H', b'W', b'A', b'L', b'L'];
        assert!(names_equal(&name, "AASHWALL"));
        assert!(names_equal(&name, "aashwall"));
        assert!(!names_equal(&name, "AASHWAL"));
    }

    #[test]
    fn test_lump_name_to_string() {
        let name: [u8; 8] = [b'T', b'E', b'S', b'T', 0, 0, 0, 0];
        assert_eq!(lump_name_to_string(&name), "TEST");
        let full: [u8; 8] = [b'F', b'U', b'L', b'L', b'N', b'A', b'M', b'E'];
        assert_eq!(lump_name_to_string(&full), "FULLNAME");
    }

    #[test]
    fn test_get_column_composite_wrap() {
        let mut ds = DataState::new();
        ds.numtextures = 1;
        ds.texturewidthmask = vec![3];
        ds.textureheight = vec![4 << 16];
        ds.texturecolumnlump = vec![vec![-1i16; 4]];
        ds.texturecolumnofs = vec![vec![0, 4, 8, 12]];
        ds.texturecomposite = vec![Some(vec![
            10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43,
        ])];
        ds.column_lump_cache = Vec::new();

        // get_column returns slice from offset to end (C pointer semantics).
        // The caller is responsible for reading only `height` bytes.
        let height = 4;
        assert_eq!(&ds.get_column(0, 0)[..height], &[10, 11, 12, 13]);
        assert_eq!(&ds.get_column(0, 1)[..height], &[20, 21, 22, 23]);
        // Wrapping: col 4 -> col 0
        assert_eq!(&ds.get_column(0, 4)[..height], &[10, 11, 12, 13]);
        // Wrapping: col -1 -> col 3 (twos complement: -1 & 3 = 3)
        assert_eq!(&ds.get_column(0, -1)[..height], &[40, 41, 42, 43]);
    }

    #[test]
    fn test_get_column_single_patch() {
        let mut ds = DataState::new();
        ds.numtextures = 1;
        ds.texturewidthmask = vec![1];
        ds.textureheight = vec![3 << 16];
        ds.texturecolumnlump = vec![vec![5, -1]];
        ds.texturecolumnofs = vec![vec![2, 0]];
        ds.texturecomposite = vec![Some(vec![99, 98, 97])];
        ds.column_lump_cache = vec![None; 6];
        ds.column_lump_cache[5] = Some(vec![0, 0, 50, 51, 52, 53, 54]);

        // get_column returns from offset to end; caller reads `height` bytes.
        let height = 3;
        assert_eq!(&ds.get_column(0, 0)[..height], &[50, 51, 52]);
        assert_eq!(&ds.get_column(0, 1)[..height], &[99, 98, 97]);
    }
}
