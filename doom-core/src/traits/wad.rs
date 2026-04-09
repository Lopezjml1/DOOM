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

//! Re-export of the [`WadProvider`] trait from the `doom-wad` crate.
//!
//! This module bridges the WAD access contract into the `doom-core::traits`
//! namespace, allowing game logic modules to depend on a single traits module
//! for all platform abstractions.
//!
//! The `WadProvider` trait provides the WAD file access contract:
//! lump lookup by name, lump reading, and tag-based lump caching.
//!
//! ## Original C Interface
//!
//! Translated from `linuxdoom-1.10/w_wad.h` (lines 69-79):
//! - `W_CheckNumForName` → `WadProvider::check_num_for_name`
//! - `W_GetNumForName` → `WadProvider::get_num_for_name`
//! - `W_LumpLength` → `WadProvider::lump_length`
//! - `W_ReadLump` → `WadProvider::read_lump`
//! - `W_CacheLumpNum` → `WadProvider::cache_lump_num`
//! - `W_CacheLumpName` → `WadProvider::cache_lump_name`
//!
//! ## Import Pattern
//!
//! Old C: `#include "w_wad.h"` / `W_CacheLumpName("PLAYPAL", PU_CACHE)`
//! New Rust: `use doom_core::traits::WadProvider;` / `wad.cache_lump_name("PLAYPAL", PurgeTag::Cache)`

pub use doom_wad::WadProvider;
