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

//! Utility modules for the DOOM engine.
//!
//! General-purpose utility functions used across the engine:
//! - [`argv`] — Command-line argument parsing (from `m_argv.c/h`)
//! - [`bbox`] — Bounding box operations (from `m_bbox.c/h`)
//! - [`cheat`] — Cheat code sequence detection (from `m_cheat.c/h`)
//! - [`misc`] — File I/O, config defaults, screenshots (from `m_misc.c/h`)
//! - [`random`] — Deterministic PRNG (from `m_random.c/h`)
//! - [`swap`] — Endian byte-swap utilities (from `m_swap.c/h`)
//!
//! Translated from the `m_*.c/h` utility files in `linuxdoom-1.10/`.

// ---------------------------------------------------------------------------
// Child module declarations (alphabetical order)
// ---------------------------------------------------------------------------

pub mod argv;
pub mod bbox;
pub mod cheat;
pub mod misc;
pub mod random;
pub mod swap;

// ---------------------------------------------------------------------------
// Convenience re-exports for commonly-used types and functions
// ---------------------------------------------------------------------------
// These re-exports allow ergonomic access from other doom-core modules and
// downstream crates. For example, `use doom_core::util::Args;` instead of
// `use doom_core::util::argv::Args;`.

pub use argv::Args;
pub use bbox::{add_to_box, clear_box, BBox, BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};
pub use cheat::CheatSeq;
pub use misc::{read_file, write_file, ConfigDefaults};
pub use random::{DoomRandom, RNDTABLE};
pub use swap::{long, short};
