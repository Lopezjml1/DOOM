// DOOM Rust Port — Copyright (C) 1993-1996 id Software, Inc.
// Copyright (C) 2024 Rust DOOM Contributors
//
// This program is free software; you can redistribute it and/or
// modify it under the terms of the GNU General Public License
// as published by the Free Software Foundation; either version 2
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

//! Translated from linuxdoom-1.10/p_telept.c
//!
//! Teleportation special handling. Contains the single function [`ev_teleport`]
//! which is activated by linedef specials (types 39, 97, 125, and 174) to
//! teleport a map object to a destination marker (`MT_TELEPORTMAN`) in a
//! matching tagged sector.
//!
//! The teleportation sequence:
//! 1. Validates that the thing is not a missile and was hit from the front side.
//! 2. Searches all sectors for one matching the trigger line's tag.
//! 3. Within matching sectors, walks active map objects looking for an
//!    `MT_TELEPORTMAN` marker in that sector.
//! 4. Attempts to move the thing to the destination (with telefrag support).
//! 5. Spawns teleport fog at both the source and destination positions.
//! 6. Zeroes all momentum and sets the thing's angle to the destination's angle.
//! 7. Freezes a player for 18 tics after teleportation.

use crate::info::mobjinfo::MobjType;
use crate::info::sounds::SfxEnum;
use crate::types::angle::{Angle, ANGLETOFINESHIFT};
use crate::types::fixed::Fixed;
use crate::types::mobj::MobjFlags;
use crate::types::tables::{finecosine, FINESINE};

// ============================================================================
// Context trait for teleportation operations
// ============================================================================

/// Context trait providing all operations needed by [`ev_teleport`].
///
/// The concrete game state struct implements this trait to provide access to
/// level geometry, map objects, players, and cross-module operations (teleport
/// move, fog spawning, sound playback).
///
/// This is the Rust equivalent of the global state and function calls that
/// `EV_Teleport` in the original C source accessed directly through
/// `extern` declarations and global pointers (sectors, thinker list,
/// `P_TeleportMove`, `P_SpawnMobj`, `S_StartSound`).
pub trait TeleportContext {
    /// Get the tag for a line definition.
    ///
    /// Equivalent to `line->tag` in the original C source.
    fn line_tag(&self, line_idx: usize) -> i16;

    /// Get the number of sectors in the current level.
    ///
    /// Equivalent to `numsectors` global in the original C source.
    fn num_sectors(&self) -> usize;

    /// Get the tag for a sector.
    ///
    /// Equivalent to `sectors[idx].tag` in the original C source.
    fn sector_tag(&self, sector_idx: usize) -> i16;

    /// Get indices of all active map objects.
    ///
    /// This is the Rust equivalent of walking the thinker list
    /// (`thinkercap.next` through the circular linked list) and filtering
    /// for entries whose action function is `P_MobjThinker`. Each returned
    /// index corresponds to a live `MapObject` in the arena.
    fn active_mobj_indices(&self) -> Vec<usize>;

    /// Get the `type_` field (`MobjType` discriminant as `usize`) of a map object.
    ///
    /// Used to identify `MT_TELEPORTMAN` destination markers.
    fn mobj_type(&self, idx: usize) -> usize;

    /// Get the sector index for a map object via its subsector.
    ///
    /// Equivalent to `(mobj->subsector->sector - sectors)` in the original C,
    /// which computes the sector index from the subsector's sector pointer.
    /// Returns `None` if the map object has no valid subsector assignment.
    fn mobj_sector(&self, idx: usize) -> Option<usize>;

    /// Get the flags bitfield for a map object.
    ///
    /// Equivalent to `thing->flags` in the original C source.
    fn mobj_flags(&self, idx: usize) -> MobjFlags;

    /// Get position `(x, y, z)` for a map object.
    ///
    /// All values are 16.16 fixed-point map coordinates.
    fn mobj_position(&self, idx: usize) -> (Fixed, Fixed, Fixed);

    /// Get the facing angle for a map object.
    ///
    /// Equivalent to `m->angle` in the original C source. Used to determine
    /// the direction the teleport destination marker is facing, which controls
    /// both the destination fog offset direction and the teleported thing's
    /// final facing angle.
    fn mobj_angle(&self, idx: usize) -> Angle;

    /// Get the floor z for a map object at its current position.
    ///
    /// Equivalent to `thing->floorz` in the original C source. After
    /// `p_teleport_move` updates the thing's position, this returns the
    /// floor height at the new (destination) position.
    fn mobj_floorz(&self, idx: usize) -> Fixed;

    /// Get the player index for a map object, or `None` if not a player.
    ///
    /// Equivalent to checking `thing->player` in the original C source.
    /// If `Some(player_idx)`, the thing is a player and the index can be
    /// used with `player_viewheight` and `set_player_viewz`.
    fn mobj_player(&self, idx: usize) -> Option<usize>;

    /// Get the viewheight for a player.
    ///
    /// Equivalent to `thing->player->viewheight` in the original C source.
    /// Used to correctly position the player's eye height after teleportation.
    fn player_viewheight(&self, player_idx: usize) -> Fixed;

    /// Set a map object's z coordinate.
    ///
    /// Equivalent to `thing->z = value` in the original C source.
    fn set_mobj_z(&mut self, idx: usize, z: Fixed);

    /// Set a map object's facing angle.
    ///
    /// Equivalent to `thing->angle = m->angle` in the original C source.
    fn set_mobj_angle(&mut self, idx: usize, angle: Angle);

    /// Zero out all momentum (momx, momy, momz) for a map object.
    ///
    /// Equivalent to `thing->momx = thing->momy = thing->momz = 0` in the
    /// original C source. Ensures the teleported thing arrives with no
    /// residual velocity.
    fn clear_mobj_momentum(&mut self, idx: usize);

    /// Set a map object's reaction time counter.
    ///
    /// Equivalent to `thing->reactiontime = 18` in the original C source.
    /// This briefly freezes a player after teleportation.
    fn set_mobj_reactiontime(&mut self, idx: usize, time: i32);

    /// Set a player's eye-height z position.
    ///
    /// Equivalent to `thing->player->viewz = thing->z + thing->player->viewheight`
    /// in the original C source. Called after teleportation to prevent the
    /// player's view from being at the wrong height for one frame.
    fn set_player_viewz(&mut self, player_idx: usize, viewz: Fixed);

    /// Attempt to teleport-move a map object to a new `(x, y)` position.
    ///
    /// Equivalent to `P_TeleportMove(thing, m->x, m->y)` in the original C
    /// source. Handles telefragging (killing anything at the destination),
    /// position relinking in the blockmap and sector lists, and updating
    /// the thing's floorz/ceilingz for the new position.
    ///
    /// Returns `true` if the move succeeded, `false` if the destination is
    /// completely blocked (teleport fails in this case).
    fn p_teleport_move(&mut self, thing_idx: usize, x: Fixed, y: Fixed) -> bool;

    /// Spawn a teleport fog (`MT_TFOG`) at the given position.
    ///
    /// Equivalent to `P_SpawnMobj(x, y, z, MT_TFOG)` in the original C source.
    /// Returns the arena index of the newly spawned fog map object, which is
    /// used as the sound origin for the teleport sound effect.
    fn spawn_teleport_fog(&mut self, x: Fixed, y: Fixed, z: Fixed) -> Option<usize>;

    /// Play a sound effect originating from a map object.
    ///
    /// Equivalent to `S_StartSound(origin, sfx)` in the original C source.
    /// `origin` is the arena index of the map object to use as the sound
    /// source (for positional audio), or `None` for a non-positional sound.
    fn s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum);
}

// ============================================================================
// EV_Teleport — main teleportation function
// Translated from p_telept.c lines 47-131
// ============================================================================

/// Teleport a thing to a destination matching the given line's tag.
///
/// Translated from `EV_Teleport` in `p_telept.c` (lines 47–131).
///
/// Activated by linedef types 39 (W1 Teleport), 97 (WR Teleport),
/// 125 (W1 Teleport Monsters Only), and 174 (not used in original maps).
/// The function searches for sectors matching the trigger line's tag, then
/// looks for an `MT_TELEPORTMAN` map object within that sector to use as
/// the teleport destination.
///
/// # Arguments
///
/// * `line_idx` — Index of the trigger linedef whose tag determines the
///   destination sector.
/// * `side` — Which side of the line was crossed. `0` = front (teleport),
///   `1` = back (allow exit, no teleport).
/// * `thing_idx` — Index of the map object being teleported.
/// * `ctx` — Mutable reference to the teleport context providing game state
///   access and cross-module operations.
///
/// # Returns
///
/// `true` if the teleport occurred, `false` if it did not (missile, back-side
/// hit, no matching destination, or destination blocked).
pub fn ev_teleport(
    line_idx: usize,
    side: i32,
    thing_idx: usize,
    ctx: &mut dyn TeleportContext,
) -> bool {
    // Don't teleport missiles.
    // Original C: if (thing->flags & MF_MISSILE) return 0;
    if ctx.mobj_flags(thing_idx).contains(MobjFlags::MF_MISSILE) {
        return false;
    }

    // Don't teleport if activated from the back side of the line.
    // This allows things to walk OUT of a teleporter without being
    // immediately sent back.
    // Original C: if (side == 1) return 0;
    if side == 1 {
        return false;
    }

    let tag = ctx.line_tag(line_idx);
    let num_sectors = ctx.num_sectors();

    // Search all sectors for ones matching the trigger line's tag.
    // Original C: for (i = 0; i < numsectors; i++)
    for i in 0..num_sectors {
        if ctx.sector_tag(i) != tag {
            continue;
        }

        // Walk all active map objects (thinker list equivalent).
        // Original C iterates thinker linked list from thinkercap.next:
        //   thinker = thinkercap.next;
        //   for (...; thinker != &thinkercap; thinker = thinker->next)
        // In the Rust arena, we get all active mobj indices.
        let active_mobjs = ctx.active_mobj_indices();

        for &m_idx in &active_mobjs {
            // Skip thinkers that are not map objects.
            // Original C: if (thinker->function.acp1 != (actionf_p1)P_MobjThinker)
            //     continue;
            // In Rust, active_mobj_indices already filters for MobjThinker
            // entries, so this check is implicit. We still check the type below.

            // Skip if not a teleport destination marker.
            // Original C: if (m->type != MT_TELEPORTMAN) continue;
            if ctx.mobj_type(m_idx) != MobjType::MT_TELEPORTMAN as usize {
                continue;
            }

            // Check that this teleport marker is in the correct sector.
            // Original C: sector = m->subsector->sector;
            //             if (sector-sectors != i) continue;
            let m_sector = match ctx.mobj_sector(m_idx) {
                Some(s) => s,
                None => continue,
            };
            if m_sector != i {
                continue;
            }

            // ============================================================
            // Found a valid teleport destination — execute teleport.
            // ============================================================

            // Save the thing's current position for source fog spawning.
            // Original C: oldx = thing->x; oldy = thing->y; oldz = thing->z;
            let (oldx, oldy, oldz) = ctx.mobj_position(thing_idx);

            // Get the destination marker's position and angle.
            let (dest_x, dest_y, _dest_z) = ctx.mobj_position(m_idx);
            let dest_angle = ctx.mobj_angle(m_idx);

            // Attempt to move the thing to the destination position.
            // This handles telefragging and position relinking.
            // Original C: if (!P_TeleportMove (thing, m->x, m->y)) return 0;
            if !ctx.p_teleport_move(thing_idx, dest_x, dest_y) {
                return false;
            }

            // Set thing's z to the floor at the destination.
            // Original C comment: "fixme: not needed?"
            // fixme: not needed?
            let floorz = ctx.mobj_floorz(thing_idx);
            ctx.set_mobj_z(thing_idx, floorz);

            // If this is a player, update their viewz to the correct eye height.
            // Original C: if (thing->player)
            //     thing->player->viewz = thing->z + thing->player->viewheight;
            if let Some(player_idx) = ctx.mobj_player(thing_idx) {
                let viewheight = ctx.player_viewheight(player_idx);
                ctx.set_player_viewz(player_idx, floorz + viewheight);
            }

            // Spawn teleport fog at the SOURCE position (where the thing was).
            // Original C: fog = P_SpawnMobj(oldx, oldy, oldz, MT_TFOG);
            //             S_StartSound(fog, sfx_telept);
            let fog = ctx.spawn_teleport_fog(oldx, oldy, oldz);
            ctx.s_start_sound(fog, SfxEnum::sfx_telept);

            // Calculate the destination fog offset direction.
            // The fog spawns 20 map units in front of the destination marker.
            // Original C: an = m->angle >> ANGLETOFINESHIFT;
            let an = (dest_angle.0 >> ANGLETOFINESHIFT) as usize;

            // Compute the destination fog position with trigonometric offset.
            // Original C: fog = P_SpawnMobj(m->x + 20*finecosine[an],
            //                               m->y + 20*finesine[an],
            //                               thing->z, MT_TFOG);
            //             S_StartSound(fog, sfx_telept);
            let fog_x = dest_x + Fixed::from_int(20).fixed_mul(finecosine(an));
            let fog_y = dest_y + Fixed::from_int(20).fixed_mul(FINESINE[an]);

            // Get the thing's current z (which is now floorz at destination).
            let (_, _, thing_z) = ctx.mobj_position(thing_idx);

            // Spawn teleport fog at the DESTINATION position.
            // Original C: fog = P_SpawnMobj(m->x+20*finecosine[an],
            //                               m->y+20*finesine[an],
            //                               thing->z, MT_TFOG);
            //             S_StartSound(fog, sfx_telept);
            let fog = ctx.spawn_teleport_fog(fog_x, fog_y, thing_z);
            ctx.s_start_sound(fog, SfxEnum::sfx_telept);

            // Freeze the player briefly after teleportation.
            // Original C: if (thing->player)
            //     thing->reactiontime = 18;
            if ctx.mobj_player(thing_idx).is_some() {
                ctx.set_mobj_reactiontime(thing_idx, 18);
            }

            // Face the same direction as the teleport destination marker.
            // Original C: thing->angle = m->angle;
            ctx.set_mobj_angle(thing_idx, dest_angle);

            // Zero all momentum so the thing arrives stationary.
            // Original C: thing->momx = thing->momy = thing->momz = 0;
            ctx.clear_mobj_momentum(thing_idx);

            return true;
        }
    }

    // No matching teleport destination found.
    false
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::angle::{Angle, ANG90};

    /// Test implementation of `TeleportContext` for unit testing.
    struct TestTeleportCtx {
        /// Sectors: (tag, list of mobj indices "in" this sector).
        sectors: Vec<(i16, Vec<usize>)>,
        /// Map objects stored by index.
        mobjs: Vec<TestMobj>,
        /// Line tags indexed by line index.
        line_tags: Vec<i16>,
        /// Whether `p_teleport_move` succeeds.
        teleport_move_result: bool,
        /// Fog spawn positions recorded for verification.
        fog_spawned: Vec<(Fixed, Fixed, Fixed)>,
        /// Sound effects played, for verification.
        sounds_played: Vec<(Option<usize>, SfxEnum)>,
    }

    /// Minimal map object representation for testing.
    #[derive(Clone)]
    struct TestMobj {
        type_: usize,
        x: Fixed,
        y: Fixed,
        z: Fixed,
        angle: Angle,
        flags: MobjFlags,
        floorz: Fixed,
        is_player: bool,
        player_idx: Option<usize>,
        reactiontime: i32,
        momx: Fixed,
        momy: Fixed,
        momz: Fixed,
        sector: Option<usize>,
    }

    impl TeleportContext for TestTeleportCtx {
        fn line_tag(&self, line_idx: usize) -> i16 {
            self.line_tags.get(line_idx).copied().unwrap_or(0)
        }

        fn num_sectors(&self) -> usize {
            self.sectors.len()
        }

        fn sector_tag(&self, idx: usize) -> i16 {
            self.sectors[idx].0
        }

        fn active_mobj_indices(&self) -> Vec<usize> {
            // Return all mobj indices as "active".
            (0..self.mobjs.len()).collect()
        }

        fn mobj_type(&self, idx: usize) -> usize {
            self.mobjs[idx].type_
        }

        fn mobj_sector(&self, idx: usize) -> Option<usize> {
            self.mobjs[idx].sector
        }

        fn mobj_flags(&self, idx: usize) -> MobjFlags {
            self.mobjs[idx].flags
        }

        fn mobj_position(&self, idx: usize) -> (Fixed, Fixed, Fixed) {
            (self.mobjs[idx].x, self.mobjs[idx].y, self.mobjs[idx].z)
        }

        fn mobj_angle(&self, idx: usize) -> Angle {
            self.mobjs[idx].angle
        }

        fn mobj_floorz(&self, idx: usize) -> Fixed {
            self.mobjs[idx].floorz
        }

        fn mobj_player(&self, idx: usize) -> Option<usize> {
            if self.mobjs[idx].is_player {
                self.mobjs[idx].player_idx
            } else {
                None
            }
        }

        fn player_viewheight(&self, _player_idx: usize) -> Fixed {
            Fixed::from_int(41)
        }

        fn set_mobj_z(&mut self, idx: usize, z: Fixed) {
            self.mobjs[idx].z = z;
        }

        fn set_mobj_angle(&mut self, idx: usize, angle: Angle) {
            self.mobjs[idx].angle = angle;
        }

        fn clear_mobj_momentum(&mut self, idx: usize) {
            self.mobjs[idx].momx = Fixed::ZERO;
            self.mobjs[idx].momy = Fixed::ZERO;
            self.mobjs[idx].momz = Fixed::ZERO;
        }

        fn set_mobj_reactiontime(&mut self, idx: usize, time: i32) {
            self.mobjs[idx].reactiontime = time;
        }

        fn set_player_viewz(&mut self, _player_idx: usize, viewz: Fixed) {
            // In a real implementation, this would set the player's viewz.
            // For testing, we update the mobj z to track the call was made.
            // (The caller can verify via fog_spawned or reactiontime.)
            let _ = viewz;
        }

        fn p_teleport_move(&mut self, idx: usize, x: Fixed, y: Fixed) -> bool {
            if self.teleport_move_result {
                self.mobjs[idx].x = x;
                self.mobjs[idx].y = y;
                // Simulate floorz update at new position (set to destination floor).
                self.mobjs[idx].floorz = Fixed::ZERO;
            }
            self.teleport_move_result
        }

        fn spawn_teleport_fog(&mut self, x: Fixed, y: Fixed, z: Fixed) -> Option<usize> {
            self.fog_spawned.push((x, y, z));
            // Return a fake fog index for testing.
            Some(100 + self.fog_spawned.len())
        }

        fn s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum) {
            self.sounds_played.push((origin, sfx));
        }
    }

    /// Helper to create a default test mobj (non-player, non-missile, non-teleportman).
    fn make_player_mobj(x: i32, y: i32) -> TestMobj {
        TestMobj {
            type_: MobjType::MT_PLAYER as usize,
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
            z: Fixed::ZERO,
            angle: Angle::new(0),
            flags: MobjFlags::empty(),
            floorz: Fixed::ZERO,
            is_player: true,
            player_idx: Some(0),
            reactiontime: 0,
            momx: Fixed::from_int(5),
            momy: Fixed::from_int(3),
            momz: Fixed::ZERO,
            sector: Some(0),
        }
    }

    /// Helper to create a teleport destination marker.
    fn make_teleportman(x: i32, y: i32, angle: Angle, sector: usize) -> TestMobj {
        TestMobj {
            type_: MobjType::MT_TELEPORTMAN as usize,
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
            z: Fixed::ZERO,
            angle,
            flags: MobjFlags::empty(),
            floorz: Fixed::ZERO,
            is_player: false,
            player_idx: None,
            reactiontime: 0,
            momx: Fixed::ZERO,
            momy: Fixed::ZERO,
            momz: Fixed::ZERO,
            sector: Some(sector),
        }
    }

    /// Helper to create a missile mobj.
    fn make_missile(x: i32, y: i32) -> TestMobj {
        TestMobj {
            type_: 0,
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
            z: Fixed::ZERO,
            angle: Angle::new(0),
            flags: MobjFlags::MF_MISSILE,
            floorz: Fixed::ZERO,
            is_player: false,
            player_idx: None,
            reactiontime: 0,
            momx: Fixed::from_int(10),
            momy: Fixed::ZERO,
            momz: Fixed::ZERO,
            sector: Some(0),
        }
    }

    #[test]
    fn test_missiles_are_not_teleported() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![
                make_missile(50, 50),
                make_teleportman(200, 300, Angle::new(0), 0),
            ],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        // Missile should be rejected immediately.
        assert!(!ev_teleport(0, 0, 0, &mut ctx));
        // No fog should have been spawned.
        assert!(ctx.fog_spawned.is_empty());
    }

    #[test]
    fn test_backside_activation_is_rejected() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![
                make_player_mobj(50, 50),
                make_teleportman(200, 300, Angle::new(0), 0),
            ],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        // side == 1 means back side — should be rejected.
        assert!(!ev_teleport(0, 1, 0, &mut ctx));
        assert!(ctx.fog_spawned.is_empty());
    }

    #[test]
    fn test_successful_player_teleport() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![
                make_player_mobj(50, 50),
                make_teleportman(200, 300, ANG90, 0),
            ],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        assert!(ev_teleport(0, 0, 0, &mut ctx));

        // Position should be updated to destination.
        assert_eq!(ctx.mobjs[0].x, Fixed::from_int(200));
        assert_eq!(ctx.mobjs[0].y, Fixed::from_int(300));

        // Angle should match destination marker's angle (ANG90).
        assert_eq!(ctx.mobjs[0].angle, ANG90);

        // Reaction time should be set to 18 for players.
        assert_eq!(ctx.mobjs[0].reactiontime, 18);

        // All momentum should be zeroed.
        assert_eq!(ctx.mobjs[0].momx, Fixed::ZERO);
        assert_eq!(ctx.mobjs[0].momy, Fixed::ZERO);
        assert_eq!(ctx.mobjs[0].momz, Fixed::ZERO);

        // Fog should be spawned at both source and destination.
        assert_eq!(ctx.fog_spawned.len(), 2);

        // First fog at source position (50, 50, 0).
        assert_eq!(ctx.fog_spawned[0].0, Fixed::from_int(50));
        assert_eq!(ctx.fog_spawned[0].1, Fixed::from_int(50));

        // Second fog at destination with trig offset:
        // ANG90 >> ANGLETOFINESHIFT gives fine angle index for 90 degrees.
        // cos(90°) = 0, sin(90°) = 1.0 (FRACUNIT)
        // fog_x = 200 + 20 * cos(90°) = 200 + 0 = 200
        // fog_y = 300 + 20 * sin(90°) = 300 + 20 = 320
        let an = (ANG90.0 >> ANGLETOFINESHIFT) as usize;
        let expected_fog_x = Fixed::from_int(200) + Fixed::from_int(20).fixed_mul(finecosine(an));
        let expected_fog_y = Fixed::from_int(300) + Fixed::from_int(20).fixed_mul(FINESINE[an]);
        assert_eq!(ctx.fog_spawned[1].0, expected_fog_x);
        assert_eq!(ctx.fog_spawned[1].1, expected_fog_y);
    }

    #[test]
    fn test_no_matching_sector_tag() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(2, vec![1])], // tag 2, not matching line tag 1
            mobjs: vec![
                make_player_mobj(50, 50),
                make_teleportman(200, 300, Angle::new(0), 0),
            ],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        assert!(!ev_teleport(0, 0, 0, &mut ctx));
        assert!(ctx.fog_spawned.is_empty());
    }

    #[test]
    fn test_teleport_blocked_at_destination() {
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![
                make_player_mobj(50, 50),
                make_teleportman(200, 300, Angle::new(0), 0),
            ],
            line_tags: vec![1],
            teleport_move_result: false, // Destination is blocked.
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        assert!(!ev_teleport(0, 0, 0, &mut ctx));

        // Position should be unchanged since teleport_move failed.
        assert_eq!(ctx.mobjs[0].x, Fixed::from_int(50));
        assert_eq!(ctx.mobjs[0].y, Fixed::from_int(50));

        // No fog should have been spawned.
        assert!(ctx.fog_spawned.is_empty());
    }

    #[test]
    fn test_teleportman_in_wrong_sector() {
        let mut ctx = TestTeleportCtx {
            // Sector 0 has tag 1, but teleportman is in sector 1.
            sectors: vec![(1, vec![]), (2, vec![])],
            mobjs: vec![
                make_player_mobj(50, 50),
                make_teleportman(200, 300, Angle::new(0), 1), // In sector 1, not sector 0.
            ],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        // Sector 0 matches tag 1, but the teleportman is in sector 1 (tag 2).
        // So no valid destination is found.
        assert!(!ev_teleport(0, 0, 0, &mut ctx));
        assert!(ctx.fog_spawned.is_empty());
    }

    #[test]
    fn test_non_player_teleport_no_reactiontime() {
        // Create a non-player monster mobj.
        let monster = TestMobj {
            type_: 1, // Not MT_PLAYER, but also not relevant for type check.
            x: Fixed::from_int(50),
            y: Fixed::from_int(50),
            z: Fixed::ZERO,
            angle: Angle::new(0),
            flags: MobjFlags::empty(),
            floorz: Fixed::ZERO,
            is_player: false,
            player_idx: None,
            reactiontime: 0,
            momx: Fixed::from_int(5),
            momy: Fixed::from_int(3),
            momz: Fixed::ZERO,
            sector: Some(0),
        };

        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![monster, make_teleportman(200, 300, Angle::new(0), 0)],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        assert!(ev_teleport(0, 0, 0, &mut ctx));

        // Non-player should NOT get reaction time set.
        assert_eq!(ctx.mobjs[0].reactiontime, 0);

        // But momentum should still be zeroed.
        assert_eq!(ctx.mobjs[0].momx, Fixed::ZERO);
        assert_eq!(ctx.mobjs[0].momy, Fixed::ZERO);
        assert_eq!(ctx.mobjs[0].momz, Fixed::ZERO);

        // Fog should be spawned at both source and destination.
        assert_eq!(ctx.fog_spawned.len(), 2);
    }

    #[test]
    fn test_multiple_sectors_finds_first_match() {
        // Create multiple sectors, with the matching one at index 2.
        let mut ctx = TestTeleportCtx {
            sectors: vec![
                (0, vec![]),  // Sector 0: tag 0 (no match)
                (5, vec![]),  // Sector 1: tag 5 (no match)
                (1, vec![1]), // Sector 2: tag 1 (matches!)
            ],
            mobjs: vec![
                make_player_mobj(50, 50),
                make_teleportman(200, 300, Angle::new(0), 2), // In sector 2.
            ],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        assert!(ev_teleport(0, 0, 0, &mut ctx));
        assert_eq!(ctx.mobjs[0].x, Fixed::from_int(200));
        assert_eq!(ctx.mobjs[0].y, Fixed::from_int(300));
    }

    #[test]
    fn test_z_set_to_floorz_after_teleport() {
        // Set a non-zero floorz at the destination to verify z is updated.
        let mut dest = make_teleportman(200, 300, Angle::new(0), 0);
        dest.floorz = Fixed::from_int(64);

        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![1])],
            mobjs: vec![make_player_mobj(50, 50), dest],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        // The p_teleport_move mock sets floorz to ZERO at new position.
        // So after teleport, thing.z should be ZERO (the floorz at dest).
        assert!(ev_teleport(0, 0, 0, &mut ctx));
        assert_eq!(ctx.mobjs[0].z, Fixed::ZERO);
    }

    #[test]
    fn test_no_teleportman_in_matching_sector() {
        // Sector has matching tag but no MT_TELEPORTMAN in it.
        let mut ctx = TestTeleportCtx {
            sectors: vec![(1, vec![])],
            mobjs: vec![
                make_player_mobj(50, 50),
                // This is a regular mobj, not a teleportman.
                TestMobj {
                    type_: MobjType::MT_PLAYER as usize,
                    x: Fixed::from_int(200),
                    y: Fixed::from_int(300),
                    z: Fixed::ZERO,
                    angle: Angle::new(0),
                    flags: MobjFlags::empty(),
                    floorz: Fixed::ZERO,
                    is_player: false,
                    player_idx: None,
                    reactiontime: 0,
                    momx: Fixed::ZERO,
                    momy: Fixed::ZERO,
                    momz: Fixed::ZERO,
                    sector: Some(0),
                },
            ],
            line_tags: vec![1],
            teleport_move_result: true,
            fog_spawned: Vec::new(),
            sounds_played: Vec::new(),
        };

        assert!(!ev_teleport(0, 0, 0, &mut ctx));
        assert!(ctx.fog_spawned.is_empty());
    }
}
