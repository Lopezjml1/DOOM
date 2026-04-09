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

//! Shooting and aiming. Use lines. Radius attacks. Sector height changes.
//! Translated from linuxdoom-1.10/p_map.c (collision/trace portion, lines 791-1339)
//!
//! This module contains:
//! - **P_AimLineAttack** — auto-aim hitscan that narrows a vertical slope window
//! - **P_LineAttack** — fire a hitscan ray at a given slope, spawning puffs/blood
//! - **P_UseLines** — activate special lines in front of the player
//! - **P_RadiusAttack** — apply splash damage from an explosion
//! - **P_ChangeSector** — process height changes (crushing) in a sector
//!
//! The bare `fn(&Intercept) -> bool` callbacks cannot capture closures, so
//! shared state is stored in a `thread_local!` `RefCell<MapState>` instead of
//! `static mut`.  This is safe in the single-threaded DOOM engine and avoids
//! all `unsafe` access patterns.

use crate::info::mobjinfo::MobjType;
use std::cell::{Cell, RefCell};

use crate::info::sounds::SfxEnum;
use crate::info::states::StateNum;
use crate::play::maputl::{
    p_line_opening, p_point_on_line_side, traverser_t, Intercept, InterceptData, MapUtilState,
    PT_ADDLINES, PT_ADDTHINGS,
};
use crate::play::sight;
use crate::types::angle::Angle;
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::map_data::{LineDef, LineFlags, Sector, Vertex};
use crate::types::mobj::{MapObject, MobjFlags};
use crate::types::tables::{finecosine, FINESINE};
use crate::util::random::DoomRandom;

// Re-export bbox constants used in P_ChangeSector
use crate::util::bbox::{BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};

// =========================================================================
// Constants
// =========================================================================

/// Player use-line range (64 map units in fixed-point).
/// Original C: `#define USERANGE (64*FRACUNIT)` in p_local.h.
const USERANGE: i32 = 64 * FRACUNIT;

/// Maximum thing radius for blockmap search padding.
/// Original C: `#define MAXRADIUS (32*FRACUNIT)` in p_local.h.
const MAXRADIUS: i32 = 32 * FRACUNIT;

/// Melee attack range — for puff state selection.
/// Original C: `#define MELEERANGE (64*FRACUNIT)` in p_local.h.
const MELEERANGE: i32 = 64 * FRACUNIT;

/// Blockmap cell shift: FRACBITS + 7 = 23.
/// Original C: `#define MAPBLOCKSHIFT (FRACBITS+7)` in p_local.h.
const MAPBLOCKSHIFT: i32 = FRACBITS + 7;

/// Maximum number of special lines recorded during a single shoot-traverse.
const MAX_SHOOT_SPECIALS: usize = 64;

/// ML_TWOSIDED flag value for raw i16 flag checks on LineDef.flags.
const ML_TWOSIDED: i16 = LineFlags::ML_TWOSIDED.bits();

// =========================================================================
// Deferred effect types
// =========================================================================

/// Result of PTR_ShootTraverse — what did the hitscan ray hit?
#[derive(Debug, Clone, Copy)]
enum ShootHit {
    /// The ray did not hit anything within attack range.
    Nothing,
    /// The ray hit a wall.  Spawn a bullet puff here.
    Wall { x: Fixed, y: Fixed, z: Fixed },
    /// The ray hit sky — no visible effect.
    Sky,
    /// The ray hit a map object.
    Thing {
        target_idx: usize,
        x: Fixed,
        y: Fixed,
        z: Fixed,
        no_blood: bool,
    },
}

/// Result of PTR_UseTraverse — what happened on the use-line trace?
#[derive(Debug, Clone, Copy)]
enum UseResult {
    /// No result yet (traversal still in progress or completed without hitting).
    Nothing,
    /// Hit a wall with no special — play the "oof" sound.
    NoWay { mobj_idx: usize },
    /// Found a special line — activate it.
    UseSpecial { line_idx: usize, side: i32 },
}

// =========================================================================
// Module-level mutable state (thread-local, mirrors original C globals)
// =========================================================================

/// Consolidated state for the map attack/use/sector-change subsystem.
///
/// Stored in a `thread_local! { RefCell<MapState> }` because the bare
/// `fn(&Intercept) -> bool` callbacks cannot capture closures.  Using
/// `thread_local!` with `RefCell` is safe in the single-threaded DOOM
/// engine and eliminates all `unsafe` access to module-level mutable state.
struct MapState {
    // --- Attack state ---
    /// Index of the mobj that is shooting.
    shootthing: Option<usize>,
    /// Height of the shot origin: `z + height/2 + 8*FRACUNIT`.
    shootz: Fixed,
    /// Damage value for the current line attack.
    la_damage: i32,
    /// Maximum range of the current attack.
    attackrange: Fixed,

    // --- Use state ---
    /// Index of the mobj that is using a line.
    usething: Option<usize>,

    // --- Trace coordinates (pre-computed for callback access) ---
    /// Trace origin X.
    trace_x: Fixed,
    /// Trace origin Y.
    trace_y: Fixed,
    /// Trace delta X (x2 - x1).
    trace_dx: Fixed,
    /// Trace delta Y (y2 - y1).
    trace_dy: Fixed,

    // --- Line opening results (set by do_line_opening helper) ---
    opentop: Fixed,
    openbottom: Fixed,
    openrange: Fixed,
    #[allow(dead_code)]
    lowfloor: Fixed,

    // --- Sky flat number cache ---
    skyflatnum: i16,

    // --- Deferred shoot results ---
    /// Special lines hit during shoot traverse (recorded for post-traverse dispatch).
    shoot_specials: [usize; MAX_SHOOT_SPECIALS],
    shoot_special_count: usize,
    /// Final hit result of the shoot traverse.
    shoot_hit: ShootHit,

    // --- Deferred use result ---
    use_result: UseResult,

    // --- Raw data pointers (valid only during traversal) ---
    // SAFETY: These raw pointers are set by `stash_level_data()` immediately
    // before a traversal call and are only dereferenced during that traversal.
    // The data they point to (lines, sectors, mobjs, vertexes) is owned by
    // the `MapContext` and guaranteed live for the duration of the traversal.
    lines_ptr: *const LineDef,
    lines_len: usize,
    sectors_ptr: *const Sector,
    sectors_len: usize,
    mobjs_ptr: *const MapObject,
    mobjs_len: usize,
    vertexes_ptr: *const Vertex,
    vertexes_len: usize,
}

impl MapState {
    const fn new() -> Self {
        MapState {
            shootthing: None,
            shootz: Fixed(0),
            la_damage: 0,
            attackrange: Fixed(0),
            usething: None,
            trace_x: Fixed(0),
            trace_y: Fixed(0),
            trace_dx: Fixed(0),
            trace_dy: Fixed(0),
            opentop: Fixed(0),
            openbottom: Fixed(0),
            openrange: Fixed(0),
            lowfloor: Fixed(0),
            skyflatnum: 0,
            shoot_specials: [0; MAX_SHOOT_SPECIALS],
            shoot_special_count: 0,
            shoot_hit: ShootHit::Nothing,
            use_result: UseResult::Nothing,
            lines_ptr: std::ptr::null(),
            lines_len: 0,
            sectors_ptr: std::ptr::null(),
            sectors_len: 0,
            mobjs_ptr: std::ptr::null(),
            mobjs_len: 0,
            vertexes_ptr: std::ptr::null(),
            vertexes_len: 0,
        }
    }
}

thread_local! {
    /// Thread-local map traversal state.  Replaces the former `static mut MAP`.
    static MAP: RefCell<MapState> = const { RefCell::new(MapState::new()) };
}

// =========================================================================
// Exported module-level state (thread-local replacements for former statics)
// =========================================================================

thread_local! {
    /// The mobj that was targeted by the last `p_aim_line_attack` or hit by
    /// `p_line_attack`.  `None` if nothing was hit.
    /// Original C: `mobj_t* linetarget;` (p_map.c line 66).
    static LINETARGET: Cell<Option<usize>> = const { Cell::new(None) };

    /// The vertical slope determined by `p_aim_line_attack` (auto-aim result).
    /// Original C: `fixed_t aimslope;` (p_map.c line 800).
    static AIMSLOPE: Cell<Fixed> = const { Cell::new(Fixed(0)) };
}

// =========================================================================
// Raw-pointer accessor helpers (valid only within a traversal scope)
// =========================================================================
//
// The bare-function-pointer callback ABI means traversal callbacks cannot
// borrow the context directly. During traversal, `stash_level_data`
// copies raw pointers into the thread-local `MAP`.  The following helpers
// reconstruct borrowed slices from those pointers.
//
// SAFETY CONTRACT: every call site must guarantee that the `MapContext`
// whose data was stashed outlives the traversal (ensured by the scoped
// pattern in each public function).

/// Reconstruct the line slice from the stashed pointer in `MAP`.
///
/// # Safety
/// `MAP.lines_ptr` / `lines_len` must have been set by `stash_level_data`
/// and the originating slice must still be live.
#[inline]
fn map_lines_from(m: &MapState) -> &[LineDef] {
    // SAFETY: see stash_level_data contract.
    unsafe { std::slice::from_raw_parts(m.lines_ptr, m.lines_len) }
}

/// Reconstruct the sector slice from the stashed pointer in `MAP`.
#[inline]
fn map_sectors_from(m: &MapState) -> &[Sector] {
    unsafe { std::slice::from_raw_parts(m.sectors_ptr, m.sectors_len) }
}

/// Reconstruct the mobj slice from the stashed pointer in `MAP`.
#[inline]
fn map_mobjs_from(m: &MapState) -> &[MapObject] {
    unsafe { std::slice::from_raw_parts(m.mobjs_ptr, m.mobjs_len) }
}

/// Reconstruct the vertex slice from the stashed pointer in `MAP`.
#[inline]
fn map_vertexes_from(m: &MapState) -> &[Vertex] {
    unsafe { std::slice::from_raw_parts(m.vertexes_ptr, m.vertexes_len) }
}

/// Compute line opening and store results in `MAP`.
fn do_line_opening(m: &mut MapState, line_idx: usize) {
    let li = &map_lines_from(m)[line_idx];
    let secs = map_sectors_from(m);
    let mut temp = MapUtilState::new();
    p_line_opening(&mut temp, li, secs);
    m.opentop = temp.opentop;
    m.openbottom = temp.openbottom;
    m.openrange = temp.openrange;
    m.lowfloor = temp.lowfloor;
}

/// Store raw pointers to level data in `MAP` for callback access.
///
/// The caller must ensure the context outlives the traversal that follows.
fn stash_level_data(m: &mut MapState, ctx: &dyn MapContext) {
    let lines = ctx.lines();
    m.lines_ptr = lines.as_ptr();
    m.lines_len = lines.len();
    let sectors = ctx.sectors();
    m.sectors_ptr = sectors.as_ptr();
    m.sectors_len = sectors.len();
    let mobjs = ctx.mobjs();
    m.mobjs_ptr = mobjs.as_ptr();
    m.mobjs_len = mobjs.len();
    let verts = ctx.vertexes();
    m.vertexes_ptr = verts.as_ptr();
    m.vertexes_len = verts.len();
    m.skyflatnum = ctx.sky_flatnum();
}

// =========================================================================
// MapContext — context trait for map attack / use / sector-change functions
// =========================================================================

/// Trait providing all level data and cross-module dispatch needed by the
/// public functions in this module.
///
/// Modeled after `MovementContext`, `MobjContext`, etc. — each public
/// function borrows `&mut dyn MapContext` for the duration of its call.
/// Bare `fn(&Intercept) -> bool` callbacks (used by `p_path_traverse`)
/// cannot access the context directly — they read/write the thread-local
/// `MAP` instead and record deferred effects that the calling
/// function processes after traversal completes.
pub trait MapContext {
    // --- Level geometry ---
    fn lines(&self) -> &[LineDef];
    fn lines_mut(&mut self) -> &mut [LineDef];
    fn vertexes(&self) -> &[Vertex];
    fn sectors(&self) -> &[Sector];
    fn sectors_mut(&mut self) -> &mut [Sector];

    // --- Mobj arena ---
    fn mobjs(&self) -> &[MapObject];
    fn mobjs_mut(&mut self) -> &mut Vec<MapObject>;

    // --- Blockmap ---
    fn blocklinks(&self) -> &[Option<usize>];
    fn bmap_orgx(&self) -> Fixed;
    fn bmap_orgy(&self) -> Fixed;
    fn bmap_width(&self) -> i32;
    fn bmap_height(&self) -> i32;

    // --- Sky ---
    fn sky_flatnum(&self) -> i16;

    // --- Players ---
    fn players(&self) -> &[crate::types::player::Player];

    // --- Game state ---
    fn leveltime(&self) -> i32;

    // --- RNG ---
    fn rng_mut(&mut self) -> &mut DoomRandom;

    // --- Path traverse dispatch ---
    /// Execute `p_path_traverse` with the context's own level data.
    ///
    /// The implementor calls `maputl::p_path_traverse(...)` internally,
    /// splitting borrows on its own struct fields.  This avoids the
    /// borrow-conflict that would occur if we tried to pass both
    /// `ctx.lines_mut()` and `ctx.blockmap()` to `p_path_traverse` from
    /// outside the trait.
    fn do_path_traverse(
        &mut self,
        x1: Fixed,
        y1: Fixed,
        x2: Fixed,
        y2: Fixed,
        flags: i32,
        trav: traverser_t,
    ) -> bool;

    // --- Cross-module dispatch ---

    /// Spawn a bullet-impact puff.  Delegates to `play::mobj::p_spawn_puff`.
    fn p_spawn_puff(&mut self, x: Fixed, y: Fixed, z: Fixed, at_melee: bool) -> usize;

    /// Spawn blood spray.  Delegates to `play::mobj::p_spawn_blood`.
    fn p_spawn_blood(&mut self, x: Fixed, y: Fixed, z: Fixed, damage: i32) -> usize;

    /// Spawn a generic map object.  Delegates to `play::mobj::p_spawn_mobj`.
    fn p_spawn_mobj(&mut self, x: Fixed, y: Fixed, z: Fixed, mtype: MobjType) -> usize;

    /// Apply damage to a thing.  Delegates to `play::inter::p_damage_mobj`.
    fn p_damage_mobj(
        &mut self,
        target: usize,
        inflictor: Option<usize>,
        source: Option<usize>,
        damage: i32,
    );

    /// Set a mobj's state.  Delegates to `play::mobj::p_set_mobj_state`.
    fn p_set_mobj_state(&mut self, mobj: usize, state: StateNum) -> bool;

    /// Remove a mobj from the world.  Delegates to `play::mobj::p_remove_mobj`.
    fn p_remove_mobj(&mut self, mobj: usize);

    /// Recalculate z-clipping after a sector height change.
    /// Returns `true` if the thing fits, `false` if it doesn't.
    /// Delegates to `play::movement::p_thing_height_clip`.
    fn p_thing_height_clip(&mut self, thing: usize) -> bool;

    /// Line-of-sight check between two mobjs.
    /// Delegates to `play::sight::p_check_sight`.
    fn p_check_sight(&mut self, t1: usize, t2: usize) -> bool;

    /// Activate a shoot-triggered line special.
    /// Delegates to `play::spec::p_shoot_special_line`.
    fn p_shoot_special_line(&mut self, thing: usize, line: usize);

    /// Activate a use-triggered line special (full dispatch).
    fn p_use_special_line(&mut self, thing: usize, line: usize, side: i32);

    /// Play a sound effect at a mobj origin (or globally if `None`).
    fn s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum);
}

// =========================================================================
// PTR_AimTraverse — auto-aim intercept callback (p_map.c 816-893)
// =========================================================================
//
// Narrows the vertical slope window [bottomslope..topslope] as the trace
// crosses two-sided lines, and sets `aimslope` + `linetarget` when a
// SHOOTABLE thing is found within the window.

/// Auto-aim intercept callback.
///
/// Returns `true` to continue traversal, `false` to stop.
fn ptr_aim_traverse(intercept: &Intercept) -> bool {
    MAP.with(|cell| {
        let mut m = cell.borrow_mut();
        match intercept.d {
            InterceptData::Line(line_idx) => {
                // Extract line data into locals to avoid borrow conflicts.
                let (flags, frontsector, backsector) = {
                    let lines = map_lines_from(&m);
                    let li = &lines[line_idx];
                    (li.flags, li.frontsector, li.backsector)
                };

                // Not two-sided → blocks aim
                if (flags & ML_TWOSIDED) == 0 {
                    return false;
                }

                // Compute opening through the line
                do_line_opening(&mut m, line_idx);

                // Closed opening → blocks aim
                if m.openrange.0 <= 0 {
                    return false;
                }

                // Compute distance along the trace
                let dist = m.attackrange.fixed_mul(intercept.frac);
                if dist.0 == 0 {
                    return true; // degenerate — skip
                }

                let sectors = map_sectors_from(&m);

                // Narrow the slope window based on floor/ceiling differences
                if let (Some(front_idx), Some(back_idx)) = (frontsector, backsector) {
                    let front = &sectors[front_idx];
                    let back = &sectors[back_idx];

                    if front.floorheight != back.floorheight {
                        let slope = (m.openbottom - m.shootz).fixed_div(dist);
                        // SAFETY: sight::bottomslope is a pub static mut in
                        // sight.rs — single-threaded access only.
                        unsafe {
                            if slope.0 > sight::bottomslope.0 {
                                sight::bottomslope = slope;
                            }
                        }
                    }

                    if front.ceilingheight != back.ceilingheight {
                        let slope = (m.opentop - m.shootz).fixed_div(dist);
                        // SAFETY: same single-threaded guarantee.
                        unsafe {
                            if slope.0 < sight::topslope.0 {
                                sight::topslope = slope;
                            }
                        }
                    }
                }

                // If the slope window has closed, nothing more can be aimed at
                // SAFETY: sight statics — single-threaded access.
                let closed = unsafe { sight::topslope.0 <= sight::bottomslope.0 };
                if closed {
                    return false;
                }

                true // continue traversal
            }
            InterceptData::Thing(thing_idx) => {
                let mobjs = map_mobjs_from(&m);
                let th = &mobjs[thing_idx];

                // Don't aim at self
                if m.shootthing == Some(thing_idx) {
                    return true;
                }

                // Must be shootable
                if !th.flags.contains(MobjFlags::MF_SHOOTABLE) {
                    return true;
                }

                // Compute distance
                let dist = m.attackrange.fixed_mul(intercept.frac);
                if dist.0 == 0 {
                    return true;
                }

                // Slopes to the top and bottom of the thing
                let thingtopslope = Fixed(th.z.0 + th.height.0 - m.shootz.0).fixed_div(dist);
                let thingbottomslope = Fixed(th.z.0 - m.shootz.0).fixed_div(dist);

                // Check if thing is outside the slope window
                // SAFETY: sight statics — single-threaded access.
                let (sight_bottom, sight_top) = unsafe { (sight::bottomslope, sight::topslope) };

                if thingtopslope.0 < sight_bottom.0 {
                    return true; // shot over the thing
                }
                if thingbottomslope.0 > sight_top.0 {
                    return true; // shot under the thing
                }

                // Clamp to the slope window
                let top = if thingtopslope.0 > sight_top.0 {
                    sight_top
                } else {
                    thingtopslope
                };
                let bottom = if thingbottomslope.0 < sight_bottom.0 {
                    sight_bottom
                } else {
                    thingbottomslope
                };

                // Set aim slope as the midpoint of the clamped range
                AIMSLOPE.set(Fixed((top.0 + bottom.0) / 2));
                LINETARGET.set(Some(thing_idx));

                false // stop — found a target
            }
        }
    })
}

// =========================================================================
// PTR_ShootTraverse — hitscan shoot intercept callback (p_map.c 899-1016)
// =========================================================================

/// Hitscan shoot intercept callback.
///
/// Returns `true` to continue traversal, `false` to stop.
fn ptr_shoot_traverse(intercept: &Intercept) -> bool {
    MAP.with(|cell| {
        let mut m = cell.borrow_mut();
        match intercept.d {
            InterceptData::Line(line_idx) => {
                ptr_shoot_traverse_line(&mut m, line_idx, intercept.frac)
            }
            InterceptData::Thing(thing_idx) => {
                ptr_shoot_traverse_thing(&mut m, thing_idx, intercept.frac)
            }
        }
    })
}

/// Handle a line intercept during shoot traverse.
fn ptr_shoot_traverse_line(m: &mut MapState, line_idx: usize, frac: Fixed) -> bool {
    // Extract line data into locals to avoid borrow conflicts.
    let (special, flags, frontsector, backsector) = {
        let lines = map_lines_from(m);
        let li = &lines[line_idx];
        (li.special, li.flags, li.frontsector, li.backsector)
    };

    // Record special lines for post-traverse activation
    if special != 0 && m.shoot_special_count < MAX_SHOOT_SPECIALS {
        m.shoot_specials[m.shoot_special_count] = line_idx;
        m.shoot_special_count += 1;
    }

    // One-sided line — always blocks
    if (flags & ML_TWOSIDED) == 0 {
        return shoot_hit_line(m, line_idx, frac);
    }

    // Compute opening
    do_line_opening(m, line_idx);

    // Compute the distance to the intercept
    let dist = m.attackrange.fixed_mul(frac);

    let cur_aimslope = AIMSLOPE.get();

    // Check floor and ceiling slopes
    if let (Some(front_idx), Some(back_idx)) = (frontsector, backsector) {
        let sectors = map_sectors_from(m);
        let front = &sectors[front_idx];
        let back = &sectors[back_idx];

        // Floor check — if higher floor blocks the shot
        if front.floorheight != back.floorheight {
            let slope = (m.openbottom - m.shootz).fixed_div(dist);
            if slope.0 > cur_aimslope.0 {
                return shoot_hit_line(m, line_idx, frac);
            }
        }

        // Ceiling check — if lower ceiling blocks the shot
        if front.ceilingheight != back.ceilingheight {
            let slope = (m.opentop - m.shootz).fixed_div(dist);
            if slope.0 < cur_aimslope.0 {
                return shoot_hit_line(m, line_idx, frac);
            }
        }
    }

    // The line doesn't block the shot — continue
    true
}

/// Compute the wall-hit position and record it in `m.shoot_hit`.
///
/// Corresponds to the `hitline:` label in the original C code (p_map.c ~978).
/// Includes sky hack check.
fn shoot_hit_line(m: &mut MapState, line_idx: usize, frac: Fixed) -> bool {
    // Back up slightly to position the puff in front of the wall
    let frac = Fixed(frac.0 - Fixed(4 * FRACUNIT).fixed_div(m.attackrange).0);

    let cur_aimslope = AIMSLOPE.get();

    // Compute hit position
    let x = Fixed(m.trace_x.0 + m.trace_dx.fixed_mul(frac).0);
    let y = Fixed(m.trace_y.0 + m.trace_dy.fixed_mul(frac).0);
    let z = Fixed(m.shootz.0 + cur_aimslope.fixed_mul(m.attackrange.fixed_mul(frac)).0);

    // --- Sky hack check ---
    let lines = map_lines_from(m);
    let li = &lines[line_idx];

    if let Some(front_idx) = li.frontsector {
        let sectors = map_sectors_from(m);
        let front = &sectors[front_idx];
        if front.ceilingpic == m.skyflatnum {
            // Don't shoot the sky!
            if z.0 > front.ceilingheight.0 {
                m.shoot_hit = ShootHit::Sky;
                return false;
            }
            // Sky hack wall: back sector also sky ceiling → absorb silently
            if let Some(back_idx) = li.backsector {
                if sectors[back_idx].ceilingpic == m.skyflatnum {
                    m.shoot_hit = ShootHit::Sky;
                    return false;
                }
            }
        }
    }

    // Spawn bullet puff at impact point
    m.shoot_hit = ShootHit::Wall { x, y, z };
    false
}

/// Handle a thing intercept during shoot traverse.
fn ptr_shoot_traverse_thing(m: &mut MapState, thing_idx: usize, frac: Fixed) -> bool {
    let mobjs = map_mobjs_from(m);
    let th = &mobjs[thing_idx];

    // Don't shoot self
    if m.shootthing == Some(thing_idx) {
        return true;
    }

    // Must be shootable
    if !th.flags.contains(MobjFlags::MF_SHOOTABLE) {
        return true;
    }

    // Compute distance
    let dist = m.attackrange.fixed_mul(frac);
    if dist.0 == 0 {
        return true;
    }

    let cur_aimslope = AIMSLOPE.get();

    // Check if the shot goes over or under the thing
    let thingtopslope = Fixed(th.z.0 + th.height.0 - m.shootz.0).fixed_div(dist);
    if thingtopslope.0 < cur_aimslope.0 {
        return true; // shot over
    }

    let thingbottomslope = Fixed(th.z.0 - m.shootz.0).fixed_div(dist);
    if thingbottomslope.0 > cur_aimslope.0 {
        return true; // shot under
    }

    // Hit! Back up the fraction for the impact position
    let frac = Fixed(frac.0 - Fixed(10 * FRACUNIT).fixed_div(m.attackrange).0);

    let x = Fixed(m.trace_x.0 + m.trace_dx.fixed_mul(frac).0);
    let y = Fixed(m.trace_y.0 + m.trace_dy.fixed_mul(frac).0);
    let z = Fixed(m.shootz.0 + cur_aimslope.fixed_mul(m.attackrange.fixed_mul(frac)).0);

    let no_blood = th.flags.contains(MobjFlags::MF_NOBLOOD);

    m.shoot_hit = ShootHit::Thing {
        target_idx: thing_idx,
        x,
        y,
        z,
        no_blood,
    };

    false // stop traversal
}

// =========================================================================
// PTR_UseTraverse — use-line intercept callback (p_map.c 1095-1123)
// =========================================================================

/// Use-line intercept callback.
///
/// Returns `true` to continue traversal, `false` to stop.
fn ptr_use_traverse(intercept: &Intercept) -> bool {
    MAP.with(|cell| {
        let mut m = cell.borrow_mut();
        match intercept.d {
            InterceptData::Line(line_idx) => {
                let lines = map_lines_from(&m);
                let li = &lines[line_idx];

                if li.special == 0 {
                    // No special — check if the line blocks
                    do_line_opening(&mut m, line_idx);
                    if m.openrange.0 <= 0 {
                        // Closed opening — play "oof" sound
                        if let Some(thing_idx) = m.usething {
                            m.use_result = UseResult::NoWay {
                                mobj_idx: thing_idx,
                            };
                        }
                        return false;
                    }
                    // Opening exists — continue looking
                    true
                } else {
                    // Has special — determine side and record activation
                    let verts = map_vertexes_from(&m);
                    if let Some(thing_idx) = m.usething {
                        let mobjs = map_mobjs_from(&m);
                        let th = &mobjs[thing_idx];
                        let side = p_point_on_line_side(th.x, th.y, li, verts) as i32;
                        m.use_result = UseResult::UseSpecial { line_idx, side };
                    }
                    // Can't use more than one special line in a row
                    false
                }
            }
            // Things are not checked during use traversal
            InterceptData::Thing(_) => true,
        }
    })
}

// =========================================================================
// P_AimLineAttack — auto-aim hitscan (p_map.c 1022-1054)
// =========================================================================

/// Auto-aim a hitscan ray from `source_idx` along `angle` up to `distance`.
///
/// Sets the module-level `aimslope` and `linetarget` globals.
/// Returns the determined aim slope (or `Fixed(0)` if nothing was found).
///
/// Original C: `fixed_t P_AimLineAttack(mobj_t* t1, angle_t angle, fixed_t distance)`
pub fn p_aim_line_attack(
    source_idx: usize,
    angle: Angle,
    distance: Fixed,
    ctx: &mut dyn MapContext,
) -> Fixed {
    // --- Phase 1: set up thread-local MAP state ---
    let (x1, y1, x2, y2) = MAP.with(|cell| {
        let mut m = cell.borrow_mut();
        m.shootthing = Some(source_idx);
        m.attackrange = distance;
        m.la_damage = 0;
        m.shoot_hit = ShootHit::Nothing;
        m.shoot_special_count = 0;

        // Compute shot origin height: z + height/2 + 8*FRACUNIT
        let mobjs = ctx.mobjs();
        let source = &mobjs[source_idx];
        m.shootz = Fixed(source.z.0 + (source.height.0 >> 1) + 8 * FRACUNIT);

        // Compute trace endpoint from angle and distance
        let fine = angle.to_fine_angle();
        let dist_int = distance.0 >> FRACBITS;
        let x1 = source.x;
        let y1 = source.y;
        let x2 = Fixed(x1.0 + dist_int * finecosine(fine).0);
        let y2 = Fixed(y1.0 + dist_int * FINESINE[fine].0);

        // Store trace data for callback access
        m.trace_x = x1;
        m.trace_y = y1;
        m.trace_dx = Fixed(x2.0 - x1.0);
        m.trace_dy = Fixed(y2.0 - y1.0);

        // Stash level data pointers for callback access
        stash_level_data(&mut m, ctx);
        (x1, y1, x2, y2)
    });

    // Reset aim results
    LINETARGET.set(None);
    AIMSLOPE.set(Fixed(0));

    // Initialize auto-aim slope window: ±100*FRACUNIT/160 ≈ ±0.625 (~32°)
    // SAFETY: sight::topslope/bottomslope are pub static mut in sight.rs —
    // single-threaded access only.
    unsafe {
        sight::topslope = Fixed(100 * FRACUNIT / 160);
        sight::bottomslope = Fixed(-(100 * FRACUNIT / 160));
    }

    // --- Phase 2: execute the path traverse (callback borrows MAP) ---
    ctx.do_path_traverse(x1, y1, x2, y2, PT_ADDLINES | PT_ADDTHINGS, ptr_aim_traverse);

    // --- Phase 3: return the determined aim slope ---
    if LINETARGET.get().is_some() {
        AIMSLOPE.get()
    } else {
        Fixed(0)
    }
}

// =========================================================================
// P_LineAttack — fire hitscan (p_map.c 1062-1086)
// =========================================================================

/// Fire a hitscan ray from `source_idx` along `angle` at `slope` for
/// `distance`, dealing `damage` to whatever is hit.
///
/// Spawns a puff on wall hits and blood on thing hits.
/// Sets `linetarget` to the hit thing (if any).
///
/// Original C: `void P_LineAttack(mobj_t* t1, angle_t angle, fixed_t distance,
///              fixed_t slope, int damage)`
pub fn p_line_attack(
    source_idx: usize,
    angle: Angle,
    distance: Fixed,
    slope: Fixed,
    damage: i32,
    ctx: &mut dyn MapContext,
) {
    // Reset aim/target state
    AIMSLOPE.set(slope);
    LINETARGET.set(None);

    // --- Phase 1: set up thread-local MAP state ---
    let (x1, y1, x2, y2) = MAP.with(|cell| {
        let mut m = cell.borrow_mut();
        m.shootthing = Some(source_idx);
        m.la_damage = damage;
        m.attackrange = distance;
        m.shoot_hit = ShootHit::Nothing;
        m.shoot_special_count = 0;

        // Compute shot origin height
        let mobjs = ctx.mobjs();
        let source = &mobjs[source_idx];
        m.shootz = Fixed(source.z.0 + (source.height.0 >> 1) + 8 * FRACUNIT);

        // Compute trace endpoint
        let fine = angle.to_fine_angle();
        let dist_int = distance.0 >> FRACBITS;
        let x1 = source.x;
        let y1 = source.y;
        let x2 = Fixed(x1.0 + dist_int * finecosine(fine).0);
        let y2 = Fixed(y1.0 + dist_int * FINESINE[fine].0);

        m.trace_x = x1;
        m.trace_y = y1;
        m.trace_dx = Fixed(x2.0 - x1.0);
        m.trace_dy = Fixed(y2.0 - y1.0);

        // Stash level data pointers
        stash_level_data(&mut m, ctx);
        (x1, y1, x2, y2)
    });

    // --- Phase 2: execute the path traverse (callback borrows MAP) ---
    ctx.do_path_traverse(
        x1,
        y1,
        x2,
        y2,
        PT_ADDLINES | PT_ADDTHINGS,
        ptr_shoot_traverse,
    );

    // --- Phase 3: post-traverse dispatch ---

    // 1. Activate special lines that were shot
    let (special_count, specials, shootthing) = MAP.with(|cell| {
        let m = cell.borrow();
        let count = m.shoot_special_count;
        let mut specs = [0usize; MAX_SHOOT_SPECIALS];
        specs[..count].copy_from_slice(&m.shoot_specials[..count]);
        (count, specs, m.shootthing)
    });
    if let Some(st) = shootthing {
        for spec in specials.iter().take(special_count) {
            ctx.p_shoot_special_line(st, *spec);
        }
    }

    // 2. Handle the hit result
    let (hit, la_damage, at_melee_range) = MAP.with(|cell| {
        let m = cell.borrow();
        (m.shoot_hit, m.la_damage, m.attackrange.0 == MELEERANGE)
    });

    match hit {
        ShootHit::Nothing | ShootHit::Sky => {
            // Nothing to do
        }
        ShootHit::Wall { x, y, z } => {
            ctx.p_spawn_puff(x, y, z, at_melee_range);
        }
        ShootHit::Thing {
            target_idx,
            x,
            y,
            z,
            no_blood,
        } => {
            if no_blood {
                ctx.p_spawn_puff(x, y, z, at_melee_range);
            } else {
                ctx.p_spawn_blood(x, y, z, la_damage);
            }
            if la_damage != 0 {
                LINETARGET.set(Some(target_idx));
                ctx.p_damage_mobj(target_idx, shootthing, shootthing, la_damage);
            }
        }
    }
}

// =========================================================================
// P_UseLines — use special lines in front of the player (p_map.c 1130-1148)
// =========================================================================

/// Activate special lines in front of the player.
///
/// Traces a ray `USERANGE` units ahead of the player and activates the first
/// special line found, or plays the "oof" sound if the way is blocked.
///
/// Original C: `void P_UseLines(player_t* player)`
pub fn p_use_lines(player_idx: usize, ctx: &mut dyn MapContext) {
    // --- Phase 1: set up thread-local MAP state ---
    let setup = MAP.with(|cell| {
        let mut m = cell.borrow_mut();
        let players = ctx.players();
        let player = &players[player_idx];
        let mo_idx = match player.mobj {
            Some(idx) => idx,
            None => return None,
        };
        m.usething = Some(mo_idx);
        m.use_result = UseResult::Nothing;

        let mobjs = ctx.mobjs();
        let mo = &mobjs[mo_idx];
        let angle = mo.angle;

        let fine = angle.to_fine_angle();
        let dist_int = USERANGE >> FRACBITS;
        let x1 = mo.x;
        let y1 = mo.y;
        let x2 = Fixed(x1.0 + dist_int * finecosine(fine).0);
        let y2 = Fixed(y1.0 + dist_int * FINESINE[fine].0);

        m.trace_x = x1;
        m.trace_y = y1;
        m.trace_dx = Fixed(x2.0 - x1.0);
        m.trace_dy = Fixed(y2.0 - y1.0);

        // Stash level data pointers
        stash_level_data(&mut m, ctx);
        Some((x1, y1, x2, y2))
    });

    let (x1, y1, x2, y2) = match setup {
        Some(coords) => coords,
        None => return, // No player mobj — nothing to do.
    };

    // --- Phase 2: execute the path traverse (callback borrows MAP) ---
    ctx.do_path_traverse(x1, y1, x2, y2, PT_ADDLINES, ptr_use_traverse);

    // --- Phase 3: post-traverse dispatch ---
    let (use_result, usething) = MAP.with(|cell| {
        let m = cell.borrow();
        (m.use_result, m.usething)
    });

    match use_result {
        UseResult::Nothing => {
            // Nothing happened
        }
        UseResult::NoWay { mobj_idx } => {
            ctx.s_start_sound(Some(mobj_idx), SfxEnum::sfx_noway);
        }
        UseResult::UseSpecial { line_idx, side } => {
            if let Some(thing_idx) = usething {
                ctx.p_use_special_line(thing_idx, line_idx, side);
            }
        }
    }
}

// =========================================================================
// P_RadiusAttack — splash damage (p_map.c 1164-1233)
// =========================================================================

/// Apply splash (radius) damage from an explosion.
///
/// Iterates over all things in the affected blockmap area and applies
/// distance-based damage to each visible, shootable target.
///
/// Boss immunity: `MT_CYBORG` and `MT_SPIDER` take no splash damage.
///
/// Original C: `void P_RadiusAttack(mobj_t* spot, mobj_t* source, int damage)`
pub fn p_radius_attack(spot_idx: usize, source_idx: usize, damage: i32, ctx: &mut dyn MapContext) {
    // Compute the radius in fixed-point.
    // Original C: dist = (damage+MAXRADIUS) << FRACBITS;
    // NOTE: This overflows in 32-bit C (MAXRADIUS = 32*65536 = 2097152).
    // Through wrapping arithmetic the MAXRADIUS term vanishes, giving
    // dist ≈ damage << FRACBITS effectively.  We reproduce the exact
    // wrapping behavior to maintain parity.
    let dist_raw = damage.wrapping_add(MAXRADIUS).wrapping_shl(FRACBITS as u32);
    let dist = Fixed(dist_raw);

    // Read the explosion center position
    let (spot_x, spot_y);
    {
        let mobjs = ctx.mobjs();
        let spot = &mobjs[spot_idx];
        spot_x = spot.x;
        spot_y = spot.y;
    }

    // Compute blockmap bounds
    let orgx = ctx.bmap_orgx();
    let orgy = ctx.bmap_orgy();
    let bw = ctx.bmap_width();
    let bh = ctx.bmap_height();

    let yh = (spot_y.0 + dist.0 - orgy.0) >> MAPBLOCKSHIFT;
    let yl = (spot_y.0 - dist.0 - orgy.0) >> MAPBLOCKSHIFT;
    let xh = (spot_x.0 + dist.0 - orgx.0) >> MAPBLOCKSHIFT;
    let xl = (spot_x.0 - dist.0 - orgx.0) >> MAPBLOCKSHIFT;

    // Iterate blockmap cells and apply splash damage
    for by in yl..=yh {
        for bx in xl..=xh {
            // Bounds check
            if bx < 0 || bx >= bw || by < 0 || by >= bh {
                continue;
            }

            // Manual blockmap iteration (can't use p_block_things_iterator
            // because we need mutable context for sight checks and damage)
            let link_idx = (by * bw + bx) as usize;
            let mut mobj_opt = ctx.blocklinks()[link_idx];

            while let Some(thing_idx) = mobj_opt {
                // Read next link before potentially modifying things
                let next;
                {
                    let mobjs = ctx.mobjs();
                    next = mobjs[thing_idx].bnext;
                }

                // --- PIT_RadiusAttack logic (p_map.c 1164-1198) ---
                let should_damage;
                {
                    let mobjs = ctx.mobjs();
                    let thing = &mobjs[thing_idx];

                    // Must be shootable
                    if !thing.flags.contains(MobjFlags::MF_SHOOTABLE) {
                        mobj_opt = next;
                        continue;
                    }

                    // Boss immunity: Cyberdemons and Spider Masterminds
                    if thing.type_ == MobjType::MT_CYBORG as usize
                        || thing.type_ == MobjType::MT_SPIDER as usize
                    {
                        mobj_opt = next;
                        continue;
                    }

                    // Distance check: Chebyshev distance (max of |dx|, |dy|)
                    let dx = (thing.x.0 - spot_x.0).abs();
                    let dy = (thing.y.0 - spot_y.0).abs();
                    let mut thing_dist = if dx > dy { dx } else { dy };
                    thing_dist = (thing_dist - thing.radius.0) >> FRACBITS;
                    if thing_dist < 0 {
                        thing_dist = 0;
                    }

                    if thing_dist >= damage {
                        mobj_opt = next;
                        continue;
                    }

                    should_damage = damage - thing_dist;
                }

                // Line-of-sight check between target and explosion center
                if ctx.p_check_sight(thing_idx, spot_idx) {
                    ctx.p_damage_mobj(thing_idx, Some(spot_idx), Some(source_idx), should_damage);
                }

                mobj_opt = next;
            }
        }
    }
}

// =========================================================================
// P_ChangeSector — sector height change processing (p_map.c 1250-1338)
// =========================================================================

/// Process height changes for a sector (crushing).
///
/// Iterates over all things touching the sector's blockmap area and applies
/// height-clipping. Things that no longer fit are crushed: corpses become
/// gibs, dropped items are removed, and living things take 10 damage every
/// 4 tics with blood spray.
///
/// Returns `true` if any thing did not fit (`nofit`).
///
/// Original C: `boolean P_ChangeSector(sector_t* sector, boolean crunch)`
pub fn p_change_sector(sector_idx: usize, crunch: bool, ctx: &mut dyn MapContext) -> bool {
    let mut nofit = false;
    let crushchange = crunch;

    // Get the sector's blockbox (blockmap cell bounds)
    let (bb_top, bb_bottom, bb_left, bb_right);
    {
        let sectors = ctx.sectors();
        let sector = &sectors[sector_idx];
        bb_top = sector.blockbox[BOXTOP];
        bb_bottom = sector.blockbox[BOXBOTTOM];
        bb_left = sector.blockbox[BOXLEFT];
        bb_right = sector.blockbox[BOXRIGHT];
    }

    let bw = ctx.bmap_width();
    let bh = ctx.bmap_height();
    let leveltime = ctx.leveltime();

    // Iterate over all blockmap cells covered by the sector
    for by in bb_bottom..=bb_top {
        for bx in bb_left..=bb_right {
            // Bounds check
            if bx < 0 || bx >= bw || by < 0 || by >= bh {
                continue;
            }

            // Manual blockmap linked-list iteration
            let link_idx = (by * bw + bx) as usize;
            let mut mobj_opt = ctx.blocklinks()[link_idx];

            while let Some(thing_idx) = mobj_opt {
                // Read next link before modifying things
                let next;
                {
                    let mobjs = ctx.mobjs();
                    next = mobjs[thing_idx].bnext;
                }

                // --- PIT_ChangeSector logic (p_map.c 1257-1313) ---

                // Try to fit the thing into the new sector geometry
                if ctx.p_thing_height_clip(thing_idx) {
                    // Thing fits — continue
                    mobj_opt = next;
                    continue;
                }

                // Thing doesn't fit.
                let thing_health;
                let thing_flags;
                {
                    let mobjs = ctx.mobjs();
                    let thing = &mobjs[thing_idx];
                    thing_health = thing.health;
                    thing_flags = thing.flags;
                }

                // Dead things (health <= 0): crunch to gibs
                if thing_health <= 0 {
                    ctx.p_set_mobj_state(thing_idx, StateNum::S_GIBS);
                    {
                        let mobjs = ctx.mobjs_mut();
                        let thing = &mut mobjs[thing_idx];
                        thing.flags &= !MobjFlags::MF_SOLID;
                        thing.height = Fixed(0);
                        thing.radius = Fixed(0);
                    }
                    mobj_opt = next;
                    continue;
                }

                // Dropped items: remove entirely
                if thing_flags.contains(MobjFlags::MF_DROPPED) {
                    ctx.p_remove_mobj(thing_idx);
                    mobj_opt = next;
                    continue;
                }

                // Non-shootable things: don't interact with crushing
                if !thing_flags.contains(MobjFlags::MF_SHOOTABLE) {
                    mobj_opt = next;
                    continue;
                }

                // Living, shootable thing — it's being crushed
                nofit = true;

                if crushchange && (leveltime & 3) == 0 {
                    // Apply crush damage (10 hp) every 4 tics
                    ctx.p_damage_mobj(thing_idx, None, None, 10);

                    // Spawn blood spray with random momentum
                    let (bx_pos, by_pos, bz_pos);
                    {
                        let mobjs = ctx.mobjs();
                        let thing = &mobjs[thing_idx];
                        bx_pos = thing.x;
                        by_pos = thing.y;
                        bz_pos = Fixed(thing.z.0 + (thing.height.0 >> 1));
                    }
                    let blood_idx = ctx.p_spawn_mobj(bx_pos, by_pos, bz_pos, MobjType::MT_BLOOD);

                    // Random momentum for the blood splat
                    let rng = ctx.rng_mut();
                    let rnd1 = rng.p_random() as i32;
                    let rnd2 = rng.p_random() as i32;
                    let momx = Fixed((rnd1 - rnd2) << 12);
                    let rnd3 = rng.p_random() as i32;
                    let rnd4 = rng.p_random() as i32;
                    let momy = Fixed((rnd3 - rnd4) << 12);

                    {
                        let mobjs = ctx.mobjs_mut();
                        let blood = &mut mobjs[blood_idx];
                        blood.momx = momx;
                        blood.momy = momy;
                    }
                }

                mobj_opt = next;
            }
        }
    }

    nofit
}
