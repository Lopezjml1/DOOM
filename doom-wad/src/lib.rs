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

//! # doom-wad — WAD/IWAD File Format Parser and Lump Cache
//!
//! This crate provides the WAD file loading, lump directory management,
//! and cached lump access for the DOOM engine. It is translated from
//! `linuxdoom-1.10/w_wad.c` and `linuxdoom-1.10/w_wad.h`.
//!
//! The crate has no dependencies on any other `doom-*` crate and no
//! platform-specific dependencies, making it the foundational data layer.
//!
//! ## Key Features
//! - IWAD and PWAD file loading with lump directory construction
//! - Backward-scan lump lookup (later WADs override earlier ones)
//! - Tag-based lump caching replacing the original zone memory allocator
//! - Single-lump file support for non-WAD data files
//! - Hot-reload support for development workflows
//!
//! ## Cross-Crate Import Patterns
//!
//! Old C: `#include "w_wad.h"` / `W_CacheLumpName("PLAYPAL", PU_CACHE)`
//! New Rust: `use doom_wad::WadProvider;` / `wad.cache_lump_name("PLAYPAL", PurgeTag::Cache)`

pub mod lump_cache;
pub mod types;
pub mod wad_provider;

// Re-export key public types at crate root for ergonomic access.
// Downstream crates can write `use doom_wad::WadType;` instead of
// `use doom_wad::types::WadType;`.
pub use lump_cache::LumpCache;
pub use types::{CachedLump, FileLump, LumpInfo, LumpNum, PurgeTag, WadError, WadInfo, WadType};
pub use wad_provider::WadProvider;
