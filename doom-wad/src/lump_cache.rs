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

//! Lump caching with tag-based eviction — translated from linuxdoom-1.10/w_wad.c
//! (W_CacheLumpNum/W_CacheLumpName) and z_zone.c/z_zone.h
//!
//! # Purpose
//!
//! This module implements the lump caching system that replaces the zone
//! allocator's `lumpcache` array from `w_wad.c` (lines 64, 309–316, 475–513)
//! and the `z_zone.c`/`z_zone.h` zone memory tagging system. It provides
//! [`LumpCache`], a safe `HashMap`-based data structure that offers
//! `W_CacheLumpNum` and `W_CacheLumpName` equivalents using Rust's standard
//! allocator instead of `Z_Malloc`/`Z_Free`/`Z_ChangeTag`.
//!
//! # Architecture (AAP §0.7.4)
//!
//! The original zone allocator managed a single `malloc`'d 6 MB heap block
//! with tagged allocations supporting three purge levels:
//!
//! - `PU_STATIC` (1) — Never freed automatically
//! - `PU_LEVEL` (50) — Freed at level change via `Z_FreeTags`
//! - `PU_CACHE` (101) — Purgable whenever memory pressure requires it
//!
//! In the Rust port, the zone allocator is **replaced** by:
//! - **Standard Rust allocator** for all general allocations (no 6 MB limit)
//! - **`LumpCache`** implementing tag-based eviction semantics for WAD lump data
//! - Cache entries stored in a `HashMap<usize, CachedLump>` where [`CachedLump`]
//!   tracks both the data bytes and the purge tag
//!
//! # Source Mapping
//!
//! | Rust Method | C Function | Location |
//! |---|---|---|
//! | [`LumpCache::new`] | `memset(lumpcache, 0, size)` | `w_wad.c:315` |
//! | [`LumpCache::cache_lump_num`] | `W_CacheLumpNum` | `w_wad.c:475–500` |
//! | [`LumpCache::cache_lump_name`] | `W_CacheLumpName` | `w_wad.c:507–513` |
//! | [`LumpCache::get_cached`] | *(no C equivalent)* | New convenience method |
//! | [`LumpCache::change_tag`] | `Z_ChangeTag` macro | `z_zone.h:72–77` |
//! | [`LumpCache::free_tags`] | `Z_FreeTags` | `z_zone.c:296–318` |
//! | [`LumpCache::purge_cache`] | Zone rover purge | `z_zone.c:225–239` |
//! | [`LumpCache::invalidate`] | `Z_Free(lumpcache[i])` | `w_wad.c:267–268` |
//! | [`LumpCache::clear`] | *(no C equivalent)* | Full cache reset |

use std::collections::HashMap;

use tracing::{debug, trace};

use crate::types::{CachedLump, PurgeTag};

// =============================================================================
// LumpCache — HashMap-based lump cache (replaces lumpcache[] + z_zone)
// =============================================================================

/// HashMap-based lump cache — replaces the `void** lumpcache` global array
/// from `w_wad.c:64` and the zone allocator's per-block tag tracking from
/// `z_zone.h`.
///
/// # Overview
///
/// The cache maps lump indices (`usize`) to [`CachedLump`] entries containing
/// the raw lump data bytes and a [`PurgeTag`] controlling eviction behavior.
/// This is a direct replacement for the original C engine's combination of:
///
/// - `lumpcache` — a `void**` array indexed by lump number (`w_wad.c:64`)
/// - `memblock_t.tag` — the zone allocator's per-allocation purge tag
///   (`z_zone.h:58–66`)
///
/// # Tag-Based Eviction Semantics
///
/// Tags faithfully replicate the `z_zone.h` purge tag definitions:
///
/// | Tag | Value | Purgable | Description |
/// |-----|-------|----------|-------------|
/// | `PU_STATIC` | 1 | No | Static for entire execution time |
/// | `PU_SOUND` | 2 | No | Static while sound effect is playing |
/// | `PU_MUSIC` | 3 | No | Static while music is playing |
/// | `PU_DAVE` | 4 | No | Miscellaneous static data |
/// | `PU_LEVEL` | 50 | No | Static until level exit |
/// | `PU_LEVSPEC` | 51 | No | Level-specific thinker data |
/// | `PU_PURGELEVEL` | 100 | Yes | Boundary: tags ≥ 100 are purgable |
/// | `PU_CACHE` | 101 | Yes | Purgable whenever needed |
///
/// # Safety
///
/// This implementation uses only safe Rust — no `unsafe` blocks, no raw
/// pointers, no custom allocator. All memory management is handled by
/// Rust's standard `Vec<u8>` and `HashMap` types.
///
/// # Example
///
/// ```
/// use doom_wad::lump_cache::LumpCache;
/// use doom_wad::types::PurgeTag;
///
/// let mut cache = LumpCache::new();
///
/// // Cache a lump with a read function
/// let data = cache.cache_lump_num(0, PurgeTag::Cache, |_lump| {
///     vec![0xDE, 0xAD, 0xBE, 0xEF]
/// });
/// assert_eq!(data, &[0xDE, 0xAD, 0xBE, 0xEF]);
///
/// // Subsequent access returns cached data without calling read_fn
/// let data2 = cache.cache_lump_num(0, PurgeTag::Static, |_| {
///     panic!("should not be called — data is cached");
/// });
/// assert_eq!(data2, &[0xDE, 0xAD, 0xBE, 0xEF]);
/// ```
pub struct LumpCache {
    /// Maps lump index → cached data + tag. Replaces the `void** lumpcache`
    /// global array from `w_wad.c:64` and the zone allocator's per-block
    /// tracking from `z_zone.h`.
    cache: HashMap<usize, CachedLump>,
}

impl LumpCache {
    // =========================================================================
    // Construction
    // =========================================================================

    /// Creates a new, empty lump cache.
    ///
    /// Equivalent of `memset(lumpcache, 0, size)` from `w_wad.c:315`, which
    /// zero-initialized the `lumpcache` pointer array after allocation in
    /// `W_InitMultipleFiles`.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    ///
    /// let cache = LumpCache::new();
    /// assert!(cache.get_cached(0).is_none());
    /// ```
    #[inline]
    pub fn new() -> Self {
        LumpCache {
            cache: HashMap::new(),
        }
    }

    // =========================================================================
    // Primary cache access
    // =========================================================================

    /// Retrieves lump data from the cache, reading from disk on a cache miss.
    ///
    /// This is the Rust equivalent of `W_CacheLumpNum` from `w_wad.c:475–500`.
    /// The original C logic was:
    ///
    /// ```c
    /// if (!lumpcache[lump]) {
    ///     // cache miss
    ///     ptr = Z_Malloc(W_LumpLength(lump), tag, &lumpcache[lump]);
    ///     W_ReadLump(lump, lumpcache[lump]);
    /// } else {
    ///     // cache hit — update tag
    ///     Z_ChangeTag(lumpcache[lump], tag);
    /// }
    /// return lumpcache[lump];
    /// ```
    ///
    /// # Parameters
    ///
    /// - `lump` — The lump index to retrieve (0-based, must be valid).
    /// - `tag` — The [`PurgeTag`] to assign to this cache entry. On a cache
    ///   hit, the existing entry's tag is updated to this value (equivalent to
    ///   `Z_ChangeTag` at `w_wad.c:496`).
    /// - `read_fn` — A closure that reads the lump data from disk when called
    ///   with the lump index. This replaces the tight coupling between
    ///   `W_CacheLumpNum` and `W_ReadLump` in the original C code.
    ///
    /// # Returns
    ///
    /// A byte slice referencing the cached lump data. The slice borrows from
    /// the cache and remains valid until the cache is mutably accessed.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    /// use doom_wad::types::PurgeTag;
    ///
    /// let mut cache = LumpCache::new();
    ///
    /// // First access triggers a read
    /// let data = cache.cache_lump_num(42, PurgeTag::Cache, |lump_idx| {
    ///     assert_eq!(lump_idx, 42);
    ///     vec![1, 2, 3, 4]
    /// });
    /// assert_eq!(data, &[1, 2, 3, 4]);
    /// ```
    pub fn cache_lump_num<F>(&mut self, lump: usize, tag: PurgeTag, read_fn: F) -> &[u8]
    where
        F: FnOnce(usize) -> Vec<u8>,
    {
        use std::collections::hash_map::Entry;

        // Use the entry API to handle both cache hit and miss in a single
        // lookup, as recommended by clippy::map_entry. This replaces the
        // original pattern of contains_key() + insert().
        let cached = match self.cache.entry(lump) {
            Entry::Occupied(mut occupied) => {
                // Cache hit — update the purge tag.
                // Equivalent to Z_ChangeTag(lumpcache[lump], tag) at w_wad.c:496.
                // The commented-out printf at w_wad.c:495 is replaced by this
                // debug event.
                debug!(lump, "cache hit on lump");
                occupied.get_mut().tag = tag;
                occupied.into_mut()
            }
            Entry::Vacant(vacant) => {
                // Cache miss — read lump data from disk and insert into cache.
                // Equivalent to Z_Malloc + W_ReadLump at w_wad.c:489–491.
                // The commented-out printf at w_wad.c:489 is replaced by this
                // debug event.
                debug!(lump, "cache miss on lump");
                let data = read_fn(lump);
                vacant.insert(CachedLump { data, tag })
            }
        };
        &cached.data
    }

    /// Retrieves lump data by name, looking up the lump index first.
    ///
    /// This is the Rust equivalent of `W_CacheLumpName` from `w_wad.c:507–513`:
    ///
    /// ```c
    /// return W_CacheLumpNum(W_GetNumForName(name), tag);
    /// ```
    ///
    /// # Parameters
    ///
    /// - `name` — The lump name to look up (case-insensitive, up to 8 chars).
    /// - `tag` — The [`PurgeTag`] to assign to the cache entry.
    /// - `lookup_fn` — A closure that resolves the lump name to a lump index.
    ///   This replaces the direct call to `W_GetNumForName(name)` in the
    ///   original C code.
    /// - `read_fn` — A closure that reads the lump data from disk (same as in
    ///   [`cache_lump_num`](LumpCache::cache_lump_num)).
    ///
    /// # Returns
    ///
    /// A byte slice referencing the cached lump data.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    /// use doom_wad::types::PurgeTag;
    ///
    /// let mut cache = LumpCache::new();
    ///
    /// let data = cache.cache_lump_name(
    ///     "PLAYPAL",
    ///     PurgeTag::Cache,
    ///     |name| {
    ///         assert_eq!(name, "PLAYPAL");
    ///         7 // pretend lump index
    ///     },
    ///     |lump_idx| {
    ///         assert_eq!(lump_idx, 7);
    ///         vec![0xFF; 768] // 256-color palette
    ///     },
    /// );
    /// assert_eq!(data.len(), 768);
    /// ```
    pub fn cache_lump_name<G, F>(
        &mut self,
        name: &str,
        tag: PurgeTag,
        lookup_fn: G,
        read_fn: F,
    ) -> &[u8]
    where
        G: FnOnce(&str) -> usize,
        F: FnOnce(usize) -> Vec<u8>,
    {
        let lump = lookup_fn(name);
        self.cache_lump_num(lump, tag, read_fn)
    }

    /// Peeks into the cache without modifying the entry's purge tag.
    ///
    /// Returns `Some(&[u8])` if the lump is currently cached, or `None` if
    /// it has not been loaded or has been evicted. Unlike
    /// [`cache_lump_num`](LumpCache::cache_lump_num), this method does not
    /// trigger a disk read on a cache miss and does not update the tag on a
    /// cache hit.
    ///
    /// # Parameters
    ///
    /// - `lump` — The lump index to look up.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    /// use doom_wad::types::PurgeTag;
    ///
    /// let mut cache = LumpCache::new();
    /// assert!(cache.get_cached(0).is_none());
    ///
    /// cache.cache_lump_num(0, PurgeTag::Static, |_| vec![42]);
    /// assert_eq!(cache.get_cached(0), Some([42].as_slice()));
    /// ```
    #[inline]
    pub fn get_cached(&self, lump: usize) -> Option<&[u8]> {
        self.cache.get(&lump).map(|entry| entry.data.as_slice())
    }

    // =========================================================================
    // Tag management
    // =========================================================================

    /// Updates the purge tag of a cached lump entry.
    ///
    /// Equivalent of `Z_ChangeTag(lumpcache[lump], tag)` — the
    /// `Z_ChangeTag` macro at `z_zone.h:72–77` which validated the block's
    /// zone ID and then called `Z_ChangeTag2(ptr, tag)` to update the
    /// `memblock_t.tag` field.
    ///
    /// If the lump is not in the cache, this method is a no-op (with a
    /// trace-level diagnostic).
    ///
    /// # Parameters
    ///
    /// - `lump` — The lump index whose tag should be updated.
    /// - `tag` — The new [`PurgeTag`] to assign.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    /// use doom_wad::types::PurgeTag;
    ///
    /// let mut cache = LumpCache::new();
    /// cache.cache_lump_num(5, PurgeTag::Cache, |_| vec![1, 2, 3]);
    ///
    /// // Promote to static so it won't be purged
    /// cache.change_tag(5, PurgeTag::Static);
    /// ```
    pub fn change_tag(&mut self, lump: usize, tag: PurgeTag) {
        if let Some(entry) = self.cache.get_mut(&lump) {
            trace!(lump, old_tag = %entry.tag, new_tag = %tag, "changing lump tag");
            entry.tag = tag;
        } else {
            trace!(lump, "change_tag called for uncached lump — no-op");
        }
    }

    // =========================================================================
    // Eviction
    // =========================================================================

    /// Removes all cache entries whose tag falls within the inclusive range
    /// `[low, high]`.
    ///
    /// This is the Rust equivalent of `Z_FreeTags(lowtag, hightag)` from
    /// `z_zone.c:296–318`. The original C implementation iterated the zone's
    /// block linked list and called `Z_Free` on every block with
    /// `tag >= lowtag && tag <= hightag`.
    ///
    /// # Typical Usage
    ///
    /// At level change, the engine calls:
    /// ```c
    /// Z_FreeTags(PU_LEVEL, PU_PURGELEVEL - 1);
    /// ```
    /// to free all level-scoped data. In Rust:
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    /// use doom_wad::types::PurgeTag;
    ///
    /// let mut cache = LumpCache::new();
    /// cache.cache_lump_num(0, PurgeTag::Level, |_| vec![1]);
    /// cache.cache_lump_num(1, PurgeTag::LevSpec, |_| vec![2]);
    /// cache.cache_lump_num(2, PurgeTag::Static, |_| vec![3]);
    ///
    /// cache.free_tags(PurgeTag::Level, PurgeTag::LevSpec);
    ///
    /// assert!(cache.get_cached(0).is_none()); // PU_LEVEL freed
    /// assert!(cache.get_cached(1).is_none()); // PU_LEVSPEC freed
    /// assert!(cache.get_cached(2).is_some()); // PU_STATIC preserved
    /// ```
    ///
    /// # Parameters
    ///
    /// - `low` — Lower bound of the tag range (inclusive).
    /// - `high` — Upper bound of the tag range (inclusive).
    pub fn free_tags(&mut self, low: PurgeTag, high: PurgeTag) {
        let before = self.cache.len();
        self.cache.retain(|_lump, entry| {
            // Keep entries whose tag is outside the [low, high] range.
            // Mirrors z_zone.c:315: `if (block->tag >= lowtag && block->tag <= hightag)`
            !(entry.tag >= low && entry.tag <= high)
        });
        let removed = before - self.cache.len();
        trace!(
            low = %low,
            high = %high,
            removed,
            remaining = self.cache.len(),
            "freed tags in range"
        );
    }

    /// Removes all purgable cache entries (tag ≥ `PU_PURGELEVEL`).
    ///
    /// This replaces the zone allocator's automatic purging behavior under
    /// memory pressure. In the original C engine, `Z_Malloc` would
    /// automatically free purgable blocks (tag ≥ `PU_PURGELEVEL = 100`) when
    /// the zone heap ran out of space (`z_zone.c:225–239`).
    ///
    /// The eviction rule from `z_zone.h:35–44`:
    /// - Tags < 100 (`PU_STATIC`=1, `PU_SOUND`=2, `PU_MUSIC`=3, `PU_DAVE`=4,
    ///   `PU_LEVEL`=50, `PU_LEVSPEC`=51): **NOT** purgable
    /// - Tags ≥ 100 (`PU_PURGELEVEL`=100, `PU_CACHE`=101): **Purgable**
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    /// use doom_wad::types::PurgeTag;
    ///
    /// let mut cache = LumpCache::new();
    /// cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
    /// cache.cache_lump_num(1, PurgeTag::Cache, |_| vec![2]);
    /// cache.cache_lump_num(2, PurgeTag::PurgeLevel, |_| vec![3]);
    ///
    /// cache.purge_cache();
    ///
    /// assert!(cache.get_cached(0).is_some());  // PU_STATIC kept
    /// assert!(cache.get_cached(1).is_none());  // PU_CACHE purged
    /// assert!(cache.get_cached(2).is_none());  // PU_PURGELEVEL purged
    /// ```
    pub fn purge_cache(&mut self) {
        let before = self.cache.len();
        self.cache.retain(|_lump, entry| {
            // Keep entries that are NOT purgable.
            // PurgeTag::is_purgable() returns true for tags >= PU_PURGELEVEL (100).
            !entry.tag.is_purgable()
        });
        let removed = before - self.cache.len();
        trace!(
            threshold = %PurgeTag::PurgeLevel,
            removed,
            remaining = self.cache.len(),
            "purged purgable cache entries"
        );
    }

    /// Removes a specific entry from the cache.
    ///
    /// Equivalent of `Z_Free(lumpcache[i])` as used in `W_Reload`
    /// (`w_wad.c:267–268`):
    ///
    /// ```c
    /// if (lumpcache[i])
    ///     Z_Free(lumpcache[i]);
    /// ```
    ///
    /// If the lump is not in the cache, this method is a no-op.
    ///
    /// # Parameters
    ///
    /// - `lump` — The lump index to remove from the cache.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    /// use doom_wad::types::PurgeTag;
    ///
    /// let mut cache = LumpCache::new();
    /// cache.cache_lump_num(10, PurgeTag::Static, |_| vec![0xAB]);
    /// assert!(cache.get_cached(10).is_some());
    ///
    /// cache.invalidate(10);
    /// assert!(cache.get_cached(10).is_none());
    /// ```
    pub fn invalidate(&mut self, lump: usize) {
        if self.cache.remove(&lump).is_some() {
            trace!(lump, "invalidated cached lump");
        } else {
            trace!(lump, "invalidate called for uncached lump — no-op");
        }
    }

    /// Removes ALL entries from the cache — full reset.
    ///
    /// This is a more aggressive version of [`purge_cache`](LumpCache::purge_cache)
    /// that removes every entry regardless of its purge tag, including
    /// `PU_STATIC` entries.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_wad::lump_cache::LumpCache;
    /// use doom_wad::types::PurgeTag;
    ///
    /// let mut cache = LumpCache::new();
    /// cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
    /// cache.cache_lump_num(1, PurgeTag::Cache, |_| vec![2]);
    ///
    /// cache.clear();
    ///
    /// assert!(cache.get_cached(0).is_none());
    /// assert!(cache.get_cached(1).is_none());
    /// ```
    pub fn clear(&mut self) {
        let count = self.cache.len();
        self.cache.clear();
        trace!(removed = count, "cleared entire lump cache");
    }
}

impl Default for LumpCache {
    /// Creates a new, empty lump cache — identical to [`LumpCache::new`].
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
    use crate::types::PurgeTag;

    // -------------------------------------------------------------------------
    // Construction tests
    // -------------------------------------------------------------------------

    #[test]
    fn new_cache_is_empty() {
        let cache = LumpCache::new();
        assert!(cache.get_cached(0).is_none());
        assert!(cache.get_cached(100).is_none());
    }

    #[test]
    fn default_cache_is_empty() {
        let cache = LumpCache::default();
        assert!(cache.get_cached(0).is_none());
    }

    // -------------------------------------------------------------------------
    // cache_lump_num tests
    // -------------------------------------------------------------------------

    #[test]
    fn cache_miss_calls_read_fn() {
        let mut cache = LumpCache::new();
        let mut called = false;

        let data = cache.cache_lump_num(5, PurgeTag::Cache, |lump| {
            assert_eq!(lump, 5);
            called = true;
            vec![0xDE, 0xAD]
        });

        assert!(called);
        assert_eq!(data, &[0xDE, 0xAD]);
    }

    #[test]
    fn cache_hit_does_not_call_read_fn() {
        let mut cache = LumpCache::new();

        // First access — miss
        cache.cache_lump_num(3, PurgeTag::Cache, |_| vec![1, 2, 3]);

        // Second access — hit (read_fn should not be called)
        let data = cache.cache_lump_num(3, PurgeTag::Static, |_| {
            panic!("read_fn should not be called on cache hit");
        });

        assert_eq!(data, &[1, 2, 3]);
    }

    #[test]
    fn cache_hit_updates_tag() {
        let mut cache = LumpCache::new();

        // Insert with PU_CACHE
        cache.cache_lump_num(7, PurgeTag::Cache, |_| vec![42]);

        // Re-access with PU_STATIC — tag should be updated
        cache.cache_lump_num(7, PurgeTag::Static, |_| {
            panic!("should not read");
        });

        // Purge purgable entries — lump 7 should survive because tag is now Static
        cache.purge_cache();
        assert!(cache.get_cached(7).is_some());
    }

    #[test]
    fn cache_multiple_lumps_independently() {
        let mut cache = LumpCache::new();

        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![10]);
        cache.cache_lump_num(1, PurgeTag::Level, |_| vec![20]);
        cache.cache_lump_num(2, PurgeTag::Cache, |_| vec![30]);

        assert_eq!(cache.get_cached(0), Some([10].as_slice()));
        assert_eq!(cache.get_cached(1), Some([20].as_slice()));
        assert_eq!(cache.get_cached(2), Some([30].as_slice()));
    }

    // -------------------------------------------------------------------------
    // cache_lump_name tests
    // -------------------------------------------------------------------------

    #[test]
    fn cache_lump_name_delegates_to_cache_lump_num() {
        let mut cache = LumpCache::new();

        let data = cache.cache_lump_name(
            "PLAYPAL",
            PurgeTag::Cache,
            |name| {
                assert_eq!(name, "PLAYPAL");
                42
            },
            |lump| {
                assert_eq!(lump, 42);
                vec![0xFF; 768]
            },
        );

        assert_eq!(data.len(), 768);
        assert_eq!(cache.get_cached(42), Some(vec![0xFF; 768].as_slice()));
    }

    #[test]
    fn cache_lump_name_uses_cached_data_on_hit() {
        let mut cache = LumpCache::new();

        // Pre-populate via cache_lump_num
        cache.cache_lump_num(10, PurgeTag::Cache, |_| vec![1, 2, 3]);

        // Access by name — should hit the cache
        let data = cache.cache_lump_name(
            "DEMO1",
            PurgeTag::Static,
            |_| 10,
            |_| panic!("should not read on hit"),
        );

        assert_eq!(data, &[1, 2, 3]);
    }

    // -------------------------------------------------------------------------
    // get_cached tests
    // -------------------------------------------------------------------------

    #[test]
    fn get_cached_returns_none_for_uncached() {
        let cache = LumpCache::new();
        assert!(cache.get_cached(999).is_none());
    }

    #[test]
    fn get_cached_returns_data_for_cached() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![100, 200]);
        assert_eq!(cache.get_cached(0), Some([100, 200].as_slice()));
    }

    // -------------------------------------------------------------------------
    // change_tag tests
    // -------------------------------------------------------------------------

    #[test]
    fn change_tag_updates_existing_entry() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(5, PurgeTag::Cache, |_| vec![1]);

        // Change tag to Static
        cache.change_tag(5, PurgeTag::Static);

        // Entry should survive a purge
        cache.purge_cache();
        assert!(cache.get_cached(5).is_some());
    }

    #[test]
    fn change_tag_noop_for_uncached() {
        let mut cache = LumpCache::new();
        // Should not panic
        cache.change_tag(999, PurgeTag::Static);
    }

    #[test]
    fn change_tag_to_purgable_allows_eviction() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(5, PurgeTag::Static, |_| vec![1]);

        // Promote to purgable
        cache.change_tag(5, PurgeTag::Cache);

        // Should be evicted by purge
        cache.purge_cache();
        assert!(cache.get_cached(5).is_none());
    }

    // -------------------------------------------------------------------------
    // free_tags tests
    // -------------------------------------------------------------------------

    #[test]
    fn free_tags_removes_entries_in_range() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
        cache.cache_lump_num(1, PurgeTag::Level, |_| vec![2]);
        cache.cache_lump_num(2, PurgeTag::LevSpec, |_| vec![3]);
        cache.cache_lump_num(3, PurgeTag::Cache, |_| vec![4]);

        // Free PU_LEVEL..PU_LEVSPEC range
        cache.free_tags(PurgeTag::Level, PurgeTag::LevSpec);

        assert!(cache.get_cached(0).is_some()); // PU_STATIC — outside range
        assert!(cache.get_cached(1).is_none()); // PU_LEVEL — in range
        assert!(cache.get_cached(2).is_none()); // PU_LEVSPEC — in range
        assert!(cache.get_cached(3).is_some()); // PU_CACHE — outside range
    }

    #[test]
    fn free_tags_with_single_tag_range() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(0, PurgeTag::Level, |_| vec![1]);
        cache.cache_lump_num(1, PurgeTag::Level, |_| vec![2]);
        cache.cache_lump_num(2, PurgeTag::Static, |_| vec![3]);

        cache.free_tags(PurgeTag::Level, PurgeTag::Level);

        assert!(cache.get_cached(0).is_none());
        assert!(cache.get_cached(1).is_none());
        assert!(cache.get_cached(2).is_some());
    }

    #[test]
    fn free_tags_empty_cache_is_noop() {
        let mut cache = LumpCache::new();
        // Should not panic on empty cache
        cache.free_tags(PurgeTag::Level, PurgeTag::Cache);
    }

    #[test]
    fn free_tags_all_tags_clears_everything() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
        cache.cache_lump_num(1, PurgeTag::Level, |_| vec![2]);
        cache.cache_lump_num(2, PurgeTag::Cache, |_| vec![3]);

        // Free the entire range from Static to Cache
        cache.free_tags(PurgeTag::Static, PurgeTag::Cache);

        assert!(cache.get_cached(0).is_none());
        assert!(cache.get_cached(1).is_none());
        assert!(cache.get_cached(2).is_none());
    }

    // -------------------------------------------------------------------------
    // purge_cache tests
    // -------------------------------------------------------------------------

    #[test]
    fn purge_cache_removes_purgable_entries() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
        cache.cache_lump_num(1, PurgeTag::Sound, |_| vec![2]);
        cache.cache_lump_num(2, PurgeTag::Level, |_| vec![3]);
        cache.cache_lump_num(3, PurgeTag::PurgeLevel, |_| vec![4]);
        cache.cache_lump_num(4, PurgeTag::Cache, |_| vec![5]);

        cache.purge_cache();

        // Non-purgable entries preserved
        assert!(cache.get_cached(0).is_some()); // PU_STATIC (1) — kept
        assert!(cache.get_cached(1).is_some()); // PU_SOUND (2) — kept
        assert!(cache.get_cached(2).is_some()); // PU_LEVEL (50) — kept

        // Purgable entries removed
        assert!(cache.get_cached(3).is_none()); // PU_PURGELEVEL (100) — purged
        assert!(cache.get_cached(4).is_none()); // PU_CACHE (101) — purged
    }

    #[test]
    fn purge_cache_empty_is_noop() {
        let mut cache = LumpCache::new();
        cache.purge_cache(); // Should not panic
    }

    #[test]
    fn purge_cache_all_static_preserves_all() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
        cache.cache_lump_num(1, PurgeTag::Static, |_| vec![2]);

        cache.purge_cache();

        assert!(cache.get_cached(0).is_some());
        assert!(cache.get_cached(1).is_some());
    }

    // -------------------------------------------------------------------------
    // invalidate tests
    // -------------------------------------------------------------------------

    #[test]
    fn invalidate_removes_specific_entry() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
        cache.cache_lump_num(1, PurgeTag::Static, |_| vec![2]);

        cache.invalidate(0);

        assert!(cache.get_cached(0).is_none());
        assert!(cache.get_cached(1).is_some());
    }

    #[test]
    fn invalidate_uncached_is_noop() {
        let mut cache = LumpCache::new();
        cache.invalidate(42); // Should not panic
    }

    #[test]
    fn invalidate_allows_re_caching() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(5, PurgeTag::Static, |_| vec![1, 2, 3]);

        cache.invalidate(5);
        assert!(cache.get_cached(5).is_none());

        // Re-cache with different data
        let data = cache.cache_lump_num(5, PurgeTag::Cache, |_| vec![4, 5, 6]);
        assert_eq!(data, &[4, 5, 6]);
    }

    // -------------------------------------------------------------------------
    // clear tests
    // -------------------------------------------------------------------------

    #[test]
    fn clear_removes_all_entries() {
        let mut cache = LumpCache::new();
        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
        cache.cache_lump_num(1, PurgeTag::Level, |_| vec![2]);
        cache.cache_lump_num(2, PurgeTag::Cache, |_| vec![3]);

        cache.clear();

        assert!(cache.get_cached(0).is_none());
        assert!(cache.get_cached(1).is_none());
        assert!(cache.get_cached(2).is_none());
    }

    #[test]
    fn clear_empty_is_noop() {
        let mut cache = LumpCache::new();
        cache.clear(); // Should not panic
    }

    // -------------------------------------------------------------------------
    // Integration / workflow tests
    // -------------------------------------------------------------------------

    #[test]
    fn level_change_workflow() {
        // Simulates a typical level change scenario:
        // 1. Load level data with PU_LEVEL tag
        // 2. Load textures with PU_CACHE tag
        // 3. Load static data with PU_STATIC tag
        // 4. On level change, free PU_LEVEL..PU_LEVSPEC
        // 5. Static and cache data should survive

        let mut cache = LumpCache::new();

        // Level-scoped data
        cache.cache_lump_num(100, PurgeTag::Level, |_| vec![0xAA; 1024]);
        cache.cache_lump_num(101, PurgeTag::LevSpec, |_| vec![0xBB; 512]);

        // Cached textures
        cache.cache_lump_num(200, PurgeTag::Cache, |_| vec![0xCC; 4096]);

        // Static data (palette, colormap)
        cache.cache_lump_num(300, PurgeTag::Static, |_| vec![0xDD; 768]);

        // Level change — free level data
        cache.free_tags(PurgeTag::Level, PurgeTag::LevSpec);

        // Level data freed
        assert!(cache.get_cached(100).is_none());
        assert!(cache.get_cached(101).is_none());

        // Cache and static data survive
        assert!(cache.get_cached(200).is_some());
        assert!(cache.get_cached(300).is_some());
    }

    #[test]
    fn memory_pressure_workflow() {
        // Simulates memory pressure: purge all purgable entries,
        // then re-cache on next access.

        let mut cache = LumpCache::new();

        cache.cache_lump_num(0, PurgeTag::Static, |_| vec![1]);
        cache.cache_lump_num(1, PurgeTag::Cache, |_| vec![2]);
        cache.cache_lump_num(2, PurgeTag::Cache, |_| vec![3]);

        // Memory pressure — purge
        cache.purge_cache();

        assert!(cache.get_cached(0).is_some()); // Static survives
        assert!(cache.get_cached(1).is_none()); // Cache purged
        assert!(cache.get_cached(2).is_none()); // Cache purged

        // Re-cache on next access (triggers read_fn)
        let data = cache.cache_lump_num(1, PurgeTag::Cache, |_| vec![20]);
        assert_eq!(data, &[20]);
    }

    #[test]
    fn reload_workflow() {
        // Simulates W_Reload: invalidate specific lumps so they are re-read
        // from disk on next access.

        let mut cache = LumpCache::new();

        cache.cache_lump_num(50, PurgeTag::Level, |_| vec![0xAA]);
        cache.cache_lump_num(51, PurgeTag::Level, |_| vec![0xBB]);

        // Reload — invalidate reloadable lumps (w_wad.c:267-268)
        cache.invalidate(50);
        cache.invalidate(51);

        // Next access reads fresh data
        let data = cache.cache_lump_num(50, PurgeTag::Level, |_| vec![0xCC]);
        assert_eq!(data, &[0xCC]);
    }

    #[test]
    fn empty_lump_data() {
        // Edge case: lump with zero-length data
        let mut cache = LumpCache::new();
        let data = cache.cache_lump_num(0, PurgeTag::Static, |_| vec![]);
        assert!(data.is_empty());
        assert_eq!(cache.get_cached(0), Some([].as_slice()));
    }

    #[test]
    fn large_lump_indices() {
        // Edge case: very large lump indices
        let mut cache = LumpCache::new();
        let idx = usize::MAX - 1;
        cache.cache_lump_num(idx, PurgeTag::Cache, |_| vec![42]);
        assert_eq!(cache.get_cached(idx), Some([42].as_slice()));
    }
}
