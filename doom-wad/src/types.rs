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

//! WAD type definitions and error types — translated from linuxdoom-1.10/w_wad.h and linuxdoom-1.10/z_zone.h
//!
//! This module contains the Rust equivalents of all C WAD data structures
//! declared in `w_wad.h` (lines 35–62), the zone memory purge tags from
//! `z_zone.h` (lines 36–44), plus new types introduced for the Rust port:
//!
//! - [`WadInfo`] — WAD file header (`wadinfo_t`)
//! - [`FileLump`] — On-disk directory entry (`filelump_t`)
//! - [`LumpInfo`] — Runtime directory entry (`lumpinfo_t`)
//! - [`WadType`] — IWAD vs PWAD discrimination
//! - [`LumpNum`] — Type-safe lump index (newtype pattern per AAP §0.4.3)
//! - [`PurgeTag`] — Cache eviction tags from `z_zone.h` `PU_*` defines
//! - [`CachedLump`] — Cached lump data with associated purge tag
//! - [`WadError`] — Typed error enum replacing `I_Error()` calls in `w_wad.c`
//!
//! All integer fields correspond to little-endian 32-bit values in the on-disk
//! WAD format. Actual byte-order reading is performed by `wad_file.rs` via the
//! `byteorder` crate (replacing the `LONG()` / `SHORT()` macros from
//! `m_swap.h`).
//!
//! This file has **no** internal crate dependencies — it is the most
//! foundational module in `doom-wad` and all other modules depend on it.

use std::fmt;

use thiserror::Error;

// =============================================================================
// WadInfo — WAD file header (wadinfo_t from w_wad.h:35-42)
// =============================================================================

/// WAD file header — equivalent of `wadinfo_t` from `w_wad.h:35-42`.
///
/// # On-disk layout (12 bytes, all little-endian)
///
/// | Offset | Size | Field           | Description                        |
/// |--------|------|-----------------|------------------------------------|
/// | 0      | 4    | identification  | `"IWAD"` or `"PWAD"` ASCII magic   |
/// | 4      | 4    | numlumps        | Number of lumps in the directory    |
/// | 8      | 4    | infotableofs    | File offset to the lump directory   |
///
/// # Original C
///
/// ```c
/// typedef struct {
///     char identification[4]; // "IWAD" or "PWAD"
///     int  numlumps;
///     int  infotableofs;
/// } wadinfo_t;
/// ```
#[derive(Debug, Clone)]
pub struct WadInfo {
    /// Should be `b"IWAD"` or `b"PWAD"` — 4-byte ASCII identification.
    pub identification: [u8; 4],
    /// Number of lumps in the WAD file directory.
    pub numlumps: i32,
    /// File offset (in bytes) to the beginning of the lump directory.
    pub infotableofs: i32,
}

// =============================================================================
// FileLump — On-disk directory entry (filelump_t from w_wad.h:45-51)
// =============================================================================

/// Directory entry as stored on disk in a WAD file — equivalent of
/// `filelump_t` from `w_wad.h:45-51`.
///
/// # On-disk layout (16 bytes, all little-endian)
///
/// | Offset | Size | Field   | Description                            |
/// |--------|------|---------|----------------------------------------|
/// | 0      | 4    | filepos | File offset where lump data begins     |
/// | 4      | 4    | size    | Size of the lump data in bytes         |
/// | 8      | 8    | name    | 8-char name, null-padded, uppercase    |
///
/// # Original C
///
/// ```c
/// typedef struct {
///     int  filepos;
///     int  size;
///     char name[8];
/// } filelump_t;
/// ```
#[derive(Debug, Clone)]
pub struct FileLump {
    /// File offset where the lump data begins.
    pub filepos: i32,
    /// Size of the lump data in bytes.
    pub size: i32,
    /// 8-character name, null-padded, uppercase ASCII.
    pub name: [u8; 8],
}

// =============================================================================
// LumpInfo — Runtime directory entry (lumpinfo_t from w_wad.h:56-62)
// =============================================================================

/// Runtime lump directory entry — equivalent of `lumpinfo_t` from
/// `w_wad.h:56-62`.
///
/// Unlike [`FileLump`] (the on-disk format), this includes runtime metadata
/// such as the file handle index used to read lump data from the correct
/// WAD file.
///
/// # Key difference from C
///
/// The `handle` field is `Option<usize>` instead of a raw `int`. The original
/// C code stores `-1` for reloadable files (`w_wad.c:214`:
/// `storehandle = reloadname ? -1 : handle`). In Rust, `None` represents the
/// reload case, and `Some(index)` indexes into the file handles vector.
///
/// # Original C
///
/// ```c
/// typedef struct {
///     char name[8];
///     int  handle;    // fd or -1 for reload files
///     int  position;
///     int  size;
/// } lumpinfo_t;
/// ```
#[derive(Debug, Clone)]
pub struct LumpInfo {
    /// 8-character name, null-padded, uppercase ASCII.
    pub name: [u8; 8],
    /// Index into the file handles vector (replaces C `int handle`).
    /// `None` indicates a reloadable file that uses `reloadname` for access.
    pub handle: Option<usize>,
    /// File offset where the lump data begins.
    pub position: i32,
    /// Size of the lump data in bytes.
    pub size: i32,
}

// =============================================================================
// WadType — IWAD vs PWAD discrimination
// =============================================================================

/// WAD file type discrimination — IWAD (Internal WAD with complete game data)
/// vs PWAD (Patch WAD with user modifications).
///
/// Determined by reading the 4-byte identification header from [`WadInfo`].
/// The original C code compared the raw bytes at `w_wad.c:185-192`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WadType {
    /// Internal WAD — `"IWAD"` identification. Contains complete game data
    /// (levels, textures, sprites, sounds, music).
    Iwad,
    /// Patch WAD — `"PWAD"` identification. Contains user modifications
    /// that override or supplement lumps from an IWAD.
    Pwad,
}

impl fmt::Display for WadType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WadType::Iwad => write!(f, "IWAD"),
            WadType::Pwad => write!(f, "PWAD"),
        }
    }
}

// =============================================================================
// LumpNum — Type-safe lump index (newtype per AAP §0.4.3)
// =============================================================================

/// Type-safe lump index — replaces bare `int` used throughout `w_wad.c`.
///
/// Uses the newtype pattern (AAP §0.4.3) to prevent accidental mixing of lump
/// indices with other integer values. The inner value is `usize` because Rust
/// uses unsigned indices for collections.
///
/// # Examples
///
/// ```
/// use doom_wad::types::LumpNum;
///
/// let lump = LumpNum(42);
/// assert_eq!(lump.0, 42);
///
/// // Convert from usize
/// let lump2: LumpNum = 10.into();
/// assert_eq!(lump2.0, 10);
///
/// // Convert to usize
/// let idx: usize = lump.into();
/// assert_eq!(idx, 42);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LumpNum(pub usize);

impl From<usize> for LumpNum {
    #[inline]
    fn from(value: usize) -> Self {
        LumpNum(value)
    }
}

impl From<LumpNum> for usize {
    #[inline]
    fn from(lump: LumpNum) -> Self {
        lump.0
    }
}

impl fmt::Display for LumpNum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// =============================================================================
// PurgeTag — Zone memory cache eviction tags (z_zone.h:36-44)
// =============================================================================

/// Cache purge/eviction tags — equivalent of `PU_*` defines from
/// `z_zone.h:36-44`.
///
/// These tags control when cached lump data may be automatically evicted:
///
/// - **Tags < 100** are *not* purgable — they persist until explicitly freed
///   or until a bulk `free_tags` operation targets their range.
/// - **Tags ≥ 100** are *purgable* — they may be evicted whenever the cache
///   needs to reclaim memory.
///
/// # Original C defines
///
/// ```c
/// #define PU_STATIC     1    // static entire execution time
/// #define PU_SOUND      2    // static while playing
/// #define PU_MUSIC      3    // static while playing
/// #define PU_DAVE       4    // anything else Dave wants static
/// #define PU_LEVEL     50    // static until level exited
/// #define PU_LEVSPEC   51    // a special thinker in a level
/// #define PU_PURGELEVEL 100  // boundary: tags >= this are purgable
/// #define PU_CACHE     101   // purgable whenever needed
/// ```
///
/// The `#[repr(u32)]` ensures discriminant values match the original C
/// `#define` values exactly for range comparison and serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum PurgeTag {
    /// `PU_STATIC = 1` — Static for the entire execution time. Never
    /// automatically freed.
    Static = 1,
    /// `PU_SOUND = 2` — Static while sound effect is playing.
    Sound = 2,
    /// `PU_MUSIC = 3` — Static while music is playing.
    Music = 3,
    /// `PU_DAVE = 4` — Anything else Dave Taylor wants to keep static.
    Dave = 4,
    /// `PU_LEVEL = 50` — Static until the current level is exited.
    /// Freed by `Z_FreeTags(PU_LEVEL, PU_LEVSPEC)` at level change.
    Level = 50,
    /// `PU_LEVSPEC = 51` — A special thinker allocated during level play.
    /// Freed alongside `PU_LEVEL` at level change.
    LevSpec = 51,
    /// `PU_PURGELEVEL = 100` — Boundary marker: tags ≥ this value are
    /// purgable whenever the cache needs to reclaim memory.
    PurgeLevel = 100,
    /// `PU_CACHE = 101` — Purgable whenever needed to reclaim memory.
    /// This is the standard tag for cached WAD lump data.
    Cache = 101,
}

impl PurgeTag {
    /// Returns `true` if this tag indicates a purgable cache entry.
    ///
    /// Codifies the `z_zone.h` rule: "Tags ≥ 100 are purgable whenever
    /// needed." In the original C code, this was checked as
    /// `block->tag >= PU_PURGELEVEL`.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::types::PurgeTag;
    ///
    /// assert!(!PurgeTag::Static.is_purgable());
    /// assert!(!PurgeTag::Level.is_purgable());
    /// assert!(PurgeTag::PurgeLevel.is_purgable());
    /// assert!(PurgeTag::Cache.is_purgable());
    /// ```
    #[inline]
    pub fn is_purgable(&self) -> bool {
        *self >= PurgeTag::PurgeLevel
    }

    /// Returns the numeric tag value as `u32` for range comparisons and
    /// diagnostic output.
    ///
    /// The returned value matches the original `#define` value from
    /// `z_zone.h` exactly (e.g., `PurgeTag::Static.as_u32() == 1`,
    /// `PurgeTag::Cache.as_u32() == 101`).
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::types::PurgeTag;
    ///
    /// assert_eq!(PurgeTag::Static.as_u32(), 1);
    /// assert_eq!(PurgeTag::Level.as_u32(), 50);
    /// assert_eq!(PurgeTag::Cache.as_u32(), 101);
    /// ```
    #[inline]
    pub fn as_u32(&self) -> u32 {
        *self as u32
    }
}

impl fmt::Display for PurgeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PurgeTag::Static => write!(f, "PU_STATIC(1)"),
            PurgeTag::Sound => write!(f, "PU_SOUND(2)"),
            PurgeTag::Music => write!(f, "PU_MUSIC(3)"),
            PurgeTag::Dave => write!(f, "PU_DAVE(4)"),
            PurgeTag::Level => write!(f, "PU_LEVEL(50)"),
            PurgeTag::LevSpec => write!(f, "PU_LEVSPEC(51)"),
            PurgeTag::PurgeLevel => write!(f, "PU_PURGELEVEL(100)"),
            PurgeTag::Cache => write!(f, "PU_CACHE(101)"),
        }
    }
}

// =============================================================================
// CachedLump — Cached lump entry for the HashMap-based lump cache
// =============================================================================

/// A cached lump entry — stores the lump data and its purge tag.
///
/// Used by `LumpCache` (in `lump_cache.rs`) as the value type in its internal
/// `HashMap`. This replaces the `lumpcache[i]` `void*` pointer from
/// `w_wad.c:64` combined with the `memblock_t.tag` field from the zone
/// allocator.
///
/// # Fields
///
/// - `data`: The raw lump data bytes read from the WAD file.
/// - `tag`: The current [`PurgeTag`] controlling eviction behavior. This can
///   be updated via `Z_ChangeTag` equivalent operations without re-reading
///   the data from disk.
#[derive(Debug, Clone)]
pub struct CachedLump {
    /// The raw lump data bytes.
    pub data: Vec<u8>,
    /// Current purge tag controlling eviction behavior.
    pub tag: PurgeTag,
}

// =============================================================================
// WadError — Typed error enum (replaces I_Error() from w_wad.c)
// =============================================================================

/// Error types for WAD operations — replaces `I_Error()` calls in `w_wad.c`.
///
/// In the original C engine, all WAD errors were fatal: `I_Error()` printed a
/// message to stderr and called `exit(-1)`. In the Rust port, these are
/// returned as typed `Result` errors, allowing callers to decide whether to
/// abort or attempt recovery.
///
/// Each variant documents the original `I_Error()` call it replaces, including
/// the line number in `w_wad.c`.
#[derive(Debug, Error)]
pub enum WadError {
    /// WAD file header is neither `"IWAD"` nor `"PWAD"`.
    ///
    /// Replaces: `I_Error("Wad file %s doesn't have IWAD or PWAD id", ...)`
    /// at `w_wad.c:190-191`.
    #[error("WAD file '{0}' doesn't have IWAD or PWAD identification")]
    InvalidWad(String),

    /// Lump name not found in the lump directory.
    ///
    /// Replaces: `I_Error("W_GetNumForName: %s not found!", name)`
    /// at `w_wad.c:406`.
    #[error("Lump '{0}' not found")]
    LumpNotFound(String),

    /// Lump index is out of bounds for the current directory.
    ///
    /// Replaces multiple bounds checks in `w_wad.c`:
    /// - `I_Error("W_LumpLength: %i >= numlumps", lump)` at line 419
    /// - `I_Error("W_ReadLump: %i >= numlumps", lump)` at line 441
    /// - `I_Error("W_CacheLumpNum: %i >= numlumps", lump)` at line 483
    #[error("Lump index {0} is out of bounds (total: {1})")]
    LumpIndexOutOfBounds(usize, usize),

    /// Incomplete read from disk — fewer bytes were read than expected.
    ///
    /// Replaces: `I_Error("W_ReadLump: only read %i of %i on lump %i", ...)`
    /// at `w_wad.c:460`.
    #[error("Incomplete read of lump {lump}: read {read} of {expected} bytes")]
    IncompleteRead {
        /// The lump index that was being read.
        lump: usize,
        /// Number of bytes actually read.
        read: usize,
        /// Number of bytes expected (lump size).
        expected: usize,
    },

    /// File could not be opened.
    ///
    /// Replaces: `printf(" couldn't open %s\n", filename)` at `w_wad.c:165`.
    #[error("Could not open file: {0}")]
    FileOpen(String),

    /// No WAD files were found during initialization.
    ///
    /// Replaces: `I_Error("W_InitFiles: no files found")` at `w_wad.c:306`.
    #[error("No WAD files found")]
    NoFilesFound,

    /// Filename base exceeds the 8-character limit for lump names.
    ///
    /// Replaces: `I_Error("Filename base of %s >8 chars", path)`
    /// at `w_wad.c:110`.
    #[error("Filename base of '{0}' exceeds 8 characters")]
    FilenameTooLong(String),

    /// Reload file could not be opened during `W_Reload`.
    ///
    /// Replaces: `I_Error("W_Reload: couldn't open %s", reloadname)`
    /// at `w_wad.c:250`.
    #[error("Could not open reload file: {0}")]
    ReloadError(String),

    /// Generic I/O error wrapping [`std::io::Error`].
    ///
    /// Catches all other I/O failures (permission denied, disk full,
    /// interrupted read, etc.) that don't map to a more specific variant.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // WadInfo tests
    // -------------------------------------------------------------------------

    #[test]
    fn wad_info_iwad_identification() {
        let info = WadInfo {
            identification: *b"IWAD",
            numlumps: 100,
            infotableofs: 12,
        };
        assert_eq!(&info.identification, b"IWAD");
        assert_eq!(info.numlumps, 100);
        assert_eq!(info.infotableofs, 12);
    }

    #[test]
    fn wad_info_pwad_identification() {
        let info = WadInfo {
            identification: *b"PWAD",
            numlumps: 50,
            infotableofs: 1024,
        };
        assert_eq!(&info.identification, b"PWAD");
        assert_eq!(info.numlumps, 50);
        assert_eq!(info.infotableofs, 1024);
    }

    #[test]
    fn wad_info_clone() {
        let info = WadInfo {
            identification: *b"IWAD",
            numlumps: 42,
            infotableofs: 256,
        };
        let cloned = info.clone();
        assert_eq!(cloned.numlumps, 42);
        assert_eq!(cloned.infotableofs, 256);
    }

    // -------------------------------------------------------------------------
    // FileLump tests
    // -------------------------------------------------------------------------

    #[test]
    fn file_lump_fields() {
        let lump = FileLump {
            filepos: 0x1000,
            size: 4096,
            name: *b"PLAYPAL\0",
        };
        assert_eq!(lump.filepos, 0x1000);
        assert_eq!(lump.size, 4096);
        assert_eq!(&lump.name, b"PLAYPAL\0");
    }

    #[test]
    fn file_lump_name_padding() {
        // Short names are null-padded to 8 bytes.
        let lump = FileLump {
            filepos: 0,
            size: 0,
            name: *b"MAP01\0\0\0",
        };
        assert_eq!(&lump.name[..5], b"MAP01");
        assert_eq!(lump.name[5], 0);
        assert_eq!(lump.name[6], 0);
        assert_eq!(lump.name[7], 0);
    }

    // -------------------------------------------------------------------------
    // LumpInfo tests
    // -------------------------------------------------------------------------

    #[test]
    fn lump_info_normal_handle() {
        let info = LumpInfo {
            name: *b"DEMO1\0\0\0",
            handle: Some(0),
            position: 0x2000,
            size: 8192,
        };
        assert_eq!(info.handle, Some(0));
        assert_eq!(info.position, 0x2000);
        assert_eq!(info.size, 8192);
    }

    #[test]
    fn lump_info_reload_handle() {
        // None represents the C -1 sentinel for reloadable files.
        let info = LumpInfo {
            name: *b"THINGS\0\0",
            handle: None,
            position: 0x100,
            size: 512,
        };
        assert!(info.handle.is_none());
    }

    // -------------------------------------------------------------------------
    // WadType tests
    // -------------------------------------------------------------------------

    #[test]
    fn wad_type_equality() {
        assert_eq!(WadType::Iwad, WadType::Iwad);
        assert_eq!(WadType::Pwad, WadType::Pwad);
        assert_ne!(WadType::Iwad, WadType::Pwad);
    }

    #[test]
    fn wad_type_copy() {
        let wt = WadType::Iwad;
        let wt2 = wt; // Copy
        assert_eq!(wt, wt2);
    }

    #[test]
    fn wad_type_display() {
        assert_eq!(format!("{}", WadType::Iwad), "IWAD");
        assert_eq!(format!("{}", WadType::Pwad), "PWAD");
    }

    // -------------------------------------------------------------------------
    // LumpNum tests
    // -------------------------------------------------------------------------

    #[test]
    fn lump_num_newtype() {
        let lump = LumpNum(42);
        assert_eq!(lump.0, 42);
    }

    #[test]
    fn lump_num_from_usize() {
        let lump: LumpNum = 10.into();
        assert_eq!(lump.0, 10);
    }

    #[test]
    fn lump_num_into_usize() {
        let lump = LumpNum(99);
        let idx: usize = lump.into();
        assert_eq!(idx, 99);
    }

    #[test]
    fn lump_num_equality() {
        assert_eq!(LumpNum(0), LumpNum(0));
        assert_ne!(LumpNum(1), LumpNum(2));
    }

    #[test]
    fn lump_num_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(LumpNum(1));
        set.insert(LumpNum(2));
        set.insert(LumpNum(1)); // duplicate
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn lump_num_ordering() {
        assert!(LumpNum(1) < LumpNum(2));
        assert!(LumpNum(10) > LumpNum(5));
    }

    #[test]
    fn lump_num_display() {
        assert_eq!(format!("{}", LumpNum(42)), "42");
    }

    // -------------------------------------------------------------------------
    // PurgeTag tests
    // -------------------------------------------------------------------------

    #[test]
    fn purge_tag_exact_values() {
        // Verify exact mapping from z_zone.h:36-44.
        assert_eq!(PurgeTag::Static as u32, 1);
        assert_eq!(PurgeTag::Sound as u32, 2);
        assert_eq!(PurgeTag::Music as u32, 3);
        assert_eq!(PurgeTag::Dave as u32, 4);
        assert_eq!(PurgeTag::Level as u32, 50);
        assert_eq!(PurgeTag::LevSpec as u32, 51);
        assert_eq!(PurgeTag::PurgeLevel as u32, 100);
        assert_eq!(PurgeTag::Cache as u32, 101);
    }

    #[test]
    fn purge_tag_as_u32() {
        assert_eq!(PurgeTag::Static.as_u32(), 1);
        assert_eq!(PurgeTag::Sound.as_u32(), 2);
        assert_eq!(PurgeTag::Music.as_u32(), 3);
        assert_eq!(PurgeTag::Dave.as_u32(), 4);
        assert_eq!(PurgeTag::Level.as_u32(), 50);
        assert_eq!(PurgeTag::LevSpec.as_u32(), 51);
        assert_eq!(PurgeTag::PurgeLevel.as_u32(), 100);
        assert_eq!(PurgeTag::Cache.as_u32(), 101);
    }

    #[test]
    fn purge_tag_is_purgable() {
        // Tags < 100 are NOT purgable.
        assert!(!PurgeTag::Static.is_purgable());
        assert!(!PurgeTag::Sound.is_purgable());
        assert!(!PurgeTag::Music.is_purgable());
        assert!(!PurgeTag::Dave.is_purgable());
        assert!(!PurgeTag::Level.is_purgable());
        assert!(!PurgeTag::LevSpec.is_purgable());

        // Tags >= 100 ARE purgable.
        assert!(PurgeTag::PurgeLevel.is_purgable());
        assert!(PurgeTag::Cache.is_purgable());
    }

    #[test]
    fn purge_tag_ordering() {
        // PartialOrd/Ord must respect numeric ordering.
        assert!(PurgeTag::Static < PurgeTag::Sound);
        assert!(PurgeTag::Sound < PurgeTag::Music);
        assert!(PurgeTag::Music < PurgeTag::Dave);
        assert!(PurgeTag::Dave < PurgeTag::Level);
        assert!(PurgeTag::Level < PurgeTag::LevSpec);
        assert!(PurgeTag::LevSpec < PurgeTag::PurgeLevel);
        assert!(PurgeTag::PurgeLevel < PurgeTag::Cache);
    }

    #[test]
    fn purge_tag_display() {
        assert_eq!(format!("{}", PurgeTag::Static), "PU_STATIC(1)");
        assert_eq!(format!("{}", PurgeTag::Cache), "PU_CACHE(101)");
        assert_eq!(format!("{}", PurgeTag::Level), "PU_LEVEL(50)");
    }

    #[test]
    fn purge_tag_copy() {
        let tag = PurgeTag::Cache;
        let tag2 = tag; // Copy
        assert_eq!(tag, tag2);
    }

    // -------------------------------------------------------------------------
    // CachedLump tests
    // -------------------------------------------------------------------------

    #[test]
    fn cached_lump_creation() {
        let cached = CachedLump {
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
            tag: PurgeTag::Cache,
        };
        assert_eq!(cached.data.len(), 4);
        assert_eq!(cached.data[0], 0xDE);
        assert_eq!(cached.tag, PurgeTag::Cache);
    }

    #[test]
    fn cached_lump_clone() {
        let cached = CachedLump {
            data: vec![1, 2, 3],
            tag: PurgeTag::Static,
        };
        let cloned = cached.clone();
        assert_eq!(cloned.data, vec![1, 2, 3]);
        assert_eq!(cloned.tag, PurgeTag::Static);
    }

    #[test]
    fn cached_lump_empty_data() {
        let cached = CachedLump {
            data: Vec::new(),
            tag: PurgeTag::Level,
        };
        assert!(cached.data.is_empty());
        assert_eq!(cached.tag, PurgeTag::Level);
    }

    // -------------------------------------------------------------------------
    // WadError tests
    // -------------------------------------------------------------------------

    #[test]
    fn wad_error_invalid_wad_display() {
        let err = WadError::InvalidWad("test.wad".to_string());
        assert_eq!(
            format!("{err}"),
            "WAD file 'test.wad' doesn't have IWAD or PWAD identification"
        );
    }

    #[test]
    fn wad_error_lump_not_found_display() {
        let err = WadError::LumpNotFound("PLAYPAL".to_string());
        assert_eq!(format!("{err}"), "Lump 'PLAYPAL' not found");
    }

    #[test]
    fn wad_error_lump_index_out_of_bounds_display() {
        let err = WadError::LumpIndexOutOfBounds(500, 100);
        assert_eq!(
            format!("{err}"),
            "Lump index 500 is out of bounds (total: 100)"
        );
    }

    #[test]
    fn wad_error_incomplete_read_display() {
        let err = WadError::IncompleteRead {
            lump: 42,
            read: 100,
            expected: 200,
        };
        assert_eq!(
            format!("{err}"),
            "Incomplete read of lump 42: read 100 of 200 bytes"
        );
    }

    #[test]
    fn wad_error_file_open_display() {
        let err = WadError::FileOpen("doom.wad".to_string());
        assert_eq!(format!("{err}"), "Could not open file: doom.wad");
    }

    #[test]
    fn wad_error_no_files_found_display() {
        let err = WadError::NoFilesFound;
        assert_eq!(format!("{err}"), "No WAD files found");
    }

    #[test]
    fn wad_error_filename_too_long_display() {
        let err = WadError::FilenameTooLong("verylongfilename".to_string());
        assert_eq!(
            format!("{err}"),
            "Filename base of 'verylongfilename' exceeds 8 characters"
        );
    }

    #[test]
    fn wad_error_reload_error_display() {
        let err = WadError::ReloadError("reload.wad".to_string());
        assert_eq!(format!("{err}"), "Could not open reload file: reload.wad");
    }

    #[test]
    fn wad_error_io_from_std() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let wad_err: WadError = io_err.into();
        let msg = format!("{wad_err}");
        assert!(msg.contains("file missing"));
    }

    #[test]
    fn wad_error_is_std_error() {
        // Verify that WadError implements std::error::Error.
        let err: Box<dyn std::error::Error> = Box::new(WadError::NoFilesFound);
        assert_eq!(format!("{err}"), "No WAD files found");
    }

    #[test]
    fn wad_error_debug_format() {
        let err = WadError::InvalidWad("bad.wad".to_string());
        let debug = format!("{err:?}");
        assert!(debug.contains("InvalidWad"));
        assert!(debug.contains("bad.wad"));
    }
}
