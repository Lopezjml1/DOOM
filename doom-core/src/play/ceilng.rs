// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Ceiling movement thinkers — crush ceilings, raise/lower ceilings.
//!
//! Translated from linuxdoom-1.10/p_ceilng.c
//!
//! # Overview
//!
//! This module provides the ceiling movement thinker (`T_MoveCeiling`), the
//! event dispatch function (`EV_DoCeiling`), ceiling crush stop
//! (`EV_CeilingCrushStop`), and the active ceiling management utilities
//! (`P_AddActiveCeiling`, `P_RemoveActiveCeiling`,
//! `P_ActivateInStasisCeiling`).
//!
//! # Original C functions → Rust mapping
//!
//! | C function                 | Rust function                    | Description                        |
//! |----------------------------|----------------------------------|------------------------------------|
//! | `T_MoveCeiling`            | [`t_move_ceiling`]               | Ceiling movement thinker callback  |
//! | `EV_DoCeiling`             | [`ev_do_ceiling`]                | Spawn ceiling movers by line tag   |
//! | `P_AddActiveCeiling`       | [`p_add_active_ceiling`]         | Track active ceiling               |
//! | `P_RemoveActiveCeiling`    | [`p_remove_active_ceiling`]      | Remove active ceiling              |
//! | `P_ActivateInStasisCeiling` | [`p_activate_in_stasis_ceiling`] | Resume stasis ceiling              |
//! | `EV_CeilingCrushStop`      | [`ev_ceiling_crush_stop`]        | Stop a crush ceiling               |

use crate::info::sounds::SfxEnum;
use crate::play::floor::t_move_plane;
use crate::play::spec::{
    p_find_highest_ceiling_surrounding, p_find_sector_from_line_tag, CeilingT, CeilingType,
    ResultE, SpecContext,
};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::Sector;
use crate::types::thinker::ActionFn;

// ============================================================================
// Constants
// ============================================================================

/// Maximum number of simultaneously active ceiling movers.
///
/// Original C: `#define MAXCEILINGS 30` (p_spec.h line 155).
/// Slots that are `None` represent free entries.
pub const MAXCEILINGS: usize = 30;

/// Default ceiling movement speed (1.0 in 16.16 fixed-point).
///
/// Original C: `#define CEILSPEED FRACUNIT` (p_spec.h line 152).
/// Equals one map unit per tic.  Special variants use multiples:
/// - Fast crush: `CEILSPEED * 2`
/// - Crush slowdown: `CEILSPEED / 8`
pub const CEILSPEED: Fixed = Fixed(FRACUNIT);

// ============================================================================
// Active ceiling tracking
// ============================================================================

/// Tracks active ceiling thinkers for stasis/resume and crush-stop
/// operations.
///
/// Original C: `ceiling_t* activeceilings[MAXCEILINGS]` global array
/// (p_ceilng.c line 35).
///
/// Each slot holds `Some(data_index)` when a ceiling mover is active, or
/// `None` when the slot is free.  The `data_index` refers to the index
/// into the game-state's ceiling data storage (a `Vec<CeilingT>`).
#[derive(Debug)]
pub struct ActiveCeilings {
    /// Arena indices of active ceiling thinkers (`None` = empty slot).
    pub slots: [Option<usize>; MAXCEILINGS],
}

impl ActiveCeilings {
    /// Create an empty active ceilings tracker (all slots `None`).
    pub fn new() -> Self {
        Self {
            slots: [None; MAXCEILINGS],
        }
    }

    /// Add a ceiling to the active tracking list.
    ///
    /// Returns the slot index if added, `None` if the array is full.
    /// The original C silently ignores the request when no slot is
    /// available (there is no `I_Error` call).
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
    ///
    /// Finds the first slot matching `ceiling_idx` and clears it.
    /// If the index is not found, the call is a no-op.
    pub fn remove(&mut self, ceiling_idx: usize) {
        for slot in self.slots.iter_mut() {
            if *slot == Some(ceiling_idx) {
                *slot = None;
                return;
            }
        }
    }
}

impl Default for ActiveCeilings {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// T_MoveCeiling — Ceiling movement thinker callback
// Translated from linuxdoom-1.10/p_ceilng.c lines 42-164
// ============================================================================

/// Advance a ceiling movement thinker by one tic.
///
/// This is the per-tic callback for all ceiling movers.  It calls
/// [`t_move_plane`] to perform the actual sector plane movement and then
/// decides what to do based on the result:
///
/// - **Up movement** that reaches `pastdest`: `RaiseToHighest` is removed;
///   crush variants reverse direction downward.
/// - **Down movement** that reaches `pastdest`: crush variants reverse
///   direction upward (resetting speed for non-fast crushers);
///   `LowerAndCrush` and `LowerToFloor` are removed.
/// - **Down movement** that reports `Crushed` (something blocked): crush
///   types slow down to `CEILSPEED / 8` (except `FastCrushAndRaise` which
///   keeps its original speed).
///
/// Sound `sfx_stnmov` is played every 8 tics during movement, except for
/// `SilentCrushAndRaise` which produces no movement sound.  The stop sound
/// `sfx_pstop` is played when `SilentCrushAndRaise` reverses direction.
///
/// # Parameters
///
/// * `ceiling`          — mutable reference to the ceiling thinker data
/// * `sector`           — mutable reference to the sector being moved
/// * `leveltime`        — current game tic count (for 8-tic sound cadence)
/// * `change_sector_fn` — equivalent of `P_ChangeSector(sec, crush)`.
///   Takes `crush: bool`, returns `true` if something doesn't fit
/// * `sound_fn`         — plays a sound effect at the sector's sound origin
///
/// # Returns
///
/// `true` if the ceiling mover has completed and should be removed
/// from the thinker list and active ceiling tracker, `false` if it
/// should continue running.
pub fn t_move_ceiling(
    ceiling: &mut CeilingT,
    sector: &mut Sector,
    leveltime: i32,
    change_sector_fn: &mut dyn FnMut(bool) -> bool,
    sound_fn: &mut dyn FnMut(SfxEnum),
) -> bool {
    match ceiling.direction {
        // ----------------------------------------------------------------
        // Direction 0 — IN STASIS (paused by EV_CeilingCrushStop)
        // Original C: case 0: break;
        // ----------------------------------------------------------------
        0 => false,

        // ----------------------------------------------------------------
        // Direction 1 — MOVING UP
        // Original C: p_ceilng.c lines 57-99
        // ----------------------------------------------------------------
        1 => {
            let res = t_move_plane(
                sector,
                ceiling.speed,
                ceiling.topheight,
                false, // crush = false for upward movement
                1,     // floor_or_ceiling = 1 (ceiling)
                ceiling.direction,
                change_sector_fn,
            );

            // Play stone-movement sound every 8 tics.
            // Original C: if (!(leveltime&7))
            // Exception: silentCrushAndRaise produces no movement sound.
            if (leveltime & 7) == 0 && ceiling.ceiling_type != CeilingType::SilentCrushAndRaise {
                sound_fn(SfxEnum::sfx_stnmov);
            }

            if res == ResultE::PastDest {
                match ceiling.ceiling_type {
                    // raiseToHighest: destination reached — remove thinker.
                    CeilingType::RaiseToHighest => {
                        return true;
                    }

                    // silentCrushAndRaise: play stop sound, then reverse.
                    // C fall-through: silentCrushAndRaise → fastCrushAndRaise
                    //                                     → crushAndRaise
                    CeilingType::SilentCrushAndRaise => {
                        sound_fn(SfxEnum::sfx_pstop);
                        // Fall through — reverse direction (same as crush types)
                        ceiling.direction = -1;
                    }

                    // fastCrushAndRaise / crushAndRaise: reverse to downward.
                    CeilingType::FastCrushAndRaise | CeilingType::CrushAndRaise => {
                        ceiling.direction = -1;
                    }

                    // Default: do nothing for other types.
                    _ => {}
                }
            }

            false
        }

        // ----------------------------------------------------------------
        // Direction -1 — MOVING DOWN
        // Original C: p_ceilng.c lines 101-163
        // ----------------------------------------------------------------
        -1 => {
            let res = t_move_plane(
                sector,
                ceiling.speed,
                ceiling.bottomheight,
                ceiling.crush, // use ceiling's crush flag for downward
                1,             // floor_or_ceiling = 1 (ceiling)
                ceiling.direction,
                change_sector_fn,
            );

            // Play stone-movement sound every 8 tics.
            // Exception: silentCrushAndRaise produces no movement sound.
            if (leveltime & 7) == 0 && ceiling.ceiling_type != CeilingType::SilentCrushAndRaise {
                sound_fn(SfxEnum::sfx_stnmov);
            }

            if res == ResultE::PastDest {
                // Destination reached while moving down.
                match ceiling.ceiling_type {
                    // silentCrushAndRaise: play stop sound, reset speed,
                    // reverse to upward.
                    // C fall-through:
                    //   silentCrushAndRaise → sfx_pstop, then fall to
                    //   crushAndRaise      → speed = CEILSPEED, then fall to
                    //   fastCrushAndRaise   → direction = 1
                    CeilingType::SilentCrushAndRaise => {
                        sound_fn(SfxEnum::sfx_pstop);
                        ceiling.speed = CEILSPEED;
                        ceiling.direction = 1;
                    }

                    // crushAndRaise: reset speed, reverse to upward.
                    CeilingType::CrushAndRaise => {
                        ceiling.speed = CEILSPEED;
                        ceiling.direction = 1;
                    }

                    // fastCrushAndRaise: reverse to upward (keep fast speed).
                    CeilingType::FastCrushAndRaise => {
                        ceiling.direction = 1;
                    }

                    // lowerAndCrush / lowerToFloor: destination reached — remove.
                    CeilingType::LowerAndCrush | CeilingType::LowerToFloor => {
                        return true;
                    }

                    // Default: do nothing.
                    _ => {}
                }
            } else if res == ResultE::Crushed {
                // Something was crushed while moving down.
                // Slow down crush speed to CEILSPEED / 8 for most crush types.
                // FastCrushAndRaise does NOT slow down (stays at fast speed).
                match ceiling.ceiling_type {
                    CeilingType::SilentCrushAndRaise
                    | CeilingType::CrushAndRaise
                    | CeilingType::LowerAndCrush => {
                        ceiling.speed = Fixed(FRACUNIT / 8);
                    }
                    _ => {
                        // fastCrushAndRaise and others: no speed change.
                    }
                }
            }

            false
        }

        // Direction outside [-1, 0, 1] — should never happen; treat as no-op.
        _ => false,
    }
}

// ============================================================================
// EV_DoCeiling — Spawn ceiling movers by line tag
// Translated from linuxdoom-1.10/p_ceilng.c lines 171-250
// ============================================================================

/// Spawn ceiling movement thinkers for all sectors matching the trigger
/// line's tag.
///
/// Returns `true` if any ceiling movement was started.
///
/// For crush-type ceilings (`FastCrushAndRaise`, `SilentCrushAndRaise`,
/// `CrushAndRaise`), any previously stopped (stasis) ceilings with a
/// matching tag are re-activated before spawning new ones.
///
/// The `ceiling_type` parameter determines the movement behaviour:
///
/// | Type                  | Direction | Speed        | Crush | Target heights                                  |
/// |-----------------------|-----------|-------------|-------|------------------------------------------------|
/// | `FastCrushAndRaise`   | Down      | CEILSPEED×2 | Yes   | top = sector ceiling, bottom = sector floor + 8 |
/// | `SilentCrushAndRaise` | Down      | CEILSPEED   | Yes   | top = sector ceiling, bottom = sector floor + 8 |
/// | `CrushAndRaise`       | Down      | CEILSPEED   | Yes   | top = sector ceiling, bottom = sector floor + 8 |
/// | `LowerAndCrush`       | Down      | CEILSPEED   | No    | bottom = sector floor + 8                        |
/// | `LowerToFloor`        | Down      | CEILSPEED   | No    | bottom = sector floor                            |
/// | `RaiseToHighest`      | Up        | CEILSPEED   | No    | top = highest surrounding ceiling                |
///
/// # Parameters
///
/// * `line_idx`     — index of the trigger linedef (used for tag matching)
/// * `ceiling_type` — the type of ceiling movement to create
/// * `ctx`          — game context providing sector/line/thinker access
pub fn ev_do_ceiling(
    line_idx: usize,
    ceiling_type: CeilingType,
    ctx: &mut dyn SpecContext,
) -> bool {
    let mut rtn = false;
    let mut secnum: i32 = -1;

    // ---- Reactivate in-stasis ceilings for crush types ----
    // Original C (p_ceilng.c lines 185-193):
    //   switch(type) {
    //     case fastCrushAndRaise:
    //     case silentCrushAndRaise:
    //     case crushAndRaise:
    //       P_ActivateInStasisCeiling(line);
    //     default: break;
    //   }
    match ceiling_type {
        CeilingType::FastCrushAndRaise
        | CeilingType::SilentCrushAndRaise
        | CeilingType::CrushAndRaise => {
            let tag = ctx.lines()[line_idx].tag as i32;
            ctx.p_activate_in_stasis_ceiling(tag);
        }
        _ => {}
    }

    // ---- Iterate all sectors matching the trigger line's tag ----
    loop {
        secnum = p_find_sector_from_line_tag(&ctx.lines()[line_idx], secnum, ctx.sectors());
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // Skip if sector already has an active mover.
        // Original C: if (sec->specialdata) continue;
        if ctx.sectors()[sec_idx].specialdata.is_some() {
            continue;
        }

        // A new ceiling thinker will be created.
        rtn = true;

        let mut ceiling = CeilingT::new(sec_idx);
        ceiling.thinker.function = ActionFn::MoveCeiling;
        ceiling.crush = false;

        // ---- Configure ceiling based on type ----
        // The original C uses fall-through in the switch statement. The
        // Rust translation replicates the resulting field assignments for
        // each variant explicitly.
        match ceiling_type {
            // fastCrushAndRaise: crush = true, 2× speed, starts going down.
            // Separate case in C (does not fall through).
            CeilingType::FastCrushAndRaise => {
                ceiling.crush = true;
                ceiling.topheight = ctx.sectors()[sec_idx].ceilingheight;
                ceiling.bottomheight = ctx.sectors()[sec_idx].floorheight + Fixed(8 * FRACUNIT);
                ceiling.direction = -1;
                ceiling.speed = Fixed(CEILSPEED.0 * 2);
            }

            // silentCrushAndRaise / crushAndRaise:
            //   crush = true, topheight = ceilingheight
            //   THEN fall-through to lowerAndCrush/lowerToFloor:
            //     bottomheight = floorheight + 8 (since type != lowerToFloor)
            //     direction = -1, speed = CEILSPEED
            CeilingType::SilentCrushAndRaise | CeilingType::CrushAndRaise => {
                ceiling.crush = true;
                ceiling.topheight = ctx.sectors()[sec_idx].ceilingheight;
                ceiling.bottomheight = ctx.sectors()[sec_idx].floorheight + Fixed(8 * FRACUNIT);
                ceiling.direction = -1;
                ceiling.speed = CEILSPEED;
            }

            // lowerAndCrush:
            //   crush stays false (not set by C code)
            //   bottomheight = floorheight + 8 (since type != lowerToFloor)
            //   direction = -1, speed = CEILSPEED
            CeilingType::LowerAndCrush => {
                ceiling.bottomheight = ctx.sectors()[sec_idx].floorheight + Fixed(8 * FRACUNIT);
                ceiling.direction = -1;
                ceiling.speed = CEILSPEED;
            }

            // lowerToFloor:
            //   crush stays false
            //   bottomheight = floorheight (NO +8 — type IS lowerToFloor)
            //   direction = -1, speed = CEILSPEED
            CeilingType::LowerToFloor => {
                ceiling.bottomheight = ctx.sectors()[sec_idx].floorheight;
                ceiling.direction = -1;
                ceiling.speed = CEILSPEED;
            }

            // raiseToHighest:
            //   topheight = highest ceiling in surrounding sectors
            //   direction = 1 (up), speed = CEILSPEED
            CeilingType::RaiseToHighest => {
                ceiling.topheight =
                    p_find_highest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                ceiling.direction = 1;
                ceiling.speed = CEILSPEED;
            }
        }

        // Set tag and type AFTER the type-specific configuration.
        // Original C (p_ceilng.c lines 240-241):
        //   ceiling->tag = sec->tag;
        //   ceiling->type = type;
        ceiling.tag = ctx.sectors()[sec_idx].tag as i32;
        ceiling.ceiling_type = ceiling_type;

        // Register the ceiling thinker with the game state.
        // The SpecContext implementation is responsible for:
        //   1. Storing the CeilingT data
        //   2. Adding a thinker entry (ActionFn::MoveCeiling)
        //   3. Setting sector.specialdata to the data index
        //   4. Calling p_add_active_ceiling to track it
        ctx.p_add_thinker_ceiling(ceiling);
    }

    rtn
}

// ============================================================================
// P_AddActiveCeiling — Track active ceiling
// Translated from linuxdoom-1.10/p_ceilng.c lines 251-263
// ============================================================================

/// Add a ceiling mover to the active ceiling tracking array.
///
/// Finds the first empty slot and stores `ceiling_idx`.  If the array is
/// full (all `MAXCEILINGS` slots occupied), the request is silently
/// ignored — matching the original C behavior which simply returns without
/// calling `I_Error`.
///
/// # Parameters
///
/// * `active`      — mutable reference to the active ceilings tracker
/// * `ceiling_idx` — data index of the ceiling mover to track
pub fn p_add_active_ceiling(active: &mut ActiveCeilings, ceiling_idx: usize) {
    if active.add(ceiling_idx).is_none() {
        // Original C silently fails when no slot is available.
        // Log a warning for diagnostics but do not panic.
        tracing::warn!(
            "P_AddActiveCeiling: no more ceiling slots (MAXCEILINGS = {})",
            MAXCEILINGS
        );
    }
}

// ============================================================================
// P_RemoveActiveCeiling — Remove active ceiling
// Translated from linuxdoom-1.10/p_ceilng.c lines 270-284
// ============================================================================

/// Remove a ceiling mover from the active tracking array and clean up.
///
/// Performs three operations matching the original C:
/// 1. Clears `sector.specialdata` to `None`
/// 2. Marks the thinker for pending removal (`ActionFn::PendingRemoval`)
/// 3. Removes the slot from the active ceilings tracking array
///
/// The thinker list garbage-collection pass will later reclaim the entry
/// marked `PendingRemoval`.
///
/// # Parameters
///
/// * `ceiling_idx` — data index of the ceiling mover to remove
/// * `ceilings`    — mutable slice of all ceiling thinker data
/// * `sectors`     — mutable slice of all sectors
/// * `active`      — mutable reference to the active ceilings tracker
pub fn p_remove_active_ceiling(
    ceiling_idx: usize,
    ceilings: &mut [CeilingT],
    sectors: &mut [Sector],
    active: &mut ActiveCeilings,
) {
    let sec_idx = ceilings[ceiling_idx].sector;
    // Original C: activeceilings[i]->sector->specialdata = NULL;
    sectors[sec_idx].specialdata = None;
    // Original C: P_RemoveThinker (&activeceilings[i]->thinker);
    ceilings[ceiling_idx].thinker.function = ActionFn::PendingRemoval;
    // Original C: activeceilings[i] = NULL;
    active.remove(ceiling_idx);
}

// ============================================================================
// P_ActivateInStasisCeiling — Resume stasis ceilings
// Translated from linuxdoom-1.10/p_ceilng.c lines 291-306
// ============================================================================

/// Re-activate all ceiling movers in stasis whose tag matches the given tag.
///
/// A ceiling is in stasis when `direction == 0` (set by
/// [`ev_ceiling_crush_stop`]).  Re-activation restores the previously saved
/// `olddirection` and sets the thinker action back to
/// [`ActionFn::MoveCeiling`] so it resumes movement on the next tic.
///
/// Called at the start of [`ev_do_ceiling`] for crush-type ceilings to
/// resume any paused crushers before spawning new ones.
///
/// # Parameters
///
/// * `tag`      — tag value to match against ceiling tags
/// * `ceilings` — mutable slice of all ceiling thinker data
/// * `active`   — the active ceilings tracker (read-only — used for iteration)
pub fn p_activate_in_stasis_ceiling(tag: i32, ceilings: &mut [CeilingT], active: &ActiveCeilings) {
    for &slot in active.slots.iter() {
        if let Some(ceil_idx) = slot {
            if ceil_idx < ceilings.len()
                && ceilings[ceil_idx].tag == tag
                && ceilings[ceil_idx].direction == 0
            {
                // Restore the saved direction.
                ceilings[ceil_idx].direction = ceilings[ceil_idx].olddirection;
                // Re-enable the thinker callback.
                ceilings[ceil_idx].thinker.function = ActionFn::MoveCeiling;
            }
        }
    }
}

// ============================================================================
// EV_CeilingCrushStop — Stop crush ceilings by tag
// Translated from linuxdoom-1.10/p_ceilng.c lines 314-335
// ============================================================================

/// Stop all active crush ceilings whose tag matches the given tag.
///
/// For each matching active ceiling that is currently moving
/// (`direction != 0`):
/// 1. Saves the current `direction` into `olddirection`
/// 2. Sets `direction` to 0 (stasis)
/// 3. Clears the thinker action to [`ActionFn::None`] to pause execution
///
/// The ceiling can later be resumed by [`p_activate_in_stasis_ceiling`]
/// when a matching crush trigger fires again.
///
/// Returns `true` if any ceiling was stopped (matching original C return
/// value).
///
/// # Parameters
///
/// * `tag`      — tag value from the trigger linedef
/// * `ceilings` — mutable slice of all ceiling thinker data
/// * `active`   — the active ceilings tracker (read-only — used for iteration)
pub fn ev_ceiling_crush_stop(tag: i32, ceilings: &mut [CeilingT], active: &ActiveCeilings) -> bool {
    let mut rtn = false;
    for &slot in active.slots.iter() {
        if let Some(ceil_idx) = slot {
            if ceil_idx < ceilings.len()
                && ceilings[ceil_idx].tag == tag
                && ceilings[ceil_idx].direction != 0
            {
                // Save current direction for later resumption.
                ceilings[ceil_idx].olddirection = ceilings[ceil_idx].direction;
                // Deactivate the thinker callback.
                // Original C: activeceilings[i]->thinker.function.acp1 = NULL;
                ceilings[ceil_idx].thinker.function = ActionFn::None;
                // Put the ceiling into stasis.
                ceilings[ceil_idx].direction = 0;
                rtn = true;
            }
        }
    }
    rtn
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::spec::CeilingType;
    use crate::types::fixed::FRACUNIT;
    use crate::types::thinker::{ActionFn, Thinker};

    // ---- Constants ----

    #[test]
    fn test_maxceilings_value() {
        assert_eq!(MAXCEILINGS, 30);
    }

    #[test]
    fn test_ceilspeed_value() {
        assert_eq!(CEILSPEED, Fixed(FRACUNIT));
        assert_eq!(CEILSPEED.0, 65536);
    }

    // ---- ActiveCeilings ----

    #[test]
    fn test_active_ceilings_new() {
        let ac = ActiveCeilings::new();
        for slot in ac.slots.iter() {
            assert!(slot.is_none());
        }
    }

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
    fn test_active_ceilings_add_fills_first_free_slot() {
        let mut ac = ActiveCeilings::new();
        ac.add(10);
        ac.add(20);
        ac.add(30);
        ac.remove(20); // frees slot 1
        assert_eq!(ac.add(40), Some(1)); // should reuse slot 1
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
    fn test_active_ceilings_remove_nonexistent() {
        let mut ac = ActiveCeilings::new();
        ac.add(10);
        ac.remove(999); // should be a no-op
        assert_eq!(ac.slots[0], Some(10));
    }

    #[test]
    fn test_active_ceilings_default() {
        let ac = ActiveCeilings::default();
        for slot in ac.slots.iter() {
            assert!(slot.is_none());
        }
    }

    // ---- p_add_active_ceiling ----

    #[test]
    fn test_p_add_active_ceiling_success() {
        let mut ac = ActiveCeilings::new();
        p_add_active_ceiling(&mut ac, 5);
        assert_eq!(ac.slots[0], Some(5));
    }

    #[test]
    fn test_p_add_active_ceiling_full_no_panic() {
        let mut ac = ActiveCeilings::new();
        for i in 0..MAXCEILINGS {
            p_add_active_ceiling(&mut ac, i);
        }
        // This should NOT panic — just warn.
        p_add_active_ceiling(&mut ac, 999);
    }

    // ---- p_remove_active_ceiling ----

    #[test]
    fn test_p_remove_active_ceiling() {
        let mut ac = ActiveCeilings::new();
        let mut sectors = vec![make_test_sector()];
        let mut ceilings = vec![make_test_ceiling_t(0)];

        sectors[0].specialdata = Some(42);
        ac.add(0);

        p_remove_active_ceiling(0, &mut ceilings, &mut sectors, &mut ac);

        assert!(sectors[0].specialdata.is_none());
        assert_eq!(ceilings[0].thinker.function, ActionFn::PendingRemoval);
        assert!(ac.slots[0].is_none());
    }

    // ---- p_activate_in_stasis_ceiling ----

    #[test]
    fn test_p_activate_in_stasis_ceiling_matching() {
        let mut ac = ActiveCeilings::new();
        let mut ceilings = vec![make_test_ceiling_t(0), make_test_ceiling_t(0)];

        // Ceiling 0: tag=5, direction=0 (in stasis), olddirection=-1
        ceilings[0].tag = 5;
        ceilings[0].direction = 0;
        ceilings[0].olddirection = -1;
        ceilings[0].thinker.function = ActionFn::None;

        // Ceiling 1: tag=5, direction=1 (NOT in stasis)
        ceilings[1].tag = 5;
        ceilings[1].direction = 1;
        ceilings[1].thinker.function = ActionFn::MoveCeiling;

        ac.add(0);
        ac.add(1);

        p_activate_in_stasis_ceiling(5, &mut ceilings, &ac);

        // Ceiling 0 should be re-activated.
        assert_eq!(ceilings[0].direction, -1);
        assert_eq!(ceilings[0].thinker.function, ActionFn::MoveCeiling);

        // Ceiling 1 should be unchanged (was not in stasis).
        assert_eq!(ceilings[1].direction, 1);
    }

    #[test]
    fn test_p_activate_in_stasis_ceiling_wrong_tag() {
        let mut ac = ActiveCeilings::new();
        let mut ceilings = vec![make_test_ceiling_t(0)];

        ceilings[0].tag = 3;
        ceilings[0].direction = 0;
        ceilings[0].olddirection = -1;
        ceilings[0].thinker.function = ActionFn::None;
        ac.add(0);

        p_activate_in_stasis_ceiling(5, &mut ceilings, &ac);

        // Should NOT be re-activated (wrong tag).
        assert_eq!(ceilings[0].direction, 0);
        assert_eq!(ceilings[0].thinker.function, ActionFn::None);
    }

    // ---- ev_ceiling_crush_stop ----

    #[test]
    fn test_ev_ceiling_crush_stop_matching() {
        let mut ac = ActiveCeilings::new();
        let mut ceilings = vec![make_test_ceiling_t(0)];

        ceilings[0].tag = 7;
        ceilings[0].direction = -1;
        ceilings[0].thinker.function = ActionFn::MoveCeiling;
        ac.add(0);

        let result = ev_ceiling_crush_stop(7, &mut ceilings, &ac);

        assert!(result);
        assert_eq!(ceilings[0].olddirection, -1);
        assert_eq!(ceilings[0].direction, 0);
        assert_eq!(ceilings[0].thinker.function, ActionFn::None);
    }

    #[test]
    fn test_ev_ceiling_crush_stop_already_stopped() {
        let mut ac = ActiveCeilings::new();
        let mut ceilings = vec![make_test_ceiling_t(0)];

        ceilings[0].tag = 7;
        ceilings[0].direction = 0; // already in stasis
        ac.add(0);

        let result = ev_ceiling_crush_stop(7, &mut ceilings, &ac);

        assert!(!result);
        assert_eq!(ceilings[0].direction, 0);
    }

    #[test]
    fn test_ev_ceiling_crush_stop_wrong_tag() {
        let mut ac = ActiveCeilings::new();
        let mut ceilings = vec![make_test_ceiling_t(0)];

        ceilings[0].tag = 7;
        ceilings[0].direction = -1;
        ac.add(0);

        let result = ev_ceiling_crush_stop(999, &mut ceilings, &ac);

        assert!(!result);
        assert_eq!(ceilings[0].direction, -1);
    }

    // ---- t_move_ceiling ----

    #[test]
    fn test_t_move_ceiling_stasis_does_nothing() {
        let mut ceiling = make_test_ceiling_t(0);
        ceiling.direction = 0; // stasis
        let mut sector = make_test_sector();

        let remove = t_move_ceiling(&mut ceiling, &mut sector, 0, &mut |_| false, &mut |_| {});

        assert!(!remove);
        assert_eq!(ceiling.direction, 0);
    }

    #[test]
    fn test_t_move_ceiling_raise_to_highest_removes_on_pastdest() {
        let mut ceiling = make_test_ceiling_t(0);
        ceiling.ceiling_type = CeilingType::RaiseToHighest;
        ceiling.direction = 1;
        ceiling.topheight = Fixed(100 * FRACUNIT);
        let mut sector = make_test_sector();
        // Set ceilingheight to match topheight so t_move_plane returns PastDest
        sector.ceilingheight = Fixed(100 * FRACUNIT);

        let remove = t_move_ceiling(&mut ceiling, &mut sector, 0, &mut |_| false, &mut |_| {});

        assert!(remove);
    }

    #[test]
    fn test_t_move_ceiling_crush_and_raise_reverses_on_up_pastdest() {
        let mut ceiling = make_test_ceiling_t(0);
        ceiling.ceiling_type = CeilingType::CrushAndRaise;
        ceiling.direction = 1;
        ceiling.topheight = Fixed(100 * FRACUNIT);
        let mut sector = make_test_sector();
        sector.ceilingheight = Fixed(100 * FRACUNIT);

        let remove = t_move_ceiling(&mut ceiling, &mut sector, 0, &mut |_| false, &mut |_| {});

        assert!(!remove);
        assert_eq!(ceiling.direction, -1);
    }

    #[test]
    fn test_t_move_ceiling_silent_crush_plays_pstop_on_up_pastdest() {
        let mut ceiling = make_test_ceiling_t(0);
        ceiling.ceiling_type = CeilingType::SilentCrushAndRaise;
        ceiling.direction = 1;
        ceiling.topheight = Fixed(100 * FRACUNIT);
        let mut sector = make_test_sector();
        sector.ceilingheight = Fixed(100 * FRACUNIT);

        let mut sounds_played = Vec::new();
        let remove = t_move_ceiling(&mut ceiling, &mut sector, 0, &mut |_| false, &mut |sfx| {
            sounds_played.push(sfx)
        });

        assert!(!remove);
        assert_eq!(ceiling.direction, -1);
        assert!(sounds_played.contains(&SfxEnum::sfx_pstop));
    }

    #[test]
    fn test_t_move_ceiling_lower_to_floor_removes_on_pastdest() {
        let mut ceiling = make_test_ceiling_t(0);
        ceiling.ceiling_type = CeilingType::LowerToFloor;
        ceiling.direction = -1;
        ceiling.bottomheight = Fixed(0);
        let mut sector = make_test_sector();
        sector.ceilingheight = Fixed(0); // at destination

        let remove = t_move_ceiling(&mut ceiling, &mut sector, 0, &mut |_| false, &mut |_| {});

        assert!(remove);
    }

    #[test]
    fn test_t_move_ceiling_sound_every_8_tics() {
        let mut ceiling = make_test_ceiling_t(0);
        ceiling.ceiling_type = CeilingType::CrushAndRaise;
        ceiling.direction = -1;
        ceiling.bottomheight = Fixed(-1000 * FRACUNIT);
        ceiling.speed = CEILSPEED;
        ceiling.crush = true;
        let mut sector = make_test_sector();
        sector.ceilingheight = Fixed(100 * FRACUNIT);

        // Test at leveltime=0 (should play sound: 0 & 7 == 0)
        let mut sound_count_0 = 0;
        t_move_ceiling(&mut ceiling, &mut sector, 0, &mut |_| false, &mut |_| {
            sound_count_0 += 1
        });
        assert_eq!(sound_count_0, 1);

        // Test at leveltime=3 (should NOT play sound: 3 & 7 != 0)
        let mut sound_count_3 = 0;
        t_move_ceiling(&mut ceiling, &mut sector, 3, &mut |_| false, &mut |_| {
            sound_count_3 += 1
        });
        assert_eq!(sound_count_3, 0);

        // Test at leveltime=8 (should play sound: 8 & 7 == 0)
        let mut sound_count_8 = 0;
        t_move_ceiling(&mut ceiling, &mut sector, 8, &mut |_| false, &mut |_| {
            sound_count_8 += 1
        });
        assert_eq!(sound_count_8, 1);
    }

    #[test]
    fn test_t_move_ceiling_silent_crush_no_movement_sound() {
        let mut ceiling = make_test_ceiling_t(0);
        ceiling.ceiling_type = CeilingType::SilentCrushAndRaise;
        ceiling.direction = -1;
        ceiling.bottomheight = Fixed(-1000 * FRACUNIT);
        ceiling.speed = CEILSPEED;
        ceiling.crush = true;
        let mut sector = make_test_sector();
        sector.ceilingheight = Fixed(100 * FRACUNIT);

        // At leveltime=0, silentCrushAndRaise should NOT play sfx_stnmov
        let mut sounds = Vec::new();
        t_move_ceiling(&mut ceiling, &mut sector, 0, &mut |_| false, &mut |sfx| {
            sounds.push(sfx)
        });

        assert!(!sounds.contains(&SfxEnum::sfx_stnmov));
    }

    // ---- Test helpers ----

    /// Create a minimal test Sector for unit testing.
    fn make_test_sector() -> Sector {
        use crate::types::map_data::DegenMobj;
        use crate::types::thinker::Thinker as ThinkerNode;
        Sector {
            floorheight: Fixed::ZERO,
            ceilingheight: Fixed(128 * FRACUNIT),
            floorpic: 0,
            ceilingpic: 0,
            lightlevel: 255,
            special: 0,
            tag: 0,
            soundtraversed: 0,
            soundtarget: None,
            blockbox: [0; 4],
            soundorg: DegenMobj {
                thinker: ThinkerNode::default(),
                x: Fixed::ZERO,
                y: Fixed::ZERO,
                z: Fixed::ZERO,
            },
            validcount: 0,
            thinglist: None,
            specialdata: None,
            linecount: 0,
            lines: Vec::new(),
        }
    }

    /// Create a minimal test CeilingT for unit testing.
    fn make_test_ceiling_t(sector: usize) -> CeilingT {
        CeilingT {
            thinker: Thinker::new(ActionFn::MoveCeiling),
            ceiling_type: CeilingType::CrushAndRaise,
            sector,
            bottomheight: Fixed::ZERO,
            topheight: Fixed(128 * FRACUNIT),
            speed: CEILSPEED,
            crush: true,
            direction: -1,
            tag: 0,
            olddirection: 0,
        }
    }
}
