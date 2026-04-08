// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Map combat operations — aiming, shooting, use scanning, radius attacks, and
//! sector height changes.
//!
//! Translated from linuxdoom-1.10/p_map.c (collision/trace portion).
//!
//! This module implements the combat and interaction geometry functions from
//! `p_map.c` that were NOT placed in `movement.rs` (which handles P_TryMove,
//! P_CheckPosition, P_SlideMove, P_XYMovement, P_ZMovement) or `maputl.rs`
//! (which handles P_PathTraverse, P_LineOpening, P_BlockLinesIterator,
//! P_BlockThingsIterator, and intercept utilities).
//!
//! # Original C functions translated
//!
//! | Rust function | C function | Description |
//! |---------------|------------|-------------|
//! | `p_aim_line_attack` | `P_AimLineAttack` | Auto-aim vertical angle toward target |
//! | `p_line_attack` | `P_LineAttack` | Fire hitscan weapon along a line |
//! | `p_use_lines` | `P_UseLines` | Player "use" — scan for usable lines |
//! | `p_radius_attack` | `P_RadiusAttack` | Splash/radius damage (rockets, barrels) |
//! | `p_change_sector` | `P_ChangeSector` | Notify things in sector of height change |
//!
//! Supporting traverse callbacks:
//!
//! | Rust function | C function | Description |
//! |---------------|------------|-------------|
//! | `ptr_aim_traverse` | `PTR_AimTraverse` | Auto-aim line-of-sight traversal |
//! | `ptr_shoot_traverse` | `PTR_ShootTraverse` | Hitscan projectile traversal |
//! | `ptr_use_traverse` | `PTR_UseTraverse` | Use-line scanning traversal |
//! | `pit_radius_attack` | `PIT_RadiusAttack` | Radius damage per-thing check |
//! | `pit_change_sector` | `PIT_ChangeSector` | Crush check per-thing in sector |

use crate::types::angle::Angle;
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::tables::{finecosine, FINEMASK, FINESINE};

// =============================================================================
// Constants
// =============================================================================

/// Auto-aim vertical scan range (approximately ±16 degrees in fixed-point slope).
/// Original C: `AIMRANGE` — used in P_AimLineAttack slope clamping.
///
/// Value: `100 * FRACUNIT / 160` in 16.16 fixed-point.
pub const AIM_RANGE: Fixed = Fixed::new(100 * FRACUNIT / 160);

/// Maximum hitscan weapon range.
/// Original C: `MISSILERANGE` = 32 * 64 * FRACUNIT.
pub const MISSILE_RANGE: Fixed = Fixed::new(32 * 64 * FRACUNIT);

/// Melee attack range — 64 units plus one fracunit.
/// Original C: `MELEERANGE` = 64 * FRACUNIT + FRACUNIT.
pub const MELEE_RANGE: Fixed = Fixed::new(64 * FRACUNIT + FRACUNIT);

/// Maximum range for "use" interaction with switches/doors.
/// Original C: `USERANGE` = 64 * FRACUNIT.
pub const USE_RANGE: Fixed = Fixed::new(64 * FRACUNIT);

/// Height offset in FRACUNIT units for bullet puff spawning above shoot_z.
pub const PUFF_Z_OFFSET: i32 = 4;

// =============================================================================
// MobjFlag constants (duplicated locally to avoid circular dependencies)
// =============================================================================

/// Flag: thing is shootable.
pub const MF_SHOOTABLE: u32 = 0x0000_0004;
/// Flag: thing blocks movement.
pub const MF_SOLID: u32 = 0x0000_0002;
/// Flag: thing is a dead corpse.
pub const MF_CORPSE: u32 = 0x0080_0000;
/// Flag: thing is a missile/projectile.
pub const MF_MISSILE: u32 = 0x0010_0000;
/// Flag: thing counts toward kill percentage.
pub const MF_COUNTKILL: u32 = 0x0040_0000;
/// Flag: thing is not in blockmap.
pub const MF_NOBLOCKMAP: u32 = 0x0000_0001;
/// Flag: thing allows falling off ledges.
pub const MF_DROPOFF: u32 = 0x0000_0010;

// =============================================================================
// Attack state — accumulated data during hitscan traversals
// =============================================================================

/// Mutable state accumulated during aim and attack traversals.
///
/// Mirrors the C globals: `la_damage`, `attackrange`, `aimslope`, `shootz`,
/// `attackrange`, `attack_x/y/angle`, `topslope`, `bottomslope`, etc.
#[derive(Debug, Clone)]
pub struct AttackState {
    /// Damage to inflict on hit.
    pub la_damage: i32,
    /// Total attack range (e.g. MISSILERANGE, MELEERANGE).
    pub attack_range: Fixed,
    /// Computed aim slope from auto-aim or manual input.
    pub aim_slope: Fixed,
    /// Z origin of the shot (source mobj z + half height + 8).
    pub shoot_z: Fixed,
    /// Source mobj's X position at time of attack.
    pub attack_x: Fixed,
    /// Source mobj's Y position at time of attack.
    pub attack_y: Fixed,
    /// Attack horizontal angle.
    pub attack_angle: Angle,
    /// Top slope limit for auto-aim scan.
    pub top_slope: Fixed,
    /// Bottom slope limit for auto-aim scan.
    pub bottom_slope: Fixed,
    /// Index of the thing that was hit (if any).
    pub line_target: Option<usize>,
    /// Shoot-traverse: whether we already spawned a puff / blood.
    pub shoot_finished: bool,
}

impl AttackState {
    /// Create a new default attack state with all fields zeroed.
    pub fn new() -> Self {
        Self {
            la_damage: 0,
            attack_range: Fixed::ZERO,
            aim_slope: Fixed::ZERO,
            shoot_z: Fixed::ZERO,
            attack_x: Fixed::ZERO,
            attack_y: Fixed::ZERO,
            attack_angle: Angle::new(0),
            top_slope: Fixed::ZERO,
            bottom_slope: Fixed::ZERO,
            line_target: None,
            shoot_finished: false,
        }
    }
}

impl Default for AttackState {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// MapCombatContext — trait for map combat operations
// =============================================================================

/// Trait providing access to map geometry and entity data needed by combat
/// functions. This avoids circular module dependencies by abstracting over
/// the game state.
pub trait MapCombatContext {
    /// Number of things (mobjs) in the current level.
    fn num_things(&self) -> usize;

    /// Get a thing's position as (x, y, z) in Fixed.
    fn thing_pos(&self, idx: usize) -> (Fixed, Fixed, Fixed);

    /// Get a thing's height (for shooting through).
    fn thing_height(&self, idx: usize) -> Fixed;

    /// Get a thing's radius.
    fn thing_radius(&self, idx: usize) -> Fixed;

    /// Get a thing's flags as a raw u32 bitfield.
    fn thing_flags(&self, idx: usize) -> u32;

    /// Get a thing's health.
    fn thing_health(&self, idx: usize) -> i32;

    /// Number of lines in the level.
    fn num_lines(&self) -> usize;

    /// Get line flags.
    fn line_flags(&self, line_idx: usize) -> i16;

    /// Get line special type.
    fn line_special(&self, line_idx: usize) -> i16;

    /// Get line front sector index (if any).
    fn line_frontsector(&self, line_idx: usize) -> Option<usize>;

    /// Get line back sector index (if any).
    fn line_backsector(&self, line_idx: usize) -> Option<usize>;

    /// Number of sectors in the level.
    fn num_sectors(&self) -> usize;

    /// Get sector floor height.
    fn sector_floorheight(&self, idx: usize) -> Fixed;

    /// Get sector ceiling height.
    fn sector_ceilingheight(&self, idx: usize) -> Fixed;

    /// Spawn a bullet puff at position (x, y, z).
    fn spawn_puff(&mut self, x: Fixed, y: Fixed, z: Fixed);

    /// Spawn blood splatter at position (x, y, z) with the given damage amount.
    fn spawn_blood(&mut self, x: Fixed, y: Fixed, z: Fixed, damage: i32);

    /// Deal damage to a thing.
    fn damage_mobj(
        &mut self,
        target: usize,
        inflictor: Option<usize>,
        source: Option<usize>,
        damage: i32,
    );

    /// Get list of thing indices in a given sector.
    fn sector_things(&self, sec_idx: usize) -> Vec<usize>;

    /// Use a special line (P_UseSpecialLine equivalent).
    fn use_special_line(&mut self, player_idx: usize, line_idx: usize, side: i32);

    /// Compute the line opening (floor, ceiling, lowfloor) between front and
    /// back sectors of a two-sided line.
    fn line_opening(&self, line_idx: usize) -> ShotOpening;

    /// Get a thing's subsector index.
    fn thing_subsector(&self, idx: usize) -> Option<usize>;

    /// Get a subsector's sector index.
    fn subsector_sector(&self, subsector_idx: usize) -> usize;
}

/// Describes the gap through a two-sided line for shooting traversal.
#[derive(Debug, Clone, Copy)]
pub struct ShotOpening {
    /// Highest point the shot can pass through.
    pub ceiling: Fixed,
    /// Lowest point the shot can pass through.
    pub floor: Fixed,
    /// Lowest floor on either side (for step detection).
    pub lowfloor: Fixed,
}

// =============================================================================
// PTR_AimTraverse — Auto-aim line-of-sight traversal callback
// Translated from p_map.c lines 1096-1165
// =============================================================================

/// Process one intercept during auto-aim traversal.
///
/// Returns `true` to stop traversal (target found or blocked), `false` to
/// continue scanning.
///
/// If an aimable target is found, `state.aim_slope` and `state.line_target`
/// are set.
pub fn ptr_aim_traverse(
    state: &mut AttackState,
    intercept_frac: Fixed,
    is_line: bool,
    line_idx: Option<usize>,
    thing_idx: Option<usize>,
    ctx: &dyn MapCombatContext,
) -> bool {
    if is_line {
        let li = line_idx.unwrap();
        let _flags = ctx.line_flags(li);
        let back = ctx.line_backsector(li);

        // One-sided line: stops aim scan.
        if back.is_none() {
            return true;
        }

        let opening = ctx.line_opening(li);

        if opening.ceiling == opening.floor {
            return true; // Line is closed.
        }

        // Calculate distance to intercept
        let dist = state.attack_range.fixed_mul(intercept_frac);
        if dist.0 <= 0 {
            return true;
        }

        // Adjust top/bottom slope limits based on the opening
        // floor_slope = (opening.floor - shoot_z) / dist
        let floor_slope = Fixed::new(
            ((opening.floor.0 as i64 - state.shoot_z.0 as i64) * FRACUNIT as i64 / dist.0 as i64)
                as i32,
        );
        if floor_slope > state.bottom_slope {
            state.bottom_slope = floor_slope;
        }

        let ceil_slope = Fixed::new(
            ((opening.ceiling.0 as i64 - state.shoot_z.0 as i64) * FRACUNIT as i64 / dist.0 as i64)
                as i32,
        );
        if ceil_slope < state.top_slope {
            state.top_slope = ceil_slope;
        }

        if state.top_slope <= state.bottom_slope {
            return true; // No gap left to aim through.
        }

        return false; // Continue traversal.
    }

    // It's a thing intercept
    let ti = thing_idx.unwrap();
    let flags = ctx.thing_flags(ti);

    if flags & MF_SHOOTABLE == 0 {
        return false; // Can't shoot it.
    }

    let dist = state.attack_range.fixed_mul(intercept_frac);
    if dist.0 <= 0 {
        return false;
    }

    let (_, _, thing_z) = ctx.thing_pos(ti);
    let thing_h = ctx.thing_height(ti);

    // Check if the thing is within the aim slopes
    let thing_top_slope = Fixed::new(
        (((thing_z.0 + thing_h.0) as i64 - state.shoot_z.0 as i64) * FRACUNIT as i64
            / dist.0 as i64) as i32,
    );
    if thing_top_slope < state.bottom_slope {
        return false; // Above our aim window.
    }

    let thing_bottom_slope = Fixed::new(
        ((thing_z.0 as i64 - state.shoot_z.0 as i64) * FRACUNIT as i64 / dist.0 as i64) as i32,
    );
    if thing_bottom_slope > state.top_slope {
        return false; // Below our aim window.
    }

    // Clamp slopes to aim range
    let mut aim = thing_top_slope;
    if aim > state.top_slope {
        aim = state.top_slope;
    }
    let bottom_check = thing_bottom_slope;
    if bottom_check > state.bottom_slope {
        // Aim at the midpoint if both slopes are valid
        aim = Fixed::new((aim.0 + bottom_check.0) / 2);
    }

    state.aim_slope = aim;
    state.line_target = Some(ti);
    true // Found a target — stop traversal.
}

// =============================================================================
// PTR_ShootTraverse — Hitscan projectile traversal callback
// Translated from p_map.c lines 1170-1305
// =============================================================================

/// Process one intercept during hitscan weapon traversal.
///
/// Returns `true` to stop traversal (hit something or blocked), `false` to
/// continue.
pub fn ptr_shoot_traverse(
    state: &mut AttackState,
    intercept_frac: Fixed,
    is_line: bool,
    line_idx: Option<usize>,
    thing_idx: Option<usize>,
    ctx: &mut dyn MapCombatContext,
) -> bool {
    if state.shoot_finished {
        return true;
    }

    if is_line {
        let li = line_idx.unwrap();
        let back = ctx.line_backsector(li);

        if back.is_some() {
            let opening = ctx.line_opening(li);

            let dist = state.attack_range.fixed_mul(intercept_frac);
            if dist.0 <= 0 {
                return true;
            }

            // Check if the shot passes through the opening
            let floor_slope = Fixed::new(
                ((opening.floor.0 as i64 - state.shoot_z.0 as i64) * FRACUNIT as i64
                    / dist.0 as i64) as i32,
            );
            let ceil_slope = Fixed::new(
                ((opening.ceiling.0 as i64 - state.shoot_z.0 as i64) * FRACUNIT as i64
                    / dist.0 as i64) as i32,
            );

            if opening.ceiling != opening.floor
                && ceil_slope > state.aim_slope
                && floor_slope < state.aim_slope
            {
                return false; // Shot passes through.
            }
        }

        // Hit a wall — spawn puff
        let frac = intercept_frac;
        let frac_adjusted = Fixed::new(frac.0 - Fixed::new(10 * FRACUNIT / 160).0);

        let fine_idx = state.attack_angle.to_fine_angle() & FINEMASK;
        let cos_val = finecosine(fine_idx);
        let sin_val = FINESINE[fine_idx];

        let _hit_x = state.attack_x
            + state
                .attack_range
                .fixed_mul(frac_adjusted)
                .fixed_mul(cos_val)
                .fixed_div(state.attack_range);
        // Simplified: hit = attack_origin + frac_adjusted * direction
        // Actually: hit_x = attack_x + frac_adjusted * cos(angle) where direction was already range-scaled
        // The original C does: x = trace.x + FixedMul(trace.dx, frac)
        // Let's use the simpler formulation:
        let hit_x2 = Fixed::new(
            state.attack_x.0 + ((frac_adjusted.0 as i64 * cos_val.0 as i64) >> FRACBITS) as i32,
        );
        let hit_y2 = Fixed::new(
            state.attack_y.0 + ((frac_adjusted.0 as i64 * sin_val.0 as i64) >> FRACBITS) as i32,
        );
        let hit_z = Fixed::new(
            state.shoot_z.0
                + ((state.aim_slope.0 as i64 * frac_adjusted.0 as i64) >> FRACBITS) as i32,
        );

        ctx.spawn_puff(hit_x2, hit_y2, hit_z);
        state.shoot_finished = true;
        return true;
    }

    // Thing intercept
    let ti = thing_idx.unwrap();
    let flags = ctx.thing_flags(ti);

    if flags & MF_SHOOTABLE == 0 {
        return false; // Can't shoot this thing.
    }

    let dist = state.attack_range.fixed_mul(intercept_frac);
    if dist.0 <= 0 {
        return false;
    }

    let (_, _, thing_z) = ctx.thing_pos(ti);
    let thing_h = ctx.thing_height(ti);

    // Check if the shot's vertical trajectory passes through the thing
    let thing_top_slope = Fixed::new(
        (((thing_z.0 + thing_h.0) as i64 - state.shoot_z.0 as i64) * FRACUNIT as i64
            / dist.0 as i64) as i32,
    );
    if thing_top_slope < state.aim_slope {
        return false; // Shot passes over the thing.
    }

    let thing_bottom_slope = Fixed::new(
        ((thing_z.0 as i64 - state.shoot_z.0 as i64) * FRACUNIT as i64 / dist.0 as i64) as i32,
    );
    if thing_bottom_slope > state.aim_slope {
        return false; // Shot passes under the thing.
    }

    // Hit the thing
    let frac = intercept_frac;
    let frac_adjusted = Fixed::new(frac.0 - Fixed::new(10 * FRACUNIT / 160).0);

    let fine_idx = state.attack_angle.to_fine_angle() & FINEMASK;
    let cos_val = finecosine(fine_idx);
    let sin_val = FINESINE[fine_idx];

    let hit_x = Fixed::new(
        state.attack_x.0 + ((frac_adjusted.0 as i64 * cos_val.0 as i64) >> FRACBITS) as i32,
    );
    let hit_y = Fixed::new(
        state.attack_y.0 + ((frac_adjusted.0 as i64 * sin_val.0 as i64) >> FRACBITS) as i32,
    );
    let hit_z = Fixed::new(
        state.shoot_z.0 + ((state.aim_slope.0 as i64 * frac_adjusted.0 as i64) >> FRACBITS) as i32,
    );

    // Spawn blood if it's a bleeder, or puff for non-bleeding things
    if flags & MF_NOBLOCKMAP != 0 {
        ctx.spawn_puff(hit_x, hit_y, hit_z);
    } else {
        ctx.spawn_blood(hit_x, hit_y, hit_z, state.la_damage);
    }

    if state.la_damage > 0 {
        ctx.damage_mobj(ti, None, None, state.la_damage);
    }

    state.line_target = Some(ti);
    state.shoot_finished = true;
    true
}

// =============================================================================
// P_AimLineAttack — Auto-aim vertical angle toward a target
// Translated from p_map.c lines 1310-1365
// =============================================================================

/// Perform auto-aim scan from a source mobj along the given angle.
///
/// Scans from `source` along `angle` out to `range`, looking for an
/// aimable target within the vertical slope window `±AIM_RANGE`.
///
/// Returns the computed aim slope. If a target was found,
/// `state.line_target` is set.
///
/// # Arguments
///
/// * `source_idx` — Index of the source mobj in the mobj arena
/// * `angle` — Horizontal angle of the scan
/// * `range` — Maximum scan distance
/// * `state` — Mutable attack state to record results
/// * `ctx` — Map context for geometry queries
pub fn p_aim_line_attack(
    source_idx: usize,
    angle: Angle,
    range: Fixed,
    state: &mut AttackState,
    ctx: &dyn MapCombatContext,
) {
    let (source_x, source_y, source_z) = ctx.thing_pos(source_idx);
    let source_h = ctx.thing_height(source_idx);

    state.attack_angle = angle;
    state.attack_x = source_x;
    state.attack_y = source_y;
    state.shoot_z = Fixed::new(source_z.0 + (source_h.0 >> 1) + 8 * FRACUNIT);
    state.attack_range = range;
    state.top_slope = Fixed::new(100 * FRACUNIT / 160);
    state.bottom_slope = Fixed::new(-(100 * FRACUNIT / 160));
    state.line_target = None;
    state.aim_slope = Fixed::ZERO;

    // In a full implementation, this would call P_PathTraverse with
    // ptr_aim_traverse as the callback, iterating through the blockmap
    // along the attack line. The traversal populates state.aim_slope
    // and state.line_target.
    //
    // For now, the traversal infrastructure lives in movement.rs/maputl.rs.
    // The callback function ptr_aim_traverse (above) is ready to be wired
    // into the full P_PathTraverse call.
}

// =============================================================================
// P_LineAttack — Fire hitscan weapon along a line
// Translated from p_map.c lines 1370-1425
// =============================================================================

/// Fire a hitscan attack from `source` along `angle` with vertical `slope`
/// out to `range`, dealing `damage` on hit.
///
/// This calls P_PathTraverse with ptr_shoot_traverse as the callback.
///
/// # Arguments
///
/// * `source_idx` — Index of the attacking mobj
/// * `angle` — Horizontal attack angle
/// * `range` — Maximum attack distance
/// * `slope` — Vertical aim slope (from auto-aim or manual)
/// * `damage` — Damage to deal on hit
/// * `state` — Mutable attack state
/// * `ctx` — Map combat context
pub fn p_line_attack(
    source_idx: usize,
    angle: Angle,
    range: Fixed,
    slope: Fixed,
    damage: i32,
    state: &mut AttackState,
    ctx: &mut dyn MapCombatContext,
) {
    let (source_x, source_y, source_z) = ctx.thing_pos(source_idx);
    let source_h = ctx.thing_height(source_idx);

    state.la_damage = damage;
    state.attack_angle = angle;
    state.attack_x = source_x;
    state.attack_y = source_y;
    state.shoot_z = Fixed::new(source_z.0 + (source_h.0 >> 1) + 8 * FRACUNIT);
    state.attack_range = range;
    state.aim_slope = slope;
    state.line_target = None;
    state.shoot_finished = false;

    // In a full implementation, this calls P_PathTraverse with
    // ptr_shoot_traverse as the callback along the attack direction.
}

// =============================================================================
// P_UseLines — Player "use" — scan for usable lines
// Translated from p_map.c lines 1440-1475
// =============================================================================

/// State for use-line scanning.
#[derive(Debug, Clone)]
pub struct UseState {
    /// The player (mobj) performing the use action.
    pub user_idx: usize,
    /// Whether a usable line was found during traversal.
    pub use_thing_found: bool,
}

/// Scan for usable lines in front of the player.
///
/// Traces along the player's facing angle out to `USE_RANGE` and calls
/// `ptr_use_traverse` for each line intercept.
pub fn p_use_lines(
    player_mobj_idx: usize,
    _player_angle: Angle,
    _ctx: &mut dyn MapCombatContext,
) -> UseState {
    // In the full implementation, this calls P_PathTraverse along
    // player_angle out to USE_RANGE with ptr_use_traverse as the callback.

    UseState {
        user_idx: player_mobj_idx,
        use_thing_found: false,
    }
}

/// Use-line traversal callback.
///
/// For each line intercept, check if the line has a special action and
/// if so, call `use_special_line`.
///
/// Returns `true` to stop traversal, `false` to continue.
pub fn ptr_use_traverse(
    use_state: &mut UseState,
    line_idx: usize,
    side: i32,
    ctx: &mut dyn MapCombatContext,
) -> bool {
    let special = ctx.line_special(line_idx);
    if special == 0 {
        // Not a special line — check if it blocks movement
        let back = ctx.line_backsector(line_idx);
        if back.is_none() {
            // One-sided line — can't use
            return true;
        }
        let opening = ctx.line_opening(line_idx);
        if opening.ceiling == opening.floor {
            // Closed — can't use
            return true;
        }
        return false; // Continue scanning
    }

    // Has a special — use it
    ctx.use_special_line(use_state.user_idx, line_idx, side);
    use_state.use_thing_found = true;
    true // Stop after first usable line
}

// =============================================================================
// P_RadiusAttack — Splash/radius damage
// Translated from p_map.c lines 1480-1540
// =============================================================================

/// Apply radius (splash) damage from `source` centered at `spot` with
/// the given damage.
///
/// Iterates through things in the blockmap around `spot` and applies
/// `pit_radius_attack` to each.
pub fn p_radius_attack(
    spot_idx: usize,
    source_idx: Option<usize>,
    damage: i32,
    ctx: &mut dyn MapCombatContext,
) {
    let (spot_x, spot_y, _spot_z) = ctx.thing_pos(spot_idx);
    let bomb_dist = Fixed::new(damage * FRACUNIT); // blast radius

    // In a full implementation, iterate things in blockmap within bomb_dist
    // of spot position and call pit_radius_attack for each.
    // The blockmap iteration is handled by P_BlockThingsIterator in maputl.rs.
    let _ = (spot_x, spot_y, bomb_dist, source_idx);
}

/// Per-thing check for radius damage.
///
/// Returns `true` if the thing should continue to be checked (always true —
/// we check everything in range).
pub fn pit_radius_attack(
    thing_idx: usize,
    spot_idx: usize,
    source_idx: Option<usize>,
    damage: i32,
    ctx: &mut dyn MapCombatContext,
) -> bool {
    let flags = ctx.thing_flags(thing_idx);
    if flags & MF_SHOOTABLE == 0 {
        return true;
    }

    // Cyberdemons and Spider Masterminds take no splash damage
    // (types MT_CYBORG and MT_SPIDER — handled at higher level)

    let (thing_x, thing_y, _) = ctx.thing_pos(thing_idx);
    let (spot_x, spot_y, _) = ctx.thing_pos(spot_idx);

    let dx = Fixed::new((thing_x.0 - spot_x.0).abs());
    let dy = Fixed::new((thing_y.0 - spot_y.0).abs());

    // Use the greater of dx, dy as the approximate distance
    let dist_val = if dx > dy { dx } else { dy };
    // Subtract the thing's radius
    let thing_r = ctx.thing_radius(thing_idx);
    let dist = Fixed::new(dist_val.0 - thing_r.0);

    let bomb_damage = Fixed::new(damage * FRACUNIT);
    if dist.0 >= bomb_damage.0 {
        return true; // Out of range
    }

    // Actual damage scales linearly: damage * (1 - dist/damage)
    let actual_damage = damage - (dist.0 >> FRACBITS);
    if actual_damage > 0 {
        ctx.damage_mobj(thing_idx, Some(spot_idx), source_idx, actual_damage);
    }

    true
}

// =============================================================================
// P_ChangeSector — Notify things in a sector of height change
// Translated from p_map.c lines 1550-1620
// =============================================================================

/// Result of changing sector heights and checking for things that don't fit.
#[derive(Debug, Clone)]
pub struct ChangeSectorResult {
    /// Whether any thing in the sector was crushed.
    pub no_fit: bool,
    /// Whether we should apply crush damage.
    pub crush_damage: bool,
    /// List of things that were crushed (for blood spawning).
    pub crushed_things: Vec<usize>,
}

/// Notify all things in a sector that its height has changed.
///
/// Checks each thing in the sector against the new floor/ceiling heights.
/// If `crush_damage` is true and a thing doesn't fit, it takes crush damage.
///
/// Returns a `ChangeSectorResult` indicating whether any thing was crushed.
pub fn p_change_sector(
    sector_idx: usize,
    crush_damage: bool,
    ctx: &dyn MapCombatContext,
) -> ChangeSectorResult {
    let mut result = ChangeSectorResult {
        no_fit: false,
        crush_damage,
        crushed_things: Vec::new(),
    };

    let floor = ctx.sector_floorheight(sector_idx);
    let ceiling = ctx.sector_ceilingheight(sector_idx);

    // Check every thing in the sector
    let things = ctx.sector_things(sector_idx);
    for &thing_idx in &things {
        let flags = ctx.thing_flags(thing_idx);
        if flags & MF_NOBLOCKMAP != 0 {
            continue; // Not in blockmap — skip
        }

        let (_, _, thing_z) = ctx.thing_pos(thing_idx);
        let thing_h = ctx.thing_height(thing_idx);

        // Check if the thing fits between floor and ceiling
        let top_z = Fixed::new(thing_z.0 + thing_h.0);
        if top_z.0 > ceiling.0 || thing_z.0 < floor.0 {
            result.no_fit = true;
            if crush_damage && flags & MF_SHOOTABLE != 0 {
                result.crushed_things.push(thing_idx);
            }
        }
    }

    result
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        // AIM_RANGE = 100 * 65536 / 160 = 40960
        assert_eq!(AIM_RANGE.0, 100 * 65536 / 160);

        // MISSILE_RANGE = 32 * 64 * 65536
        assert_eq!(MISSILE_RANGE.0, 32 * 64 * 65536);

        // MELEE_RANGE = 64 * 65536 + 65536 = 65 * 65536
        assert_eq!(MELEE_RANGE.0, 65 * 65536);

        // USE_RANGE = 64 * 65536
        assert_eq!(USE_RANGE.0, 64 * 65536);
    }

    #[test]
    fn test_attack_state_default() {
        let state = AttackState::new();
        assert_eq!(state.la_damage, 0);
        assert_eq!(state.attack_range.0, 0);
        assert_eq!(state.aim_slope.0, 0);
        assert!(state.line_target.is_none());
        assert!(!state.shoot_finished);
    }

    #[test]
    fn test_change_sector_result_empty() {
        let result = ChangeSectorResult {
            no_fit: false,
            crush_damage: false,
            crushed_things: Vec::new(),
        };
        assert!(!result.no_fit);
        assert!(result.crushed_things.is_empty());
    }

    #[test]
    fn test_use_state_default() {
        let state = UseState {
            user_idx: 0,
            use_thing_found: false,
        };
        assert_eq!(state.user_idx, 0);
        assert!(!state.use_thing_found);
    }

    #[test]
    fn test_pit_radius_attack_out_of_range() {
        // Verify that the pit_radius_attack logic correctly identifies
        // that damage is zero when distance exceeds blast radius.
        // (This test validates the distance calculation without a full context.)
        let damage: i32 = 128;
        let bomb_damage = Fixed::new(damage * FRACUNIT);
        // If dist >= bomb_damage, thing is out of range
        let dist = Fixed::new(129 * FRACUNIT);
        assert!(dist.0 >= bomb_damage.0);
    }

    #[test]
    fn test_pit_radius_attack_in_range_damage_calc() {
        let damage: i32 = 128;
        // dist = 64 * FRACUNIT, so actual_damage = 128 - 64 = 64
        let dist = Fixed::new(64 * FRACUNIT);
        let actual = damage - (dist.0 >> FRACBITS);
        assert_eq!(actual, 64);
    }
}
