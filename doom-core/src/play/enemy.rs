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

//! Enemy thinking, AI. Action Pointer Functions associated with states/frames.
//! Translated from linuxdoom-1.10/p_enemy.c
//!
//! This is the largest gameplay module, containing ALL monster AI logic:
//! direction tracking, movement, chase behavior, melee/missile range checks,
//! pathfinding, target acquisition, and every monster-specific action function.

use crate::info::mobjinfo::{MobjType, MOBJINFO};
use crate::info::sounds::SfxEnum;
// SpriteNum used indirectly through state machine
use crate::info::states::{StateNum, STATES};
use crate::play::maputl::p_aprox_distance;
use crate::play::spec::{FloorType, VldoorType};
use crate::play::{FLOATSPEED, MELEERANGE, MISSILERANGE};
use crate::types::angle::{Angle, ANG180, ANG270, ANG90, ANGLETOFINESHIFT};
use crate::types::doomdef::{GameMode, Skill};
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::map_data::{LineDef, LineFlags, Sector, SideDef, Subsector};
use crate::types::mobj::{MapObject, MobjFlags};
use crate::types::player::Player;
use crate::types::tables::{finecosine, point_to_angle2, FINESINE};
use crate::util::random::DoomRandom;

use std::cell::RefCell;

/// Constant lookup table of all SfxEnum variants in discriminant order.
/// Used for safe index-based conversion without `transmute`.
const SFX_VARIANTS: [SfxEnum; 109] = [
    SfxEnum::sfx_None,
    SfxEnum::sfx_pistol,
    SfxEnum::sfx_shotgn,
    SfxEnum::sfx_sgcock,
    SfxEnum::sfx_dshtgn,
    SfxEnum::sfx_dbopn,
    SfxEnum::sfx_dbcls,
    SfxEnum::sfx_dbload,
    SfxEnum::sfx_plasma,
    SfxEnum::sfx_bfg,
    SfxEnum::sfx_sawup,
    SfxEnum::sfx_sawidl,
    SfxEnum::sfx_sawful,
    SfxEnum::sfx_sawhit,
    SfxEnum::sfx_rlaunc,
    SfxEnum::sfx_rxplod,
    SfxEnum::sfx_firsht,
    SfxEnum::sfx_firxpl,
    SfxEnum::sfx_pstart,
    SfxEnum::sfx_pstop,
    SfxEnum::sfx_doropn,
    SfxEnum::sfx_dorcls,
    SfxEnum::sfx_stnmov,
    SfxEnum::sfx_swtchn,
    SfxEnum::sfx_swtchx,
    SfxEnum::sfx_plpain,
    SfxEnum::sfx_dmpain,
    SfxEnum::sfx_popain,
    SfxEnum::sfx_vipain,
    SfxEnum::sfx_mnpain,
    SfxEnum::sfx_pepain,
    SfxEnum::sfx_slop,
    SfxEnum::sfx_itemup,
    SfxEnum::sfx_wpnup,
    SfxEnum::sfx_oof,
    SfxEnum::sfx_telept,
    SfxEnum::sfx_posit1,
    SfxEnum::sfx_posit2,
    SfxEnum::sfx_posit3,
    SfxEnum::sfx_bgsit1,
    SfxEnum::sfx_bgsit2,
    SfxEnum::sfx_sgtsit,
    SfxEnum::sfx_cacsit,
    SfxEnum::sfx_brssit,
    SfxEnum::sfx_cybsit,
    SfxEnum::sfx_spisit,
    SfxEnum::sfx_bspsit,
    SfxEnum::sfx_kntsit,
    SfxEnum::sfx_vilsit,
    SfxEnum::sfx_mansit,
    SfxEnum::sfx_pesit,
    SfxEnum::sfx_sklatk,
    SfxEnum::sfx_sgtatk,
    SfxEnum::sfx_skepch,
    SfxEnum::sfx_vilatk,
    SfxEnum::sfx_claw,
    SfxEnum::sfx_skeswg,
    SfxEnum::sfx_pldeth,
    SfxEnum::sfx_pdiehi,
    SfxEnum::sfx_podth1,
    SfxEnum::sfx_podth2,
    SfxEnum::sfx_podth3,
    SfxEnum::sfx_bgdth1,
    SfxEnum::sfx_bgdth2,
    SfxEnum::sfx_sgtdth,
    SfxEnum::sfx_cacdth,
    SfxEnum::sfx_skldth,
    SfxEnum::sfx_brsdth,
    SfxEnum::sfx_cybdth,
    SfxEnum::sfx_spidth,
    SfxEnum::sfx_bspdth,
    SfxEnum::sfx_vildth,
    SfxEnum::sfx_kntdth,
    SfxEnum::sfx_pedth,
    SfxEnum::sfx_skedth,
    SfxEnum::sfx_posact,
    SfxEnum::sfx_bgact,
    SfxEnum::sfx_dmact,
    SfxEnum::sfx_bspact,
    SfxEnum::sfx_bspwlk,
    SfxEnum::sfx_vilact,
    SfxEnum::sfx_noway,
    SfxEnum::sfx_barexp,
    SfxEnum::sfx_punch,
    SfxEnum::sfx_hoof,
    SfxEnum::sfx_metal,
    SfxEnum::sfx_chgun,
    SfxEnum::sfx_tink,
    SfxEnum::sfx_bdopn,
    SfxEnum::sfx_bdcls,
    SfxEnum::sfx_itmbk,
    SfxEnum::sfx_flame,
    SfxEnum::sfx_flamst,
    SfxEnum::sfx_getpow,
    SfxEnum::sfx_bospit,
    SfxEnum::sfx_boscub,
    SfxEnum::sfx_bossit,
    SfxEnum::sfx_bospn,
    SfxEnum::sfx_bosdth,
    SfxEnum::sfx_manatk,
    SfxEnum::sfx_mandth,
    SfxEnum::sfx_sssit,
    SfxEnum::sfx_ssdth,
    SfxEnum::sfx_keenpn,
    SfxEnum::sfx_keendt,
    SfxEnum::sfx_skeact,
    SfxEnum::sfx_skesit,
    SfxEnum::sfx_skeatk,
    SfxEnum::sfx_radio,
];

/// Safely convert a usize to SfxEnum via const lookup table.
/// Returns None if out of range.
#[inline]
fn sfx_from_usize(val: usize) -> Option<SfxEnum> {
    SFX_VARIANTS.get(val).copied()
}

// ============================================================================
// Context trait — unified interface for all enemy AI operations
// ============================================================================

/// Context trait providing all operations needed by enemy AI functions.
///
/// This trait unifies access to game state, level geometry, combat mechanics,
/// object lifecycle, thinker iteration, and boss death triggers — everything
/// the monster AI subsystem needs from the rest of the engine.
pub trait EnemyContext {
    // --- MapObject access ---
    fn mobjs(&self) -> &[MapObject];
    fn mobjs_mut(&mut self) -> &mut Vec<MapObject>;

    // --- Sector / Line / Side / Subsector access ---
    fn sectors(&self) -> &[Sector];
    fn sectors_mut(&mut self) -> &mut [Sector];
    fn lines(&self) -> &[LineDef];
    fn sides(&self) -> &[SideDef];
    fn subsectors(&self) -> &[Subsector];
    fn num_lines(&self) -> usize;

    // --- Blockmap ---
    fn blocklinks(&self) -> &[Option<usize>];
    fn bmap_orgx(&self) -> Fixed;
    fn bmap_orgy(&self) -> Fixed;
    fn bmap_width(&self) -> i32;
    fn bmap_height(&self) -> i32;

    // --- Player state ---
    fn players(&self) -> &[Player];
    fn players_mut(&mut self) -> &mut [Player];
    fn playeringame(&self) -> &[bool];

    // --- Game state ---
    fn gamemode(&self) -> GameMode;
    fn gameskill(&self) -> Skill;
    fn gamemap(&self) -> i32;
    fn gameepisode(&self) -> i32;
    fn gametic(&self) -> i32;
    fn netgame(&self) -> bool;
    fn deathmatch(&self) -> i32;
    fn leveltime(&self) -> i32;
    fn nomonsters(&self) -> bool;
    fn respawnmonsters(&self) -> bool;

    // --- Sound ---
    fn s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum);

    // --- Random ---
    fn rng_mut(&mut self) -> &mut DoomRandom;

    // --- Movement / Collision ---
    fn p_try_move(&mut self, thing_idx: usize, x: Fixed, y: Fixed) -> bool;
    fn p_teleport_move(&mut self, thing_idx: usize, x: Fixed, y: Fixed) -> bool;
    /// Read `floatok` flag from most recent P_TryMove / P_CheckPosition.
    fn movement_floatok(&self) -> bool;
    /// Read `tmfloorz` from most recent P_TryMove / P_CheckPosition.
    fn movement_tmfloorz(&self) -> Fixed;
    /// Retrieve spechit line indices recorded during most recent P_TryMove.
    fn movement_spechit(&self) -> &[usize];
    /// Count of special-line hits from most recent P_TryMove.
    fn movement_numspechit(&self) -> usize;

    // --- Combat ---
    fn p_check_sight(&mut self, t1: usize, t2: usize) -> bool;
    fn p_aim_line_attack(
        &mut self,
        source: usize,
        angle: Angle,
        range: Fixed,
    ) -> (Fixed, Option<usize>);
    fn p_line_attack(
        &mut self,
        source: usize,
        angle: Angle,
        range: Fixed,
        slope: Fixed,
        damage: i32,
    );
    fn p_radius_attack(&mut self, spot: usize, source: usize, damage: i32);
    fn p_damage_mobj(
        &mut self,
        target: usize,
        inflictor: Option<usize>,
        source: Option<usize>,
        damage: i32,
    );

    // --- Object lifecycle ---
    fn p_set_mobj_state(&mut self, mobj: usize, state: StateNum) -> bool;
    fn p_spawn_mobj(&mut self, x: Fixed, y: Fixed, z: Fixed, mtype: MobjType) -> usize;
    fn p_spawn_missile(&mut self, source: usize, dest: usize, mtype: MobjType) -> usize;
    fn p_remove_mobj(&mut self, mobj: usize);
    fn p_spawn_puff(&mut self, x: Fixed, y: Fixed, z: Fixed) -> usize;

    // --- Switch / Special line activation ---
    fn p_use_special_line(&mut self, thing: usize, line: usize, side: i32) -> bool;

    // --- Thinker iteration ---
    /// Collect the mobj arena indices of all active MobjThinker entities.
    fn collect_mobj_thinker_indices(&self) -> Vec<usize>;

    // --- Boss triggers ---
    /// Execute a door action on all sectors matching the given tag.
    fn ev_do_door_by_tag(&mut self, tag: i16, door_type: VldoorType) -> bool;
    /// Execute a floor action on all sectors matching the given tag.
    fn ev_do_floor_by_tag(&mut self, tag: i16, floor_type: FloorType) -> bool;
    /// Signal level exit (used by A_BossDeath and A_BrainDie).
    fn g_exit_level(&mut self);

    // --- Archvile corpse search ---
    /// Search the blockmap around the vile's position for a raiseable corpse.
    /// If found, prepares the corpse (sets momx/momy=0, height check) and
    /// returns its index. Returns `None` if no corpse can be raised.
    fn find_vile_corpse(&mut self, vile_x: Fixed, vile_y: Fixed) -> Option<usize>;

    // --- P_LookForPlayers: check for targets ---
    fn p_look_for_players_impl(&mut self, actor_idx: usize, allaround: bool) -> bool;
}

// ============================================================================
// Direction system — translated from p_enemy.c lines 51-79
// ============================================================================

/// Opposite direction for each direction (for turnaround avoidance).
/// Index by Direction as i32 (East=0..NoDir=8).
///
/// Original C: `opposite[]` (p_enemy.c:53-63)
const OPPOSITE: [i32; 9] = [
    4, // East    -> West
    5, // NE      -> SW
    6, // North   -> South
    7, // NW      -> SE
    0, // West    -> East
    1, // SW      -> NE
    2, // South   -> North
    3, // SE      -> NW
    8, // NoDir   -> NoDir
];

/// Diagonal direction lookup: indexed by `((deltay < 0) as usize) * 2 + (deltax > 0) as usize`.
///
/// Original C: `diags[]` (p_enemy.c:65-70)
const DIAGS: [i32; 4] = [
    3, // NorthWest  (deltay >= 0, deltax <= 0)
    1, // NorthEast  (deltay >= 0, deltax > 0)
    5, // SouthWest  (deltay < 0,  deltax <= 0)
    7, // SouthEast  (deltay < 0,  deltax > 0)
];

/// Direction constants matching Direction enum values.
#[allow(dead_code)]
const DI_EAST: i32 = 0;
#[allow(dead_code)]
const DI_NORTHEAST: i32 = 1;
const DI_NORTH: i32 = 2;
#[allow(dead_code)]
const DI_NORTHWEST: i32 = 3;
const DI_WEST: i32 = 4;
#[allow(dead_code)]
const DI_SOUTHWEST: i32 = 5;
const DI_SOUTH: i32 = 6;
#[allow(dead_code)]
const DI_SOUTHEAST: i32 = 7;
const DI_NODIR: i32 = 8;

// ============================================================================
// Movement speed tables — p_enemy.c lines 264-265
// ============================================================================

/// X movement speed per direction (fixed-point).
/// Original C: `fixed_t xspeed[8]`
const XSPEED: [Fixed; 8] = [
    Fixed(FRACUNIT),  // East
    Fixed(47000),     // NorthEast (≈ FRACUNIT * cos(45°))
    Fixed(0),         // North
    Fixed(-47000),    // NorthWest
    Fixed(-FRACUNIT), // West
    Fixed(-47000),    // SouthWest
    Fixed(0),         // South
    Fixed(47000),     // SouthEast
];

/// Y movement speed per direction (fixed-point).
/// Original C: `fixed_t yspeed[8]`
const YSPEED: [Fixed; 8] = [
    Fixed(0),         // East
    Fixed(47000),     // NorthEast
    Fixed(FRACUNIT),  // North
    Fixed(47000),     // NorthWest
    Fixed(0),         // West
    Fixed(-47000),    // SouthWest
    Fixed(-FRACUNIT), // South
    Fixed(-47000),    // SouthEast
];

// ============================================================================
// Constants
// ============================================================================

/// Homing missile turn rate per adjustment.
/// Original C: `#define TRACEANGLE (0xc000000)` (p_enemy.c line 922)
const TRACEANGLE: u32 = 0x0c000000;

/// Mancubus attack spread angle = ANG90 / 8.
/// Original C: `#define FATSPREAD (ANG90/8)` (p_enemy.c line 1124)
const FATSPREAD: Angle = Angle(ANG90.0 / 8);

/// Lost Soul flying attack speed = 20 * FRACUNIT.
/// Original C: `#define SKULLSPEED (20*FRACUNIT)` (p_enemy.c line 1290)
const SKULLSPEED: Fixed = Fixed(20 * FRACUNIT);

// ============================================================================
// Brain boss global state — p_enemy.c lines 1668-1672
// ============================================================================

/// Maximum number of boss target spots.
const MAX_BRAIN_TARGETS: usize = 32;

/// Brain boss state — consolidates the C static globals for Icon of Sin logic.
/// Replaces the four `static mut` variables (braintargets, numbraintargets,
/// braintargeton, easy) with a safe struct accessed via thread_local.
struct BrainState {
    /// Arena indices of MT_BOSSTARGET map objects.
    targets: [Option<usize>; MAX_BRAIN_TARGETS],
    /// Number of valid entries in `targets`.
    num_targets: usize,
    /// Index of the next brain target to fire at (cycles through targets).
    target_on: usize,
    /// Easy-mode spit toggle: alternates 0/1 each call to A_BrainSpit.
    /// On skill ≤ Easy, every other spit is skipped.
    easy: i32,
}

impl BrainState {
    /// Create a zeroed/default brain state.
    const fn new() -> Self {
        Self {
            targets: [None; MAX_BRAIN_TARGETS],
            num_targets: 0,
            target_on: 0,
            easy: 0,
        }
    }
}

thread_local! {
    /// Thread-local brain state for the Icon of Sin boss fight.
    static BRAIN: RefCell<BrainState> = const { RefCell::new(BrainState::new()) };
}

// ============================================================================
// Sound Propagation — p_enemy.c lines 97-166
// ============================================================================

/// Recursively propagate sound through sectors via two-sided lines.
///
/// Translated from `P_RecursiveSound` (p_enemy.c:97-155).
/// Sound propagation is limited by:
/// - Closed doors (openrange ≤ 0)
/// - Sound-blocking lines (ML_SOUNDBLOCK) — allows one additional step
/// - Already-visited sectors (soundtraversed check)
pub fn p_recursive_sound(
    sector_idx: usize,
    soundblocks: i32,
    sound_target: Option<usize>,
    validcount: i32,
    ctx: &mut dyn EnemyContext,
) {
    // Increment soundtraversed to mark visited
    let already_traversed = {
        let sec = &ctx.sectors()[sector_idx];
        if sec.validcount == validcount && sec.soundtraversed <= soundblocks + 1 {
            return; // already flooded equal or better
        }
        false
    };
    let _ = already_traversed;

    {
        let sec = &mut ctx.sectors_mut()[sector_idx];
        sec.validcount = validcount;
        sec.soundtraversed = soundblocks + 1;
        sec.soundtarget = sound_target;
    }

    // Iterate lines of this sector
    // We need to collect the line indices first to avoid borrow issues
    let num_lines = ctx.lines().len();
    let _num_sides = ctx.sides().len();

    // Collect lines belonging to this sector
    // In the original C, each sector has a `lines` pointer array.
    // In our Rust port, we iterate all lines and check if they border this sector.
    // This is less efficient but correct; the concrete implementation may optimize.
    let mut line_indices: Vec<usize> = Vec::new();
    for i in 0..num_lines {
        let line = &ctx.lines()[i];
        let front_sec = line.frontsector;
        let back_sec = line.backsector;
        if front_sec == Some(sector_idx) || back_sec == Some(sector_idx) {
            line_indices.push(i);
        }
    }

    for line_idx in line_indices {
        let (has_two_sides, has_soundblock, front_sec, back_sec) = {
            let line = &ctx.lines()[line_idx];
            let two_sided = (line.flags & LineFlags::ML_TWOSIDED.bits()) != 0;
            let soundblock = (line.flags & LineFlags::ML_SOUNDBLOCK.bits()) != 0;
            (two_sided, soundblock, line.frontsector, line.backsector)
        };

        if !has_two_sides {
            continue;
        }

        // Determine the other sector
        let other_sector = if front_sec == Some(sector_idx) {
            back_sec
        } else {
            front_sec
        };

        let other_idx = match other_sector {
            Some(idx) => idx,
            None => continue,
        };

        // Check if door is closed (openrange <= 0)
        // We compute the opening from sector heights
        let door_closed = {
            let sectors = ctx.sectors();
            let front = &sectors[sector_idx];
            let other = &sectors[other_idx];
            let top = if front.ceilingheight < other.ceilingheight {
                front.ceilingheight
            } else {
                other.ceilingheight
            };
            let bottom = if front.floorheight > other.floorheight {
                front.floorheight
            } else {
                other.floorheight
            };
            (top.0 - bottom.0) <= 0
        };

        if door_closed {
            continue;
        }

        // If the line has ML_SOUNDBLOCK, increment soundblocks
        if has_soundblock {
            if soundblocks == 0 {
                p_recursive_sound(other_idx, 1, sound_target, validcount, ctx);
            }
            // soundblocks >= 1 means we've already passed through one blocker
        } else {
            p_recursive_sound(other_idx, soundblocks, sound_target, validcount, ctx);
        }
    }
}

/// Alert monsters in hearing range that a noise has been made.
///
/// Translated from `P_NoiseAlert` (p_enemy.c:157-166).
/// Sets the sound target and recursively floods adjacent sectors.
pub fn p_noise_alert(
    target_idx: usize,
    emitter_idx: usize,
    validcount: &mut i32,
    ctx: &mut dyn EnemyContext,
) {
    // Determine emitter's sector
    let emitter_sector = {
        let mo = &ctx.mobjs()[emitter_idx];
        match mo.subsector {
            Some(ss_idx) => ctx.subsectors()[ss_idx].sector,
            None => {
                tracing::error!("p_noise_alert: emitter has no subsector");
                return;
            }
        }
    };

    *validcount += 1;
    p_recursive_sound(emitter_sector, 0, Some(target_idx), *validcount, ctx);
}

// ============================================================================
// Range Checks — p_enemy.c lines 174-256
// ============================================================================

/// Check if actor is in melee range of its target.
///
/// Translated from `P_CheckMeleeRange` (p_enemy.c:174-201).
pub fn p_check_melee_range(actor_idx: usize, ctx: &mut dyn EnemyContext) -> bool {
    let (target_idx, dist, has_sight_target) = {
        let actor = &ctx.mobjs()[actor_idx];
        let target_idx = match actor.target {
            Some(t) => t,
            None => return false,
        };
        let target = &ctx.mobjs()[target_idx];
        let dx = Fixed((actor.x.0).wrapping_sub(target.x.0)).0.wrapping_abs();
        let dy = Fixed((actor.y.0).wrapping_sub(target.y.0)).0.wrapping_abs();
        let dist = p_aprox_distance(Fixed(dx), Fixed(dy));

        let _info_idx = actor.type_;
        let target_info_idx = target.type_;
        let melee_threshold =
            Fixed(MELEERANGE.0 - 20 * FRACUNIT + MOBJINFO[target_info_idx].radius);

        if dist.0 >= melee_threshold.0 {
            return false;
        }
        (target_idx, dist, true)
    };

    let _ = (dist, has_sight_target);

    // Must have line of sight
    ctx.p_check_sight(actor_idx, target_idx)
}

/// Check if actor should fire a missile at its target.
///
/// Translated from `P_CheckMissileRange` (p_enemy.c:210-256).
pub fn p_check_missile_range(actor_idx: usize, ctx: &mut dyn EnemyContext) -> bool {
    let (target_idx, actor_type, _has_melee, dist_val) = {
        let actor = &ctx.mobjs()[actor_idx];

        // If MF_JUSTHIT is set, the target just hit us — fire back immediately
        if actor.flags.contains(MobjFlags::MF_JUSTHIT) {
            // Clear the flag and return true (fire!)
            // We need mutable access, handle below
            let target_idx = match actor.target {
                Some(t) => t,
                None => return false,
            };
            let _actor_type = actor.type_;
            // Check sight first
            if !ctx.p_check_sight(actor_idx, target_idx) {
                return false;
            }
            // Now clear the flag
            ctx.mobjs_mut()[actor_idx]
                .flags
                .remove(MobjFlags::MF_JUSTHIT);
            return true;
        }

        let target_idx = match actor.target {
            Some(t) => t,
            None => return false,
        };

        if actor.reactiontime > 0 {
            return false;
        }

        let target = &ctx.mobjs()[target_idx];
        let dx = Fixed((actor.x.0).wrapping_sub(target.x.0));
        let dy = Fixed((actor.y.0).wrapping_sub(target.y.0));
        let mut dist = p_aprox_distance(dx, dy).0 - 64 * FRACUNIT;

        let info = &MOBJINFO[actor.type_];
        if info.meleestate == StateNum::S_NULL {
            dist -= 128 * FRACUNIT; // no melee: increase effective distance
        }

        dist >>= FRACBITS; // convert to map units

        (
            target_idx,
            actor.type_,
            info.meleestate != StateNum::S_NULL,
            dist,
        )
    };

    let mut dist = dist_val;

    // Type-specific range adjustments
    match MobjType::from_index(actor_type) {
        Some(MobjType::MT_VILE) => {
            if dist > 14 * 64 {
                return false;
            }
        }
        Some(MobjType::MT_UNDEAD) => {
            if dist < 196 {
                return false;
            }
            dist >>= 1;
        }
        Some(MobjType::MT_CYBORG) | Some(MobjType::MT_SPIDER) | Some(MobjType::MT_SKULL) => {
            dist >>= 1;
        }
        _ => {}
    }

    // Clamp maximum distance threshold
    if dist > 200 {
        dist = 200;
    }

    // Special clamp for Cyberdemon and Spider Mastermind
    if matches!(
        MobjType::from_index(actor_type),
        Some(MobjType::MT_CYBORG) | Some(MobjType::MT_SPIDER)
    ) && dist > 160
    {
        dist = 160;
    }

    // Random chance: lower distance = higher fire probability
    let rng_val = ctx.rng_mut().p_random() as i32;
    if rng_val < dist {
        return false;
    }

    // Must have line of sight
    ctx.p_check_sight(actor_idx, target_idx)
}

// ============================================================================
// Monster Movement — p_enemy.c lines 260-490
// ============================================================================

/// Attempt to move an actor in its current `movedir` direction.
///
/// Returns `true` if the move succeeded (or the actor floated).
/// Handles opening doors when blocked (via spechit), and z-adjustment
/// for floating monsters.
///
/// Translated from `P_Move` (p_enemy.c:270-335).
pub fn p_move(actor_idx: usize, ctx: &mut dyn EnemyContext) -> bool {
    let (_movedir, try_x, try_y) = {
        let actor = &ctx.mobjs()[actor_idx];
        let dir = actor.movedir;

        if !(0..=8).contains(&dir) {
            tracing::error!("p_move: impossible movedir {}", dir);
            return false;
        }
        if dir == DI_NODIR {
            return false;
        }

        let info = &MOBJINFO[actor.type_];
        let speed = Fixed(info.speed);

        let try_x = Fixed(
            actor
                .x
                .0
                .wrapping_add(speed.fixed_mul(XSPEED[dir as usize]).0),
        );
        let try_y = Fixed(
            actor
                .y
                .0
                .wrapping_add(speed.fixed_mul(YSPEED[dir as usize]).0),
        );

        (dir, try_x, try_y)
    };

    let good = ctx.p_try_move(actor_idx, try_x, try_y);

    if !good {
        // Floating monsters: try vertical adjustment if floatok
        let is_float = ctx.mobjs()[actor_idx].flags.contains(MobjFlags::MF_FLOAT);
        let floatok = ctx.movement_floatok();
        if is_float && floatok {
            let (actor_z, target_z) = {
                let actor = &ctx.mobjs()[actor_idx];
                let target_z = match actor.target {
                    Some(t) => ctx.mobjs()[t].z,
                    None => actor.z,
                };
                (actor.z, target_z)
            };
            if actor_z.0 < target_z.0 {
                ctx.mobjs_mut()[actor_idx].z =
                    Fixed(ctx.mobjs()[actor_idx].z.0.wrapping_add(FLOATSPEED.0));
            } else {
                ctx.mobjs_mut()[actor_idx].z =
                    Fixed(ctx.mobjs()[actor_idx].z.0.wrapping_sub(FLOATSPEED.0));
            }
            ctx.mobjs_mut()[actor_idx]
                .flags
                .insert(MobjFlags::MF_INFLOAT);
            return true;
        }

        // Try activating special lines (doors, etc.)
        let num_spechit = ctx.movement_numspechit();
        if num_spechit == 0 {
            return false;
        }

        ctx.mobjs_mut()[actor_idx].movedir = DI_NODIR;
        let mut good_line = false;
        let spechit_copy: Vec<usize> = ctx.movement_spechit()[..num_spechit].to_vec();
        for &line_idx in spechit_copy.iter().rev() {
            if ctx.p_use_special_line(actor_idx, line_idx, 0) {
                good_line = true;
            }
        }
        return good_line;
    }

    // Successful move: clear INFLOAT
    ctx.mobjs_mut()[actor_idx]
        .flags
        .remove(MobjFlags::MF_INFLOAT);

    // Non-floating actors: snap z to floor
    if !ctx.mobjs()[actor_idx].flags.contains(MobjFlags::MF_FLOAT) {
        let floor = ctx.mobjs()[actor_idx].floorz;
        ctx.mobjs_mut()[actor_idx].z = floor;
    }

    true
}

/// Attempt to walk in the current movedir. If successful, reset movecount.
///
/// Translated from `P_TryWalk` (p_enemy.c:345-355).
fn p_try_walk(actor_idx: usize, ctx: &mut dyn EnemyContext) -> bool {
    if !p_move(actor_idx, ctx) {
        return false;
    }
    let mc = ctx.rng_mut().p_random() as i32 & 15;
    ctx.mobjs_mut()[actor_idx].movecount = mc;
    true
}

/// Select a new chase direction for an actor pursuing its target.
///
/// Translated from `P_NewChaseDir` (p_enemy.c:363-489).
pub fn p_new_chase_dir(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let (olddir, turnaround, deltax, deltay) = {
        let actor = &ctx.mobjs()[actor_idx];
        let target_idx = match actor.target {
            Some(t) => t,
            None => {
                tracing::error!("p_new_chase_dir: called with no target");
                return;
            }
        };
        let target = &ctx.mobjs()[target_idx];

        let olddir = actor.movedir;
        let turnaround = OPPOSITE[olddir as usize];

        let deltax = target.x.0.wrapping_sub(actor.x.0);
        let deltay = target.y.0.wrapping_sub(actor.y.0);
        (olddir, turnaround, deltax, deltay)
    };

    let d_x = if deltax > 10 * FRACUNIT {
        DI_EAST
    } else if deltax < -10 * FRACUNIT {
        DI_WEST
    } else {
        DI_NODIR
    };

    let d_y = if deltay < -10 * FRACUNIT {
        DI_SOUTH
    } else if deltay > 10 * FRACUNIT {
        DI_NORTH
    } else {
        DI_NODIR
    };

    // Try diagonal first
    if d_x != DI_NODIR && d_y != DI_NODIR {
        let diag_idx = ((deltay < 0) as usize) * 2 + (deltax > 0) as usize;
        let diag = DIAGS[diag_idx];
        ctx.mobjs_mut()[actor_idx].movedir = diag;
        if diag != turnaround && p_try_walk(actor_idx, ctx) {
            return;
        }
    }

    // Randomize priority
    let rng_val = ctx.rng_mut().p_random();
    let try_x_first = rng_val > 200 || deltay.abs() > deltax.abs();

    let (d1, d2) = if try_x_first { (d_x, d_y) } else { (d_y, d_x) };

    if d1 != DI_NODIR {
        ctx.mobjs_mut()[actor_idx].movedir = d1;
        if d1 != turnaround && p_try_walk(actor_idx, ctx) {
            return;
        }
    }

    if d2 != DI_NODIR {
        ctx.mobjs_mut()[actor_idx].movedir = d2;
        if d2 != turnaround && p_try_walk(actor_idx, ctx) {
            return;
        }
    }

    // Try old direction
    if olddir != DI_NODIR {
        ctx.mobjs_mut()[actor_idx].movedir = olddir;
        if p_try_walk(actor_idx, ctx) {
            return;
        }
    }

    // Sweep directions
    if rng_val & 1 != 0 {
        for tdir in 0..=7i32 {
            if tdir != turnaround {
                ctx.mobjs_mut()[actor_idx].movedir = tdir;
                if p_try_walk(actor_idx, ctx) {
                    return;
                }
            }
        }
    } else {
        for tdir in (0..=7i32).rev() {
            if tdir != turnaround {
                ctx.mobjs_mut()[actor_idx].movedir = tdir;
                if p_try_walk(actor_idx, ctx) {
                    return;
                }
            }
        }
    }

    // Last resort: turnaround
    if turnaround != DI_NODIR {
        ctx.mobjs_mut()[actor_idx].movedir = turnaround;
        if p_try_walk(actor_idx, ctx) {
            return;
        }
    }

    ctx.mobjs_mut()[actor_idx].movedir = DI_NODIR;
}

/// Scan players to find a new target for the actor.
///
/// Translated from `P_LookForPlayers` (p_enemy.c:498-558).
pub fn p_look_for_players(actor_idx: usize, allaround: bool, ctx: &mut dyn EnemyContext) -> bool {
    ctx.p_look_for_players_impl(actor_idx, allaround)
}

// ============================================================================
// Core AI Actions — p_enemy.c lines 562-776
// ============================================================================

/// A_KeenDie — Commander Keen (MAP32) dies.
/// When all Keens are dead, open tag-666 doors.
///
/// Translated from `A_KeenDie` (p_enemy.c:562-597).
pub fn a_keen_die(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    a_fall(actor_idx, ctx);

    let keen_type = ctx.mobjs()[actor_idx].type_;
    let mobj_indices = ctx.collect_mobj_thinker_indices();
    for &idx in &mobj_indices {
        let mo = &ctx.mobjs()[idx];
        if mo.type_ != keen_type {
            continue;
        }
        if mo.health > 0 {
            return; // at least one Keen alive
        }
    }

    ctx.ev_do_door_by_tag(666, VldoorType::Open);
}

/// A_Look — Stay idle until a player is sighted or a sound is heard.
///
/// Translated from `A_Look` (p_enemy.c:606-680).
pub fn a_look(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.mobjs_mut()[actor_idx].threshold = 0;

    let (sound_target, has_ambush) = {
        let actor = &ctx.mobjs()[actor_idx];
        let sector_idx = match actor.subsector {
            Some(ss) => ctx.subsectors()[ss].sector,
            None => {
                if p_look_for_players(actor_idx, false, ctx) {
                    goto_seesound(actor_idx, ctx);
                }
                return;
            }
        };
        let st = ctx.sectors()[sector_idx].soundtarget;
        (st, actor.flags.contains(MobjFlags::MF_AMBUSH))
    };

    if let Some(targ_idx) = sound_target {
        let is_shootable = ctx.mobjs()[targ_idx]
            .flags
            .contains(MobjFlags::MF_SHOOTABLE);
        if is_shootable {
            ctx.mobjs_mut()[actor_idx].target = Some(targ_idx);
            if has_ambush {
                if ctx.p_check_sight(actor_idx, targ_idx) {
                    goto_seesound(actor_idx, ctx);
                    return;
                }
            } else {
                goto_seesound(actor_idx, ctx);
                return;
            }
        }
    }

    if p_look_for_players(actor_idx, false, ctx) {
        goto_seesound(actor_idx, ctx);
    }
}

/// Helper: play see sound and transition to see state.
fn goto_seesound(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let (actor_type, see_sound, see_state) = {
        let actor = &ctx.mobjs()[actor_idx];
        let info = &MOBJINFO[actor.type_];
        (actor.type_, info.seesound, info.seestate)
    };

    let sfx = match sfx_from_usize(see_sound as usize) {
        Some(s) => s,
        None => SfxEnum::sfx_None,
    };

    // Random see sound variants
    let sfx = match sfx {
        SfxEnum::sfx_posit1 | SfxEnum::sfx_posit2 | SfxEnum::sfx_posit3 => {
            let r = ctx.rng_mut().p_random() as usize;
            match r % 3 {
                0 => SfxEnum::sfx_posit1,
                1 => SfxEnum::sfx_posit2,
                _ => SfxEnum::sfx_posit3,
            }
        }
        SfxEnum::sfx_bgsit1 | SfxEnum::sfx_bgsit2 => {
            let r = ctx.rng_mut().p_random() as usize;
            match r % 2 {
                0 => SfxEnum::sfx_bgsit1,
                _ => SfxEnum::sfx_bgsit2,
            }
        }
        _ => sfx,
    };

    let use_full_volume = matches!(
        MobjType::from_index(actor_type),
        Some(MobjType::MT_SPIDER) | Some(MobjType::MT_CYBORG)
    );

    if sfx != SfxEnum::sfx_None {
        if use_full_volume {
            ctx.s_start_sound(None, sfx);
        } else {
            ctx.s_start_sound(Some(actor_idx), sfx);
        }
    }

    ctx.p_set_mobj_state(actor_idx, see_state);
}

/// A_Chase — Main monster chase/pursuit behavior.
///
/// Translated from `A_Chase` (p_enemy.c:693-776).
pub fn a_chase(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    // Decrement reactiontime
    if ctx.mobjs()[actor_idx].reactiontime > 0 {
        ctx.mobjs_mut()[actor_idx].reactiontime -= 1;
    }

    // Adjust threshold — decrement regardless of condition (preserves original behavior)
    if ctx.mobjs()[actor_idx].threshold > 0 {
        ctx.mobjs_mut()[actor_idx].threshold -= 1;
    }

    // Turn toward movedir (snap by ANG90/2)
    {
        let movedir = ctx.mobjs()[actor_idx].movedir;
        if movedir < 8 {
            let target_angle = Angle((movedir as u32) << 29);
            let actor_angle = ctx.mobjs()[actor_idx].angle;
            let delta = target_angle.0.wrapping_sub(actor_angle.0);
            let step = ANG90.0 / 2;
            if delta != 0 {
                if delta < 0x80000000 {
                    // Turn counter-clockwise
                    if delta > step {
                        ctx.mobjs_mut()[actor_idx].angle = Angle(actor_angle.0.wrapping_add(step));
                    } else {
                        ctx.mobjs_mut()[actor_idx].angle = target_angle;
                    }
                } else {
                    // Turn clockwise
                    let neg_delta = 0u32.wrapping_sub(delta);
                    if neg_delta > step {
                        ctx.mobjs_mut()[actor_idx].angle = Angle(actor_angle.0.wrapping_sub(step));
                    } else {
                        ctx.mobjs_mut()[actor_idx].angle = target_angle;
                    }
                }
            }
        }
    }

    // Check target validity
    let target_valid = {
        let actor = &ctx.mobjs()[actor_idx];
        match actor.target {
            Some(t) => {
                let target = &ctx.mobjs()[t];
                target.flags.contains(MobjFlags::MF_SHOOTABLE) && target.health > 0
            }
            None => false,
        }
    };

    if !target_valid {
        if p_look_for_players(actor_idx, true, ctx) {
            return;
        }
        let spawn_state = MOBJINFO[ctx.mobjs()[actor_idx].type_].spawnstate;
        ctx.p_set_mobj_state(actor_idx, spawn_state);
        return;
    }

    // MF_JUSTATTACKED
    if ctx.mobjs()[actor_idx]
        .flags
        .contains(MobjFlags::MF_JUSTATTACKED)
    {
        ctx.mobjs_mut()[actor_idx]
            .flags
            .remove(MobjFlags::MF_JUSTATTACKED);
        if ctx.gameskill() != Skill::Nightmare && !ctx.respawnmonsters() {
            p_new_chase_dir(actor_idx, ctx);
        }
        return;
    }

    // Check melee range
    let actor_type = ctx.mobjs()[actor_idx].type_;
    let melee_state = MOBJINFO[actor_type].meleestate;
    if melee_state != StateNum::S_NULL && p_check_melee_range(actor_idx, ctx) {
        let attack_sound = MOBJINFO[actor_type].attacksound;
        if let Some(sfx) = sfx_from_usize(attack_sound as usize) {
            if sfx != SfxEnum::sfx_None {
                ctx.s_start_sound(Some(actor_idx), sfx);
            }
        }
        ctx.p_set_mobj_state(actor_idx, melee_state);
        return;
    }

    // Check missile range
    let missile_state = MOBJINFO[actor_type].missilestate;
    if missile_state != StateNum::S_NULL {
        let skill_gate = ctx.gameskill() != Skill::Nightmare && !ctx.respawnmonsters();
        let movecount = ctx.mobjs()[actor_idx].movecount;
        if (!skill_gate || movecount == 0) && p_check_missile_range(actor_idx, ctx) {
            ctx.p_set_mobj_state(actor_idx, missile_state);
            ctx.mobjs_mut()[actor_idx]
                .flags
                .insert(MobjFlags::MF_JUSTATTACKED);
            return;
        }
    }

    // Multiplayer: retarget if current target out of sight
    if ctx.netgame() && ctx.mobjs()[actor_idx].threshold == 0 {
        let target_idx = ctx.mobjs()[actor_idx].target.unwrap_or(0);
        if !ctx.p_check_sight(actor_idx, target_idx) && p_look_for_players(actor_idx, true, ctx) {
            return;
        }
    }

    // Movement
    let movecount = ctx.mobjs()[actor_idx].movecount;
    ctx.mobjs_mut()[actor_idx].movecount -= 1;
    if movecount <= 0 || !p_move(actor_idx, ctx) {
        p_new_chase_dir(actor_idx, ctx);
    }

    // Active sound (3/256 chance)
    let active_sound = MOBJINFO[ctx.mobjs()[actor_idx].type_].activesound;
    if let Some(sfx) = sfx_from_usize(active_sound as usize) {
        if sfx != SfxEnum::sfx_None {
            let r = ctx.rng_mut().p_random();
            if r < 3 {
                ctx.s_start_sound(Some(actor_idx), sfx);
            }
        }
    }
}

/// A_FaceTarget — Turn to face the actor's target.
///
/// Translated from `A_FaceTarget` (p_enemy.c:780-800).
pub fn a_face_target(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.mobjs_mut()[actor_idx]
        .flags
        .remove(MobjFlags::MF_AMBUSH);

    let target_idx = match ctx.mobjs()[actor_idx].target {
        Some(t) => t,
        None => return,
    };

    let angle = {
        let actor = &ctx.mobjs()[actor_idx];
        let target = &ctx.mobjs()[target_idx];
        point_to_angle2(actor.x, actor.y, target.x, target.y)
    };

    let target_has_shadow = ctx.mobjs()[target_idx].flags.contains(MobjFlags::MF_SHADOW);

    if target_has_shadow {
        let r1 = ctx.rng_mut().p_random() as i32;
        let r2 = ctx.rng_mut().p_random() as i32;
        let fuzz = ((r1 - r2) as u32) << 21;
        ctx.mobjs_mut()[actor_idx].angle = Angle(angle.0.wrapping_add(fuzz));
    } else {
        ctx.mobjs_mut()[actor_idx].angle = angle;
    }
}

// ============================================================================
// Hitscan Attack Functions — p_enemy.c lines 800-880
// ============================================================================

/// A_PosAttack — Zombieman (former human) attack.
/// Single hitscan shot with random spread.
///
/// Translated from `A_PosAttack` (p_enemy.c:802-825).
pub fn a_pos_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }

    a_face_target(actor_idx, ctx);

    let angle = ctx.mobjs()[actor_idx].angle;
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_pistol);

    let r1 = ctx.rng_mut().p_random() as i32;
    let r2 = ctx.rng_mut().p_random() as i32;
    let spread = Angle(((r1 - r2) as u32) << 20);
    let fire_angle = Angle(angle.0.wrapping_add(spread.0));
    let damage = ((ctx.rng_mut().p_random() as i32 % 5) + 1) * 3;

    let (slope, _target) = ctx.p_aim_line_attack(actor_idx, fire_angle, MISSILERANGE);
    ctx.p_line_attack(actor_idx, fire_angle, MISSILERANGE, slope, damage);
}

/// A_SPosAttack — Shotgunner (former sergeant) attack.
/// Three hitscan shots with random spread.
///
/// Translated from `A_SPosAttack` (p_enemy.c:832-858).
pub fn a_spos_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }

    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_shotgn);
    a_face_target(actor_idx, ctx);

    let bangle = ctx.mobjs()[actor_idx].angle;

    let (slope, _) = ctx.p_aim_line_attack(actor_idx, bangle, MISSILERANGE);

    for _ in 0..3 {
        let r1 = ctx.rng_mut().p_random() as i32;
        let r2 = ctx.rng_mut().p_random() as i32;
        let spread = Angle(((r1 - r2) as u32) << 20);
        let fire_angle = Angle(bangle.0.wrapping_add(spread.0));
        let damage = ((ctx.rng_mut().p_random() as i32 % 5) + 1) * 3;
        ctx.p_line_attack(actor_idx, fire_angle, MISSILERANGE, slope, damage);
    }
}

/// A_CPosAttack — Chaingunner (heavy weapon dude) attack.
/// Single hitscan shot per frame (continuous fire via refire).
///
/// Translated from `A_CPosAttack` (p_enemy.c:863-888).
pub fn a_cpos_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }

    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_shotgn);
    a_face_target(actor_idx, ctx);

    let bangle = ctx.mobjs()[actor_idx].angle;
    let (slope, _) = ctx.p_aim_line_attack(actor_idx, bangle, MISSILERANGE);

    let r1 = ctx.rng_mut().p_random() as i32;
    let r2 = ctx.rng_mut().p_random() as i32;
    let spread = Angle(((r1 - r2) as u32) << 20);
    let fire_angle = Angle(bangle.0.wrapping_add(spread.0));
    let damage = ((ctx.rng_mut().p_random() as i32 % 5) + 1) * 3;
    ctx.p_line_attack(actor_idx, fire_angle, MISSILERANGE, slope, damage);
}

/// A_CPosRefire — Chaingunner refire check.
/// Keeps firing unless target lost or random chance to stop.
///
/// Translated from `A_CPosRefire` (p_enemy.c:890-906).
pub fn a_cpos_refire(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    a_face_target(actor_idx, ctx);

    let r = ctx.rng_mut().p_random() as i32;
    if r < 40 {
        return; // keep firing
    }

    let target_idx = match ctx.mobjs()[actor_idx].target {
        Some(t) => t,
        None => {
            // No target: stop firing → go to see state
            let see_state = MOBJINFO[ctx.mobjs()[actor_idx].type_].seestate;
            ctx.p_set_mobj_state(actor_idx, see_state);
            return;
        }
    };

    let target_dead = ctx.mobjs()[target_idx].health <= 0;
    if target_dead || !ctx.p_check_sight(actor_idx, target_idx) {
        let see_state = MOBJINFO[ctx.mobjs()[actor_idx].type_].seestate;
        ctx.p_set_mobj_state(actor_idx, see_state);
    }
}

/// A_SpidRefire — Spider Mastermind refire check.
/// Like CPosRefire but with 10% chance instead of ~15%.
///
/// Translated from `A_SpidRefire` (p_enemy.c:908-924).
pub fn a_spid_refire(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    a_face_target(actor_idx, ctx);

    let r = ctx.rng_mut().p_random() as i32;
    if r < 10 {
        return; // keep firing
    }

    let target_idx = match ctx.mobjs()[actor_idx].target {
        Some(t) => t,
        None => {
            let see_state = MOBJINFO[ctx.mobjs()[actor_idx].type_].seestate;
            ctx.p_set_mobj_state(actor_idx, see_state);
            return;
        }
    };

    let target_dead = ctx.mobjs()[target_idx].health <= 0;
    if target_dead || !ctx.p_check_sight(actor_idx, target_idx) {
        let see_state = MOBJINFO[ctx.mobjs()[actor_idx].type_].seestate;
        ctx.p_set_mobj_state(actor_idx, see_state);
    }
}

// ============================================================================
// Missile Attack Functions — p_enemy.c lines 926-1120
// ============================================================================

/// A_BspiAttack — Arachnotron attack. Fires MT_ARACHPLAZ missile.
///
/// Translated from `A_BspiAttack` (p_enemy.c:926-932).
pub fn a_bspi_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);
    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_ARACHPLAZ);
}

/// A_TroopAttack — Imp attack. Melee at close range, MT_TROOPSHOT otherwise.
///
/// Translated from `A_TroopAttack` (p_enemy.c:938-957).
pub fn a_troop_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    if p_check_melee_range(actor_idx, ctx) {
        ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_claw);
        let damage = ((ctx.rng_mut().p_random() as i32 % 8) + 1) * 3;
        let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
        ctx.p_damage_mobj(target_idx, Some(actor_idx), Some(actor_idx), damage);
        return;
    }

    // Fire missile
    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_TROOPSHOT);
}

/// A_SargAttack — Demon (Pinky) melee attack.
///
/// Translated from `A_SargAttack` (p_enemy.c:964-979).
pub fn a_sarg_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    if p_check_melee_range(actor_idx, ctx) {
        let damage = ((ctx.rng_mut().p_random() as i32 % 10) + 1) * 4;
        let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
        ctx.p_damage_mobj(target_idx, Some(actor_idx), Some(actor_idx), damage);
    }
}

/// A_HeadAttack — Cacodemon attack. Melee or MT_HEADSHOT.
///
/// Translated from `A_HeadAttack` (p_enemy.c:986-1006).
pub fn a_head_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    if p_check_melee_range(actor_idx, ctx) {
        let damage = ((ctx.rng_mut().p_random() as i32 % 6) + 1) * 10;
        let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
        ctx.p_damage_mobj(target_idx, Some(actor_idx), Some(actor_idx), damage);
        return;
    }

    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_HEADSHOT);
}

/// A_CyberAttack — Cyberdemon attack. Fires MT_ROCKET.
///
/// Translated from `A_CyberAttack` (p_enemy.c:1013-1021).
pub fn a_cyber_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);
    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_ROCKET);
}

/// A_BruisAttack — Baron of Hell / Hell Knight attack. Melee or MT_BRUISERSHOT.
///
/// Translated from `A_BruisAttack` (p_enemy.c:1028-1046).
pub fn a_bruis_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }

    if p_check_melee_range(actor_idx, ctx) {
        ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_claw);
        let damage = ((ctx.rng_mut().p_random() as i32 % 8) + 1) * 10;
        let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
        ctx.p_damage_mobj(target_idx, Some(actor_idx), Some(actor_idx), damage);
        return;
    }

    // Fire missile
    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_BRUISERSHOT);
}

// ============================================================================
// Revenant Functions — p_enemy.c lines 1052-1120
// ============================================================================

/// A_SkelMissile — Revenant fires homing MT_TRACER missile.
///
/// Translated from `A_SkelMissile` (p_enemy.c:1052-1072).
pub fn a_skel_missile(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    // Raise z by 16*FRACUNIT for missile spawn, then restore
    ctx.mobjs_mut()[actor_idx].z = Fixed(ctx.mobjs()[actor_idx].z.0 + 16 * FRACUNIT);
    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    let mo_idx = ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_TRACER);
    ctx.mobjs_mut()[actor_idx].z = Fixed(ctx.mobjs()[actor_idx].z.0 - 16 * FRACUNIT);

    // Set tracer to target for homing
    ctx.mobjs_mut()[mo_idx].tracer = ctx.mobjs()[actor_idx].target;

    // Advance missile position by one tic's worth of momentum
    let momx = ctx.mobjs()[mo_idx].momx;
    let momy = ctx.mobjs()[mo_idx].momy;
    ctx.mobjs_mut()[mo_idx].x = Fixed(ctx.mobjs()[mo_idx].x.0.wrapping_add(momx.0));
    ctx.mobjs_mut()[mo_idx].y = Fixed(ctx.mobjs()[mo_idx].y.0.wrapping_add(momy.0));
}

/// A_Tracer — Homing missile logic (called every 4th gametic).
///
/// Translated from `A_Tracer` (p_enemy.c:1082-1148).
pub fn a_tracer(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    // Only home every 4th tic
    if ctx.gametic() & 3 != 0 {
        return;
    }

    // Spawn smoke trail
    let (mx, my, mz) = {
        let mo = &ctx.mobjs()[actor_idx];
        (mo.x, mo.y, mo.z)
    };
    let _smoke = ctx.p_spawn_puff(mx, my, mz);

    let dest_idx = match ctx.mobjs()[actor_idx].tracer {
        Some(t) => t,
        None => return,
    };

    // Check if target is still alive
    if ctx.mobjs()[dest_idx].health <= 0 {
        return;
    }

    // Adjust angle toward target
    let exact = {
        let mo = &ctx.mobjs()[actor_idx];
        let dest = &ctx.mobjs()[dest_idx];
        point_to_angle2(mo.x, mo.y, dest.x, dest.y)
    };

    let current_angle = ctx.mobjs()[actor_idx].angle;
    if exact.0 != current_angle.0 {
        if exact.0.wrapping_sub(current_angle.0) > ANG180.0 {
            // Turn clockwise
            let new_angle = current_angle.0.wrapping_sub(TRACEANGLE);
            ctx.mobjs_mut()[actor_idx].angle = Angle(new_angle);
            if exact.0.wrapping_sub(new_angle) < ANG180.0 {
                ctx.mobjs_mut()[actor_idx].angle = exact;
            }
        } else {
            // Turn counter-clockwise
            let new_angle = current_angle.0.wrapping_add(TRACEANGLE);
            ctx.mobjs_mut()[actor_idx].angle = Angle(new_angle);
            if exact.0.wrapping_sub(new_angle) > ANG180.0 {
                ctx.mobjs_mut()[actor_idx].angle = exact;
            }
        }
    }

    // Update velocity from new angle
    let final_angle = ctx.mobjs()[actor_idx].angle;
    let fine = (final_angle.0 >> ANGLETOFINESHIFT) as usize;
    let speed = {
        let info = &MOBJINFO[ctx.mobjs()[actor_idx].type_];
        Fixed(info.speed)
    };
    ctx.mobjs_mut()[actor_idx].momx = speed.fixed_mul(finecosine(fine));
    ctx.mobjs_mut()[actor_idx].momy = speed.fixed_mul(FINESINE[fine]);

    // Adjust slope toward target
    let dist = {
        let mo = &ctx.mobjs()[actor_idx];
        let dest = &ctx.mobjs()[dest_idx];
        let dx = Fixed(dest.x.0.wrapping_sub(mo.x.0));
        let dy = Fixed(dest.y.0.wrapping_sub(mo.y.0));
        let d = p_aprox_distance(dx, dy);
        // Prevent division by zero
        let tics = d.0 / speed.0;
        if tics < 1 {
            1
        } else {
            tics
        }
    };

    let slope = {
        let dest = &ctx.mobjs()[dest_idx];
        let mo = &ctx.mobjs()[actor_idx];
        Fixed((dest.z.0.wrapping_add(40 * FRACUNIT).wrapping_sub(mo.z.0)) / dist)
    };

    let momz = ctx.mobjs()[actor_idx].momz;
    if slope.0 < momz.0 {
        ctx.mobjs_mut()[actor_idx].momz = Fixed(momz.0 - FRACUNIT / 8);
    } else {
        ctx.mobjs_mut()[actor_idx].momz = Fixed(momz.0 + FRACUNIT / 8);
    }
}

/// A_SkelWhoosh — Revenant melee whoosh sound.
///
/// Translated from `A_SkelWhoosh` (p_enemy.c:1150-1157).
pub fn a_skel_whoosh(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_skeswg);
}

/// A_SkelFist — Revenant melee punch.
///
/// Translated from `A_SkelFist` (p_enemy.c:1163-1179).
pub fn a_skel_fist(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    if p_check_melee_range(actor_idx, ctx) {
        let damage = ((ctx.rng_mut().p_random() as i32 % 10) + 1) * 6;
        ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_skeatk);
        let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
        ctx.p_damage_mobj(target_idx, Some(actor_idx), Some(actor_idx), damage);
    }
}

// ============================================================================
// Archvile Functions — p_enemy.c lines 1185-1290
// ============================================================================

/// A_VileChase — Archvile chase with corpse resurrection check.
///
/// Translated from `A_VileChase` (p_enemy.c:1222-1280).
pub fn a_vile_chase(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    // Check for corpses to resurrect
    let (vile_x, vile_y) = {
        let actor = &ctx.mobjs()[actor_idx];
        (actor.x, actor.y)
    };

    if let Some(corpse_idx) = ctx.find_vile_corpse(vile_x, vile_y) {
        // Found a raiseable corpse — raise it!
        let (cx, cy) = {
            let corpse = &ctx.mobjs()[corpse_idx];
            (corpse.x, corpse.y)
        };

        // Turn toward corpse
        let new_angle = point_to_angle2(ctx.mobjs()[actor_idx].x, ctx.mobjs()[actor_idx].y, cx, cy);
        ctx.mobjs_mut()[actor_idx].angle = new_angle;

        // Set archvile to heal state
        let heal_state = StateNum::S_VILE_HEAL1;
        ctx.p_set_mobj_state(actor_idx, heal_state);
        ctx.s_start_sound(Some(corpse_idx), SfxEnum::sfx_slop);

        // Restore corpse
        let raise_state = {
            let corpse = &ctx.mobjs()[corpse_idx];
            let info = &MOBJINFO[corpse.type_];
            info.raisestate
        };

        ctx.p_set_mobj_state(corpse_idx, raise_state);

        // Restore corpse properties
        {
            let info_idx = ctx.mobjs()[corpse_idx].type_;
            let info = &MOBJINFO[info_idx];
            ctx.mobjs_mut()[corpse_idx].height = Fixed(info.height);
            ctx.mobjs_mut()[corpse_idx].radius = Fixed(info.radius);
            ctx.mobjs_mut()[corpse_idx].flags = MobjFlags::from_bits_truncate(info.flags);
            ctx.mobjs_mut()[corpse_idx].health = info.spawnhealth;
            ctx.mobjs_mut()[corpse_idx].target = None;
        }
        return;
    }

    // No corpse found — do normal chase
    a_chase(actor_idx, ctx);
}

/// A_VileStart — Archvile begins attack, plays sfx_vilatk.
///
/// Translated from `A_VileStart` (p_enemy.c:1286-1292).
pub fn a_vile_start(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_vilatk);
}

/// A_VileTarget — Spawn fire at target position.
///
/// **BUG PRESERVED**: Original uses target->x for BOTH x and y of spawn.
/// `fog = P_SpawnMobj(actor->target->x, actor->target->x, actor->target->z, MT_FIRE);`
///
/// Translated from `A_VileTarget` (p_enemy.c:1295-1312).
pub fn a_vile_target(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    // BUG: x used for both x and y — this is the original bug, preserved exactly
    let (tx, tz) = {
        let target = &ctx.mobjs()[target_idx];
        (target.x, target.z)
    };

    let fog_idx = ctx.p_spawn_mobj(tx, tx, tz, MobjType::MT_FIRE);
    ctx.mobjs_mut()[actor_idx].tracer = Some(fog_idx);
    ctx.mobjs_mut()[fog_idx].target = Some(actor_idx);
    ctx.mobjs_mut()[fog_idx].tracer = ctx.mobjs()[actor_idx].target;

    a_fire(fog_idx, ctx);
}

/// A_VileAttack — Archvile attack: damage, vertical knockback, radius blast.
///
/// Translated from `A_VileAttack` (p_enemy.c:1318-1358).
pub fn a_vile_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let target_idx = match ctx.mobjs()[actor_idx].target {
        Some(t) => t,
        None => return,
    };

    a_face_target(actor_idx, ctx);

    // Must have line of sight
    if !ctx.p_check_sight(actor_idx, target_idx) {
        return;
    }

    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_barexp);

    // 20 damage to target
    ctx.p_damage_mobj(target_idx, Some(actor_idx), Some(actor_idx), 20);

    // Vertical knockback: momz = 1000 * FRACUNIT / target.mass
    let mass = MOBJINFO[ctx.mobjs()[target_idx].type_].mass;
    let knockback = if mass > 0 {
        Fixed(1000 * FRACUNIT / mass)
    } else {
        Fixed(0)
    };
    ctx.mobjs_mut()[target_idx].momz = knockback;

    // Radius attack centered on the fire
    let fire_idx = match ctx.mobjs()[actor_idx].tracer {
        Some(f) => f,
        None => return,
    };

    // Move fire to target position for the blast
    let (tx, ty) = {
        let target = &ctx.mobjs()[target_idx];
        (target.x, target.y)
    };

    let (_fire_angle, fire_offset_x, fire_offset_y) = {
        let fire_angle = ctx.mobjs()[actor_idx].angle;
        let fine = (fire_angle.0 >> ANGLETOFINESHIFT) as usize;
        let offset = 24 * FRACUNIT;
        (
            fire_angle,
            Fixed(finecosine(fine).0.wrapping_mul(offset) >> FRACBITS),
            Fixed(FINESINE[fine].0.wrapping_mul(offset) >> FRACBITS),
        )
    };

    ctx.mobjs_mut()[fire_idx].x = Fixed(tx.0.wrapping_add(fire_offset_x.0));
    ctx.mobjs_mut()[fire_idx].y = Fixed(ty.0.wrapping_add(fire_offset_y.0));

    ctx.p_radius_attack(fire_idx, actor_idx, 70);
}

/// A_StartFire — Start fire animation, then move fire.
///
/// Translated from `A_StartFire` (p_enemy.c:1361-1367).
pub fn a_start_fire(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_flamst);
    a_fire(actor_idx, ctx);
}

/// A_Fire — Move fire to a position ahead of the archvile's target.
///
/// Translated from `A_Fire` (p_enemy.c:1373-1397).
pub fn a_fire(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let dest_idx = match ctx.mobjs()[actor_idx].tracer {
        Some(t) => t,
        None => return,
    };

    let source_idx = match ctx.mobjs()[actor_idx].target {
        Some(s) => s,
        None => return,
    };

    // Check if archvile still has sight to target
    if !ctx.p_check_sight(source_idx, dest_idx) {
        return;
    }

    // Position fire at target + offset from source's angle
    let (dx, dy, dz) = {
        let dest = &ctx.mobjs()[dest_idx];
        (dest.x, dest.y, dest.z)
    };

    let (offset_x, offset_y) = {
        let angle = ctx.mobjs()[source_idx].angle;
        let fine = (angle.0 >> ANGLETOFINESHIFT) as usize;
        (
            finecosine(fine).fixed_mul(Fixed(24 * FRACUNIT)),
            FINESINE[fine].fixed_mul(Fixed(24 * FRACUNIT)),
        )
    };

    ctx.mobjs_mut()[actor_idx].x = Fixed(dx.0.wrapping_add(offset_x.0));
    ctx.mobjs_mut()[actor_idx].y = Fixed(dy.0.wrapping_add(offset_y.0));
    ctx.mobjs_mut()[actor_idx].z = dz;
}

/// A_FireCrackle — Fire crackle sound, then move fire.
///
/// Translated from `A_FireCrackle` (p_enemy.c:1401-1407).
pub fn a_fire_crackle(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_flame);
    a_fire(actor_idx, ctx);
}

// ============================================================================
// Mancubus Functions — p_enemy.c lines 1124-1186
// ============================================================================

/// A_FatRaise — Mancubus raises for attack.
///
/// Translated from `A_FatRaise` (p_enemy.c:1128-1134).
pub fn a_fat_raise(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    a_face_target(actor_idx, ctx);
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_manatk);
}

/// A_FatAttack1 — Mancubus first volley: two shots at ±FATSPREAD.
///
/// Translated from `A_FatAttack1` (p_enemy.c:1140-1156).
pub fn a_fat_attack1(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    // Adjust angle + FATSPREAD
    let angle = ctx.mobjs()[actor_idx].angle;
    ctx.mobjs_mut()[actor_idx].angle = Angle(angle.0.wrapping_add(FATSPREAD.0));
    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_FATSHOT);

    // Adjust angle - FATSPREAD (net = original angle + FATSPREAD - FATSPREAD)
    // But original code adds FATSPREAD to actor angle before second missile too...
    // Actually, original does: an += FATSPREAD, spawn, an -= 2*FATSPREAD, spawn
    ctx.mobjs_mut()[actor_idx].angle = Angle(angle.0.wrapping_sub(FATSPREAD.0));
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_FATSHOT);

    // Restore angle
    ctx.mobjs_mut()[actor_idx].angle = angle;
}

/// A_FatAttack2 — Mancubus second volley: two shots biased right.
///
/// Translated from `A_FatAttack2` (p_enemy.c:1160-1176).
pub fn a_fat_attack2(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    let angle = ctx.mobjs()[actor_idx].angle;
    // First shot: -FATSPREAD
    ctx.mobjs_mut()[actor_idx].angle = Angle(angle.0.wrapping_sub(FATSPREAD.0));
    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_FATSHOT);

    // Second shot: -FATSPREAD/2
    ctx.mobjs_mut()[actor_idx].angle = Angle(angle.0.wrapping_sub(FATSPREAD.0 / 2));
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_FATSHOT);

    // Restore
    ctx.mobjs_mut()[actor_idx].angle = angle;
}

/// A_FatAttack3 — Mancubus third volley: two shots biased left.
///
/// Translated from `A_FatAttack3` (p_enemy.c:1180-1196).
pub fn a_fat_attack3(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);

    let angle = ctx.mobjs()[actor_idx].angle;
    // First shot: +FATSPREAD/2
    ctx.mobjs_mut()[actor_idx].angle = Angle(angle.0.wrapping_add(FATSPREAD.0 / 2));
    let target_idx = ctx.mobjs()[actor_idx].target.unwrap();
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_FATSHOT);

    // Second shot: -FATSPREAD/2
    ctx.mobjs_mut()[actor_idx].angle = Angle(angle.0.wrapping_sub(FATSPREAD.0 / 2));
    ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_FATSHOT);

    // Restore
    ctx.mobjs_mut()[actor_idx].angle = angle;
}

// ============================================================================
// Lost Soul / Pain Elemental — p_enemy.c lines 1290-1530
// ============================================================================

/// A_SkullAttack — Lost Soul flying attack.
///
/// Translated from `A_SkullAttack` (p_enemy.c:1298-1330).
pub fn a_skull_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let target_idx = match ctx.mobjs()[actor_idx].target {
        Some(t) => t,
        None => return,
    };

    ctx.s_start_sound(Some(actor_idx), {
        let info = &MOBJINFO[ctx.mobjs()[actor_idx].type_];
        match sfx_from_usize(info.attacksound as usize) {
            Some(s) => s,
            None => SfxEnum::sfx_None,
        }
    });

    a_face_target(actor_idx, ctx);

    // Set SKULLFLY flag
    ctx.mobjs_mut()[actor_idx]
        .flags
        .insert(MobjFlags::MF_SKULLFLY);

    // Compute velocity toward target
    let angle = ctx.mobjs()[actor_idx].angle;
    let fine = (angle.0 >> ANGLETOFINESHIFT) as usize;

    ctx.mobjs_mut()[actor_idx].momx = SKULLSPEED.fixed_mul(finecosine(fine));
    ctx.mobjs_mut()[actor_idx].momy = SKULLSPEED.fixed_mul(FINESINE[fine]);

    // Vertical component
    let dist = {
        let actor = &ctx.mobjs()[actor_idx];
        let target = &ctx.mobjs()[target_idx];
        let dx = Fixed(target.x.0.wrapping_sub(actor.x.0));
        let dy = Fixed(target.y.0.wrapping_sub(actor.y.0));
        p_aprox_distance(dx, dy)
    };

    let num = {
        let target = &ctx.mobjs()[target_idx];
        let actor = &ctx.mobjs()[actor_idx];
        target
            .z
            .0
            .wrapping_add(target.height.0 / 2)
            .wrapping_sub(actor.z.0)
    };

    let denom = if dist.0 / SKULLSPEED.0 < 1 {
        1
    } else {
        dist.0 / SKULLSPEED.0
    };

    ctx.mobjs_mut()[actor_idx].momz = Fixed(num / denom);
}

/// Internal helper: spawn a Lost Soul from a Pain Elemental's direction.
///
/// Translated from `A_PainShootSkull` (p_enemy.c:1340-1412).
fn a_pain_shoot_skull(actor_idx: usize, angle: Angle, ctx: &mut dyn EnemyContext) {
    // Count existing skulls — limit of 20
    let mobj_indices = ctx.collect_mobj_thinker_indices();
    let mut skull_count = 0i32;
    for &idx in &mobj_indices {
        if ctx.mobjs()[idx].type_ == MobjType::MT_SKULL as usize {
            skull_count += 1;
        }
        if skull_count >= 20 {
            return; // too many skulls
        }
    }

    // Compute prestep distance
    let (prestep_x, prestep_y, spawn_z) = {
        let actor = &ctx.mobjs()[actor_idx];
        let fine = (angle.0 >> ANGLETOFINESHIFT) as usize;
        let skull_radius = Fixed(MOBJINFO[MobjType::MT_SKULL as usize].radius);
        let prestep = Fixed(4 * FRACUNIT + 3 * (actor.radius.0 + skull_radius.0) / 2);

        let px = Fixed(
            actor
                .x
                .0
                .wrapping_add(prestep.fixed_mul(finecosine(fine)).0),
        );
        let py = Fixed(actor.y.0.wrapping_add(prestep.fixed_mul(FINESINE[fine]).0));
        let pz = Fixed(actor.z.0 + 8 * FRACUNIT);
        (px, py, pz)
    };

    // Spawn the skull
    let skull_idx = ctx.p_spawn_mobj(prestep_x, prestep_y, spawn_z, MobjType::MT_SKULL);

    // Check if it fits
    if !ctx.p_try_move(skull_idx, prestep_x, prestep_y) {
        // Doesn't fit — kill it
        let _info_damage = MOBJINFO[ctx.mobjs()[skull_idx].type_].damage;
        ctx.p_damage_mobj(skull_idx, Some(actor_idx), Some(actor_idx), 10000);
        return;
    }

    // Set target to actor's target
    ctx.mobjs_mut()[skull_idx].target = ctx.mobjs()[actor_idx].target;

    a_skull_attack(skull_idx, ctx);
}

/// A_PainAttack — Pain Elemental shoots a Lost Soul forward.
///
/// Translated from `A_PainAttack` (p_enemy.c:1419-1429).
pub fn a_pain_attack(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    if ctx.mobjs()[actor_idx].target.is_none() {
        return;
    }
    a_face_target(actor_idx, ctx);
    let angle = ctx.mobjs()[actor_idx].angle;
    a_pain_shoot_skull(actor_idx, angle, ctx);
}

/// A_PainDie — Pain Elemental death: shoots 3 skulls in cardinal directions.
///
/// Translated from `A_PainDie` (p_enemy.c:1434-1445).
pub fn a_pain_die(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    a_fall(actor_idx, ctx);

    let angle = ctx.mobjs()[actor_idx].angle;
    a_pain_shoot_skull(actor_idx, Angle(angle.0.wrapping_add(ANG90.0)), ctx);
    a_pain_shoot_skull(actor_idx, Angle(angle.0.wrapping_add(ANG180.0)), ctx);
    a_pain_shoot_skull(actor_idx, Angle(angle.0.wrapping_add(ANG270.0)), ctx);
}

// ============================================================================
// Death / Sound Actions — p_enemy.c lines 1535-1601
// ============================================================================

/// A_Scream — Death scream with random variants for certain types.
///
/// Translated from `A_Scream` (p_enemy.c:1535-1575).
pub fn a_scream(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let info = &MOBJINFO[ctx.mobjs()[actor_idx].type_];
    let death_sound = match sfx_from_usize(info.deathsound as usize) {
        Some(s) => s,
        None => return,
    };

    let sfx = match death_sound {
        SfxEnum::sfx_podth1 | SfxEnum::sfx_podth2 | SfxEnum::sfx_podth3 => {
            let r = ctx.rng_mut().p_random() as usize;
            match r % 3 {
                0 => SfxEnum::sfx_podth1,
                1 => SfxEnum::sfx_podth2,
                _ => SfxEnum::sfx_podth3,
            }
        }
        SfxEnum::sfx_bgdth1 | SfxEnum::sfx_bgdth2 => {
            let r = ctx.rng_mut().p_random() as usize;
            match r % 2 {
                0 => SfxEnum::sfx_bgdth1,
                _ => SfxEnum::sfx_bgdth2,
            }
        }
        _ => death_sound,
    };

    // Full volume for Cyberdemon and Spider Mastermind
    let actor_type = ctx.mobjs()[actor_idx].type_;
    let full_volume = matches!(
        MobjType::from_index(actor_type),
        Some(MobjType::MT_SPIDER) | Some(MobjType::MT_CYBORG)
    );

    if full_volume {
        ctx.s_start_sound(None, sfx);
    } else {
        ctx.s_start_sound(Some(actor_idx), sfx);
    }
}

/// A_XScream — Extreme death scream (gibbed).
///
/// Translated from `A_XScream` (p_enemy.c:1581-1585).
pub fn a_xscream(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_slop);
}

/// A_Pain — Play pain sound.
///
/// Translated from `A_Pain` (p_enemy.c:1589-1596).
pub fn a_pain(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let info = &MOBJINFO[ctx.mobjs()[actor_idx].type_];
    if let Some(sfx) = sfx_from_usize(info.painsound as usize) {
        ctx.s_start_sound(Some(actor_idx), sfx);
    }
}

/// A_Fall — Remove SOLID flag (corpse falls).
///
/// Translated from `A_Fall` (p_enemy.c:1601-1606).
pub fn a_fall(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.mobjs_mut()[actor_idx].flags.remove(MobjFlags::MF_SOLID);
}

/// A_Explode — Radius explosion (barrel, rocket).
///
/// Translated from `A_Explode` (p_enemy.c:1611-1617).
pub fn a_explode(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let source = ctx.mobjs()[actor_idx].target.unwrap_or(actor_idx);
    ctx.p_radius_attack(actor_idx, source, 128);
}

// ============================================================================
// Boss Actions — p_enemy.c lines 1609-1756
// ============================================================================

/// A_BossDeath — Handle boss death triggers per-episode/map.
///
/// Translated from `A_BossDeath` (p_enemy.c:1621-1714).
pub fn a_boss_death(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let gamemode = ctx.gamemode();
    let gamemap = ctx.gamemap();
    let gameepisode = ctx.gameepisode();
    let actor_type = ctx.mobjs()[actor_idx].type_;

    if gamemode == GameMode::Commercial {
        if gamemap != 7 {
            return;
        }
        let mt = MobjType::from_index(actor_type);
        if !matches!(mt, Some(MobjType::MT_FATSO) | Some(MobjType::MT_BABY)) {
            return;
        }
    } else {
        match gameepisode {
            1 => {
                if gamemap != 8 {
                    return;
                }
                if !matches!(MobjType::from_index(actor_type), Some(MobjType::MT_BRUISER)) {
                    return;
                }
            }
            2 => {
                if gamemap != 8 {
                    return;
                }
                if !matches!(MobjType::from_index(actor_type), Some(MobjType::MT_CYBORG)) {
                    return;
                }
            }
            3 => {
                if gamemap != 8 {
                    return;
                }
                if !matches!(MobjType::from_index(actor_type), Some(MobjType::MT_SPIDER)) {
                    return;
                }
            }
            4 => match gamemap {
                6 => {
                    if !matches!(MobjType::from_index(actor_type), Some(MobjType::MT_CYBORG)) {
                        return;
                    }
                }
                8 => {
                    if !matches!(MobjType::from_index(actor_type), Some(MobjType::MT_SPIDER)) {
                        return;
                    }
                }
                _ => return,
            },
            _ => {
                if gamemap != 8 {
                    return;
                }
            }
        }
    }

    // Check that all monsters of this type are dead
    let mobj_indices = ctx.collect_mobj_thinker_indices();
    for &idx in &mobj_indices {
        if idx == actor_idx {
            continue;
        }
        let mo = &ctx.mobjs()[idx];
        if mo.type_ == actor_type && mo.health > 0 {
            return; // not all dead yet
        }
    }

    // All bosses dead — trigger action
    if gamemode == GameMode::Commercial {
        if gamemap == 7 {
            match MobjType::from_index(actor_type) {
                Some(MobjType::MT_FATSO) => {
                    ctx.ev_do_floor_by_tag(666, FloorType::LowerFloorToLowest);
                    return;
                }
                Some(MobjType::MT_BABY) => {
                    ctx.ev_do_floor_by_tag(667, FloorType::RaiseToTexture);
                    return;
                }
                _ => {}
            }
        }
    } else {
        match gameepisode {
            1 => {
                // E1M8: lower floor
                ctx.ev_do_floor_by_tag(666, FloorType::LowerFloorToLowest);
                return;
            }
            2 => {
                // E2M8: lower floor
                ctx.ev_do_floor_by_tag(666, FloorType::LowerFloorToLowest);
                return;
            }
            3 => {
                // E3M8: lower floor
                ctx.ev_do_floor_by_tag(666, FloorType::LowerFloorToLowest);
                return;
            }
            4 => {
                match gamemap {
                    6 => {
                        // E4M6: blaze open doors
                        ctx.ev_do_door_by_tag(666, VldoorType::BlazeOpen);
                        return;
                    }
                    8 => {
                        // E4M8: lower floor
                        ctx.ev_do_floor_by_tag(666, FloorType::LowerFloorToLowest);
                        return;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // Default: exit the level
    ctx.g_exit_level();
}

/// A_Hoof — Cyberdemon hoof sound + chase.
///
/// Translated from `A_Hoof` (p_enemy.c:1720-1727).
pub fn a_hoof(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_hoof);
    a_chase(actor_idx, ctx);
}

/// A_Metal — Spider Mastermind metal clanking + chase.
///
/// Translated from `A_Metal` (p_enemy.c:1732-1739).
pub fn a_metal(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_metal);
    a_chase(actor_idx, ctx);
}

/// A_BabyMetal — Arachnotron walking sound + chase.
///
/// Translated from `A_BabyMetal` (p_enemy.c:1744-1751).
pub fn a_baby_metal(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_bspwlk);
    a_chase(actor_idx, ctx);
}

// ============================================================================
// Shotgun2 Sound Actions — p_enemy.c lines 1756-1805
// ============================================================================

/// A_OpenShotgun2 — SSG open sound.
///
/// Translated from `A_OpenShotgun2` (p_enemy.c:1758-1764).
pub fn a_open_shotgun2(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_dbopn);
}

/// A_LoadShotgun2 — SSG load sound.
///
/// Translated from `A_LoadShotgun2` (p_enemy.c:1769-1775).
pub fn a_load_shotgun2(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_dbload);
}

/// A_CloseShotgun2 — SSG close sound + refire.
///
/// Translated from `A_CloseShotgun2` (p_enemy.c:1780-1788).
pub fn a_close_shotgun2(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_dbcls);
    // Note: A_ReFire is called from the weapon sprite state machine,
    // not directly from here. The original C code calls A_ReFire(player, psp)
    // but that is handled by the player weapon sprite system, not enemy AI.
}

// ============================================================================
// Brain Boss Functions — p_enemy.c lines 1809-2008
// ============================================================================

/// A_BrainAwake — Boss Brain awakens: scan for MT_BOSSTARGET spots.
///
/// Translated from `A_BrainAwake` (p_enemy.c:1812-1832).
pub fn a_brain_awake(_actor_idx: usize, ctx: &mut dyn EnemyContext) {
    // Scan thinkers for MT_BOSSTARGET
    let mobj_indices = ctx.collect_mobj_thinker_indices();

    BRAIN.with(|cell| {
        let mut b = cell.borrow_mut();
        b.num_targets = 0;
        b.targets = [None; MAX_BRAIN_TARGETS];

        for &idx in &mobj_indices {
            if ctx.mobjs()[idx].type_ == MobjType::MT_BOSSTARGET as usize
                && b.num_targets < MAX_BRAIN_TARGETS
            {
                let slot = b.num_targets;
                b.targets[slot] = Some(idx);
                b.num_targets += 1;
            }
        }
    });

    ctx.s_start_sound(None, SfxEnum::sfx_bossit);
}

/// A_BrainPain — Boss Brain pain sound.
///
/// Translated from `A_BrainPain` (p_enemy.c:1838-1843).
pub fn a_brain_pain(_actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(None, SfxEnum::sfx_bospn);
}

/// A_BrainScream — Boss Brain death: spawn rocket explosions in arc.
///
/// Translated from `A_BrainScream` (p_enemy.c:1849-1878).
pub fn a_brain_scream(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let actor_x = ctx.mobjs()[actor_idx].x.0;
    let actor_y = ctx.mobjs()[actor_idx].y.0;

    // Spawn explosions in arc from x-196 to x+320, step 8
    let mut x = actor_x - 196 * FRACUNIT;
    while x < actor_x + 320 * FRACUNIT {
        let y = actor_y - 320 * FRACUNIT;
        let z = 128 + (ctx.rng_mut().p_random() as i32) * 2 * FRACUNIT;

        let th = ctx.p_spawn_mobj(Fixed(x), Fixed(y), Fixed(z), MobjType::MT_ROCKET);
        ctx.mobjs_mut()[th].momz = Fixed((ctx.rng_mut().p_random() as i32) * 512);
        ctx.p_set_mobj_state(th, StateNum::S_BRAINEXPLODE1);

        let tics = ctx.rng_mut().p_random() as i32;
        ctx.mobjs_mut()[th].tics -= tics & 7;
        if ctx.mobjs()[th].tics < 1 {
            ctx.mobjs_mut()[th].tics = 1;
        }

        x += 8 * FRACUNIT;
    }

    ctx.s_start_sound(None, SfxEnum::sfx_bosdth);
}

/// A_BrainExplode — Single random explosion from brain.
///
/// Translated from `A_BrainExplode` (p_enemy.c:1884-1903).
pub fn a_brain_explode(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    let actor_x = ctx.mobjs()[actor_idx].x.0;
    let actor_y = ctx.mobjs()[actor_idx].y.0;

    let r = ctx.rng_mut().p_random() as i32;
    let x = actor_x + (r - (ctx.rng_mut().p_random() as i32)) * 2048;
    let z = 128 + (ctx.rng_mut().p_random() as i32) * 2 * FRACUNIT;

    let th = ctx.p_spawn_mobj(Fixed(x), Fixed(actor_y), Fixed(z), MobjType::MT_ROCKET);
    ctx.mobjs_mut()[th].momz = Fixed((ctx.rng_mut().p_random() as i32) * 512);
    ctx.p_set_mobj_state(th, StateNum::S_BRAINEXPLODE1);

    let tics_sub = ctx.rng_mut().p_random() as i32 & 7;
    ctx.mobjs_mut()[th].tics -= tics_sub;
    if ctx.mobjs()[th].tics < 1 {
        ctx.mobjs_mut()[th].tics = 1;
    }
}

/// A_BrainDie — Boss Brain dies: exit the level.
///
/// Translated from `A_BrainDie` (p_enemy.c:1908-1913).
pub fn a_brain_die(_actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.g_exit_level();
}

/// A_BrainSpit — Boss Brain spits a spawn cube at next target.
///
/// Translated from `A_BrainSpit` (p_enemy.c:1919-1958).
pub fn a_brain_spit(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    // Access brain state via thread_local to determine target and easy-skip.
    let target_idx = BRAIN.with(|cell| {
        let mut b = cell.borrow_mut();

        if b.num_targets == 0 {
            return None;
        }

        // Easy-mode toggle: skip every other spit on skill ≤ Easy
        b.easy ^= 1;
        if ctx.gameskill() as i32 <= Skill::Easy as i32 && b.easy == 0 {
            return None;
        }

        // Get next target, cycling through the target list
        let idx = b.target_on;
        b.target_on = (b.target_on + 1) % b.num_targets;
        b.targets[idx]
    });

    let target_idx = match target_idx {
        Some(t) => t,
        None => return,
    };

    // Spawn the cube
    let (_targ_x, targ_y, _spawn_z) = {
        let target = &ctx.mobjs()[target_idx];
        (
            target.x,
            target.y,
            ctx.mobjs()[actor_idx].z.0 + 16 * FRACUNIT,
        )
    };

    let cube_idx = ctx.p_spawn_missile(actor_idx, target_idx, MobjType::MT_SPAWNSHOT);

    // Set cube's target to the spawn spot
    ctx.mobjs_mut()[cube_idx].target = Some(target_idx);

    // Calculate reactiontime = distance / speed / state_tics
    let _cube_info = &MOBJINFO[ctx.mobjs()[cube_idx].type_];
    let momy = ctx.mobjs()[cube_idx].momy;
    let state_idx = ctx.mobjs()[cube_idx].state;
    let state_tics = match state_idx {
        Some(si) => {
            let st = &STATES[si];
            if st.tics > 0 {
                st.tics
            } else {
                1
            }
        }
        None => 1,
    };

    if momy.0 != 0 {
        let dist = targ_y.0.wrapping_sub(ctx.mobjs()[actor_idx].y.0);
        let travel_time = dist / momy.0;
        let rt = travel_time / state_tics;
        ctx.mobjs_mut()[cube_idx].reactiontime = rt;
    }

    ctx.s_start_sound(None, SfxEnum::sfx_bospit);
}

/// A_SpawnSound — Spawn cube in-flight sound + attempt spawn fly.
///
/// Translated from `A_SpawnSound` (p_enemy.c:1964-1971).
pub fn a_spawn_sound(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    ctx.s_start_sound(Some(actor_idx), SfxEnum::sfx_boscub);
    a_spawn_fly(actor_idx, ctx);
}

/// A_SpawnFly — When cube reaches target, spawn a random monster.
///
/// Translated from `A_SpawnFly` (p_enemy.c:1977-2050).
pub fn a_spawn_fly(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    // Countdown
    let rt = ctx.mobjs()[actor_idx].reactiontime;
    ctx.mobjs_mut()[actor_idx].reactiontime -= 1;
    if rt > 0 {
        return; // still in flight
    }

    let targ_idx = match ctx.mobjs()[actor_idx].target {
        Some(t) => t,
        None => return,
    };

    // Spawn teleport fog at target
    let (tx, ty, tz) = {
        let targ = &ctx.mobjs()[targ_idx];
        (targ.x, targ.y, targ.z)
    };
    let fog_idx = ctx.p_spawn_mobj(tx, ty, tz, MobjType::MT_SPAWNFIRE);
    ctx.s_start_sound(Some(fog_idx), SfxEnum::sfx_telept);

    // Select random monster type based on probability distribution
    let r = ctx.rng_mut().p_random() as i32;
    let mtype = if r < 50 {
        MobjType::MT_TROOP
    } else if r < 90 {
        MobjType::MT_SERGEANT
    } else if r < 120 {
        MobjType::MT_SHADOWS
    } else if r < 130 {
        MobjType::MT_PAIN
    } else if r < 160 {
        MobjType::MT_HEAD
    } else if r < 162 {
        MobjType::MT_VILE
    } else if r < 172 {
        MobjType::MT_UNDEAD
    } else if r < 192 {
        MobjType::MT_BABY
    } else if r < 222 {
        MobjType::MT_FATSO
    } else if r < 246 {
        MobjType::MT_KNIGHT
    } else {
        MobjType::MT_BRUISER
    };

    let newmobj = ctx.p_spawn_mobj(tx, ty, tz, mtype);

    // Try to fit the new monster
    if !p_look_for_players(newmobj, true, ctx) {
        // Still try to place it
    }

    // Make it teleport in
    let see_state = MOBJINFO[ctx.mobjs()[newmobj].type_].seestate;
    ctx.p_set_mobj_state(newmobj, see_state);

    // Teleport move to spawn spot
    ctx.p_teleport_move(newmobj, tx, ty);

    // Remove the cube
    ctx.p_remove_mobj(actor_idx);
}

/// A_PlayerScream — Player death scream.
/// Uses sfx_pdiehi on commercial if health < -50.
///
/// Translated from `A_PlayerScream` (p_enemy.c:2005-2018).
pub fn a_player_scream(actor_idx: usize, ctx: &mut dyn EnemyContext) {
    // Determine if we should use the high-pitched scream
    let health = ctx.mobjs()[actor_idx].health;
    let is_commercial = ctx.gamemode() == GameMode::Commercial;

    let sfx = if is_commercial && health < -50 {
        // Extra-death scream
        SfxEnum::sfx_pdiehi
    } else {
        SfxEnum::sfx_pldeth
    };

    ctx.s_start_sound(Some(actor_idx), sfx);
}
