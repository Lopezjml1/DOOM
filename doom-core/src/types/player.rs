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

//! Translated from linuxdoom-1.10/d_player.h (lines 1-220).
//!
//! Defines the complete player state structure used throughout the DOOM engine.
//! Also incorporates `pspdef_t` and related constants from `p_pspr.h` (lines
//! 46-75), and references weapon/ammo/power enums from `doomdef.h`.
//!
//! # Player State Architecture
//!
//! The [`Player`] struct is the central data structure for each player in the
//! game. It holds everything the engine needs to simulate a player's presence
//! in the world: movement state, health, armor, inventory, weapon state,
//! power-up counters, intermission statistics, and the weapon overlay sprites
//! drawn on top of the first-person view.
//!
//! In the original C code, the player's map object (`mobj_t*`) was stored as a
//! raw pointer. In this Rust port, we use `Option<usize>` (an arena index) to
//! avoid `unsafe` pointer dereferences while preserving the ability to
//! reference game objects.
//!
//! # Intermission Structures
//!
//! [`WbPlayerStruct`] and [`WbStartStruct`] carry statistics between levels
//! for the intermission screen (WI_Stuff). These are populated at level end
//! and consumed by the intermission drawer to display kill percentages, item
//! counts, secret counts, and par times.
//!
//! # Behavioral Parity
//!
//! All enum discriminant values, struct field ordering, and array sizes match
//! the original C definitions exactly. This is critical for save/load
//! compatibility and deterministic gameplay behavior.

use super::doomdef::{
    AmmoType, Card, PowerType, WeaponType, MAXPLAYERS, NUMAMMO, NUMCARDS, NUMPOWERS, NUMWEAPONS,
};
use super::fixed::Fixed;
use super::ticcmd::TicCmd;

// =============================================================================
// Constants from p_pspr.h
// =============================================================================

/// Number of player sprite overlays (weapon + muzzle flash).
///
/// Translated from C `NUMPSPRITES` in `p_pspr.h` line 64.
/// The original C enum counted: `ps_weapon = 0`, `ps_flash = 1`,
/// `NUMPSPRITES = 2`.
pub const NUMPSPRITES: usize = 2;

/// Frame flag: render at full brightness regardless of sector light level.
///
/// When this bit is set in a sprite frame number, the sprite is drawn at
/// maximum brightness (used for muzzle flashes, fireballs, etc.).
///
/// Translated from C `#define FF_FULLBRIGHT 0x8000` in `p_pspr.h` line 50.
pub const FF_FULLBRIGHT: u32 = 0x8000;

/// Frame flag mask: extract the actual frame index by masking off the
/// full-bright flag bit.
///
/// Translated from C `#define FF_FRAMEMASK 0x7fff` in `p_pspr.h` line 51.
pub const FF_FRAMEMASK: u32 = 0x7fff;

// =============================================================================
// PsprNum — Player sprite position enum (from p_pspr.h lines 60-66)
// =============================================================================

/// Identifies which player sprite overlay slot is being referenced.
///
/// DOOM draws two overlay sprites on the first-person view: the weapon sprite
/// and the muzzle flash sprite. These are indexed by `PsprNum` into the
/// `Player.psprites` array.
///
/// Translated from C `psprnum_t` in `p_pspr.h` lines 60-66.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum PsprNum {
    /// The main weapon sprite overlay (index 0).
    Weapon = 0,
    /// The muzzle flash overlay drawn on top of the weapon (index 1).
    Flash = 1,
}

// =============================================================================
// PspDef — Player sprite definition (from p_pspr.h lines 68-75)
// =============================================================================

/// A single player sprite overlay definition (weapon or flash).
///
/// Each player has [`NUMPSPRITES`] (2) of these: one for the weapon graphic
/// and one for the muzzle flash. The `state` field indexes into the global
/// state table; when `None`, the sprite is not active (equivalent to the
/// original C `NULL` state pointer).
///
/// Translated from C `pspdef_t` in `p_pspr.h` lines 68-75.
#[derive(Debug, Clone, Copy)]
pub struct PspDef {
    /// Index into the global state table, or `None` if inactive.
    ///
    /// Original C: `state_t* state;` — `NULL` means not active.
    pub state: Option<usize>,

    /// Remaining tics in the current animation frame.
    ///
    /// Decremented each game tic; when it reaches zero, the state machine
    /// advances to the next state.
    pub tics: i32,

    /// Horizontal position offset for the sprite on screen (fixed-point).
    ///
    /// Origin is the center of the 320×200 view. Used for weapon bobbing
    /// and positioning.
    pub sx: Fixed,

    /// Vertical position offset for the sprite on screen (fixed-point).
    ///
    /// Origin is the top of the 320×200 view. Used for weapon raise/lower
    /// animations and bobbing.
    pub sy: Fixed,
}

impl Default for PspDef {
    /// Returns an inactive player sprite definition with all fields zeroed.
    ///
    /// State is `None` (not active), tics is zero, and position offsets
    /// are at the origin.
    fn default() -> Self {
        PspDef {
            state: None,
            tics: 0,
            sx: Fixed::default(),
            sy: Fixed::default(),
        }
    }
}

// =============================================================================
// PlayerState — Player state enum (from d_player.h lines 53-62)
// =============================================================================

/// The current life-state of a player.
///
/// Controls how the engine processes the player each tic: live players
/// receive input and physics; dead players have their view follow the
/// killer; reborn players are waiting to respawn.
///
/// Translated from C `playerstate_t` in `d_player.h` lines 53-62.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PlayerState {
    /// Playing or camping — the player is alive and receiving input.
    Live = 0,
    /// Dead on the ground — view follows the killer.
    Dead = 1,
    /// Ready to restart / respawn.
    Reborn = 2,
}

impl Default for PlayerState {
    /// Returns [`PlayerState::Live`], matching the C zero-initialization
    /// behavior where `PST_LIVE = 0`.
    fn default() -> Self {
        PlayerState::Live
    }
}

// =============================================================================
// CheatFlags — Player cheat/debug flags (from d_player.h lines 68-77)
// =============================================================================

bitflags::bitflags! {
    /// Bitfield flags for player cheat codes and debug aids.
    ///
    /// These flags are stored in the `Player.cheats` field (as `i32`) and
    /// toggled by cheat code entry. The engine checks them each tic to
    /// bypass normal gameplay rules.
    ///
    /// Translated from C `cheat_t` in `d_player.h` lines 68-77.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CheatFlags: i32 {
        /// No clipping: walk through walls and barriers.
        /// Original C: `CF_NOCLIP = 1`
        const CF_NOCLIP = 1;

        /// God mode: no damage, no health loss.
        /// Original C: `CF_GODMODE = 2`
        const CF_GODMODE = 2;

        /// No momentum: debug aid, player stops instantly.
        /// Original C: `CF_NOMOMENTUM = 4`
        const CF_NOMOMENTUM = 4;
    }
}

// =============================================================================
// Player — Extended player object info (from d_player.h lines 83-166)
// =============================================================================

/// Complete player state structure.
///
/// This is the central data structure for each player. It contains all the
/// state needed to simulate a player's presence in the game world, including:
///
/// - Map object reference (position, physics, collision)
/// - Life state (alive, dead, respawning)
/// - Input command buffer
/// - View parameters (viewz, viewheight, bob)
/// - Health and armor
/// - Power-up counters
/// - Key cards held
/// - Weapon inventory and ammunition
/// - Cheat/debug flags
/// - Intermission statistics (kills, items, secrets)
/// - Screen flash counters (damage red, bonus bright)
/// - Lighting overrides
/// - Weapon overlay sprite definitions
///
/// # Pointer Replacement Strategy
///
/// The original C struct uses raw `mobj_t*` pointers for `mo` (the player's
/// map object) and `attacker` (the entity that last damaged the player).
/// In this Rust port, both are replaced with `Option<usize>` arena indices,
/// eliminating unsafe pointer dereferences while preserving the ability to
/// reference game objects.
///
/// Translated from C `player_t` in `d_player.h` lines 83-166.
#[derive(Debug, Clone)]
pub struct Player {
    /// Reference to the player's map object (arena index).
    ///
    /// Original C: `mobj_t* mo;`
    /// `None` when the player has no map object (e.g., between levels).
    pub mobj: Option<usize>,

    /// Current life-state of the player.
    pub playerstate: PlayerState,

    /// Input command for the current tic.
    ///
    /// Buffered within the player struct; filled by the input sampling code
    /// (or received from a network peer in multiplayer).
    pub cmd: TicCmd,

    // -- Point of View (POV) --
    /// Focal origin height above the floor (fixed-point).
    ///
    /// This is the Z coordinate of the player's viewpoint, computed from
    /// the floor height plus `viewheight` plus any view bobbing.
    pub viewz: Fixed,

    /// Base height above floor for the viewpoint (fixed-point).
    ///
    /// Normally 41 in fixed-point units. When the player is hit, this
    /// drops to simulate the "head snap" effect.
    pub viewheight: Fixed,

    /// Bob/squat speed for viewpoint oscillation (fixed-point).
    ///
    /// Drives the smooth transition of `viewheight` back to its normal
    /// value after damage or landing from a fall.
    pub deltaviewheight: Fixed,

    /// Bounded/scaled total momentum for view bobbing (fixed-point).
    ///
    /// Computed from the player's XY velocity. Used to create the
    /// characteristic "bobbing" effect while walking.
    pub bob: Fixed,

    // -- Health and Armor (between-levels state) --
    /// Player health percentage (0-200).
    ///
    /// This is the between-levels copy. During gameplay, `mo->health` is
    /// the authoritative health value; this field is synchronized at level
    /// transitions and used by the intermission screen.
    pub health: i32,

    /// Current armor points (0-200).
    pub armorpoints: i32,

    /// Armor type: 0 = none, 1 = green armor (1/3 absorb), 2 = blue armor
    /// (1/2 absorb).
    pub armortype: i32,

    // -- Power-ups --
    /// Power-up tic counters. Invulnerability and invisibility are countdown
    /// timers; strength (berserk) is a boolean flag; others are countdown
    /// timers that disable the effect when they reach zero.
    ///
    /// Indexed by `PowerType` ordinal value.
    pub powers: [i32; NUMPOWERS],

    /// Key cards and skull keys held by the player.
    ///
    /// Indexed by `Card` ordinal value. `true` = player has the key.
    pub cards: [bool; NUMCARDS],

    /// Whether the player has picked up a backpack (doubles max ammo).
    pub backpack: bool,

    // -- Frags --
    /// Kill count of each other player (for deathmatch scoring).
    ///
    /// `frags[i]` = number of times this player has killed player `i`.
    /// Self-kills (suicides) decrement `frags[own_index]`.
    pub frags: [i32; MAXPLAYERS],

    /// Currently selected and ready weapon.
    pub readyweapon: WeaponType,

    /// Weapon the player is switching to, or `WeaponType::NoChange` if not
    /// currently switching weapons.
    pub pendingweapon: WeaponType,

    /// Which weapons the player has picked up.
    ///
    /// Indexed by `WeaponType` ordinal value (0..NUMWEAPONS).
    /// `true` = player owns the weapon.
    pub weaponowned: [bool; NUMWEAPONS],

    /// Current ammunition counts.
    ///
    /// Indexed by `AmmoType` ordinal value (0..NUMAMMO).
    pub ammo: [i32; NUMAMMO],

    /// Maximum ammunition capacities (doubled when backpack is picked up).
    ///
    /// Indexed by `AmmoType` ordinal value (0..NUMAMMO).
    pub maxammo: [i32; NUMAMMO],

    // -- Input state --
    /// Non-zero if the attack (fire) button was held down last tic.
    ///
    /// Used to prevent auto-repeat on some weapons and to detect button
    /// release for weapon re-fire logic.
    pub attackdown: i32,

    /// Non-zero if the use (open/activate) button was held down last tic.
    ///
    /// Prevents repeated door activations from a single button hold.
    pub usedown: i32,

    /// Bit flags for active cheats and debug aids.
    ///
    /// Although typed as `i32` for C compatibility, the valid flags are
    /// defined in [`CheatFlags`]: `CF_NOCLIP`, `CF_GODMODE`, `CF_NOMOMENTUM`.
    pub cheats: i32,

    /// Refire counter: refired shots are less accurate.
    ///
    /// Incremented on each consecutive shot without releasing the fire
    /// button. Resets to zero when the button is released. Affects the
    /// spread angle of hitscan weapons.
    pub refire: i32,

    // -- Intermission statistics --
    /// Total monsters killed by this player in the current level.
    pub killcount: i32,

    /// Total items picked up by this player in the current level.
    pub itemcount: i32,

    /// Total secrets discovered by this player in the current level.
    pub secretcount: i32,

    /// Hint message string to display on the HUD, or `None` if no message.
    ///
    /// Original C: `char* message;` — `NULL` means no message.
    /// Set when the player picks up items, activates switches, etc.
    pub message: Option<String>,

    // -- Screen flash effects --
    /// Damage flash counter (red screen tint).
    ///
    /// Set when the player takes damage; decremented each tic. The screen
    /// is tinted red proportional to this value.
    pub damagecount: i32,

    /// Bonus flash counter (bright/gold screen tint).
    ///
    /// Set when the player picks up items; decremented each tic. The screen
    /// is tinted bright/gold proportional to this value.
    pub bonuscount: i32,

    /// Reference to the entity that last damaged this player (arena index).
    ///
    /// Original C: `mobj_t* attacker;`
    /// `None` for environmental damage (floors, ceilings, crushers).
    pub attacker: Option<usize>,

    // -- Lighting --
    /// Extra light level added to the player's view.
    ///
    /// Set by weapon muzzle flashes (gun flashes light up areas).
    /// Added to the base sector light level during rendering.
    pub extralight: i32,

    /// Fixed colormap index override.
    ///
    /// When non-zero, forces a specific colormap for the entire view.
    /// Used for pain (red) and pickup (gold) screen tints, and for the
    /// invulnerability grayscale effect.
    pub fixedcolormap: i32,

    /// Player skin color shift (0-3).
    ///
    /// Selects which color translation table to use when drawing the
    /// player's sprite in multiplayer. Each value maps to a different
    /// color palette shift (green, indigo, brown, red).
    pub colormap: i32,

    // -- Weapon overlay sprites --
    /// Player sprite overlay definitions (weapon + flash).
    ///
    /// `psprites[PsprNum::Weapon]` is the main weapon graphic.
    /// `psprites[PsprNum::Flash]` is the muzzle flash overlay.
    pub psprites: [PspDef; NUMPSPRITES],

    /// Whether the player has completed the secret level in this episode.
    ///
    /// Used to determine whether to show "secret level completed" on the
    /// intermission screen's world map.
    pub didsecret: bool,
}

impl Default for Player {
    /// Returns a zero-initialized player state matching C zero-initialization.
    ///
    /// All numeric fields are zero, all booleans are false, all option fields
    /// are `None`, weapon fields default to `Fist` (ordinal 0), and the
    /// player starts in [`PlayerState::Live`].
    fn default() -> Self {
        Player {
            mobj: None,
            playerstate: PlayerState::default(),
            cmd: TicCmd::default(),
            viewz: Fixed::default(),
            viewheight: Fixed::default(),
            deltaviewheight: Fixed::default(),
            bob: Fixed::default(),
            health: 0,
            armorpoints: 0,
            armortype: 0,
            powers: [0; NUMPOWERS],
            cards: [false; NUMCARDS],
            backpack: false,
            frags: [0; MAXPLAYERS],
            readyweapon: WeaponType::Fist,
            pendingweapon: WeaponType::Fist,
            weaponowned: [false; NUMWEAPONS],
            ammo: [0; NUMAMMO],
            maxammo: [0; NUMAMMO],
            attackdown: 0,
            usedown: 0,
            cheats: 0,
            refire: 0,
            killcount: 0,
            itemcount: 0,
            secretcount: 0,
            message: None,
            damagecount: 0,
            bonuscount: 0,
            attacker: None,
            extralight: 0,
            fixedcolormap: 0,
            colormap: 0,
            psprites: [PspDef::default(); NUMPSPRITES],
            didsecret: false,
        }
    }
}

impl Player {
    /// Returns the remaining tics for the given power-up type.
    ///
    /// Provides type-safe indexing into the `powers` array using the
    /// [`PowerType`] enum instead of raw integer indices.
    ///
    /// # Example
    /// ```ignore
    /// if player.power(PowerType::Invulnerability) > 0 {
    ///     // player is invulnerable
    /// }
    /// ```
    #[inline]
    pub fn power(&self, power: PowerType) -> i32 {
        self.powers[power as usize]
    }

    /// Returns a mutable reference to the remaining tics for a power-up type.
    #[inline]
    pub fn power_mut(&mut self, power: PowerType) -> &mut i32 {
        &mut self.powers[power as usize]
    }

    /// Returns whether the player has the specified key card or skull key.
    ///
    /// Provides type-safe indexing into the `cards` array using the
    /// [`Card`] enum.
    #[inline]
    pub fn has_card(&self, card: Card) -> bool {
        self.cards[card as usize]
    }

    /// Returns the current ammunition count for the given ammo type.
    ///
    /// Provides type-safe indexing into the `ammo` array using the
    /// [`AmmoType`] enum.
    #[inline]
    pub fn ammo_count(&self, ammo: AmmoType) -> i32 {
        self.ammo[ammo as usize]
    }

    /// Returns the maximum ammunition capacity for the given ammo type.
    #[inline]
    pub fn max_ammo(&self, ammo: AmmoType) -> i32 {
        self.maxammo[ammo as usize]
    }
}

// =============================================================================
// WbPlayerStruct — Intermission per-player data (from d_player.h lines 173-185)
// =============================================================================

/// Per-player statistics passed to the intermission screen.
///
/// Populated at the end of each level with the player's kill, item, and
/// secret counts, and consumed by `WI_Start` / `WI_Drawer` to display
/// the intermission stats.
///
/// Translated from C `wbplayerstruct_t` in `d_player.h` lines 173-185.
///
/// # Field Naming
///
/// The C field `in` is renamed to `in_game` because `in` is a reserved
/// keyword in Rust.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WbPlayerStruct {
    /// Whether this player slot is active in the current game.
    ///
    /// Original C: `boolean in;` — renamed from `in` (Rust reserved keyword).
    pub in_game: bool,

    /// Total monsters killed by this player.
    pub skills: i32,

    /// Total items collected by this player.
    pub sitems: i32,

    /// Total secrets found by this player.
    pub ssecret: i32,

    /// Time spent on the level (in tics).
    pub stime: i32,

    /// Frag counts against each player (for deathmatch).
    ///
    /// Fixed-size array of 4 entries matching the C definition:
    /// `int frags[4];`
    pub frags: [i32; 4],

    /// Current cumulative score on entry, modified on return.
    pub score: i32,
}

// =============================================================================
// WbStartStruct — Intermission session data (from d_player.h lines 187-211)
// =============================================================================

/// Level transition data passed to the intermission screen.
///
/// Contains both the global level-transition metadata (episode, previous/next
/// level, par time, kill/item/secret maximums) and per-player statistics for
/// all [`MAXPLAYERS`] player slots.
///
/// Translated from C `wbstartstruct_t` in `d_player.h` lines 187-211.
#[derive(Debug, Clone)]
pub struct WbStartStruct {
    /// Episode number (0-based: 0 = Episode 1, 1 = Episode 2, 2 = Episode 3).
    pub epsd: i32,

    /// Whether the secret level was completed in this episode.
    ///
    /// When true, the intermission screen shows a "splash" for the secret
    /// level on the world map.
    pub didsecret: bool,

    /// Previous level number (0-based origin).
    pub last: i32,

    /// Next level number (0-based origin).
    pub next: i32,

    /// Maximum possible monster kills on the completed level.
    pub maxkills: i32,

    /// Maximum possible items on the completed level.
    pub maxitems: i32,

    /// Maximum possible secrets on the completed level.
    pub maxsecret: i32,

    /// Maximum frags for deathmatch intermission display.
    pub maxfrags: i32,

    /// Par time for the completed level (in tics).
    pub partime: i32,

    /// Index of the local (viewing) player in the `plyr` array.
    pub pnum: i32,

    /// Per-player statistics for all player slots.
    pub plyr: [WbPlayerStruct; MAXPLAYERS],
}

impl Default for WbStartStruct {
    /// Returns a zero-initialized intermission struct.
    fn default() -> Self {
        WbStartStruct {
            epsd: 0,
            didsecret: false,
            last: 0,
            next: 0,
            maxkills: 0,
            maxitems: 0,
            maxsecret: 0,
            maxfrags: 0,
            partime: 0,
            pnum: 0,
            plyr: [WbPlayerStruct::default(); MAXPLAYERS],
        }
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numpsprites_value() {
        assert_eq!(NUMPSPRITES, 2);
    }

    #[test]
    fn test_ff_constants() {
        assert_eq!(FF_FULLBRIGHT, 0x8000);
        assert_eq!(FF_FRAMEMASK, 0x7fff);
        // Full-bright and frame mask should be complementary in the lower 16 bits
        assert_eq!(FF_FULLBRIGHT | FF_FRAMEMASK, 0xffff);
        assert_eq!(FF_FULLBRIGHT & FF_FRAMEMASK, 0);
    }

    #[test]
    fn test_psprnum_values() {
        assert_eq!(PsprNum::Weapon as usize, 0);
        assert_eq!(PsprNum::Flash as usize, 1);
    }

    #[test]
    fn test_pspdef_default() {
        let psp = PspDef::default();
        assert_eq!(psp.state, None);
        assert_eq!(psp.tics, 0);
        assert_eq!(psp.sx, Fixed::default());
        assert_eq!(psp.sy, Fixed::default());
    }

    #[test]
    fn test_playerstate_values() {
        assert_eq!(PlayerState::Live as i32, 0);
        assert_eq!(PlayerState::Dead as i32, 1);
        assert_eq!(PlayerState::Reborn as i32, 2);
    }

    #[test]
    fn test_playerstate_default() {
        assert_eq!(PlayerState::default(), PlayerState::Live);
    }

    #[test]
    fn test_cheatflags_values() {
        assert_eq!(CheatFlags::CF_NOCLIP.bits(), 1);
        assert_eq!(CheatFlags::CF_GODMODE.bits(), 2);
        assert_eq!(CheatFlags::CF_NOMOMENTUM.bits(), 4);
    }

    #[test]
    fn test_cheatflags_combinations() {
        let combined = CheatFlags::CF_NOCLIP | CheatFlags::CF_GODMODE;
        assert!(combined.contains(CheatFlags::CF_NOCLIP));
        assert!(combined.contains(CheatFlags::CF_GODMODE));
        assert!(!combined.contains(CheatFlags::CF_NOMOMENTUM));
        assert_eq!(combined.bits(), 3);
    }

    #[test]
    fn test_cheatflags_empty() {
        let empty = CheatFlags::empty();
        assert_eq!(empty.bits(), 0);
        assert!(!empty.contains(CheatFlags::CF_NOCLIP));
    }

    #[test]
    fn test_player_default() {
        let p = Player::default();
        assert_eq!(p.mobj, None);
        assert_eq!(p.playerstate, PlayerState::Live);
        assert_eq!(p.health, 0);
        assert_eq!(p.armorpoints, 0);
        assert_eq!(p.armortype, 0);
        assert_eq!(p.readyweapon, WeaponType::Fist);
        assert_eq!(p.pendingweapon, WeaponType::Fist);
        assert_eq!(p.backpack, false);
        assert_eq!(p.attackdown, 0);
        assert_eq!(p.usedown, 0);
        assert_eq!(p.cheats, 0);
        assert_eq!(p.refire, 0);
        assert_eq!(p.killcount, 0);
        assert_eq!(p.itemcount, 0);
        assert_eq!(p.secretcount, 0);
        assert_eq!(p.message, None);
        assert_eq!(p.damagecount, 0);
        assert_eq!(p.bonuscount, 0);
        assert_eq!(p.attacker, None);
        assert_eq!(p.extralight, 0);
        assert_eq!(p.fixedcolormap, 0);
        assert_eq!(p.colormap, 0);
        assert_eq!(p.didsecret, false);
    }

    #[test]
    fn test_player_array_sizes() {
        let p = Player::default();
        assert_eq!(p.powers.len(), NUMPOWERS);
        assert_eq!(p.cards.len(), NUMCARDS);
        assert_eq!(p.frags.len(), MAXPLAYERS);
        assert_eq!(p.weaponowned.len(), NUMWEAPONS);
        assert_eq!(p.ammo.len(), NUMAMMO);
        assert_eq!(p.maxammo.len(), NUMAMMO);
        assert_eq!(p.psprites.len(), NUMPSPRITES);
    }

    #[test]
    fn test_player_powers_zeroed() {
        let p = Player::default();
        for &pw in &p.powers {
            assert_eq!(pw, 0);
        }
    }

    #[test]
    fn test_player_cards_false() {
        let p = Player::default();
        for &card in &p.cards {
            assert!(!card);
        }
    }

    #[test]
    fn test_player_frags_zeroed() {
        let p = Player::default();
        for &f in &p.frags {
            assert_eq!(f, 0);
        }
    }

    #[test]
    fn test_player_weapons_not_owned() {
        let p = Player::default();
        for &w in &p.weaponowned {
            assert!(!w);
        }
    }

    #[test]
    fn test_player_ammo_zeroed() {
        let p = Player::default();
        for &a in &p.ammo {
            assert_eq!(a, 0);
        }
        for &ma in &p.maxammo {
            assert_eq!(ma, 0);
        }
    }

    #[test]
    fn test_player_psprites_inactive() {
        let p = Player::default();
        for psp in &p.psprites {
            assert_eq!(psp.state, None);
            assert_eq!(psp.tics, 0);
        }
    }

    #[test]
    fn test_player_clone() {
        let mut p = Player::default();
        p.health = 100;
        p.armorpoints = 50;
        p.readyweapon = WeaponType::Shotgun;
        p.cards[0] = true;
        p.message = Some("Picked up a shotgun.".to_string());

        let p2 = p.clone();
        assert_eq!(p2.health, 100);
        assert_eq!(p2.armorpoints, 50);
        assert_eq!(p2.readyweapon, WeaponType::Shotgun);
        assert!(p2.cards[0]);
        assert_eq!(p2.message, Some("Picked up a shotgun.".to_string()));
    }

    #[test]
    fn test_wb_player_struct_default() {
        let wp = WbPlayerStruct::default();
        assert!(!wp.in_game);
        assert_eq!(wp.skills, 0);
        assert_eq!(wp.sitems, 0);
        assert_eq!(wp.ssecret, 0);
        assert_eq!(wp.stime, 0);
        assert_eq!(wp.frags, [0; 4]);
        assert_eq!(wp.score, 0);
    }

    #[test]
    fn test_wb_start_struct_default() {
        let ws = WbStartStruct::default();
        assert_eq!(ws.epsd, 0);
        assert!(!ws.didsecret);
        assert_eq!(ws.last, 0);
        assert_eq!(ws.next, 0);
        assert_eq!(ws.maxkills, 0);
        assert_eq!(ws.maxitems, 0);
        assert_eq!(ws.maxsecret, 0);
        assert_eq!(ws.maxfrags, 0);
        assert_eq!(ws.partime, 0);
        assert_eq!(ws.pnum, 0);
        assert_eq!(ws.plyr.len(), MAXPLAYERS);
        for plyr in &ws.plyr {
            assert!(!plyr.in_game);
        }
    }

    #[test]
    fn test_wb_start_struct_plyr_size() {
        let ws = WbStartStruct::default();
        assert_eq!(ws.plyr.len(), 4); // MAXPLAYERS = 4
    }

    #[test]
    fn test_cheatflags_from_i32() {
        // Verify we can round-trip through i32 (matching the Player.cheats field type)
        let flags = CheatFlags::CF_NOCLIP | CheatFlags::CF_GODMODE;
        let raw: i32 = flags.bits();
        let restored = CheatFlags::from_bits_truncate(raw);
        assert_eq!(flags, restored);
    }

    #[test]
    fn test_playerstate_equality() {
        assert_ne!(PlayerState::Live, PlayerState::Dead);
        assert_ne!(PlayerState::Dead, PlayerState::Reborn);
        assert_ne!(PlayerState::Live, PlayerState::Reborn);
    }

    #[test]
    fn test_psprnum_can_index_psprites() {
        let p = Player::default();
        // Verify PsprNum values can be used to index the psprites array
        let _weapon = &p.psprites[PsprNum::Weapon as usize];
        let _flash = &p.psprites[PsprNum::Flash as usize];
    }
}
