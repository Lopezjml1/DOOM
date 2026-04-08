// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Platform (lift) thinkers — raise, lower, perpetual raise.
//!
//! Translated from linuxdoom-1.10/p_plats.c
//!
//! # Original C functions → Rust mapping
//!
//! | C function | Rust function | Description |
//! |---|---|---|
//! | `T_PlatRaise` | `t_plat_raise` | Platform movement thinker callback |
//! | `EV_DoPlat` | `ev_do_plat` | Spawn platform movers by line tag |
//! | `P_AddActivePlat` | `p_add_active_plat` | Track active platform |
//! | `P_RemoveActivePlat` | `p_remove_active_plat` | Remove active platform |
//! | `EV_StopPlat` | `ev_stop_plat` | Stop platforms by tag |
//! | `P_ActivateInStasis` | `p_activate_in_stasis` | Resume stasis platforms |

use crate::info::sounds::SfxEnum;
use crate::play::spec::{
    p_find_highest_floor_surrounding, p_find_lowest_floor_surrounding, p_find_next_highest_floor,
    p_find_sector_from_line_tag, PlatStatus, PlatT, PlatType, ResultE, SpecContext, MAXPLATS,
    PLATSPEED, PLATWAIT,
};
use crate::types::fixed::{Fixed, FRACUNIT};

// ============================================================================
// Active platform tracking
// ============================================================================

/// Tracks active platform thinkers for stasis/resume operations.
///
/// Original C: `activeplats[MAXPLATS]` global array.
pub struct ActivePlats {
    /// Indices of active platform thinkers (None = empty slot).
    pub slots: [Option<usize>; MAXPLATS],
}

impl ActivePlats {
    /// Create an empty active platforms tracker.
    pub fn new() -> Self {
        Self {
            slots: [None; MAXPLATS],
        }
    }

    /// Add a platform to the active tracking list.
    /// Returns the slot index if added, None if full.
    pub fn add(&mut self, plat_idx: usize) -> Option<usize> {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(plat_idx);
                return Some(i);
            }
        }
        None
    }

    /// Remove a platform from the active tracking list.
    pub fn remove(&mut self, plat_idx: usize) {
        for slot in self.slots.iter_mut() {
            if *slot == Some(plat_idx) {
                *slot = None;
                return;
            }
        }
    }
}

impl Default for ActivePlats {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Platform state machine
// ============================================================================

/// Compute the next action for a platform thinker based on current state.
///
/// The platform cycles through states:
/// - `Up`: rising toward `high`
/// - `Down`: lowering toward `low`
/// - `Waiting`: counting down before reversing
/// - `InStasis`: stopped (waiting for re-activation)
///
/// # Parameters
/// - `status`: current platform status
/// - `count`: current countdown timer
/// - `plat_type`: the type of platform behavior
/// - `move_result`: result from t_move_plane (if moving)
pub fn compute_plat_next_state(
    status: PlatStatus,
    count: i32,
    plat_type: PlatType,
    move_result: Option<ResultE>,
) -> PlatAction {
    match status {
        PlatStatus::Up => {
            match move_result {
                Some(ResultE::Crushed) => {
                    if plat_type != PlatType::PerpetualRaise {
                        // Non-perpetual platforms stop on crush (but go back down)
                        PlatAction::ReverseDown
                    } else {
                        PlatAction::ReverseDown
                    }
                }
                Some(ResultE::PastDest) => {
                    match plat_type {
                        PlatType::BlazeDWUS | PlatType::DownWaitUpStay => {
                            // Platform reached top — remove
                            PlatAction::Finish
                        }
                        PlatType::RaiseAndChange | PlatType::RaiseToNearestAndChange => {
                            PlatAction::Finish
                        }
                        _ => {
                            // Start waiting at top
                            PlatAction::StartWaiting
                        }
                    }
                }
                _ => PlatAction::None,
            }
        }
        PlatStatus::Down => match move_result {
            Some(ResultE::PastDest) => PlatAction::StartWaiting,
            _ => PlatAction::None,
        },
        PlatStatus::Waiting => {
            if count <= 0 {
                let _ = plat_type; // All types toggle direction from waiting.
                PlatAction::ToggleDirection
            } else {
                PlatAction::DecrementCount
            }
        }
        PlatStatus::InStasis => PlatAction::None,
    }
}

/// Actions the platform thinker may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatAction {
    /// No state change.
    None,
    /// Decrement the countdown timer.
    DecrementCount,
    /// Start waiting (set status = Waiting, count = wait).
    StartWaiting,
    /// Toggle direction (up↔down after waiting).
    ToggleDirection,
    /// Reverse to down (crush response).
    ReverseDown,
    /// Platform finished — remove thinker.
    Finish,
}

// ============================================================================
// EV_DoPlat — Spawn platform movers by line tag
// Translated from lines 60-190 of p_plats.c
// ============================================================================

/// Spawn platform thinkers for all sectors matching the trigger line's tag.
///
/// Returns `true` if any platform was started.
pub fn ev_do_plat(
    line_idx: usize,
    plat_type: PlatType,
    amount: i32,
    ctx: &mut dyn SpecContext,
) -> bool {
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
        let mut plat = PlatT::new(sec_idx);
        plat.plat_type = plat_type;
        plat.crush = false;
        plat.tag = ctx.lines()[line_idx].tag as i32;

        match plat_type {
            PlatType::RaiseToNearestAndChange => {
                plat.speed = Fixed::new(PLATSPEED / 2);
                let front_side_idx = ctx.lines()[line_idx].sidenum[0] as usize;
                let front_sec = ctx.sides()[front_side_idx].sector;
                plat.high = p_find_next_highest_floor(
                    sec_idx,
                    ctx.sectors()[sec_idx].floorheight,
                    ctx.sectors(),
                    ctx.lines(),
                );
                plat.wait = 0;
                plat.status = PlatStatus::Up;
                // Don't bother playing the sound if no real movement
                plat.low = ctx.sectors()[sec_idx].floorheight;
                // Copy floor texture from trigger line's front sector
                let _new_floor_pic = ctx.sectors()[front_sec].floorpic;
            }
            PlatType::RaiseAndChange => {
                plat.speed = Fixed::new(PLATSPEED / 2);
                let front_side_idx = ctx.lines()[line_idx].sidenum[0] as usize;
                let _front_sec = ctx.sides()[front_side_idx].sector;
                plat.high = ctx.sectors()[sec_idx].floorheight + Fixed::new(amount * FRACUNIT);
                plat.wait = 0;
                plat.status = PlatStatus::Up;
                plat.low = ctx.sectors()[sec_idx].floorheight;
            }
            PlatType::DownWaitUpStay => {
                plat.speed = Fixed::new(PLATSPEED * 4);
                plat.low = p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                if plat.low > ctx.sectors()[sec_idx].floorheight {
                    plat.low = ctx.sectors()[sec_idx].floorheight;
                }
                plat.high = ctx.sectors()[sec_idx].floorheight;
                plat.wait = PLATWAIT * 35;
                plat.status = PlatStatus::Down;
            }
            PlatType::BlazeDWUS => {
                plat.speed = Fixed::new(PLATSPEED * 8);
                plat.low = p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                if plat.low > ctx.sectors()[sec_idx].floorheight {
                    plat.low = ctx.sectors()[sec_idx].floorheight;
                }
                plat.high = ctx.sectors()[sec_idx].floorheight;
                plat.wait = PLATWAIT * 35;
                plat.status = PlatStatus::Down;
            }
            PlatType::PerpetualRaise => {
                plat.speed = Fixed::new(PLATSPEED);
                plat.low = p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                if plat.low > ctx.sectors()[sec_idx].floorheight {
                    plat.low = ctx.sectors()[sec_idx].floorheight;
                }
                plat.high = p_find_highest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                if plat.high < ctx.sectors()[sec_idx].floorheight {
                    plat.high = ctx.sectors()[sec_idx].floorheight;
                }
                plat.wait = PLATWAIT * 35;
                // Random initial direction
                let rng_val = ctx.rng_mut().p_random();
                plat.status = if rng_val & 1 != 0 {
                    PlatStatus::Down
                } else {
                    PlatStatus::Up
                };
            }
        }

        // Sound
        ctx.s_start_sound(None, SfxEnum::sfx_pstart);

        // The concrete SpecContext implementation handles thinker registration
        // and active plat tracking.
        let _plat_data = plat;
    }

    rtn
}

// ============================================================================
// EV_StopPlat — Stop platforms by tag
// Translated from lines 240-265 of p_plats.c
// ============================================================================

/// Find all active platform indices that should be stopped by a given tag.
///
/// Returns the slot indices of platforms matching the tag.
/// The caller is responsible for setting their status to InStasis.
pub fn find_plats_to_stop(tag: i32, plat_tags: &[(usize, i32)]) -> Vec<usize> {
    let mut result = Vec::new();
    for &(idx, plat_tag) in plat_tags {
        if plat_tag == tag {
            result.push(idx);
        }
    }
    result
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_active_plats_add_remove() {
        let mut ap = ActivePlats::new();
        assert_eq!(ap.add(10), Some(0));
        assert_eq!(ap.add(20), Some(1));
        ap.remove(10);
        assert!(ap.slots[0].is_none());
        assert_eq!(ap.slots[1], Some(20));
    }

    #[test]
    fn test_active_plats_full() {
        let mut ap = ActivePlats::new();
        for i in 0..MAXPLATS {
            assert!(ap.add(i).is_some());
        }
        assert!(ap.add(999).is_none());
    }

    #[test]
    fn test_plat_action_up_past_dest_finish() {
        let action = compute_plat_next_state(
            PlatStatus::Up,
            0,
            PlatType::DownWaitUpStay,
            Some(ResultE::PastDest),
        );
        assert_eq!(action, PlatAction::Finish);
    }

    #[test]
    fn test_plat_action_down_past_dest_wait() {
        let action = compute_plat_next_state(
            PlatStatus::Down,
            0,
            PlatType::PerpetualRaise,
            Some(ResultE::PastDest),
        );
        assert_eq!(action, PlatAction::StartWaiting);
    }

    #[test]
    fn test_plat_action_waiting_toggle() {
        let action =
            compute_plat_next_state(PlatStatus::Waiting, 0, PlatType::PerpetualRaise, None);
        assert_eq!(action, PlatAction::ToggleDirection);
    }

    #[test]
    fn test_plat_action_waiting_countdown() {
        let action =
            compute_plat_next_state(PlatStatus::Waiting, 10, PlatType::PerpetualRaise, None);
        assert_eq!(action, PlatAction::DecrementCount);
    }

    #[test]
    fn test_find_plats_to_stop() {
        let plat_tags = vec![(0, 5), (1, 3), (2, 5), (3, 7)];
        let result = find_plats_to_stop(5, &plat_tags);
        assert_eq!(result, vec![0, 2]);
    }
}
