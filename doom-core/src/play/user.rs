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

//! Player related stuff: bobbing POV/weapon, movement, pending weapon.
//!
//! Translated from linuxdoom-1.10/p_user.c
//!
//! This module implements the per-tic player processing:
//! - [`p_thrust`]: Apply horizontal thrust to the player's mobj.
//! - [`p_calc_height`]: Calculate view height with bobbing.
//! - [`p_move_player`]: Process movement input (turning, thrust, animation).
//! - [`p_death_think`]: Handle the death state (view falling, turn toward killer).
//! - [`p_player_think`]: Main per-tic player logic (cheat flags, weapon change,
//!   powerup countdowns, colormap effects, and delegation to the above functions).

use crate::info::states::StateNum;
use crate::play::mobj::VIEWHEIGHT;
use crate::types::angle::{Angle, ANG180, ANG90, ANGLETOFINESHIFT, FINEANGLES, FINEMASK};
use crate::types::doomdef::{GameMode, PowerType, WeaponType};
use crate::types::event::{BT_CHANGE, BT_SPECIAL, BT_USE, BT_WEAPONMASK, BT_WEAPONSHIFT};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::mobj::{MapObject, MobjFlags};
use crate::types::player::{CheatFlags, Player, PlayerState};
use crate::types::tables::{finecosine, point_to_angle2, FINESINE};

// =============================================================================
// Constants (p_user.c top-level defines and statics)
// =============================================================================

/// Index of the special effects (INVUL inverse) colormap.
/// Original C: `#define INVERSECOLORMAP 32`.
const INVERSECOLORMAP: i32 = 32;

/// Maximum view bobbing amplitude (16 pixels in 16.16 fixed-point).
/// Original C: `#define MAXBOB 0x100000`.
const MAXBOB: Fixed = Fixed(0x100000);

/// 5-degree angle step for turning toward the killer in death state.
/// Original C: `#define ANG5 (ANG90/18)`.
const ANG5: Angle = Angle(ANG90.0 / 18);

// =============================================================================
// UserContext trait — cross-cutting access needed by this module
// =============================================================================

/// Context trait for the player think / movement functions.
///
/// This trait abstracts all access to game state that the player processing
/// functions need: map objects, player data, game mode, level timing, sector
/// queries, and cross-module function dispatch.
///
/// The caller (game loop / tick driver) implements this trait to provide
/// concrete access to the arena-allocated game state.
pub trait UserContext {
    // --- Map object access ---

    /// Get an immutable reference to a map object by arena index.
    fn get_mobj(&self, idx: usize) -> &MapObject;

    /// Get a mutable reference to a map object by arena index.
    fn get_mobj_mut(&mut self, idx: usize) -> &mut MapObject;

    // --- Player access ---

    /// Get an immutable reference to a player by index.
    fn get_player(&self, idx: usize) -> &Player;

    /// Get a mutable reference to a player by index.
    fn get_player_mut(&mut self, idx: usize) -> &mut Player;

    // --- Game state ---

    /// Current game mode (Shareware, Registered, Commercial, Retail).
    fn game_mode(&self) -> GameMode;

    /// Current level time in tics. Used for view bob angle calculation.
    fn level_time(&self) -> i32;

    // --- Sector queries ---

    /// Returns the sector special value for the sector containing the given mobj.
    ///
    /// Traverses `mobj.subsector → subsector.sector → sector.special`.
    /// Returns 0 if the mobj has no valid subsector.
    fn get_mobj_sector_special(&self, mobj_idx: usize) -> i16;

    // --- Cross-module function dispatch ---

    /// `P_SetMobjState` — change a map object's animation state.
    /// Returns `false` if the mobj was removed (entered `S_NULL`).
    fn p_set_mobj_state(&mut self, mobj_idx: usize, state: StateNum) -> bool;

    /// `P_MovePsprites` — advance weapon sprite animation for the given player.
    fn p_move_psprites(&mut self, player_idx: usize);

    /// `P_PlayerInSpecialSector` — handle sector-based damage and secrets.
    fn p_player_in_special_sector(&mut self, player_idx: usize);

    /// `P_UseLines` — activate usable lines in front of the player.
    fn p_use_lines(&mut self, player_idx: usize);
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Convert a raw weapon index (from button mask extraction) to a [`WeaponType`].
///
/// Values 0–8 map to the corresponding weapon; any other value yields
/// [`WeaponType::NoChange`].
fn weapon_from_usize(val: usize) -> WeaponType {
    match val {
        0 => WeaponType::Fist,
        1 => WeaponType::Pistol,
        2 => WeaponType::Shotgun,
        3 => WeaponType::Chaingun,
        4 => WeaponType::Missile,
        5 => WeaponType::Plasma,
        6 => WeaponType::Bfg,
        7 => WeaponType::Chainsaw,
        8 => WeaponType::SuperShotgun,
        _ => WeaponType::NoChange,
    }
}

// =============================================================================
// P_Thrust (p_user.c lines 54–68)
// =============================================================================

/// Apply horizontal thrust to the player's map object along the given angle.
///
/// Adds `move_val * cos(angle)` to `momx` and `move_val * sin(angle)` to
/// `momy` of the player's mobj. Used by [`p_move_player`] for forward and
/// sideways movement.
///
/// Original C: `void P_Thrust(player_t* player, angle_t angle, fixed_t move)`
pub fn p_thrust(player_idx: usize, angle: Angle, move_val: Fixed, ctx: &mut dyn UserContext) {
    let mobj_idx = match ctx.get_player(player_idx).mobj {
        Some(idx) => idx,
        None => return,
    };

    let fine = (angle.0 >> ANGLETOFINESHIFT) as usize;
    let cos_val = finecosine(fine);
    let sin_val = FINESINE[fine];
    let dx = move_val.fixed_mul(cos_val);
    let dy = move_val.fixed_mul(sin_val);

    let mo = ctx.get_mobj_mut(mobj_idx);
    mo.momx = mo.momx + dx;
    mo.momy = mo.momy + dy;
}

// =============================================================================
// P_CalcHeight (p_user.c lines 77–141)
// =============================================================================

/// Calculate the walking / running height adjustment (view bobbing).
///
/// Computes the bob magnitude from the player's horizontal momentum, then
/// applies a sine-wave modulation based on `level_time` to produce the
/// per-tic `viewz` value. Also handles the `VIEWHEIGHT` adjustment for
/// landing (`deltaviewheight`) and the special case when `CF_NOMOMENTUM` is
/// active or the player is airborne.
///
/// # Behavioral note
///
/// The original C code at lines 98–104 sets `viewz` to `mo->z + VIEWHEIGHT`,
/// applies a ceiling clamp, and then immediately overwrites `viewz` with
/// `mo->z + player->viewheight`. This dead-code ceiling clamp is intentionally
/// preserved for behavioral parity with the original engine.
///
/// Original C: `void P_CalcHeight(player_t* player)` (p_user.c lines 77–141)
pub fn p_calc_height(player_idx: usize, on_ground: bool, ctx: &mut dyn UserContext) {
    // Retrieve mobj index
    let mobj_idx = match ctx.get_player(player_idx).mobj {
        Some(idx) => idx,
        None => return,
    };

    // Read mobj position data (short-lived immutable borrow)
    let (mobj_z, mobj_ceilingz, momx, momy) = {
        let mo = ctx.get_mobj(mobj_idx);
        (mo.z, mo.ceilingz, mo.momx, mo.momy)
    };

    // -------------------------------------------------------------------------
    // Compute bob magnitude: momx² + momy², shifted right 2, clamped to MAXBOB
    // Original C: player->bob = FixedMul(momx,momx) + FixedMul(momy,momy);
    //             player->bob >>= 2;
    //             if (player->bob > MAXBOB) player->bob = MAXBOB;
    // -------------------------------------------------------------------------
    let bob_magnitude = {
        let raw = momx.fixed_mul(momx) + momy.fixed_mul(momy);
        let shifted = Fixed(raw.0 >> 2);
        if shifted.0 > MAXBOB.0 {
            MAXBOB
        } else {
            shifted
        }
    };
    ctx.get_player_mut(player_idx).bob = bob_magnitude;

    // -------------------------------------------------------------------------
    // Special case: CF_NOMOMENTUM cheat or player is airborne
    // -------------------------------------------------------------------------
    let player_cheats = ctx.get_player(player_idx).cheats;
    if player_cheats & CheatFlags::CF_NOMOMENTUM.bits() != 0 || !on_ground {
        // Set viewz to mo.z + VIEWHEIGHT, then ceiling-clamp
        let mut viewz = mobj_z + VIEWHEIGHT;
        let ceiling_limit = mobj_ceilingz - Fixed(4 * FRACUNIT);
        if viewz.0 > ceiling_limit.0 {
            viewz = ceiling_limit;
        }

        // Original C line 104: overwrites viewz with mo.z + player.viewheight.
        // This makes the ceiling clamp above effectively dead code.
        // Preserved exactly for behavioral parity.
        let player = ctx.get_player_mut(player_idx);
        player.viewz = viewz; // immediately overwritten — matches original
        player.viewz = mobj_z + player.viewheight;
        return;
    }

    // -------------------------------------------------------------------------
    // Normal path: compute angle-modulated bob offset
    // Original C: angle = (FINEANGLES/20*leveltime) & FINEMASK;
    //             bob = FixedMul(player->bob/2, finesine[angle]);
    // -------------------------------------------------------------------------
    let level_time = ctx.level_time();
    let raw_angle = (FINEANGLES as i32 / 20).wrapping_mul(level_time);
    let angle_idx = (raw_angle as u32 & FINEMASK) as usize;
    let bob_offset = Fixed(bob_magnitude.0 / 2).fixed_mul(FINESINE[angle_idx]);

    // -------------------------------------------------------------------------
    // Move viewheight — only when alive (PST_LIVE)
    // -------------------------------------------------------------------------
    let playerstate = ctx.get_player(player_idx).playerstate;
    if playerstate == PlayerState::Live {
        let player = ctx.get_player_mut(player_idx);
        player.viewheight = player.viewheight + player.deltaviewheight;

        if player.viewheight.0 > VIEWHEIGHT.0 {
            player.viewheight = VIEWHEIGHT;
            player.deltaviewheight = Fixed::ZERO;
        }

        let half_viewheight = Fixed(VIEWHEIGHT.0 / 2);
        if player.viewheight.0 < half_viewheight.0 {
            player.viewheight = half_viewheight;
            if player.deltaviewheight.0 <= 0 {
                player.deltaviewheight = Fixed(1);
            }
        }

        if player.deltaviewheight.0 != 0 {
            player.deltaviewheight = player.deltaviewheight + Fixed(FRACUNIT / 4);
            if player.deltaviewheight.0 == 0 {
                player.deltaviewheight = Fixed(1);
            }
        }
    }

    // -------------------------------------------------------------------------
    // Final viewz = mo.z + viewheight + bob, clamped to ceiling - 4 units
    // -------------------------------------------------------------------------
    let viewheight = ctx.get_player(player_idx).viewheight;
    let mut viewz = mobj_z + viewheight + bob_offset;
    let ceiling_limit = mobj_ceilingz - Fixed(4 * FRACUNIT);
    if viewz.0 > ceiling_limit.0 {
        viewz = ceiling_limit;
    }
    ctx.get_player_mut(player_idx).viewz = viewz;
}

// =============================================================================
// P_MovePlayer (p_user.c lines 148–171)
// =============================================================================

/// Process player movement input: turning, thrust, and animation transition.
///
/// Applies the tic command's `angleturn` to the mobj angle, checks whether
/// the player is on the ground, applies forward and side thrust if grounded,
/// and transitions the player's animation state from standing to running
/// when movement input is detected.
///
/// Sets `*on_ground` based on the player's vertical position relative to
/// the floor, mirroring the original file-level `onground` variable.
///
/// Original C: `void P_MovePlayer(player_t* player)` (p_user.c lines 148–171)
pub fn p_move_player(player_idx: usize, on_ground: &mut bool, ctx: &mut dyn UserContext) {
    // Get mobj index and read tic command
    let mobj_idx = match ctx.get_player(player_idx).mobj {
        Some(idx) => idx,
        None => return,
    };

    let cmd = ctx.get_player(player_idx).cmd;

    // Apply turning: mo.angle += (angleturn << 16)
    // In the original C, angleturn is signed short, shifted left 16 bits.
    {
        let mo = ctx.get_mobj_mut(mobj_idx);
        let delta = ((cmd.angleturn as i32) << 16) as u32;
        mo.angle = Angle(mo.angle.0.wrapping_add(delta));
    }

    // Determine if the player is on the ground.
    // Do not let the player control movement if not onground.
    {
        let mo = ctx.get_mobj(mobj_idx);
        *on_ground = mo.z.0 <= mo.floorz.0;
    }

    // Apply forward thrust if on ground
    let mo_angle = ctx.get_mobj(mobj_idx).angle;
    if cmd.forwardmove != 0 && *on_ground {
        p_thrust(
            player_idx,
            mo_angle,
            Fixed((cmd.forwardmove as i32) * 2048),
            ctx,
        );
    }

    // Apply side thrust if on ground (perpendicular: angle - ANG90)
    if cmd.sidemove != 0 && *on_ground {
        let side_angle = Angle(mo_angle.0.wrapping_sub(ANG90.0));
        p_thrust(
            player_idx,
            side_angle,
            Fixed((cmd.sidemove as i32) * 2048),
            ctx,
        );
    }

    // Transition from standing (S_PLAY) to running (S_PLAY_RUN1) animation
    // if there is any movement input.
    if (cmd.forwardmove != 0 || cmd.sidemove != 0)
        && ctx.get_mobj(mobj_idx).state == Some(StateNum::S_PLAY as usize)
    {
        ctx.p_set_mobj_state(mobj_idx, StateNum::S_PLAY_RUN1);
    }
}

// =============================================================================
// P_DeathThink (p_user.c lines 182–229)
// =============================================================================

/// Process the player's death state: lower the view, turn toward the killer,
/// and wait for the use button to trigger respawn.
///
/// Called each tic from [`p_player_think`] when `playerstate == PST_DEAD`.
/// Handles weapon sprite animation (lowering weapons), view height falling
/// toward 6 units, turning toward the attacker, and respawning when `BT_USE`
/// is pressed.
///
/// Original C: `void P_DeathThink(player_t* player)` (p_user.c lines 182–229)
pub fn p_death_think(player_idx: usize, on_ground: &mut bool, ctx: &mut dyn UserContext) {
    // Advance weapon sprite animation (weapon lowering during death)
    ctx.p_move_psprites(player_idx);

    let mobj_idx = match ctx.get_player(player_idx).mobj {
        Some(idx) => idx,
        None => return,
    };

    // Fall to the ground: lower viewheight toward 6*FRACUNIT
    let six_units = Fixed(6 * FRACUNIT);
    {
        let player = ctx.get_player_mut(player_idx);
        if player.viewheight.0 > six_units.0 {
            player.viewheight = player.viewheight - Fixed(FRACUNIT);
        }
        if player.viewheight.0 < six_units.0 {
            player.viewheight = six_units;
        }
        player.deltaviewheight = Fixed::ZERO;
    }

    // Set on_ground state
    {
        let mo = ctx.get_mobj(mobj_idx);
        *on_ground = mo.z.0 <= mo.floorz.0;
    }

    // Calculate view height (uses on_ground)
    p_calc_height(player_idx, *on_ground, ctx);

    // -------------------------------------------------------------------------
    // Turn toward the attacker
    // -------------------------------------------------------------------------
    let attacker_idx = ctx.get_player(player_idx).attacker;
    if let Some(att_idx) = attacker_idx {
        if att_idx != mobj_idx {
            // Compute angle from player to attacker via R_PointToAngle2
            let (mo_x, mo_y, mo_angle) = {
                let mo = ctx.get_mobj(mobj_idx);
                (mo.x, mo.y, mo.angle)
            };
            let (att_x, att_y) = {
                let att = ctx.get_mobj(att_idx);
                (att.x, att.y)
            };
            let angle = point_to_angle2(mo_x, mo_y, att_x, att_y);
            let delta = Angle(angle.0.wrapping_sub(mo_angle.0));

            // (unsigned)-ANG5 in C: two's complement negation of ANG5
            let neg_ang5 = Angle(0u32.wrapping_sub(ANG5.0));

            if delta.0 < ANG5.0 || delta.0 > neg_ang5.0 {
                // Looking at killer — snap angle and fade damage flash down
                ctx.get_mobj_mut(mobj_idx).angle = angle;
                let player = ctx.get_player_mut(player_idx);
                if player.damagecount > 0 {
                    player.damagecount -= 1;
                }
            } else if delta.0 < ANG180.0 {
                let mo = ctx.get_mobj_mut(mobj_idx);
                mo.angle = Angle(mo.angle.0.wrapping_add(ANG5.0));
            } else {
                let mo = ctx.get_mobj_mut(mobj_idx);
                mo.angle = Angle(mo.angle.0.wrapping_sub(ANG5.0));
            }
        } else {
            // Attacker is self — just decrement damage count
            let player = ctx.get_player_mut(player_idx);
            if player.damagecount > 0 {
                player.damagecount -= 1;
            }
        }
    } else {
        // No attacker — decrement damage count
        let player = ctx.get_player_mut(player_idx);
        if player.damagecount > 0 {
            player.damagecount -= 1;
        }
    }

    // Check for respawn (BT_USE pressed)
    let buttons = ctx.get_player(player_idx).cmd.buttons;
    if buttons & BT_USE != 0 {
        ctx.get_player_mut(player_idx).playerstate = PlayerState::Reborn;
    }
}

// =============================================================================
// P_PlayerThink (p_user.c lines 236–384)
// =============================================================================

/// Main per-tic player processing: cheat flags, weapon switching, movement,
/// powerup countdowns, and colormap effects.
///
/// This is the top-level function called once per game tic for each player.
/// It coordinates all player sub-systems in the exact same order as the
/// original C implementation to preserve deterministic behavior.
///
/// Original C: `void P_PlayerThink(player_t* player)` (p_user.c lines 236–384)
pub fn p_player_think(player_idx: usize, ctx: &mut dyn UserContext) {
    let mobj_idx = match ctx.get_player(player_idx).mobj {
        Some(idx) => idx,
        None => return,
    };

    // Read the TicCmd buttons into a local before any mutations.
    // (forwardmove and sidemove are read directly inside P_MovePlayer.)
    let mut cmd_buttons: u8 = ctx.get_player(player_idx).cmd.buttons;

    // -------------------------------------------------------------------------
    // 1. Synchronise MF_NOCLIP flag with CF_NOCLIP cheat
    // -------------------------------------------------------------------------
    {
        let cheats = ctx.get_player(player_idx).cheats;
        let mo = ctx.get_mobj_mut(mobj_idx);
        if cheats & CheatFlags::CF_NOCLIP.bits() != 0 {
            mo.flags.insert(MobjFlags::MF_NOCLIP);
        } else {
            mo.flags.remove(MobjFlags::MF_NOCLIP);
        }
    }

    // -------------------------------------------------------------------------
    // 2. Chainsaw recoil handling (MF_JUSTATTACKED)
    //    Forces the player to face straight ahead for one tic.
    // -------------------------------------------------------------------------
    {
        let just_attacked = ctx
            .get_mobj(mobj_idx)
            .flags
            .contains(MobjFlags::MF_JUSTATTACKED);
        if just_attacked {
            // 0xc800/512 ≈ 100. Original C uses (cmd->forwardmove = 0xc800/512)
            // which is 100 in signed decimal. angleturn=0, sidemove=0.
            {
                let player = ctx.get_player_mut(player_idx);
                player.cmd.angleturn = 0;
                player.cmd.forwardmove = 100; // 0xc800/512
                player.cmd.sidemove = 0;
            }
            ctx.get_mobj_mut(mobj_idx)
                .flags
                .remove(MobjFlags::MF_JUSTATTACKED);
        }
    }

    // -------------------------------------------------------------------------
    // 3. Handle dead player state
    // -------------------------------------------------------------------------
    if ctx.get_player(player_idx).playerstate == PlayerState::Dead {
        let mut on_ground = false;
        p_death_think(player_idx, &mut on_ground, ctx);
        return;
    }

    // -------------------------------------------------------------------------
    // 4. If BT_SPECIAL, clear buttons (prevents weapon change in same tic)
    // -------------------------------------------------------------------------
    if cmd_buttons & BT_SPECIAL != 0 {
        // Original C: cmd->buttons = 0; (cmd points to player->cmd)
        // Clear both the local copy and the actual player command.
        cmd_buttons = 0;
        ctx.get_player_mut(player_idx).cmd.buttons = 0;
    }

    // -------------------------------------------------------------------------
    // 5. Reaction time — decrement or process movement
    // -------------------------------------------------------------------------
    {
        let rt = ctx.get_mobj(mobj_idx).reactiontime;
        if rt > 0 {
            ctx.get_mobj_mut(mobj_idx).reactiontime -= 1;
        } else {
            let mut on_ground = false;
            p_move_player(player_idx, &mut on_ground, ctx);
        }
    }

    // -------------------------------------------------------------------------
    // 6. Calculate the walking / running height adjustment
    // -------------------------------------------------------------------------
    {
        let mo = ctx.get_mobj(mobj_idx);
        let on_ground = mo.z.0 <= mo.floorz.0;
        p_calc_height(player_idx, on_ground, ctx);
    }

    // -------------------------------------------------------------------------
    // 7. Check for special sector effects (damage, secret, etc.)
    // -------------------------------------------------------------------------
    {
        let special = ctx.get_mobj_sector_special(mobj_idx);
        if special != 0 {
            ctx.p_player_in_special_sector(player_idx);
        }
    }

    // -------------------------------------------------------------------------
    // 8. Weapon switching logic
    // -------------------------------------------------------------------------
    if cmd_buttons & BT_CHANGE != 0 {
        // Extract the weapon number from the button byte.
        let newweapon_raw = ((cmd_buttons & BT_WEAPONMASK) >> BT_WEAPONSHIFT) as usize;
        let mut newweapon = weapon_from_usize(newweapon_raw);

        // Fist → Chainsaw upgrade: if selecting fist and player owns chainsaw,
        // and it is not the case that player already has chainsaw + strength.
        let game_mode = ctx.game_mode();
        {
            let player = ctx.get_player(player_idx);
            if newweapon == WeaponType::Fist
                && player.weaponowned[WeaponType::Chainsaw as usize]
                && !(player.readyweapon == WeaponType::Chainsaw
                    && player.powers[PowerType::Strength as usize] != 0)
            {
                newweapon = WeaponType::Chainsaw;
            }
        }

        // Commercial mode: Shotgun → Super Shotgun upgrade if owned
        if game_mode == GameMode::Commercial {
            let player = ctx.get_player(player_idx);
            if newweapon == WeaponType::Shotgun
                && player.weaponowned[WeaponType::SuperShotgun as usize]
                && player.readyweapon != WeaponType::SuperShotgun
            {
                newweapon = WeaponType::SuperShotgun;
            }
        }

        // Shareware restriction: block plasma and BFG
        let blocked = if game_mode == GameMode::Shareware {
            newweapon == WeaponType::Plasma || newweapon == WeaponType::Bfg
        } else {
            false
        };

        // Set pending weapon if player owns it, it is different from ready,
        // and it is not blocked by shareware restriction.
        if !blocked {
            let player = ctx.get_player(player_idx);
            if player.weaponowned[newweapon as usize] && newweapon != player.readyweapon {
                ctx.get_player_mut(player_idx).pendingweapon = newweapon;
            }
        }
    }

    // -------------------------------------------------------------------------
    // 9. Use button handling (BT_USE)
    // -------------------------------------------------------------------------
    if cmd_buttons & BT_USE != 0 {
        if ctx.get_player(player_idx).usedown == 0 {
            ctx.p_use_lines(player_idx);
            ctx.get_player_mut(player_idx).usedown = 1;
        }
    } else {
        ctx.get_player_mut(player_idx).usedown = 0;
    }

    // -------------------------------------------------------------------------
    // 10. Advance weapon sprite animation
    // -------------------------------------------------------------------------
    ctx.p_move_psprites(player_idx);

    // -------------------------------------------------------------------------
    // 11. Power countdown logic
    //     pw_strength counts UP, all others count DOWN.
    // -------------------------------------------------------------------------
    let mut clear_shadow = false;
    {
        let player = ctx.get_player_mut(player_idx);

        // pw_strength: counts UP (berserk "wears off" visually but mechanically stays)
        if player.powers[PowerType::Strength as usize] != 0 {
            player.powers[PowerType::Strength as usize] += 1;
        }

        // pw_invulnerability: counts down
        if player.powers[PowerType::Invulnerability as usize] > 0 {
            player.powers[PowerType::Invulnerability as usize] -= 1;
        }

        // pw_invisibility: counts down, clear MF_SHADOW when it reaches 0
        if player.powers[PowerType::Invisibility as usize] > 0 {
            player.powers[PowerType::Invisibility as usize] -= 1;
            if player.powers[PowerType::Invisibility as usize] == 0 {
                clear_shadow = true;
            }
        }

        // pw_infrared: counts down
        if player.powers[PowerType::InfraRed as usize] > 0 {
            player.powers[PowerType::InfraRed as usize] -= 1;
        }

        // pw_ironfeet: counts down
        if player.powers[PowerType::IronFeet as usize] > 0 {
            player.powers[PowerType::IronFeet as usize] -= 1;
        }

        // Damage / bonus screen flash counters
        if player.damagecount > 0 {
            player.damagecount -= 1;
        }
        if player.bonuscount > 0 {
            player.bonuscount -= 1;
        }
    }

    // Clear MF_SHADOW from mobj if invisibility just expired
    if clear_shadow {
        ctx.get_mobj_mut(mobj_idx)
            .flags
            .remove(MobjFlags::MF_SHADOW);
    }

    // -------------------------------------------------------------------------
    // 12. Colormap handling (invulnerability glow, infrared, normal)
    // -------------------------------------------------------------------------
    {
        let player = ctx.get_player(player_idx);
        let pw_invul = player.powers[PowerType::Invulnerability as usize];
        let pw_infra = player.powers[PowerType::InfraRed as usize];

        let new_colormap = if pw_invul > 0 {
            // Full strength: solid INVERSECOLORMAP.
            // Blinking at the end: > 4*32 ticks remaining, or (value & 8) non-zero.
            if pw_invul > 4 * 32 || (pw_invul & 8) != 0 {
                INVERSECOLORMAP
            } else {
                0
            }
        } else if pw_infra > 0 {
            // Infrared goggle effect with blinking at end.
            if pw_infra > 4 * 32 || (pw_infra & 8) != 0 {
                1
            } else {
                0
            }
        } else {
            0
        };

        ctx.get_player_mut(player_idx).fixedcolormap = new_colormap;
    }
}
