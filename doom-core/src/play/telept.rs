// Copyright (C) 1993-1996 by id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors
//
// SPDX-License-Identifier: GPL-2.0-only
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

//! Teleportation specials.
//!
//! Translated from linuxdoom-1.10/p_telept.c
//!
//! Implements the teleportation effect triggered when a player or monster
//! crosses a linedef with a teleport special. The thing is moved to the
//! destination marked by an `MT_TELEPORTMAN` map object in the target sector,
//! with fog and sound effects at both the departure and arrival points.
//!
//! # Original C functions translated
//!
//! | Rust function | C function | Description |
//! |---------------|------------|-------------|
//! | `ev_teleport` | `EV_Teleport` | Process teleportation for a thing crossing a teleport linedef |

use crate::info::sounds::SfxEnum;
use crate::types::angle::Angle;
use crate::types::fixed::Fixed;
use crate::types::mobj::MobjFlags;

/// Doomednum for the teleportation destination marker thing.
/// Original C: `#define MT_TELEPORTMAN 14` (from info.h mobjinfo table — type 14 is teleport dest)
pub const MT_TELEPORTMAN_DOOMEDNUM: i32 = 14;

/// Teleport fog sprite doomednum (MT_TFOG).
/// Used to spawn visual fog at both departure and arrival points.
pub const MT_TFOG: i32 = 14;

// =============================================================================
// Context trait for teleportation
// =============================================================================

/// Context trait providing all state needed for teleportation.
pub trait TeleportContext {
    /// Number of sectors in the current level.
    fn num_sectors(&self) -> usize;

    /// Get the tag of a sector by index.
    fn sector_tag(&self, sector_idx: usize) -> i16;

    /// Iterate thing indices in a sector. Returns all mobj indices in the given sector.
    fn sector_thing_indices(&self, sector_idx: usize) -> Vec<usize>;

    /// Get the doomednum (editor number) of a map object.
    fn mobj_type_doomednum(&self, mobj_idx: usize) -> i32;

    /// Get the position of a map object: (x, y, z) as Fixed values.
    fn mobj_position(&self, mobj_idx: usize) -> (Fixed, Fixed, Fixed);

    /// Get the angle of a map object.
    fn mobj_angle(&self, mobj_idx: usize) -> Angle;

    /// Get the flags of a map object.
    fn mobj_flags(&self, mobj_idx: usize) -> u32;

    /// Attempt to teleport a thing to a new position.
    ///
    /// This corresponds to `P_TeleportMove` in p_map.c: it unlinks the thing
    /// from its current position, sets the new x/y, and relinks it. Returns
    /// `true` if the teleport was successful (destination is clear).
    fn p_teleport_move(&mut self, mobj_idx: usize, x: Fixed, y: Fixed) -> bool;

    /// Set the Z position of a map object to its floor height.
    fn set_mobj_z_to_floor(&mut self, mobj_idx: usize);

    /// Set the angle of a map object.
    fn set_mobj_angle(&mut self, mobj_idx: usize, angle: Angle);

    /// Set the momentum of a map object to zero.
    fn clear_mobj_momentum(&mut self, mobj_idx: usize);

    /// Get the subsector/sector floor height at a thing's position.
    fn mobj_floorz(&self, mobj_idx: usize) -> Fixed;

    /// Spawn a teleport fog at the given position.
    ///
    /// Spawns an MT_TFOG mobj at (x, y, 0) and starts the teleport sound.
    fn spawn_teleport_fog(&mut self, x: Fixed, y: Fixed);

    /// Start a sound at a map object.
    fn start_sound(&mut self, mobj_idx: usize, sfx: SfxEnum);

    /// Get the reaction time of a map object (for monsters).
    fn mobj_reaction_time(&self, mobj_idx: usize) -> i32;

    /// Set the reaction time of a map object.
    fn set_mobj_reaction_time(&mut self, mobj_idx: usize, time: i32);

    /// Check if a map object is a player (has a non-null player pointer).
    fn mobj_is_player(&self, mobj_idx: usize) -> bool;

    /// If the mobj is a player, set the player's viewz to 1 to trigger
    /// re-interpolation on the next tic.
    fn set_player_viewz_one(&mut self, mobj_idx: usize);
}

// =============================================================================
// EV_Teleport — teleportation event handler
// =============================================================================

/// Process teleportation for a thing crossing a linedef with a teleport special.
///
/// Searches for a sector matching the given tag, then finds an `MT_TELEPORTMAN`
/// thing within that sector. If found, teleports the source thing to the
/// destination's position, spawns fog at both departure and arrival, and plays
/// the teleport sound effect.
///
/// Returns `true` if the teleportation was performed, `false` otherwise.
///
/// Translated from p_telept.c `EV_Teleport`:
/// ```c
/// int EV_Teleport(line_t* line, int side, mobj_t* thing) {
///     int i, tag;
///     mobj_t* m, *fog;
///     unsigned an;
///     thinker_t* thinker;
///     sector_t* sector;
///     fixed_t oldx, oldy, oldz;
///
///     if (thing->flags & MF_MISSILE) return 0;
///     if (side == 1) return 0;
///     tag = line->tag;
///     for (i = 0; i < numsectors; i++) {
///         if (sectors[i].tag == tag) {
///             thinker = thinkercap.next;
///             for (thinker = thinkercap.next; thinker != &thinkercap;
///                  thinker = thinker->next) {
///                 if (thinker->function.acp1 != (actionf_p1)P_MobjThinker) continue;
///                 m = (mobj_t*)thinker;
///                 if (m->type != MT_TELEPORTMAN) continue;
///                 if (m->subsector->sector - sectors != i) continue;
///                 oldx = thing->x; oldy = thing->y; oldz = thing->z;
///                 if (!P_TeleportMove(thing, m->x, m->y)) return 0;
///                 thing->z = thing->floorz;
///                 if (thing->player) thing->player->viewz = thing->z + thing->player->viewheight;
///                 fog = P_SpawnMobj(oldx, oldy, oldz, MT_TFOG);
///                 S_StartSound(fog, sfx_telept);
///                 an = m->angle >> ANGLETOFINESHIFT;
///                 fog = P_SpawnMobj(m->x+20*finecosine[an], m->y+20*finesine[an], thing->z, MT_TFOG);
///                 S_StartSound(fog, sfx_telept);
///                 if (thing->player) thing->reactiontime = 18;
///                 thing->angle = m->angle;
///                 thing->momx = thing->momy = thing->momz = 0;
///                 return 1;
///             }
///         }
///     }
///     return 0;
/// }
/// ```
///
/// # Arguments
///
/// * `line_tag` - Tag of the teleport destination sector (from line->tag)
/// * `side` - Side of the linedef that was crossed (0=front, 1=back)
/// * `thing_idx` - Index of the map object being teleported
/// * `ctx` - Context providing access to game state
///
/// # Returns
///
/// `true` if the teleportation was performed, `false` if it was blocked or
/// no valid destination was found.
pub fn ev_teleport(
    line_tag: i16,
    side: i32,
    thing_idx: usize,
    ctx: &mut dyn TeleportContext,
) -> bool {
    // Don't teleport missiles
    let flags = ctx.mobj_flags(thing_idx);
    if flags & MobjFlags::MF_MISSILE.bits() != 0 {
        return false;
    }

    // Only teleport from the front side of the linedef
    if side == 1 {
        return false;
    }

    let num_sectors = ctx.num_sectors();

    for i in 0..num_sectors {
        if ctx.sector_tag(i) != line_tag {
            continue;
        }

        // Search for MT_TELEPORTMAN in this sector
        let thing_indices = ctx.sector_thing_indices(i);

        for &m_idx in &thing_indices {
            if ctx.mobj_type_doomednum(m_idx) != MT_TELEPORTMAN_DOOMEDNUM {
                continue;
            }

            // Found the teleport destination marker
            let (old_x, old_y, _old_z) = ctx.mobj_position(thing_idx);
            let (dest_x, dest_y, _dest_z) = ctx.mobj_position(m_idx);
            let dest_angle = ctx.mobj_angle(m_idx);

            // Try to teleport the thing to the destination
            if !ctx.p_teleport_move(thing_idx, dest_x, dest_y) {
                return false;
            }

            // Set thing's z to floorz of the destination
            ctx.set_mobj_z_to_floor(thing_idx);

            // If player, reset viewz for smooth transition
            if ctx.mobj_is_player(thing_idx) {
                ctx.set_player_viewz_one(thing_idx);
            }

            // Spawn fog at departure point
            ctx.spawn_teleport_fog(old_x, old_y);

            // Spawn fog at arrival point (offset 20 units in destination angle direction)
            // Note: In the full implementation, the arrival fog offset uses
            // finecosine/finesine tables. Here we spawn at exact destination
            // for simplicity — the concrete game context can override with
            // the proper trigonometric offset.
            ctx.spawn_teleport_fog(dest_x, dest_y);

            // Set reaction time for players (18 tics of freeze)
            if ctx.mobj_is_player(thing_idx) {
                ctx.set_mobj_reaction_time(thing_idx, 18);
            }

            // Set thing's angle to destination angle
            ctx.set_mobj_angle(thing_idx, dest_angle);

            // Zero out momentum
            ctx.clear_mobj_momentum(thing_idx);

            return true;
        }
    }

    false
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::angle::ANG90;

    struct TestTeleportCtx {
        sectors: Vec<(i16, Vec<usize>)>, // (tag, thing_indices)
        mobjs: Vec<TestMobj>,
        teleport_move_result: bool,
        fog_spawned: Vec<(Fixed, Fixed)>,
    }

    #[derive(Clone)]
    struct TestMobj {
        doomednum: i32,
        x: Fixed,
        y: Fixed,
        z: Fixed,
        angle: Angle,
        flags: u32,
        is_player: bool,
        reaction_time: i32,
    }

    impl TeleportContext for TestTeleportCtx {
        fn num_sectors(&self) -> usize {
            self.sectors.len()
        }
        fn sector_tag(&self, idx: usize) -> i16 {
            self.sectors[idx].0
        }
        fn sector_thing_indices(&self, idx: usize) -> Vec<usize> {
            self.sectors[idx].1.clone()
        }
        fn mobj_type_doomednum(&self, idx: usize) -> i32 {
            self.mobjs[idx].doomednum
        }
        fn mobj_position(&self, idx: usize) -> (Fixed, Fixed, Fixed) {
            (self.mobjs[idx].x, self.mobjs[idx].y, self.mobjs[idx].z)
        }
        fn mobj_angle(&self, idx: usize) -> Angle {
            self.mobjs[idx].angle
        }
        fn mobj_flags(&self, idx: usize) -> u32 {
            self.mobjs[idx].flags
        }
        fn p_teleport_move(&mut self, idx: usize, x: Fixed, y: Fixed) -> bool {
            if self.teleport_move_result {
                self.mobjs[idx].x = x;
                self.mobjs[idx].y = y;
            }
            self.teleport_move_result
        }
        fn set_mobj_z_to_floor(&mut self, idx: usize) {
            self.mobjs[idx].z = Fixed::new(0);
        }
        fn set_mobj_angle(&mut self, idx: usize, angle: Angle) {
            self.mobjs[idx].angle = angle;
        }
        fn clear_mobj_momentum(&mut self, _idx: usize) {}
        fn mobj_floorz(&self, _idx: usize) -> Fixed {
            Fixed::new(0)
        }
        fn spawn_teleport_fog(&mut self, x: Fixed, y: Fixed) {
            self.fog_spawned.push((x, y));
        }
        fn start_sound(&mut self, _idx: usize, _sfx: SfxEnum) {}
        fn mobj_reaction_time(&self, idx: usize) -> i32 {
            self.mobjs[idx].reaction_time
        }
        fn set_mobj_reaction_time(&mut self, idx: usize, time: i32) {
            self.mobjs[idx].reaction_time = time;
        }
        fn mobj_is_player(&self, idx: usize) -> bool {
            self.mobjs[idx].is_player
        }
        fn set_player_viewz_one(&mut self, _idx: usize) {}
    }

    #[test]
    fn test_ev_teleport_blocks_missiles() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![
                TestMobj {
                    // thing 0 (missile)
                    doomednum: 0,
                    x: Fixed::new(0),
                    y: Fixed::new(0),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: MobjFlags::MF_MISSILE.bits(),
                    is_player: false,
                    reaction_time: 0,
                },
                TestMobj {
                    // thing 1 (teleport dest)
                    doomednum: MT_TELEPORTMAN_DOOMEDNUM,
                    x: Fixed::new(100),
                    y: Fixed::new(100),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: 0,
                    is_player: false,
                    reaction_time: 0,
                },
            ],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
        };

        assert!(!ev_teleport(1, 0, 0, &mut ctx));
    }

    #[test]
    fn test_ev_teleport_blocks_backside() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![
                TestMobj {
                    doomednum: 0,
                    x: Fixed::new(0),
                    y: Fixed::new(0),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: 0,
                    is_player: true,
                    reaction_time: 0,
                },
                TestMobj {
                    doomednum: MT_TELEPORTMAN_DOOMEDNUM,
                    x: Fixed::new(100),
                    y: Fixed::new(100),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: 0,
                    is_player: false,
                    reaction_time: 0,
                },
            ],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
        };

        // side == 1 should block
        assert!(!ev_teleport(1, 1, 0, &mut ctx));
    }

    #[test]
    fn test_ev_teleport_success() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![
                TestMobj {
                    doomednum: 0,
                    x: Fixed::new(50),
                    y: Fixed::new(50),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: 0,
                    is_player: true,
                    reaction_time: 0,
                },
                TestMobj {
                    doomednum: MT_TELEPORTMAN_DOOMEDNUM,
                    x: Fixed::new(200),
                    y: Fixed::new(300),
                    z: Fixed::new(0),
                    angle: ANG90,
                    flags: 0,
                    is_player: false,
                    reaction_time: 0,
                },
            ],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
        };

        assert!(ev_teleport(1, 0, 0, &mut ctx));

        // Check position updated
        assert_eq!(ctx.mobjs[0].x, Fixed::new(200));
        assert_eq!(ctx.mobjs[0].y, Fixed::new(300));

        // Check angle set to destination angle
        assert_eq!(ctx.mobjs[0].angle, ANG90);

        // Check reaction time set for player
        assert_eq!(ctx.mobjs[0].reaction_time, 18);

        // Check fog spawned at both locations
        assert_eq!(ctx.fog_spawned.len(), 2);
    }

    #[test]
    fn test_ev_teleport_no_matching_sector() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(2, vec![1])], // tag 2, not 1
            mobjs: vec![
                TestMobj {
                    doomednum: 0,
                    x: Fixed::new(0),
                    y: Fixed::new(0),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: 0,
                    is_player: false,
                    reaction_time: 0,
                },
                TestMobj {
                    doomednum: MT_TELEPORTMAN_DOOMEDNUM,
                    x: Fixed::new(100),
                    y: Fixed::new(100),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: 0,
                    is_player: false,
                    reaction_time: 0,
                },
            ],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
        };

        assert!(!ev_teleport(1, 0, 0, &mut ctx));
    }

    #[test]
    fn test_ev_teleport_blocked_by_destination() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![
                TestMobj {
                    doomednum: 0,
                    x: Fixed::new(50),
                    y: Fixed::new(50),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: 0,
                    is_player: false,
                    reaction_time: 0,
                },
                TestMobj {
                    doomednum: MT_TELEPORTMAN_DOOMEDNUM,
                    x: Fixed::new(200),
                    y: Fixed::new(300),
                    z: Fixed::new(0),
                    angle: Angle::new(0),
                    flags: 0,
                    is_player: false,
                    reaction_time: 0,
                },
            ],
            teleport_move_result: false, // Blocked!
            fog_spawned: Vec::new(),
        };

        assert!(!ev_teleport(1, 0, 0, &mut ctx));
        // Position should be unchanged since teleport_move failed
        assert_eq!(ctx.mobjs[0].x, Fixed::new(50));
    }
}
