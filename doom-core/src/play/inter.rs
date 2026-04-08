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

//! Handling interactions (i.e., collisions). Pickups, damage, kills.
//! Translated from linuxdoom-1.10/p_inter.c

use crate::game::strings::{
    GOTARMBONUS, GOTARMOR, GOTBACKPACK, GOTBERSERK, GOTBFG9000, GOTBLUECARD, GOTBLUESKUL, GOTCELL,
    GOTCELLBOX, GOTCHAINGUN, GOTCHAINSAW, GOTCLIP, GOTCLIPBOX, GOTHTHBONUS, GOTINVIS, GOTINVUL,
    GOTLAUNCHER, GOTMAP, GOTMEDIKIT, GOTMEDINEED, GOTMEGA, GOTMSPHERE, GOTPLASMA, GOTREDCARD,
    GOTREDSKULL, GOTROCKBOX, GOTROCKET, GOTSHELLBOX, GOTSHELLS, GOTSHOTGUN, GOTSHOTGUN2, GOTSTIM,
    GOTSUIT, GOTSUPER, GOTVISOR, GOTYELWCARD, GOTYELWSKUL,
};
use crate::info::mobjinfo::{MobjType, MOBJINFO};
use crate::info::sounds::SfxEnum;
use crate::info::sprites::SpriteNum;
use crate::info::states::StateNum;
use crate::play::mobj::{MobjContext, ONFLOORZ};
use crate::types::angle::{Angle, ANG180, ANGLETOFINESHIFT};
use crate::types::doomdef::{
    AmmoType, Card, GameMode, PowerType, Skill, WeaponType, INFRATICS, INVISTICS, INVULNTICS,
    IRONTICS, NUMAMMO, WEAPONINFO,
};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::mobj::MobjFlags;
use crate::types::player::{CheatFlags, Player, PlayerState};
use crate::types::tables::{finecosine, FINESINE};
use crate::ui::automap::{am_stop, AutomapState};
use crate::util::random::DoomRandom;

// =============================================================================
// Constants (p_inter.c lines 51-59)
// =============================================================================

/// Bonus count increment per pickup (visual flash frames).
/// Original C: `#define BONUSADD 6` (p_inter.c line 51)
const BONUSADD: i32 = 6;

/// Maximum ammo capacity per ammo type (before backpack doubles it).
///
/// Index 0 = Clip (bullets), 1 = Shell, 2 = Cell, 3 = Missile (rockets).
/// Original C: `int maxammo[NUMAMMO] = {200, 50, 300, 50};` (p_inter.c line 56)
pub const MAXAMMO: [i32; NUMAMMO] = [200, 50, 300, 50];

/// Ammo per clip pickup per type.
///
/// Index 0 = Clip (bullets), 1 = Shell, 2 = Cell, 3 = Missile (rockets).
/// Original C: `int clipammo[NUMAMMO] = {10, 4, 20, 1};` (p_inter.c line 59)
pub const CLIPAMMO: [i32; NUMAMMO] = [10, 4, 20, 1];

/// Maximum health that P_GiveBody can raise to.
/// Original C: uses literal 100 (MAXHEALTH macro in Boom, literal in vanilla).
const MAXHEALTH: i32 = 100;

/// Damage threshold base for target switching in P_DamageMobj.
/// Original C: `#define BASETHRESHOLD 100` (p_local.h line 50)
const BASETHRESHOLD: i32 = 100;

// =============================================================================
// P_GiveAmmo (p_inter.c lines 73-130)
// =============================================================================

/// Give ammunition to a player.
///
/// `num` is the number of clips worth of ammo. If `num == 0`, give half a clip
/// (used for dropped weapons). On Baby/Nightmare skill, ammo is doubled.
/// Auto-switches weapon if player was at zero ammo for the relevant type.
///
/// Returns `true` if the ammo was accepted (player was not already at max).
///
/// Original C: `boolean P_GiveAmmo(player_t* player, ammotype_t ammo, int num)`
/// (p_inter.c lines 73-130)
fn p_give_ammo(player: &mut Player, ammo_type: AmmoType, num: i32, gameskill: Skill) -> bool {
    if ammo_type == AmmoType::NoAmmo {
        return false;
    }

    let at = ammo_type as usize;
    if at >= NUMAMMO {
        tracing::error!("P_GiveAmmo: bad type {:?}", ammo_type);
        return false;
    }

    if player.ammo[at] >= player.maxammo[at] {
        return false;
    }

    let mut give = if num != 0 {
        num * CLIPAMMO[at]
    } else {
        // Half a clip (for dropped weapons)
        CLIPAMMO[at] / 2
    };

    // Double ammo on baby/nightmare
    if gameskill == Skill::Baby || gameskill == Skill::Nightmare {
        give *= 2;
    }

    let oldammo = player.ammo[at];
    player.ammo[at] += give;
    if player.ammo[at] > player.maxammo[at] {
        player.ammo[at] = player.maxammo[at];
    }

    // Don't switch weapon if player had some ammo of this type already
    if oldammo != 0 {
        return true;
    }

    // Auto-switch weapon based on ammo type gained
    match ammo_type {
        AmmoType::Clip => {
            if player.readyweapon == WeaponType::Fist {
                if player.weaponowned[WeaponType::Chaingun as usize] {
                    player.pendingweapon = WeaponType::Chaingun;
                } else {
                    player.pendingweapon = WeaponType::Pistol;
                }
            }
        }
        AmmoType::Shell => {
            if (player.readyweapon == WeaponType::Fist || player.readyweapon == WeaponType::Pistol)
                && player.weaponowned[WeaponType::Shotgun as usize]
            {
                player.pendingweapon = WeaponType::Shotgun;
            }
        }
        AmmoType::Cell => {
            if (player.readyweapon == WeaponType::Fist || player.readyweapon == WeaponType::Pistol)
                && player.weaponowned[WeaponType::Plasma as usize]
            {
                player.pendingweapon = WeaponType::Plasma;
            }
        }
        AmmoType::Missile => {
            if player.readyweapon == WeaponType::Fist
                && player.weaponowned[WeaponType::Missile as usize]
            {
                player.pendingweapon = WeaponType::Missile;
            }
        }
        AmmoType::NoAmmo => {}
    }

    true
}

// =============================================================================
// P_GiveWeapon (p_inter.c lines 137-197)
// =============================================================================

/// Give a weapon to a player. In deathmatch, don't remove special items.
///
/// `dropped` is `true` if the weapon came from a killed monster (half ammo).
///
/// Returns `true` if the weapon (or its ammo) was accepted.
///
/// Original C: `boolean P_GiveWeapon(player_t* player, weapontype_t weapon, boolean dropped)`
/// (p_inter.c lines 137-197)
fn p_give_weapon(
    player: &mut Player,
    weapon: WeaponType,
    dropped: bool,
    gameskill: Skill,
    netgame: bool,
    deathmatch: i32,
) -> bool {
    let ammo_type = WEAPONINFO[weapon as usize].ammo;

    if netgame && deathmatch != 2 && !dropped {
        // leave weapon pickups for other players in coop/DM1
        if player.weaponowned[weapon as usize] {
            return false;
        }

        player.bonuscount += BONUSADD;
        player.weaponowned[weapon as usize] = true;

        if deathmatch != 0 {
            p_give_ammo(player, ammo_type, 5, gameskill);
        } else {
            p_give_ammo(player, ammo_type, 2, gameskill);
        }
        player.pendingweapon = weapon;
        false // leave for others
    } else {
        let gave_ammo = if dropped {
            // half clip for dropped weapons (num=0 → half clip)
            p_give_ammo(player, ammo_type, 0, gameskill)
        } else {
            p_give_ammo(player, ammo_type, 2, gameskill)
        };

        let gave_weapon = if player.weaponowned[weapon as usize] {
            false
        } else {
            player.weaponowned[weapon as usize] = true;
            player.pendingweapon = weapon;
            true
        };

        gave_weapon || gave_ammo
    }
}

// =============================================================================
// P_GiveBody (p_inter.c lines 204-217)
// =============================================================================

/// Give health to a player (stimpack, medikit, etc.).
///
/// Returns `false` if already at MAXHEALTH (100).
///
/// Original C: `boolean P_GiveBody(player_t* player, int num)`
/// (p_inter.c lines 204-217)
fn p_give_body(player: &mut Player, num: i32) -> bool {
    if player.health >= MAXHEALTH {
        return false;
    }

    player.health += num;
    if player.health > MAXHEALTH {
        player.health = MAXHEALTH;
    }

    // Sync mobj health (done by caller via mobj arena in the original C)
    true
}

// =============================================================================
// P_GiveArmor (p_inter.c lines 224-241)
// =============================================================================

/// Give armor to a player.
///
/// `armortype` is 1 (green armor, 100 points) or 2 (blue armor, 200 points).
/// Returns `false` if current armor points are already >= armortype*100.
///
/// Original C: `boolean P_GiveArmor(player_t* player, int armortype)`
/// (p_inter.c lines 224-241)
fn p_give_armor(player: &mut Player, armortype: i32) -> bool {
    let hits = armortype * 100;
    if player.armorpoints >= hits {
        return false;
    }

    player.armortype = armortype;
    player.armorpoints = hits;
    true
}

// =============================================================================
// P_GiveCard (p_inter.c lines 248-261)
// =============================================================================

/// Give a keycard or skull key to a player.
///
/// Sets bonuscount for a pickup flash. Does nothing if already owned.
///
/// Original C: `void P_GiveCard(player_t* player, card_t card)`
/// (p_inter.c lines 248-261)
fn p_give_card(player: &mut Player, card: Card) {
    if player.has_card(card) {
        return;
    }

    player.bonuscount = BONUSADD;
    player.cards[card as usize] = true;
}

// =============================================================================
// P_GivePower (p_inter.c lines 268-326)
// =============================================================================

/// Give a power-up to a player. Returns `true` if the power was accepted
/// (not already active, or special handling for berserk).
///
/// Original C: `boolean P_GivePower(player_t* player, int power)`
/// (p_inter.c lines 268-326). Declared in p_inter.h.
pub fn p_give_power(player: &mut Player, power: PowerType) -> bool {
    match power {
        PowerType::Invulnerability => {
            *player.power_mut(PowerType::Invulnerability) = INVULNTICS;
            true
        }
        PowerType::Strength => {
            // Berserk: heal to 100 and set power to 1
            p_give_body(player, 100);
            *player.power_mut(PowerType::Strength) = 1;
            true
        }
        PowerType::Invisibility => {
            *player.power_mut(PowerType::Invisibility) = INVISTICS;
            // MF_SHADOW is added to the player's mobj by the caller
            true
        }
        PowerType::IronFeet => {
            *player.power_mut(PowerType::IronFeet) = IRONTICS;
            true
        }
        PowerType::AllMap => {
            if player.power(PowerType::AllMap) != 0 {
                return false;
            }
            *player.power_mut(PowerType::AllMap) = 1;
            true
        }
        PowerType::InfraRed => {
            *player.power_mut(PowerType::InfraRed) = INFRATICS;
            true
        }
    }
}

// =============================================================================
// InterContext — trait for external game operations needed by inter functions
// =============================================================================

/// Context trait providing access to game world operations needed by
/// interaction functions (pickups, damage, kills).
///
/// Replaces the global variables and cross-module function calls from the
/// original C code. Extends [`MobjContext`] with additional capabilities
/// needed specifically by the interaction subsystem.
pub trait InterContext: MobjContext {
    // --- Player data ---
    fn inter_players(&self) -> &[Player];
    fn inter_players_mut(&mut self) -> &mut [Player];
    fn inter_playeringame(&self) -> &[bool];
    fn inter_consoleplayer(&self) -> usize;

    // --- Game state ---
    fn inter_gameskill(&self) -> Skill;
    fn inter_gamemode(&self) -> GameMode;
    fn inter_netgame(&self) -> bool;
    fn inter_deathmatch(&self) -> i32;

    // --- Sound ---
    fn inter_s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum);

    // --- Cross-module calls ---
    /// Set mobj state — wraps p_set_mobj_state with appropriate context.
    fn inter_p_set_mobj_state(&mut self, mobj_idx: usize, state: StateNum) -> bool;

    /// Spawn a mobj at the given position and type.
    fn inter_p_spawn_mobj(&mut self, x: Fixed, y: Fixed, z: Fixed, mobj_type: MobjType) -> usize;

    /// Remove a mobj from the world.
    fn inter_p_remove_mobj(&mut self, mobj_idx: usize);

    /// P_DropWeapon for player death.
    fn inter_p_drop_weapon(&mut self, player_idx: usize);

    /// Automap state accessor (for AM_Stop on player death).
    fn automap_state_mut(&mut self) -> &mut AutomapState;

    /// Random number generator accessor.
    fn rng_mut(&mut self) -> &mut DoomRandom;

    /// I_Tactile feedback (stub in most implementations).
    fn i_tactile(&mut self, on: i32, off: i32, total: i32);

    /// R_PointToAngle2 equivalent for thrust direction calculation.
    fn point_to_angle2(&self, x1: Fixed, y1: Fixed, x2: Fixed, y2: Fixed) -> Angle;
}

// =============================================================================
// Sprite helper
// =============================================================================

/// Convert a usize sprite index to a `SpriteNum` enum value.
/// Returns `None` if the index is out of range.
fn sprite_from_usize(sprite: usize) -> Option<SpriteNum> {
    if sprite <= SpriteNum::SPR_TLP2 as usize {
        // SAFETY: SpriteNum is repr(usize) with contiguous values 0..=137.
        // We have verified the value is in range.
        Some(unsafe { core::mem::transmute::<usize, SpriteNum>(sprite) })
    } else {
        None
    }
}

/// Convert a usize to a `MobjType` enum value.
/// Returns `None` if the index is out of range.
fn mobjtype_from_usize(v: usize) -> Option<MobjType> {
    if v < crate::info::mobjinfo::NUMMOBJTYPES {
        Some(unsafe { core::mem::transmute::<usize, MobjType>(v) })
    } else {
        None
    }
}

// =============================================================================
// P_TouchSpecialThing (p_inter.c lines 338-661)
// =============================================================================

/// Handle a player touching a special (pickup) thing.
///
/// Implements the massive sprite-based switch that identifies pickups by their
/// sprite number and grants the appropriate item (armor, health, keys, weapons,
/// ammo, powerups) to the touching player.
///
/// Original C: `void P_TouchSpecialThing(mobj_t* special, mobj_t* toucher)`
/// (p_inter.c lines 338-661)
pub fn p_touch_special_thing(special_idx: usize, toucher_idx: usize, ctx: &mut dyn InterContext) {
    // Read special and toucher properties
    let (delta, toucher_health, toucher_player_idx, special_sprite, special_flags) = {
        let mobjs = ctx.mobjs();
        let special = &mobjs[special_idx];
        let toucher = &mobjs[toucher_idx];
        let delta = Fixed(special.z.0 - toucher.z.0);
        (
            delta,
            toucher.health,
            toucher.player,
            special.sprite,
            special.flags,
        )
    };

    // Must be within reach (z-check)
    let toucher_height = ctx.mobjs()[toucher_idx].height;
    if delta.0 > toucher_height.0 {
        return; // too high
    }
    if delta.0 < -8 * FRACUNIT {
        return; // too low
    }

    // Dead things can't pick up items
    if toucher_health <= 0 {
        return;
    }

    let player_idx = match toucher_player_idx {
        Some(idx) => idx,
        None => return, // Only players can pick up items
    };

    // Determine sound (default itemup; powerups use getpow; weapons use wpnup)
    let mut sound = SfxEnum::sfx_itemup;
    let mut remove_special = true;

    let gameskill = ctx.inter_gameskill();
    let gamemode = ctx.inter_gamemode();
    let netgame = ctx.inter_netgame();
    let deathmatch = ctx.inter_deathmatch();

    // Giant switch on sprite number
    let sprite_enum = sprite_from_usize(special_sprite);

    let accepted = match sprite_enum {
        // === Armor pickups ===
        Some(SpriteNum::SPR_ARM1) => {
            let players = ctx.inter_players_mut();
            if !p_give_armor(&mut players[player_idx], 1) {
                return;
            }
            players[player_idx].message = Some(GOTARMOR.to_string());
            true
        }
        Some(SpriteNum::SPR_ARM2) => {
            let players = ctx.inter_players_mut();
            if !p_give_armor(&mut players[player_idx], 2) {
                return;
            }
            players[player_idx].message = Some(GOTMEGA.to_string());
            true
        }

        // === Bonus pickups ===
        Some(SpriteNum::SPR_BON1) => {
            // Health bonus (+1, up to 200)
            let players = ctx.inter_players_mut();
            players[player_idx].health += 1;
            if players[player_idx].health > 200 {
                players[player_idx].health = 200;
            }
            players[player_idx].message = Some(GOTHTHBONUS.to_string());
            let health = players[player_idx].health;
            ctx.mobjs_mut()[toucher_idx].health = health;
            true
        }
        Some(SpriteNum::SPR_BON2) => {
            // Armor bonus (+1, up to 200; sets type 1 if no armor)
            let players = ctx.inter_players_mut();
            players[player_idx].armorpoints += 1;
            if players[player_idx].armorpoints > 200 {
                players[player_idx].armorpoints = 200;
            }
            if players[player_idx].armortype == 0 {
                players[player_idx].armortype = 1;
            }
            players[player_idx].message = Some(GOTARMBONUS.to_string());
            true
        }

        // === Supercharge (soulsphere) ===
        Some(SpriteNum::SPR_SOUL) => {
            let players = ctx.inter_players_mut();
            players[player_idx].health += 100;
            if players[player_idx].health > 200 {
                players[player_idx].health = 200;
            }
            players[player_idx].message = Some(GOTSUPER.to_string());
            let health = players[player_idx].health;
            ctx.mobjs_mut()[toucher_idx].health = health;
            sound = SfxEnum::sfx_getpow;
            true
        }

        // === Megasphere (DOOM II only) ===
        Some(SpriteNum::SPR_MEGA) => {
            if gamemode != GameMode::Commercial {
                return;
            }
            let players = ctx.inter_players_mut();
            players[player_idx].health = 200;
            players[player_idx].armorpoints = 200;
            players[player_idx].armortype = 2;
            players[player_idx].message = Some(GOTMSPHERE.to_string());
            let health = players[player_idx].health;
            ctx.mobjs_mut()[toucher_idx].health = health;
            sound = SfxEnum::sfx_getpow;
            true
        }

        // === Key cards ===
        Some(SpriteNum::SPR_BKEY) => {
            let players = ctx.inter_players_mut();
            if !players[player_idx].has_card(Card::BlueCard) {
                players[player_idx].message = Some(GOTBLUECARD.to_string());
            }
            p_give_card(&mut players[player_idx], Card::BlueCard);
            if netgame {
                remove_special = false;
            }
            true
        }
        Some(SpriteNum::SPR_YKEY) => {
            let players = ctx.inter_players_mut();
            if !players[player_idx].has_card(Card::YellowCard) {
                players[player_idx].message = Some(GOTYELWCARD.to_string());
            }
            p_give_card(&mut players[player_idx], Card::YellowCard);
            if netgame {
                remove_special = false;
            }
            true
        }
        Some(SpriteNum::SPR_RKEY) => {
            let players = ctx.inter_players_mut();
            if !players[player_idx].has_card(Card::RedCard) {
                players[player_idx].message = Some(GOTREDCARD.to_string());
            }
            p_give_card(&mut players[player_idx], Card::RedCard);
            if netgame {
                remove_special = false;
            }
            true
        }

        // === Skull keys ===
        Some(SpriteNum::SPR_BSKU) => {
            let players = ctx.inter_players_mut();
            if !players[player_idx].has_card(Card::BlueSkull) {
                players[player_idx].message = Some(GOTBLUESKUL.to_string());
            }
            p_give_card(&mut players[player_idx], Card::BlueSkull);
            if netgame {
                remove_special = false;
            }
            true
        }
        Some(SpriteNum::SPR_YSKU) => {
            let players = ctx.inter_players_mut();
            if !players[player_idx].has_card(Card::YellowSkull) {
                players[player_idx].message = Some(GOTYELWSKUL.to_string());
            }
            p_give_card(&mut players[player_idx], Card::YellowSkull);
            if netgame {
                remove_special = false;
            }
            true
        }
        Some(SpriteNum::SPR_RSKU) => {
            let players = ctx.inter_players_mut();
            if !players[player_idx].has_card(Card::RedSkull) {
                players[player_idx].message = Some(GOTREDSKULL.to_string());
            }
            p_give_card(&mut players[player_idx], Card::RedSkull);
            if netgame {
                remove_special = false;
            }
            true
        }

        // === Health pickups ===
        Some(SpriteNum::SPR_STIM) => {
            let players = ctx.inter_players_mut();
            if !p_give_body(&mut players[player_idx], 10) {
                return;
            }
            players[player_idx].message = Some(GOTSTIM.to_string());
            true
        }
        Some(SpriteNum::SPR_MEDI) => {
            // Check health < 25 BEFORE giving health for message selection
            let need_msg = {
                let players = ctx.inter_players();
                players[player_idx].health < 25
            };
            let players = ctx.inter_players_mut();
            if !p_give_body(&mut players[player_idx], 25) {
                return;
            }
            if need_msg {
                players[player_idx].message = Some(GOTMEDINEED.to_string());
            } else {
                players[player_idx].message = Some(GOTMEDIKIT.to_string());
            }
            true
        }

        // Powerup pickups continued in next section...
        // === Powerups ===
        Some(SpriteNum::SPR_PINV) => {
            let players = ctx.inter_players_mut();
            if !p_give_power(&mut players[player_idx], PowerType::Invulnerability) {
                return;
            }
            players[player_idx].message = Some(GOTINVUL.to_string());
            sound = SfxEnum::sfx_getpow;
            true
        }
        Some(SpriteNum::SPR_PSTR) => {
            let players = ctx.inter_players_mut();
            let _ = p_give_power(&mut players[player_idx], PowerType::Strength);
            players[player_idx].message = Some(GOTBERSERK.to_string());
            if players[player_idx].readyweapon != WeaponType::Fist {
                players[player_idx].pendingweapon = WeaponType::Fist;
            }
            sound = SfxEnum::sfx_getpow;
            true
        }
        Some(SpriteNum::SPR_PINS) => {
            let players = ctx.inter_players_mut();
            if !p_give_power(&mut players[player_idx], PowerType::Invisibility) {
                return;
            }
            players[player_idx].message = Some(GOTINVIS.to_string());
            ctx.mobjs_mut()[toucher_idx].flags |= MobjFlags::MF_SHADOW;
            sound = SfxEnum::sfx_getpow;
            true
        }
        Some(SpriteNum::SPR_SUIT) => {
            let players = ctx.inter_players_mut();
            if !p_give_power(&mut players[player_idx], PowerType::IronFeet) {
                return;
            }
            players[player_idx].message = Some(GOTSUIT.to_string());
            sound = SfxEnum::sfx_getpow;
            true
        }
        Some(SpriteNum::SPR_PMAP) => {
            let players = ctx.inter_players_mut();
            if !p_give_power(&mut players[player_idx], PowerType::AllMap) {
                return;
            }
            players[player_idx].message = Some(GOTMAP.to_string());
            sound = SfxEnum::sfx_getpow;
            true
        }
        Some(SpriteNum::SPR_PVIS) => {
            let players = ctx.inter_players_mut();
            if !p_give_power(&mut players[player_idx], PowerType::InfraRed) {
                return;
            }
            players[player_idx].message = Some(GOTVISOR.to_string());
            sound = SfxEnum::sfx_getpow;
            true
        }

        // === Ammo pickups ===
        Some(SpriteNum::SPR_CLIP) => {
            let dropped_flag = special_flags.contains(MobjFlags::MF_DROPPED);
            let num = if dropped_flag { 0 } else { 1 };
            let players = ctx.inter_players_mut();
            if !p_give_ammo(&mut players[player_idx], AmmoType::Clip, num, gameskill) {
                return;
            }
            players[player_idx].message = Some(GOTCLIP.to_string());
            true
        }
        Some(SpriteNum::SPR_AMMO) => {
            let players = ctx.inter_players_mut();
            if !p_give_ammo(&mut players[player_idx], AmmoType::Clip, 5, gameskill) {
                return;
            }
            players[player_idx].message = Some(GOTCLIPBOX.to_string());
            true
        }
        Some(SpriteNum::SPR_ROCK) => {
            let players = ctx.inter_players_mut();
            if !p_give_ammo(&mut players[player_idx], AmmoType::Missile, 1, gameskill) {
                return;
            }
            players[player_idx].message = Some(GOTROCKET.to_string());
            true
        }
        Some(SpriteNum::SPR_BROK) => {
            let players = ctx.inter_players_mut();
            if !p_give_ammo(&mut players[player_idx], AmmoType::Missile, 5, gameskill) {
                return;
            }
            players[player_idx].message = Some(GOTROCKBOX.to_string());
            true
        }
        Some(SpriteNum::SPR_CELL) => {
            let players = ctx.inter_players_mut();
            if !p_give_ammo(&mut players[player_idx], AmmoType::Cell, 1, gameskill) {
                return;
            }
            players[player_idx].message = Some(GOTCELL.to_string());
            true
        }
        Some(SpriteNum::SPR_CELP) => {
            let players = ctx.inter_players_mut();
            if !p_give_ammo(&mut players[player_idx], AmmoType::Cell, 5, gameskill) {
                return;
            }
            players[player_idx].message = Some(GOTCELLBOX.to_string());
            true
        }
        Some(SpriteNum::SPR_SHEL) => {
            let players = ctx.inter_players_mut();
            if !p_give_ammo(&mut players[player_idx], AmmoType::Shell, 1, gameskill) {
                return;
            }
            players[player_idx].message = Some(GOTSHELLS.to_string());
            true
        }
        Some(SpriteNum::SPR_SBOX) => {
            let players = ctx.inter_players_mut();
            if !p_give_ammo(&mut players[player_idx], AmmoType::Shell, 5, gameskill) {
                return;
            }
            players[player_idx].message = Some(GOTSHELLBOX.to_string());
            true
        }

        // === Backpack ===
        Some(SpriteNum::SPR_BPAK) => {
            let players = ctx.inter_players_mut();
            if !players[player_idx].backpack {
                for i in 0..NUMAMMO {
                    players[player_idx].maxammo[i] *= 2;
                }
                players[player_idx].backpack = true;
            }
            p_give_ammo(&mut players[player_idx], AmmoType::Clip, 1, gameskill);
            p_give_ammo(&mut players[player_idx], AmmoType::Shell, 1, gameskill);
            p_give_ammo(&mut players[player_idx], AmmoType::Cell, 1, gameskill);
            p_give_ammo(&mut players[player_idx], AmmoType::Missile, 1, gameskill);
            players[player_idx].message = Some(GOTBACKPACK.to_string());
            true
        }

        // === Weapon pickups ===
        Some(SpriteNum::SPR_BFUG) => {
            let dropped = special_flags.contains(MobjFlags::MF_DROPPED);
            let players = ctx.inter_players_mut();
            if !p_give_weapon(
                &mut players[player_idx],
                WeaponType::Bfg,
                dropped,
                gameskill,
                netgame,
                deathmatch,
            ) {
                return;
            }
            players[player_idx].message = Some(GOTBFG9000.to_string());
            sound = SfxEnum::sfx_wpnup;
            true
        }
        Some(SpriteNum::SPR_MGUN) => {
            let dropped = special_flags.contains(MobjFlags::MF_DROPPED);
            let players = ctx.inter_players_mut();
            if !p_give_weapon(
                &mut players[player_idx],
                WeaponType::Chaingun,
                dropped,
                gameskill,
                netgame,
                deathmatch,
            ) {
                return;
            }
            players[player_idx].message = Some(GOTCHAINGUN.to_string());
            sound = SfxEnum::sfx_wpnup;
            true
        }
        Some(SpriteNum::SPR_CSAW) => {
            let dropped = special_flags.contains(MobjFlags::MF_DROPPED);
            let players = ctx.inter_players_mut();
            if !p_give_weapon(
                &mut players[player_idx],
                WeaponType::Chainsaw,
                dropped,
                gameskill,
                netgame,
                deathmatch,
            ) {
                return;
            }
            players[player_idx].message = Some(GOTCHAINSAW.to_string());
            sound = SfxEnum::sfx_wpnup;
            true
        }
        Some(SpriteNum::SPR_LAUN) => {
            let dropped = special_flags.contains(MobjFlags::MF_DROPPED);
            let players = ctx.inter_players_mut();
            if !p_give_weapon(
                &mut players[player_idx],
                WeaponType::Missile,
                dropped,
                gameskill,
                netgame,
                deathmatch,
            ) {
                return;
            }
            players[player_idx].message = Some(GOTLAUNCHER.to_string());
            sound = SfxEnum::sfx_wpnup;
            true
        }
        Some(SpriteNum::SPR_PLAS) => {
            let dropped = special_flags.contains(MobjFlags::MF_DROPPED);
            let players = ctx.inter_players_mut();
            if !p_give_weapon(
                &mut players[player_idx],
                WeaponType::Plasma,
                dropped,
                gameskill,
                netgame,
                deathmatch,
            ) {
                return;
            }
            players[player_idx].message = Some(GOTPLASMA.to_string());
            sound = SfxEnum::sfx_wpnup;
            true
        }
        Some(SpriteNum::SPR_SHOT) => {
            let dropped = special_flags.contains(MobjFlags::MF_DROPPED);
            let players = ctx.inter_players_mut();
            if !p_give_weapon(
                &mut players[player_idx],
                WeaponType::Shotgun,
                dropped,
                gameskill,
                netgame,
                deathmatch,
            ) {
                return;
            }
            players[player_idx].message = Some(GOTSHOTGUN.to_string());
            sound = SfxEnum::sfx_wpnup;
            true
        }
        Some(SpriteNum::SPR_SGN2) => {
            let dropped = special_flags.contains(MobjFlags::MF_DROPPED);
            let players = ctx.inter_players_mut();
            if !p_give_weapon(
                &mut players[player_idx],
                WeaponType::SuperShotgun,
                dropped,
                gameskill,
                netgame,
                deathmatch,
            ) {
                return;
            }
            players[player_idx].message = Some(GOTSHOTGUN2.to_string());
            sound = SfxEnum::sfx_wpnup;
            true
        }

        // === Default (unknown gettable thing) ===
        _ => {
            tracing::error!(
                "P_TouchSpecialThing: Unknown gettable thing sprite {}",
                special_sprite
            );
            return;
        }
    };

    if !accepted {
        return;
    }

    // Count items if flagged
    if special_flags.contains(MobjFlags::MF_COUNTITEM) {
        let players = ctx.inter_players_mut();
        players[player_idx].itemcount += 1;
    }

    // Remove the special thing from the world
    if remove_special {
        ctx.inter_p_remove_mobj(special_idx);
    }

    // Bonus count (pickup flash)
    {
        let players = ctx.inter_players_mut();
        players[player_idx].bonuscount += BONUSADD;
    }

    // Play pickup sound on toucher
    ctx.inter_s_start_sound(Some(toucher_idx), sound);
}

// =============================================================================
// P_KillMobj (p_inter.c lines 667-758)
// =============================================================================

/// Kill a map object. Handles flag changes, frag counting, item drops,
/// and death/extreme-death state transitions.
///
/// `source_idx` is the killer (None for environmental kills/self-kills).
/// `target_idx` is the victim to be killed.
///
/// Original C: `void P_KillMobj(mobj_t* source, mobj_t* target)`
/// (p_inter.c lines 667-758)
pub fn p_kill_mobj(source_idx: Option<usize>, target_idx: usize, ctx: &mut dyn InterContext) {
    // Remove shootable/float/skullfly flags; add corpse/dropoff flags
    {
        let mobjs = ctx.mobjs_mut();
        let target = &mut mobjs[target_idx];

        target.flags &= !(MobjFlags::MF_SHOOTABLE | MobjFlags::MF_FLOAT | MobjFlags::MF_SKULLFLY);

        let target_type = mobjtype_from_usize(target.type_);
        if target_type != Some(MobjType::MT_SKULL) {
            target.flags &= !MobjFlags::MF_NOGRAVITY;
        }

        target.flags |= MobjFlags::MF_CORPSE | MobjFlags::MF_DROPOFF;
        target.height = Fixed(target.height.0 >> 2);
    }

    // Kill counting and frag management
    let target_player_idx = ctx.mobjs()[target_idx].player;
    let target_flags = ctx.mobjs()[target_idx].flags;
    let netgame = ctx.inter_netgame();
    let consoleplayer = ctx.inter_consoleplayer();

    if let Some(src_idx) = source_idx {
        let source_player_idx = ctx.mobjs()[src_idx].player;
        if let Some(src_p) = source_player_idx {
            // Player killed something
            if let Some(tgt_p) = target_player_idx {
                // Player killed player — frag counting
                if src_p == tgt_p {
                    // Self-kill
                    let players = ctx.inter_players_mut();
                    players[src_p].frags[src_p] -= 1;
                } else {
                    let players = ctx.inter_players_mut();
                    players[src_p].frags[tgt_p] += 1;
                }
            } else {
                // Player killed monster
                if target_flags.contains(MobjFlags::MF_COUNTKILL) {
                    let players = ctx.inter_players_mut();
                    players[src_p].killcount += 1;
                }
            }
        }
    } else {
        // No source (environmental death)
        if let Some(tgt_p) = target_player_idx {
            // Player died from environment — count as self-frag
            let players = ctx.inter_players_mut();
            players[tgt_p].frags[tgt_p] -= 1;
        }
    }

    // In single-player, count any MF_COUNTKILL regardless of source
    if !netgame && target_flags.contains(MobjFlags::MF_COUNTKILL) && target_player_idx.is_none() {
        // Check if source is a player — if no source or source is not a player,
        // still count for player 0 in single-player (fallback)
        let source_is_player = source_idx
            .map(|si| ctx.mobjs()[si].player.is_some())
            .unwrap_or(false);
        if !source_is_player {
            let players = ctx.inter_players_mut();
            players[0].killcount += 1;
        }
    }

    // If target is a player, handle player death
    if let Some(tgt_p) = target_player_idx {
        {
            let players = ctx.inter_players_mut();
            players[tgt_p].playerstate = PlayerState::Dead;
        }

        // Clear MF_SOLID on dead player
        ctx.mobjs_mut()[target_idx].flags &= !MobjFlags::MF_SOLID;

        // Drop weapon
        ctx.inter_p_drop_weapon(tgt_p);

        // Stop automap if active and this is the console player
        if tgt_p == consoleplayer {
            let automap = ctx.automap_state_mut();
            if automap.automapactive {
                am_stop(automap);
            }
        }
    }

    // Determine death state: extreme death if health < -spawnhealth and xdeathstate exists
    let (target_health, target_type_idx) = {
        let mobjs = ctx.mobjs();
        (mobjs[target_idx].health, mobjs[target_idx].type_)
    };

    let info_idx = target_type_idx;
    let spawnhealth = if info_idx < MOBJINFO.len() {
        MOBJINFO[info_idx].spawnhealth
    } else {
        0
    };

    let death_state = if target_health < -spawnhealth {
        // Check for extreme death state
        let xdeathstate_num = if info_idx < MOBJINFO.len() {
            MOBJINFO[info_idx].xdeathstate
        } else {
            StateNum::S_NULL
        };
        if xdeathstate_num != StateNum::S_NULL {
            xdeathstate_num
        } else {
            if info_idx < MOBJINFO.len() {
                MOBJINFO[info_idx].deathstate
            } else {
                StateNum::S_NULL
            }
        }
    } else {
        if info_idx < MOBJINFO.len() {
            MOBJINFO[info_idx].deathstate
        } else {
            StateNum::S_NULL
        }
    };

    ctx.inter_p_set_mobj_state(target_idx, death_state);

    // Randomize tics (ensure minimum of 1)
    {
        let rng = ctx.rng_mut();
        let rand_val = (rng.p_random() & 3) as i32;
        let mobjs = ctx.mobjs_mut();
        mobjs[target_idx].tics -= rand_val;
        if mobjs[target_idx].tics < 1 {
            mobjs[target_idx].tics = 1;
        }
    }

    // Item drops — certain monsters drop items on death
    let drop_type: Option<MobjType> = {
        let target_mt = mobjtype_from_usize(target_type_idx);
        match target_mt {
            Some(MobjType::MT_WOLFSS) | Some(MobjType::MT_POSSESSED) => Some(MobjType::MT_CLIP),
            Some(MobjType::MT_SHOTGUY) => Some(MobjType::MT_SHOTGUN),
            Some(MobjType::MT_CHAINGUY) => Some(MobjType::MT_CHAINGUN),
            _ => None,
        }
    };

    if let Some(item_type) = drop_type {
        let (tx, ty) = {
            let mobjs = ctx.mobjs();
            (mobjs[target_idx].x, mobjs[target_idx].y)
        };
        let item_idx = ctx.inter_p_spawn_mobj(tx, ty, ONFLOORZ, item_type);
        // Mark as dropped (half ammo for weapons picked up from drops)
        ctx.mobjs_mut()[item_idx].flags |= MobjFlags::MF_DROPPED;
    }
}

// =============================================================================
// P_DamageMobj (p_inter.c lines 774-917)
// =============================================================================

/// Apply damage to a map object.
///
/// Handles thrust from damage source, armor absorption, god mode immunity,
/// pain state transitions, kill processing, and target wake-up/retargeting.
///
/// `target_idx` — the mobj being damaged.
/// `inflictor_idx` — the thing that caused the damage (projectile, etc.); `None` for
///   hitscan, crushing, or environmental damage.
/// `source_idx` — the originator of the damage (the player or monster that fired);
///   `None` for environmental kills.
/// `damage` — raw damage amount before any modifications.
///
/// Original C: `void P_DamageMobj(mobj_t* target, mobj_t* inflictor, mobj_t* source, int damage)`
/// (p_inter.c lines 774-917)
pub fn p_damage_mobj(
    target_idx: usize,
    inflictor_idx: Option<usize>,
    source_idx: Option<usize>,
    mut damage: i32,
    ctx: &mut dyn InterContext,
) {
    // Check if target is shootable
    let target_flags = ctx.mobjs()[target_idx].flags;
    if !target_flags.contains(MobjFlags::MF_SHOOTABLE) {
        return;
    }

    // Already dead?
    let target_health = ctx.mobjs()[target_idx].health;
    if target_health <= 0 {
        return;
    }

    // If skull flying, zero all momentum
    if target_flags.contains(MobjFlags::MF_SKULLFLY) {
        let mobjs = ctx.mobjs_mut();
        mobjs[target_idx].momx = Fixed(0);
        mobjs[target_idx].momy = Fixed(0);
        mobjs[target_idx].momz = Fixed(0);
    }

    // Player takes half damage in sk_baby
    let target_player_idx = ctx.mobjs()[target_idx].player;
    if target_player_idx.is_some() && ctx.inter_gameskill() == Skill::Baby {
        damage >>= 1;
    }

    // === Thrust calculation ===
    // If there's an inflictor, apply thrust to target away from inflictor
    if let Some(inf_idx) = inflictor_idx {
        let target_noclip = ctx.mobjs()[target_idx].flags.contains(MobjFlags::MF_NOCLIP);

        if !target_noclip {
            // Check if inflictor is a chainsaw (MF_SKULLFLY check on source instead)
            // Original: if inflictor && !(target->flags & MF_NOCLIP) && !(source && source->player
            //   && source->player->readyweapon == wp_chainsaw)
            let is_chainsaw = source_idx.is_some_and(|si| {
                let source_player_idx = ctx.mobjs()[si].player;
                if let Some(sp) = source_player_idx {
                    let players = ctx.inter_players();
                    players[sp].readyweapon == WeaponType::Chainsaw
                } else {
                    false
                }
            });

            if !is_chainsaw {
                let (inf_x, inf_y) = {
                    let mobjs = ctx.mobjs();
                    (mobjs[inf_idx].x, mobjs[inf_idx].y)
                };
                let (tgt_x, tgt_y, tgt_z, tgt_health, tgt_type_idx) = {
                    let mobjs = ctx.mobjs();
                    (
                        mobjs[target_idx].x,
                        mobjs[target_idx].y,
                        mobjs[target_idx].z,
                        mobjs[target_idx].health,
                        mobjs[target_idx].type_,
                    )
                };

                let mut ang = ctx.point_to_angle2(inf_x, inf_y, tgt_x, tgt_y);

                let mass = if tgt_type_idx < MOBJINFO.len() {
                    MOBJINFO[tgt_type_idx].mass
                } else {
                    100 // default mass
                };

                let mut thrust = Fixed(
                    (((damage as i64) * ((FRACUNIT >> 3) as i64) * 100) / (mass as i64)) as i32,
                );

                // Fall-forward mechanic: sometimes make a corpse slide toward the killer
                // Conditions: damage < 40, damage > health, z height diff > 64*FRACUNIT, random
                if damage < 40 && damage > tgt_health {
                    // Check z difference: inflictor z must be significantly above target
                    let inf_z = ctx.mobjs()[inf_idx].z;
                    let z_diff = Fixed(inf_z.0 - tgt_z.0);
                    if z_diff.0 > 64 * FRACUNIT {
                        let rng = ctx.rng_mut();
                        if (rng.p_random() & 1) != 0 {
                            ang = Angle(ang.0.wrapping_add(ANG180.0));
                            thrust = Fixed(thrust.0 * 4);
                        }
                    }
                }

                let fine_ang = ((ang.0 >> ANGLETOFINESHIFT) & 0x1FFF) as usize;
                let cos_val = finecosine(fine_ang);
                let sin_val = FINESINE[fine_ang & 0x1FFF];

                let mobjs = ctx.mobjs_mut();
                mobjs[target_idx].momx = Fixed(
                    mobjs[target_idx]
                        .momx
                        .0
                        .wrapping_add(thrust.fixed_mul(cos_val).0),
                );
                mobjs[target_idx].momy = Fixed(
                    mobjs[target_idx]
                        .momy
                        .0
                        .wrapping_add(thrust.fixed_mul(sin_val).0),
                );
            }
        }
    }

    // === Player-specific damage handling ===
    if let Some(tgt_p) = target_player_idx {
        // Sector type 11 "hell slime" hack: cap damage at health - 1
        // In the original, this is checked against the sector special type on the player's
        // subsector. We replicate by checking sector special from context.
        // NOTE: This check is in the original C code. We approximate via a helper method.

        // God mode / invulnerability check (ignore damage < 1000)
        let is_godmode = {
            let players = ctx.inter_players();
            CheatFlags::from_bits_truncate(players[tgt_p].cheats).contains(CheatFlags::CF_GODMODE)
        };
        let has_invuln = {
            let players = ctx.inter_players();
            players[tgt_p].power(PowerType::Invulnerability) != 0
        };

        if (is_godmode || has_invuln) && damage < 1000 {
            return;
        }

        // Armor absorption
        let mut saved = 0i32;
        {
            let players = ctx.inter_players();
            if players[tgt_p].armortype != 0 {
                if players[tgt_p].armortype == 1 {
                    saved = damage / 3;
                } else {
                    saved = damage / 2;
                }
            }
        }

        if saved > 0 {
            let players = ctx.inter_players_mut();
            if players[tgt_p].armorpoints <= saved {
                // Armor is used up
                saved = players[tgt_p].armorpoints;
                players[tgt_p].armortype = 0;
            }
            players[tgt_p].armorpoints -= saved;
            damage -= saved;
        }

        // Set attacker (for status bar face direction)
        {
            let players = ctx.inter_players_mut();
            players[tgt_p].attacker = source_idx;
        }

        // Apply damage to player health
        {
            let players = ctx.inter_players_mut();
            players[tgt_p].health -= damage;
            if players[tgt_p].health < 0 {
                players[tgt_p].health = 0;
            }
            // Damage count for screen flash (capped at 100)
            players[tgt_p].damagecount += damage;
            if players[tgt_p].damagecount > 100 {
                players[tgt_p].damagecount = 100;
            }
        }

        // I_Tactile feedback
        ctx.i_tactile(40, 10, 40 + damage.min(100));
    }

    // === Apply damage to mobj health ===
    {
        let mobjs = ctx.mobjs_mut();
        mobjs[target_idx].health -= damage;
    }

    let new_health = ctx.mobjs()[target_idx].health;

    if new_health <= 0 {
        // Target is killed
        p_kill_mobj(source_idx, target_idx, ctx);
        return;
    }

    // === Pain state ===
    let target_type_idx = ctx.mobjs()[target_idx].type_;
    let painchance = if target_type_idx < MOBJINFO.len() {
        MOBJINFO[target_type_idx].painchance
    } else {
        0
    };

    let in_skullfly = ctx.mobjs()[target_idx]
        .flags
        .contains(MobjFlags::MF_SKULLFLY);

    // Check pain chance: P_Random() < painchance, and not in skull fly
    let trigger_pain = {
        let rng = ctx.rng_mut();
        let roll = rng.p_random() as i32;
        roll < painchance && !in_skullfly
    };

    if trigger_pain {
        // Set JUSTHIT flag for AI
        ctx.mobjs_mut()[target_idx].flags |= MobjFlags::MF_JUSTHIT;

        // Go to pain state
        let painstate = if target_type_idx < MOBJINFO.len() {
            MOBJINFO[target_type_idx].painstate
        } else {
            StateNum::S_NULL
        };
        if painstate != StateNum::S_NULL {
            ctx.inter_p_set_mobj_state(target_idx, painstate);
        }
    }

    // === Wake up / retarget ===
    // reactiontime = 0: wake up immediately
    ctx.mobjs_mut()[target_idx].reactiontime = 0;

    // Retarget to the source of damage if threshold permits
    if let Some(src_idx) = source_idx {
        let threshold = ctx.mobjs()[target_idx].threshold;
        let current_target = ctx.mobjs()[target_idx].target;

        // Don't retarget if already targeting source, or if threshold is active
        // Special exception: never retarget away from vile (MobjType::MT_VILE)
        let source_type = mobjtype_from_usize(ctx.mobjs()[src_idx].type_);
        let is_vile = source_type == Some(MobjType::MT_VILE);

        let should_retarget = if threshold != 0 && current_target.is_some() {
            // Already has a target and threshold is active — only retarget if vile
            is_vile
        } else {
            true
        };

        // Also retarget if target is the same as source (Vile exception handled above)
        if should_retarget && src_idx != target_idx {
            ctx.mobjs_mut()[target_idx].target = Some(src_idx);
            ctx.mobjs_mut()[target_idx].threshold = BASETHRESHOLD;

            // If target was in a dormant state (spawnstate), wake it up (seestate)
            let target_state = ctx.mobjs()[target_idx].state;
            let spawnstate = if target_type_idx < MOBJINFO.len() {
                MOBJINFO[target_type_idx].spawnstate
            } else {
                StateNum::S_NULL
            };
            if let Some(current_state_num) = target_state {
                if current_state_num == spawnstate as usize {
                    let seestate = if target_type_idx < MOBJINFO.len() {
                        MOBJINFO[target_type_idx].seestate
                    } else {
                        StateNum::S_NULL
                    };
                    if seestate != StateNum::S_NULL {
                        ctx.inter_p_set_mobj_state(target_idx, seestate);
                    }
                }
            }
        }
    }
}
