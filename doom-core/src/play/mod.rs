// DOOM Rust Port — Copyright (C) 1993-1996 id Software, Inc.
// Copyright (C) 2024 Rust DOOM Contributors
//
// This program is free software; you can redistribute it and/or
// modify it under the terms of the GNU General Public License
// as published by the Free Software Foundation; either version 2
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

//! Translated from linuxdoom-1.10/p_local.h — Play functions, animation, global header.
//!
//! This module serves as the root of the play subsystem, consolidating all
//! gameplay-related submodules and defining shared constants used across the
//! play layer. It corresponds to the C `p_local.h` umbrella header that every
//! `p_*.c` file includes, plus selected constants from `p_spec.h` that are
//! referenced cross-module by the sector-mover thinkers (ceilings, doors,
//! floors, platforms, lights, and switches).
//!
//! # Submodule Organisation
//!
//! The 20 child modules map 1-to-1 to the original `p_*.c` translation units:
//!
//! | Module      | C Source      | Responsibility                              |
//! |-------------|---------------|---------------------------------------------|
//! | `tick`      | `p_tick.c`    | Thinker loop driver, per-tic heartbeat      |
//! | `mobj`      | `p_mobj.c`   | Map object lifecycle, missile/effect spawn   |
//! | `movement`  | `p_map.c`    | Movement physics, collision callbacks        |
//! | `map`       | `p_map.c`    | Hitscan/use-line/radius attacks              |
//! | `maputl`    | `p_maputl.c` | Map geometry, blockmap, intercepts           |
//! | `user`      | `p_user.c`   | Player input → movement/view processing      |
//! | `pspr`      | `p_pspr.c`   | Weapon sprite logic, fire/refire             |
//! | `inter`     | `p_inter.c`  | Pickups, damage, kills                       |
//! | `enemy`     | `p_enemy.c`  | Monster AI (chase, look, boss specials)      |
//! | `sight`     | `p_sight.c`  | Line-of-sight via BSP + reject table         |
//! | `spec`      | `p_spec.c`   | Animated textures, trigger dispatch          |
//! | `ceilng`    | `p_ceilng.c` | Ceiling movement thinkers                    |
//! | `doors`     | `p_doors.c`  | Door open/close thinkers                     |
//! | `floor`     | `p_floor.c`  | Floor movement, stair builders               |
//! | `lights`    | `p_lights.c` | Light flicker/strobe/glow effects            |
//! | `plats`     | `p_plats.c`  | Platform lift thinkers                       |
//! | `switch`    | `p_switch.c` | Wall switch texture changes                  |
//! | `telept`    | `p_telept.c` | Teleport specials                            |
//! | `setup`     | `p_setup.c`  | Level loading from WAD lumps                 |
//! | `saveg`     | `p_saveg.c`  | Save/load game serialisation                 |

// ---------------------------------------------------------------------------
// Child module declarations (20 submodules)
// ---------------------------------------------------------------------------

pub mod ceilng;
pub mod doors;
pub mod enemy;
pub mod floor;
pub mod inter;
pub mod lights;
pub mod map;
pub mod maputl;
pub mod mobj;
pub mod movement;
pub mod plats;
pub mod pspr;
pub mod saveg;
pub mod setup;
pub mod sight;
pub mod spec;
pub mod switch;
pub mod telept;
pub mod tick;
pub mod user;

// ---------------------------------------------------------------------------
// Imports — fixed-point types used to define play-subsystem constants
// ---------------------------------------------------------------------------

use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};

// ============================================================================
// Play-specific constants — translated from p_local.h lines 30–61
// ============================================================================

/// Movement speed for floating monsters (4 map units per tic in fixed-point).
///
/// Original C: `#define FLOATSPEED (FRACUNIT*4)`
pub const FLOATSPEED: Fixed = Fixed(FRACUNIT * 4);

/// Maximum default player health.
///
/// Original C: `#define MAXHEALTH 100`
pub const MAXHEALTH: i32 = 100;

/// Default player view height above the floor (41 map units in fixed-point).
///
/// Original C: `#define VIEWHEIGHT (41*FRACUNIT)`
pub const VIEWHEIGHT: Fixed = Fixed(41 * FRACUNIT);

// -- Blockmap constants -----------------------------------------------------

/// Blockmap unit size in map units.
///
/// Original C: `#define MAPBLOCKUNITS 128`
pub const MAPBLOCKUNITS: i32 = 128;

/// Blockmap cell size in fixed-point (128 × FRACUNIT).
///
/// Original C: `#define MAPBLOCKSIZE (MAPBLOCKUNITS*FRACUNIT)`
pub const MAPBLOCKSIZE: Fixed = Fixed(MAPBLOCKUNITS * FRACUNIT);

/// Bit shift for converting world coordinates to blockmap indices.
/// Equals `FRACBITS + 7` = 23.
///
/// Original C: `#define MAPBLOCKSHIFT (FRACBITS+7)`
pub const MAPBLOCKSHIFT: i32 = FRACBITS + 7;

/// Bitmask for the fractional part within a blockmap cell.
///
/// Original C: `#define MAPBMASK (MAPBLOCKSIZE-1)`
pub const MAPBMASK: i32 = MAPBLOCKSIZE.0 - 1;

/// Shift distance from blockmap-relative to fixed-point fractional.
/// Equals `MAPBLOCKSHIFT - FRACBITS` = 7.
///
/// Original C: `#define MAPBTOFRAC (MAPBLOCKSHIFT-FRACBITS)`
pub const MAPBTOFRAC: i32 = MAPBLOCKSHIFT - FRACBITS;

// -- Movement constants -----------------------------------------------------

/// Player collision radius (16 map units in fixed-point).
///
/// Original C: `#define PLAYERRADIUS 16*FRACUNIT`
pub const PLAYERRADIUS: Fixed = Fixed(16 * FRACUNIT);

/// Maximum radius used for pre-calculated sector blockmap boxes (32 map units).
///
/// Original C: `#define MAXRADIUS 32*FRACUNIT`
pub const MAXRADIUS: Fixed = Fixed(32 * FRACUNIT);

/// Gravity acceleration applied each tic (1.0 in fixed-point).
///
/// Original C: `#define GRAVITY FRACUNIT`
pub const GRAVITY: Fixed = Fixed(FRACUNIT);

/// Maximum horizontal movement per tic (30 map units in fixed-point).
///
/// Original C: `#define MAXMOVE (30*FRACUNIT)`
pub const MAXMOVE: Fixed = Fixed(30 * FRACUNIT);

// -- Range constants --------------------------------------------------------

/// Maximum distance for use-line activation (64 map units).
///
/// Original C: `#define USERANGE (64*FRACUNIT)`
pub const USERANGE: Fixed = Fixed(64 * FRACUNIT);

/// Maximum distance for melee attacks (64 map units).
///
/// Original C: `#define MELEERANGE (64*FRACUNIT)`
pub const MELEERANGE: Fixed = Fixed(64 * FRACUNIT);

/// Maximum distance for missile/hitscan attacks (32×64 = 2048 map units).
///
/// Original C: `#define MISSILERANGE (32*64*FRACUNIT)`
pub const MISSILERANGE: Fixed = Fixed(32 * 64 * FRACUNIT);

/// Monster target-follow threshold. After this many tics without seeing its
/// target, a monster may switch to a new target.
///
/// Original C: `#define BASETHRESHOLD 100`
pub const BASETHRESHOLD: i32 = 100;

// ============================================================================
// Map-object z-coordinate sentinels — from p_local.h
// ============================================================================

/// Sentinel z-coordinate: spawn the object on the floor.
/// Uses `i32::MIN` as the Rust equivalent of C's `MININT`.
///
/// Original C: `#define ONFLOORZ MININT`
pub const ONFLOORZ: Fixed = Fixed(i32::MIN);

/// Sentinel z-coordinate: spawn the object on the ceiling.
/// Uses `i32::MAX` as the Rust equivalent of C's `MAXINT`.
///
/// Original C: `#define ONCEILINGZ MAXINT`
pub const ONCEILINGZ: Fixed = Fixed(i32::MAX);

/// Size of the item-respawn queue (Nightmare difficulty).
///
/// Original C: `#define ITEMQUESIZE 128`
pub const ITEMQUESIZE: usize = 128;

// ============================================================================
// Path-traverse flags — from p_local.h
// ============================================================================

/// Add lines to the intercept list during `P_PathTraverse`.
///
/// Original C: `#define PT_ADDLINES 1`
pub const PT_ADDLINES: i32 = 1;

/// Add things (map objects) to the intercept list during `P_PathTraverse`.
///
/// Original C: `#define PT_ADDTHINGS 2`
pub const PT_ADDTHINGS: i32 = 2;

/// Allow early exit from `P_PathTraverse` when a blocking intercept is found.
///
/// Original C: `#define PT_EARLYOUT 4`
pub const PT_EARLYOUT: i32 = 4;

// ============================================================================
// Intercept limit — from p_local.h
// ============================================================================

/// Maximum number of intercept entries during a single path traversal.
///
/// Original C: `#define MAXINTERCEPTS 128`
pub const MAXINTERCEPTS: usize = 128;

// ============================================================================
// Sector-mover speed constants — from p_spec.h
//
// These are consolidated here because they are referenced by multiple
// submodules (ceilng, doors, floor, plats, lights, switch, spec).
// ============================================================================

/// Ceiling movement speed (1.0 in fixed-point).
///
/// Original C (p_spec.h): `#define CEILSPEED FRACUNIT`
pub const CEILSPEED: Fixed = Fixed(FRACUNIT);

/// Vertical door movement speed (2.0 in fixed-point).
///
/// Original C (p_spec.h): `#define VDOORSPEED (FRACUNIT*2)`
pub const VDOORSPEED: Fixed = Fixed(FRACUNIT * 2);

/// Vertical door wait time before closing (150 tics ≈ 4.3 seconds).
///
/// Original C (p_spec.h): `#define VDOORWAIT 150`
pub const VDOORWAIT: i32 = 150;

/// Floor movement speed (1.0 in fixed-point).
///
/// Original C (p_spec.h): `#define FLOORSPEED FRACUNIT`
pub const FLOORSPEED: Fixed = Fixed(FRACUNIT);

/// Platform (lift) movement speed (1.0 in fixed-point).
///
/// Original C (p_spec.h): `#define PLATSPEED FRACUNIT`
pub const PLATSPEED: Fixed = Fixed(FRACUNIT);

/// Platform wait time in seconds (the engine multiplies by TICRATE at runtime).
///
/// Original C (p_spec.h): `#define PLATWAIT 3`
pub const PLATWAIT: i32 = 3;

/// Strobe-bright duration in tics.
///
/// Original C (p_spec.h): `#define STROBEBRIGHT 5`
pub const STROBEBRIGHT: i32 = 5;

/// Slow-strobe dark duration in tics (1 second at 35 Hz).
///
/// Original C (p_spec.h): `#define SLOWDARK 35`
pub const SLOWDARK: i32 = 35;

/// Fast-strobe dark duration in tics.
///
/// Original C (p_spec.h): `#define FASTDARK 15`
pub const FASTDARK: i32 = 15;

/// Glow light-level change per tic.
///
/// Original C (p_spec.h): `#define GLOWSPEED 8`
pub const GLOWSPEED: i32 = 8;

/// Maximum number of simultaneously-active ceiling movers.
///
/// Original C (p_spec.h): `#define MAXCEILINGS 30`
pub const MAXCEILINGS: usize = 30;

/// Maximum number of simultaneously-active platform lifts.
///
/// Original C (p_spec.h): `#define MAXPLATS 30`
pub const MAXPLATS: usize = 30;

/// Maximum number of simultaneously-tracked button (switch revert) timers.
///
/// Original C (p_spec.h): `#define MAXBUTTONS 16`
pub const MAXBUTTONS: usize = 16;

/// Duration before a switch texture reverts (35 tics = 1 second).
///
/// Original C (p_spec.h): `#define BUTTONTIME 35`
pub const BUTTONTIME: i32 = 35;

/// Maximum number of switch-texture definitions loaded from `SWITCHES` lump.
///
/// Original C (p_spec.h): `#define MAXSWITCHES 50`
pub const MAXSWITCHES: usize = 50;
