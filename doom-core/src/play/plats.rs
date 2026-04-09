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
//! | `T_PlatRaise` | [`t_plat_raise`] | Platform movement thinker callback |
//! | `EV_DoPlat` | [`ev_do_plat`] | Spawn platform movers by line tag |
//! | `P_AddActivePlat` | [`p_add_active_plat`] | Track active platform |
//! | `P_RemoveActivePlat` | [`p_remove_active_plat`] | Remove active platform |
//! | `EV_StopPlat` | [`ev_stop_plat`] | Stop platforms by tag |
//! | `P_ActivateInStasis` | [`p_activate_in_stasis`] | Resume stasis platforms |

use crate::info::sounds::SfxEnum;
use crate::play::floor::t_move_plane;
use crate::play::spec::{
    p_find_highest_floor_surrounding, p_find_lowest_floor_surrounding, p_find_next_highest_floor,
    p_find_sector_from_line_tag, PlatT, PlatType, ResultE, SpecContext, MAXPLATS, PLATSPEED,
    PLATWAIT,
};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::Sector;

// Re-export PlatStatus so that downstream code can import it from this module.
pub use crate::play::spec::PlatStatus;

// ============================================================================
// Active platform tracking
// ============================================================================

/// Tracks active platform thinkers for stasis/resume operations.
///
/// Original C: `activeplats[MAXPLATS]` global array.
pub struct ActivePlats {
    /// Indices of active platform thinker data (None = empty slot).
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
    /// Returns the slot index if added, `None` if all slots are full.
    ///
    /// Translated from `P_AddActivePlat` in p_plats.c lines 288-299.
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
    ///
    /// Translated from `P_RemoveActivePlat` in p_plats.c lines 301-314.
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
// T_PlatRaise — Platform movement thinker callback
// Translated from lines 42-116 of p_plats.c
// ============================================================================

/// Platform movement thinker — called each tic for each active platform mover.
///
/// Moves the platform's sector floor toward its high or low destination using
/// [`t_move_plane`], plays movement/arrival sounds, and manages the
/// up → waiting → down → waiting cycle (or one-shot variants).
///
/// # Parameters
///
/// * `plat` — mutable reference to the platform thinker data
/// * `sector` — mutable reference to the sector being moved
/// * `leveltime` — current level time in tics (for 8-tic sound interval)
/// * `change_sector_fn` — `P_ChangeSector` equivalent: `(crush) -> nofit`
/// * `sound_fn` — `S_StartSound` equivalent: `(sfx_id)` — the caller
///   captures the sound origin (sector soundorg) in the closure
///
/// # Returns
///
/// `true` if the platform thinker should be removed (reached final
/// destination for a one-shot type). `false` if movement is ongoing.
pub fn t_plat_raise(
    plat: &mut PlatT,
    sector: &mut Sector,
    leveltime: i32,
    change_sector_fn: &mut dyn FnMut(bool) -> bool,
    sound_fn: &mut dyn FnMut(SfxEnum),
) -> bool {
    match plat.status {
        PlatStatus::Up => {
            // Move floor upward toward plat.high
            let res = t_move_plane(
                sector,
                plat.speed,
                plat.high,
                plat.crush,
                0, // floor
                1, // direction: up
                change_sector_fn,
            );

            // Play stone-movement sound every 8 tics for raise-and-change types.
            // Original C (line 67): if (plat->type == raiseAndChange
            //                         || plat->type == raiseToNearestAndChange)
            //                           if (!(leveltime&7))
            //                             S_StartSound(&sec->soundorg, sfx_stnmov);
            if (plat.plat_type == PlatType::RaiseAndChange
                || plat.plat_type == PlatType::RaiseToNearestAndChange)
                && (leveltime & 7) == 0
            {
                sound_fn(SfxEnum::sfx_stnmov);
            }

            if res == ResultE::Crushed && !plat.crush {
                // Something blocked us and we don't crush — reverse to down.
                plat.count = plat.wait;
                plat.status = PlatStatus::Down;
                sound_fn(SfxEnum::sfx_pstart);
            } else if res == ResultE::PastDest {
                // Reached the top.
                plat.count = plat.wait;
                plat.status = PlatStatus::Waiting;
                sound_fn(SfxEnum::sfx_pstop);

                // One-shot types: remove the thinker immediately.
                match plat.plat_type {
                    PlatType::BlazeDWUS
                    | PlatType::DownWaitUpStay
                    | PlatType::RaiseAndChange
                    | PlatType::RaiseToNearestAndChange => {
                        return true; // signal removal
                    }
                    _ => {}
                }
            }
        }

        PlatStatus::Down => {
            // Move floor downward toward plat.low
            let res = t_move_plane(
                sector,
                plat.speed,
                plat.low,
                false, // never crush downward in original C
                0,     // floor
                -1,    // direction: down
                change_sector_fn,
            );

            if res == ResultE::PastDest {
                // Reached the bottom — start waiting.
                plat.count = plat.wait;
                plat.status = PlatStatus::Waiting;
                sound_fn(SfxEnum::sfx_pstop);
            }
        }

        PlatStatus::Waiting => {
            plat.count -= 1;
            if plat.count == 0 {
                // Timer expired — toggle direction based on current floor position.
                if sector.floorheight == plat.low {
                    plat.status = PlatStatus::Up;
                } else {
                    plat.status = PlatStatus::Down;
                }
                sound_fn(SfxEnum::sfx_pstart);
            }
            // Note: In the original C, the `waiting` case falls through to
            // `in_stasis` which is a no-op (break). No action needed here.
        }

        PlatStatus::InStasis => {
            // No-op — platform is stopped, waiting for P_ActivateInStasis.
        }
    }

    false // thinker is still active
}

// ============================================================================
// EV_DoPlat — Spawn platform movers by line tag
// Translated from lines 123-230 of p_plats.c
// ============================================================================

/// Spawn platform thinkers for all sectors matching the trigger line's tag.
///
/// For `PerpetualRaise`, first reactivates any platforms in stasis with the
/// matching tag via [`p_activate_in_stasis`].
///
/// # Parameters
///
/// * `line_idx` — index of the trigger linedef
/// * `plat_type` — the type of platform behavior to create
/// * `amount` — height amount for `RaiseAndChange` (units, multiplied by `FRACUNIT`)
/// * `ctx` — mutable reference to the spec context for world access
///
/// # Returns
///
/// `true` if any platform was started.
pub fn ev_do_plat(
    line_idx: usize,
    plat_type: PlatType,
    amount: i32,
    ctx: &mut dyn SpecContext,
) -> bool {
    // For perpetualRaise, reactivate any platforms in stasis with the
    // same tag first.
    // Original C (line 138): if (type != perpetualRaise)
    //     switch(type) { ... }
    // Actually line 134-138:
    //   switch(type) {
    //     case perpetualRaise:
    //       P_ActivateInStasis(line->tag);
    //       break;
    //     default: break;
    //   }
    if plat_type == PlatType::PerpetualRaise {
        p_activate_in_stasis(ctx.lines()[line_idx].tag as i32, ctx);
    }

    let mut rtn = false;
    let mut secnum: i32 = -1;

    loop {
        secnum = p_find_sector_from_line_tag(&ctx.lines()[line_idx], secnum, ctx.sectors());
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // Skip if sector already has active specialdata.
        if ctx.sectors()[sec_idx].specialdata.is_some() {
            continue;
        }

        rtn = true;

        // Create platform thinker data.
        let mut plat = PlatT::new(sec_idx);
        plat.plat_type = plat_type;
        plat.crush = false;
        plat.tag = ctx.sectors()[sec_idx].tag as i32;

        match plat_type {
            PlatType::RaiseToNearestAndChange => {
                plat.speed = Fixed::new(PLATSPEED / 2);
                // Copy floor texture from trigger line's front side sector.
                let front_side_idx = ctx.lines()[line_idx].sidenum[0] as usize;
                let front_sec = ctx.sides()[front_side_idx].sector;
                let new_floorpic = ctx.sectors()[front_sec].floorpic;

                plat.high = p_find_next_highest_floor(
                    sec_idx,
                    ctx.sectors()[sec_idx].floorheight,
                    ctx.sectors(),
                    ctx.lines(),
                );
                plat.wait = 0;
                plat.status = PlatStatus::Up;
                plat.low = ctx.sectors()[sec_idx].floorheight;

                // Copy floorpic and clear sector special.
                // Original C: sec->floorpic = sides[line->sidenum[0]].sector->floorpic;
                //             sec->special = 0;
                ctx.sectors_mut()[sec_idx].floorpic = new_floorpic;
                ctx.sectors_mut()[sec_idx].special = 0;

                ctx.s_start_sound(None, SfxEnum::sfx_stnmov);
            }

            PlatType::RaiseAndChange => {
                plat.speed = Fixed::new(PLATSPEED / 2);
                // Copy floor texture from trigger line's front side sector.
                let front_side_idx = ctx.lines()[line_idx].sidenum[0] as usize;
                let front_sec = ctx.sides()[front_side_idx].sector;
                let new_floorpic = ctx.sectors()[front_sec].floorpic;

                plat.high = ctx.sectors()[sec_idx].floorheight + Fixed::new(amount * FRACUNIT);
                plat.wait = 0;
                plat.status = PlatStatus::Up;
                plat.low = ctx.sectors()[sec_idx].floorheight;

                // Copy floorpic to sector.
                ctx.sectors_mut()[sec_idx].floorpic = new_floorpic;

                ctx.s_start_sound(None, SfxEnum::sfx_stnmov);
            }

            PlatType::DownWaitUpStay => {
                plat.speed = Fixed::new(PLATSPEED * 4);
                plat.low = p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                // Clamp: low must not exceed sector floorheight.
                if plat.low > ctx.sectors()[sec_idx].floorheight {
                    plat.low = ctx.sectors()[sec_idx].floorheight;
                }
                plat.high = ctx.sectors()[sec_idx].floorheight;
                plat.wait = PLATWAIT * 35;
                plat.status = PlatStatus::Down;

                ctx.s_start_sound(None, SfxEnum::sfx_pstart);
            }

            PlatType::BlazeDWUS => {
                plat.speed = Fixed::new(PLATSPEED * 8);
                plat.low = p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                // Clamp: low must not exceed sector floorheight.
                if plat.low > ctx.sectors()[sec_idx].floorheight {
                    plat.low = ctx.sectors()[sec_idx].floorheight;
                }
                plat.high = ctx.sectors()[sec_idx].floorheight;
                plat.wait = PLATWAIT * 35;
                plat.status = PlatStatus::Down;

                ctx.s_start_sound(None, SfxEnum::sfx_pstart);
            }

            PlatType::PerpetualRaise => {
                plat.speed = Fixed::new(PLATSPEED);
                plat.low = p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                // Clamp: low must not exceed sector floorheight.
                if plat.low > ctx.sectors()[sec_idx].floorheight {
                    plat.low = ctx.sectors()[sec_idx].floorheight;
                }
                plat.high = p_find_highest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                // Clamp: high must not be below sector floorheight.
                if plat.high < ctx.sectors()[sec_idx].floorheight {
                    plat.high = ctx.sectors()[sec_idx].floorheight;
                }
                plat.wait = PLATWAIT * 35;
                // Random initial direction via P_Random() & 1.
                let rng_val = ctx.rng_mut().p_random();
                plat.status = if rng_val & 1 != 0 {
                    PlatStatus::Down
                } else {
                    PlatStatus::Up
                };

                ctx.s_start_sound(None, SfxEnum::sfx_pstart);
            }
        }

        // Register the platform thinker with the thinker system and record
        // it in the active platforms list via the SpecContext trait.
        // This makes the platform functional — T_PlatRaise will execute
        // each tic to move the platform.
        // Original C: P_AddThinker + P_AddActivePlat (p_plats.c lines 257-258)
        ctx.p_add_thinker_plat(plat);
    }

    rtn
}

// ============================================================================
// EV_StopPlat — Stop platforms by tag
// Translated from lines 273-286 of p_plats.c
// ============================================================================

/// Stop all active platforms whose tag matches the trigger line's tag.
///
/// For each matching active platform not already in stasis, saves the current
/// status to `oldstatus`, sets the status to `InStasis`, and clears the
/// thinker action function so the platform stops moving.
///
/// # Parameters
///
/// * `tag` — the tag value from the trigger linedef (`line->tag`)
/// * `plats` — mutable slice of platform thinker data
/// * `active` — the active platforms tracker
pub fn ev_stop_plat(tag: i32, plats: &mut [PlatT], active: &ActivePlats) {
    for &slot in active.slots.iter() {
        if let Some(plat_idx) = slot {
            if plat_idx < plats.len()
                && plats[plat_idx].tag == tag
                && plats[plat_idx].status != PlatStatus::InStasis
            {
                plats[plat_idx].oldstatus = plats[plat_idx].status;
                plats[plat_idx].status = PlatStatus::InStasis;
                // Original C sets thinker function to NULL (ActionFn::None).
                // The dispatch logic in TickContext::dispatch_thinker should
                // skip platforms in InStasis, or the thinker function field
                // on the PlatT's embedded thinker is set to None.
                plats[plat_idx].thinker.function = crate::types::thinker::ActionFn::None;
            }
        }
    }
}

// ============================================================================
// P_ActivateInStasis — Resume stasis platforms
// Translated from lines 258-271 of p_plats.c
// ============================================================================

/// Reactivate all platforms in stasis whose tag matches the given tag.
///
/// For each matching platform in `InStasis`, restores `oldstatus` and sets
/// the thinker action back to `PlatRaise` so it resumes movement.
///
/// This is called at the start of `EV_DoPlat` for `PerpetualRaise` type
/// before spawning new platforms.
///
/// # Parameters
///
/// * `tag` — the tag value to match against platform tags
/// * `ctx` — mutable spec context (provides access to platform state)
pub fn p_activate_in_stasis(tag: i32, ctx: &mut dyn SpecContext) {
    // Delegate to the SpecContext trait method which has access to the
    // active platform thinkers in the thinker list. This matches the
    // original C logic (p_plats.c:158-175) which iterates the activeplats
    // array and reactivates any in-stasis platforms with a matching tag.
    ctx.p_activate_in_stasis_plat(tag);
}

/// Standalone version of `P_ActivateInStasis` operating on raw platform data
/// and the active platforms tracker.
///
/// Reactivates all platforms in stasis whose tag matches the given tag.
///
/// # Parameters
///
/// * `tag` — the tag value to match
/// * `plats` — mutable slice of all platform thinker data
/// * `active` — the active platforms tracker
pub fn p_activate_in_stasis_raw(tag: i32, plats: &mut [PlatT], active: &ActivePlats) {
    for &slot in active.slots.iter() {
        if let Some(plat_idx) = slot {
            if plat_idx < plats.len()
                && plats[plat_idx].tag == tag
                && plats[plat_idx].status == PlatStatus::InStasis
            {
                plats[plat_idx].status = plats[plat_idx].oldstatus;
                plats[plat_idx].thinker.function = crate::types::thinker::ActionFn::PlatRaise;
            }
        }
    }
}

// ============================================================================
// P_AddActivePlat — Track active platform
// Translated from lines 288-299 of p_plats.c
// ============================================================================

/// Add a platform thinker index to the active platforms tracker.
///
/// Finds the first empty slot and stores the index. Panics (via `I_Error`
/// equivalent) if no slot is available — the original C calls `I_Error`
/// which terminates the process.
///
/// # Parameters
///
/// * `active` — mutable reference to the active platforms tracker
/// * `plat_idx` — the thinker data index to register
pub fn p_add_active_plat(active: &mut ActivePlats, plat_idx: usize) {
    if active.add(plat_idx).is_none() {
        // Original C: I_Error("P_AddActivePlat: no more plats!");
        tracing::error!("P_AddActivePlat: no more plats!");
        panic!("P_AddActivePlat: no more plats!");
    }
}

// ============================================================================
// P_RemoveActivePlat — Remove active platform
// Translated from lines 301-314 of p_plats.c
// ============================================================================

/// Remove a platform thinker from the active platforms tracker and clean up.
///
/// Clears the sector's `specialdata`, marks the thinker for removal, and
/// frees the tracking slot.
///
/// In the original C, this also calls `P_RemoveThinker`. The caller is
/// responsible for removing the thinker from the thinker list after calling
/// this function (or the concrete SpecContext handles it).
///
/// # Parameters
///
/// * `plat_idx` — the platform thinker data index
/// * `plats` — mutable slice of all platform thinker data
/// * `sectors` — mutable slice of all sectors
/// * `active` — mutable reference to the active platforms tracker
pub fn p_remove_active_plat(
    plat_idx: usize,
    plats: &mut [PlatT],
    sectors: &mut [Sector],
    active: &mut ActivePlats,
) {
    let sec_idx = plats[plat_idx].sector;
    // Clear sector specialdata — sector is no longer occupied.
    sectors[sec_idx].specialdata = None;
    // Mark thinker for removal (the thinker list will garbage-collect it).
    plats[plat_idx].thinker.function = crate::types::thinker::ActionFn::PendingRemoval;
    // Remove from active tracking.
    active.remove(plat_idx);
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::spec::MAXPLATS;

    // ---- ActivePlats tests ----

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
    fn test_active_plats_default() {
        let ap = ActivePlats::default();
        assert!(ap.slots.iter().all(|s| s.is_none()));
    }

    #[test]
    fn test_active_plats_add_to_freed_slot() {
        let mut ap = ActivePlats::new();
        ap.add(10);
        ap.add(20);
        ap.remove(10);
        // Should reuse slot 0
        assert_eq!(ap.add(30), Some(0));
        assert_eq!(ap.slots[0], Some(30));
        assert_eq!(ap.slots[1], Some(20));
    }

    // ---- PlatStatus re-export test ----

    #[test]
    fn test_plat_status_variants() {
        let _ = PlatStatus::Up;
        let _ = PlatStatus::Down;
        let _ = PlatStatus::Waiting;
        let _ = PlatStatus::InStasis;
    }

    // ---- T_PlatRaise tests ----

    fn make_test_plat(status: PlatStatus, plat_type: PlatType) -> PlatT {
        let mut plat = PlatT::new(0);
        plat.status = status;
        plat.plat_type = plat_type;
        plat.speed = Fixed::new(PLATSPEED);
        plat.low = Fixed::new(0);
        plat.high = Fixed::new(128 * FRACUNIT);
        plat.wait = 105; // PLATWAIT * 35
        plat.count = 10;
        plat.crush = false;
        plat
    }

    fn make_test_sector(floorheight: i32) -> Sector {
        Sector {
            floorheight: Fixed::new(floorheight),
            ceilingheight: Fixed::new(256 * FRACUNIT),
            floorpic: 0,
            ceilingpic: 0,
            lightlevel: 128,
            special: 0,
            tag: 1,
            soundtraversed: 0,
            soundtarget: None,
            validcount: 0,
            specialdata: Some(0),
            soundorg: crate::types::map_data::DegenMobj::default(),
            linecount: 0,
            lines: Vec::new(),
            blockbox: [0; 4],
            thinglist: None,
        }
    }

    #[test]
    fn test_t_plat_raise_waiting_countdown() {
        let mut plat = make_test_plat(PlatStatus::Waiting, PlatType::PerpetualRaise);
        plat.count = 5;
        let mut sector = make_test_sector(0);
        let mut sounds: Vec<SfxEnum> = Vec::new();

        let should_remove = t_plat_raise(&mut plat, &mut sector, 100, &mut |_| false, &mut |sfx| {
            sounds.push(sfx)
        });

        assert!(!should_remove);
        assert_eq!(plat.count, 4);
        assert_eq!(plat.status, PlatStatus::Waiting);
        assert!(sounds.is_empty());
    }

    #[test]
    fn test_t_plat_raise_waiting_expired_at_low() {
        let mut plat = make_test_plat(PlatStatus::Waiting, PlatType::PerpetualRaise);
        plat.count = 1; // Will decrement to 0
        plat.low = Fixed::new(0);
        let mut sector = make_test_sector(0); // floorheight == low
        let mut sounds: Vec<SfxEnum> = Vec::new();

        let should_remove = t_plat_raise(&mut plat, &mut sector, 100, &mut |_| false, &mut |sfx| {
            sounds.push(sfx)
        });

        assert!(!should_remove);
        assert_eq!(plat.count, 0);
        assert_eq!(plat.status, PlatStatus::Up);
        assert_eq!(sounds, vec![SfxEnum::sfx_pstart]);
    }

    #[test]
    fn test_t_plat_raise_waiting_expired_at_high() {
        let mut plat = make_test_plat(PlatStatus::Waiting, PlatType::PerpetualRaise);
        plat.count = 1;
        plat.low = Fixed::new(0);
        let mut sector = make_test_sector(128 * FRACUNIT); // floorheight != low
        let mut sounds: Vec<SfxEnum> = Vec::new();

        let should_remove = t_plat_raise(&mut plat, &mut sector, 100, &mut |_| false, &mut |sfx| {
            sounds.push(sfx)
        });

        assert!(!should_remove);
        assert_eq!(plat.status, PlatStatus::Down);
        assert_eq!(sounds, vec![SfxEnum::sfx_pstart]);
    }

    #[test]
    fn test_t_plat_raise_in_stasis_noop() {
        let mut plat = make_test_plat(PlatStatus::InStasis, PlatType::PerpetualRaise);
        let mut sector = make_test_sector(0);
        let mut sounds: Vec<SfxEnum> = Vec::new();

        let should_remove = t_plat_raise(&mut plat, &mut sector, 100, &mut |_| false, &mut |sfx| {
            sounds.push(sfx)
        });

        assert!(!should_remove);
        assert!(sounds.is_empty());
    }

    // ---- EV_StopPlat tests ----

    #[test]
    fn test_ev_stop_plat_sets_stasis() {
        let mut plats = vec![make_test_plat(PlatStatus::Up, PlatType::PerpetualRaise)];
        plats[0].tag = 5;
        let mut active = ActivePlats::new();
        active.add(0);

        ev_stop_plat(5, &mut plats, &active);

        assert_eq!(plats[0].status, PlatStatus::InStasis);
        assert_eq!(plats[0].oldstatus, PlatStatus::Up);
        assert_eq!(
            plats[0].thinker.function,
            crate::types::thinker::ActionFn::None
        );
    }

    #[test]
    fn test_ev_stop_plat_skips_already_stasis() {
        let mut plats = vec![make_test_plat(
            PlatStatus::InStasis,
            PlatType::PerpetualRaise,
        )];
        plats[0].tag = 5;
        let old_status = plats[0].oldstatus;
        let mut active = ActivePlats::new();
        active.add(0);

        ev_stop_plat(5, &mut plats, &active);

        // Should remain unchanged since already InStasis.
        assert_eq!(plats[0].status, PlatStatus::InStasis);
        assert_eq!(plats[0].oldstatus, old_status);
    }

    #[test]
    fn test_ev_stop_plat_skips_wrong_tag() {
        let mut plats = vec![make_test_plat(PlatStatus::Up, PlatType::PerpetualRaise)];
        plats[0].tag = 5;
        let mut active = ActivePlats::new();
        active.add(0);

        ev_stop_plat(999, &mut plats, &active);

        // Should remain unchanged since tag doesn't match.
        assert_eq!(plats[0].status, PlatStatus::Up);
    }

    // ---- P_ActivateInStasis tests ----

    #[test]
    fn test_p_activate_in_stasis_raw_reactivates() {
        let mut plats = vec![make_test_plat(
            PlatStatus::InStasis,
            PlatType::PerpetualRaise,
        )];
        plats[0].tag = 5;
        plats[0].oldstatus = PlatStatus::Down;
        let active = ActivePlats {
            slots: {
                let mut s = [None; MAXPLATS];
                s[0] = Some(0);
                s
            },
        };

        p_activate_in_stasis_raw(5, &mut plats, &active);

        assert_eq!(plats[0].status, PlatStatus::Down);
        assert_eq!(
            plats[0].thinker.function,
            crate::types::thinker::ActionFn::PlatRaise
        );
    }

    #[test]
    fn test_p_activate_in_stasis_raw_skips_non_stasis() {
        let mut plats = vec![make_test_plat(PlatStatus::Up, PlatType::PerpetualRaise)];
        plats[0].tag = 5;
        let active = ActivePlats {
            slots: {
                let mut s = [None; MAXPLATS];
                s[0] = Some(0);
                s
            },
        };

        p_activate_in_stasis_raw(5, &mut plats, &active);

        // Should remain unchanged since not in stasis.
        assert_eq!(plats[0].status, PlatStatus::Up);
    }

    #[test]
    fn test_p_activate_in_stasis_raw_skips_wrong_tag() {
        let mut plats = vec![make_test_plat(
            PlatStatus::InStasis,
            PlatType::PerpetualRaise,
        )];
        plats[0].tag = 5;
        plats[0].oldstatus = PlatStatus::Down;
        let active = ActivePlats {
            slots: {
                let mut s = [None; MAXPLATS];
                s[0] = Some(0);
                s
            },
        };

        p_activate_in_stasis_raw(999, &mut plats, &active);

        // Should remain unchanged since tag doesn't match.
        assert_eq!(plats[0].status, PlatStatus::InStasis);
    }

    // ---- P_AddActivePlat tests ----

    #[test]
    fn test_p_add_active_plat_success() {
        let mut active = ActivePlats::new();
        p_add_active_plat(&mut active, 42);
        assert_eq!(active.slots[0], Some(42));
    }

    #[test]
    #[should_panic(expected = "P_AddActivePlat: no more plats!")]
    fn test_p_add_active_plat_full_panics() {
        let mut active = ActivePlats::new();
        for i in 0..MAXPLATS {
            p_add_active_plat(&mut active, i);
        }
        // This should panic.
        p_add_active_plat(&mut active, 999);
    }

    // ---- P_RemoveActivePlat tests ----

    #[test]
    fn test_p_remove_active_plat() {
        let mut plats = vec![make_test_plat(PlatStatus::Up, PlatType::DownWaitUpStay)];
        plats[0].sector = 0;
        let mut sectors = vec![make_test_sector(0)];
        sectors[0].specialdata = Some(0);
        let mut active = ActivePlats::new();
        active.add(0);

        p_remove_active_plat(0, &mut plats, &mut sectors, &mut active);

        assert!(sectors[0].specialdata.is_none());
        assert_eq!(
            plats[0].thinker.function,
            crate::types::thinker::ActionFn::PendingRemoval
        );
        assert!(active.slots[0].is_none());
    }
}
