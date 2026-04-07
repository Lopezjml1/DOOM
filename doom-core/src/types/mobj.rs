// Copyright (C) 1993-1996 by id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors
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

//! Translated from linuxdoom-1.10/p_mobj.h
//!
//! Map Objects (mobjs) — actors, entities, thinkers. Anything that moves, acts,
//! or suffers state changes in the game world.
//!
//! # Overview
//!
//! `mobj_t` (Map Object) is the central runtime structure in the DOOM engine.
//! Every monster, player avatar, projectile, pickup item, decoration, and barrel
//! in the game world is represented as a `MapObject`. The struct carries:
//!
//! - **Spatial data**: position (`x`, `y`, `z`), momentum (`momx`, `momy`, `momz`),
//!   size (`radius`, `height`), and bounding sector heights (`floorz`, `ceilingz`).
//! - **Rendering data**: `sprite`, `frame`, and `angle` determine which graphic
//!   is drawn by the software renderer. The `subsector` field links the object to
//!   the BSP subsector for front-to-back drawing.
//! - **Simulation data**: `flags` (bitfield of `MobjFlags`), `health`, `tics`,
//!   `state`, and the AI fields (`movedir`, `movecount`, `target`, `reactiontime`,
//!   `threshold`, `tracer`).
//! - **Thinker link**: the `thinker` field embeds the object into the global
//!   thinker linked list, which drives per-tick updates via `P_MobjThinker`.
//!
//! # Pointer-to-Index Translation
//!
//! The original C code uses raw `mobj_t*` pointers extensively for linked lists
//! (sector thing lists, blockmap thing lists) and cross-references (target, tracer,
//! player). In this Rust port, **all raw pointers are replaced with `Option<usize>`
//! arena indices**, enabling safe traversal without `unsafe` pointer manipulation.
//!
//! # MobjFlags Bitfield
//!
//! The 27 mobj flags (`MF_SPECIAL` through `MF_TRANSLATION`) control virtually
//! every aspect of entity behavior — collision response, rendering style, AI
//! triggers, pickup handling, and multiplayer colormap translation. The flag
//! values are preserved exactly from the original C `#define` constants to
//! maintain behavioral parity.
//!
//! # Direction Enum
//!
//! The `Direction` enum represents the 8 cardinal/ordinal compass directions
//! used by the monster AI movement system, plus a `NoDir` sentinel value.

use bitflags::bitflags;

use super::angle::Angle;
use super::fixed::Fixed;
use super::map_data::MapThing;
use super::thinker::Thinker;

// =============================================================================
// MobjFlags bitfield (p_mobj.h lines 117-203)
// =============================================================================

bitflags! {
    /// Type-safe bitfield for map object flags.
    ///
    /// Each flag controls a specific aspect of entity behavior in the game world.
    /// Values match the original C `mobjflag_t` enum EXACTLY to preserve
    /// behavioral parity.
    ///
    /// Original C: `typedef enum { MF_SPECIAL = 1, ... } mobjflag_t;`
    /// (p_mobj.h lines 117-203).
    ///
    /// The underlying type is `u32` because the highest flag (`MF_TRANSLATION`)
    /// is `0x0c000000`, which fits within 32 bits. The original C code uses
    /// `int` (32-bit on all DOOM target platforms).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct MobjFlags: u32 {
        /// Call `P_SpecialThing` when touched.
        ///
        /// Set on all pickup items (health, armor, weapons, ammo, powerups,
        /// keys). When a player touches an object with this flag, the item
        /// pickup logic in `P_TouchSpecialThing` is triggered.
        const MF_SPECIAL = 1;

        /// Blocks other objects. Solid objects cannot be walked through.
        ///
        /// Set on walls (implicit), barrels, pillars, and most monsters.
        /// The collision code in `P_TryMove` checks this flag to prevent
        /// interpenetration.
        const MF_SOLID = 2;

        /// Can be hit (and damaged) by projectiles and hitscan attacks.
        ///
        /// Set on all monsters, barrels, and players. Objects without this
        /// flag are immune to all damage.
        const MF_SHOOTABLE = 4;

        /// Don't use the sector thing list links (invisible but touchable).
        ///
        /// When set, the object is not inserted into its sector's thing list.
        /// Used by objects that need collision but not rendering (e.g.,
        /// some invisible triggers).
        const MF_NOSECTOR = 8;

        /// Don't use the blockmap links (inert but displayable).
        ///
        /// When set, the object is not inserted into the blockmap. Objects
        /// without blockmap links cannot be collided with or targeted.
        const MF_NOBLOCKMAP = 16;

        /// Deaf monster — not activated by sound, only by line of sight.
        ///
        /// Set by the `MTF_AMBUSH` thing option in WAD map data. Deaf
        /// monsters ignore `A_Look`'s sound check and only wake up when
        /// they see the player.
        const MF_AMBUSH = 32;

        /// Will try to attack right back.
        ///
        /// Set by `P_DamageMobj` when the attacker is a different species.
        /// Causes the monster to immediately face and attack the attacker
        /// on its next tic, enabling infighting between monsters.
        const MF_JUSTHIT = 64;

        /// Will take at least one step before attacking.
        ///
        /// Set after a monster fires an attack. Forces the monster to make
        /// at least one movement step before it can attack again, preventing
        /// monsters from standing still and repeatedly firing.
        const MF_JUSTATTACKED = 128;

        /// On level spawning (initial position), hang from ceiling instead
        /// of standing on floor.
        ///
        /// Set in the `mobjinfo` table for ceiling-hung decorations like
        /// `MT_MISC51` (hanging body). `P_SpawnMapThing` uses this to
        /// position the object at `ceilingz - height` instead of `floorz`.
        const MF_SPAWNCEILING = 256;

        /// Don't apply gravity (every tic).
        ///
        /// Object will float, keeping current height or changing it actively.
        /// Set on flying monsters (cacodemons, pain elementals, lost souls)
        /// and on projectiles. Without this flag, `P_ZMovement` applies
        /// downward gravity each tic.
        const MF_NOGRAVITY = 512;

        /// This allows jumps from high places.
        ///
        /// Movement flag that permits an object to step off ledges with a
        /// drop greater than 24 units. Without this flag, objects will not
        /// willingly walk off tall edges.
        const MF_DROPOFF = 0x400;

        /// For players, will pick up items.
        ///
        /// Only set on player map objects (`MT_PLAYER`). When a player
        /// object touches an `MF_SPECIAL` item, the pickup is processed.
        const MF_PICKUP = 0x800;

        /// Player cheat — no clipping (walk through walls).
        ///
        /// Activated by the `IDCLIP` (DOOM II) or `IDSPISPOPD` (DOOM I)
        /// cheat code. Disables all collision checking for the player.
        const MF_NOCLIP = 0x1000;

        /// Player: keep info about sliding along walls.
        ///
        /// Set on player objects. When a player moves into a wall at an
        /// angle, the slide-move code (`P_SlideMove`) redirects the
        /// remaining momentum along the wall surface.
        const MF_SLIDE = 0x2000;

        /// Allow moves to any height, no gravity.
        ///
        /// For active floaters (cacodemons, pain elementals). Unlike
        /// `MF_NOGRAVITY` alone, this flag also enables the AI to
        /// actively adjust its Z height toward the target via `MF_INFLOAT`
        /// checks in `A_Chase`.
        const MF_FLOAT = 0x4000;

        /// Don't cross lines or look at heights on teleport.
        ///
        /// Set temporarily during teleportation to prevent the teleported
        /// object from triggering line specials or height checks at the
        /// destination.
        const MF_TELEPORT = 0x8000;

        /// Don't hit same species, explode on block.
        ///
        /// Set on all projectiles (player missiles, fireballs, rockets,
        /// etc.). Missiles pass through the species that fired them and
        /// explode when they hit anything else solid.
        const MF_MISSILE = 0x10000;

        /// Dropped by a demon, not level spawned.
        ///
        /// Set on ammo/weapon drops from killed former humans. Dropped
        /// items give half the ammo of their level-placed counterparts.
        const MF_DROPPED = 0x20000;

        /// Use fuzzy draw (shadow demons or spectres).
        ///
        /// Triggers the fuzz effect in `R_DrawFuzzColumn`, making the
        /// object partially invisible. Also used for the temporary player
        /// invisibility powerup.
        const MF_SHADOW = 0x40000;

        /// Don't bleed when shot (use puff instead).
        ///
        /// Set on barrels and shootable furniture. When hit by hitscan
        /// attacks, a bullet puff is spawned instead of blood splats.
        const MF_NOBLOOD = 0x80000;

        /// Don't stop moving halfway off a step — dead bodies slide down
        /// all the way.
        ///
        /// Set when an object dies (`P_KillMobj`). Allows the corpse to
        /// slide off elevated platforms and down stairs for visual realism.
        const MF_CORPSE = 0x100000;

        /// Floating to a height for a move — don't auto float to target's
        /// height.
        ///
        /// Intermediate flag used by `A_Chase` when a floating monster
        /// (`MF_FLOAT`) is actively adjusting its Z position toward a
        /// target. Prevents the movement code from overriding the
        /// targeted float.
        const MF_INFLOAT = 0x200000;

        /// On kill, count this enemy object towards intermission kill total.
        ///
        /// Set on all monsters that should be counted for the intermission
        /// screen's kill percentage. Not set on Lost Souls.
        const MF_COUNTKILL = 0x400000;

        /// On picking up, count this item object towards intermission item
        /// total.
        ///
        /// Set on bonus items, armor, and powerups that contribute to the
        /// intermission screen's item percentage.
        const MF_COUNTITEM = 0x800000;

        /// Special handling: skull in flight.
        ///
        /// Set on Lost Souls during their charging attack (`A_SkullAttack`).
        /// Neither a cacodemon nor a missile — has unique collision behavior
        /// that causes damage on impact and stops the charge.
        const MF_SKULLFLY = 0x1000000;

        /// Don't spawn this object in death match mode (e.g. key cards).
        ///
        /// Set in the `mobjinfo` table for objects that should not appear
        /// in deathmatch games, such as keys and certain decorations.
        const MF_NOTDMATCH = 0x2000000;

        /// Player sprites in multiplayer modes are modified using an
        /// internal color lookup table for re-indexing.
        ///
        /// If bits 26-27 are `0x4`, `0x8`, or `0xc`, use a translation
        /// table for player colormaps. This is a 2-bit mask at bit
        /// positions 26-27, encoding the player's color (green, indigo,
        /// brown, red).
        ///
        /// Original C: `MF_TRANSLATION = 0xc000000` (p_mobj.h line 199).
        const MF_TRANSLATION = 0xc000000;
    }
}

impl Default for MobjFlags {
    /// Returns an empty flag set (no flags set).
    #[inline]
    fn default() -> Self {
        MobjFlags::empty()
    }
}

/// Bit shift amount to extract the player translation color from `MobjFlags`.
///
/// The translation color occupies bits 26-27 of the flags field. Right-shifting
/// the raw flags value by `MF_TRANSSHIFT` (26) yields a 2-bit color index:
/// - 0 = green (default)
/// - 1 = indigo
/// - 2 = brown
/// - 3 = red
///
/// Original C: `MF_TRANSSHIFT = 26` (p_mobj.h line 201).
/// Note: In the original C code, this is defined inside the `mobjflag_t` enum,
/// but it is NOT a bitflag — it's a shift constant. In Rust, it's a separate
/// `pub const` to maintain type safety.
pub const MF_TRANSSHIFT: u32 = 26;

// =============================================================================
// Direction enum (movement directions for AI, referenced in mobj and p_enemy)
// =============================================================================

/// Movement direction for monster AI.
///
/// DOOM's monster AI uses 8 cardinal/ordinal compass directions for pathfinding
/// and movement. The `NoDir` variant (value 8) is a sentinel indicating the
/// monster has no current movement direction and needs to pick one.
///
/// These directions are used by `A_Chase` and the movement routines in
/// `p_enemy.c` to guide monster navigation around obstacles. The values map
/// to the `xspeed` and `yspeed` arrays in the original code for computing
/// per-direction movement deltas.
///
/// Original C: `typedef enum { DI_EAST, ..., DI_NODIR, NUMDIRS } dirtype_t;`
/// The original C uses `DI_` prefix; this Rust port uses full direction names
/// for clarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Direction {
    /// East (right on the automap). DI_EAST = 0 in original C.
    East = 0,
    /// North-East (upper-right). DI_NORTHEAST = 1.
    NorthEast = 1,
    /// North (up on the automap). DI_NORTH = 2.
    North = 2,
    /// North-West (upper-left). DI_NORTHWEST = 3.
    NorthWest = 3,
    /// West (left on the automap). DI_WEST = 4.
    West = 4,
    /// South-West (lower-left). DI_SOUTHWEST = 5.
    SouthWest = 5,
    /// South (down on the automap). DI_SOUTH = 6.
    South = 6,
    /// South-East (lower-right). DI_SOUTHEAST = 7.
    SouthEast = 7,
    /// No direction — sentinel value indicating the monster needs to pick
    /// a new movement direction. DI_NODIR = 8.
    NoDir = 8,
}

/// The number of valid movement directions (excluding `NoDir`).
///
/// Used as the upper bound when iterating over direction arrays or selecting
/// random movement directions in the AI code.
///
/// Original C: `NUMDIRS` in the `dirtype_t` enum (auto-valued to 8, placed
/// before `DI_NODIR` in some versions or used as a count).
pub const NUMDIRS: usize = 8;

impl Direction {
    /// Convert a raw integer value to a `Direction`.
    ///
    /// Returns `None` if the value is outside the valid range (0..=8).
    /// This is used when reading the `movedir` field from save games or
    /// when converting from the integer representation used in the movement
    /// code.
    #[inline]
    pub fn from_i32(value: i32) -> Option<Direction> {
        match value {
            0 => Some(Direction::East),
            1 => Some(Direction::NorthEast),
            2 => Some(Direction::North),
            3 => Some(Direction::NorthWest),
            4 => Some(Direction::West),
            5 => Some(Direction::SouthWest),
            6 => Some(Direction::South),
            7 => Some(Direction::SouthEast),
            8 => Some(Direction::NoDir),
            _ => None,
        }
    }

    /// Convert this direction to its integer representation.
    #[inline]
    pub fn to_i32(self) -> i32 {
        self as i32
    }
}

impl Default for Direction {
    /// Returns `Direction::East` (value 0), matching the default zero-initialization
    /// behavior of the original C code.
    #[inline]
    fn default() -> Self {
        Direction::East
    }
}

// =============================================================================
// MapObject struct (p_mobj.h lines 207-286)
// =============================================================================

/// Map Object (mobj) — the central runtime entity structure in DOOM.
///
/// Every monster, player, projectile, item, and decoration in the game world
/// is a `MapObject`. This struct is the Rust translation of the C `mobj_t`
/// structure defined in `p_mobj.h` lines 207-286.
///
/// # Field Layout
///
/// The field order follows the original C struct exactly. This is important
/// for save game serialization compatibility and for matching the behavioral
/// expectations of the game code.
///
/// # Arena-Based Pointer Replacement
///
/// All raw C pointers (`mobj_t*`, `state_t*`, `mobjinfo_t*`, `player_s*`,
/// `subsector_s*`) are replaced with `Option<usize>` arena indices. The arenas
/// are managed externally by the game state. `None` represents a NULL pointer
/// in the original C code.
///
/// # Thinker Integration
///
/// The `thinker` field (first in the struct) embeds this object into the
/// global thinker linked list. Every mobj is updated once per game tic via
/// `P_MobjThinker`, which handles state machine transitions, physics
/// (gravity, friction, momentum), and action function dispatch.
///
/// Original C: `typedef struct mobj_s { ... } mobj_t;` (p_mobj.h lines 207-286).
#[derive(Debug, Clone)]
pub struct MapObject {
    // -------------------------------------------------------------------------
    // Thinker links (p_mobj.h line 210)
    // -------------------------------------------------------------------------
    /// Thinker list node — embeds this object into the global thinker list.
    ///
    /// Every active mobj has a thinker with `prev`/`next` arena indices linking
    /// it into the circular doubly-linked thinker list. The `function` field is
    /// set to `ActionFn::MobjThinker` for active map objects.
    ///
    /// Original C: `thinker_t thinker;`
    pub thinker: Thinker,

    // -------------------------------------------------------------------------
    // Position (p_mobj.h lines 213-215)
    // -------------------------------------------------------------------------
    /// X position in 16.16 fixed-point map coordinates.
    ///
    /// The origin point represents the bottom-center of the sprite (between
    /// the feet of a biped).
    ///
    /// Original C: `fixed_t x;`
    pub x: Fixed,

    /// Y position in 16.16 fixed-point map coordinates.
    ///
    /// Original C: `fixed_t y;`
    pub y: Fixed,

    /// Z position in 16.16 fixed-point map coordinates.
    ///
    /// For a walking creature, this equals the floor height it stands on.
    /// For flying creatures and projectiles, this can be any height between
    /// `floorz` and `ceilingz - height`.
    ///
    /// Original C: `fixed_t z;`
    pub z: Fixed,

    // -------------------------------------------------------------------------
    // Sector links (p_mobj.h lines 218-219)
    // -------------------------------------------------------------------------
    /// Arena index of the next mobj in the same sector's thing list.
    ///
    /// Used by the renderer to iterate all objects in a sector for drawing.
    /// `None` indicates this is the last object in the sector list (or the
    /// object has `MF_NOSECTOR` set and is not in any sector list).
    ///
    /// Original C: `struct mobj_s* snext;`
    pub snext: Option<usize>,

    /// Arena index of the previous mobj in the same sector's thing list.
    ///
    /// Original C: `struct mobj_s* sprev;`
    pub sprev: Option<usize>,

    // -------------------------------------------------------------------------
    // Drawing info (p_mobj.h lines 222-224)
    // -------------------------------------------------------------------------
    /// Orientation angle in Binary Angle Measurement (BAM) format.
    ///
    /// Determines which rotated sprite frame is drawn. A full circle is
    /// 2^32 BAM units.
    ///
    /// Original C: `angle_t angle;`
    pub angle: Angle,

    /// Sprite number — index into the sprite name table.
    ///
    /// Used together with `frame` to determine which patch (graphic) to
    /// draw for this object.
    ///
    /// Original C: `spritenum_t sprite;` (enum index)
    pub sprite: usize,

    /// Animation frame number. May be OR'ed with `FF_FULLBRIGHT` (0x8000)
    /// to indicate the frame should be drawn at full brightness regardless
    /// of the sector's light level.
    ///
    /// Original C: `int frame;`
    pub frame: i32,

    // -------------------------------------------------------------------------
    // Blockmap links (p_mobj.h lines 228-231)
    // -------------------------------------------------------------------------
    /// Arena index of the next mobj in the same blockmap block's thing list.
    ///
    /// The blockmap divides the level into 128×128 unit blocks. Each block
    /// tracks all interactable mobjs whose origin is contained within it.
    ///
    /// Original C: `struct mobj_s* bnext;`
    pub bnext: Option<usize>,

    /// Arena index of the previous mobj in the same blockmap block.
    ///
    /// Original C: `struct mobj_s* bprev;`
    pub bprev: Option<usize>,

    /// Arena index of the subsector this object is in.
    ///
    /// Found via `R_PointInSubsector(x, y)`. The sector can be obtained
    /// from `subsector->sector`. Used by the renderer for drawing and by
    /// the sound code for stereo positioning.
    ///
    /// Original C: `struct subsector_s* subsector;`
    pub subsector: Option<usize>,

    // -------------------------------------------------------------------------
    // Sector bounds (p_mobj.h lines 234-235)
    // -------------------------------------------------------------------------
    /// The floor height of the closest contacted sector beneath this object.
    ///
    /// Updated by `P_CheckPosition` and the movement code. Gravity pulls
    /// the object down toward this height.
    ///
    /// Original C: `fixed_t floorz;`
    pub floorz: Fixed,

    /// The ceiling height of the closest contacted sector above this object.
    ///
    /// Limits upward movement. Projectiles explode when they hit the ceiling.
    ///
    /// Original C: `fixed_t ceilingz;`
    pub ceilingz: Fixed,

    // -------------------------------------------------------------------------
    // Size (p_mobj.h lines 238-239)
    // -------------------------------------------------------------------------
    /// Collision radius in 16.16 fixed-point.
    ///
    /// The object occupies a circular area of this radius for collision
    /// detection purposes. Defined in the `mobjinfo` table.
    ///
    /// Original C: `fixed_t radius;`
    pub radius: Fixed,

    /// Height in 16.16 fixed-point.
    ///
    /// Vertical extent of the object for collision detection and rendering.
    /// Defined in the `mobjinfo` table.
    ///
    /// Original C: `fixed_t height;`
    pub height: Fixed,

    // -------------------------------------------------------------------------
    // Momentum (p_mobj.h lines 242-244)
    // -------------------------------------------------------------------------
    /// X momentum in 16.16 fixed-point per tic.
    ///
    /// Added to `x` each tic by `P_XYMovement`.
    ///
    /// Original C: `fixed_t momx;`
    pub momx: Fixed,

    /// Y momentum in 16.16 fixed-point per tic.
    ///
    /// Added to `y` each tic by `P_XYMovement`.
    ///
    /// Original C: `fixed_t momy;`
    pub momy: Fixed,

    /// Z momentum in 16.16 fixed-point per tic.
    ///
    /// Added to `z` each tic by `P_ZMovement`. Gravity subtracts from this
    /// each tic for non-`MF_NOGRAVITY` objects.
    ///
    /// Original C: `fixed_t momz;`
    pub momz: Fixed,

    // -------------------------------------------------------------------------
    // Validation (p_mobj.h line 247)
    // -------------------------------------------------------------------------
    /// Per-frame/per-check validation counter.
    ///
    /// If equal to the global `validcount`, this object has already been
    /// processed in the current iteration (sight check, sound propagation,
    /// etc.) and should be skipped.
    ///
    /// Original C: `int validcount;`
    pub validcount: i32,

    // -------------------------------------------------------------------------
    // Type info (p_mobj.h lines 249-250)
    // -------------------------------------------------------------------------
    /// Index into the `mobjinfo` table identifying the type of this object.
    ///
    /// Determines the object's base health, speed, radius, height, mass,
    /// damage, sounds, and state machine. Named `type_` because `type` is
    /// a reserved keyword in Rust.
    ///
    /// Original C: `mobjtype_t type;`
    pub type_: usize,

    /// Arena index into the `mobjinfo` table entry for this object's type.
    ///
    /// This is a reference to `&mobjinfo[mobj->type]` in the original C.
    /// Stored as an index rather than a reference to avoid lifetime issues.
    /// `None` indicates the info has not been set (should not occur for
    /// properly initialized objects).
    ///
    /// Original C: `mobjinfo_t* info;`
    pub info: Option<usize>,

    // -------------------------------------------------------------------------
    // State (p_mobj.h lines 252-255)
    // -------------------------------------------------------------------------
    /// State tic counter — number of tics remaining in the current state.
    ///
    /// Decremented each tic by `P_MobjThinker`. When it reaches 0, the
    /// object transitions to the state's `nextstate`. A value of -1 means
    /// the state lasts forever (until explicitly changed).
    ///
    /// Original C: `int tics;`
    pub tics: i32,

    /// Arena index into the states table for the current animation/behavior
    /// state.
    ///
    /// The state determines the sprite, frame, duration, and action function
    /// for the object's current behavior. `None` indicates the object has
    /// been removed.
    ///
    /// Original C: `state_t* state;`
    pub state: Option<usize>,

    /// Bitfield of `MobjFlags` controlling behavior, collision, rendering,
    /// and AI properties.
    ///
    /// Original C: `int flags;`
    pub flags: MobjFlags,

    /// Current health points.
    ///
    /// Initialized from `mobjinfo[type].spawnhealth`. When reduced to 0 or
    /// below by `P_DamageMobj`, the object enters its death sequence.
    ///
    /// Original C: `int health;`
    pub health: i32,

    // -------------------------------------------------------------------------
    // Movement direction (p_mobj.h lines 258-259)
    // -------------------------------------------------------------------------
    /// Movement direction for monster AI (0-7 for compass directions, 8 for
    /// no direction).
    ///
    /// Stores the raw integer value corresponding to a `Direction` enum
    /// variant. Used by `A_Chase` and the movement code to determine which
    /// way the monster is currently moving.
    ///
    /// Original C: `int movedir;`
    pub movedir: i32,

    /// Movement step counter. When this reaches 0, `A_Chase` selects a new
    /// movement direction.
    ///
    /// Decremented each time the monster successfully moves. A higher value
    /// means the monster will continue in its current direction longer.
    ///
    /// Original C: `int movecount;`
    pub movecount: i32,

    // -------------------------------------------------------------------------
    // Chase/attack target (p_mobj.h lines 263)
    // -------------------------------------------------------------------------
    /// Arena index of the thing being chased/attacked, or the originator
    /// of a missile.
    ///
    /// For monsters: the player or other monster they are targeting.
    /// For missiles: the object that fired the missile (used to prevent
    /// same-species hits with `MF_MISSILE`).
    /// `None` if no target.
    ///
    /// Original C: `struct mobj_s* target;`
    pub target: Option<usize>,

    // -------------------------------------------------------------------------
    // Reaction (p_mobj.h lines 267-271)
    // -------------------------------------------------------------------------
    /// Reaction time: if non-zero, don't attack yet.
    ///
    /// Used by monsters to delay their first attack after spawning or
    /// waking up. Also used by players to freeze briefly after teleporting.
    /// Decremented each tic; the monster can attack when it reaches 0.
    ///
    /// Original C: `int reactiontime;`
    pub reactiontime: i32,

    /// Chase threshold: if greater than 0, the target will be chased no
    /// matter what (even if shot by another monster).
    ///
    /// Prevents monsters from constantly switching targets during infighting.
    /// Decremented each tic; when it reaches 0, the monster may switch
    /// targets if shot by a different attacker.
    ///
    /// Original C: `int threshold;`
    pub threshold: i32,

    // -------------------------------------------------------------------------
    // Player info (p_mobj.h lines 275-278)
    // -------------------------------------------------------------------------
    /// Arena index of the player structure, if this is a player avatar.
    ///
    /// Only valid if `type_ == MT_PLAYER`. For non-player objects, this
    /// is `None`.
    ///
    /// Original C: `struct player_s* player;`
    pub player: Option<usize>,

    /// Player number last looked for (0..MAXPLAYERS-1).
    ///
    /// Used by `A_Look` to cycle through players when checking for targets.
    /// Each monster remembers which player it last checked, so different
    /// monsters don't all fixate on the same player.
    ///
    /// Original C: `int lastlook;`
    pub lastlook: i32,

    // -------------------------------------------------------------------------
    // Nightmare respawn (p_mobj.h line 281)
    // -------------------------------------------------------------------------
    /// Original map thing data stored for nightmare mode respawning.
    ///
    /// When monsters are killed in Nightmare difficulty, they respawn after
    /// a delay at their original map position. This field stores the
    /// original `mapthing_t` data (x, y, angle, type, options) from the
    /// WAD file.
    ///
    /// Original C: `mapthing_t spawnpoint;`
    pub spawnpoint: MapThing,

    // -------------------------------------------------------------------------
    // Tracer (p_mobj.h line 284)
    // -------------------------------------------------------------------------
    /// Arena index of the thing being chased/attacked for homing tracers.
    ///
    /// Used by the Revenant's homing missiles to track their target.
    /// Also used by the Arch-Vile's attack to track the attack target.
    /// `None` if no tracer target.
    ///
    /// Original C: `struct mobj_s* tracer;`
    pub tracer: Option<usize>,
}

impl Default for MapObject {
    /// Creates a default `MapObject` with all fields zero-initialized or set
    /// to `None`.
    ///
    /// This matches the behavior of `memset(mobj, 0, sizeof(mobj_t))` followed
    /// by field-specific initialization in the original C `P_SpawnMobj`.
    fn default() -> Self {
        MapObject {
            thinker: Thinker::default(),
            x: Fixed::default(),
            y: Fixed::default(),
            z: Fixed::default(),
            snext: None,
            sprev: None,
            angle: Angle::default(),
            sprite: 0,
            frame: 0,
            bnext: None,
            bprev: None,
            subsector: None,
            floorz: Fixed::default(),
            ceilingz: Fixed::default(),
            radius: Fixed::default(),
            height: Fixed::default(),
            momx: Fixed::default(),
            momy: Fixed::default(),
            momz: Fixed::default(),
            validcount: 0,
            type_: 0,
            info: None,
            tics: 0,
            state: None,
            flags: MobjFlags::default(),
            health: 0,
            movedir: 0,
            movecount: 0,
            target: None,
            reactiontime: 0,
            threshold: 0,
            player: None,
            lastlook: 0,
            spawnpoint: MapThing::default(),
            tracer: None,
        }
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // MobjFlags tests — verify ALL flag values match original C exactly
    // -------------------------------------------------------------------------

    #[test]
    fn test_mobj_flags_values_match_original() {
        // p_mobj.h lines 117-203: every flag value verified
        assert_eq!(MobjFlags::MF_SPECIAL.bits(), 1);
        assert_eq!(MobjFlags::MF_SOLID.bits(), 2);
        assert_eq!(MobjFlags::MF_SHOOTABLE.bits(), 4);
        assert_eq!(MobjFlags::MF_NOSECTOR.bits(), 8);
        assert_eq!(MobjFlags::MF_NOBLOCKMAP.bits(), 16);
        assert_eq!(MobjFlags::MF_AMBUSH.bits(), 32);
        assert_eq!(MobjFlags::MF_JUSTHIT.bits(), 64);
        assert_eq!(MobjFlags::MF_JUSTATTACKED.bits(), 128);
        assert_eq!(MobjFlags::MF_SPAWNCEILING.bits(), 256);
        assert_eq!(MobjFlags::MF_NOGRAVITY.bits(), 512);
        assert_eq!(MobjFlags::MF_DROPOFF.bits(), 0x400);
        assert_eq!(MobjFlags::MF_PICKUP.bits(), 0x800);
        assert_eq!(MobjFlags::MF_NOCLIP.bits(), 0x1000);
        assert_eq!(MobjFlags::MF_SLIDE.bits(), 0x2000);
        assert_eq!(MobjFlags::MF_FLOAT.bits(), 0x4000);
        assert_eq!(MobjFlags::MF_TELEPORT.bits(), 0x8000);
        assert_eq!(MobjFlags::MF_MISSILE.bits(), 0x10000);
        assert_eq!(MobjFlags::MF_DROPPED.bits(), 0x20000);
        assert_eq!(MobjFlags::MF_SHADOW.bits(), 0x40000);
        assert_eq!(MobjFlags::MF_NOBLOOD.bits(), 0x80000);
        assert_eq!(MobjFlags::MF_CORPSE.bits(), 0x100000);
        assert_eq!(MobjFlags::MF_INFLOAT.bits(), 0x200000);
        assert_eq!(MobjFlags::MF_COUNTKILL.bits(), 0x400000);
        assert_eq!(MobjFlags::MF_COUNTITEM.bits(), 0x800000);
        assert_eq!(MobjFlags::MF_SKULLFLY.bits(), 0x1000000);
        assert_eq!(MobjFlags::MF_NOTDMATCH.bits(), 0x2000000);
        assert_eq!(MobjFlags::MF_TRANSLATION.bits(), 0xc000000);
    }

    #[test]
    fn test_mf_transshift_value() {
        // p_mobj.h line 201: MF_TRANSSHIFT = 26
        assert_eq!(MF_TRANSSHIFT, 26);
    }

    #[test]
    fn test_translation_mask_with_shift() {
        // The translation color is extracted by: (flags & MF_TRANSLATION) >> MF_TRANSSHIFT
        // For player color 1 (indigo): flags have bits 26-27 = 01 → 0x4000000
        let flags = MobjFlags::from_bits_truncate(0x4000000);
        let color = (flags & MobjFlags::MF_TRANSLATION).bits() >> MF_TRANSSHIFT;
        assert_eq!(color, 1);

        // For player color 2 (brown): bits 26-27 = 10 → 0x8000000
        let flags = MobjFlags::from_bits_truncate(0x8000000);
        let color = (flags & MobjFlags::MF_TRANSLATION).bits() >> MF_TRANSSHIFT;
        assert_eq!(color, 2);

        // For player color 3 (red): bits 26-27 = 11 → 0xc000000
        let flags = MobjFlags::from_bits_truncate(0xc000000);
        let color = (flags & MobjFlags::MF_TRANSLATION).bits() >> MF_TRANSSHIFT;
        assert_eq!(color, 3);
    }

    #[test]
    fn test_mobj_flags_bitwise_operations() {
        // Typical monster flags: solid, shootable, count kill
        let monster_flags = MobjFlags::MF_SOLID | MobjFlags::MF_SHOOTABLE | MobjFlags::MF_COUNTKILL;
        assert!(monster_flags.contains(MobjFlags::MF_SOLID));
        assert!(monster_flags.contains(MobjFlags::MF_SHOOTABLE));
        assert!(monster_flags.contains(MobjFlags::MF_COUNTKILL));
        assert!(!monster_flags.contains(MobjFlags::MF_MISSILE));

        // Typical missile flags: no gravity, missile, drop off
        let missile_flags = MobjFlags::MF_NOGRAVITY | MobjFlags::MF_MISSILE | MobjFlags::MF_DROPOFF;
        assert!(missile_flags.contains(MobjFlags::MF_NOGRAVITY));
        assert!(missile_flags.contains(MobjFlags::MF_MISSILE));
        assert!(!missile_flags.contains(MobjFlags::MF_SOLID));
    }

    #[test]
    fn test_mobj_flags_default_is_empty() {
        let flags = MobjFlags::default();
        assert!(flags.is_empty());
        assert_eq!(flags.bits(), 0);
    }

    // -------------------------------------------------------------------------
    // Direction enum tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_direction_values_match_original() {
        // dirtype_t enum ordering: DI_EAST=0 through DI_SOUTHEAST=7, DI_NODIR=8
        assert_eq!(Direction::East as i32, 0);
        assert_eq!(Direction::NorthEast as i32, 1);
        assert_eq!(Direction::North as i32, 2);
        assert_eq!(Direction::NorthWest as i32, 3);
        assert_eq!(Direction::West as i32, 4);
        assert_eq!(Direction::SouthWest as i32, 5);
        assert_eq!(Direction::South as i32, 6);
        assert_eq!(Direction::SouthEast as i32, 7);
        assert_eq!(Direction::NoDir as i32, 8);
    }

    #[test]
    fn test_numdirs_value() {
        assert_eq!(NUMDIRS, 8);
    }

    #[test]
    fn test_direction_from_i32_valid() {
        assert_eq!(Direction::from_i32(0), Some(Direction::East));
        assert_eq!(Direction::from_i32(4), Some(Direction::West));
        assert_eq!(Direction::from_i32(8), Some(Direction::NoDir));
    }

    #[test]
    fn test_direction_from_i32_invalid() {
        assert_eq!(Direction::from_i32(-1), None);
        assert_eq!(Direction::from_i32(9), None);
        assert_eq!(Direction::from_i32(100), None);
    }

    #[test]
    fn test_direction_roundtrip() {
        for i in 0..=8 {
            let dir = Direction::from_i32(i).unwrap();
            assert_eq!(dir.to_i32(), i);
        }
    }

    // -------------------------------------------------------------------------
    // MapObject tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_map_object_default() {
        let mobj = MapObject::default();

        // Position should be zero
        assert_eq!(mobj.x, Fixed::default());
        assert_eq!(mobj.y, Fixed::default());
        assert_eq!(mobj.z, Fixed::default());

        // All optional/pointer fields should be None
        assert_eq!(mobj.snext, None);
        assert_eq!(mobj.sprev, None);
        assert_eq!(mobj.bnext, None);
        assert_eq!(mobj.bprev, None);
        assert_eq!(mobj.subsector, None);
        assert_eq!(mobj.info, None);
        assert_eq!(mobj.state, None);
        assert_eq!(mobj.target, None);
        assert_eq!(mobj.player, None);
        assert_eq!(mobj.tracer, None);

        // Angle should be default (0)
        assert_eq!(mobj.angle, Angle::default());

        // Flags should be empty
        assert!(mobj.flags.is_empty());

        // Integer fields should be zero
        assert_eq!(mobj.sprite, 0);
        assert_eq!(mobj.frame, 0);
        assert_eq!(mobj.validcount, 0);
        assert_eq!(mobj.type_, 0);
        assert_eq!(mobj.tics, 0);
        assert_eq!(mobj.health, 0);
        assert_eq!(mobj.movedir, 0);
        assert_eq!(mobj.movecount, 0);
        assert_eq!(mobj.reactiontime, 0);
        assert_eq!(mobj.threshold, 0);
        assert_eq!(mobj.lastlook, 0);

        // Spawnpoint should be default
        assert_eq!(mobj.spawnpoint, MapThing::default());
    }

    #[test]
    fn test_map_object_field_assignment() {
        let mut mobj = MapObject::default();

        // Set position
        mobj.x = Fixed::new(100 << 16);
        mobj.y = Fixed::new(200 << 16);
        mobj.z = Fixed::new(0);

        // Set flags (typical Imp flags)
        mobj.flags = MobjFlags::MF_SOLID | MobjFlags::MF_SHOOTABLE | MobjFlags::MF_COUNTKILL;

        // Set health
        mobj.health = 60;

        // Verify
        assert_eq!(mobj.x.raw(), 100 << 16);
        assert_eq!(mobj.y.raw(), 200 << 16);
        assert_eq!(mobj.health, 60);
        assert!(mobj.flags.contains(MobjFlags::MF_SOLID));
        assert!(mobj.flags.contains(MobjFlags::MF_SHOOTABLE));
        assert!(mobj.flags.contains(MobjFlags::MF_COUNTKILL));
    }

    #[test]
    fn test_map_object_clone() {
        let mut mobj = MapObject::default();
        mobj.x = Fixed::new(42 << 16);
        mobj.health = 100;
        mobj.flags = MobjFlags::MF_SOLID;

        let cloned = mobj.clone();
        assert_eq!(cloned.x, mobj.x);
        assert_eq!(cloned.health, mobj.health);
        assert_eq!(cloned.flags, mobj.flags);
    }

    #[test]
    fn test_map_object_all_fields_present() {
        // Verify all 34 fields are accessible by assigning and reading each one
        let mobj = MapObject {
            thinker: Thinker::default(),
            x: Fixed::new(1),
            y: Fixed::new(2),
            z: Fixed::new(3),
            snext: Some(10),
            sprev: Some(11),
            angle: Angle::new(0x40000000),
            sprite: 5,
            frame: 6,
            bnext: Some(20),
            bprev: Some(21),
            subsector: Some(30),
            floorz: Fixed::new(4),
            ceilingz: Fixed::new(5),
            radius: Fixed::new(6),
            height: Fixed::new(7),
            momx: Fixed::new(8),
            momy: Fixed::new(9),
            momz: Fixed::new(10),
            validcount: 42,
            type_: 100,
            info: Some(100),
            tics: 15,
            state: Some(50),
            flags: MobjFlags::MF_SOLID,
            health: 200,
            movedir: 3,
            movecount: 7,
            target: Some(60),
            reactiontime: 8,
            threshold: 0,
            player: Some(0),
            lastlook: 2,
            spawnpoint: MapThing {
                x: 100,
                y: 200,
                angle: 90,
                type_: 3004,
                options: 7,
            },
            tracer: Some(70),
        };

        assert_eq!(mobj.x, Fixed::new(1));
        assert_eq!(mobj.y, Fixed::new(2));
        assert_eq!(mobj.z, Fixed::new(3));
        assert_eq!(mobj.snext, Some(10));
        assert_eq!(mobj.sprev, Some(11));
        assert_eq!(mobj.angle.value(), 0x40000000);
        assert_eq!(mobj.sprite, 5);
        assert_eq!(mobj.frame, 6);
        assert_eq!(mobj.bnext, Some(20));
        assert_eq!(mobj.bprev, Some(21));
        assert_eq!(mobj.subsector, Some(30));
        assert_eq!(mobj.floorz, Fixed::new(4));
        assert_eq!(mobj.ceilingz, Fixed::new(5));
        assert_eq!(mobj.radius, Fixed::new(6));
        assert_eq!(mobj.height, Fixed::new(7));
        assert_eq!(mobj.momx, Fixed::new(8));
        assert_eq!(mobj.momy, Fixed::new(9));
        assert_eq!(mobj.momz, Fixed::new(10));
        assert_eq!(mobj.validcount, 42);
        assert_eq!(mobj.type_, 100);
        assert_eq!(mobj.info, Some(100));
        assert_eq!(mobj.tics, 15);
        assert_eq!(mobj.state, Some(50));
        assert_eq!(mobj.flags, MobjFlags::MF_SOLID);
        assert_eq!(mobj.health, 200);
        assert_eq!(mobj.movedir, 3);
        assert_eq!(mobj.movecount, 7);
        assert_eq!(mobj.target, Some(60));
        assert_eq!(mobj.reactiontime, 8);
        assert_eq!(mobj.threshold, 0);
        assert_eq!(mobj.player, Some(0));
        assert_eq!(mobj.lastlook, 2);
        assert_eq!(mobj.spawnpoint.x, 100);
        assert_eq!(mobj.spawnpoint.type_, 3004);
        assert_eq!(mobj.tracer, Some(70));
    }
}
