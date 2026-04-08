// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Floor and ceiling movement plane mover — shared `t_move_plane` utility,
//! floor movement thinker, floor event dispatch, and stair builder.
//!
//! Translated from linuxdoom-1.10/p_floor.c
//!
//! # Overview
//!
//! This module provides four public functions:
//!
//! | C function       | Rust function      | Description                           |
//! |------------------|--------------------|---------------------------------------|
//! | `T_MovePlane`    | [`t_move_plane`]   | Generic sector plane mover (shared)   |
//! | `T_MoveFloor`    | [`t_move_floor`]   | Floor movement thinker callback       |
//! | `EV_DoFloor`     | [`ev_do_floor`]    | Spawn floor movement thinkers by tag  |
//! | `EV_BuildStairs` | [`ev_build_stairs`]| Build staircase sequence by tag       |
//!
//! `t_move_plane` is the **core shared mover** used by ALL sector movers
//! (ceilings, doors, floors, platforms). It handles four movement branches:
//! floor down, floor up, ceiling down, ceiling up.

use crate::info::sounds::SfxEnum;
use crate::play::spec::{
    get_side, p_find_highest_floor_surrounding, p_find_lowest_ceiling_surrounding,
    p_find_lowest_floor_surrounding, p_find_next_highest_floor, p_find_sector_from_line_tag,
    two_sided, FloorMoveT, FloorType, ResultE, SpecContext, StairType, FLOORSPEED,
};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::Sector;

// ============================================================================
// T_MovePlane — Generic plane (floor or ceiling) movement
// Translated from linuxdoom-1.10/p_floor.c lines 42-158
// ============================================================================

/// Move a sector's floor or ceiling plane toward a destination height.
///
/// This is the **core shared function** called by ALL sector movers:
/// ceilings (`T_MoveCeiling`), doors (`T_VerticalDoor`), floors
/// (`T_MoveFloor`), and platforms (`T_PlatRaise`).
///
/// Returns [`ResultE::Ok`] if still moving, [`ResultE::PastDest`] if the
/// destination has been reached, or [`ResultE::Crushed`] if movement was
/// blocked by an entity.
///
/// # Parameters
///
/// * `sector` — the sector being modified (mutated in place)
/// * `speed` — absolute speed of movement per tic (16.16 fixed-point)
/// * `dest` — destination height (16.16 fixed-point)
/// * `crush` — if `true`, damage things caught in the moving plane
/// * `floor_or_ceiling` — `0` = floor, `1` = ceiling
/// * `direction` — `-1` = lower, `1` = raise
/// * `change_sector_fn` — callback equivalent to `P_ChangeSector`. Takes
///   `crush: bool` and returns `true` if something does not fit (nofit).
///   The caller captures the sector index in the closure.
///
/// # Behavioral parity notes
///
/// * **Floor DOWN past dest**: if `P_ChangeSector` reports nofit, the height
///   is restored but the function **still returns `PastDest`** (the original
///   C code has `//return crushed;` commented out on line 67).
/// * **Floor UP past dest with crush**: if things don't fit **and** `crush`
///   is `true`, the height is NOT restored — it stays at `dest`, and
///   `Crushed` is returned. If `crush` is `false`, the height IS restored.
/// * **Ceiling UP normal**: the `#if 0` block (lines 149-155 in C) is NOT
///   implemented — ceiling raising never checks P_ChangeSector in normal
///   movement.
pub fn t_move_plane(
    sector: &mut Sector,
    speed: Fixed,
    dest: Fixed,
    crush: bool,
    floor_or_ceiling: i32,
    direction: i32,
    change_sector_fn: &mut dyn FnMut(bool) -> bool,
) -> ResultE {
    match floor_or_ceiling {
        // ---- FLOOR ----
        0 => match direction {
            // Floor moving DOWN
            -1 => {
                if sector.floorheight - speed < dest {
                    // Would overshoot — snap to destination
                    let lastpos = sector.floorheight;
                    sector.floorheight = dest;
                    let flag = change_sector_fn(crush);
                    if flag {
                        // Things don't fit — restore and re-run change
                        sector.floorheight = lastpos;
                        change_sector_fn(crush);
                        // NOTE: Original C has `//return crushed;` COMMENTED OUT
                        // Fall through to return PastDest
                    }
                    ResultE::PastDest
                } else {
                    // Normal movement step
                    let lastpos = sector.floorheight;
                    sector.floorheight = sector.floorheight - speed;
                    let flag = change_sector_fn(crush);
                    if flag {
                        // Things don't fit — restore
                        sector.floorheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::Ok
                }
            }
            // Floor moving UP
            1 => {
                if sector.floorheight + speed > dest {
                    // Would overshoot — snap to destination
                    let lastpos = sector.floorheight;
                    sector.floorheight = dest;
                    let flag = change_sector_fn(crush);
                    if flag {
                        if crush {
                            // Crush allowed: keep dest height, report crushed
                            return ResultE::Crushed;
                        }
                        // No crush: restore height
                        sector.floorheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::PastDest
                } else {
                    // Normal movement step — COULD GET CRUSHED
                    let lastpos = sector.floorheight;
                    sector.floorheight = sector.floorheight + speed;
                    let flag = change_sector_fn(crush);
                    if flag {
                        if crush {
                            // Crush allowed: keep new height, report crushed
                            return ResultE::Crushed;
                        }
                        // No crush: restore height
                        sector.floorheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::Ok
                }
            }
            _ => ResultE::Ok,
        },

        // ---- CEILING ----
        1 => match direction {
            // Ceiling moving DOWN
            -1 => {
                if sector.ceilingheight - speed < dest {
                    // Would overshoot — snap to destination
                    let lastpos = sector.ceilingheight;
                    sector.ceilingheight = dest;
                    let flag = change_sector_fn(crush);
                    if flag {
                        if crush {
                            // Crush allowed: keep dest height, report crushed
                            return ResultE::Crushed;
                        }
                        // No crush: restore height, fall through to PastDest
                        sector.ceilingheight = lastpos;
                        change_sector_fn(crush);
                    }
                    ResultE::PastDest
                } else {
                    // Normal movement step — COULD GET CRUSHED
                    let lastpos = sector.ceilingheight;
                    sector.ceilingheight = sector.ceilingheight - speed;
                    let flag = change_sector_fn(crush);
                    if flag {
                        if crush {
                            // Crush allowed: keep new height, report crushed
                            return ResultE::Crushed;
                        }
                        // No crush: restore height
                        sector.ceilingheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::Ok
                }
            }
            // Ceiling moving UP
            1 => {
                if sector.ceilingheight + speed > dest {
                    // Would overshoot — snap to destination
                    let lastpos = sector.ceilingheight;
                    sector.ceilingheight = dest;
                    let flag = change_sector_fn(crush);
                    if flag {
                        // Things don't fit — restore
                        sector.ceilingheight = lastpos;
                        change_sector_fn(crush);
                    }
                    // Always return PastDest regardless of nofit
                    ResultE::PastDest
                } else {
                    // Normal movement step
                    // NOTE: The original C code has a `#if 0` block here that
                    // would check P_ChangeSector for ceiling-up movement. This
                    // code is intentionally DISABLED in the original and is NOT
                    // implemented here.
                    sector.ceilingheight = sector.ceilingheight + speed;
                    ResultE::Ok
                }
            }
            _ => ResultE::Ok,
        },

        _ => ResultE::Ok,
    }
}

// ============================================================================
// T_MoveFloor — Floor movement thinker callback
// Translated from linuxdoom-1.10/p_floor.c lines 165-217
// ============================================================================

/// Floor movement thinker — called each tic for each active floor mover.
///
/// Moves the floor toward its destination height using [`t_move_plane`],
/// plays the stone-movement sound every 8 tics, and removes the thinker
/// when the destination is reached.
///
/// # Parameters
///
/// * `floor` — the floor mover data (read-only; sector changes go through
///   the sector reference)
/// * `sector` — mutable reference to the sector being moved
/// * `leveltime` — current level time in tics (for 8-tic sound interval)
/// * `change_sector_fn` — `P_ChangeSector` equivalent: `(crush) -> nofit`
/// * `sound_fn` — `S_StartSound` equivalent: `(sfx_id)` — the caller
///   captures the sound origin (sector soundorg) in the closure
///
/// # Returns
///
/// `true` if the floor reached its destination (`PastDest`) and the thinker
/// should be removed by the caller. `false` if movement is still in progress.
pub fn t_move_floor(
    floor: &FloorMoveT,
    sector: &mut Sector,
    leveltime: i32,
    change_sector_fn: &mut dyn FnMut(bool) -> bool,
    sound_fn: &mut dyn FnMut(SfxEnum),
) -> bool {
    // Move the floor plane toward its destination.
    let res = t_move_plane(
        sector,
        floor.speed,
        floor.floordestheight,
        floor.crush,
        0, // floor_or_ceiling = 0 (floor)
        floor.direction,
        change_sector_fn,
    );

    // Play stone-movement sound every 8 tics.
    // Original C: `if (!(leveltime&7))`
    if (leveltime & 7) == 0 {
        sound_fn(SfxEnum::sfx_stnmov);
    }

    // Handle destination reached.
    if res == ResultE::PastDest {
        // Clear sector's active mover reference.
        sector.specialdata = None;

        // Apply texture/special changes for specific floor types.
        if floor.direction == 1 {
            // Floor was raising — check for donutRaise
            if floor.floor_type == FloorType::DonutRaise {
                sector.special = floor.newsecspecial as i16;
                sector.floorpic = floor.newtexture;
            }
        } else if floor.direction == -1 {
            // Floor was lowering — check for lowerAndChange
            if floor.floor_type == FloorType::LowerAndChange {
                sector.special = floor.newsecspecial as i16;
                sector.floorpic = floor.newtexture;
            }
        }

        // Play platform-stop sound.
        sound_fn(SfxEnum::sfx_pstop);

        // Signal caller to remove the thinker.
        return true;
    }

    // Movement still in progress.
    false
}

// ============================================================================
// EV_DoFloor — Spawn floor movers by line tag
// Translated from linuxdoom-1.10/p_floor.c lines 225-415
// ============================================================================

/// Spawn floor movement thinkers for all sectors matching the trigger
/// line's tag.
///
/// Returns `true` if any floor movement was started.
///
/// The `floor_type` parameter selects from 13 floor movement variants
/// defined in [`FloorType`]. Each variant determines the destination
/// height, speed, crush behavior, and optional texture/special changes.
///
/// # Parameters
///
/// * `line_idx` — index of the trigger linedef (used for tag matching and
///   front-sector texture/special copying)
/// * `floor_type` — the type of floor movement to create
/// * `ctx` — game context providing sector, line, side, thinker, and sound
///   access
pub fn ev_do_floor(line_idx: usize, floor_type: FloorType, ctx: &mut dyn SpecContext) -> bool {
    let mut rtn = false;
    let mut secnum: i32 = -1;

    loop {
        secnum = p_find_sector_from_line_tag(&ctx.lines()[line_idx], secnum, ctx.sectors());
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // ALREADY MOVING? IF SO, KEEP GOING...
        if ctx.sectors()[sec_idx].specialdata.is_some() {
            continue;
        }

        // New floor thinker
        rtn = true;
        let mut floor = FloorMoveT::new(sec_idx);
        floor.floor_type = floor_type;
        floor.crush = false;

        match floor_type {
            // ---- lowerFloor ----
            // Lower to highest surrounding floor.
            FloorType::LowerFloor => {
                floor.direction = -1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    p_find_highest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
            }

            // ---- lowerFloorToLowest ----
            // Lower to lowest surrounding floor.
            FloorType::LowerFloorToLowest => {
                floor.direction = -1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
            }

            // ---- turboLower ----
            // Turbo lower to highest surrounding floor + 8 units.
            FloorType::TurboLower => {
                floor.direction = -1;
                floor.speed = Fixed::new(FLOORSPEED * 4);
                floor.floordestheight =
                    p_find_highest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                // If dest differs from current height, raise dest by 8 units
                // to create a lip (original C: p_floor.c line 296).
                if floor.floordestheight != ctx.sectors()[sec_idx].floorheight {
                    floor.floordestheight = floor.floordestheight + Fixed::new(8 * FRACUNIT);
                }
            }

            // ---- raiseFloorCrush (falls through to raiseFloor in C) ----
            // ---- raiseFloor ----
            // Both raise to lowest surrounding ceiling. Crush variant subtracts
            // 8 units and enables crush damage.
            FloorType::RaiseFloorCrush => {
                floor.crush = true;
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                if floor.floordestheight > ctx.sectors()[sec_idx].ceilingheight {
                    floor.floordestheight = ctx.sectors()[sec_idx].ceilingheight;
                }
                // Subtract 8 units for crush variant
                floor.floordestheight = floor.floordestheight - Fixed::new(8 * FRACUNIT);
            }
            FloorType::RaiseFloor => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                if floor.floordestheight > ctx.sectors()[sec_idx].ceilingheight {
                    floor.floordestheight = ctx.sectors()[sec_idx].ceilingheight;
                }
                // No subtraction for non-crush variant (the C code uses
                // `(8*FRACUNIT) * (floortype == raiseFloorCrush)` which is 0
                // for raiseFloor).
            }

            // ---- raiseFloorTurbo ----
            // Turbo raise to next highest floor (4x speed).
            FloorType::RaiseFloorTurbo => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED * 4);
                floor.floordestheight = p_find_next_highest_floor(
                    sec_idx,
                    ctx.sectors()[sec_idx].floorheight,
                    ctx.sectors(),
                    ctx.lines(),
                );
            }

            // ---- raiseFloorToNearest ----
            // Raise to next highest floor (normal speed).
            FloorType::RaiseFloorToNearest => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight = p_find_next_highest_floor(
                    sec_idx,
                    ctx.sectors()[sec_idx].floorheight,
                    ctx.sectors(),
                    ctx.lines(),
                );
            }

            // ---- raiseFloor24 ----
            // Raise floor by exactly 24 map units.
            FloorType::RaiseFloor24 => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(24 * FRACUNIT);
            }

            // ---- raiseFloor512 ----
            // Raise floor by exactly 512 map units.
            FloorType::RaiseFloor512 => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(512 * FRACUNIT);
            }

            // ---- raiseFloor24AndChange ----
            // Raise floor 24 units and copy texture+special from trigger
            // line's front sector. Changes are applied IMMEDIATELY to the
            // target sector (not deferred to pastdest).
            FloorType::RaiseFloor24AndChange => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(24 * FRACUNIT);
                // Copy texture and special from the trigger line's front sector
                // directly into the target sector (original C lines 371-372).
                if let Some(front_sec_idx) = ctx.lines()[line_idx].frontsector {
                    let front_floorpic = ctx.sectors()[front_sec_idx].floorpic;
                    let front_special = ctx.sectors()[front_sec_idx].special;
                    ctx.sectors_mut()[sec_idx].floorpic = front_floorpic;
                    ctx.sectors_mut()[sec_idx].special = front_special;
                }
            }

            // ---- raiseToTexture ----
            // Raise floor by the shortest lower texture height found on
            // two-sided lines bounding this sector. Both sides of each
            // two-sided line are checked.
            FloorType::RaiseToTexture => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                let mut min_size = i32::MAX;
                // Collect sector's line indices to avoid borrow conflicts.
                let sec_line_indices: Vec<usize> = ctx.sectors()[sec_idx].lines.clone();
                for &line_i in sec_line_indices.iter() {
                    if two_sided(sec_idx, line_i, ctx.lines()) {
                        // Check side 0 bottom texture
                        let side0 = get_side(sec_idx, line_i, 0, ctx.lines(), ctx.sides());
                        if side0.bottomtexture >= 0 {
                            let th = ctx.texture_height(side0.bottomtexture as i32);
                            if th.raw() < min_size {
                                min_size = th.raw();
                            }
                        }
                        // Check side 1 bottom texture
                        let side1 = get_side(sec_idx, line_i, 1, ctx.lines(), ctx.sides());
                        if side1.bottomtexture >= 0 {
                            let th = ctx.texture_height(side1.bottomtexture as i32);
                            if th.raw() < min_size {
                                min_size = th.raw();
                            }
                        }
                    }
                }
                floor.floordestheight = ctx.sectors()[sec_idx].floorheight + Fixed::new(min_size);
            }

            // ---- lowerAndChange ----
            // Lower to lowest surrounding floor and copy texture+special from
            // the neighboring sector whose floor height matches the destination.
            FloorType::LowerAndChange => {
                floor.direction = -1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                // Save current sector's floorpic (may be overwritten below).
                floor.newtexture = ctx.sectors()[sec_idx].floorpic;

                // Find the adjacent sector whose floor height matches dest,
                // and copy its texture + special. Original C lines 392-421.
                let sec_line_indices: Vec<usize> = ctx.sectors()[sec_idx].lines.clone();
                let dest_height = floor.floordestheight;
                for &line_i in &sec_line_indices {
                    if two_sided(sec_idx, line_i, ctx.lines()) {
                        let li = &ctx.lines()[line_i];
                        let side0_sector = ctx.sides()[li.sidenum[0] as usize].sector;
                        // Determine which side is "us" and which is "other"
                        let other_sec_idx = if side0_sector == sec_idx {
                            // Side 0 is our sector — get side 1's sector
                            let side1_idx = li.sidenum[1];
                            if side1_idx < 0 {
                                continue;
                            }
                            ctx.sides()[side1_idx as usize].sector
                        } else {
                            // Side 0 is other sector — use it
                            side0_sector
                        };
                        if ctx.sectors()[other_sec_idx].floorheight == dest_height {
                            floor.newtexture = ctx.sectors()[other_sec_idx].floorpic;
                            floor.newsecspecial = ctx.sectors()[other_sec_idx].special as i32;
                            break;
                        }
                    }
                }
            }

            // ---- donutRaise ----
            // Donut raise is normally handled by EV_DoDonut in spec.rs, but
            // when triggered through EV_DoFloor, it falls through to the
            // default case in the original C code (no special setup beyond
            // the default). We preserve this behavior.
            FloorType::DonutRaise => {
                // The C code's default case is a no-op break. The floor mover
                // will use whatever defaults were set above (direction=0,
                // speed=0, floordestheight=0). In practice, DonutRaise is
                // never passed to EV_DoFloor directly — it's created by
                // EV_DoDonut with pre-configured fields.
            }
        }

        // Register the floor mover with the thinker system and mark the
        // sector as having active specialdata.
        let _thinker_idx = ctx.p_add_thinker_floor(floor);
    }

    rtn
}

// ============================================================================
// EV_BuildStairs — Build a staircase
// Translated from linuxdoom-1.10/p_floor.c lines 421-556
// ============================================================================

/// Build a staircase sequence starting from sectors matching the trigger
/// line's tag.
///
/// Returns `true` if any stairs were started.
///
/// The stair builder chains adjacent sectors by matching floor textures
/// through two-sided lines. Each step in the chain gets a floor mover
/// raising it by `stairsize` units above the previous step. The chain
/// follows the **front→back** direction: for each two-sided line where
/// the current sector is the front sector, the back sector is the next
/// candidate step.
///
/// # Parameters
///
/// * `line_idx` — index of the trigger linedef
/// * `stair_type` — `Build8` (8-unit steps, 1/4 speed) or `Turbo16`
///   (16-unit steps, 4x speed)
/// * `ctx` — game context
pub fn ev_build_stairs(line_idx: usize, stair_type: StairType, ctx: &mut dyn SpecContext) -> bool {
    let mut rtn = false;
    let mut secnum: i32 = -1;

    // Determine speed and step size from stair type.
    let (speed, stairsize) = match stair_type {
        StairType::Build8 => (Fixed::new(FLOORSPEED / 4), Fixed::new(8 * FRACUNIT)),
        StairType::Turbo16 => (Fixed::new(FLOORSPEED * 4), Fixed::new(16 * FRACUNIT)),
    };

    loop {
        secnum = p_find_sector_from_line_tag(&ctx.lines()[line_idx], secnum, ctx.sectors());
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // ALREADY MOVING? IF SO, KEEP GOING...
        if ctx.sectors()[sec_idx].specialdata.is_some() {
            continue;
        }

        // Create the first step's floor mover.
        rtn = true;
        let mut height = ctx.sectors()[sec_idx].floorheight + stairsize;
        let mut floor = FloorMoveT::new(sec_idx);
        floor.direction = 1;
        floor.speed = speed;
        floor.floordestheight = height;
        let _ = ctx.p_add_thinker_floor(floor);

        // Save the floor texture for chain-matching.
        let texture = ctx.sectors()[sec_idx].floorpic;

        // Stair chaining loop: follow adjacent sectors with matching textures.
        // Original C algorithm (lines 492-550):
        //   1. For each line in the current sector, check two-sided lines.
        //   2. The line's frontsector MUST be the current sector.
        //   3. The backsector is the candidate next step.
        //   4. The candidate's floorpic must match the saved texture.
        //   5. If candidate already has specialdata, skip (height still bumps).
        //   6. Otherwise, create a mover for the candidate, advance, and repeat.
        let mut cur_sec_idx = sec_idx;
        let mut _cur_secnum = secnum;

        loop {
            let mut ok = false;
            // Collect current sector's line indices to avoid borrow conflicts.
            let sec_lines: Vec<usize> = ctx.sectors()[cur_sec_idx].lines.clone();

            for &line_i in &sec_lines {
                let li = &ctx.lines()[line_i];

                // Must be two-sided.
                if (li.flags & 0x04) == 0 {
                    // Not ML_TWOSIDED
                    continue;
                }

                // The line's frontsector must be the current sector.
                // Original C: `tsec = sec->lines[i]->frontsector;`
                //             `newsecnum = tsec - sectors;`
                //             `if (secnum != newsecnum) continue;`
                let front_sec = match li.frontsector {
                    Some(idx) => idx,
                    None => continue,
                };
                if front_sec != cur_sec_idx {
                    continue;
                }

                // Get the backsector as the next candidate.
                let back_sec = match li.backsector {
                    Some(idx) => idx,
                    None => continue,
                };

                // Floor texture must match the staircase texture.
                if ctx.sectors()[back_sec].floorpic != texture {
                    continue;
                }

                // Increment height BEFORE checking specialdata.
                // This matches the original C behavior (line 531-533).
                height = height + stairsize;

                // If candidate already has an active mover, skip creating one
                // but height was already incremented (creating a gap).
                if ctx.sectors()[back_sec].specialdata.is_some() {
                    continue;
                }

                // Create floor mover for the next step.
                cur_sec_idx = back_sec;
                _cur_secnum = back_sec as i32;
                let mut step_floor = FloorMoveT::new(cur_sec_idx);
                step_floor.direction = 1;
                step_floor.speed = speed;
                step_floor.floordestheight = height;
                let _ = ctx.p_add_thinker_floor(step_floor);

                ok = true;
                break; // Restart from the new sector's lines
            }

            if !ok {
                break;
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

    // --- T_MovePlane tests ---

    /// Test: floor lowering past destination snaps to dest and returns PastDest.
    #[test]
    fn test_floor_lower_past_dest() {
        let mut sector = Sector {
            floorheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(8 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 0, -1, &mut |_| false);
        assert_eq!(res, ResultE::PastDest);
        assert_eq!(sector.floorheight, dest);
    }

    /// Test: floor lowering past dest with nofit still returns PastDest
    /// (the commented-out `return crushed` behavior).
    #[test]
    fn test_floor_lower_past_dest_nofit_still_pastdest() {
        let mut sector = Sector {
            floorheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(8 * FRACUNIT);
        let original_height = sector.floorheight;
        let res = t_move_plane(&mut sector, speed, dest, false, 0, -1, &mut |_| true);
        // CRITICAL: Even though P_ChangeSector says nofit, result is PastDest.
        assert_eq!(res, ResultE::PastDest);
        // Height was restored to lastpos (original height).
        assert_eq!(sector.floorheight, original_height);
    }

    /// Test: floor lowering normal movement (not past dest).
    #[test]
    fn test_floor_lower_normal() {
        let mut sector = Sector {
            floorheight: Fixed::new(100 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(10 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 0, -1, &mut |_| false);
        assert_eq!(res, ResultE::Ok);
        assert_eq!(sector.floorheight, Fixed::new(99 * FRACUNIT));
    }

    /// Test: floor lowering normal movement with nofit returns Crushed and
    /// restores height.
    #[test]
    fn test_floor_lower_normal_nofit() {
        let mut sector = Sector {
            floorheight: Fixed::new(100 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(10 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 0, -1, &mut |_| true);
        assert_eq!(res, ResultE::Crushed);
        // Height restored
        assert_eq!(sector.floorheight, Fixed::new(100 * FRACUNIT));
    }

    /// Test: floor raising past dest without crush, no nofit → PastDest.
    #[test]
    fn test_floor_raise_past_dest_no_crush_no_nofit() {
        let mut sector = Sector {
            floorheight: Fixed::new(98 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(100 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 0, 1, &mut |_| false);
        assert_eq!(res, ResultE::PastDest);
        assert_eq!(sector.floorheight, dest);
    }

    /// Test: floor raising past dest with crush=true, nofit → Crushed, height stays at dest.
    #[test]
    fn test_floor_raise_past_dest_crush_nofit() {
        let mut sector = Sector {
            floorheight: Fixed::new(98 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(100 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, true, 0, 1, &mut |_| true);
        assert_eq!(res, ResultE::Crushed);
        // With crush=true, floor STAYS at dest (not restored)
        assert_eq!(sector.floorheight, dest);
    }

    /// Test: floor raising past dest with crush=false, nofit → Crushed, height restored.
    #[test]
    fn test_floor_raise_past_dest_nocrush_nofit() {
        let mut sector = Sector {
            floorheight: Fixed::new(98 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(100 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 0, 1, &mut |_| true);
        assert_eq!(res, ResultE::Crushed);
        // With crush=false, floor is restored
        assert_eq!(sector.floorheight, Fixed::new(98 * FRACUNIT));
    }

    /// Test: floor raising normal movement with crush=true, nofit → Crushed, height stays.
    #[test]
    fn test_floor_raise_normal_crush_nofit() {
        let mut sector = Sector {
            floorheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(100 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, true, 0, 1, &mut |_| true);
        assert_eq!(res, ResultE::Crushed);
        // With crush=true, floor stays at new position (10+1 = 11)
        assert_eq!(sector.floorheight, Fixed::new(11 * FRACUNIT));
    }

    /// Test: floor raising normal movement with crush=false, nofit → Crushed, height restored.
    #[test]
    fn test_floor_raise_normal_nocrush_nofit() {
        let mut sector = Sector {
            floorheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(100 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 0, 1, &mut |_| true);
        assert_eq!(res, ResultE::Crushed);
        // With crush=false, floor is restored
        assert_eq!(sector.floorheight, Fixed::new(10 * FRACUNIT));
    }

    // --- Ceiling tests ---

    /// Test: ceiling raising past dest, no nofit → PastDest.
    #[test]
    fn test_ceiling_raise_past_dest() {
        let mut sector = Sector {
            ceilingheight: Fixed::new(126 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(128 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 1, 1, &mut |_| false);
        assert_eq!(res, ResultE::PastDest);
        assert_eq!(sector.ceilingheight, dest);
    }

    /// Test: ceiling raising past dest with nofit → PastDest, height restored.
    #[test]
    fn test_ceiling_raise_past_dest_nofit() {
        let mut sector = Sector {
            ceilingheight: Fixed::new(126 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(128 * FRACUNIT);
        let original = sector.ceilingheight;
        let res = t_move_plane(&mut sector, speed, dest, false, 1, 1, &mut |_| true);
        // ALWAYS returns PastDest for ceiling UP, even with nofit
        assert_eq!(res, ResultE::PastDest);
        // Height was restored
        assert_eq!(sector.ceilingheight, original);
    }

    /// Test: ceiling lowering past dest with crush=true, nofit → Crushed.
    #[test]
    fn test_ceiling_lower_past_dest_crush_nofit() {
        let mut sector = Sector {
            ceilingheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(8 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, true, 1, -1, &mut |_| true);
        assert_eq!(res, ResultE::Crushed);
        // With crush=true, height stays at dest
        assert_eq!(sector.ceilingheight, dest);
    }

    /// Test: ceiling lowering past dest with crush=false, nofit → PastDest.
    #[test]
    fn test_ceiling_lower_past_dest_nocrush_nofit() {
        let mut sector = Sector {
            ceilingheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(8 * FRACUNIT);
        let original = sector.ceilingheight;
        let res = t_move_plane(&mut sector, speed, dest, false, 1, -1, &mut |_| true);
        // With crush=false, the height is restored and result is PastDest
        assert_eq!(res, ResultE::PastDest);
        assert_eq!(sector.ceilingheight, original);
    }

    /// Test: ceiling lowering normal movement with crush=true, nofit → Crushed.
    #[test]
    fn test_ceiling_lower_normal_crush_nofit() {
        let mut sector = Sector {
            ceilingheight: Fixed::new(100 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(10 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, true, 1, -1, &mut |_| true);
        assert_eq!(res, ResultE::Crushed);
    }

    /// Test: ceiling UP normal movement (no P_ChangeSector check per #if 0).
    #[test]
    fn test_ceiling_raise_normal() {
        let mut sector = Sector {
            ceilingheight: Fixed::new(100 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(200 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 1, 1, &mut |_| {
            panic!("change_sector_fn should NOT be called for ceiling UP normal");
        });
        assert_eq!(res, ResultE::Ok);
        assert_eq!(sector.ceilingheight, Fixed::new(101 * FRACUNIT));
    }

    // --- T_MoveFloor tests ---

    /// Test: t_move_floor plays sfx_stnmov every 8 tics.
    #[test]
    fn test_t_move_floor_sound_timing() {
        let floor = FloorMoveT {
            thinker: Default::default(),
            floor_type: FloorType::RaiseFloor,
            crush: false,
            sector: 0,
            direction: 1,
            newsecspecial: 0,
            newtexture: 0,
            floordestheight: Fixed::new(100 * FRACUNIT),
            speed: Fixed::new(FRACUNIT),
        };
        let mut sector = Sector {
            floorheight: Fixed::new(50 * FRACUNIT),
            ..Sector::default()
        };

        let mut sounds: Vec<SfxEnum> = Vec::new();

        // At leveltime = 0 (0 & 7 == 0), sound should play
        let _done = t_move_floor(&floor, &mut sector, 0, &mut |_| false, &mut |sfx| {
            sounds.push(sfx)
        });
        assert!(sounds.contains(&SfxEnum::sfx_stnmov));

        // At leveltime = 3 (3 & 7 != 0), no stnmov sound
        sounds.clear();
        let _done = t_move_floor(&floor, &mut sector, 3, &mut |_| false, &mut |sfx| {
            sounds.push(sfx)
        });
        assert!(!sounds.contains(&SfxEnum::sfx_stnmov));
    }

    /// Test: t_move_floor returns true on PastDest and clears specialdata.
    #[test]
    fn test_t_move_floor_pastdest() {
        let floor = FloorMoveT {
            thinker: Default::default(),
            floor_type: FloorType::RaiseFloor,
            crush: false,
            sector: 0,
            direction: 1,
            newsecspecial: 0,
            newtexture: 0,
            floordestheight: Fixed::new(51 * FRACUNIT),
            speed: Fixed::new(2 * FRACUNIT),
        };
        let mut sector = Sector {
            floorheight: Fixed::new(50 * FRACUNIT),
            specialdata: Some(42), // Active mover
            ..Sector::default()
        };

        let mut sounds: Vec<SfxEnum> = Vec::new();
        let done = t_move_floor(&floor, &mut sector, 0, &mut |_| false, &mut |sfx| {
            sounds.push(sfx)
        });
        assert!(done);
        assert!(sector.specialdata.is_none());
        assert!(sounds.contains(&SfxEnum::sfx_pstop));
    }

    /// Test: t_move_floor applies lowerAndChange texture/special on pastdest.
    #[test]
    fn test_t_move_floor_lower_and_change() {
        let floor = FloorMoveT {
            thinker: Default::default(),
            floor_type: FloorType::LowerAndChange,
            crush: false,
            sector: 0,
            direction: -1,
            newsecspecial: 7,
            newtexture: 42,
            floordestheight: Fixed::new(49 * FRACUNIT),
            speed: Fixed::new(2 * FRACUNIT),
        };
        let mut sector = Sector {
            floorheight: Fixed::new(50 * FRACUNIT),
            specialdata: Some(1),
            special: 0,
            floorpic: 0,
            ..Sector::default()
        };

        let done = t_move_floor(&floor, &mut sector, 0, &mut |_| false, &mut |_| {});
        assert!(done);
        assert_eq!(sector.special, 7);
        assert_eq!(sector.floorpic, 42);
    }
}
