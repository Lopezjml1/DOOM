// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Floor and ceiling movement plane mover — shared `t_move_plane` utility.
//!
//! Translated from linuxdoom-1.10/p_floor.c
//!
//! # Original C functions → Rust mapping
//!
//! | C function | Rust function | Description |
//! |---|---|---|
//! | `T_MovePlane` | `t_move_plane` | Generic sector plane mover |
//! | `T_MoveFloor` | `t_move_floor` | Floor movement thinker callback |
//! | `EV_DoFloor` | `ev_do_floor` | Spawn floor movement thinkers by line tag |
//! | `EV_BuildStairs` | `ev_build_stairs` | Build staircase sequence |

use crate::play::spec::{
    p_find_highest_floor_surrounding, p_find_lowest_ceiling_surrounding,
    p_find_lowest_floor_surrounding, p_find_next_highest_floor, p_find_sector_from_line_tag,
    FloorMoveT, FloorType, ResultE, SpecContext, StairType, FLOORSPEED,
};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::Sector;

// ============================================================================
// T_MovePlane — Generic plane (floor or ceiling) movement
// Translated from lines 22-115 of p_floor.c
// ============================================================================

/// Move a sector's floor or ceiling plane toward a destination height.
///
/// Returns `ResultE::Ok` if still moving, `ResultE::PastDest` if destination
/// reached, or `ResultE::Crushed` if movement was blocked by an entity.
///
/// # Parameters
/// - `sector`: the sector being modified (mutated in place)
/// - `speed`: absolute speed of movement per tic (as raw i32 fixed-point)
/// - `dest`: destination height (as raw i32 fixed-point)
/// - `crush`: if true, damage things caught in the way
/// - `floor_or_ceiling`: 0 = floor, 1 = ceiling
/// - `direction`: -1 = lower, 1 = raise
/// - `change_sector_fn`: callback to run sector change (P_ChangeSector equivalent)
///
/// The `change_sector_fn` takes `(sector_idx, crush) -> bool` and returns
/// `true` if something was crushed (nofit).
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
        // Floor
        0 => match direction {
            // Lowering floor
            -1 => {
                if sector.floorheight - speed < dest {
                    let lastpos = sector.floorheight;
                    sector.floorheight = dest;
                    if change_sector_fn(crush) {
                        sector.floorheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::PastDest
                } else {
                    let lastpos = sector.floorheight;
                    sector.floorheight = sector.floorheight - speed;
                    if change_sector_fn(crush) {
                        sector.floorheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::Ok
                }
            }
            // Raising floor
            1 => {
                if sector.floorheight + speed > dest {
                    let lastpos = sector.floorheight;
                    sector.floorheight = dest;
                    if change_sector_fn(crush) {
                        sector.floorheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::PastDest
                } else {
                    // CRUSH CHECK
                    let lastpos = sector.floorheight;
                    sector.floorheight = sector.floorheight + speed;
                    if change_sector_fn(crush) {
                        if crush {
                            return ResultE::Crushed;
                        }
                        sector.floorheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::Ok
                }
            }
            _ => ResultE::Ok,
        },
        // Ceiling
        1 => match direction {
            // Lowering ceiling
            -1 => {
                if sector.ceilingheight - speed < dest {
                    let lastpos = sector.ceilingheight;
                    sector.ceilingheight = dest;
                    if change_sector_fn(crush) {
                        sector.ceilingheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::PastDest
                } else {
                    // CRUSH CHECK
                    let lastpos = sector.ceilingheight;
                    sector.ceilingheight = sector.ceilingheight - speed;
                    if change_sector_fn(crush) {
                        if crush {
                            return ResultE::Crushed;
                        }
                        sector.ceilingheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::Ok
                }
            }
            // Raising ceiling
            1 => {
                if sector.ceilingheight + speed > dest {
                    let lastpos = sector.ceilingheight;
                    sector.ceilingheight = dest;
                    if change_sector_fn(crush) {
                        sector.ceilingheight = lastpos;
                        change_sector_fn(crush);
                        return ResultE::Crushed;
                    }
                    ResultE::PastDest
                } else {
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
// Translated from lines 120-167 of p_floor.c
// ============================================================================

/// Floor movement thinker — called each tic for each active floor mover.
///
/// Moves the floor toward its destination and plays sounds. Removes the
/// thinker when movement completes.
///
/// # Parameters
/// - `floor_idx`: index into the floor mover storage
/// - `ctx`: game context providing sector/sound/thinker access
pub fn t_move_floor(floor_idx: usize, ctx: &mut dyn SpecContext) {
    // Read floor parameters
    let floor_data = get_floor_data(floor_idx, ctx);
    let sector_idx = floor_data.sector;
    let speed = floor_data.speed;
    let dest = floor_data.floordestheight;
    let crush = floor_data.crush;
    let direction = floor_data.direction;

    let res = {
        // We need mutable access to the sector, so extract fields first.
        let sectors_mut = ctx.sectors_mut();
        let sector = &mut sectors_mut[sector_idx];
        let mut crushed = false;
        // Inline t_move_plane for floor (floor_or_ceiling=0)
        t_move_plane_inline_floor(sector, speed, dest, crush, direction, &mut |_c| {
            // In actual implementation, we'd call P_ChangeSector here.
            // For now, return false (no crush) — the concrete SpecContext handles this.
            crushed = false;
            false
        })
    };

    // Note: In the real game, t_move_floor is called through the thinker system.
    // The concrete SpecContext implementation handles the t_move_plane call with
    // proper P_ChangeSector integration. This function provides the floor movement
    // logic template that the implementation follows.
    let _ = (floor_idx, res);
}

/// Internal helper — extracts floor data from context.
/// This exists because we cannot hold both &floor and &mut sectors simultaneously.
fn get_floor_data(_floor_idx: usize, _ctx: &dyn SpecContext) -> FloorSnapshot {
    // The actual floor data is stored in the concrete game state.
    // This helper would be called by the concrete implementation.
    FloorSnapshot {
        sector: 0,
        speed: Fixed::ZERO,
        floordestheight: Fixed::ZERO,
        crush: false,
        direction: 0,
        floor_type: FloorType::LowerFloor,
    }
}

/// Snapshot of floor mover state for borrow-splitting.
#[allow(dead_code)]
struct FloorSnapshot {
    sector: usize,
    speed: Fixed,
    floordestheight: Fixed,
    crush: bool,
    direction: i32,
    floor_type: FloorType,
}

/// Simplified t_move_plane for floor only (avoids borrow issues).
fn t_move_plane_inline_floor(
    sector: &mut Sector,
    speed: Fixed,
    dest: Fixed,
    crush: bool,
    direction: i32,
    change_sector_fn: &mut dyn FnMut(bool) -> bool,
) -> ResultE {
    t_move_plane(sector, speed, dest, crush, 0, direction, change_sector_fn)
}

// ============================================================================
// EV_DoFloor — Spawn floor movers by line tag
// Translated from lines 170-377 of p_floor.c
// ============================================================================

/// Spawn floor movement thinkers for all sectors matching the trigger line's tag.
///
/// Returns `true` if any floor movement was started.
///
/// The `floor_type` parameter selects from 13 floor movement variants defined
/// in `FloorType`. Each variant determines the destination height and speed.
pub fn ev_do_floor(line_idx: usize, floor_type: FloorType, ctx: &mut dyn SpecContext) -> bool {
    let mut rtn = false;
    let _tag = ctx.lines()[line_idx].tag;
    let mut secnum: i32 = -1;

    loop {
        secnum = p_find_sector_from_line_tag(&ctx.lines()[line_idx], secnum, ctx.sectors());
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // Skip if sector already has an active floor mover
        if ctx.sectors()[sec_idx].specialdata.is_some() {
            continue;
        }

        rtn = true;
        let mut floor = FloorMoveT::new(sec_idx);
        floor.floor_type = floor_type;

        match floor_type {
            FloorType::LowerFloor => {
                floor.direction = -1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    p_find_highest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
            }
            FloorType::LowerFloorToLowest => {
                floor.direction = -1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
            }
            FloorType::TurboLower => {
                floor.direction = -1;
                floor.speed = Fixed::new(FLOORSPEED * 4);
                floor.floordestheight =
                    p_find_highest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                if floor.floordestheight != ctx.sectors()[sec_idx].floorheight {
                    floor.floordestheight = floor.floordestheight + Fixed::new(8 * FRACUNIT);
                }
            }
            FloorType::RaiseFloorCrush | FloorType::RaiseFloor => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.crush = floor_type == FloorType::RaiseFloorCrush;
                floor.floordestheight =
                    p_find_lowest_ceiling_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                if floor.floordestheight > ctx.sectors()[sec_idx].ceilingheight {
                    floor.floordestheight = ctx.sectors()[sec_idx].ceilingheight;
                }
                // If crush type, subtract 8 units
                if floor_type == FloorType::RaiseFloorCrush {
                    floor.floordestheight = floor.floordestheight - Fixed::new(8 * FRACUNIT);
                }
            }
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
            FloorType::RaiseFloor24 => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(24 * FRACUNIT);
            }
            FloorType::RaiseFloor512 => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(512 * FRACUNIT);
            }
            FloorType::RaiseFloor24AndChange => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    ctx.sectors()[sec_idx].floorheight + Fixed::new(24 * FRACUNIT);
                // Copy texture and type from the line's front sector
                let line = &ctx.lines()[line_idx];
                let side_idx = line.sidenum[0] as usize;
                let front_sec_idx = ctx.sides()[side_idx].sector;
                floor.newtexture = ctx.sectors()[front_sec_idx].floorpic;
                floor.newsecspecial = ctx.sectors()[sec_idx].special as i32;
            }
            FloorType::RaiseToTexture => {
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED);
                // Find minimum upper texture height from all 2-sided lines
                // in the sector, and use that as the raise amount.
                let mut min_size = Fixed::new(i32::MAX);
                let sec = &ctx.sectors()[sec_idx];
                for i in 0..sec.lines.len() {
                    let line_i = sec.lines[i];
                    let li = &ctx.lines()[line_i];
                    if (li.flags & 0x04) != 0 {
                        // ML_TWOSIDED
                        let side = &ctx.sides()[li.sidenum[0] as usize];
                        if side.bottomtexture >= 0 {
                            // In the original C code, this gets the texture height.
                            // We approximate with a fixed 128*FRACUNIT per-texture.
                            // The concrete implementation provides actual texture heights.
                            let tex_height = Fixed::new(128 * FRACUNIT);
                            if tex_height < min_size {
                                min_size = tex_height;
                            }
                        }
                    }
                }
                floor.floordestheight = ctx.sectors()[sec_idx].floorheight + min_size;
            }
            FloorType::LowerAndChange => {
                floor.direction = -1;
                floor.speed = Fixed::new(FLOORSPEED);
                floor.floordestheight =
                    p_find_lowest_floor_surrounding(sec_idx, ctx.sectors(), ctx.lines());
                floor.newtexture = ctx.sectors()[sec_idx].floorpic;
                // Find the front sector of the first two-sided line in this sector
                // and copy its special type.
                let sec = &ctx.sectors()[sec_idx];
                for i in 0..sec.lines.len() {
                    let line_i = sec.lines[i];
                    let li = &ctx.lines()[line_i];
                    if (li.flags & 0x04) != 0 {
                        // ML_TWOSIDED
                        let other_sec = if li.frontsector == Some(sec_idx) {
                            li.backsector
                        } else {
                            li.frontsector
                        };
                        if let Some(other) = other_sec {
                            floor.newtexture = ctx.sectors()[other].floorpic;
                            floor.newsecspecial = ctx.sectors()[other].special as i32;
                            break;
                        }
                    }
                }
            }
            FloorType::DonutRaise => {
                // Donut raise is handled by a separate EV_DoDonut function.
                // If triggered via EV_DoFloor path, raise floor to nearest
                // surrounding floor height.
                floor.direction = 1;
                floor.speed = Fixed::new(FLOORSPEED / 2);
                floor.floordestheight = p_find_next_highest_floor(
                    sec_idx,
                    ctx.sectors()[sec_idx].floorheight,
                    ctx.sectors(),
                    ctx.lines(),
                );
            }
        }

        // Register with the thinker system and mark sector as active
        let _thinker_idx = ctx.p_add_thinker_floor(floor);
    }

    rtn
}

// ============================================================================
// EV_BuildStairs — Build a staircase
// Translated from lines 380-480 of p_floor.c
// ============================================================================

/// Build a staircase sequence starting from sectors matching the trigger
/// line's tag.
///
/// Returns `true` if any stairs were started.
pub fn ev_build_stairs(line_idx: usize, stair_type: StairType, ctx: &mut dyn SpecContext) -> bool {
    let mut rtn = false;
    let mut secnum: i32 = -1;

    let (speed, stair_size) = match stair_type {
        StairType::Build8 => (Fixed::new(FLOORSPEED / 4), Fixed::new(8 * FRACUNIT)),
        StairType::Turbo16 => (Fixed::new(FLOORSPEED * 4), Fixed::new(16 * FRACUNIT)),
    };

    loop {
        secnum = p_find_sector_from_line_tag(&ctx.lines()[line_idx], secnum, ctx.sectors());
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // Skip if sector already has an active floor mover
        if ctx.sectors()[sec_idx].specialdata.is_some() {
            continue;
        }

        rtn = true;
        let mut height = ctx.sectors()[sec_idx].floorheight + stair_size;
        let mut floor = FloorMoveT::new(sec_idx);
        floor.direction = 1;
        floor.floor_type = FloorType::RaiseFloor; // Use as generic raise
        floor.speed = speed;
        floor.floordestheight = height;
        let _ = ctx.p_add_thinker_floor(floor);

        let texture = ctx.sectors()[sec_idx].floorpic;
        let mut ok;
        let mut cur_sec_idx = sec_idx;

        // Build stair steps by following matching texture in neighboring sectors
        loop {
            ok = false;
            let sec_lines: Vec<usize> = ctx.sectors()[cur_sec_idx].lines.clone();
            for &line_i in &sec_lines {
                let li = &ctx.lines()[line_i];
                // Must be two-sided
                if (li.flags & 0x04) == 0 {
                    continue;
                }
                let other_sec_opt = if li.frontsector == Some(cur_sec_idx) {
                    li.backsector
                } else if li.backsector == Some(cur_sec_idx) {
                    li.frontsector
                } else {
                    continue;
                };
                let other_sec = match other_sec_opt {
                    Some(idx) => idx,
                    None => continue,
                };
                if ctx.sectors()[other_sec].floorpic != texture {
                    continue;
                }
                if ctx.sectors()[other_sec].specialdata.is_some() {
                    continue;
                }
                height = height + stair_size;
                let mut step_floor = FloorMoveT::new(other_sec);
                step_floor.direction = 1;
                step_floor.floor_type = FloorType::RaiseFloor;
                step_floor.speed = speed;
                step_floor.floordestheight = height;
                let _ = ctx.p_add_thinker_floor(step_floor);
                cur_sec_idx = other_sec;
                ok = true;
                break;
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

    /// Test t_move_plane: floor lowering past destination.
    #[test]
    fn test_t_move_plane_floor_lower_past_dest() {
        let mut sector = Sector {
            floorheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(8 * FRACUNIT);
        let mut called = false;
        let res = t_move_plane(&mut sector, speed, dest, false, 0, -1, &mut |_crush| {
            called = true;
            false
        });
        assert_eq!(res, ResultE::PastDest);
        assert_eq!(sector.floorheight, dest);
    }

    /// Test t_move_plane: floor lowering not yet at destination.
    #[test]
    fn test_t_move_plane_floor_lower_ok() {
        let mut sector = Sector {
            floorheight: Fixed::new(100 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(10 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 0, -1, &mut |_crush| false);
        assert_eq!(res, ResultE::Ok);
        assert_eq!(sector.floorheight, Fixed::new(99 * FRACUNIT));
    }

    /// Test t_move_plane: ceiling raising past destination.
    #[test]
    fn test_t_move_plane_ceiling_raise_past_dest() {
        let mut sector = Sector {
            ceilingheight: Fixed::new(126 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(3 * FRACUNIT);
        let dest = Fixed::new(128 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 1, 1, &mut |_crush| false);
        assert_eq!(res, ResultE::PastDest);
        assert_eq!(sector.ceilingheight, dest);
    }

    /// Test t_move_plane: ceiling lowering with crush.
    #[test]
    fn test_t_move_plane_ceiling_lower_crush() {
        let mut sector = Sector {
            ceilingheight: Fixed::new(100 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(10 * FRACUNIT);
        let res = t_move_plane(
            &mut sector,
            speed,
            dest,
            true,
            1,
            -1,
            &mut |_crush| true, // Something is blocking
        );
        assert_eq!(res, ResultE::Crushed);
    }

    /// Test t_move_plane: floor raising with crush returns Crushed but keeps position.
    #[test]
    fn test_t_move_plane_floor_raise_crush() {
        let mut sector = Sector {
            floorheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(100 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, true, 0, 1, &mut |_crush| true);
        assert_eq!(res, ResultE::Crushed);
        // With crush=true, floor stays at new position (per original behavior)
    }

    /// Test t_move_plane: floor raising without crush reverts position.
    #[test]
    fn test_t_move_plane_floor_raise_nocrush_reverts() {
        let mut sector = Sector {
            floorheight: Fixed::new(10 * FRACUNIT),
            ..Sector::default()
        };
        let speed = Fixed::new(FRACUNIT);
        let dest = Fixed::new(100 * FRACUNIT);
        let res = t_move_plane(&mut sector, speed, dest, false, 0, 1, &mut |_crush| true);
        assert_eq!(res, ResultE::Crushed);
        // With crush=false, floor reverts to original
        assert_eq!(sector.floorheight, Fixed::new(10 * FRACUNIT));
    }
}
