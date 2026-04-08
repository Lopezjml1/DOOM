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

//! Weapon sprite animation, weapon objects. Action functions for weapons.
//! Translated from linuxdoom-1.10/p_pspr.c and p_pspr.h
//!
//! This module manages player weapon sprites (psprites): state transitions,
//! raising/lowering animations, and all weapon-specific attack action
//! functions (punch, saw, pistol, shotgun, chaingun, missile, plasma, BFG).
//!
//! The weapon state machine operates by setting a psprite to a state from the
//! global STATES table. Each state specifies a duration (tics), an optional
//! action function, and a next-state. Zero-tic states are chained immediately
//! (within the same tic) until a state with non-zero tics is reached.
//!
//! External game operations (sound, collision, spawning) are accessed through
//! the [`PsprContext`] trait, allowing this module to remain decoupled from
//! the concrete game world implementation.

use crate::info::mobjinfo::MobjType;
use crate::info::sounds::SfxEnum;
use crate::info::states::{ActionFnId, StateNum, STATES};
use crate::types::angle::{Angle, ANG180, ANG90, FINEANGLES, FINEMASK};
use crate::types::doomdef::{AmmoType, GameMode, PowerType, WeaponType, WEAPONINFO};
use crate::types::event::BT_ATTACK;
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::mobj::{MapObject, MobjFlags};
use crate::types::player::{Player, PlayerState, NUMPSPRITES};
use crate::types::tables::{finecosine, point_to_angle2, FINESINE};
use crate::util::random::DoomRandom;

// ==========================================================================
// Constants (p_pspr.c lines 46-56, p_pspr.h, p_local.h)
// ==========================================================================

/// Speed at which the weapon sprite lowers off-screen (6 fixed-point units/tic).
/// Original C: `#define LOWERSPEED (FRACUNIT*6)` (p_pspr.c line 46)
const LOWERSPEED: Fixed = Fixed(6 << FRACBITS);

/// Speed at which the weapon sprite raises on-screen (6 fixed-point units/tic).
/// Original C: `#define RAISESPEED (FRACUNIT*6)` (p_pspr.c line 47)
const RAISESPEED: Fixed = Fixed(6 << FRACBITS);

/// Y-coordinate where the weapon sprite is fully off-screen (bottom).
/// Original C: `#define WEAPONBOTTOM (128*FRACUNIT)` (p_pspr.c line 49)
const WEAPONBOTTOM: Fixed = Fixed(128 << FRACBITS);

/// Y-coordinate where the weapon sprite is fully visible (top/ready position).
/// Original C: `#define WEAPONTOP (32*FRACUNIT)` (p_pspr.c line 50)
const WEAPONTOP: Fixed = Fixed(32 << FRACBITS);

/// Number of plasma cells consumed per BFG shot.
/// Original C: `#define BFGCELLS 40` (p_pspr.c line 54)
const BFGCELLS: i32 = 40;

/// Melee attack range (64 map units in fixed-point).
/// Original C: `#define MELEERANGE (64*FRACUNIT)` (p_local.h)
const MELEERANGE: Fixed = Fixed(64 << FRACBITS);

/// Maximum hitscan/auto-aim range (2048 map units in fixed-point).
/// Original C: `#define MISSILERANGE (32*64*FRACUNIT)` (p_local.h)
const MISSILERANGE: Fixed = Fixed(32 * 64 * (1 << FRACBITS));

/// Psprite slot index for the main weapon sprite.
/// Original C: `ps_weapon = 0` (p_pspr.h)
pub const PS_WEAPON: usize = 0;

/// Psprite slot index for the muzzle flash overlay sprite.
/// Original C: `ps_flash = 1` (p_pspr.h)
pub const PS_FLASH: usize = 1;

// ==========================================================================
// PsprContext — trait for external game world operations
// ==========================================================================

/// Context trait providing access to game world operations needed by weapon
/// action functions.
///
/// This trait abstracts the cross-module dependencies that the original C code
/// accessed via direct function calls and global variables. Implementors
/// provide access to the mobj arena, sound system, collision detection, and
/// projectile spawning.
pub trait PsprContext {
    /// Set a map object to a new animation state.
    /// Returns `false` if the mobj was removed (entered S_NULL).
    /// Original C: `P_SetMobjState` (p_mobj.c)
    fn p_set_mobj_state(&mut self, mobj_idx: usize, state: StateNum) -> bool;

    /// Alert nearby monsters of noise from the given source.
    /// Original C: `P_NoiseAlert` (p_enemy.c)
    fn p_noise_alert(&mut self, target_idx: usize, emitter_idx: usize);

    /// Auto-aim line attack trace. Searches for a target along the given
    /// angle within the specified range. Returns `(slope, linetarget_index)`.
    /// The slope is the vertical aiming angle in fixed-point.
    /// `linetarget_index` is `Some(idx)` if an aimable target was found.
    /// Original C: `P_AimLineAttack` (p_map.c), sets global `linetarget`
    fn p_aim_line_attack(
        &mut self,
        source_idx: usize,
        angle: Angle,
        range: Fixed,
    ) -> (Fixed, Option<usize>);

    /// Fire a hitscan line attack dealing the specified damage.
    /// Original C: `P_LineAttack` (p_map.c)
    fn p_line_attack(
        &mut self,
        source_idx: usize,
        angle: Angle,
        range: Fixed,
        slope: Fixed,
        damage: i32,
    );

    /// Spawn a player-fired missile of the given type.
    /// Original C: `P_SpawnPlayerMissile` (p_mobj.c)
    fn p_spawn_player_missile(&mut self, source_idx: usize, mobj_type: MobjType);

    /// Spawn a new map object at the given position.
    /// Returns the arena index of the newly spawned mobj.
    /// Original C: `P_SpawnMobj` (p_mobj.c)
    fn p_spawn_mobj(&mut self, x: Fixed, y: Fixed, z: Fixed, mobj_type: MobjType) -> usize;

    /// Apply damage to a target mobj.
    /// `inflictor` is the thing that caused the damage (projectile, etc.).
    /// `source` is the thing responsible (the shooter).
    /// Original C: `P_DamageMobj` (p_inter.c)
    fn p_damage_mobj(
        &mut self,
        target_idx: usize,
        inflictor: Option<usize>,
        source: Option<usize>,
        damage: i32,
    );

    /// Play a sound effect at the given origin.
    /// `origin` is `None` for ambient/UI sounds, `Some(idx)` for positional.
    /// Original C: `S_StartSound` (s_sound.c)
    fn s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum);

    /// Get an immutable reference to a map object by arena index.
    fn get_mobj(&self, idx: usize) -> &MapObject;

    /// Get a mutable reference to a map object by arena index.
    fn get_mobj_mut(&mut self, idx: usize) -> &mut MapObject;

    /// Get the current game mode (Shareware, Commercial, etc.).
    fn game_mode(&self) -> GameMode;

    /// Get the current level time in tics (for weapon bob calculation).
    /// Original C: `leveltime` global (g_game.c)
    fn level_time(&self) -> i32;
}

// ==========================================================================
// Action dispatch (internal helper)
// ==========================================================================

/// Dispatch a weapon psprite action function identified by [`ActionFnId`].
///
/// This is the Rust equivalent of the C `state->action.acp2(player, psp)`
/// function pointer call. Only weapon-related actions are handled here;
/// non-weapon actions (monster AI, etc.) are logged and skipped since they
/// should never appear in psprite state entries.
fn dispatch_psprite_action(
    action: ActionFnId,
    player: &mut Player,
    psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    match action {
        ActionFnId::None => {}
        ActionFnId::A_WeaponReady => a_weapon_ready(player, psp_idx, ctx, rng),
        ActionFnId::A_Lower => a_lower(player, psp_idx, ctx, rng),
        ActionFnId::A_Raise => a_raise(player, psp_idx, ctx, rng),
        ActionFnId::A_Punch => a_punch(player, psp_idx, ctx, rng),
        ActionFnId::A_ReFire => a_refire(player, psp_idx, ctx, rng),
        ActionFnId::A_FirePistol => a_fire_pistol(player, psp_idx, ctx, rng),
        ActionFnId::A_Light1 => a_light1(player, psp_idx, ctx, rng),
        ActionFnId::A_FireShotgun => a_fire_shotgun(player, psp_idx, ctx, rng),
        ActionFnId::A_Light2 => a_light2(player, psp_idx, ctx, rng),
        ActionFnId::A_FireShotgun2 => a_fire_shotgun2(player, psp_idx, ctx, rng),
        ActionFnId::A_CheckReload => a_check_reload(player, psp_idx, ctx, rng),
        ActionFnId::A_OpenShotgun2 => open_shotgun2(player, ctx),
        ActionFnId::A_LoadShotgun2 => load_shotgun2(player, ctx),
        ActionFnId::A_CloseShotgun2 => close_shotgun2(player, psp_idx, ctx, rng),
        ActionFnId::A_FireCGun => a_fire_cgun(player, psp_idx, ctx, rng),
        ActionFnId::A_GunFlash => a_gun_flash(player, psp_idx, ctx, rng),
        ActionFnId::A_FireMissile => a_fire_missile(player, psp_idx, ctx, rng),
        ActionFnId::A_Saw => a_saw(player, psp_idx, ctx, rng),
        ActionFnId::A_FirePlasma => a_fire_plasma(player, psp_idx, ctx, rng),
        ActionFnId::A_BFGsound => a_bfg_sound(player, psp_idx, ctx, rng),
        ActionFnId::A_FireBFG => a_fire_bfg(player, psp_idx, ctx, rng),
        ActionFnId::A_Light0 => a_light0(player, psp_idx, ctx, rng),
        // A_BFGSpray is a mobj action (called on the BFG projectile), not a
        // psprite action. It should never appear in weapon state entries.
        ActionFnId::A_BFGSpray => {
            tracing::warn!("A_BFGSpray dispatched in psprite context — ignoring");
        }
        // All other actions are monster/mobj actions, not weapon actions.
        _ => {
            tracing::debug!(
                "Non-weapon action {:?} dispatched in psprite context — ignoring",
                action
            );
        }
    }
}

// ==========================================================================
// P_SetPsprite — core state machine driver (p_pspr.c lines 58-102)
// ==========================================================================

/// Set a player's psprite to the specified state, executing the state machine.
///
/// Loops through consecutive zero-tic states, calling each state's action
/// function. The loop terminates when a state with non-zero tics is reached,
/// or when the psprite state is set to `None` (S_NULL).
///
/// # Arguments
/// * `player` — The player whose psprite is being set
/// * `position` — Psprite slot index (PS_WEAPON=0 or PS_FLASH=1)
/// * `stnum` — Initial state to set
/// * `ctx` — Game world context for action function callbacks
/// * `rng` — Random number generator for action functions
///
/// Original C: `P_SetPsprite` (p_pspr.c lines 58-102)
pub fn p_set_psprite(
    player: &mut Player,
    position: usize,
    mut stnum: StateNum,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    loop {
        if stnum == StateNum::S_NULL {
            // Object removed itself.
            player.psprites[position].state = None;
            break;
        }

        let state = &STATES[stnum as usize];
        player.psprites[position].state = Some(stnum as usize);
        player.psprites[position].tics = state.tics;

        // Coordinate set from state's misc fields (used by DeHackEd patches).
        if state.misc1 != 0 {
            player.psprites[position].sx = Fixed(state.misc1 << FRACBITS);
            player.psprites[position].sy = Fixed(state.misc2 << FRACBITS);
        }

        // Call the state's action function.
        if state.action != ActionFnId::None {
            dispatch_psprite_action(state.action, player, position, ctx, rng);
            // Action may have cleared the state (set it to None).
            if player.psprites[position].state.is_none() {
                break;
            }
        }

        // Read nextstate from the current psp state, which may have been
        // changed by the action function (via recursive P_SetPsprite calls).
        let current_state_idx = match player.psprites[position].state {
            Some(idx) => idx,
            None => break,
        };
        stnum = STATES[current_state_idx].nextstate;

        // Continue looping only while tics == 0 (zero-tic state chaining).
        if player.psprites[position].tics != 0 {
            break;
        }
    }
}

// ==========================================================================
// P_CalcSwing — unused weapon swing calculation (p_pspr.c lines 108-128)
// ==========================================================================

/// Calculate weapon swing offsets based on player bob and level time.
///
/// **NOTE**: This function is defined in the original C source but is never
/// called by any code path in vanilla DOOM 1.10. It is preserved here for
/// behavioral parity and potential use by mods/DeHackEd patches.
///
/// Returns `(swingx, swingy)` in fixed-point.
///
/// Original C: `P_CalcSwing` (p_pspr.c lines 108-128)
#[allow(dead_code)]
pub fn p_calc_swing(player: &Player, level_time: i32) -> (Fixed, Fixed) {
    let swing = player.bob;

    // swingx = FixedMul(swing, finesine[(FINEANGLES/70*leveltime)&FINEMASK])
    let swing_angle = (((FINEANGLES / 70) as usize) * (level_time as usize)) & (FINEMASK as usize);
    let swingx = swing.fixed_mul(FINESINE[swing_angle]);

    // swingy = -FixedMul(swingx, finesine[(FINEANGLES/70*leveltime + FINEANGLES/2)&FINEMASK])
    let swing_angle2 = (((FINEANGLES / 70) as usize) * (level_time as usize)
        + (FINEANGLES as usize) / 2)
        & (FINEMASK as usize);
    let swingy = Fixed(0i32.wrapping_sub(swingx.fixed_mul(FINESINE[swing_angle2]).0));

    (swingx, swingy)
}

// ==========================================================================
// P_BringUpWeapon — start weapon raise animation (p_pspr.c lines 138-154)
// ==========================================================================

/// Start raising the pending weapon to the ready position.
///
/// Sets the psprite to the weapon's upstate and positions it at the
/// bottom of the screen. If the weapon is a chainsaw, plays the
/// characteristic raise sound.
///
/// Original C: `P_BringUpWeapon` (p_pspr.c lines 138-154)
pub fn p_bring_up_weapon(player: &mut Player, ctx: &mut dyn PsprContext, rng: &mut DoomRandom) {
    // If no pending weapon change, use the current ready weapon.
    if player.pendingweapon == WeaponType::NoChange {
        player.pendingweapon = player.readyweapon;
    }

    // Play chainsaw raise sound.
    if player.pendingweapon == WeaponType::Chainsaw {
        if let Some(mo_idx) = player.mobj {
            ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_sawup);
        }
    }

    let newstate = WEAPONINFO[player.pendingweapon as usize].upstate;
    player.pendingweapon = WeaponType::NoChange;
    player.psprites[PS_WEAPON].sy = WEAPONBOTTOM;

    tracing::trace!(
        "P_BringUpWeapon: raising weapon {:?} from bottom",
        player.readyweapon
    );

    p_set_psprite(player, PS_WEAPON, newstate, ctx, rng);
}

// ==========================================================================
// P_CheckAmmo — verify ammo and auto-switch if depleted (p_pspr.c lines 161-240)
// ==========================================================================

/// Check if the player has enough ammo for the current weapon.
///
/// If insufficient ammo is available, automatically selects the best
/// alternative weapon (in a fixed priority order matching the original
/// engine behavior) and begins the weapon-down animation.
///
/// Returns `true` if the current weapon has sufficient ammo, `false` if
/// a weapon switch was initiated.
///
/// Original C: `P_CheckAmmo` (p_pspr.c lines 161-240)
pub fn p_check_ammo(player: &mut Player, ctx: &mut dyn PsprContext, rng: &mut DoomRandom) -> bool {
    let ammo = WEAPONINFO[player.readyweapon as usize].ammo;

    // Determine minimum ammo count for one shot.
    let count = if player.readyweapon == WeaponType::Bfg {
        BFGCELLS
    } else if player.readyweapon == WeaponType::SuperShotgun {
        2 // Double barrel requires 2 shells.
    } else {
        1 // All other weapons use 1 unit per shot.
    };

    // Weapons that don't require ammo (fist, chainsaw) always have enough.
    // Also return true if current ammo is sufficient.
    if ammo == AmmoType::NoAmmo || player.ammo[ammo as usize] >= count {
        return true;
    }

    tracing::debug!(
        "P_CheckAmmo: weapon {:?} out of ammo, selecting alternative",
        player.readyweapon
    );

    // Out of ammo — pick a weapon to change to.
    // The do-while loop in C always terminates because fist is the fallback.
    let game_mode = ctx.game_mode();
    loop {
        // Priority 1: Plasma rifle (not available in shareware).
        if player.weaponowned[WeaponType::Plasma as usize]
            && player.ammo[AmmoType::Cell as usize] > 0
            && game_mode != GameMode::Shareware
        {
            player.pendingweapon = WeaponType::Plasma;
        }
        // Priority 2: Super shotgun (commercial only, needs >2 shells).
        else if player.weaponowned[WeaponType::SuperShotgun as usize]
            && player.ammo[AmmoType::Shell as usize] > 2
            && game_mode == GameMode::Commercial
        {
            player.pendingweapon = WeaponType::SuperShotgun;
        }
        // Priority 3: Chaingun.
        else if player.weaponowned[WeaponType::Chaingun as usize]
            && player.ammo[AmmoType::Clip as usize] > 0
        {
            player.pendingweapon = WeaponType::Chaingun;
        }
        // Priority 4: Shotgun.
        else if player.weaponowned[WeaponType::Shotgun as usize]
            && player.ammo[AmmoType::Shell as usize] > 0
        {
            player.pendingweapon = WeaponType::Shotgun;
        }
        // Priority 5: Pistol (just needs clip ammo; always owned).
        else if player.ammo[AmmoType::Clip as usize] > 0 {
            player.pendingweapon = WeaponType::Pistol;
        }
        // Priority 6: Chainsaw.
        else if player.weaponowned[WeaponType::Chainsaw as usize] {
            player.pendingweapon = WeaponType::Chainsaw;
        }
        // Priority 7: Rocket launcher.
        else if player.weaponowned[WeaponType::Missile as usize]
            && player.ammo[AmmoType::Missile as usize] > 0
        {
            player.pendingweapon = WeaponType::Missile;
        }
        // Priority 8: BFG (not shareware, needs >40 cells).
        else if player.weaponowned[WeaponType::Bfg as usize]
            && player.ammo[AmmoType::Cell as usize] > 40
            && game_mode != GameMode::Shareware
        {
            player.pendingweapon = WeaponType::Bfg;
        }
        // Priority 9: Fist (always available, no ammo needed).
        else {
            player.pendingweapon = WeaponType::Fist;
        }

        // C: do {} while (pendingweapon == wp_nochange) — always exits first pass.
        if player.pendingweapon != WeaponType::NoChange {
            break;
        }
    }

    // Set weapon overlay to the down-state of the current weapon.
    let downstate = WEAPONINFO[player.readyweapon as usize].downstate;
    p_set_psprite(player, PS_WEAPON, downstate, ctx, rng);

    false
}

// ==========================================================================
// P_FireWeapon — initiate weapon fire (p_pspr.c lines 246-257)
// ==========================================================================

/// Initiate firing the player's current weapon.
///
/// Original C: `P_FireWeapon` (p_pspr.c lines 246-257)
pub fn p_fire_weapon(player: &mut Player, ctx: &mut dyn PsprContext, rng: &mut DoomRandom) {
    if !p_check_ammo(player, ctx, rng) {
        return;
    }

    // Set player body to attack state.
    if let Some(mo_idx) = player.mobj {
        ctx.p_set_mobj_state(mo_idx, StateNum::S_PLAY_ATK1);
    }

    let atkstate = WEAPONINFO[player.readyweapon as usize].atkstate;
    p_set_psprite(player, PS_WEAPON, atkstate, ctx, rng);

    // Alert nearby monsters.
    if let Some(mo_idx) = player.mobj {
        ctx.p_noise_alert(mo_idx, mo_idx);
    }
}

// ==========================================================================
// P_DropWeapon — begin lowering current weapon (p_pspr.c lines 265-270)
// ==========================================================================

/// Begin lowering the current weapon off-screen.
///
/// Original C: `P_DropWeapon` (p_pspr.c lines 265-270)
pub fn p_drop_weapon(player: &mut Player, ctx: &mut dyn PsprContext, rng: &mut DoomRandom) {
    let downstate = WEAPONINFO[player.readyweapon as usize].downstate;
    p_set_psprite(player, PS_WEAPON, downstate, ctx, rng);
}

// ==========================================================================
// A_WeaponReady — weapon idle/ready state (p_pspr.c lines 281-334)
// ==========================================================================

/// Weapon idle/ready state action function.
///
/// Original C: `A_WeaponReady` (p_pspr.c lines 281-334)
pub fn a_weapon_ready(
    player: &mut Player,
    psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    // Get out of attack state if player body is in ATK1 or ATK2.
    if let Some(mo_idx) = player.mobj {
        let mo_state = ctx.get_mobj(mo_idx).state;
        if mo_state == Some(StateNum::S_PLAY_ATK1 as usize)
            || mo_state == Some(StateNum::S_PLAY_ATK2 as usize)
        {
            ctx.p_set_mobj_state(mo_idx, StateNum::S_PLAY);
        }
    }

    // Play chainsaw idle sound when in the S_SAW state.
    if player.readyweapon == WeaponType::Chainsaw
        && player.psprites[psp_idx].state == Some(StateNum::S_SAW as usize)
    {
        if let Some(mo_idx) = player.mobj {
            ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_sawidl);
        }
    }

    // Check for weapon change or player death.
    if player.pendingweapon != WeaponType::NoChange || player.health <= 0 {
        let newstate = WEAPONINFO[player.readyweapon as usize].downstate;
        p_set_psprite(player, PS_WEAPON, newstate, ctx, rng);
        return;
    }

    // Check for fire button press.
    // Missile launcher and BFG do not auto-fire.
    if (player.cmd.buttons & BT_ATTACK) != 0 {
        if player.attackdown == 0
            || (player.readyweapon != WeaponType::Missile && player.readyweapon != WeaponType::Bfg)
        {
            player.attackdown = 1;
            p_fire_weapon(player, ctx, rng);
            return;
        }
    } else {
        player.attackdown = 0;
    }

    // Bob the weapon based on player movement speed.
    let level_time = ctx.level_time();
    let angle = (128usize.wrapping_mul(level_time as usize)) & (FINEMASK as usize);
    player.psprites[psp_idx].sx =
        Fixed((1i32 << FRACBITS).wrapping_add(player.bob.fixed_mul(finecosine(angle)).0));
    let angle2 = angle & ((FINEANGLES as usize) / 2 - 1);
    player.psprites[psp_idx].sy = Fixed(
        WEAPONTOP
            .0
            .wrapping_add(player.bob.fixed_mul(FINESINE[angle2]).0),
    );
}

// ==========================================================================
// A_ReFire — continuous fire check (p_pspr.c lines 343-362)
// ==========================================================================

/// Check if the player should continue firing (refire).
///
/// Original C: `A_ReFire` (p_pspr.c lines 343-362)
pub fn a_refire(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    if (player.cmd.buttons & BT_ATTACK) != 0
        && player.pendingweapon == WeaponType::NoChange
        && player.health > 0
    {
        player.refire += 1;
        p_fire_weapon(player, ctx, rng);
    } else {
        player.refire = 0;
        p_check_ammo(player, ctx, rng);
    }
}

// ==========================================================================
// A_CheckReload — ammo check action (p_pspr.c lines 364-375)
// ==========================================================================

/// Simple ammo check action function.
///
/// Original C: `A_CheckReload` (p_pspr.c lines 364-375)
pub fn a_check_reload(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    p_check_ammo(player, ctx, rng);
}

// ==========================================================================
// A_Lower — weapon lowering animation (p_pspr.c lines 384-416)
// ==========================================================================

/// Lower the weapon sprite off-screen each tic.
///
/// Moves the weapon sprite down by LOWERSPEED per tic. When it reaches
/// WEAPONBOTTOM, transitions to the pending weapon or handles death state.
///
/// Original C: `A_Lower` (p_pspr.c lines 384-416)
pub fn a_lower(
    player: &mut Player,
    psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    player.psprites[psp_idx].sy = Fixed(player.psprites[psp_idx].sy.0.wrapping_add(LOWERSPEED.0));

    // Is already down.
    if player.psprites[psp_idx].sy < WEAPONBOTTOM {
        return;
    }

    // Player is dead.
    if player.playerstate == PlayerState::Dead {
        player.psprites[psp_idx].sy = WEAPONBOTTOM;
        return;
    }

    // The old weapon has been lowered off the screen, now change weapon
    // and start raising the new one.
    if player.health <= 0 {
        // Player is dead, so keep the weapon off-screen.
        p_set_psprite(player, PS_WEAPON, StateNum::S_NULL, ctx, rng);
        return;
    }

    player.readyweapon = player.pendingweapon;
    p_bring_up_weapon(player, ctx, rng);
}

// ==========================================================================
// A_Raise — weapon raising animation (p_pspr.c lines 422-441)
// ==========================================================================

/// Raise the weapon sprite onto the screen each tic.
///
/// Moves the weapon sprite up by RAISESPEED per tic. When it reaches
/// WEAPONTOP, transitions to the weapon's ready state.
///
/// Original C: `A_Raise` (p_pspr.c lines 422-441)
pub fn a_raise(
    player: &mut Player,
    psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    player.psprites[psp_idx].sy = Fixed(player.psprites[psp_idx].sy.0.wrapping_sub(RAISESPEED.0));

    if player.psprites[psp_idx].sy > WEAPONTOP {
        return;
    }

    player.psprites[psp_idx].sy = WEAPONTOP;

    // The weapon has been raised all the way, so change to the ready state.
    let readystate = WEAPONINFO[player.readyweapon as usize].readystate;

    tracing::trace!(
        "A_Raise: weapon {:?} reached ready position",
        player.readyweapon
    );

    p_set_psprite(player, PS_WEAPON, readystate, ctx, rng);
}

// ==========================================================================
// A_GunFlash — set muzzle flash state (p_pspr.c lines 447-464)
// ==========================================================================

/// Set the muzzle flash psprite and player body to attack state 2.
///
/// Original C: `A_GunFlash` (p_pspr.c lines 447-464)
pub fn a_gun_flash(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    if let Some(mo_idx) = player.mobj {
        ctx.p_set_mobj_state(mo_idx, StateNum::S_PLAY_ATK2);
    }

    let flashstate = WEAPONINFO[player.readyweapon as usize].flashstate;
    p_set_psprite(player, PS_FLASH, flashstate, ctx, rng);
}

// ==========================================================================
// A_Punch — fist attack action (p_pspr.c lines 468-495)
// ==========================================================================

/// Fist (and berserk fist) melee attack action.
///
/// Damage: `(P_Random() % 10 + 1) << 1`, multiplied by 10 if the player
/// has berserk power active. Uses MELEERANGE with random angle spread.
/// Plays punch sound only if a target is hit.
///
/// Original C: `A_Punch` (p_pspr.c lines 468-495)
pub fn a_punch(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    // Calculate base damage: (P_Random() % 10 + 1) << 1
    let mut damage = ((rng.p_random() as i32 % 10) + 1) << 1;

    // Berserk power multiplies damage by 10.
    if player.powers[PowerType::Strength as usize] != 0 {
        damage *= 10;
    }

    let Some(mo_idx) = player.mobj else {
        return;
    };

    // Calculate random angle spread: angle += (P_Random() - P_Random()) << 18
    let mo_angle = ctx.get_mobj(mo_idx).angle;
    let spread = ((rng.p_random() as i32) - (rng.p_random() as i32)) << 18;
    let angle = Angle(mo_angle.0.wrapping_add(spread as u32));

    // Auto-aim at melee range.
    let (slope, linetarget) = ctx.p_aim_line_attack(mo_idx, angle, MELEERANGE);

    // Fire the line attack.
    ctx.p_line_attack(mo_idx, angle, MELEERANGE, slope, damage);

    // Only play sound if we hit something.
    if let Some(_target_idx) = linetarget {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_punch);

        // Turn to face the target.
        let mo = ctx.get_mobj(mo_idx);
        let mo_x = mo.x;
        let mo_y = mo.y;
        let target = ctx.get_mobj(_target_idx);
        let target_x = target.x;
        let target_y = target.y;
        let face_angle = point_to_angle2(mo_x, mo_y, target_x, target_y);
        ctx.get_mobj_mut(mo_idx).angle = face_angle;
    }
}

// ==========================================================================
// A_Saw — chainsaw attack action (p_pspr.c lines 501-543)
// ==========================================================================

/// Chainsaw melee attack action.
///
/// Damage: `2 * (P_Random() % 10 + 1)`. Range: MELEERANGE + 1.
/// Plays sfx_sawful on miss, sfx_sawhit on hit. On hit, turns the player
/// to face the target with a limited turn rate (ANG90/20 per tic) and
/// sets MF_JUSTATTACKED on the player mobj.
///
/// Original C: `A_Saw` (p_pspr.c lines 501-543)
pub fn a_saw(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    let damage = 2 * ((rng.p_random() as i32 % 10) + 1);

    let Some(mo_idx) = player.mobj else {
        return;
    };

    // Random angle spread.
    let mo_angle = ctx.get_mobj(mo_idx).angle;
    let spread = ((rng.p_random() as i32) - (rng.p_random() as i32)) << 18;
    let angle = Angle(mo_angle.0.wrapping_add(spread as u32));

    // Use MELEERANGE + 1 for chainsaw (slightly extra reach).
    let saw_range = Fixed(MELEERANGE.0.wrapping_add(1));

    let (slope, linetarget) = ctx.p_aim_line_attack(mo_idx, angle, saw_range);
    ctx.p_line_attack(mo_idx, angle, saw_range, slope, damage);

    if linetarget.is_none() {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_sawful);
        return;
    }

    ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_sawhit);

    // Turn to face the target.
    // Compute angle from player to target.
    let target_idx = linetarget.unwrap();
    let mo = ctx.get_mobj(mo_idx);
    let mo_x = mo.x;
    let mo_y = mo.y;
    let target = ctx.get_mobj(target_idx);
    let target_x = target.x;
    let target_y = target.y;
    let target_angle = point_to_angle2(mo_x, mo_y, target_x, target_y);

    // Limit turn rate to ANG90/20 per tic (to prevent instant snap).
    let current_angle = ctx.get_mobj(mo_idx).angle;
    let diff = Angle(target_angle.0.wrapping_sub(current_angle.0));

    if diff.0 > ANG180.0 {
        // Target is to the left (diff wrapped around).
        if diff.0.wrapping_neg() > (ANG90.0 / 20) {
            ctx.get_mobj_mut(mo_idx).angle = Angle(target_angle.0.wrapping_add(ANG90.0 / 21));
        }
    } else if diff.0 > ANG90.0 / 20 {
        // Target is to the right.
        ctx.get_mobj_mut(mo_idx).angle = Angle(target_angle.0.wrapping_sub(ANG90.0 / 21));
    } else {
        // Within turn limit — snap to target angle.
        ctx.get_mobj_mut(mo_idx).angle = target_angle;
    }

    // Set MF_JUSTATTACKED flag.
    let flags = ctx.get_mobj(mo_idx).flags;
    ctx.get_mobj_mut(mo_idx).flags = flags | MobjFlags::MF_JUSTATTACKED;
}

// ==========================================================================
// A_FireMissile — rocket launcher fire (p_pspr.c lines 549-556)
// ==========================================================================

/// Fire a rocket (MT_ROCKET).
///
/// Decrements missile ammo by 1 and spawns a rocket projectile.
///
/// Original C: `A_FireMissile` (p_pspr.c lines 549-556)
pub fn a_fire_missile(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    _rng: &mut DoomRandom,
) {
    player.ammo[AmmoType::Missile as usize] -= 1;

    if let Some(mo_idx) = player.mobj {
        ctx.p_spawn_player_missile(mo_idx, MobjType::MT_ROCKET);
    }
}

// ==========================================================================
// A_FireBFG — BFG 9000 fire (p_pspr.c lines 562-575)
// ==========================================================================

/// Fire the BFG 9000 (MT_BFG).
///
/// Decrements cell ammo by BFGCELLS (40) and spawns a BFG projectile.
///
/// Original C: `A_FireBFG` (p_pspr.c lines 562-575)
pub fn a_fire_bfg(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    _rng: &mut DoomRandom,
) {
    player.ammo[AmmoType::Cell as usize] -= BFGCELLS;

    if let Some(mo_idx) = player.mobj {
        ctx.p_spawn_player_missile(mo_idx, MobjType::MT_BFG);
    }
}

// ==========================================================================
// A_FirePlasma — plasma rifle fire (p_pspr.c lines 581-597)
// ==========================================================================

/// Fire a plasma bolt (MT_PLASMA).
///
/// Decrements cell ammo by 1, sets a random flash state offset (±1 from
/// the weapon's flash state), and spawns a plasma projectile.
///
/// Original C: `A_FirePlasma` (p_pspr.c lines 581-597)
pub fn a_fire_plasma(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    player.ammo[AmmoType::Cell as usize] -= 1;

    // Random flash state: flashstate + (P_Random() & 1)
    let flash_base = WEAPONINFO[player.readyweapon as usize].flashstate as usize;
    let flash_offset = (rng.p_random() & 1) as usize;
    let flash_idx = flash_base + flash_offset;
    if let Some(flash_stnum) = StateNum::from_index(flash_idx) {
        p_set_psprite(player, PS_FLASH, flash_stnum, ctx, rng);
    }

    if let Some(mo_idx) = player.mobj {
        ctx.p_spawn_player_missile(mo_idx, MobjType::MT_PLASMA);
    }
}

// ==========================================================================
// P_BulletSlope — auto-aim bullet slope (p_pspr.c lines 601-619)
// ==========================================================================

/// Calculate the vertical aiming slope for hitscan weapons.
///
/// Performs a 3-angle auto-aim sweep: first at the player's exact angle,
/// then offset by ±(1<<26) BAM (~5.6°). Returns the slope for the first
/// successful aim hit, or the slope from the player's angle if no target
/// is found (for vertical free-aim).
///
/// In the original C, this sets the file-scoped `bulletslope` variable.
/// In this Rust port, the slope is returned directly.
///
/// Original C: `P_BulletSlope` (p_pspr.c lines 601-619)
fn p_bullet_slope(player: &Player, ctx: &mut dyn PsprContext) -> Fixed {
    let Some(mo_idx) = player.mobj else {
        return Fixed::ZERO;
    };

    let an = ctx.get_mobj(mo_idx).angle;

    // First try: exact angle at auto-aim range 16*64*FRACUNIT.
    let autoaim_range = Fixed(16 * 64 * FRACUNIT);
    let (slope, target) = ctx.p_aim_line_attack(mo_idx, an, autoaim_range);

    if target.is_some() {
        return slope;
    }

    // Second try: offset angle to the right (+1<<26 BAM).
    let an2 = Angle(an.0.wrapping_add(1 << 26));
    let (slope2, target2) = ctx.p_aim_line_attack(mo_idx, an2, autoaim_range);

    if target2.is_some() {
        return slope2;
    }

    // Third try: offset angle to the left (-1<<26 BAM = +((1<<26)-1) wrapping).
    // C: an -= 2<<26 (relative to the +1<<26 state, so net -1<<26 from original).
    let an3 = Angle(an.0.wrapping_sub(1 << 26));
    let (slope3, _target3) = ctx.p_aim_line_attack(mo_idx, an3, autoaim_range);

    slope3
}

// ==========================================================================
// P_GunShot — single bullet hitscan (p_pspr.c lines 625-640)
// ==========================================================================

/// Fire a single hitscan bullet.
///
/// Damage: `5 * (P_Random() % 3 + 1)` (5, 10, or 15).
/// If `accurate` is false, adds random angle spread ±(P_Random()-P_Random())<<18.
///
/// Original C: `P_GunShot` (p_pspr.c lines 625-640)
fn p_gun_shot(
    player: &Player,
    bulletslope: Fixed,
    accurate: bool,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    let damage = 5 * ((rng.p_random() as i32 % 3) + 1);

    let Some(mo_idx) = player.mobj else {
        return;
    };

    let mut angle = ctx.get_mobj(mo_idx).angle;

    if !accurate {
        let spread = ((rng.p_random() as i32) - (rng.p_random() as i32)) << 18;
        angle = Angle(angle.0.wrapping_add(spread as u32));
    }

    ctx.p_line_attack(mo_idx, angle, MISSILERANGE, bulletslope, damage);
}

// ==========================================================================
// A_FirePistol — pistol fire action (p_pspr.c lines 645-658)
// ==========================================================================

/// Fire the pistol.
///
/// Plays pistol sound, sets attack state 2, decrements clip ammo,
/// activates muzzle flash, calculates bullet slope, and fires one shot.
/// First shot (refire==0) is accurate; subsequent shots have random spread.
///
/// Original C: `A_FirePistol` (p_pspr.c lines 645-658)
pub fn a_fire_pistol(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    if let Some(mo_idx) = player.mobj {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_pistol);
    }

    if let Some(mo_idx) = player.mobj {
        ctx.p_set_mobj_state(mo_idx, StateNum::S_PLAY_ATK2);
    }

    let ammo_type = WEAPONINFO[player.readyweapon as usize].ammo;
    if ammo_type != AmmoType::NoAmmo {
        player.ammo[ammo_type as usize] -= 1;
    }

    let flashstate = WEAPONINFO[player.readyweapon as usize].flashstate;
    p_set_psprite(player, PS_FLASH, flashstate, ctx, rng);

    let bulletslope = p_bullet_slope(player, ctx);
    let accurate = player.refire == 0;
    p_gun_shot(player, bulletslope, accurate, ctx, rng);
}

// ==========================================================================
// A_FireShotgun — shotgun fire action (p_pspr.c lines 664-688)
// ==========================================================================

/// Fire the shotgun (7 pellets).
///
/// Plays shotgun sound, sets attack state 2, decrements shell ammo by 1,
/// activates muzzle flash, calculates bullet slope, and fires 7 hitscan
/// pellets all with random spread (never accurate).
///
/// Original C: `A_FireShotgun` (p_pspr.c lines 664-688)
pub fn a_fire_shotgun(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    if let Some(mo_idx) = player.mobj {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_shotgn);
    }

    if let Some(mo_idx) = player.mobj {
        ctx.p_set_mobj_state(mo_idx, StateNum::S_PLAY_ATK2);
    }

    let ammo_type = WEAPONINFO[player.readyweapon as usize].ammo;
    if ammo_type != AmmoType::NoAmmo {
        player.ammo[ammo_type as usize] -= 1;
    }

    let flashstate = WEAPONINFO[player.readyweapon as usize].flashstate;
    p_set_psprite(player, PS_FLASH, flashstate, ctx, rng);

    let bulletslope = p_bullet_slope(player, ctx);

    // Fire 7 pellets, all with random spread.
    for _ in 0..7 {
        p_gun_shot(player, bulletslope, false, ctx, rng);
    }
}

// ==========================================================================
// A_FireShotgun2 — super shotgun fire action (p_pspr.c lines 694-732)
// ==========================================================================

/// Fire the super shotgun (20 pellets with wide spread).
///
/// Plays super shotgun sound, sets attack state 2, decrements shell ammo
/// by 2, activates muzzle flash, calculates bullet slope, and fires 20
/// hitscan pellets each with both horizontal (±19-bit) and vertical
/// (±5-bit) random spread.
///
/// Original C: `A_FireShotgun2` (p_pspr.c lines 694-732)
pub fn a_fire_shotgun2(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    if let Some(mo_idx) = player.mobj {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_dshtgn);
    }

    if let Some(mo_idx) = player.mobj {
        ctx.p_set_mobj_state(mo_idx, StateNum::S_PLAY_ATK2);
    }

    let ammo_type = WEAPONINFO[player.readyweapon as usize].ammo;
    if ammo_type != AmmoType::NoAmmo {
        player.ammo[ammo_type as usize] -= 2;
    }

    let flashstate = WEAPONINFO[player.readyweapon as usize].flashstate;
    p_set_psprite(player, PS_FLASH, flashstate, ctx, rng);

    let bulletslope = p_bullet_slope(player, ctx);

    let Some(mo_idx) = player.mobj else {
        return;
    };

    // Fire 20 pellets with wide spread.
    for _ in 0..20 {
        let damage = 5 * ((rng.p_random() as i32 % 3) + 1);

        let mo_angle = ctx.get_mobj(mo_idx).angle;

        // Horizontal spread: ±19 bits (wider than standard ±18).
        let h_spread = ((rng.p_random() as i32) - (rng.p_random() as i32)) << 19;
        let angle = Angle(mo_angle.0.wrapping_add(h_spread as u32));

        // Vertical spread: ±5 bits added to bulletslope.
        let v_spread = ((rng.p_random() as i32) - (rng.p_random() as i32)) << 5;
        let slope = Fixed(bulletslope.0.wrapping_add(v_spread));

        ctx.p_line_attack(mo_idx, angle, MISSILERANGE, slope, damage);
    }
}

// ==========================================================================
// Super shotgun reload sound helpers (p_pspr.c lines ~700-735)
// ==========================================================================

/// Play the super shotgun open sound (sfx_dbopn).
/// Original C: `A_OpenShotgun2` (p_pspr.c)
fn open_shotgun2(player: &Player, ctx: &mut dyn PsprContext) {
    if let Some(mo_idx) = player.mobj {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_dbopn);
    }
}

/// Play the super shotgun load sound (sfx_dbload).
/// Original C: `A_LoadShotgun2` (p_pspr.c)
fn load_shotgun2(player: &Player, ctx: &mut dyn PsprContext) {
    if let Some(mo_idx) = player.mobj {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_dbload);
    }
}

/// Play the super shotgun close sound (sfx_dbcls) and check for refire.
/// Original C: `A_CloseShotgun2` (p_pspr.c)
fn close_shotgun2(
    player: &mut Player,
    psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    if let Some(mo_idx) = player.mobj {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_dbcls);
    }
    a_refire(player, psp_idx, ctx, rng);
}

// ==========================================================================
// A_FireCGun — chaingun fire action (p_pspr.c lines 738-760)
// ==========================================================================

/// Fire the chaingun (single bullet per animation frame).
///
/// Plays pistol sound, checks ammo, sets attack state 2, decrements clip
/// ammo by 1, calculates flash state offset based on current state relative
/// to S_CHAIN1, and fires one bullet. First shot (refire==0) is accurate.
///
/// Original C: `A_FireCGun` (p_pspr.c lines 738-760)
pub fn a_fire_cgun(
    player: &mut Player,
    psp_idx: usize,
    ctx: &mut dyn PsprContext,
    rng: &mut DoomRandom,
) {
    if let Some(mo_idx) = player.mobj {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_pistol);
    }

    let ammo_type = WEAPONINFO[player.readyweapon as usize].ammo;
    if ammo_type != AmmoType::NoAmmo {
        if player.ammo[ammo_type as usize] <= 0 {
            return;
        }
        player.ammo[ammo_type as usize] -= 1;
    }

    if let Some(mo_idx) = player.mobj {
        ctx.p_set_mobj_state(mo_idx, StateNum::S_PLAY_ATK2);
    }

    // Flash state offset: current psp state offset from S_CHAIN1.
    // This alternates between the two chaingun flash frames.
    let flash_base = WEAPONINFO[player.readyweapon as usize].flashstate as usize;
    let psp_state_idx = player.psprites[psp_idx].state.unwrap_or(0);
    let chain1_idx = StateNum::S_CHAIN1 as usize;
    let offset = psp_state_idx.saturating_sub(chain1_idx);
    let flash_idx = flash_base + offset;
    if let Some(flash_stnum) = StateNum::from_index(flash_idx) {
        p_set_psprite(player, PS_FLASH, flash_stnum, ctx, rng);
    }

    let bulletslope = p_bullet_slope(player, ctx);
    let accurate = player.refire == 0;
    p_gun_shot(player, bulletslope, accurate, ctx, rng);
}

// ==========================================================================
// A_Light0/1/2 — muzzle flash brightness (p_pspr.c lines 751-760)
// ==========================================================================

/// Set muzzle flash extra light level to 0 (off).
///
/// Original C: `A_Light0` (p_pspr.c line 756)
pub fn a_light0(
    player: &mut Player,
    _psp_idx: usize,
    _ctx: &mut dyn PsprContext,
    _rng: &mut DoomRandom,
) {
    player.extralight = 0;
}

/// Set muzzle flash extra light level to 1.
///
/// Original C: `A_Light1` (p_pspr.c line 758)
pub fn a_light1(
    player: &mut Player,
    _psp_idx: usize,
    _ctx: &mut dyn PsprContext,
    _rng: &mut DoomRandom,
) {
    player.extralight = 1;
}

/// Set muzzle flash extra light level to 2.
///
/// Original C: `A_Light2` (p_pspr.c line 760)
pub fn a_light2(
    player: &mut Player,
    _psp_idx: usize,
    _ctx: &mut dyn PsprContext,
    _rng: &mut DoomRandom,
) {
    player.extralight = 2;
}

// ==========================================================================
// A_BFGSpray — BFG secondary explosion (p_pspr.c lines 765-823)
// ==========================================================================

/// BFG 9000 secondary explosion: 40-ray tracer damage.
///
/// Called on the BFG projectile (not the player) when it explodes. Fires
/// 40 auto-aim rays spread across ANG90 (±45° from center), each dealing
/// damage = sum of 15 × (P_Random() & 7 + 1) per target hit. Spawns a
/// MT_EXTRABFG visual effect at each hit target's position.
///
/// This function takes a mobj index (the exploding BFG projectile) rather
/// than a player+psprite, matching the original C signature.
///
/// Original C: `A_BFGSpray` (p_pspr.c lines 765-823)
pub fn a_bfg_spray(mo_idx: usize, ctx: &mut dyn PsprContext, rng: &mut DoomRandom) {
    // mo->target is the player who fired the BFG.
    let source_idx = match ctx.get_mobj(mo_idx).target {
        Some(idx) => idx,
        None => {
            tracing::warn!("A_BFGSpray: BFG projectile has no target (shooter)");
            return;
        }
    };

    let autoaim_range = Fixed(16 * 64 * FRACUNIT);

    // Fire 40 rays spread across ANG90 (centered).
    for i in 0..40 {
        // Angle: source.angle - ANG90/2 + ANG90/40 * i
        let source_angle = ctx.get_mobj(source_idx).angle;
        let base_offset = ANG90.0 / 2;
        let per_ray = ANG90.0 / 40;
        let an = Angle(
            source_angle
                .0
                .wrapping_sub(base_offset)
                .wrapping_add(per_ray.wrapping_mul(i as u32)),
        );

        // Auto-aim at this angle.
        let (_slope, linetarget) = ctx.p_aim_line_attack(source_idx, an, autoaim_range);

        let Some(target_idx) = linetarget else {
            continue;
        };

        // Spawn a MT_EXTRABFG visual at the target's position.
        let target = ctx.get_mobj(target_idx);
        let tx = target.x;
        let ty = target.y;
        let tz = Fixed(target.z.0.wrapping_add(target.height.0 >> 2));
        ctx.p_spawn_mobj(tx, ty, tz, MobjType::MT_EXTRABFG);

        // Calculate damage: sum of 15 random values (1-8 each).
        let mut damage: i32 = 0;
        for _ in 0..15 {
            damage += ((rng.p_random() & 7) as i32) + 1;
        }

        ctx.p_damage_mobj(target_idx, Some(source_idx), Some(source_idx), damage);
    }
}

// ==========================================================================
// A_BFGsound — BFG charging sound (p_pspr.c line 825)
// ==========================================================================

/// Play the BFG charging sound (sfx_bfg).
///
/// Original C: `A_BFGsound` (p_pspr.c line 825)
pub fn a_bfg_sound(
    player: &mut Player,
    _psp_idx: usize,
    ctx: &mut dyn PsprContext,
    _rng: &mut DoomRandom,
) {
    if let Some(mo_idx) = player.mobj {
        ctx.s_start_sound(Some(mo_idx), SfxEnum::sfx_bfg);
    }
}

// ==========================================================================
// P_SetupPsprites — initialize weapon sprites (p_pspr.c lines 831-842)
// ==========================================================================

/// Initialize all player weapon sprites at game start or level load.
///
/// Clears all psprite states to None, then begins raising the player's
/// current ready weapon from the bottom of the screen.
///
/// Original C: `P_SetupPsprites` (p_pspr.c lines 831-842)
pub fn p_setup_psprites(player: &mut Player, ctx: &mut dyn PsprContext, rng: &mut DoomRandom) {
    // Remove all psprites.
    for i in 0..NUMPSPRITES {
        player.psprites[i].state = None;
        player.psprites[i].tics = -1;
    }

    // Set the current weapon.
    player.pendingweapon = player.readyweapon;
    p_bring_up_weapon(player, ctx, rng);

    tracing::trace!(
        "P_SetupPsprites: initialized weapon sprites for weapon {:?}",
        player.readyweapon
    );
}

// ==========================================================================
// P_MovePsprites — advance psprite animations (p_pspr.c lines 851-877)
// ==========================================================================

/// Advance all player weapon sprite animations by one tic.
///
/// For each active psprite: decrements its tic counter (unless -1 for
/// infinite duration), and advances to the next state when the counter
/// reaches zero. After processing, synchronizes the flash psprite position
/// (sx, sy) with the weapon psprite position.
///
/// Original C: `P_MovePsprites` (p_pspr.c lines 851-877)
pub fn p_move_psprites(player: &mut Player, ctx: &mut dyn PsprContext, rng: &mut DoomRandom) {
    for i in 0..NUMPSPRITES {
        // A null state means the psprite is not active.
        if player.psprites[i].state.is_none() {
            continue;
        }

        // Drop tic count. A tic value of -1 means infinite duration (never advance).
        if player.psprites[i].tics != -1 {
            player.psprites[i].tics -= 1;

            if player.psprites[i].tics > 0 {
                continue;
            }

            // Tics reached zero — advance to next state.
            if let Some(state_idx) = player.psprites[i].state {
                let nextstate = STATES[state_idx].nextstate;
                p_set_psprite(player, i, nextstate, ctx, rng);
            }
        }
    }

    // Synchronize flash psprite position with weapon psprite position.
    player.psprites[PS_FLASH].sx = player.psprites[PS_WEAPON].sx;
    player.psprites[PS_FLASH].sy = player.psprites[PS_WEAPON].sy;
}
