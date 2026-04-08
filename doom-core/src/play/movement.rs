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

//! Movement, collision handling. Sliding along walls.
//!
//! Translated from linuxdoom-1.10/p_map.c (movement portion) and
//! p_mobj.c (P_XYMovement / P_ZMovement).
//!
//! This module provides the core movement physics for DOOM:
//! - [`p_teleport_move`] — Unconditional teleportation with stomping.
//! - [`p_check_position`] — Collision test at a proposed position.
//! - [`p_try_move`] — Attempt to move, respecting step height and dropoffs.
//! - [`p_thing_height_clip`] — Re-fit a thing vertically after sector changes.
//! - [`p_slide_move`] — Wall-hugging slide movement when blocked.
//! - [`p_xy_movement`] — Horizontal physics per tic (friction, blocking).
//! - [`p_z_movement`] — Vertical physics per tic (gravity, float, clipping).
//!
//! # Safety
//!
//! This module uses `static mut` globals for movement state, matching the
//! original C code's global-variable architecture. All access is through
//! `unsafe` blocks and is safe in DOOM's single-threaded execution model.

#![allow(non_upper_case_globals)]

use crate::info::mobjinfo::{MobjType, MOBJINFO};
use crate::info::sounds::SfxEnum;
use crate::info::states::StateNum;
use crate::play::maputl::{
    self, p_aprox_distance, p_box_on_line_side, p_line_opening, p_point_on_line_side, traverser_t,
    Intercept, InterceptData,
};
use crate::types::angle::{Angle, ANG180, ANGLETOFINESHIFT};
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::map_data::{LineDef, LineFlags, Sector, SlopeType, Subsector, Vertex};
use crate::types::mobj::{MapObject, MobjFlags};
use crate::types::player::Player;
use crate::types::tables::{finecosine, point_to_angle2, FINEMASK, FINESINE};
use crate::util::bbox::{BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};
use crate::util::random::DoomRandom;

// Re-export P_XYMovement and P_ZMovement from mobj.rs where they are
// already fully implemented. The AAP places them in movement.rs but
// they reside in mobj.rs because p_mobj_thinker (also in mobj.rs)
// calls them directly. Re-exporting satisfies the export schema.
pub use super::mobj::{p_xy_movement, p_z_movement};

// ==========================================================================
// Constants
// ==========================================================================

/// Maximum special line crossings tracked per P_TryMove.
/// Original C: `#define MAXSPECIALCROSS 8` (p_map.c line 73).
const MAXSPECIALCROSS: usize = 8;

/// Maximum step-up height (24 map units).
/// Original C: `#define MAXSTEPHEIGHT (24*FRACUNIT)` (p_local.h).
const MAXSTEPHEIGHT: Fixed = Fixed(24 * FRACUNIT);

/// Maximum thing radius used to extend blockmap search area.
/// Original C: `#define MAXRADIUS (32*FRACUNIT)` (p_local.h line 56).
const MAXRADIUS: Fixed = Fixed(32 * FRACUNIT);

/// Blockmap cell shift: `FRACBITS + 7 = 23` (each cell is 128 map units).
/// Original C: `#define MAPBLOCKSHIFT (FRACBITS+7)` (p_local.h).
const MAPBLOCKSHIFT: i32 = FRACBITS + 7;

/// Fudge factor subtracted from slide fraction to prevent getting stuck.
const SLIDE_FUDGE: i32 = 0x800;

// ==========================================================================
// Movement context trait
// ==========================================================================

/// Trait providing the game-state interface needed by movement functions.
///
/// The concrete game context must implement this trait (typically by
/// delegating to its owned `LevelData`, mobj arena, players, etc.).
/// Movement functions accept `&mut dyn MovementContext` for polymorphic
/// dispatch.
pub trait MovementContext {
    // --- Level geometry ---
    fn lines(&self) -> &[LineDef];
    fn lines_mut(&mut self) -> &mut [LineDef];
    fn vertexes(&self) -> &[Vertex];
    fn sectors(&self) -> &[Sector];
    fn sectors_mut(&mut self) -> &mut [Sector];
    fn subsectors(&self) -> &[Subsector];

    // --- Mobj arena ---
    fn mobjs(&self) -> &[MapObject];
    fn mobjs_mut(&mut self) -> &mut Vec<MapObject>;

    // --- Blockmap ---
    /// Returns the blockmap offset table (past the 4-word header).
    fn blockmap(&self) -> &[i16];
    /// Returns the full blockmap lump data (header + offset table + lists).
    fn blockmaplump(&self) -> &[i16];
    fn blocklinks(&self) -> &[Option<usize>];
    fn blocklinks_mut(&mut self) -> &mut [Option<usize>];
    fn bmap_orgx(&self) -> Fixed;
    fn bmap_orgy(&self) -> Fixed;
    fn bmap_width(&self) -> i32;
    fn bmap_height(&self) -> i32;

    // --- Subsector lookup ---
    fn point_in_subsector(&self, x: Fixed, y: Fixed) -> usize;

    // --- Validation counter ---
    /// Get the current validcount (without incrementing).
    fn validcount(&self) -> i32;
    /// Increment validcount and return the new value.
    fn inc_validcount(&mut self) -> i32;

    // --- Game state ---
    fn gamemap(&self) -> i32;
    fn sky_flatnum(&self) -> i16;

    // --- Players ---
    fn players(&self) -> &[Player];
    fn players_mut(&mut self) -> &mut [Player];

    // --- RNG ---
    fn rng_mut(&mut self) -> &mut DoomRandom;

    // --- Cross-module dispatch ---
    fn p_damage_mobj(
        &mut self,
        target: usize,
        inflictor: Option<usize>,
        source: Option<usize>,
        damage: i32,
    );
    fn p_touch_special_thing(&mut self, special: usize, toucher: usize);
    fn p_cross_special_line(&mut self, line: usize, side: i32, thing: usize);
    fn p_set_mobj_state(&mut self, mobj: usize, state: StateNum) -> bool;
    fn p_explode_missile(&mut self, mobj: usize);
    fn p_remove_mobj(&mut self, mobj: usize);
    fn s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum);

    // --- Position linking ---
    /// Unlink a thing from its current sector/blockmap position.
    /// Implementor calls `maputl::p_unset_thing_position` internally, avoiding
    /// borrow-splitting issues with the trait-object interface.
    fn unset_thing_position(&mut self, thing_idx: usize);

    /// Link a thing into sector and blockmap at its current (x, y).
    /// Implementor calls `maputl::p_set_thing_position` internally.
    fn set_thing_position(&mut self, thing_idx: usize);

    // --- Path traverse helper ---
    /// Execute a path traverse (line intercepts only) for slide movement.
    ///
    /// The concrete implementation should call
    /// `maputl::p_path_traverse(...)` with the appropriate level data
    /// extracted from its internal state.
    fn do_slide_trace(
        &mut self,
        x1: Fixed,
        y1: Fixed,
        x2: Fixed,
        y2: Fixed,
        trav: traverser_t,
    ) -> bool;
}

// ==========================================================================
// Movement globals (static mut — matches original C globals)
// ==========================================================================

/// Temporary movement bounding box.
static mut tmbbox: [Fixed; 4] = [Fixed(0); 4];

/// Arena index of the thing being moved.
static mut tmthing_idx: Option<usize> = None;

/// Cached copy of `tmthing.flags`.
static mut tmflags: MobjFlags = MobjFlags::empty();

/// Destination X coordinate.
static mut tmx: Fixed = Fixed(0);

/// Destination Y coordinate.
static mut tmy: Fixed = Fixed(0);

// Pre-cached tmthing fields for use inside PIT callbacks where the mobj
// arena may be borrowed immutably by the blockmap iterator.

static mut tmradius: Fixed = Fixed(0);
static mut tmheight: Fixed = Fixed(0);
static mut tmz: Fixed = Fixed(0);
static mut tmtype: usize = 0; // MobjType index
static mut tm_info_damage: i32 = 0;
static mut tm_info_spawnstate: StateNum = StateNum::S_NULL;
static mut tm_target: Option<usize> = None;
static mut tm_target_type: Option<usize> = None;
static mut tm_player: Option<usize> = None;

// --- Result state from P_CheckPosition ---

/// If `true`, the move is OK if within the floor-ceiling gap.
pub static mut floatok: bool = false;

/// Highest contacted floor height.
pub static mut tmfloorz: Fixed = Fixed(0);

/// Lowest contacted ceiling height.
pub static mut tmceilingz: Fixed = Fixed(0);

/// Lowest floor point contacted (for dropoff detection).
pub static mut tmdropoffz: Fixed = Fixed(0);

/// Line that lowers the ceiling (used for sky missile hack).
pub static mut ceilingline: Option<usize> = None;

// --- Special line crossing tracking ---

/// Line indices of special lines crossed during P_TryMove.
pub static mut spechit: [usize; MAXSPECIALCROSS] = [0; MAXSPECIALCROSS];

/// Number of special lines recorded in `spechit`.
pub static mut numspechit: usize = 0;

// --- Slide move state (p_map.c lines 566-575) ---

static mut bestslidefrac: Fixed = Fixed(0);
static mut secondslidefrac: Fixed = Fixed(0);
static mut bestslideline: Option<usize> = None;
static mut secondslideline: Option<usize> = None;
static mut slidemo_idx: Option<usize> = None;
static mut tmxmove: Fixed = Fixed(0);
static mut tmymove: Fixed = Fixed(0);

// --- Raw pointers for PTR_SlideTraverse access (set before path traverse) ---

static mut SLIDE_LINES_PTR: *const LineDef = std::ptr::null();
static mut SLIDE_LINES_LEN: usize = 0;
static mut SLIDE_SECTORS_PTR: *const Sector = std::ptr::null();
static mut SLIDE_SECTORS_LEN: usize = 0;
static mut SLIDE_VERTEXES_PTR: *const Vertex = std::ptr::null();
static mut SLIDE_VERTEXES_LEN: usize = 0;
static mut SLIDE_MO_X: Fixed = Fixed(0);
static mut SLIDE_MO_Y: Fixed = Fixed(0);
static mut SLIDE_MO_Z: Fixed = Fixed(0);
static mut SLIDE_MO_HEIGHT: Fixed = Fixed(0);

// ==========================================================================
// Helper: cache tmthing fields into statics
// ==========================================================================

/// Copy relevant fields from `mobjs[thing_idx]` into static mut globals
/// so that PIT_* callbacks can read them without holding an immutable
/// borrow on the mobj arena.
unsafe fn cache_tmthing(thing_idx: usize, mobjs: &[MapObject]) {
    let mo = &mobjs[thing_idx];
    tmthing_idx = Some(thing_idx);
    tmflags = mo.flags;
    tmradius = mo.radius;
    tmheight = mo.height;
    tmz = mo.z;
    tmtype = mo.type_;
    tm_player = mo.player;
    tm_target = mo.target;
    if let Some(info_idx) = mo.info {
        tm_info_damage = MOBJINFO[info_idx].damage;
        tm_info_spawnstate = MOBJINFO[info_idx].spawnstate;
    } else {
        tm_info_damage = 0;
        tm_info_spawnstate = StateNum::S_NULL;
    }
    if let Some(target_idx) = mo.target {
        if target_idx < mobjs.len() {
            tm_target_type = Some(mobjs[target_idx].type_);
        } else {
            tm_target_type = None;
        }
    } else {
        tm_target_type = None;
    }
}

// ==========================================================================
// PIT_StompThing (p_map.c lines 81-108)
// ==========================================================================

/// Callback for P_TeleportMove blockmap iteration.
///
/// Damages (telefrag) any shootable thing within stomping range.
/// Monsters can only stomp on MAP30 (boss level).
///
/// Returns `true` to continue iteration, `false` if blocked.
fn pit_stomp_thing(thing_idx: usize, ctx: &mut dyn MovementContext) -> bool {
    let (thing_flags, thing_radius, thing_x, thing_y) = {
        let mo = &ctx.mobjs()[thing_idx];
        (mo.flags, mo.radius, mo.x, mo.y)
    };

    // Skip non-shootable things
    if !thing_flags.contains(MobjFlags::MF_SHOOTABLE) {
        return true;
    }

    let tmthing_i = unsafe { tmthing_idx.unwrap() };

    let blockdist = thing_radius + unsafe { tmradius };

    // Not within stomping distance
    if (thing_x - unsafe { tmx }).0.abs() >= blockdist.0
        || (thing_y - unsafe { tmy }).0.abs() >= blockdist.0
    {
        return true;
    }

    // Skip self
    if thing_idx == tmthing_i {
        return true;
    }

    // Monsters can only telefrag on MAP30 (boss map)
    let is_player = unsafe { std::ptr::addr_of!(tm_player).read().is_some() };
    if !is_player && ctx.gamemap() != 30 {
        return false;
    }

    // Telefrag: deal 10000 damage
    ctx.p_damage_mobj(thing_idx, Some(tmthing_i), Some(tmthing_i), 10000);

    true
}

// ==========================================================================
// P_TeleportMove (p_map.c lines 114-177)
// ==========================================================================

/// Teleport a thing to an absolute position, stomping everything at the
/// destination. Used by the teleport special and initial spawn.
///
/// Returns `true` if the teleport succeeded.
///
/// Original C: `P_TeleportMove` (p_map.c:114-177).
pub fn p_teleport_move(
    thing_idx: usize,
    x: Fixed,
    y: Fixed,
    ctx: &mut dyn MovementContext,
) -> bool {
    // Cache tmthing fields
    unsafe {
        cache_tmthing(thing_idx, ctx.mobjs());
        tmx = x;
        tmy = y;

        let radius = tmradius;
        tmbbox[BOXTOP] = y + radius;
        tmbbox[BOXBOTTOM] = y - radius;
        tmbbox[BOXRIGHT] = x + radius;
        tmbbox[BOXLEFT] = x - radius;
    }

    // Determine the subsector at the destination to get floor/ceiling heights.
    let ss_idx = ctx.point_in_subsector(x, y);
    let sec_idx = ctx.subsectors()[ss_idx].sector;
    let (floor_h, ceiling_h) = {
        let s = &ctx.sectors()[sec_idx];
        (s.floorheight, s.ceilingheight)
    };

    unsafe {
        tmfloorz = floor_h;
        tmdropoffz = floor_h;
        tmceilingz = ceiling_h;
        ceilingline = None;
    }

    let _vc = ctx.inc_validcount();
    unsafe {
        numspechit = 0;
    }

    // Stomp all things in the destination area using blockmap iteration.
    let (xl, xh, yl, yh) = unsafe {
        let orgx = ctx.bmap_orgx();
        let orgy = ctx.bmap_orgy();
        (
            (tmbbox[BOXLEFT].0 - orgx.0 - MAXRADIUS.0) >> MAPBLOCKSHIFT,
            (tmbbox[BOXRIGHT].0 - orgx.0 + MAXRADIUS.0) >> MAPBLOCKSHIFT,
            (tmbbox[BOXBOTTOM].0 - orgy.0 - MAXRADIUS.0) >> MAPBLOCKSHIFT,
            (tmbbox[BOXTOP].0 - orgy.0 + MAXRADIUS.0) >> MAPBLOCKSHIFT,
        )
    };

    for bx in xl..=xh {
        for by in yl..=yh {
            let things = collect_block_things(bx, by, ctx);
            for &ti in &things {
                if !pit_stomp_thing(ti, ctx) {
                    return false;
                }
            }
        }
    }

    // Success — unlink from old position.
    ctx.unset_thing_position(thing_idx);

    // Update coordinates.
    {
        let mo = &mut ctx.mobjs_mut()[thing_idx];
        mo.x = x;
        mo.y = y;
        mo.z = unsafe { tmfloorz };
        mo.floorz = unsafe { tmfloorz };
        mo.ceilingz = unsafe { tmceilingz };
    }

    // Re-link at new position.
    ctx.set_thing_position(thing_idx);

    true
}

// ==========================================================================
// Helper: collect blockmap thing indices
// ==========================================================================

/// Pre-collect all mobj arena indices from a single blockmap cell.
///
/// This avoids holding an immutable borrow on the mobj arena during
/// PIT callback processing (which may need mutable context access).
fn collect_block_things(bx: i32, by: i32, ctx: &dyn MovementContext) -> Vec<usize> {
    let mut result = Vec::new();
    let bmw = ctx.bmap_width();
    let bmh = ctx.bmap_height();
    if bx < 0 || by < 0 || bx >= bmw || by >= bmh {
        return result;
    }
    let idx = (by * bmw + bx) as usize;
    let blocklinks = ctx.blocklinks();
    let mobjs = ctx.mobjs();
    let mut mobj_opt = blocklinks[idx];
    while let Some(mi) = mobj_opt {
        result.push(mi);
        mobj_opt = mobjs[mi].bnext;
    }
    result
}

/// Pre-collect line indices from a single blockmap cell, updating validcount.
///
/// Returns the indices of lines in this cell that have not yet been visited
/// (validcount check). The lines' validcount is updated to the current value
/// so they are not processed again from adjacent cells.
fn collect_block_lines(
    bx: i32,
    by: i32,
    validcount: i32,
    ctx: &mut dyn MovementContext,
) -> Vec<usize> {
    let mut result = Vec::new();
    let bmw = ctx.bmap_width();
    let bmh = ctx.bmap_height();
    if bx < 0 || by < 0 || bx >= bmw || by >= bmh {
        return result;
    }
    let table_idx = (by * bmw + bx) as usize;
    let offset = ctx.blockmap()[table_idx] as usize;

    let mut list_idx = offset;
    loop {
        let val = ctx.blockmaplump()[list_idx];
        if val == -1 {
            break;
        }
        let line_num = val as usize;
        list_idx += 1;

        if ctx.lines()[line_num].validcount == validcount {
            continue;
        }
        ctx.lines_mut()[line_num].validcount = validcount;
        result.push(line_num);
    }
    result
}

// ==========================================================================
// PIT_CheckLine (p_map.c lines 189-247)
// ==========================================================================

/// Line iterator callback for P_CheckPosition.
///
/// Checks whether the proposed bounding box (`tmbbox`) is blocked by
/// `line_idx`. Updates tmfloorz, tmceilingz, tmdropoffz, and records
/// special lines in `spechit[]`.
///
/// Returns `true` to continue iteration, `false` if blocked.
fn pit_check_line(line_idx: usize, ctx: &mut dyn MovementContext) -> bool {
    let ld = ctx.lines()[line_idx]; // LineDef is Copy

    // Quick bounding-box rejection (4 comparisons).
    unsafe {
        if tmbbox[BOXRIGHT].0 <= ld.bbox[BOXLEFT].0
            || tmbbox[BOXLEFT].0 >= ld.bbox[BOXRIGHT].0
            || tmbbox[BOXTOP].0 <= ld.bbox[BOXBOTTOM].0
            || tmbbox[BOXBOTTOM].0 >= ld.bbox[BOXTOP].0
        {
            return true;
        }
    }

    // Bounding box vs line side test.
    let vertexes = ctx.vertexes();
    if p_box_on_line_side(unsafe { &*std::ptr::addr_of!(tmbbox) }, &ld, vertexes) != -1 {
        return true;
    }

    // One-sided line — always blocks.
    if ld.backsector.is_none() {
        return false;
    }

    // ML_BLOCKING blocks everything.
    if (ld.flags & LineFlags::ML_BLOCKING.bits()) != 0 {
        return false;
    }

    // ML_BLOCKMONSTERS blocks non-player things.
    if unsafe { std::ptr::addr_of!(tm_player).read().is_none() }
        && (ld.flags & LineFlags::ML_BLOCKMONSTERS.bits()) != 0
    {
        return false;
    }

    // Compute opening through two-sided line.
    p_line_opening(&ld, ctx.sectors());

    unsafe {
        let open_top = maputl::opentop;
        let open_bottom = maputl::openbottom;
        let _open_range = maputl::openrange;
        let low_floor = maputl::lowfloor;

        // Adjust tmfloorz and tmceilingz.
        if open_top < tmceilingz {
            tmceilingz = open_top;
            ceilingline = Some(line_idx);
        }
        if open_bottom > tmfloorz {
            tmfloorz = open_bottom;
        }
        if low_floor < tmdropoffz {
            tmdropoffz = low_floor;
        }
    }

    // Record special lines for P_TryMove crossing detection.
    if ld.special != 0 {
        unsafe {
            if numspechit < MAXSPECIALCROSS {
                spechit[numspechit] = line_idx;
                numspechit += 1;
            }
        }
    }

    true
}

// ==========================================================================
// PIT_CheckThing (p_map.c lines 252-343)
// ==========================================================================

/// Collision response actions deferred from `pit_check_thing`.
///
/// The PIT callback operates on a snapshot of thing data (immutable).
/// Damage values that require `P_Random()` are computed later when we
/// have mutable context access.
enum ThingAction {
    /// Continue iteration — nothing special happened.
    Continue,
    /// Skull-fly slam: zero momentum, restore spawnstate, deal damage.
    SkullSlam { attacker: usize, target: usize },
    /// Missile hit: deal damage to target.
    MissileHit {
        target: usize,
        inflictor: usize,
        source: Option<usize>,
    },
    /// Item pickup: toucher picks up the special thing.
    Pickup { special: usize, toucher: usize },
}

/// Thing iterator callback for P_CheckPosition (p_map.c lines 252-343).
///
/// Reads only from the immutable `thing` reference and cached statics.
/// Returns `(continue_iteration, action)`.
fn pit_check_thing(thing_idx: usize, thing: &MapObject) -> (bool, ThingAction) {
    // Skip things that cannot interact.
    if !thing.flags.contains(MobjFlags::MF_SOLID)
        && !thing.flags.contains(MobjFlags::MF_SPECIAL)
        && !thing.flags.contains(MobjFlags::MF_SHOOTABLE)
    {
        return (true, ThingAction::Continue);
    }

    // Block distance check (combined radii).
    let blockdist = Fixed(thing.radius.0 + unsafe { tmradius.0 });
    let dx = Fixed((thing.x.0 - unsafe { tmx.0 }).abs());
    let dy = Fixed((thing.y.0 - unsafe { tmy.0 }).abs());
    if dx.0 >= blockdist.0 || dy.0 >= blockdist.0 {
        return (true, ThingAction::Continue);
    }

    // Don't clip against self.
    let tm_idx = unsafe { tmthing_idx.unwrap() };
    if thing_idx == tm_idx {
        return (true, ThingAction::Continue);
    }

    // --- Skull-fly slam (lost soul charge attack) ---
    let cached_flags = unsafe { std::ptr::addr_of!(tmflags).read() };
    if cached_flags.contains(MobjFlags::MF_SKULLFLY) {
        return (
            false,
            ThingAction::SkullSlam {
                attacker: tm_idx,
                target: thing_idx,
            },
        );
    }

    // --- Missile impact ---
    if cached_flags.contains(MobjFlags::MF_MISSILE) {
        // z-range: missile can fly over or under the target.
        let tm_z = unsafe { tmz };
        let tm_h = unsafe { tmheight };
        if tm_z.0 > thing.z.0 + thing.height.0 {
            return (true, ThingAction::Continue); // over
        }
        if tm_z.0 + tm_h.0 < thing.z.0 {
            return (true, ThingAction::Continue); // under
        }

        // Same-species no-damage rule: knight ↔ bruiser projectiles.
        let tm_target_tp = unsafe { tm_target_type };
        if unsafe { std::ptr::addr_of!(tm_target).read().is_some() }
            && tm_target_tp == Some(thing.type_)
            && (thing.type_ == MobjType::MT_KNIGHT as usize
                || thing.type_ == MobjType::MT_BRUISER as usize)
        {
            return (true, ThingAction::Continue);
        }

        // Not shootable → blocked if solid.
        if !thing.flags.contains(MobjFlags::MF_SHOOTABLE) {
            return (
                !thing.flags.contains(MobjFlags::MF_SOLID),
                ThingAction::Continue,
            );
        }

        let source = unsafe { tm_target };
        return (
            false,
            ThingAction::MissileHit {
                target: thing_idx,
                inflictor: tm_idx,
                source,
            },
        );
    }

    // --- Item pickup ---
    if thing.flags.contains(MobjFlags::MF_SPECIAL) {
        let solid = thing.flags.contains(MobjFlags::MF_SOLID);
        if cached_flags.contains(MobjFlags::MF_PICKUP) {
            return (
                !solid,
                ThingAction::Pickup {
                    special: thing_idx,
                    toucher: tm_idx,
                },
            );
        }
        return (!solid, ThingAction::Continue);
    }

    // Default: blocked if solid.
    (
        !thing.flags.contains(MobjFlags::MF_SOLID),
        ThingAction::Continue,
    )
}

// ==========================================================================
// P_CheckPosition (p_map.c lines 374-442)
// ==========================================================================

/// Check whether `thing` can occupy position (`x`, `y`).
///
/// Sets `floatok`, `tmfloorz`, `tmceilingz`, `tmdropoffz`, `ceilingline`,
/// and `spechit[]` / `numspechit` as side-effects.
///
/// Returns `true` if the position is unblocked.
pub fn p_check_position(
    thing_idx: usize,
    x: Fixed,
    y: Fixed,
    ctx: &mut dyn MovementContext,
) -> bool {
    unsafe {
        cache_tmthing(thing_idx, ctx.mobjs());
    }

    unsafe {
        tmx = x;
        tmy = y;

        let radius = tmradius;
        tmbbox[BOXTOP] = Fixed(y.0 + radius.0);
        tmbbox[BOXBOTTOM] = Fixed(y.0 - radius.0);
        tmbbox[BOXRIGHT] = Fixed(x.0 + radius.0);
        tmbbox[BOXLEFT] = Fixed(x.0 - radius.0);
    }

    // Get floor/ceiling from destination subsector.
    let new_ss = ctx.point_in_subsector(x, y);
    let sec_idx = ctx.subsectors()[new_ss].sector;
    unsafe {
        let sector = &ctx.sectors()[sec_idx];
        tmfloorz = sector.floorheight;
        tmdropoffz = sector.floorheight;
        tmceilingz = sector.ceilingheight;
    }

    ctx.inc_validcount();
    unsafe {
        ceilingline = None;
        numspechit = 0;
    }

    // MF_NOCLIP ⇒ skip all collision.
    if unsafe {
        std::ptr::addr_of!(tmflags)
            .read()
            .contains(MobjFlags::MF_NOCLIP)
    } {
        return true;
    }

    // --- Thing iteration ---
    // Compute blockmap cell range (with MAXRADIUS extension for targets).
    let orgx = ctx.bmap_orgx();
    let orgy = ctx.bmap_orgy();

    let xl = (unsafe { tmbbox[BOXLEFT].0 } - orgx.0 - MAXRADIUS.0) >> MAPBLOCKSHIFT;
    let xh = (unsafe { tmbbox[BOXRIGHT].0 } - orgx.0 + MAXRADIUS.0) >> MAPBLOCKSHIFT;
    let yl = (unsafe { tmbbox[BOXBOTTOM].0 } - orgy.0 - MAXRADIUS.0) >> MAPBLOCKSHIFT;
    let yh = (unsafe { tmbbox[BOXTOP].0 } - orgy.0 + MAXRADIUS.0) >> MAPBLOCKSHIFT;

    // Collect thing indices from all relevant blockmap cells, then process.
    let mut all_thing_indices: Vec<usize> = Vec::new();
    for bx in xl..=xh {
        for by in yl..=yh {
            let things = collect_block_things(bx, by, ctx);
            all_thing_indices.extend(things);
        }
    }

    // Process collected thing indices via pit_check_thing.
    // Deferred actions are executed immediately after detection.
    for &ti in &all_thing_indices {
        if ti >= ctx.mobjs().len() {
            continue;
        }
        let thing = ctx.mobjs()[ti].clone();
        let (cont, action) = pit_check_thing(ti, &thing);

        match action {
            ThingAction::SkullSlam { attacker, target } => {
                // Compute damage with P_Random (now we have mut access).
                let info_damage = unsafe { tm_info_damage };
                let rng_val = (ctx.rng_mut().p_random() % 8 + 1) as i32;
                let damage = rng_val * info_damage;
                ctx.p_damage_mobj(target, Some(attacker), Some(attacker), damage);
                // Clear skullfly flag, zero momentum, restore spawnstate.
                {
                    let mo = &mut ctx.mobjs_mut()[attacker];
                    mo.flags.remove(MobjFlags::MF_SKULLFLY);
                    mo.momx = Fixed::ZERO;
                    mo.momy = Fixed::ZERO;
                    mo.momz = Fixed::ZERO;
                }
                let spawn_st = unsafe { tm_info_spawnstate };
                ctx.p_set_mobj_state(attacker, spawn_st);
                return false;
            }
            ThingAction::MissileHit {
                target,
                inflictor,
                source,
            } => {
                let info_damage = unsafe { tm_info_damage };
                let rng_val = (ctx.rng_mut().p_random() % 8 + 1) as i32;
                let damage = rng_val * info_damage;
                ctx.p_damage_mobj(target, Some(inflictor), source, damage);
                return false;
            }
            ThingAction::Pickup { special, toucher } => {
                ctx.p_touch_special_thing(special, toucher);
            }
            ThingAction::Continue => {}
        }

        if !cont {
            return false;
        }
    }

    // --- Line iteration ---
    let cur_vc = ctx.validcount();
    let xl_line = (unsafe { tmbbox[BOXLEFT].0 } - orgx.0) >> MAPBLOCKSHIFT;
    let xh_line = (unsafe { tmbbox[BOXRIGHT].0 } - orgx.0) >> MAPBLOCKSHIFT;
    let yl_line = (unsafe { tmbbox[BOXBOTTOM].0 } - orgy.0) >> MAPBLOCKSHIFT;
    let yh_line = (unsafe { tmbbox[BOXTOP].0 } - orgy.0) >> MAPBLOCKSHIFT;

    for bx in xl_line..=xh_line {
        for by in yl_line..=yh_line {
            let line_indices = collect_block_lines(bx, by, cur_vc, ctx);
            for li in line_indices {
                if !pit_check_line(li, ctx) {
                    return false;
                }
            }
        }
    }

    true
}

// ==========================================================================
// P_TryMove (p_map.c lines 450-517)
// ==========================================================================

/// Attempt to move `thing` to (`x`, `y`).
///
/// Calls [`p_check_position`] to validate the destination. If the position
/// is clear, the thing is unlinked, moved, and re-linked. Any special
/// lines crossed during the move trigger their effects via
/// `P_CrossSpecialLine`.
///
/// Returns `true` if the move succeeded.
pub fn p_try_move(thing_idx: usize, x: Fixed, y: Fixed, ctx: &mut dyn MovementContext) -> bool {
    unsafe {
        floatok = false;
    }

    if !p_check_position(thing_idx, x, y, ctx) {
        return false; // solid wall or thing
    }

    let mo_flags = ctx.mobjs()[thing_idx].flags;
    let mo_height = ctx.mobjs()[thing_idx].height;
    let mo_z = ctx.mobjs()[thing_idx].z;

    if !mo_flags.contains(MobjFlags::MF_NOCLIP) {
        unsafe {
            // Doesn't fit vertically.
            if tmceilingz.0 - tmfloorz.0 < mo_height.0 {
                return false;
            }

            floatok = true;

            // Mobj must lower itself to fit under ceiling.
            if !mo_flags.contains(MobjFlags::MF_TELEPORT) && tmceilingz.0 - mo_z.0 < mo_height.0 {
                return false;
            }

            // Too big a step up (> 24 map units).
            if !mo_flags.contains(MobjFlags::MF_TELEPORT) && tmfloorz.0 - mo_z.0 > MAXSTEPHEIGHT.0 {
                return false;
            }

            // Don't stand over a dropoff.
            if !mo_flags.contains(MobjFlags::MF_DROPOFF)
                && !mo_flags.contains(MobjFlags::MF_FLOAT)
                && tmfloorz.0 - tmdropoffz.0 > MAXSTEPHEIGHT.0
            {
                return false;
            }
        }
    }

    // The move is ok — link the thing into its new position.
    let oldx = ctx.mobjs()[thing_idx].x;
    let oldy = ctx.mobjs()[thing_idx].y;

    ctx.unset_thing_position(thing_idx);

    {
        let mo = &mut ctx.mobjs_mut()[thing_idx];
        mo.floorz = unsafe { tmfloorz };
        mo.ceilingz = unsafe { tmceilingz };
        mo.x = x;
        mo.y = y;
    }

    ctx.set_thing_position(thing_idx);

    // If any special lines were hit, do the effect.
    let mo_flags2 = ctx.mobjs()[thing_idx].flags;
    if !mo_flags2.contains(MobjFlags::MF_TELEPORT) && !mo_flags2.contains(MobjFlags::MF_NOCLIP) {
        let new_x = ctx.mobjs()[thing_idx].x;
        let new_y = ctx.mobjs()[thing_idx].y;

        // Collect spechit data before mutating context.
        let count = unsafe { numspechit };
        let mut spec_data: Vec<(usize, i16)> = Vec::with_capacity(count);
        for i in (0..count).rev() {
            let li = unsafe { spechit[i] };
            let special = ctx.lines()[li].special;
            spec_data.push((li, special));
        }

        for (li, special) in spec_data {
            // Check if the line was crossed.
            let side = p_point_on_line_side(new_x, new_y, &ctx.lines()[li], ctx.vertexes());
            let oldside = p_point_on_line_side(oldx, oldy, &ctx.lines()[li], ctx.vertexes());
            if side != oldside && special != 0 {
                ctx.p_cross_special_line(li, oldside, thing_idx);
            }
        }
    }

    true
}

// ==========================================================================
// P_ThingHeightClip (p_map.c lines 530-558)
// ==========================================================================

/// Recheck the vertical fit of a thing after a sector height change.
///
/// Walking things snap to the floor; floating things are pushed down if
/// the ceiling has lowered below their top. Returns `false` if the thing
/// no longer fits between floor and ceiling.
pub fn p_thing_height_clip(thing_idx: usize, ctx: &mut dyn MovementContext) -> bool {
    let onfloor;
    let thing_x;
    let thing_y;
    {
        let mo = &ctx.mobjs()[thing_idx];
        onfloor = mo.z.0 == mo.floorz.0;
        thing_x = mo.x;
        thing_y = mo.y;
    }

    p_check_position(thing_idx, thing_x, thing_y, ctx);

    // Update floorz / ceilingz from the check results.
    {
        let mo = &mut ctx.mobjs_mut()[thing_idx];
        mo.floorz = unsafe { tmfloorz };
        mo.ceilingz = unsafe { tmceilingz };
    }

    if onfloor {
        // Walking monsters rise and fall with the floor.
        let new_floor = unsafe { tmfloorz };
        ctx.mobjs_mut()[thing_idx].z = new_floor;
    } else {
        // Don't adjust a floating monster unless forced to.
        let mo = &ctx.mobjs()[thing_idx];
        if mo.z.0 + mo.height.0 > mo.ceilingz.0 {
            let new_z = Fixed(mo.ceilingz.0 - mo.height.0);
            ctx.mobjs_mut()[thing_idx].z = new_z;
        }
    }

    let mo = &ctx.mobjs()[thing_idx];
    mo.ceilingz.0 - mo.floorz.0 >= mo.height.0
}

// ==========================================================================
// P_HitSlideLine (p_map.c lines 584-630)
// ==========================================================================

/// Adjust `tmxmove`/`tmymove` so that the next move attempt slides
/// along the wall defined by `ld`.
///
/// Uses the BAM angle of the line and the movement vector to compute
/// the projected slide direction.
fn p_hit_slide_line(ld: &LineDef, ctx: &dyn MovementContext) {
    // Axis-aligned fast paths.
    if ld.slopetype == SlopeType::Horizontal {
        unsafe {
            tmymove = Fixed::ZERO;
        }
        return;
    }
    if ld.slopetype == SlopeType::Vertical {
        unsafe {
            tmxmove = Fixed::ZERO;
        }
        return;
    }

    let smo_idx = unsafe { slidemo_idx.unwrap() };
    let (sx, sy) = {
        let mo = &ctx.mobjs()[smo_idx];
        (mo.x, mo.y)
    };

    let side = p_point_on_line_side(sx, sy, ld, ctx.vertexes());

    let mut lineangle = point_to_angle2(Fixed::ZERO, Fixed::ZERO, ld.dx, ld.dy);

    if side == 1 {
        lineangle = Angle(lineangle.0.wrapping_add(ANG180.0));
    }

    let cur_tmxmove = unsafe { tmxmove };
    let cur_tmymove = unsafe { tmymove };

    let moveangle = point_to_angle2(Fixed::ZERO, Fixed::ZERO, cur_tmxmove, cur_tmymove);
    let mut deltaangle = Angle(moveangle.0.wrapping_sub(lineangle.0));

    if deltaangle.0 > ANG180.0 {
        deltaangle = Angle(deltaangle.0.wrapping_add(ANG180.0));
    }

    let lineangle_fine = (lineangle.0 >> ANGLETOFINESHIFT) as usize;
    let deltaangle_fine = (deltaangle.0 >> ANGLETOFINESHIFT) as usize;

    let movelen = p_aprox_distance(cur_tmxmove, cur_tmymove);
    let newlen = movelen.fixed_mul(finecosine(deltaangle_fine & FINEMASK));

    unsafe {
        tmxmove = newlen.fixed_mul(finecosine(lineangle_fine & FINEMASK));
        tmymove = newlen.fixed_mul(FINESINE[lineangle_fine & FINEMASK]);
    }
}

// ==========================================================================
// PTR_SlideTraverse (p_map.c lines 636-682)
// ==========================================================================

/// Path-traverse callback for P_SlideMove.
///
/// This is a bare `fn` pointer (`traverser_t`) — it cannot capture state,
/// so it reads slide globals and raw pointer statics directly.
fn ptr_slide_traverse(intercept: &Intercept) -> bool {
    // Must be a line intercept.
    let line_idx = match intercept.d {
        InterceptData::Line(li) => li,
        InterceptData::Thing(_) => {
            // "PTR_SlideTraverse: not a line?"
            // Original calls I_Error; we treat it as non-blocking.
            return true;
        }
    };

    // Read the LineDef from the raw pointer stash.
    let li = unsafe {
        assert!(!SLIDE_LINES_PTR.is_null());
        let slice = std::slice::from_raw_parts(SLIDE_LINES_PTR, SLIDE_LINES_LEN);
        slice[line_idx]
    };

    let is_two_sided = (li.flags & LineFlags::ML_TWOSIDED.bits()) != 0;

    if !is_two_sided {
        // One-sided line — don't hit the back side.
        let verts = unsafe { std::slice::from_raw_parts(SLIDE_VERTEXES_PTR, SLIDE_VERTEXES_LEN) };
        let side = p_point_on_line_side(unsafe { SLIDE_MO_X }, unsafe { SLIDE_MO_Y }, &li, verts);
        if side == 1 {
            return true; // back side, ignore
        }
        // Fall through to "is blocking".
    } else {
        // Two-sided line — check opening.
        let sectors = unsafe { std::slice::from_raw_parts(SLIDE_SECTORS_PTR, SLIDE_SECTORS_LEN) };
        p_line_opening(&li, sectors);

        let mo_height = unsafe { SLIDE_MO_HEIGHT };
        let mo_z = unsafe { SLIDE_MO_Z };

        unsafe {
            if maputl::openrange.0 >= mo_height.0
                && maputl::opentop.0 - mo_z.0 >= mo_height.0
                && maputl::openbottom.0 - mo_z.0 <= MAXSTEPHEIGHT.0
            {
                // This line doesn't block movement.
                return true;
            }
        }
        // Fall through to "is blocking".
    }

    // The line blocks movement — see if it is closer than best so far.
    unsafe {
        if intercept.frac.0 < bestslidefrac.0 {
            secondslidefrac = bestslidefrac;
            secondslideline = bestslideline;
            bestslidefrac = intercept.frac;
            bestslideline = Some(line_idx);
        }
    }

    false // stop
}

// ==========================================================================
// P_SlideMove (p_map.c lines 695-788)
// ==========================================================================

/// The momentum move is blocked, so try to slide along a wall.
///
/// Traces the leading corners of the mobj's bounding box to find the
/// first blocking wall, moves up to it, then clips the remaining
/// movement along the wall surface. Retries up to 3 times.
pub fn p_slide_move(mo_idx: usize, ctx: &mut dyn MovementContext) {
    unsafe {
        slidemo_idx = Some(mo_idx);
    }

    // Set up raw pointer stashes for PTR_SlideTraverse.
    unsafe {
        let lines = ctx.lines();
        SLIDE_LINES_PTR = lines.as_ptr();
        SLIDE_LINES_LEN = lines.len();

        let sectors = ctx.sectors();
        SLIDE_SECTORS_PTR = sectors.as_ptr();
        SLIDE_SECTORS_LEN = sectors.len();

        let verts = ctx.vertexes();
        SLIDE_VERTEXES_PTR = verts.as_ptr();
        SLIDE_VERTEXES_LEN = verts.len();
    }

    let mut hitcount: i32 = 0;

    // retry loop — original uses goto; we use a loop.
    loop {
        hitcount += 1;
        if hitcount == 3 {
            // stairstep fallback: try Y-only then X-only.
            let mo = &ctx.mobjs()[mo_idx];
            let mox = mo.x;
            let moy = mo.y;
            let momx = mo.momx;
            let momy = mo.momy;
            if !p_try_move(mo_idx, mox, Fixed(moy.0 + momy.0), ctx) {
                p_try_move(mo_idx, Fixed(mox.0 + momx.0), moy, ctx);
            }
            return;
        }

        // Determine leading / trailing corners based on momentum direction.
        let (leadx, trailx, leady, traily, momx, momy);
        {
            let mo = &ctx.mobjs()[mo_idx];
            momx = mo.momx;
            momy = mo.momy;
            if mo.momx.0 > 0 {
                leadx = Fixed(mo.x.0 + mo.radius.0);
                trailx = Fixed(mo.x.0 - mo.radius.0);
            } else {
                leadx = Fixed(mo.x.0 - mo.radius.0);
                trailx = Fixed(mo.x.0 + mo.radius.0);
            }
            if mo.momy.0 > 0 {
                leady = Fixed(mo.y.0 + mo.radius.0);
                traily = Fixed(mo.y.0 - mo.radius.0);
            } else {
                leady = Fixed(mo.y.0 - mo.radius.0);
                traily = Fixed(mo.y.0 + mo.radius.0);
            }

            unsafe {
                SLIDE_MO_X = mo.x;
                SLIDE_MO_Y = mo.y;
                SLIDE_MO_Z = mo.z;
                SLIDE_MO_HEIGHT = mo.height;
            }
        }

        unsafe {
            bestslidefrac = Fixed(FRACUNIT + 1);
        }

        // Three path-traverse calls along leading/trailing corners.
        ctx.do_slide_trace(
            leadx,
            leady,
            Fixed(leadx.0 + momx.0),
            Fixed(leady.0 + momy.0),
            ptr_slide_traverse,
        );
        ctx.do_slide_trace(
            trailx,
            leady,
            Fixed(trailx.0 + momx.0),
            Fixed(leady.0 + momy.0),
            ptr_slide_traverse,
        );
        ctx.do_slide_trace(
            leadx,
            traily,
            Fixed(leadx.0 + momx.0),
            Fixed(traily.0 + momy.0),
            ptr_slide_traverse,
        );

        // Move up to the wall.
        if unsafe { bestslidefrac.0 } == FRACUNIT + 1 {
            // The move must have hit the middle — stairstep.
            let mo = &ctx.mobjs()[mo_idx];
            let mox = mo.x;
            let moy = mo.y;
            let mmx = mo.momx;
            let mmy = mo.momy;
            if !p_try_move(mo_idx, mox, Fixed(moy.0 + mmy.0), ctx) {
                p_try_move(mo_idx, Fixed(mox.0 + mmx.0), moy, ctx);
            }
            return;
        }

        // Fudge a bit to make sure it doesn't touch the wall.
        unsafe {
            bestslidefrac = Fixed(bestslidefrac.0 - SLIDE_FUDGE);
        }
        if unsafe { bestslidefrac.0 } > 0 {
            let newx = momx.fixed_mul(unsafe { bestslidefrac });
            let newy = momy.fixed_mul(unsafe { bestslidefrac });

            let mo_x = ctx.mobjs()[mo_idx].x;
            let mo_y = ctx.mobjs()[mo_idx].y;
            if !p_try_move(mo_idx, Fixed(mo_x.0 + newx.0), Fixed(mo_y.0 + newy.0), ctx) {
                // stairstep
                let mo2 = &ctx.mobjs()[mo_idx];
                let mx2 = mo2.x;
                let my2 = mo2.y;
                let mmx2 = mo2.momx;
                let mmy2 = mo2.momy;
                if !p_try_move(mo_idx, mx2, Fixed(my2.0 + mmy2.0), ctx) {
                    p_try_move(mo_idx, Fixed(mx2.0 + mmx2.0), my2, ctx);
                }
                return;
            }
        }

        // Now continue along the wall — calculate remainder.
        unsafe {
            bestslidefrac = Fixed(FRACUNIT - (bestslidefrac.0 + SLIDE_FUDGE));
            if bestslidefrac.0 > FRACUNIT {
                bestslidefrac = Fixed(FRACUNIT);
            }
            if bestslidefrac.0 <= 0 {
                return;
            }
        }

        unsafe {
            tmxmove = momx.fixed_mul(bestslidefrac);
            tmymove = momy.fixed_mul(bestslidefrac);
        }

        // Clip the remainder along the wall.
        let bsl = unsafe { bestslideline };
        if let Some(line_idx) = bsl {
            let ld = ctx.lines()[line_idx]; // LineDef is Copy
            p_hit_slide_line(&ld, ctx);
        }

        // Update mobj momentum.
        {
            let mo = &mut ctx.mobjs_mut()[mo_idx];
            mo.momx = unsafe { tmxmove };
            mo.momy = unsafe { tmymove };
        }

        let mo_x = ctx.mobjs()[mo_idx].x;
        let mo_y = ctx.mobjs()[mo_idx].y;
        let new_tmx = unsafe { tmxmove };
        let new_tmy = unsafe { tmymove };
        if !p_try_move(
            mo_idx,
            Fixed(mo_x.0 + new_tmx.0),
            Fixed(mo_y.0 + new_tmy.0),
            ctx,
        ) {
            // Retry — loop back.
            continue;
        }

        // Successful slide move.
        return;
    }
}
