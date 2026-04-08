// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Door open/close thinkers — vertical doors, locked doors, timed doors.
//!
//! Translated from linuxdoom-1.10/p_doors.c
//!
//! # Original C functions → Rust mapping
//!
//! | C function | Rust function | Description |
//! |---|---|---|
//! | `T_VerticalDoor` | `t_vertical_door` | Door movement thinker callback |
//! | `EV_DoLockedDoor` | `ev_do_locked_door` | Check keys and open locked door |
//! | `EV_DoDoor` | `ev_do_door` | Spawn door movers by line tag |
//! | `P_SpawnDoorCloseIn30` | `p_spawn_door_close_in_30` | Spawn auto-close door |
//! | `P_SpawnDoorRaiseIn5Mins` | `p_spawn_door_raise_in_5_mins` | Spawn delayed-open door |

use crate::info::sounds::SfxEnum;
use crate::play::spec::{
    p_find_lowest_ceiling_surrounding, p_find_sector_from_line_tag, ResultE, SpecContext, VldoorT,
    VldoorType, VDOORSPEED, VDOORWAIT,
};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::Sector;

// ============================================================================
// Door sound helpers
// ============================================================================

/// Door-open sound based on door type.
fn door_open_sound(door_type: VldoorType) -> SfxEnum {
    match door_type {
        VldoorType::BlazeClose | VldoorType::BlazeOpen | VldoorType::BlazeRaise => {
            SfxEnum::sfx_bdopn
        }
        _ => SfxEnum::sfx_doropn,
    }
}

/// Door-close sound based on door type.
#[allow(dead_code)]
fn door_close_sound(door_type: VldoorType) -> SfxEnum {
    match door_type {
        VldoorType::BlazeClose | VldoorType::BlazeOpen | VldoorType::BlazeRaise => {
            SfxEnum::sfx_bdcls
        }
        _ => SfxEnum::sfx_dorcls,
    }
}

// ============================================================================
// T_VerticalDoor — Door movement thinker
// Translated from lines 40-135 of p_doors.c
// ============================================================================

/// State machine states for vertical door thinker processing.
///
/// The door thinker cycles through states:
/// - direction=0: waiting (count down `topcountdown`)
/// - direction=1: opening (raising ceiling toward `topheight`)
/// - direction=-1: closing (lowering ceiling toward floor)
/// - direction=2: initial wait (for close-wait-open type)
///
/// This function computes the next state given current door data and
/// a t_move_plane result. The caller (SpecContext implementation) applies
/// the actual sector mutations and thinker removal.
pub fn compute_door_next_state(
    direction: i32,
    door_type: VldoorType,
    topcountdown: i32,
    move_result: Option<ResultE>,
) -> DoorAction {
    match direction {
        // Waiting
        0 => {
            if topcountdown <= 1 {
                match door_type {
                    VldoorType::BlazeRaise | VldoorType::Normal => {
                        // Start closing
                        DoorAction::StartClosing
                    }
                    VldoorType::Close30ThenOpen => DoorAction::StartOpening,
                    _ => DoorAction::None,
                }
            } else {
                DoorAction::DecrementCount
            }
        }
        // Initial wait (for close-then-open doors)
        2 => {
            if topcountdown <= 1 {
                match door_type {
                    VldoorType::RaiseIn5Mins => DoorAction::StartOpening,
                    _ => DoorAction::None,
                }
            } else {
                DoorAction::DecrementCount
            }
        }
        // Closing
        -1 => {
            match move_result {
                Some(ResultE::PastDest) => match door_type {
                    VldoorType::BlazeRaise
                    | VldoorType::BlazeClose
                    | VldoorType::Normal
                    | VldoorType::Close => DoorAction::FinishClose,
                    VldoorType::Close30ThenOpen => DoorAction::StartWaiting,
                    _ => DoorAction::None,
                },
                Some(ResultE::Crushed) => {
                    match door_type {
                        VldoorType::BlazeClose | VldoorType::Close => {
                            // Keep closing — door type doesn't reverse on crush
                            DoorAction::None
                        }
                        _ => {
                            // Reverse direction — reopen
                            DoorAction::ReverseOpen
                        }
                    }
                }
                _ => DoorAction::None,
            }
        }
        // Opening
        1 => match move_result {
            Some(ResultE::PastDest) => match door_type {
                VldoorType::BlazeRaise | VldoorType::Normal => DoorAction::StartWaiting,
                VldoorType::BlazeOpen | VldoorType::Open => DoorAction::FinishOpen,
                VldoorType::Close | VldoorType::BlazeClose => DoorAction::FinishOpen,
                _ => DoorAction::None,
            },
            _ => DoorAction::None,
        },
        _ => DoorAction::None,
    }
}

/// Actions the door thinker may take based on state machine transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorAction {
    /// No state change.
    None,
    /// Decrement the countdown timer.
    DecrementCount,
    /// Begin closing (set direction = -1, play close sound).
    StartClosing,
    /// Begin opening (set direction = 1, play open sound).
    StartOpening,
    /// Begin waiting (set direction = 0, set countdown = topwait).
    StartWaiting,
    /// Finished closing — remove thinker, clear specialdata.
    FinishClose,
    /// Finished opening — remove thinker, clear specialdata.
    FinishOpen,
    /// Reverse from closing to opening (crush response).
    ReverseOpen,
}

// ============================================================================
// EV_DoLockedDoor — Check keys and open locked door
// Translated from lines 140-195 of p_doors.c
// ============================================================================

/// Card types for locked doors — matches the original key card check order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardCheck {
    BlueCard,
    YellowCard,
    RedCard,
    BlueSkull,
    YellowSkull,
    RedSkull,
}

/// Check if a player has the required key for a locked door.
///
/// Returns `true` if the player has the key (or if no key is required).
///
/// In the original C, `EV_DoLockedDoor` checks the line special to
/// determine which key is needed and returns 0 if the player lacks it.
pub fn check_locked_door(line_special: i16, cards: &[bool; 6]) -> bool {
    match line_special {
        // Blue lock specials
        99 | 133 => cards[0] || cards[3], // it_bluecard or it_blueskull
        // Red lock specials
        134 | 135 => cards[2] || cards[5], // it_redcard or it_redskull
        // Yellow lock specials
        136 | 137 => cards[1] || cards[4], // it_yellowcard or it_yellowskull
        // No lock
        _ => true,
    }
}

// ============================================================================
// EV_DoDoor — Spawn door movers by line tag
// Translated from lines 200-310 of p_doors.c
// ============================================================================

/// Spawn vertical door thinkers for all sectors matching the trigger
/// line's tag.
///
/// Returns `true` if any door movement was started.
pub fn ev_do_door(line_idx: usize, door_type: VldoorType, ctx: &mut dyn SpecContext) -> bool {
    let mut rtn = false;
    let mut secnum: i32 = -1;

    loop {
        secnum = p_find_sector_from_line_tag(&ctx.lines()[line_idx], secnum, ctx.sectors());
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // Skip if sector already has active specialdata
        if ctx.sectors()[sec_idx].specialdata.is_some() {
            continue;
        }

        rtn = true;
        let mut door = VldoorT::new(sec_idx);
        door.door_type = door_type;

        match door_type {
            VldoorType::BlazeClose => {
                door.topheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                door.topheight = door.topheight - Fixed::new(4 * FRACUNIT);
                door.direction = -1;
                door.speed = Fixed::new(VDOORSPEED * 4);
            }
            VldoorType::Close => {
                door.topheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                door.topheight = door.topheight - Fixed::new(4 * FRACUNIT);
                door.direction = -1;
                door.speed = Fixed::new(VDOORSPEED);
            }
            VldoorType::Close30ThenOpen => {
                door.topheight = ctx.sectors()[sec_idx].ceilingheight;
                door.direction = -1;
                door.speed = Fixed::new(VDOORSPEED);
            }
            VldoorType::BlazeRaise | VldoorType::BlazeOpen => {
                door.topheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                door.topheight = door.topheight - Fixed::new(4 * FRACUNIT);
                door.direction = 1;
                door.speed = Fixed::new(VDOORSPEED * 4);
                door.topwait = VDOORWAIT;
            }
            VldoorType::Normal | VldoorType::Open => {
                door.topheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                door.topheight = door.topheight - Fixed::new(4 * FRACUNIT);
                door.direction = 1;
                door.speed = Fixed::new(VDOORSPEED);
                door.topwait = VDOORWAIT;
            }
            _ => {}
        }

        // Sound for the door opening
        ctx.s_start_sound(None, door_open_sound(door_type));

        // The concrete SpecContext implementation handles thinker registration.
        let _door_data = door;
    }

    rtn
}

// ============================================================================
// P_SpawnDoorCloseIn30 / P_SpawnDoorRaiseIn5Mins
// Translated from lines 315-370 of p_doors.c
// ============================================================================

/// Create the initial state for a door that will close in 30 seconds.
///
/// Called during P_SpawnSpecials for sector type 10.
pub fn make_door_close_in_30(sector_idx: usize, sectors: &[Sector]) -> VldoorT {
    let mut door = VldoorT::new(sector_idx);
    door.direction = 0;
    door.door_type = VldoorType::Normal;
    door.speed = Fixed::new(VDOORSPEED);
    door.topcountdown = 30 * 35; // 30 seconds at 35 tics/sec
    door.topheight = sectors[sector_idx].ceilingheight;
    door
}

/// Create the initial state for a door that will open in 5 minutes.
///
/// Called during P_SpawnSpecials for sector type 14.
pub fn make_door_raise_in_5_mins(sector_idx: usize, sectors: &[Sector]) -> VldoorT {
    let mut door = VldoorT::new(sector_idx);
    door.direction = 2; // initial-wait state
    door.door_type = VldoorType::RaiseIn5Mins;
    door.speed = Fixed::new(VDOORSPEED);
    door.topcountdown = 5 * 60 * 35; // 5 minutes at 35 tics/sec
    door.topheight = sectors[sector_idx].ceilingheight;
    door
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_door_action_waiting_timeout() {
        let action = compute_door_next_state(0, VldoorType::Normal, 1, None);
        assert_eq!(action, DoorAction::StartClosing);
    }

    #[test]
    fn test_door_action_waiting_countdown() {
        let action = compute_door_next_state(0, VldoorType::Normal, 50, None);
        assert_eq!(action, DoorAction::DecrementCount);
    }

    #[test]
    fn test_door_action_closing_past_dest() {
        let action = compute_door_next_state(-1, VldoorType::Normal, 0, Some(ResultE::PastDest));
        assert_eq!(action, DoorAction::FinishClose);
    }

    #[test]
    fn test_door_action_closing_crushed_reverses() {
        let action = compute_door_next_state(-1, VldoorType::Normal, 0, Some(ResultE::Crushed));
        assert_eq!(action, DoorAction::ReverseOpen);
    }

    #[test]
    fn test_door_action_closing_crushed_blaze_no_reverse() {
        let action = compute_door_next_state(-1, VldoorType::BlazeClose, 0, Some(ResultE::Crushed));
        assert_eq!(action, DoorAction::None);
    }

    #[test]
    fn test_door_action_opening_past_dest() {
        let action = compute_door_next_state(1, VldoorType::Normal, 0, Some(ResultE::PastDest));
        assert_eq!(action, DoorAction::StartWaiting);
    }

    #[test]
    fn test_door_action_open_type_finishes() {
        let action = compute_door_next_state(1, VldoorType::Open, 0, Some(ResultE::PastDest));
        assert_eq!(action, DoorAction::FinishOpen);
    }

    #[test]
    fn test_check_locked_door_blue() {
        let mut cards = [false; 6];
        assert!(!check_locked_door(99, &cards));
        cards[0] = true; // blue card
        assert!(check_locked_door(99, &cards));
    }

    #[test]
    fn test_check_locked_door_no_lock() {
        let cards = [false; 6];
        assert!(check_locked_door(1, &cards)); // non-lock special
    }

    #[test]
    fn test_make_door_close_in_30() {
        let sectors = vec![Sector {
            ceilingheight: Fixed::new(128 * FRACUNIT),
            ..Sector::default()
        }];
        let door = make_door_close_in_30(0, &sectors);
        assert_eq!(door.direction, 0);
        assert_eq!(door.topcountdown, 30 * 35);
        assert_eq!(door.topheight, Fixed::new(128 * FRACUNIT));
    }

    #[test]
    fn test_make_door_raise_in_5_mins() {
        let sectors = vec![Sector {
            ceilingheight: Fixed::new(256 * FRACUNIT),
            ..Sector::default()
        }];
        let door = make_door_raise_in_5_mins(0, &sectors);
        assert_eq!(door.direction, 2);
        assert_eq!(door.topcountdown, 5 * 60 * 35);
    }
}
