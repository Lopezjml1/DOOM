// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Door open/close thinkers — vertical doors, locked doors, timed doors.
//!
//! Translated from linuxdoom-1.10/p_doors.c
//!
//! # Original C functions → Rust mapping
//!
//! | C function               | Rust function                  | Description                        |
//! |--------------------------|--------------------------------|------------------------------------|
//! | `T_VerticalDoor`         | [`t_vertical_door`]            | Door movement thinker callback     |
//! | `EV_DoLockedDoor`        | [`ev_do_locked_door`]          | Check keys and open locked door    |
//! | `EV_DoDoor`              | [`ev_do_door`]                 | Spawn door movers by line tag      |
//! | `EV_VerticalDoor`        | [`ev_vertical_door`]           | Manual door activation by player   |
//! | `P_SpawnDoorCloseIn30`   | [`p_spawn_door_close_in30`]    | Spawn auto-close-in-30s door       |
//! | `P_SpawnDoorRaiseIn5Mins`| [`p_spawn_door_raise_in5_mins`]| Spawn delayed-open-in-5min door    |
//!
//! # Architecture
//!
//! Door thinkers use the [`VldoorT`] struct from `spec.rs`, which stores the
//! door's sector index, movement direction, speed, target height, and wait
//! countdown. The thinker function [`t_vertical_door`] implements a four-state
//! machine:
//!
//! - **direction = 0**: Waiting (decrementing `topcountdown`)
//! - **direction = 1**: Opening (raising ceiling toward `topheight`)
//! - **direction = -1**: Closing (lowering ceiling toward floor)
//! - **direction = 2**: Initial wait (for `RaiseIn5Mins` type)
//!
//! The shared [`t_move_plane`] function from `floor.rs` handles the actual
//! ceiling movement, while this module manages the state transitions, sound
//! effects, and re-use (bump) logic.

use crate::game::strings::{PD_BLUEK, PD_BLUEO, PD_REDK, PD_REDO, PD_YELLOWK, PD_YELLOWO};
use crate::info::sounds::SfxEnum;
use crate::play::floor::t_move_plane;
use crate::play::spec::{
    p_find_lowest_ceiling_surrounding, p_find_sector_from_line_tag, ResultE, SpecContext, VldoorT,
    VldoorType, VDOORSPEED, VDOORWAIT,
};
use crate::types::doomdef::Card;
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::Sector;
use crate::types::thinker::{ActionFn, Thinker};

// ============================================================================
// T_VerticalDoor — Door movement thinker
// Translated from linuxdoom-1.10/p_doors.c lines 42-178
// ============================================================================

/// Door movement thinker — called once per tic for each active door.
///
/// Implements the four-direction state machine that drives all vertical door
/// types. Movement is performed by [`t_move_plane`] (ceiling plane), and state
/// transitions occur when the plane reaches its destination or a timer expires.
///
/// Returns `true` if the door thinker has reached its final state and should be
/// removed from the thinker list (and `sector.specialdata` cleared). Returns
/// `false` if the door is still active.
///
/// # Parameters
///
/// * `door` — mutable reference to the door thinker data
/// * `sector` — mutable reference to the sector being moved
/// * `change_sector_fn` — callback equivalent to `P_ChangeSector`; takes
///   `crush: bool` and returns `true` if something does not fit (nofit)
/// * `sound_fn` — callback to play a sound effect at the sector's sound origin
///
/// # Behavioral parity
///
/// Exactly reproduces the C `T_VerticalDoor` logic including:
/// - `blazeRaise`/`blazeClose` playing `sfx_bdcls` on close completion
/// - `close30ThenOpen` transitioning to waiting (30 second countdown) after close
/// - Crush reversal for non-close door types
/// - `raiseIn5Mins` converting to `normal` type when countdown expires
pub fn t_vertical_door(
    door: &mut VldoorT,
    sector: &mut Sector,
    change_sector_fn: &mut dyn FnMut(bool) -> bool,
    sound_fn: &mut dyn FnMut(SfxEnum),
) -> bool {
    match door.direction {
        // ====================================================================
        // direction == 0: WAITING
        // Decrement topcountdown; when it hits zero, transition based on type.
        // ====================================================================
        0 => {
            door.topcountdown -= 1;
            if door.topcountdown <= 0 {
                match door.door_type {
                    VldoorType::BlazeRaise => {
                        // Fast door: start closing
                        door.direction = -1;
                        sound_fn(SfxEnum::sfx_bdcls);
                    }
                    VldoorType::Normal => {
                        // Normal door: start closing
                        door.direction = -1;
                        sound_fn(SfxEnum::sfx_dorcls);
                    }
                    VldoorType::Close30ThenOpen => {
                        // Was waiting after close; now reopen
                        door.direction = 1;
                        sound_fn(SfxEnum::sfx_doropn);
                    }
                    _ => {}
                }
            }
            false
        }

        // ====================================================================
        // direction == 2: INITIAL WAIT (for raiseIn5Mins doors)
        // Decrement topcountdown; when it hits zero, begin opening.
        // ====================================================================
        2 => {
            door.topcountdown -= 1;
            if door.topcountdown <= 0 && door.door_type == VldoorType::RaiseIn5Mins {
                door.direction = 1;
                door.door_type = VldoorType::Normal;
                sound_fn(SfxEnum::sfx_doropn);
            }
            false
        }

        // ====================================================================
        // direction == -1: CLOSING (moving ceiling down toward floor)
        // ====================================================================
        -1 => {
            let res = t_move_plane(
                sector,
                door.speed,
                sector.floorheight,
                false,
                1, // ceiling
                door.direction,
                change_sector_fn,
            );

            match res {
                ResultE::PastDest => {
                    match door.door_type {
                        VldoorType::BlazeRaise | VldoorType::BlazeClose => {
                            // Blaze doors: remove thinker and play close sound
                            sector.specialdata = None;
                            sound_fn(SfxEnum::sfx_bdcls);
                            return true; // remove thinker
                        }
                        VldoorType::Normal | VldoorType::Close => {
                            // Normal/close doors: just remove thinker (no extra sound)
                            sector.specialdata = None;
                            return true; // remove thinker
                        }
                        VldoorType::Close30ThenOpen => {
                            // Transition to waiting state for 30 seconds
                            door.direction = 0;
                            door.topcountdown = 35 * 30;
                        }
                        _ => {}
                    }
                    false
                }
                ResultE::Crushed => {
                    match door.door_type {
                        VldoorType::BlazeClose | VldoorType::Close => {
                            // Close-type doors do NOT reverse on crush — keep closing
                        }
                        _ => {
                            // All other types: reverse direction (reopen)
                            door.direction = 1;
                            sound_fn(SfxEnum::sfx_doropn);
                        }
                    }
                    false
                }
                _ => false,
            }
        }

        // ====================================================================
        // direction == 1: OPENING (moving ceiling up toward topheight)
        // ====================================================================
        1 => {
            let res = t_move_plane(
                sector,
                door.speed,
                door.topheight,
                false,
                1, // ceiling
                door.direction,
                change_sector_fn,
            );

            if res == ResultE::PastDest {
                match door.door_type {
                    VldoorType::BlazeRaise | VldoorType::Normal => {
                        // Reached top: start waiting before closing
                        door.direction = 0;
                        door.topcountdown = door.topwait;
                    }
                    VldoorType::Close30ThenOpen | VldoorType::BlazeOpen | VldoorType::Open => {
                        // These stay open permanently: remove thinker
                        sector.specialdata = None;
                        return true; // remove thinker
                    }
                    _ => {}
                }
            }
            false
        }

        // Unknown direction — no action
        _ => false,
    }
}

// ============================================================================
// EV_DoLockedDoor — Check keys and open locked door
// Translated from linuxdoom-1.10/p_doors.c lines 183-245
// ============================================================================

/// Check if a player has the required key for a locked door, display a
/// denial message if not, and dispatch the door action if the key is held.
///
/// Returns `true` if a door was successfully activated, `false` if the
/// activator lacks the required key or is not a player.
///
/// # Key check logic
///
/// - Specials 99, 133: require blue card OR blue skull
/// - Specials 134, 135: require red card OR red skull
/// - Specials 136, 137: require yellow card OR yellow skull
/// - All other specials: no key required
///
/// If the player lacks the key, `player.message` is set to the appropriate
/// denial string and `sfx_oof` is played.
pub fn ev_do_locked_door(
    line_idx: usize,
    door_type: VldoorType,
    thing_idx: usize,
    ctx: &mut dyn SpecContext,
) -> bool {
    // Only players can open locked doors
    let player_idx = match ctx.mobjs()[thing_idx].player {
        Some(p) => p,
        None => return false,
    };

    let line_special = ctx.lines()[line_idx].special;

    match line_special {
        // Blue lock
        99 | 133 => {
            let cards = ctx.players()[player_idx].cards;
            if !cards[Card::BlueCard as usize] && !cards[Card::BlueSkull as usize] {
                ctx.players_mut()[player_idx].message = Some(String::from(PD_BLUEO));
                ctx.s_start_sound(None, SfxEnum::sfx_oof);
                return false;
            }
        }
        // Red lock
        134 | 135 => {
            let cards = ctx.players()[player_idx].cards;
            if !cards[Card::RedCard as usize] && !cards[Card::RedSkull as usize] {
                ctx.players_mut()[player_idx].message = Some(String::from(PD_REDO));
                ctx.s_start_sound(None, SfxEnum::sfx_oof);
                return false;
            }
        }
        // Yellow lock
        136 | 137 => {
            let cards = ctx.players()[player_idx].cards;
            if !cards[Card::YellowCard as usize] && !cards[Card::YellowSkull as usize] {
                ctx.players_mut()[player_idx].message = Some(String::from(PD_YELLOWO));
                ctx.s_start_sound(None, SfxEnum::sfx_oof);
                return false;
            }
        }
        _ => {}
    }

    // Key check passed (or no key required) — activate the door
    ev_do_door(line_idx, door_type, ctx)
}

// ============================================================================
// EV_DoDoor — Spawn door movers by line tag
// Translated from linuxdoom-1.10/p_doors.c lines 250-339
// ============================================================================

/// Spawn vertical door thinkers for all sectors matching the trigger
/// line's tag.
///
/// Returns `true` if any door movement was started.
///
/// For each matching sector that does not already have an active mover
/// (`specialdata`), a new [`VldoorT`] thinker is created, configured
/// according to `door_type`, and registered with the thinker system.
///
/// # Door type behaviors
///
/// | Type | Direction | Speed | Sound |
/// |------|-----------|-------|-------|
/// | `BlazeClose` | -1 (close) | 4× VDOORSPEED | `sfx_bdcls` |
/// | `Close` | -1 (close) | VDOORSPEED | `sfx_dorcls` |
/// | `Close30ThenOpen` | -1 (close) | VDOORSPEED | `sfx_dorcls` |
/// | `BlazeRaise`/`BlazeOpen` | 1 (open) | 4× VDOORSPEED | `sfx_bdopn` (if moving) |
/// | `Normal`/`Open` | 1 (open) | VDOORSPEED | `sfx_doropn` (if moving) |
pub fn ev_do_door(line_idx: usize, door_type: VldoorType, ctx: &mut dyn SpecContext) -> bool {
    let mut rtn = false;
    let mut secnum: i32 = -1;

    loop {
        secnum = p_find_sector_from_line_tag(&ctx.lines()[line_idx], secnum, ctx.sectors());
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // Skip if sector already has an active mover
        if ctx.sectors()[sec_idx].specialdata.is_some() {
            continue;
        }

        rtn = true;

        // Create and configure the door thinker
        let mut door = VldoorT::new(sec_idx);
        door.thinker = Thinker::new(ActionFn::VerticalDoor);
        door.door_type = door_type;
        door.topwait = VDOORWAIT;
        door.speed = Fixed::new(VDOORSPEED);

        match door_type {
            VldoorType::BlazeClose => {
                door.topheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                door.topheight = door.topheight - Fixed::new(4 * FRACUNIT);
                door.direction = -1;
                door.speed = Fixed::new(VDOORSPEED * 4);
                ctx.s_start_sound(Some(sec_idx), SfxEnum::sfx_bdcls);
            }
            VldoorType::Close => {
                door.topheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                door.topheight = door.topheight - Fixed::new(4 * FRACUNIT);
                door.direction = -1;
                ctx.s_start_sound(Some(sec_idx), SfxEnum::sfx_dorcls);
            }
            VldoorType::Close30ThenOpen => {
                door.topheight = ctx.sectors()[sec_idx].ceilingheight;
                door.direction = -1;
                ctx.s_start_sound(Some(sec_idx), SfxEnum::sfx_dorcls);
            }
            VldoorType::BlazeRaise | VldoorType::BlazeOpen => {
                door.direction = 1;
                door.topheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                door.topheight = door.topheight - Fixed::new(4 * FRACUNIT);
                door.speed = Fixed::new(VDOORSPEED * 4);
                if door.topheight != ctx.sectors()[sec_idx].ceilingheight {
                    ctx.s_start_sound(Some(sec_idx), SfxEnum::sfx_bdopn);
                }
            }
            VldoorType::Normal | VldoorType::Open => {
                door.direction = 1;
                door.topheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                door.topheight = door.topheight - Fixed::new(4 * FRACUNIT);
                if door.topheight != ctx.sectors()[sec_idx].ceilingheight {
                    ctx.s_start_sound(Some(sec_idx), SfxEnum::sfx_doropn);
                }
            }
            _ => {}
        }

        // Register the door thinker and mark the sector as active.
        // The SpecContext implementor handles thinker list insertion and
        // setting sector.specialdata.
        ctx.p_add_thinker_door(door);
    }

    rtn
}

// ============================================================================
// EV_VerticalDoor — Manual door activation (player USE action)
// Translated from linuxdoom-1.10/p_doors.c lines 346-480
// ============================================================================

/// Handle manual door activation when a player presses USE on a door linedef.
///
/// This function handles:
/// 1. Lock checks for key-locked manual doors (specials 26–28, 32–34)
/// 2. Re-use (bump) behavior: reversing an already-moving door
/// 3. Creating a new door thinker for inactive sectors
/// 4. Sound dispatch based on door type (normal vs. blazing)
/// 5. Setting the correct door type and speed for each line special
///
/// # Re-use check (bump logic)
///
/// For line specials 1, 26, 27, 28, and 117, if the sector already has an
/// active door thinker:
/// - If the door is closing (direction == -1): reverse to opening
/// - If the door is opening (direction == 1) and the activator is a player:
///   reverse to closing (the "bump an opening door to close it" behavior)
/// - Monsters never close doors (the `if (!thing->player) return` guard)
///
/// # Line specials handled
///
/// | Special | Type | Key | Stays open? | Speed |
/// |---------|------|-----|-------------|-------|
/// | 1 | Normal | None | No (wait+close) | Normal |
/// | 26 | Normal | Blue | No | Normal |
/// | 27 | Normal | Yellow | No | Normal |
/// | 28 | Normal | Red | No | Normal |
/// | 31 | Open | None | Yes | Normal |
/// | 32 | Open | Blue | Yes | Normal |
/// | 33 | Open | Red | Yes | Normal |
/// | 34 | Open | Yellow | Yes | Normal |
/// | 117 | BlazeRaise | None | No | 4× speed |
/// | 118 | BlazeOpen | None | Yes | 4× speed |
pub fn ev_vertical_door(line_idx: usize, thing_idx: usize, ctx: &mut dyn SpecContext) {
    let line_special = ctx.lines()[line_idx].special;

    // ---- Lock checks for key-locked manual doors ----
    // Only players can open locked doors; monsters pass through unlocked doors.
    match line_special {
        // Blue lock (manual raise and open-stay)
        26 | 32 => {
            let player_idx = match ctx.mobjs()[thing_idx].player {
                Some(p) => p,
                None => return,
            };
            let cards = ctx.players()[player_idx].cards;
            if !cards[Card::BlueCard as usize] && !cards[Card::BlueSkull as usize] {
                ctx.players_mut()[player_idx].message = Some(String::from(PD_BLUEK));
                ctx.s_start_sound(None, SfxEnum::sfx_oof);
                return;
            }
        }
        // Yellow lock
        27 | 34 => {
            let player_idx = match ctx.mobjs()[thing_idx].player {
                Some(p) => p,
                None => return,
            };
            let cards = ctx.players()[player_idx].cards;
            if !cards[Card::YellowCard as usize] && !cards[Card::YellowSkull as usize] {
                ctx.players_mut()[player_idx].message = Some(String::from(PD_YELLOWK));
                ctx.s_start_sound(None, SfxEnum::sfx_oof);
                return;
            }
        }
        // Red lock
        28 | 33 => {
            let player_idx = match ctx.mobjs()[thing_idx].player {
                Some(p) => p,
                None => return,
            };
            let cards = ctx.players()[player_idx].cards;
            if !cards[Card::RedCard as usize] && !cards[Card::RedSkull as usize] {
                ctx.players_mut()[player_idx].message = Some(String::from(PD_REDK));
                ctx.s_start_sound(None, SfxEnum::sfx_oof);
                return;
            }
        }
        _ => {}
    }

    // ---- Get the sector on the back side of the line ----
    // In the C code: sec = sides[ line->sidenum[side^1] ].sector
    // side is always 0 for USE actions, so side^1 = 1 (back side).
    let sec_idx = {
        let line = &ctx.lines()[line_idx];
        match line.backsector {
            Some(s) => s,
            None => return, // One-sided line — cannot be a door
        }
    };

    // ---- Re-use check: bump an already-active door ----
    // Only for "raise" door types (not "open-stay" types).
    let has_active_door = ctx.sectors()[sec_idx].specialdata.is_some();
    if has_active_door {
        match line_special {
            1 | 26 | 27 | 28 | 117 => {
                // Pre-check: is the activator a player? (needed below for close logic)
                let is_player = ctx.mobjs()[thing_idx].player.is_some();

                // Try to reverse the existing door
                let door_handle = ctx.sectors()[sec_idx].specialdata.unwrap();
                if let Some(door_data) = ctx.get_door_data_mut(door_handle) {
                    if door_data.direction == -1 {
                        // Door is closing — reverse to opening
                        door_data.direction = 1;
                    } else {
                        // Door is opening or waiting — only players can close
                        if !is_player {
                            return; // Bad guys never close doors
                        }
                        door_data.direction = -1;
                    }
                }
                return;
            }
            _ => {}
        }
    }

    // ---- Play door sound ----
    match line_special {
        117 | 118 => {
            // Blazing door sound
            ctx.s_start_sound(Some(sec_idx), SfxEnum::sfx_bdopn);
        }
        _ => {
            // Normal door sound
            ctx.s_start_sound(Some(sec_idx), SfxEnum::sfx_doropn);
        }
    }

    // ---- Create new door thinker ----
    let mut door = VldoorT::new(sec_idx);
    door.thinker = Thinker::new(ActionFn::VerticalDoor);
    door.direction = 1;
    door.speed = Fixed::new(VDOORSPEED);
    door.topwait = VDOORWAIT;

    // Set door type and speed based on line special
    match line_special {
        1 | 26 | 27 | 28 => {
            door.door_type = VldoorType::Normal;
        }
        31..=34 => {
            door.door_type = VldoorType::Open;
            // One-shot: clear the line special so it can't be re-used
            ctx.lines_mut()[line_idx].special = 0;
        }
        117 => {
            // Blazing door raise
            door.door_type = VldoorType::BlazeRaise;
            door.speed = Fixed::new(VDOORSPEED * 4);
        }
        118 => {
            // Blazing door open (stay open)
            door.door_type = VldoorType::BlazeOpen;
            door.speed = Fixed::new(VDOORSPEED * 4);
            ctx.lines_mut()[line_idx].special = 0;
        }
        _ => {
            // Default: treat as normal door
            door.door_type = VldoorType::Normal;
        }
    }

    // Find the door's top height
    door.topheight = p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines())
        - Fixed::new(4 * FRACUNIT);

    // Register the door thinker and mark the sector as active
    ctx.p_add_thinker_door(door);
}

// ============================================================================
// P_SpawnDoorCloseIn30 — Spawn a door that closes in 30 seconds
// Translated from linuxdoom-1.10/p_doors.c lines 486-504
// ============================================================================

/// Spawn a door thinker that will close the sector's ceiling in 30 seconds.
///
/// Called during [`p_spawn_specials`] for sector type 10.
///
/// The door starts in the waiting state (direction = 0) with a countdown
/// of 30 × 35 = 1050 tics. When the countdown expires, `T_VerticalDoor`
/// will begin closing the door (type = normal → direction = -1).
///
/// Also clears the sector's `special` field to prevent the sector type
/// from being processed again.
pub fn p_spawn_door_close_in30(sector_idx: usize, ctx: &mut dyn SpecContext) {
    let mut door = VldoorT::new(sector_idx);
    door.thinker = Thinker::new(ActionFn::VerticalDoor);
    door.direction = 0; // waiting state
    door.door_type = VldoorType::Normal;
    door.speed = Fixed::new(VDOORSPEED);
    door.topcountdown = 30 * 35; // 30 seconds at 35 tics/sec
    door.topheight = ctx.sectors()[sector_idx].ceilingheight;

    // Clear the sector special so it doesn't trigger again
    ctx.sectors_mut()[sector_idx].special = 0;

    // Register thinker and mark sector as active
    ctx.p_add_thinker_door(door);
}

// ============================================================================
// P_SpawnDoorRaiseIn5Mins — Spawn a door that opens in 5 minutes
// Translated from linuxdoom-1.10/p_doors.c lines 510-548
// ============================================================================

/// Spawn a door thinker that will open the sector after a 5-minute delay.
///
/// Called during [`p_spawn_specials`] for sector type 14.
///
/// The door starts in the initial-wait state (direction = 2) with a countdown
/// of 5 × 60 × 35 = 10500 tics. When the countdown expires, `T_VerticalDoor`
/// converts the door type to `Normal` and begins opening.
///
/// Also clears the sector's `special` field to prevent the sector type
/// from being processed again.
pub fn p_spawn_door_raise_in5_mins(sector_idx: usize, _secnum: i32, ctx: &mut dyn SpecContext) {
    let mut door = VldoorT::new(sector_idx);
    door.thinker = Thinker::new(ActionFn::VerticalDoor);
    door.direction = 2; // initial-wait state
    door.door_type = VldoorType::RaiseIn5Mins;
    door.speed = Fixed::new(VDOORSPEED);
    door.topheight = p_find_lowest_ceiling_surrounding(sector_idx, ctx.sectors(), ctx.lines())
        - Fixed::new(4 * FRACUNIT);
    door.topwait = VDOORWAIT;
    door.topcountdown = 5 * 60 * 35; // 5 minutes at 35 tics/sec

    // Clear the sector special so it doesn't trigger again
    ctx.sectors_mut()[sector_idx].special = 0;

    // Register thinker and mark sector as active
    ctx.p_add_thinker_door(door);
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::fixed::FRACUNIT;
    use crate::types::map_data::Sector;

    // ---- t_vertical_door tests ----

    /// Helper: create a default door for testing.
    fn make_test_door(direction: i32, door_type: VldoorType) -> VldoorT {
        let mut door = VldoorT::new(0);
        door.direction = direction;
        door.door_type = door_type;
        door.speed = Fixed::new(VDOORSPEED);
        door.topwait = VDOORWAIT;
        door.topheight = Fixed::new(128 * FRACUNIT);
        door.topcountdown = VDOORWAIT;
        door
    }

    /// Helper: create a test sector.
    fn make_test_sector() -> Sector {
        Sector {
            floorheight: Fixed::ZERO,
            ceilingheight: Fixed::new(128 * FRACUNIT),
            specialdata: Some(42),
            ..Sector::default()
        }
    }

    /// No-op change_sector function (never blocks).
    fn noop_change(_crush: bool) -> bool {
        false
    }

    /// Collects sounds played during a test.
    struct SoundCollector {
        sounds: Vec<SfxEnum>,
    }

    impl SoundCollector {
        fn new() -> Self {
            Self { sounds: Vec::new() }
        }
    }

    #[test]
    fn test_t_vertical_door_waiting_decrement() {
        let mut door = make_test_door(0, VldoorType::Normal);
        door.topcountdown = 50;
        let mut sector = make_test_sector();
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result); // should NOT remove thinker
        assert_eq!(door.topcountdown, 49); // decremented
        assert_eq!(door.direction, 0); // still waiting
        assert!(sounds.sounds.is_empty()); // no sound yet
    }

    #[test]
    fn test_t_vertical_door_waiting_timeout_normal() {
        let mut door = make_test_door(0, VldoorType::Normal);
        door.topcountdown = 1; // will hit zero
        let mut sector = make_test_sector();
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result);
        assert_eq!(door.direction, -1); // started closing
        assert_eq!(sounds.sounds, vec![SfxEnum::sfx_dorcls]);
    }

    #[test]
    fn test_t_vertical_door_waiting_timeout_blaze_raise() {
        let mut door = make_test_door(0, VldoorType::BlazeRaise);
        door.topcountdown = 1;
        let mut sector = make_test_sector();
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result);
        assert_eq!(door.direction, -1);
        assert_eq!(sounds.sounds, vec![SfxEnum::sfx_bdcls]);
    }

    #[test]
    fn test_t_vertical_door_waiting_timeout_close30_then_open() {
        let mut door = make_test_door(0, VldoorType::Close30ThenOpen);
        door.topcountdown = 1;
        let mut sector = make_test_sector();
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result);
        assert_eq!(door.direction, 1); // started opening
        assert_eq!(sounds.sounds, vec![SfxEnum::sfx_doropn]);
    }

    #[test]
    fn test_t_vertical_door_initial_wait_raise_in_5_mins() {
        let mut door = make_test_door(2, VldoorType::RaiseIn5Mins);
        door.topcountdown = 1;
        let mut sector = make_test_sector();
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result);
        assert_eq!(door.direction, 1);
        assert_eq!(door.door_type, VldoorType::Normal); // converted
        assert_eq!(sounds.sounds, vec![SfxEnum::sfx_doropn]);
    }

    #[test]
    fn test_t_vertical_door_closing_past_dest_normal() {
        let mut door = make_test_door(-1, VldoorType::Normal);
        let mut sector = make_test_sector();
        // Sector ceiling is already at floor height → PastDest
        sector.ceilingheight = sector.floorheight;
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(result); // should remove thinker
        assert!(sector.specialdata.is_none()); // cleared
    }

    #[test]
    fn test_t_vertical_door_closing_past_dest_blaze() {
        let mut door = make_test_door(-1, VldoorType::BlazeRaise);
        let mut sector = make_test_sector();
        sector.ceilingheight = sector.floorheight;
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(result); // should remove
        assert!(sector.specialdata.is_none());
        assert_eq!(sounds.sounds, vec![SfxEnum::sfx_bdcls]);
    }

    #[test]
    fn test_t_vertical_door_closing_past_dest_close30_then_open() {
        let mut door = make_test_door(-1, VldoorType::Close30ThenOpen);
        let mut sector = make_test_sector();
        sector.ceilingheight = sector.floorheight;
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result); // should NOT remove (transitions to waiting)
        assert_eq!(door.direction, 0);
        assert_eq!(door.topcountdown, 35 * 30);
    }

    #[test]
    fn test_t_vertical_door_closing_crushed_reverses() {
        let mut door = make_test_door(-1, VldoorType::Normal);
        let mut sector = make_test_sector();
        // Ceiling is above floor, but change_sector reports crush
        sector.ceilingheight = Fixed::new(64 * FRACUNIT);
        let mut sounds = SoundCollector::new();

        // Make change_sector always report crush
        let mut crush_fn = |_crush: bool| -> bool { true };

        let result = t_vertical_door(&mut door, &mut sector, &mut crush_fn, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result);
        assert_eq!(door.direction, 1); // reversed to opening
        assert_eq!(sounds.sounds, vec![SfxEnum::sfx_doropn]);
    }

    #[test]
    fn test_t_vertical_door_closing_crushed_close_type_no_reverse() {
        let mut door = make_test_door(-1, VldoorType::Close);
        let mut sector = make_test_sector();
        sector.ceilingheight = Fixed::new(64 * FRACUNIT);
        let mut sounds = SoundCollector::new();

        let mut crush_fn = |_crush: bool| -> bool { true };

        let result = t_vertical_door(&mut door, &mut sector, &mut crush_fn, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result);
        assert_eq!(door.direction, -1); // still closing (no reverse for Close type)
        assert!(sounds.sounds.is_empty());
    }

    #[test]
    fn test_t_vertical_door_closing_crushed_blaze_close_no_reverse() {
        let mut door = make_test_door(-1, VldoorType::BlazeClose);
        let mut sector = make_test_sector();
        sector.ceilingheight = Fixed::new(64 * FRACUNIT);
        let mut sounds = SoundCollector::new();

        let mut crush_fn = |_crush: bool| -> bool { true };

        let result = t_vertical_door(&mut door, &mut sector, &mut crush_fn, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result);
        assert_eq!(door.direction, -1); // still closing
        assert!(sounds.sounds.is_empty());
    }

    #[test]
    fn test_t_vertical_door_opening_past_dest_normal() {
        let mut door = make_test_door(1, VldoorType::Normal);
        door.topheight = Fixed::new(128 * FRACUNIT);
        let mut sector = make_test_sector();
        // Ceiling already at topheight → PastDest
        sector.ceilingheight = door.topheight;
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(!result); // stays active (waiting)
        assert_eq!(door.direction, 0); // now waiting
        assert_eq!(door.topcountdown, VDOORWAIT);
    }

    #[test]
    fn test_t_vertical_door_opening_past_dest_open_type() {
        let mut door = make_test_door(1, VldoorType::Open);
        door.topheight = Fixed::new(128 * FRACUNIT);
        let mut sector = make_test_sector();
        sector.ceilingheight = door.topheight;
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(result); // should remove (stays open permanently)
        assert!(sector.specialdata.is_none());
    }

    #[test]
    fn test_t_vertical_door_opening_past_dest_blaze_open() {
        let mut door = make_test_door(1, VldoorType::BlazeOpen);
        door.topheight = Fixed::new(128 * FRACUNIT);
        let mut sector = make_test_sector();
        sector.ceilingheight = door.topheight;
        let mut sounds = SoundCollector::new();

        let result = t_vertical_door(&mut door, &mut sector, &mut noop_change, &mut |sfx| {
            sounds.sounds.push(sfx)
        });

        assert!(result); // should remove
        assert!(sector.specialdata.is_none());
    }
}
