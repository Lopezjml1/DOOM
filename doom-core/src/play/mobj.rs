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

//! Moving object handling. Spawn functions.
//! Translated from linuxdoom-1.10/p_mobj.c
//!
//! This module manages the complete lifecycle of map objects (mobjs):
//! spawning, removal, state machine transitions, movement physics
//! (XY friction, Z gravity), missile spawning, item respawning,
//! player spawning, and nightmare respawn.

use tracing::{debug, info, warn};

use crate::info::mobjinfo::{MobjType, MOBJINFO, NUMMOBJTYPES};
use crate::info::sounds::SfxEnum;
use crate::info::states::{ActionFnId, State, StateNum, STATES};
use crate::play::maputl::p_aprox_distance;
use crate::types::angle::{Angle, ANG45, ANGLETOFINESHIFT, FINEMASK};
use crate::types::doomdef::{GameMode, Skill, MAXPLAYERS, MTF_AMBUSH};
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::map_data::{MapThing, Sector, Subsector};
use crate::types::mobj::{MapObject, MobjFlags, MF_TRANSSHIFT};
use crate::types::player::{CheatFlags, Player, PlayerState};
use crate::types::tables::{finecosine, FINESINE};
use crate::types::thinker::ActionFn;
use crate::util::random::DoomRandom;

// =============================================================================
// Constants (from p_mobj.c and p_local.h)
// =============================================================================

/// Stop speed threshold. Objects with momentum below this value are brought
/// to a complete stop. Original C: `#define STOPSPEED 0x1000` (p_mobj.c line 28).
const STOPSPEED: Fixed = Fixed(0x1000);

/// Friction multiplier applied each tic. Approximately 0.90625.
/// Original C: `#define FRICTION 0xe800` (p_mobj.c line 29).
const FRICTION: Fixed = Fixed(0xe800_u32 as i32);

/// Gravity acceleration per tic. Equals one `FRACUNIT`.
/// Original C: `#define GRAVITY FRACUNIT` (p_local.h line 53).
const GRAVITY: Fixed = Fixed(FRACUNIT);

/// Maximum movement distance per tic in fixed-point.
/// Original C: `#define MAXMOVE (30*FRACUNIT)` (p_local.h line 54).
const MAXMOVE: Fixed = Fixed(30 * FRACUNIT);

/// Speed at which floating monsters adjust their Z position toward targets.
/// Original C: `#define FLOATSPEED (FRACUNIT*4)` (p_local.h line 30).
const FLOATSPEED: Fixed = Fixed(4 * FRACUNIT);

/// Player view height from floor.
/// Original C: `#define VIEWHEIGHT (41*FRACUNIT)` (p_local.h line 34).
pub const VIEWHEIGHT: Fixed = Fixed(41 * FRACUNIT);

/// Sentinel value: spawn on floor.
/// Original C: `#define ONFLOORZ MININT` (p_local.h line 95).
pub const ONFLOORZ: Fixed = Fixed(i32::MIN);

/// Sentinel value: spawn on ceiling.
/// Original C: `#define ONCEILINGZ MAXINT` (p_local.h line 96).
pub const ONCEILINGZ: Fixed = Fixed(i32::MAX);

/// Melee attack range in fixed-point.
/// Original C: `#define MELEERANGE (64*FRACUNIT)` (p_local.h line 57).
pub const MELEERANGE: Fixed = Fixed(64 * FRACUNIT);

/// Missile attack range in fixed-point.
/// Original C: `#define MISSILERANGE (32*64*FRACUNIT)` (p_local.h line 58).
const MISSILERANGE: Fixed = Fixed(32 * 64 * FRACUNIT);

/// Size of the item respawn queue circular buffer.
/// Original C: `#define ITEMQUESIZE 128` (p_mobj.c line 540).
pub const ITEMQUESIZE: usize = 128;

// =============================================================================
// Game context trait — provides access to game state needed by mobj functions
// =============================================================================

/// Trait providing the global game context needed by mobj lifecycle functions.
///
/// This replaces the global variables and extern function calls from the
/// original C code. Implementors provide access to the mobj arena, level data,
/// player data, sound system, and other subsystems.
pub trait MobjContext {
    // --- Mobj arena access ---
    fn mobjs(&self) -> &[MapObject];
    fn mobjs_mut(&mut self) -> &mut Vec<MapObject>;
    fn alloc_mobj(&mut self) -> usize;
    fn free_mobj(&mut self, idx: usize);

    // --- Level geometry ---
    fn sectors(&self) -> &[Sector];
    fn sectors_mut(&mut self) -> &mut [Sector];
    fn subsectors(&self) -> &[Subsector];
    fn blocklinks(&self) -> &[Option<usize>];
    fn blocklinks_mut(&mut self) -> &mut [Option<usize>];
    fn bmap_orgx(&self) -> Fixed;
    fn bmap_orgy(&self) -> Fixed;
    fn bmap_width(&self) -> i32;
    fn bmap_height(&self) -> i32;
    fn point_in_subsector(&self, x: Fixed, y: Fixed) -> usize;

    // --- Player data ---
    fn players(&self) -> &[Player];
    fn players_mut(&mut self) -> &mut [Player];
    fn playeringame(&self) -> &[bool];
    fn consoleplayer(&self) -> usize;

    // --- Game state ---
    fn gameskill(&self) -> Skill;
    fn gamemode(&self) -> GameMode;
    fn netgame(&self) -> bool;
    fn deathmatch(&self) -> i32;
    fn nomonsters(&self) -> bool;
    fn respawnmonsters(&self) -> bool;
    fn respawnparm(&self) -> bool;
    fn leveltime(&self) -> i32;

    // --- Sound ---
    fn s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum);
    fn s_stop_sound(&mut self, origin: usize);

    // --- Player spawn points ---
    fn player_starts(&self) -> &[Option<MapThing>];
    fn player_starts_mut(&mut self) -> &mut [Option<MapThing>];
    fn deathmatch_starts(&self) -> &[MapThing];
    fn deathmatch_starts_mut(&mut self) -> &mut Vec<MapThing>;

    // --- Item respawn queue ---
    fn item_respawn_que(&self) -> &[MapThing; ITEMQUESIZE];
    fn item_respawn_que_mut(&mut self) -> &mut [MapThing; ITEMQUESIZE];
    fn item_respawn_time(&self) -> &[i32; ITEMQUESIZE];
    fn item_respawn_time_mut(&mut self) -> &mut [i32; ITEMQUESIZE];
    fn iquehead(&self) -> usize;
    fn set_iquehead(&mut self, val: usize);
    fn iquetail(&self) -> usize;
    fn set_iquetail(&mut self, val: usize);

    // --- Cross-module calls ---
    fn p_try_move(&mut self, thing_idx: usize, x: Fixed, y: Fixed) -> bool;
    fn p_slide_move(&mut self, thing_idx: usize);
    fn p_check_position(&mut self, thing_idx: usize, x: Fixed, y: Fixed) -> bool;
    fn p_aim_line_attack(
        &mut self,
        source_idx: usize,
        angle: Angle,
        range: Fixed,
    ) -> (Fixed, Option<usize>);
    fn p_line_attack(
        &mut self,
        source_idx: usize,
        angle: Angle,
        range: Fixed,
        slope: Fixed,
        damage: i32,
    );

    // --- Thinker management ---
    fn p_add_thinker(&mut self, mobj_idx: usize);
    fn p_remove_thinker(&mut self, mobj_idx: usize);

    // --- Total counts ---
    fn add_total_kills(&mut self);
    fn add_total_items(&mut self);

    // --- Sky hack ---
    /// Returns the sky flat number for the current level (used for sky ceiling missile hack).
    fn sky_flatnum(&self) -> i16;

    // --- Player rebirth ---
    fn g_player_reborn(&mut self, player_idx: usize);

    // --- Player spawn post-setup ---
    /// Initialize player weapon sprites for the given player.
    /// Original C: `P_SetupPsprites(player)` in p_pspr.c.
    fn p_setup_psprites(&mut self, player_idx: usize);

    /// Re-initialize the status bar for a newly spawned console player.
    /// Original C: `ST_Start()` in st_stuff.c.
    fn st_start(&mut self);

    /// Re-initialize the HUD for a newly spawned console player.
    /// Original C: `HU_Start()` in hu_stuff.c.
    fn hu_start(&mut self);

    // --- Ceiling check for missiles ---
    /// Returns the ceiling pic of the sector containing the given subsector.
    fn get_sector_ceilingpic(&self, subsector_idx: usize) -> i16;

    // --- Radius attack ---
    /// Perform a radius (splash) damage attack centred on `source_idx`, with
    /// damage attributed to `inflictor_idx` and the given maximum `damage`.
    /// Original C: `P_RadiusAttack(thing, source, damage)` (p_map.c).
    fn p_radius_attack(&mut self, source_idx: usize, inflictor_idx: Option<usize>, damage: i32);
}

// =============================================================================
// P_SetMobjState (p_mobj.c lines 53-84)
// =============================================================================

/// Set a map object to a new animation state.
///
/// Loops through zero-tic states, calling action functions as needed.
/// Returns `false` if the mobj entered `S_NULL` (was removed).
///
/// Original C: `boolean P_SetMobjState(mobj_t* mobj, statenum_t state)`
/// (p_mobj.c lines 53-84).
pub fn p_set_mobj_state(
    mobj_idx: usize,
    state_num: StateNum,
    ctx: &mut dyn MobjContext,
    rng: &mut DoomRandom,
) -> bool {
    let mut st = state_num;

    loop {
        if st == StateNum::S_NULL {
            // Object is removed
            {
                let mobjs = ctx.mobjs_mut();
                mobjs[mobj_idx].state = None;
            }
            p_remove_mobj(mobj_idx, ctx);
            return false;
        }

        let state: &State = &STATES[st as usize];
        {
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].state = Some(st as usize);
            mobjs[mobj_idx].tics = state.tics;
            mobjs[mobj_idx].sprite = state.sprite as usize;
            mobjs[mobj_idx].frame = state.frame;
        }

        // Call action function if present.
        // Action functions are dispatched by the caller based on ActionFnId.
        // For now we record the action but actual dispatch happens externally
        // since the full action function table is complex.
        if state.action != ActionFnId::None {
            // Action function dispatch is handled by the game loop.
            // The original C calls `st->action.acp1(mobj)` here.
            // In the Rust port, action functions are dispatched externally
            // based on the ActionFnId stored in the state.
            dispatch_state_action(mobj_idx, state.action, ctx, rng);
        }

        // Check if we need to continue to the next state (zero-tic states)
        let tics = ctx.mobjs()[mobj_idx].tics;
        if tics != 0 {
            break;
        }

        st = state.nextstate;
    }

    true
}

/// Dispatch a state action function for a map object.
///
/// Routes each [`ActionFnId`] to the corresponding A_* implementation.
/// Simple mobj-specific actions (A_Fall, A_Explode, A_Pain, A_Scream,
/// A_XScream, A_PlayerScream) are implemented inline.  Enemy AI actions
/// (A_Look, A_Chase, etc.) are delegated to `enemy.rs` once that module
/// is available — until then they emit a one-time trace-level log.
/// Weapon-sprite actions (A_WeaponReady, A_Lower, etc.) are handled by
/// the psprite dispatch in `pspr.rs` and should never reach here.
///
/// Original C: the `st->action.acp1(mobj)` call in `P_SetMobjState`
/// (p_mobj.c line 71).
fn dispatch_state_action(
    mobj_idx: usize,
    action: ActionFnId,
    ctx: &mut dyn MobjContext,
    rng: &mut DoomRandom,
) {
    match action {
        ActionFnId::None => {}

        // ==================================================================
        // Death / simple actions — implemented inline
        // ==================================================================

        // A_Fall: clear MF_SOLID so corpse can be walked over.
        // Original C: p_enemy.c lines 1599-1603.
        ActionFnId::A_Fall => {
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].flags.remove(MobjFlags::MF_SOLID);
        }

        // A_XScream: slop (gib) sound.
        // Original C: p_enemy.c line 1572.
        ActionFnId::A_XScream => {
            ctx.s_start_sound(Some(mobj_idx), SfxEnum::sfx_slop);
        }

        // A_Pain: play the object's pain sound.
        // Original C: p_enemy.c lines 1577-1579.
        ActionFnId::A_Pain => {
            let painsound = MOBJINFO[ctx.mobjs()[mobj_idx].type_].painsound;
            if painsound != SfxEnum::sfx_None {
                ctx.s_start_sound(Some(mobj_idx), painsound);
            }
        }

        // A_Scream: play the object's death sound with variation for
        // zombieman/imp/shotgunner and full volume for Spider/Cyber.
        // Original C: p_enemy.c lines 1535-1569.
        ActionFnId::A_Scream => {
            a_scream(mobj_idx, ctx, rng);
        }

        // A_PlayerScream: play sfx_pldeth (or sfx_pdiehi in DOOM II
        // when health < -50 gibbing threshold).
        // Original C: p_enemy.c lines 1994-2008.
        ActionFnId::A_PlayerScream => {
            let health = ctx.mobjs()[mobj_idx].health;
            let mode = ctx.gamemode();
            let sound = if mode == GameMode::Commercial && health < -50 {
                SfxEnum::sfx_pdiehi
            } else {
                SfxEnum::sfx_pldeth
            };
            ctx.s_start_sound(Some(mobj_idx), sound);
        }

        // A_Explode: radius attack (128 damage) centred on self, targeting
        // the object stored in `target` (the attacker for barrels, the firer
        // for rockets).  Original C: p_enemy.c lines 1610-1613.
        ActionFnId::A_Explode => {
            a_explode(mobj_idx, ctx);
        }

        // ==================================================================
        // Enemy AI actions — implemented in enemy.rs (future checkpoint).
        // Until enemy.rs is available, these are logged at trace level.
        // The dispatch mechanism is in place so that enemy.rs only needs to
        // provide the function bodies; no dispatch table changes are needed.
        // ==================================================================
        ActionFnId::A_Look
        | ActionFnId::A_Chase
        | ActionFnId::A_FaceTarget
        | ActionFnId::A_PosAttack
        | ActionFnId::A_SPosAttack
        | ActionFnId::A_CPosAttack
        | ActionFnId::A_CPosRefire
        | ActionFnId::A_TroopAttack
        | ActionFnId::A_SargAttack
        | ActionFnId::A_HeadAttack
        | ActionFnId::A_BruisAttack
        | ActionFnId::A_SkullAttack
        | ActionFnId::A_SpidRefire
        | ActionFnId::A_BspiAttack
        | ActionFnId::A_CyberAttack
        | ActionFnId::A_PainAttack
        | ActionFnId::A_PainDie
        | ActionFnId::A_KeenDie
        | ActionFnId::A_BossDeath
        | ActionFnId::A_VileChase
        | ActionFnId::A_VileStart
        | ActionFnId::A_VileTarget
        | ActionFnId::A_VileAttack
        | ActionFnId::A_StartFire
        | ActionFnId::A_Fire
        | ActionFnId::A_FireCrackle
        | ActionFnId::A_Tracer
        | ActionFnId::A_SkelWhoosh
        | ActionFnId::A_SkelFist
        | ActionFnId::A_SkelMissile
        | ActionFnId::A_FatRaise
        | ActionFnId::A_FatAttack1
        | ActionFnId::A_FatAttack2
        | ActionFnId::A_FatAttack3
        | ActionFnId::A_BabyMetal
        | ActionFnId::A_Hoof
        | ActionFnId::A_Metal
        | ActionFnId::A_BrainPain
        | ActionFnId::A_BrainScream
        | ActionFnId::A_BrainDie
        | ActionFnId::A_BrainAwake
        | ActionFnId::A_BrainSpit
        | ActionFnId::A_SpawnSound
        | ActionFnId::A_SpawnFly
        | ActionFnId::A_BrainExplode => {
            tracing::trace!(
                "Action {:?} for mobj {} deferred — enemy.rs not yet available",
                action,
                mobj_idx,
            );
        }

        // ==================================================================
        // Weapon-sprite actions — handled by pspr.rs dispatch.
        // These should never be routed through mobj state dispatch.
        // ==================================================================
        ActionFnId::A_Light0
        | ActionFnId::A_Light1
        | ActionFnId::A_Light2
        | ActionFnId::A_WeaponReady
        | ActionFnId::A_Lower
        | ActionFnId::A_Raise
        | ActionFnId::A_Punch
        | ActionFnId::A_ReFire
        | ActionFnId::A_FirePistol
        | ActionFnId::A_FireShotgun
        | ActionFnId::A_FireShotgun2
        | ActionFnId::A_CheckReload
        | ActionFnId::A_OpenShotgun2
        | ActionFnId::A_LoadShotgun2
        | ActionFnId::A_CloseShotgun2
        | ActionFnId::A_FireCGun
        | ActionFnId::A_GunFlash
        | ActionFnId::A_FireMissile
        | ActionFnId::A_Saw
        | ActionFnId::A_FirePlasma
        | ActionFnId::A_BFGsound
        | ActionFnId::A_FireBFG
        | ActionFnId::A_BFGSpray => {
            tracing::trace!(
                "Weapon action {:?} for mobj {} — normally handled by psprite dispatch",
                action,
                mobj_idx,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A_Scream — death sound with random variation for certain enemy types.
// Original C: p_enemy.c lines 1535-1569.
// ---------------------------------------------------------------------------
fn a_scream(mobj_idx: usize, ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    let (deathsound, mobj_type) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (MOBJINFO[mo.type_].deathsound, mo.type_)
    };

    if deathsound == SfxEnum::sfx_None {
        return;
    }

    let sound = match deathsound {
        SfxEnum::sfx_podth1 | SfxEnum::sfx_podth2 | SfxEnum::sfx_podth3 => {
            // Zombieman / shotgunguy random death sound variation.
            const SOUNDS: [SfxEnum; 3] = [
                SfxEnum::sfx_podth1,
                SfxEnum::sfx_podth2,
                SfxEnum::sfx_podth3,
            ];
            SOUNDS[(rng.p_random() % 3) as usize]
        }
        SfxEnum::sfx_bgdth1 | SfxEnum::sfx_bgdth2 => {
            // Imp random death sound variation.
            const SOUNDS: [SfxEnum; 2] = [SfxEnum::sfx_bgdth1, SfxEnum::sfx_bgdth2];
            SOUNDS[(rng.p_random() % 2) as usize]
        }
        other => other,
    };

    // Boss monsters play at full volume (no origin attenuation).
    let mtype_enum = MobjType::from_index(mobj_type);
    if mtype_enum == Some(MobjType::MT_SPIDER) || mtype_enum == Some(MobjType::MT_CYBORG) {
        ctx.s_start_sound(None, sound);
    } else {
        ctx.s_start_sound(Some(mobj_idx), sound);
    }
}

// ---------------------------------------------------------------------------
// A_Explode — 128-damage radius attack centred on self.
// Original C: p_enemy.c lines 1610-1613.
// ---------------------------------------------------------------------------
fn a_explode(mobj_idx: usize, ctx: &mut dyn MobjContext) {
    // P_RadiusAttack(thingy, thingy->target, 128)
    // The target field stores who is responsible for the damage (the firer
    // for rockets, the attacker for exploding barrels).
    let target_idx = ctx.mobjs()[mobj_idx].target;
    ctx.p_radius_attack(mobj_idx, target_idx, 128);
}

// =============================================================================
// P_ExplodeMissile (p_mobj.c lines 90-105)
// =============================================================================

/// Convert a missile into its explosion state.
///
/// Zeroes all momentum, transitions to the death state from mobjinfo,
/// randomizes the tic duration, and clears the MF_MISSILE flag.
///
/// Original C: `void P_ExplodeMissile(mobj_t* mo)` (p_mobj.c lines 90-105).
pub fn p_explode_missile(mobj_idx: usize, ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    // Zero momentum
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[mobj_idx].momx = Fixed::ZERO;
        mobjs[mobj_idx].momy = Fixed::ZERO;
        mobjs[mobj_idx].momz = Fixed::ZERO;
    }

    let deathstate = {
        let mobjs = ctx.mobjs();
        let info_idx = mobjs[mobj_idx].type_;
        MOBJINFO[info_idx].deathstate
    };

    p_set_mobj_state(mobj_idx, deathstate, ctx, rng);

    // Randomize tics, minimum 1
    {
        let mobjs = ctx.mobjs_mut();
        let rnd = rng.p_random() as i32;
        mobjs[mobj_idx].tics -= rnd & 3;
        if mobjs[mobj_idx].tics < 1 {
            mobjs[mobj_idx].tics = 1;
        }
        // Clear MF_MISSILE flag
        mobjs[mobj_idx].flags.remove(MobjFlags::MF_MISSILE);
    }
}

// =============================================================================
// P_XYMovement (p_mobj.c lines 111-241)
// =============================================================================

/// Process horizontal (XY) movement and friction for a map object.
///
/// Handles momentum splitting for large moves, collision via P_TryMove,
/// player slide movement, missile sky-ceiling hack, and friction/stopping.
///
/// Original C: `void P_XYMovement(mobj_t* mo)` (p_mobj.c lines 111-241).
pub fn p_xy_movement(mobj_idx: usize, ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    let (momx, momy, flags, _player_opt, type_) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (mo.momx, mo.momy, mo.flags, mo.player, mo.type_)
    };

    // If no momentum and skull flying, stop skull
    if momx == Fixed::ZERO && momy == Fixed::ZERO {
        if flags.contains(MobjFlags::MF_SKULLFLY) {
            // The skull slammed into something
            let spawnstate = MOBJINFO[type_].spawnstate;
            {
                let mobjs = ctx.mobjs_mut();
                mobjs[mobj_idx].flags.remove(MobjFlags::MF_SKULLFLY);
                mobjs[mobj_idx].momx = Fixed::ZERO;
                mobjs[mobj_idx].momy = Fixed::ZERO;
                mobjs[mobj_idx].momz = Fixed::ZERO;
            }
            p_set_mobj_state(mobj_idx, spawnstate, ctx, rng);
        }
        return;
    }

    // Clamp momentum to MAXMOVE
    let mut xmove = {
        let m = momx;
        if m.0 > MAXMOVE.0 {
            MAXMOVE
        } else if m.0 < -MAXMOVE.0 {
            Fixed(-MAXMOVE.0)
        } else {
            m
        }
    };

    let mut ymove = {
        let m = momy;
        if m.0 > MAXMOVE.0 {
            MAXMOVE
        } else if m.0 < -MAXMOVE.0 {
            Fixed(-MAXMOVE.0)
        } else {
            m
        }
    };

    // Split large moves into multiple steps (do-while loop from C)
    loop {
        let (ptryx, ptryy) = if xmove.0 > MAXMOVE.0 / 2
            || ymove.0 > MAXMOVE.0 / 2
            || xmove.0 < -(MAXMOVE.0 / 2)
            || ymove.0 < -(MAXMOVE.0 / 2)
        {
            // Need to split the move
            let px = {
                let mo = &ctx.mobjs()[mobj_idx];
                Fixed(mo.x.0.wrapping_add(xmove.0 / 2))
            };
            let py = {
                let mo = &ctx.mobjs()[mobj_idx];
                Fixed(mo.y.0.wrapping_add(ymove.0 / 2))
            };
            xmove = Fixed(xmove.0 / 2);
            ymove = Fixed(ymove.0 / 2);
            (px, py)
        } else {
            let px = {
                let mo = &ctx.mobjs()[mobj_idx];
                Fixed(mo.x.0.wrapping_add(xmove.0))
            };
            let py = {
                let mo = &ctx.mobjs()[mobj_idx];
                Fixed(mo.y.0.wrapping_add(ymove.0))
            };
            xmove = Fixed::ZERO;
            ymove = Fixed::ZERO;
            (px, py)
        };

        if !ctx.p_try_move(mobj_idx, ptryx, ptryy) {
            // Blocked
            let (is_player, is_missile, _flags2) = {
                let mo = &ctx.mobjs()[mobj_idx];
                (
                    mo.player.is_some(),
                    mo.flags.contains(MobjFlags::MF_MISSILE),
                    mo.flags,
                )
            };

            if is_player {
                ctx.p_slide_move(mobj_idx);
            } else if is_missile {
                // Sky ceiling hack: if the missile hit a ceiling that has
                // the sky flat, remove it silently (no explosion).
                let subsector_opt = ctx.mobjs()[mobj_idx].subsector;
                if let Some(ss_idx) = subsector_opt {
                    let ceilingpic = ctx.get_sector_ceilingpic(ss_idx);
                    if ceilingpic == ctx.sky_flatnum() {
                        p_remove_mobj(mobj_idx, ctx);
                        return;
                    }
                }
                p_explode_missile(mobj_idx, ctx, rng);
                return;
            } else {
                // Other blocked objects: zero momentum
                let mobjs = ctx.mobjs_mut();
                mobjs[mobj_idx].momx = Fixed::ZERO;
                mobjs[mobj_idx].momy = Fixed::ZERO;
            }
        }

        // Check if we need to continue splitting
        if xmove == Fixed::ZERO && ymove == Fixed::ZERO {
            break;
        }
    }

    // Friction and stopping
    let (flags, player_opt, z, floorz) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (mo.flags, mo.player, mo.z, mo.floorz)
    };

    // No friction for missiles and skull fly
    if flags.contains(MobjFlags::MF_MISSILE) || flags.contains(MobjFlags::MF_SKULLFLY) {
        return;
    }

    // No friction when airborne
    if z.0 > floorz.0 {
        return;
    }

    // Player-specific stopping and friction
    if let Some(player_idx) = player_opt {
        // Check for CF_NOMOMENTUM cheat
        let cheats = ctx.players()[player_idx].cheats;
        if cheats & CheatFlags::CF_NOMOMENTUM.bits() != 0 {
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].momx = Fixed::ZERO;
            mobjs[mobj_idx].momy = Fixed::ZERO;
            return;
        }
    }

    // Corpses on step edges: don't stop if has some momentum
    if flags.contains(MobjFlags::MF_CORPSE) {
        let (mx, my) = {
            let mo = &ctx.mobjs()[mobj_idx];
            (mo.momx, mo.momy)
        };
        // If the corpse has very low momentum, just stop it
        if mx.0 > -(1 << 12) && mx.0 < (1 << 12) && my.0 > -(1 << 12) && my.0 < (1 << 12) {
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].momx = Fixed::ZERO;
            mobjs[mobj_idx].momy = Fixed::ZERO;
            return;
        }
        // Otherwise let friction handle it (fall through)
    }

    // Check if slow enough to stop
    let (momx_val, momy_val) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (mo.momx, mo.momy)
    };

    if momx_val.0 > -STOPSPEED.0
        && momx_val.0 < STOPSPEED.0
        && momy_val.0 > -STOPSPEED.0
        && momy_val.0 < STOPSPEED.0
    {
        // Check if player is not pressing movement keys
        let should_stop = if let Some(player_idx) = player_opt {
            let cmd = &ctx.players()[player_idx].cmd;
            cmd.forwardmove == 0 && cmd.sidemove == 0
        } else {
            true
        };

        if should_stop {
            // If in the running frames, switch to standing
            if let Some(player_idx) = player_opt {
                let state_idx = ctx.mobjs()[mobj_idx].state;
                if let Some(st_idx) = state_idx {
                    let s_play_run1 = StateNum::S_PLAY_RUN1 as usize;
                    if st_idx >= s_play_run1 && st_idx < s_play_run1 + 4 {
                        p_set_mobj_state(mobj_idx, StateNum::S_PLAY, ctx, rng);
                    }
                }
                let _ = player_idx; // suppress unused warning
            }
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].momx = Fixed::ZERO;
            mobjs[mobj_idx].momy = Fixed::ZERO;
            return;
        }
    }

    // Apply friction
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[mobj_idx].momx = mobjs[mobj_idx].momx.fixed_mul(FRICTION);
        mobjs[mobj_idx].momy = mobjs[mobj_idx].momy.fixed_mul(FRICTION);
    }
}

// =============================================================================
// P_ZMovement (p_mobj.c lines 246-349)
// =============================================================================

/// Process vertical (Z) movement, gravity, floor/ceiling clipping, and
/// bounce behavior for a map object.
///
/// Original C: `void P_ZMovement(mobj_t* mo)` (p_mobj.c lines 246-349).
pub fn p_z_movement(mobj_idx: usize, ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    // Smooth step-up for players
    {
        let (is_player, z, floorz) = {
            let mo = &ctx.mobjs()[mobj_idx];
            (mo.player.is_some(), mo.z, mo.floorz)
        };
        if is_player && z.0 < floorz.0 {
            let player_idx = ctx.mobjs()[mobj_idx].player.unwrap();
            let diff = Fixed(floorz.0.wrapping_sub(z.0));
            let players = ctx.players_mut();
            players[player_idx].viewheight =
                Fixed(players[player_idx].viewheight.0.wrapping_sub(diff.0));
            players[player_idx].deltaviewheight =
                Fixed((VIEWHEIGHT.0.wrapping_sub(players[player_idx].viewheight.0)) >> 3);
        }
    }

    // Apply z momentum
    {
        let mobjs = ctx.mobjs_mut();
        let momz = mobjs[mobj_idx].momz;
        mobjs[mobj_idx].z = Fixed(mobjs[mobj_idx].z.0.wrapping_add(momz.0));
    }

    // Float toward target (floating monsters)
    {
        let (flags, target_opt, z) = {
            let mo = &ctx.mobjs()[mobj_idx];
            (mo.flags, mo.target, mo.z)
        };

        if flags.contains(MobjFlags::MF_FLOAT)
            && !flags.contains(MobjFlags::MF_SKULLFLY)
            && !flags.contains(MobjFlags::MF_INFLOAT)
        {
            if let Some(target_idx) = target_opt {
                let (target_z, target_height) = {
                    let mobjs = ctx.mobjs();
                    if target_idx < mobjs.len() {
                        (mobjs[target_idx].z, mobjs[target_idx].height)
                    } else {
                        (Fixed::ZERO, Fixed::ZERO)
                    }
                };
                // Target mid-height
                let target_mid = Fixed(target_z.0.wrapping_add(target_height.0 / 2));
                let dist = Fixed(target_mid.0.wrapping_sub(z.0));

                let mobjs = ctx.mobjs_mut();
                if dist.0 < -(FLOATSPEED.0) {
                    mobjs[mobj_idx].z = Fixed(mobjs[mobj_idx].z.0.wrapping_sub(FLOATSPEED.0));
                } else if dist.0 > FLOATSPEED.0 {
                    mobjs[mobj_idx].z = Fixed(mobjs[mobj_idx].z.0.wrapping_add(FLOATSPEED.0));
                }
            }
        }
    }

    // Floor clipping
    let (z, floorz, flags, momz) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (mo.z, mo.floorz, mo.flags, mo.momz)
    };

    if z.0 <= floorz.0 {
        // Check for skull fly bounce
        if flags.contains(MobjFlags::MF_SKULLFLY) {
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].momz = Fixed(-mobjs[mobj_idx].momz.0);
        }

        if momz.0 < 0 {
            // Player hard landing
            if let Some(player_idx) = ctx.mobjs()[mobj_idx].player {
                if momz.0 < Fixed(-8 * GRAVITY.0).0 {
                    // Hard landing: oof!
                    let players = ctx.players_mut();
                    players[player_idx].deltaviewheight = Fixed(momz.0 >> 3);
                    ctx.s_start_sound(Some(mobj_idx), SfxEnum::sfx_oof);
                }
            }
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].momz = Fixed::ZERO;
        }

        {
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].z = mobjs[mobj_idx].floorz;
        }

        // Missile check at floor
        let is_missile = ctx.mobjs()[mobj_idx].flags.contains(MobjFlags::MF_MISSILE);
        let is_noclip = ctx.mobjs()[mobj_idx].flags.contains(MobjFlags::MF_NOCLIP);
        if is_missile && !is_noclip {
            p_explode_missile(mobj_idx, ctx, rng);
            return;
        }
    } else {
        // Gravity
        if !flags.contains(MobjFlags::MF_NOGRAVITY) {
            let mobjs = ctx.mobjs_mut();
            if mobjs[mobj_idx].momz == Fixed::ZERO {
                mobjs[mobj_idx].momz = Fixed(-GRAVITY.0 * 2);
            } else {
                mobjs[mobj_idx].momz = Fixed(mobjs[mobj_idx].momz.0.wrapping_sub(GRAVITY.0));
            }
        }
    }

    // Ceiling clipping
    let (z, height, ceilingz) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (mo.z, mo.height, mo.ceilingz)
    };

    if z.0.wrapping_add(height.0) > ceilingz.0 {
        // Cap z momentum
        {
            let mobjs = ctx.mobjs_mut();
            if mobjs[mobj_idx].momz.0 > 0 {
                mobjs[mobj_idx].momz = Fixed::ZERO;
            }
            mobjs[mobj_idx].z = Fixed(ceilingz.0.wrapping_sub(height.0));
        }

        // Skull bounce off ceiling
        if ctx.mobjs()[mobj_idx].flags.contains(MobjFlags::MF_SKULLFLY) {
            let mobjs = ctx.mobjs_mut();
            mobjs[mobj_idx].momz = Fixed(-mobjs[mobj_idx].momz.0);
        }

        // Missile at ceiling
        let is_missile = ctx.mobjs()[mobj_idx].flags.contains(MobjFlags::MF_MISSILE);
        let is_noclip = ctx.mobjs()[mobj_idx].flags.contains(MobjFlags::MF_NOCLIP);
        if is_missile && !is_noclip {
            p_explode_missile(mobj_idx, ctx, rng);
        }
    }
}

// =============================================================================
// P_NightmareRespawn (p_mobj.c lines 356-409)
// =============================================================================

/// Respawn a killed monster in nightmare mode at its original spawn location.
///
/// Teleport fog is spawned at both the old and new positions. If the spawn
/// position is blocked, the respawn is deferred.
///
/// Original C: `void P_NightmareRespawn(mobj_t* mobj)` (p_mobj.c lines 356-409).
pub fn p_nightmare_respawn(mobj_idx: usize, ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    let (sp_x, sp_y, sp_angle, _sp_type, sp_options) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (
            mo.spawnpoint.x,
            mo.spawnpoint.y,
            mo.spawnpoint.angle,
            mo.spawnpoint.type_,
            mo.spawnpoint.options,
        )
    };

    // Convert spawn point to fixed-point
    let x = Fixed((sp_x as i32) << FRACBITS);
    let y = Fixed((sp_y as i32) << FRACBITS);

    // Check if spawn position is free
    if !ctx.p_check_position(mobj_idx, x, y) {
        return; // Blocked, try again later
    }

    // Spawn teleport fog at old position
    let (old_x, old_y) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (mo.x, mo.y)
    };

    let old_ss = ctx.point_in_subsector(old_x, old_y);
    let old_sec = ctx.subsectors()[old_ss].sector;
    let old_floorz = ctx.sectors()[old_sec].floorheight;

    let fog_old = p_spawn_mobj(old_x, old_y, old_floorz, MobjType::MT_TFOG, ctx, rng);
    ctx.s_start_sound(Some(fog_old), SfxEnum::sfx_telept);

    // Spawn teleport fog at new position
    let new_ss = ctx.point_in_subsector(x, y);
    let new_sec = ctx.subsectors()[new_ss].sector;
    let new_floorz = ctx.sectors()[new_sec].floorheight;

    let fog_new = p_spawn_mobj(x, y, new_floorz, MobjType::MT_TFOG, ctx, rng);
    ctx.s_start_sound(Some(fog_new), SfxEnum::sfx_telept);

    // Spawn the new monster
    let mo_type = {
        let mo = &ctx.mobjs()[mobj_idx];
        mo.type_
    };

    // Safe conversion — mo_type was set during the original spawn and is a
    // valid MobjType discriminant.
    let mobj_type_enum =
        MobjType::from_index(mo_type).expect("P_NightmareRespawn: invalid mobj type index");
    let new_mobj = p_spawn_mobj(x, y, new_floorz, mobj_type_enum, ctx, rng);

    // Set angle and ambush flag
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[new_mobj].angle = Angle(ANG45.0.wrapping_mul((sp_angle as u32) / 45));
        if sp_options as i32 & MTF_AMBUSH != 0 {
            mobjs[new_mobj].flags.insert(MobjFlags::MF_AMBUSH);
        }
    }

    // Set reactiontime
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[new_mobj].reactiontime = 18;
    }

    // Remove old mobj
    p_remove_mobj(mobj_idx, ctx);
}

// =============================================================================
// P_MobjThinker (p_mobj.c lines 415-473)
// =============================================================================

/// Per-tic processing for a map object.
///
/// Handles XY movement, Z movement, state machine tic countdown, and
/// nightmare mode monster respawning.
///
/// Original C: `void P_MobjThinker(mobj_t* mobj)` (p_mobj.c lines 415-473).
pub fn p_mobj_thinker(mobj_idx: usize, ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    // Check for pending removal sentinel
    if ctx.mobjs()[mobj_idx].thinker.function == ActionFn::PendingRemoval {
        return;
    }

    // XY movement
    let (has_xy_mom, is_skullfly) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (
            mo.momx != Fixed::ZERO || mo.momy != Fixed::ZERO,
            mo.flags.contains(MobjFlags::MF_SKULLFLY),
        )
    };

    if has_xy_mom || is_skullfly {
        p_xy_movement(mobj_idx, ctx, rng);

        // Check if mobj was removed during XY movement
        if ctx.mobjs()[mobj_idx].thinker.function == ActionFn::PendingRemoval {
            return;
        }
    }

    // Z movement
    let (z, floorz, has_z_mom) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (mo.z, mo.floorz, mo.momz != Fixed::ZERO)
    };

    if z != floorz || has_z_mom {
        p_z_movement(mobj_idx, ctx, rng);

        // Check if mobj was removed during Z movement
        if ctx.mobjs()[mobj_idx].thinker.function == ActionFn::PendingRemoval {
            return;
        }
    }

    // State tic countdown
    let tics = ctx.mobjs()[mobj_idx].tics;
    if tics != -1 {
        let new_tics = tics - 1;
        ctx.mobjs_mut()[mobj_idx].tics = new_tics;

        if new_tics <= 0 {
            // Advance to next state
            let next_state = {
                let state_opt = ctx.mobjs()[mobj_idx].state;
                match state_opt {
                    Some(st_idx) => STATES[st_idx].nextstate,
                    None => StateNum::S_NULL,
                }
            };
            p_set_mobj_state(mobj_idx, next_state, ctx, rng);
            // Don't do anything else after state change (mobj may be removed)
            return;
        }
    }

    // Nightmare respawn check
    let (is_countkill, respawn) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (
            mo.flags.contains(MobjFlags::MF_COUNTKILL),
            ctx.respawnmonsters(),
        )
    };

    if !is_countkill || !respawn {
        return;
    }

    // Increment movecount (used as respawn timer here)
    ctx.mobjs_mut()[mobj_idx].movecount += 1;

    let movecount = ctx.mobjs()[mobj_idx].movecount;

    // Wait at least 12 seconds (12*35 tics = 420)
    if movecount < 12 * 35 {
        return;
    }

    // Only check on certain tics to spread the load
    if ctx.leveltime() & 31 != 0 {
        return;
    }

    // Random chance
    if rng.p_random() > 4 {
        return;
    }

    p_nightmare_respawn(mobj_idx, ctx, rng);
}

// =============================================================================
// P_SpawnMobj (p_mobj.c lines 479-534)
// =============================================================================

/// Spawn a new map object at the given position.
///
/// Allocates a new mobj in the arena, initializes all fields from the
/// mobjinfo table, links it into the sector/blockmap data structures,
/// and adds it to the thinker list.
///
/// Returns the arena index of the newly spawned mobj.
///
/// Original C: `mobj_t* P_SpawnMobj(fixed_t x, fixed_t y, fixed_t z, mobjtype_t type)`
/// (p_mobj.c lines 479-534).
pub fn p_spawn_mobj(
    x: Fixed,
    y: Fixed,
    z: Fixed,
    mobj_type: MobjType,
    ctx: &mut dyn MobjContext,
    rng: &mut DoomRandom,
) -> usize {
    let info = &MOBJINFO[mobj_type as usize];

    let mobj_idx = ctx.alloc_mobj();

    // Initialize the new mobj from mobjinfo
    let not_nightmare = ctx.gameskill() != Skill::Nightmare;
    {
        let mobjs = ctx.mobjs_mut();
        let mo = &mut mobjs[mobj_idx];

        mo.type_ = mobj_type as usize;
        mo.info = Some(mobj_type as usize);
        mo.x = x;
        mo.y = y;
        mo.radius = Fixed(info.radius);
        mo.height = Fixed(info.height);
        mo.flags = MobjFlags::from_bits_truncate(info.flags);
        mo.health = info.spawnhealth;

        if not_nightmare {
            mo.reactiontime = info.reactiontime;
        }

        mo.lastlook = (rng.p_random() as i32) % (MAXPLAYERS as i32);

        // Set initial state directly (don't use P_SetMobjState since
        // actions can't run on an unlinked mobj)
        let st = &STATES[info.spawnstate as usize];
        mo.state = Some(info.spawnstate as usize);
        mo.tics = st.tics;
        mo.sprite = st.sprite as usize;
        mo.frame = st.frame;

        // Set thinker function
        mo.thinker.function = ActionFn::MobjThinker;
    }

    // Link into sector and blockmap
    {
        // We need to extract the values needed for p_set_thing_position
        let (mo_x, mo_y, mo_flags) = {
            let mo = &ctx.mobjs()[mobj_idx];
            (mo.x, mo.y, mo.flags)
        };

        let ss_idx = ctx.point_in_subsector(mo_x, mo_y);
        let sec_idx = ctx.subsectors()[ss_idx].sector;

        // We need to use the maputl function, but first link subsector
        ctx.mobjs_mut()[mobj_idx].subsector = Some(ss_idx);

        // Link into sector thinglist
        if !mo_flags.contains(MobjFlags::MF_NOSECTOR) {
            ctx.mobjs_mut()[mobj_idx].sprev = None;
            let old_head = ctx.sectors()[sec_idx].thinglist;
            ctx.mobjs_mut()[mobj_idx].snext = old_head;
            if let Some(old_head_idx) = old_head {
                ctx.mobjs_mut()[old_head_idx].sprev = Some(mobj_idx);
            }
            ctx.sectors_mut()[sec_idx].thinglist = Some(mobj_idx);
        }

        // Link into blockmap
        if !mo_flags.contains(MobjFlags::MF_NOBLOCKMAP) {
            let blockx = (mo_x.0 - ctx.bmap_orgx().0) >> (FRACBITS + 7);
            let blocky = (mo_y.0 - ctx.bmap_orgy().0) >> (FRACBITS + 7);
            let bw = ctx.bmap_width();
            let bh = ctx.bmap_height();

            if blockx >= 0 && blockx < bw && blocky >= 0 && blocky < bh {
                let idx = (blocky * bw + blockx) as usize;
                ctx.mobjs_mut()[mobj_idx].bprev = None;
                let old_head = ctx.blocklinks()[idx];
                ctx.mobjs_mut()[mobj_idx].bnext = old_head;
                if let Some(old_head_idx) = old_head {
                    ctx.mobjs_mut()[old_head_idx].bprev = Some(mobj_idx);
                }
                ctx.blocklinks_mut()[idx] = Some(mobj_idx);
            } else {
                ctx.mobjs_mut()[mobj_idx].bnext = None;
                ctx.mobjs_mut()[mobj_idx].bprev = None;
            }
        }

        // Set floor and ceiling from sector
        let floorheight = ctx.sectors()[sec_idx].floorheight;
        let ceilingheight = ctx.sectors()[sec_idx].ceilingheight;
        ctx.mobjs_mut()[mobj_idx].floorz = floorheight;
        ctx.mobjs_mut()[mobj_idx].ceilingz = ceilingheight;
    }

    // Handle Z position
    {
        let floorz = ctx.mobjs()[mobj_idx].floorz;
        let ceilingz = ctx.mobjs()[mobj_idx].ceilingz;
        let height = ctx.mobjs()[mobj_idx].height;

        let mobjs = ctx.mobjs_mut();
        if z == ONFLOORZ {
            mobjs[mobj_idx].z = floorz;
        } else if z == ONCEILINGZ {
            mobjs[mobj_idx].z = Fixed(ceilingz.0.wrapping_sub(height.0));
        } else {
            mobjs[mobj_idx].z = z;
        }
    }

    // Add to thinker list
    ctx.p_add_thinker(mobj_idx);

    debug!(
        "P_SpawnMobj: spawned type {:?} at ({}, {}, {}), idx={}",
        mobj_type,
        x.0 >> FRACBITS,
        y.0 >> FRACBITS,
        ctx.mobjs()[mobj_idx].z.0 >> FRACBITS,
        mobj_idx,
    );

    mobj_idx
}

// =============================================================================
// P_RemoveMobj (p_mobj.c lines 546-570)
// =============================================================================

/// Remove a map object from the game world.
///
/// If the object is a special item (pickup) that wasn't dropped by a monster,
/// and it's not an invulnerability or invisibility sphere, its position is
/// added to the item respawn queue for deathmatch mode.
///
/// Original C: `void P_RemoveMobj(mobj_t* mobj)` (p_mobj.c lines 546-570).
pub fn p_remove_mobj(mobj_idx: usize, ctx: &mut dyn MobjContext) {
    let (flags, type_, spawnpoint) = {
        let mo = &ctx.mobjs()[mobj_idx];
        (mo.flags, mo.type_, mo.spawnpoint)
    };

    // Add to item respawn queue if applicable
    if flags.contains(MobjFlags::MF_SPECIAL)
        && !flags.contains(MobjFlags::MF_DROPPED)
        && type_ != MobjType::MT_INV as usize
        && type_ != MobjType::MT_INS as usize
    {
        let iquehead = ctx.iquehead();
        ctx.item_respawn_que_mut()[iquehead] = spawnpoint;
        ctx.item_respawn_time_mut()[iquehead] = ctx.leveltime();
        ctx.set_iquehead((iquehead + 1) & (ITEMQUESIZE - 1));

        // Lose one item off the queue if full
        if ctx.iquehead() == ctx.iquetail() {
            ctx.set_iquetail((ctx.iquetail() + 1) & (ITEMQUESIZE - 1));
        }
    }

    // Unlink from sector and blockmap
    {
        let (mo_flags, mo_snext, mo_sprev, mo_bnext, mo_bprev, mo_subsector, mo_x, mo_y) = {
            let mo = &ctx.mobjs()[mobj_idx];
            (
                mo.flags,
                mo.snext,
                mo.sprev,
                mo.bnext,
                mo.bprev,
                mo.subsector,
                mo.x,
                mo.y,
            )
        };

        // Unlink from sector thinglist
        if !mo_flags.contains(MobjFlags::MF_NOSECTOR) {
            if let Some(snext_idx) = mo_snext {
                ctx.mobjs_mut()[snext_idx].sprev = mo_sprev;
            }
            if let Some(sprev_idx) = mo_sprev {
                ctx.mobjs_mut()[sprev_idx].snext = mo_snext;
            } else if let Some(ss_idx) = mo_subsector {
                let sec_idx = ctx.subsectors()[ss_idx].sector;
                ctx.sectors_mut()[sec_idx].thinglist = mo_snext;
            }
        }

        // Unlink from blockmap
        if !mo_flags.contains(MobjFlags::MF_NOBLOCKMAP) {
            if let Some(bnext_idx) = mo_bnext {
                ctx.mobjs_mut()[bnext_idx].bprev = mo_bprev;
            }
            if let Some(bprev_idx) = mo_bprev {
                ctx.mobjs_mut()[bprev_idx].bnext = mo_bnext;
            } else {
                let blockx = (mo_x.0 - ctx.bmap_orgx().0) >> (FRACBITS + 7);
                let blocky = (mo_y.0 - ctx.bmap_orgy().0) >> (FRACBITS + 7);
                let bw = ctx.bmap_width();
                let bh = ctx.bmap_height();
                if blockx >= 0 && blockx < bw && blocky >= 0 && blocky < bh {
                    let idx = (blocky * bw + blockx) as usize;
                    ctx.blocklinks_mut()[idx] = mo_bnext;
                }
            }
        }
    }

    // Stop any sounds playing from this mobj
    ctx.s_stop_sound(mobj_idx);

    // Remove from thinker list
    ctx.p_remove_thinker(mobj_idx);
}

// =============================================================================
// P_RespawnSpecials (p_mobj.c lines 578-631)
// =============================================================================

/// Check for and respawn special items (pickups) in deathmatch mode 2.
///
/// Items are respawned 30 seconds after being picked up. A teleport fog
/// effect (MT_IFOG) is spawned at the respawn location.
///
/// Original C: `void P_RespawnSpecials(void)` (p_mobj.c lines 578-631).
pub fn p_respawn_specials(ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    // Only respawn in deathmatch mode 2
    if ctx.deathmatch() != 2 {
        return;
    }

    // Nothing in the queue
    if ctx.iquehead() == ctx.iquetail() {
        return;
    }

    // Wait at least 30 seconds
    let iquetail = ctx.iquetail();
    let respawn_time = ctx.item_respawn_time()[iquetail];
    if ctx.leveltime() - respawn_time < 30 * 35 {
        return;
    }

    let mthing = ctx.item_respawn_que()[iquetail];

    let x = Fixed((mthing.x as i32) << FRACBITS);
    let y = Fixed((mthing.y as i32) << FRACBITS);

    // Spawn teleport fog at item position
    let ss_idx = ctx.point_in_subsector(x, y);
    let sec_idx = ctx.subsectors()[ss_idx].sector;
    let floorz = ctx.sectors()[sec_idx].floorheight;

    let fog = p_spawn_mobj(x, y, floorz, MobjType::MT_IFOG, ctx, rng);
    ctx.s_start_sound(Some(fog), SfxEnum::sfx_itmbk);

    // Find the type of item to respawn
    let mut i: usize = 0;
    let mut found_type: Option<MobjType> = None;
    while i < NUMMOBJTYPES {
        if MOBJINFO[i].doomednum == mthing.type_ as i32 {
            // Safe bounds-checked conversion from loop index to MobjType.
            found_type = MobjType::from_index(i);
            break;
        }
        i += 1;
    }

    let mobj_type = match found_type {
        Some(t) => t,
        None => {
            warn!(
                "P_RespawnSpecials: unknown doomednum {} in respawn queue",
                mthing.type_
            );
            // Advance the queue and return
            ctx.set_iquetail((iquetail + 1) & (ITEMQUESIZE - 1));
            return;
        }
    };

    // Spawn the item
    let z = if MOBJINFO[mobj_type as usize].flags & MobjFlags::MF_SPAWNCEILING.bits() != 0 {
        ONCEILINGZ
    } else {
        ONFLOORZ
    };

    let mo = p_spawn_mobj(x, y, z, mobj_type, ctx, rng);

    // Set spawn angle and other properties
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[mo].angle = Angle(ANG45.0.wrapping_mul((mthing.angle as u32) / 45));
    }

    // Advance the queue
    ctx.set_iquetail((iquetail + 1) & (ITEMQUESIZE - 1));
}

// =============================================================================
// P_SpawnPlayer (p_mobj.c lines 642-700)
// =============================================================================

/// Spawn a player at the given map thing position.
///
/// Creates the player mobj, initializes player state, sets up weapon sprites,
/// gives all cards in deathmatch, and resets status bar/HUD for the console
/// player.
///
/// Original C: `void P_SpawnPlayer(mapthing_t* mthing)` (p_mobj.c lines 642-700).
pub fn p_spawn_player(mthing: &MapThing, ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    let player_num = (mthing.type_ as usize) - 1;

    // Check bounds
    if player_num >= MAXPLAYERS {
        return;
    }

    // Check if player is in the game
    if !ctx.playeringame()[player_num] {
        return;
    }

    // Check for reborn
    if ctx.players()[player_num].playerstate == PlayerState::Reborn {
        ctx.g_player_reborn(player_num);
    }

    let x = Fixed((mthing.x as i32) << FRACBITS);
    let y = Fixed((mthing.y as i32) << FRACBITS);
    let z = ONFLOORZ;

    let mobj_idx = p_spawn_mobj(x, y, z, MobjType::MT_PLAYER, ctx, rng);

    // Set color translation for multiplayer
    if player_num > 0 {
        let mobjs = ctx.mobjs_mut();
        let translation_bits = (player_num as u32) << MF_TRANSSHIFT;
        let raw_flags = mobjs[mobj_idx].flags.bits() & !(MobjFlags::MF_TRANSLATION.bits());
        mobjs[mobj_idx].flags = MobjFlags::from_bits_truncate(raw_flags | translation_bits);
    }

    // Set angle from map thing
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[mobj_idx].angle = Angle(ANG45.0.wrapping_mul((mthing.angle as u32) / 45));
    }

    // Wire player ↔ mobj references
    let player_health = ctx.players()[player_num].health;
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[mobj_idx].player = Some(player_num);
        mobjs[mobj_idx].health = player_health;
    }

    {
        let players = ctx.players_mut();
        players[player_num].mobj = Some(mobj_idx);
        players[player_num].playerstate = PlayerState::Live;
        players[player_num].refire = 0;
        players[player_num].message = None;
        players[player_num].damagecount = 0;
        players[player_num].bonuscount = 0;
        players[player_num].extralight = 0;
        players[player_num].fixedcolormap = 0;
        players[player_num].viewheight = VIEWHEIGHT;
    }

    // Setup player weapon sprites
    ctx.p_setup_psprites(player_num);

    // If this is the console player, re-init statusbar and HUD
    if player_num == ctx.consoleplayer() {
        ctx.st_start();
        ctx.hu_start();
    }

    // Give all cards in deathmatch
    if ctx.deathmatch() > 0 {
        let players = ctx.players_mut();
        for i in 0..crate::types::doomdef::NUMCARDS {
            players[player_num].cards[i] = true;
        }
    }

    info!(
        "P_SpawnPlayer: player {} spawned at ({}, {})",
        player_num + 1,
        mthing.x,
        mthing.y,
    );
}

// =============================================================================
// P_SpawnMapThing (p_mobj.c lines 708-797)
// =============================================================================

/// Spawn a map thing from WAD data.
///
/// Handles deathmatch starts, player starts, skill-based filtering,
/// nomonsters check, and the actual mobj spawn with proper initialization.
///
/// Original C: `void P_SpawnMapThing(mapthing_t* mthing)` (p_mobj.c lines 708-797).
pub fn p_spawn_map_thing(mthing: &MapThing, ctx: &mut dyn MobjContext, rng: &mut DoomRandom) {
    // Check for deathmatch start (type 11)
    if mthing.type_ == 11 {
        if ctx.deathmatch_starts().len() < 10 {
            let mt_copy = *mthing;
            ctx.deathmatch_starts_mut().push(mt_copy);
        }
        return;
    }

    // Check for player starts (type 1-4)
    if mthing.type_ >= 1 && mthing.type_ <= 4 {
        let player_num = (mthing.type_ as usize) - 1;
        ctx.player_starts_mut()[player_num] = Some(*mthing);

        // In deathmatch, don't spawn player starts
        if ctx.deathmatch() > 0 {
            return;
        }

        p_spawn_player(mthing, ctx, rng);
        return;
    }

    // Check for things that shouldn't spawn in singleplayer
    if mthing.options & 16 != 0 && !ctx.netgame() {
        return;
    }

    // Skill filter — determine the skill bit to check.
    // Original C (p_mobj.c lines 714-720):
    //   if (gameskill == sk_baby) bit = 1;
    //   else if (gameskill == sk_nightmare) bit = 4;
    //   else bit = 1 << (gameskill-1);
    //   if (!(mthing->options & bit)) return;
    let skill = ctx.gameskill();
    let skill_bit: i32 = if skill == Skill::Baby {
        crate::types::doomdef::MTF_EASY // bit 1
    } else if skill == Skill::Nightmare {
        crate::types::doomdef::MTF_HARD // bit 4
    } else {
        1 << (skill as i32 - 1)
    };

    if mthing.options as i32 & skill_bit == 0 {
        return;
    }

    // Find the mobjinfo entry by doomednum
    let mut i: usize = 0;
    let mut found_type: Option<MobjType> = None;
    while i < NUMMOBJTYPES {
        if MOBJINFO[i].doomednum == mthing.type_ as i32 {
            // Safe bounds-checked conversion from loop index to MobjType.
            found_type = MobjType::from_index(i);
            break;
        }
        i += 1;
    }

    let mobj_type = match found_type {
        Some(t) => t,
        None => {
            warn!(
                "P_SpawnMapThing: unknown type {} at ({}, {})",
                mthing.type_, mthing.x, mthing.y,
            );
            return;
        }
    };

    // Don't spawn DM-only items in non-DM
    if ctx.deathmatch() == 0
        && MOBJINFO[mobj_type as usize].flags & MobjFlags::MF_NOTDMATCH.bits() != 0
    {
        return;
    }

    // Check for nomonsters
    if ctx.nomonsters()
        && (mobj_type == MobjType::MT_SKULL
            || MOBJINFO[mobj_type as usize].flags & MobjFlags::MF_COUNTKILL.bits() != 0)
    {
        return;
    }

    // Spawn the object
    let z = if MOBJINFO[mobj_type as usize].flags & MobjFlags::MF_SPAWNCEILING.bits() != 0 {
        ONCEILINGZ
    } else {
        ONFLOORZ
    };

    let x = Fixed((mthing.x as i32) << FRACBITS);
    let y = Fixed((mthing.y as i32) << FRACBITS);

    let mobj_idx = p_spawn_mobj(x, y, z, mobj_type, ctx, rng);

    // Randomize initial tics for animation variety
    {
        let mobjs = ctx.mobjs_mut();
        if mobjs[mobj_idx].tics > 0 {
            mobjs[mobj_idx].tics = 1 + (rng.p_random() as i32 % mobjs[mobj_idx].tics);
        }
    }

    // Count kills and items
    if ctx.mobjs()[mobj_idx]
        .flags
        .contains(MobjFlags::MF_COUNTKILL)
    {
        ctx.add_total_kills();
    }
    if ctx.mobjs()[mobj_idx]
        .flags
        .contains(MobjFlags::MF_COUNTITEM)
    {
        ctx.add_total_items();
    }

    // Set angle
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[mobj_idx].angle = Angle(ANG45.0.wrapping_mul((mthing.angle as u32) / 45));
    }

    // Set ambush flag
    if mthing.options as i32 & MTF_AMBUSH != 0 {
        let mobjs = ctx.mobjs_mut();
        mobjs[mobj_idx].flags.insert(MobjFlags::MF_AMBUSH);
    }
}

// =============================================================================
// P_SpawnPuff (p_mobj.c lines 811-831)
// =============================================================================

/// Spawn a puff of smoke/dust at a position (for bullet impacts).
///
/// The Z position is randomized. If the attack was at melee range, the
/// puff uses the S_PUFF3 state (shorter animation).
///
/// Original C: `void P_SpawnPuff(fixed_t x, fixed_t y, fixed_t z)` (p_mobj.c lines 811-831).
pub fn p_spawn_puff(
    x: Fixed,
    y: Fixed,
    z: Fixed,
    at_melee_range: bool,
    ctx: &mut dyn MobjContext,
    rng: &mut DoomRandom,
) -> usize {
    // Randomize Z
    let rnd1 = rng.p_random() as i32;
    let rnd2 = rng.p_random() as i32;
    let z_adj = Fixed(z.0.wrapping_add((rnd1 - rnd2) << 10));

    let th = p_spawn_mobj(x, y, z_adj, MobjType::MT_PUFF, ctx, rng);

    // Give upward momentum
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[th].momz = Fixed(FRACUNIT);
    }

    // Randomize tics
    {
        let mobjs = ctx.mobjs_mut();
        let rnd = rng.p_random() as i32;
        mobjs[th].tics -= rnd & 3;
        if mobjs[th].tics < 1 {
            mobjs[th].tics = 1;
        }
    }

    // At melee range, use shorter puff animation
    if at_melee_range {
        p_set_mobj_state(th, StateNum::S_PUFF3, ctx, rng);
    }

    th
}

// =============================================================================
// P_SpawnBlood (p_mobj.c lines 838-859)
// =============================================================================

/// Spawn blood spray particles at a position (for bullet/melee damage).
///
/// The Z position is randomized, and the animation state depends on damage.
///
/// Original C: `void P_SpawnBlood(fixed_t x, fixed_t y, fixed_t z, int damage)`
/// (p_mobj.c lines 838-859).
pub fn p_spawn_blood(
    x: Fixed,
    y: Fixed,
    z: Fixed,
    damage: i32,
    ctx: &mut dyn MobjContext,
    rng: &mut DoomRandom,
) -> usize {
    // Randomize Z
    let rnd1 = rng.p_random() as i32;
    let rnd2 = rng.p_random() as i32;
    let z_adj = Fixed(z.0.wrapping_add((rnd1 - rnd2) << 10));

    let th = p_spawn_mobj(x, y, z_adj, MobjType::MT_BLOOD, ctx, rng);

    // Give upward momentum
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[th].momz = Fixed(FRACUNIT * 2);
    }

    // Randomize tics
    {
        let mobjs = ctx.mobjs_mut();
        let rnd = rng.p_random() as i32;
        mobjs[th].tics -= rnd & 3;
        if mobjs[th].tics < 1 {
            mobjs[th].tics = 1;
        }
    }

    // Use shorter blood animation for low damage
    if (9..=12).contains(&damage) {
        p_set_mobj_state(th, StateNum::S_BLOOD2, ctx, rng);
    } else if damage < 9 {
        p_set_mobj_state(th, StateNum::S_BLOOD3, ctx, rng);
    }

    th
}

// =============================================================================
// P_CheckMissileSpawn (p_mobj.c lines 868-882)
// =============================================================================

/// Advance a just-spawned missile forward by half its momentum and verify
/// the position is valid. If the position is blocked, the missile explodes
/// immediately.
///
/// Returns `true` if the missile is still alive after the check.
///
/// Original C: `boolean P_CheckMissileSpawn(mobj_t* th)` (p_mobj.c lines 868-882).
pub fn p_check_missile_spawn(
    th_idx: usize,
    ctx: &mut dyn MobjContext,
    rng: &mut DoomRandom,
) -> bool {
    // Randomize tics
    {
        let mobjs = ctx.mobjs_mut();
        let rnd = rng.p_random() as i32;
        mobjs[th_idx].tics -= rnd & 3;
        if mobjs[th_idx].tics < 1 {
            mobjs[th_idx].tics = 1;
        }
    }

    // Move forward by half momentum
    {
        let (momx, momy, momz) = {
            let mo = &ctx.mobjs()[th_idx];
            (mo.momx, mo.momy, mo.momz)
        };
        let mobjs = ctx.mobjs_mut();
        mobjs[th_idx].x = Fixed(mobjs[th_idx].x.0.wrapping_add(momx.0 >> 1));
        mobjs[th_idx].y = Fixed(mobjs[th_idx].y.0.wrapping_add(momy.0 >> 1));
        mobjs[th_idx].z = Fixed(mobjs[th_idx].z.0.wrapping_add(momz.0 >> 1));
    }

    // Check position
    let (x, y) = {
        let mo = &ctx.mobjs()[th_idx];
        (mo.x, mo.y)
    };

    if !ctx.p_try_move(th_idx, x, y) {
        p_explode_missile(th_idx, ctx, rng);
        return false;
    }

    true
}

// =============================================================================
// P_SpawnMissile (p_mobj.c lines 888-927)
// =============================================================================

/// Spawn a missile fired from a source toward a destination object.
///
/// The missile is spawned at the source's position (with z offset), aimed
/// toward the destination. If the target has the MF_SHADOW flag, the aiming
/// angle is randomized (fuzzy aim).
///
/// Returns the arena index of the spawned missile.
///
/// Original C: `mobj_t* P_SpawnMissile(mobj_t* source, mobj_t* dest, mobjtype_t type)`
/// (p_mobj.c lines 888-927).
pub fn p_spawn_missile(
    source_idx: usize,
    dest_idx: usize,
    mobj_type: MobjType,
    ctx: &mut dyn MobjContext,
    rng: &mut DoomRandom,
) -> usize {
    let (source_x, source_y, source_z) = {
        let mo = &ctx.mobjs()[source_idx];
        (mo.x, mo.y, mo.z)
    };

    // Spawn at source position + 32 fixed units above
    let spawn_z = Fixed(source_z.0.wrapping_add(32 * FRACUNIT));

    let th = p_spawn_mobj(source_x, source_y, spawn_z, mobj_type, ctx, rng);

    // Play the missile's see sound
    let seesound = MOBJINFO[mobj_type as usize].seesound;
    if seesound != SfxEnum::sfx_None {
        ctx.s_start_sound(Some(th), seesound);
    }

    // Set missile's target (the firer)
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[th].target = Some(source_idx);
    }

    // Compute angle to destination
    let (dest_x, dest_y, dest_z, dest_flags) = {
        let mo = &ctx.mobjs()[dest_idx];
        (mo.x, mo.y, mo.z, mo.flags)
    };

    let mut an = crate::types::tables::point_to_angle(
        Fixed(dest_x.0.wrapping_sub(source_x.0)),
        Fixed(dest_y.0.wrapping_sub(source_y.0)),
    );

    // Fuzzy aim for shadow targets
    if dest_flags.contains(MobjFlags::MF_SHADOW) {
        let rnd1 = rng.p_random() as i32;
        let rnd2 = rng.p_random() as i32;
        let spread = ((rnd1 - rnd2) << 20) as u32;
        an = Angle(an.0.wrapping_add(spread));
    }

    // Set angle and momentum
    let speed = MOBJINFO[mobj_type as usize].speed;
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[th].angle = an;

        let fine = ((an.0 >> ANGLETOFINESHIFT) as usize) & (FINEMASK as usize);
        mobjs[th].momx = Fixed(speed).fixed_mul(finecosine(fine));
        mobjs[th].momy = Fixed(speed).fixed_mul(FINESINE[fine]);
    }

    // Compute Z velocity
    let dist = p_aprox_distance(
        Fixed(dest_x.0.wrapping_sub(source_x.0)),
        Fixed(dest_y.0.wrapping_sub(source_y.0)),
    );

    // dist is the horizontal distance; convert speed to fixed for division
    let mut dist_for_div = Fixed(dist.0 / speed);
    if dist_for_div.0 < 1 {
        dist_for_div = Fixed(1);
    }

    {
        let mobjs = ctx.mobjs_mut();
        mobjs[th].momz = Fixed((dest_z.0.wrapping_sub(source_z.0)) / dist_for_div.0);
    }

    p_check_missile_spawn(th, ctx, rng);

    th
}

// =============================================================================
// P_SpawnPlayerMissile (p_mobj.c lines 934-987)
// =============================================================================

/// Spawn a missile fired by a player, with auto-aim support.
///
/// Performs a 3-angle auto-aim sweep to find a target: straight ahead,
/// slightly left, and slightly right. If no target is found, the missile
/// is aimed straight ahead at the player's look angle.
///
/// Original C: `void P_SpawnPlayerMissile(mobj_t* source, mobjtype_t type)`
/// (p_mobj.c lines 934-987).
pub fn p_spawn_player_missile(
    source_idx: usize,
    mobj_type: MobjType,
    ctx: &mut dyn MobjContext,
    rng: &mut DoomRandom,
) {
    let (source_x, source_y, source_z, source_angle) = {
        let mo = &ctx.mobjs()[source_idx];
        (mo.x, mo.y, mo.z, mo.angle)
    };

    let mut an = source_angle;

    // First: aim straight ahead
    let (mut slope, mut linetarget) = ctx.p_aim_line_attack(source_idx, an, MISSILERANGE);

    if linetarget.is_none() {
        // Second: aim slightly to the right
        an = Angle(an.0.wrapping_add(1 << 26));
        let result = ctx.p_aim_line_attack(source_idx, an, MISSILERANGE);
        slope = result.0;
        linetarget = result.1;

        if linetarget.is_none() {
            // Third: aim slightly to the left
            an = Angle(source_angle.0.wrapping_sub(1 << 26));
            let result = ctx.p_aim_line_attack(source_idx, an, MISSILERANGE);
            slope = result.0;
            linetarget = result.1;

            if linetarget.is_none() {
                // No target found: aim straight ahead
                an = source_angle;
                slope = Fixed::ZERO;
            }
        }
    }

    // Spawn at source + 32 units above
    let spawn_z = Fixed(source_z.0.wrapping_add(32 * FRACUNIT));
    let th = p_spawn_mobj(source_x, source_y, spawn_z, mobj_type, ctx, rng);

    // Play see sound
    let seesound = MOBJINFO[mobj_type as usize].seesound;
    if seesound != SfxEnum::sfx_None {
        ctx.s_start_sound(Some(th), seesound);
    }

    // Set target (the firer)
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[th].target = Some(source_idx);
        mobjs[th].angle = an;
    }

    // Set momentum
    let speed = MOBJINFO[mobj_type as usize].speed;
    {
        let mobjs = ctx.mobjs_mut();
        let fine = ((an.0 >> ANGLETOFINESHIFT) as usize) & (FINEMASK as usize);
        mobjs[th].momx = Fixed(speed).fixed_mul(finecosine(fine));
        mobjs[th].momy = Fixed(speed).fixed_mul(FINESINE[fine]);
        mobjs[th].momz = Fixed(speed).fixed_mul(slope);
    }

    p_check_missile_spawn(th, ctx, rng);
}
