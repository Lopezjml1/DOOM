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

//! WadProvider trait definition — the WAD access contract for doom-core.
//! Translated from linuxdoom-1.10/w_wad.h function declarations.
//!
//! # Purpose
//!
//! This module defines the [`WadProvider`] trait, which abstracts all WAD data
//! access behind a trait boundary. This is a core architectural pattern mandated
//! by the AAP (§0.4.3: "Trait-based platform abstraction"). By depending on
//! this trait rather than the concrete `WadFile`/`LumpCache` implementation,
//! `doom-core` and other consuming crates remain decoupled from the WAD I/O
//! implementation details.
//!
//! # Design Rationale
//!
//! The original C engine exposed WAD access as a flat set of global functions
//! (`W_CheckNumForName`, `W_GetNumForName`, `W_LumpLength`, `W_ReadLump`,
//! `W_CacheLumpNum`, `W_CacheLumpName`) declared in `w_wad.h` (lines 69–79)
//! and implemented in `w_wad.c`. All state was global — `numlumps`,
//! `lumpinfo`, and `lumpcache` were extern globals shared across the entire
//! engine.
//!
//! In the Rust port, these functions become methods on the `WadProvider` trait,
//! with all state encapsulated inside the implementing type. This provides:
//!
//! - **Decoupling**: `doom-core` depends on the trait, not the implementation.
//! - **Testability**: Tests can provide mock WAD providers without real files.
//! - **Encapsulation**: No global mutable state — the implementor owns all data.
//! - **Idiomatic Rust error handling**: `Option` and `Result` replace C's `-1`
//!   sentinel values and `I_Error()` abort calls.
//!
//! # C-to-Rust Translation Notes
//!
//! | C Pattern | Rust Equivalent | Rationale |
//! |-----------|----------------|-----------|
//! | Return `-1` for "not found" (`w_wad.c:389`) | Return `Option<usize>` / `None` | Type-safe absence representation |
//! | Call `I_Error()` and abort (`w_wad.c:406`) | Return `Result<_, WadError>` | Recoverable error handling |
//! | Write into `void* dest` buffer (`w_wad.c:434`) | Return `Vec<u8>` (owned data) | Rust ownership; no dangling pointers |
//! | Cache modifies global `lumpcache[]` | `&mut self` on cache methods | Borrow checker enforces mutation safety |
//! | Return `void*` from cache (`w_wad.c:499`) | Return `&[u8]` (borrowed slice) | Lifetime-safe reference to cached data |
//!
//! # Cross-Crate Import Pattern
//!
//! ```text
//! // Old C:
//! #include "w_wad.h"
//! W_CacheLumpName("PLAYPAL", PU_CACHE);
//!
//! // New Rust:
//! use doom_wad::WadProvider;
//! wad.cache_lump_name("PLAYPAL", PurgeTag::Cache);
//! ```
//!
//! # Source Mapping
//!
//! The trait methods map directly to C function signatures in `w_wad.h`:
//!
//! | Rust Method | C Function (w_wad.h) | C Implementation (w_wad.c) |
//! |---|---|---|
//! | [`WadProvider::check_num_for_name`] | `W_CheckNumForName` (line 72) | Lines 351–390 |
//! | [`WadProvider::get_num_for_name`] | `W_GetNumForName` (line 73) | Lines 399–409 |
//! | [`WadProvider::lump_length`] | `W_LumpLength` (line 75) | Lines 416–422 |
//! | [`WadProvider::read_lump`] | `W_ReadLump` (line 76) | Lines 431–467 |
//! | [`WadProvider::cache_lump_num`] | `W_CacheLumpNum` (line 78) | Lines 475–500 |
//! | [`WadProvider::cache_lump_name`] | `W_CacheLumpName` (line 79) | Lines 507–513 |
//! | [`WadProvider::num_lumps`] | `W_NumLumps` (not in .h) | Lines 339–342 |

use crate::types::{PurgeTag, WadError};

/// The WAD access contract — abstracts lump lookup, reading, and caching.
///
/// This trait is the Rust equivalent of the extern function declarations at
/// `w_wad.h:69–79`. Any type that can provide access to WAD lump data
/// implements this trait, enabling `doom-core` to work with WAD data without
/// depending on the concrete I/O and caching implementation.
///
/// # Implementors
///
/// The primary implementor is `WadFile` (in `wad_file.rs`) combined with
/// `LumpCache` (in `lump_cache.rs`), which together provide file-backed
/// WAD loading and HashMap-based lump caching. Test code may provide
/// lightweight mock implementations for unit testing.
///
/// # Lump Naming Convention
///
/// Lump names are case-insensitive, up to 8 ASCII characters, null-padded.
/// The original C code performed case-insensitive comparison by converting
/// to uppercase via `strupr()` (`w_wad.c:69-72`) before scanning.
/// Implementations should handle case-insensitive matching internally.
///
/// # Lump Ordering
///
/// When multiple WAD files are loaded, lumps from later files override
/// lumps with the same name from earlier files. This is achieved by the
/// backward scan in `check_num_for_name` (equivalent to `w_wad.c:377-386`
/// which scans from `numlumps-1` down to `0`).
///
/// # Error Handling
///
/// Methods that can fail return `Result<T, WadError>`. This replaces
/// the original C pattern where `I_Error()` would print a message and
/// call `exit(-1)`. Callers can now choose to propagate, log, or handle
/// errors as appropriate.
///
/// # Cache Mutation
///
/// The `cache_lump_num` and `cache_lump_name` methods require `&mut self`
/// because caching data modifies internal state (populating the cache or
/// updating the purge tag on an existing entry). Read-only queries
/// (`check_num_for_name`, `get_num_for_name`, `lump_length`, `num_lumps`)
/// only require `&self`.
///
/// The `read_lump` method also only requires `&self` because it returns
/// owned data (`Vec<u8>`) without modifying the provider's internal cache.
pub trait WadProvider {
    /// Check if a lump with the given name exists, returning its index.
    ///
    /// Equivalent of `W_CheckNumForName` (`w_wad.h:72`, `w_wad.c:351–390`).
    ///
    /// Performs a **backward scan** through the lump directory — later WAD
    /// files override earlier ones for lumps with the same name. This
    /// preserves the PWAD override semantics: if `DOOM2.WAD` defines a
    /// `MAP01` lump and a PWAD also defines `MAP01`, the PWAD version is
    /// found first because it was loaded later.
    ///
    /// # Parameters
    ///
    /// - `name`: The lump name to search for, case-insensitive, up to 8
    ///   characters. Names longer than 8 characters are truncated by the
    ///   implementation to match the WAD format limit.
    ///
    /// # Returns
    ///
    /// - `Some(index)` — The lump index if found (zero-based).
    /// - `None` — The lump was not found in any loaded WAD file.
    ///
    /// # C Translation Note
    ///
    /// The original C function returned `-1` as a "not found" sentinel
    /// (`w_wad.c:389: return -1;`). Rust uses `Option<usize>` to represent
    /// this without sentinel values, making the absent case impossible to
    /// misuse as a valid index.
    fn check_num_for_name(&self, name: &str) -> Option<usize>;

    /// Get the lump index for a given name, or return an error if not found.
    ///
    /// Equivalent of `W_GetNumForName` (`w_wad.h:73`, `w_wad.c:399–409`).
    ///
    /// This is the strict variant of [`check_num_for_name`](Self::check_num_for_name) —
    /// it returns an error instead of `None` when the lump is not found.
    /// Most engine code uses this method because a missing required lump
    /// is a fatal condition (e.g., missing `PLAYPAL` or `COLORMAP`).
    ///
    /// # Parameters
    ///
    /// - `name`: The lump name to search for, case-insensitive, up to 8
    ///   characters.
    ///
    /// # Returns
    ///
    /// - `Ok(index)` — The lump index if found.
    /// - `Err(WadError::LumpNotFound)` — The lump was not found.
    ///
    /// # C Translation Note
    ///
    /// The original C function called `I_Error("W_GetNumForName: %s not found!", name)`
    /// at `w_wad.c:406`, which printed to stderr and terminated the process.
    /// Returning `Result` allows callers to handle the error gracefully or
    /// propagate it up the call chain.
    fn get_num_for_name(&self, name: &str) -> Result<usize, WadError>;

    /// Return the size in bytes of the given lump's data.
    ///
    /// Equivalent of `W_LumpLength` (`w_wad.h:75`, `w_wad.c:416–422`).
    ///
    /// Returns the buffer size needed to load the lump data. This value
    /// comes from the `size` field of the lump's directory entry
    /// (`lumpinfo[lump].size` in the original C code).
    ///
    /// # Parameters
    ///
    /// - `lump`: The zero-based lump index. Must be less than
    ///   [`num_lumps()`](Self::num_lumps).
    ///
    /// # Returns
    ///
    /// The size of the lump data in bytes.
    ///
    /// # Panics
    ///
    /// Implementations should panic or return 0 if `lump` is out of bounds.
    /// The original C code called `I_Error("W_LumpLength: %i >= numlumps")`
    /// at `w_wad.c:419`.
    fn lump_length(&self, lump: usize) -> usize;

    /// Read the lump data from disk into a new owned buffer.
    ///
    /// Equivalent of `W_ReadLump` (`w_wad.h:76`, `w_wad.c:431–467`).
    ///
    /// Reads the complete lump data from the WAD file on disk and returns
    /// it as an owned `Vec<u8>`. This does **not** interact with the lump
    /// cache — it always performs a fresh disk read.
    ///
    /// # Parameters
    ///
    /// - `lump`: The zero-based lump index. Must be less than
    ///   [`num_lumps()`](Self::num_lumps).
    ///
    /// # Returns
    ///
    /// A `Vec<u8>` containing the complete lump data. The length of the
    /// returned vector equals [`lump_length(lump)`](Self::lump_length).
    ///
    /// # C Translation Note
    ///
    /// The original C function took a `void* dest` parameter and wrote
    /// directly into a caller-provided buffer (`w_wad.c:434`). The Rust
    /// version returns an owned `Vec<u8>` instead, which is safer (no
    /// buffer overflows) and more idiomatic (the caller doesn't need to
    /// pre-allocate a buffer of the correct size).
    ///
    /// # Panics
    ///
    /// Implementations should panic if `lump` is out of bounds or if the
    /// disk read fails, matching the original `I_Error()` behavior for
    /// these fatal conditions.
    fn read_lump(&self, lump: usize) -> Vec<u8>;

    /// Cache a lump by index, returning a reference to the cached data.
    ///
    /// Equivalent of `W_CacheLumpNum` (`w_wad.h:78`, `w_wad.c:475–500`).
    ///
    /// If the lump is already cached, returns a reference to the existing
    /// cached data and updates its purge tag to `tag`. If not cached,
    /// reads the lump from disk, stores it in the cache with the given
    /// tag, and returns a reference to the newly cached data.
    ///
    /// # Parameters
    ///
    /// - `lump`: The zero-based lump index. Must be less than
    ///   [`num_lumps()`](Self::num_lumps).
    /// - `tag`: The [`PurgeTag`] controlling cache eviction behavior for
    ///   this entry. Tags with `is_purgable() == true` (≥ `PU_PURGELEVEL`)
    ///   allow the cache to evict this entry when memory is needed. Tags
    ///   below that threshold keep the data pinned in cache.
    ///
    /// # Returns
    ///
    /// A byte slice reference to the cached lump data. The reference is
    /// valid until the cache is modified (next `&mut self` call).
    ///
    /// # Cache Behavior
    ///
    /// - **Cache miss**: Reads from disk via `read_lump`, stores in cache.
    ///   Equivalent to `w_wad.c:486-491`:
    ///   ```c
    ///   ptr = Z_Malloc(W_LumpLength(lump), tag, &lumpcache[lump]);
    ///   W_ReadLump(lump, lumpcache[lump]);
    ///   ```
    /// - **Cache hit**: Updates the tag (equivalent to `Z_ChangeTag` at
    ///   `w_wad.c:496`). This allows callers to promote a `PU_CACHE`
    ///   entry to `PU_STATIC` to prevent eviction.
    ///
    /// # Mutation Note
    ///
    /// This method requires `&mut self` because populating the cache on a
    /// miss or updating a tag on a hit both mutate internal state.
    fn cache_lump_num(&mut self, lump: usize, tag: PurgeTag) -> &[u8];

    /// Cache a lump by name, returning a reference to the cached data.
    ///
    /// Equivalent of `W_CacheLumpName` (`w_wad.h:79`, `w_wad.c:507–513`).
    ///
    /// This is a convenience method that combines
    /// [`get_num_for_name`](Self::get_num_for_name) and
    /// [`cache_lump_num`](Self::cache_lump_num). It looks up the lump
    /// index by name and then caches (or retrieves from cache) the data.
    ///
    /// # Parameters
    ///
    /// - `name`: The lump name to search for, case-insensitive, up to 8
    ///   characters.
    /// - `tag`: The [`PurgeTag`] controlling cache eviction behavior.
    ///
    /// # Returns
    ///
    /// A byte slice reference to the cached lump data.
    ///
    /// # C Implementation
    ///
    /// The original C was simply:
    /// ```c
    /// return W_CacheLumpNum(W_GetNumForName(name), tag);
    /// ```
    /// (`w_wad.c:512`)
    ///
    /// # Panics
    ///
    /// Implementations should panic if the lump name is not found, matching
    /// the original `I_Error()` abort behavior propagated through
    /// `W_GetNumForName`.
    ///
    /// # Mutation Note
    ///
    /// This method requires `&mut self` because it delegates to
    /// `cache_lump_num` which mutates the cache.
    fn cache_lump_name(&mut self, name: &str, tag: PurgeTag) -> &[u8];

    /// Return the total number of lumps loaded across all WAD files.
    ///
    /// Equivalent of `W_NumLumps` (`w_wad.c:339–342`) and the global
    /// `numlumps` variable (`w_wad.h:67`).
    ///
    /// This count includes lumps from all loaded WAD files (IWADs and
    /// PWADs). It is established during initialization
    /// (`W_InitMultipleFiles`) and remains constant for the lifetime of
    /// the provider (no dynamic loading/unloading during gameplay).
    ///
    /// # Returns
    ///
    /// The total lump count as `usize`. Valid lump indices range from
    /// `0` to `num_lumps() - 1`.
    fn num_lumps(&self) -> usize;
}
