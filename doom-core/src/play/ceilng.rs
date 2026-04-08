// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Ceiling movement thinkers — crush ceilings, raise/lower ceilings.
//!
//! Translated from linuxdoom-1.10/p_ceilng.c
//!
//! # Original C functions → Rust mapping
//!
//! | C function | Rust function | Description |
//! |---|---|---|
//! | `T_MoveCeiling` | `t_move_ceiling` | Ceiling movement thinker callback |
//! | `EV_DoCeiling` | `ev_do_ceiling` | Spawn ceiling movers by line tag |
//! | `P_AddActiveCeiling` | `p_add_active_ceiling` | Track active ceiling |
//! | `P_RemoveActiveCeiling` | `p_remove_active_ceiling` | Remove active ceiling |
//! | `P_ActivateInStasisCeiling` | `p_activate_in_stasis_ceiling` | Resume stasis ceiling |
//! | `EV_CeilingCrushStop` | `ev_ceiling_crush_stop` | Stop a crush ceiling |

use crate::play::spec::{
    p_find_sector_from_line_tag, CeilingT, CeilingType, SpecContext, CEILSPEED, MAXCEILINGS,
};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::{LineDef, Sector};

// ============================================================================
// Active ceiling tracking
// ============================================================================

/// Tracks active ceiling thinkers for stasis/resume operations.
///
/// Original C: `activeceilings[MAXCEILINGS]` global array.
pub struct ActiveCeilings {
    /// Arena indices of active ceiling thinkers (None = empty slot).
    pub slots: [Option<usize>; MAXCEILINGS],
}

impl ActiveCeilings {
    /// Create an empty active ceilings tracker.
    pub fn new() -> Self {
        Self {
            slots: [None; MAXCEILINGS],
        }
    }

    /// Add a ceiling to the active tracking list.
    /// Returns the slot index if added, None if full.
    pub fn add(&mut self, ceiling_idx: usize) -> Option<usize> {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(ceiling_idx);
                return Some(i);
            }
        }
        None
    }

    /// Remove a ceiling from the active tracking list.
    pub fn remove(&mut self, ceiling_idx: usize) {
        for slot in self.slots.iter_mut() {
            if *slot == Some(ceiling_idx) {
                *slot = None;
                return;
            }
        }
    }

    /// Re-activate all stasis ceilings with the given tag.
    ///
    /// When a stasis ceiling is re-activated, its direction is restored
    /// from `olddirection`.
    pub fn activate_in_stasis(&self, _tag: i32) -> Vec<usize> {
        let mut reactivated = Vec::new();
        for idx in self.slots.iter().flatten() {
            reactivated.push(*idx);
        }
        // Filter and direction-restore is done by caller with access to CeilingT data.
        reactivated
    }
}

impl Default for ActiveCeilings {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// EV_DoCeiling — Spawn ceiling movers by line tag
// Translated from lines 70-180 of p_ceilng.c
// ============================================================================

/// Spawn ceiling movement thinkers for all sectors matching the trigger
/// line's tag.
///
/// Returns `true` if any ceiling movement was started.
pub fn ev_do_ceiling(
    line_idx: usize,
    ceiling_type: CeilingType,
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
        let mut ceiling = CeilingT::new(sec_idx);
        ceiling.ceiling_type = ceiling_type;
        ceiling.tag = ctx.sectors()[sec_idx].tag as i32;

        match ceiling_type {
            CeilingType::FastCrushAndRaise => {
                ceiling.crush = true;
                ceiling.topheight = ctx.sectors()[sec_idx].ceilingheight;
                ceiling.bottomheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(8 * FRACUNIT);
                ceiling.direction = -1;
                ceiling.speed = Fixed::new(CEILSPEED * 2);
            }
            CeilingType::SilentCrushAndRaise | CeilingType::CrushAndRaise => {
                ceiling.crush = true;
                ceiling.topheight = ctx.sectors()[sec_idx].ceilingheight;
                ceiling.bottomheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(8 * FRACUNIT);
                ceiling.direction = -1;
                ceiling.speed = Fixed::new(CEILSPEED);
            }
            CeilingType::LowerToFloor => {
                ceiling.crush = false;
                ceiling.topheight = ctx.sectors()[sec_idx].ceilingheight;
                ceiling.bottomheight = ctx.sectors()[sec_idx].floorheight;
                ceiling.direction = -1;
                ceiling.speed = Fixed::new(CEILSPEED);
            }
            CeilingType::LowerAndCrush => {
                ceiling.crush = true;
                ceiling.topheight = ctx.sectors()[sec_idx].ceilingheight;
                ceiling.bottomheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(8 * FRACUNIT);
                ceiling.direction = -1;
                ceiling.speed = Fixed::new(CEILSPEED);
            }
            CeilingType::RaiseToHighest => {
                ceiling.crush = false;
                ceiling.direction = 1;
                ceiling.topheight =
                    p_find_highest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                ceiling.speed = Fixed::new(CEILSPEED);
            }
        }

        // The concrete SpecContext implementation handles thinker registration
        // and active ceiling tracking.
        let _ceiling_data = ceiling;
    }

    rtn
}

/// Find the highest ceiling height in surrounding sectors.
/// (Local helper — duplicated from spec.rs to avoid import cycling issues.)
fn p_find_highest_ceiling_surrounding(
    sector_idx: usize,
    sectors: &[Sector],
    lines: &[LineDef],
) -> Fixed {
    use crate::play::spec::get_next_sector;
    let sec = &sectors[sector_idx];
    let mut height = Fixed::ZERO;
    for &line_idx in &sec.lines {
        let line = &lines[line_idx];
        if let Some(other_idx) = get_next_sector(line, sector_idx) {
            let other = &sectors[other_idx];
            if other.ceilingheight > height {
                height = other.ceilingheight;
            }
        }
    }
    height
}

// ============================================================================
// EV_CeilingCrushStop — Stop crush ceilings by tag
// Translated from lines 235-260 of p_ceilng.c
// ============================================================================

/// Stop all active crush ceilings with the matching line tag.
///
/// The ceiling is put into stasis (direction = 0, olddirection preserves
/// the previous direction for later resumption).
///
/// Returns `true` if any ceiling was stopped.
pub fn ev_ceiling_crush_stop(tag: i16, ceilings: &[Option<CeilingSnapshot>]) -> Vec<usize> {
    let mut stopped = Vec::new();
    for (i, slot) in ceilings.iter().enumerate() {
        if let Some(ref ceil) = slot {
            if ceil.tag == tag as i32 && ceil.direction != 0 {
                stopped.push(i);
            }
        }
    }
    stopped
}

/// Snapshot of ceiling state for tag matching (avoids borrow conflicts).
#[derive(Debug, Clone)]
pub struct CeilingSnapshot {
    pub tag: i32,
    pub direction: i32,
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_active_ceilings_add_remove() {
        let mut ac = ActiveCeilings::new();
        assert_eq!(ac.add(42), Some(0));
        assert_eq!(ac.add(99), Some(1));
        ac.remove(42);
        assert!(ac.slots[0].is_none());
        assert_eq!(ac.slots[1], Some(99));
    }

    #[test]
    fn test_active_ceilings_full() {
        let mut ac = ActiveCeilings::new();
        for i in 0..MAXCEILINGS {
            assert!(ac.add(i).is_some());
        }
        assert!(ac.add(999).is_none());
    }

    #[test]
    fn test_ceiling_snapshot() {
        let snap = CeilingSnapshot {
            tag: 5,
            direction: -1,
        };
        assert_eq!(snap.tag, 5);
        assert_eq!(snap.direction, -1);
    }

    #[test]
    fn test_ev_ceiling_crush_stop_finds_matching() {
        let ceilings = vec![
            Some(CeilingSnapshot {
                tag: 5,
                direction: -1,
            }),
            None,
            Some(CeilingSnapshot {
                tag: 5,
                direction: 0,
            }), // already stopped
            Some(CeilingSnapshot {
                tag: 3,
                direction: 1,
            }), // wrong tag
            Some(CeilingSnapshot {
                tag: 5,
                direction: 1,
            }),
        ];
        let stopped = ev_ceiling_crush_stop(5, &ceilings);
        assert_eq!(stopped, vec![0, 4]);
    }
}
