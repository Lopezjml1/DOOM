//! Translated from linuxdoom-1.10/p_spec.c and p_spec.h
//!
//! Map special triggers, animations, sector specials, and utility functions.
//! Handles line special triggers (walkover, switch, gunfire), sector damage effects,
//! animated flats/textures, scrolling walls, button countdowns, and level timer.

use crate::info::mobjinfo::MobjType;
use crate::info::sounds::SfxEnum;
use crate::types::doomdef::{PowerType, TICRATE};
use crate::types::fixed::{Fixed, FRACUNIT};
use crate::types::map_data::{LineDef, LineFlags, Sector, SideDef};
use crate::types::mobj::{MapObject, MobjFlags};
use crate::types::player::{CheatFlags, Player};
use crate::types::thinker::{ActionFn, Thinker};
use crate::util::argv::Args;
use crate::util::random::DoomRandom;

// ============================================================================
// Constants from p_spec.h
// ============================================================================

/// Maximum number of switch texture pairs.
pub const MAXSWITCHES: usize = 50;

/// Maximum number of simultaneously active timed buttons.
pub const MAXBUTTONS: usize = 16;

/// Duration of a timed button in tics (1 second = 35 tics).
pub const BUTTONTIME: i32 = 35;

/// Maximum number of texture/flat animations.
pub const MAX_ANIMS: usize = 32;

/// Maximum number of adjoining sectors for floor search.
pub const MAX_ADJOINING_SECTORS: usize = 20;

/// Maximum number of line-based animations (scrolling lines).
pub const MAXLINEANIMS: usize = 64;

/// Floor movement speed (FRACUNIT).
pub const FLOORSPEED: i32 = FRACUNIT;

/// Platform movement speed (FRACUNIT).
pub const PLATSPEED: i32 = FRACUNIT;

/// Vertical door speed (FRACUNIT * 2).
pub const VDOORSPEED: i32 = FRACUNIT * 2;

/// Vertical door wait tics.
pub const VDOORWAIT: i32 = 150;

/// Platform wait tics.
pub const PLATWAIT: i32 = 3;

/// Ceiling movement speed (FRACUNIT).
pub const CEILSPEED: i32 = FRACUNIT;

/// Ceiling wait tics.
pub const CEILWAIT: i32 = 150;

/// Maximum active ceilings.
pub const MAXCEILINGS: usize = 30;

/// Maximum active platforms.
pub const MAXPLATS: usize = 30;

/// Glow effect speed.
pub const GLOWSPEED: i32 = 8;

/// Strobe bright time in tics.
pub const STROBEBRIGHT: i32 = 5;

/// Fast dark strobe interval in tics.
pub const FASTDARK: i32 = 15;

/// Slow dark strobe interval in tics.
pub const SLOWDARK: i32 = 35;

// ============================================================================
// Enums from p_spec.h
// ============================================================================

/// Position on a linedef where a button texture resides.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BWhere {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// Vertical door types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum VldoorType {
    /// Standard door: opens, waits, closes.
    Normal = 0,
    /// Close only.
    Close = 1,
    /// Open and stay open.
    Open = 2,
    /// Opens after 5 minutes.
    RaiseIn5Mins = 3,
    /// Fast open, wait, close.
    BlazeRaise = 4,
    /// Fast open and stay.
    BlazeOpen = 5,
    /// Fast close.
    BlazeClose = 6,
    /// Close, wait 30 seconds, then open.
    Close30ThenOpen = 7,
}

/// Floor movement types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum FloorType {
    /// Lower floor to highest surrounding floor.
    LowerFloor = 0,
    /// Lower floor to lowest surrounding floor.
    LowerFloorToLowest = 1,
    /// Turbo lower floor.
    TurboLower = 2,
    /// Raise floor to lowest ceiling.
    RaiseFloor = 3,
    /// Raise floor to nearest higher floor.
    RaiseFloorToNearest = 4,
    /// Raise floor to shortest lower texture height.
    RaiseToTexture = 5,
    /// Lower floor and change texture+type.
    LowerAndChange = 6,
    /// Raise floor by 24 units.
    RaiseFloor24 = 7,
    /// Raise floor by 512 units.
    RaiseFloor512 = 8,
    /// Raise floor by 24 units and change texture.
    RaiseFloor24AndChange = 9,
    /// Raise floor with crush damage.
    RaiseFloorCrush = 10,
    /// Turbo raise floor.
    RaiseFloorTurbo = 11,
    /// Donut raise type.
    DonutRaise = 12,
}

/// Platform movement types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PlatType {
    /// Perpetual raise cycle.
    PerpetualRaise = 0,
    /// Lower, wait, raise back.
    DownWaitUpStay = 1,
    /// Raise and change texture.
    RaiseAndChange = 2,
    /// Raise to nearest and change texture.
    RaiseToNearestAndChange = 3,
    /// Blazing (fast) down-wait-up-stay.
    BlazeDWUS = 4,
}

/// Ceiling movement types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CeilingType {
    /// Lower ceiling to floor.
    LowerToFloor = 0,
    /// Raise ceiling to highest surrounding.
    RaiseToHighest = 1,
    /// Lower and crush.
    LowerAndCrush = 2,
    /// Crush and raise cycle.
    CrushAndRaise = 3,
    /// Fast crush and raise cycle.
    FastCrushAndRaise = 4,
    /// Silent crush and raise cycle.
    SilentCrushAndRaise = 5,
}

/// Staircase build types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum StairType {
    /// Build 8-unit stairs.
    Build8 = 0,
    /// Build 16-unit turbo stairs.
    Turbo16 = 1,
}

/// Movement result status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ResultE {
    /// Movement succeeded.
    Ok = 0,
    /// Movement crushed something.
    Crushed = 1,
    /// Mover reached destination.
    PastDest = 2,
}

/// Platform status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
#[derive(Default)]
pub enum PlatStatus {
    #[default]
    Up = 0,
    Down = 1,
    Waiting = 2,
    InStasis = 3,
}

// ============================================================================
// Mover thinker structs from p_spec.h
// ============================================================================

/// Ceiling mover thinker.
#[derive(Debug, Clone)]
pub struct CeilingT {
    pub thinker: Thinker,
    pub ceiling_type: CeilingType,
    pub sector: usize,
    pub bottomheight: Fixed,
    pub topheight: Fixed,
    pub speed: Fixed,
    pub crush: bool,
    pub direction: i32,
    pub tag: i32,
    pub olddirection: i32,
}

impl CeilingT {
    pub fn new(sector: usize) -> Self {
        Self {
            thinker: Thinker::new(ActionFn::None),
            ceiling_type: CeilingType::LowerToFloor,
            sector,
            bottomheight: Fixed::ZERO,
            topheight: Fixed::ZERO,
            speed: Fixed::ZERO,
            crush: false,
            direction: 0,
            tag: 0,
            olddirection: 0,
        }
    }
}

/// Vertical door thinker.
#[derive(Debug, Clone)]
pub struct VldoorT {
    pub thinker: Thinker,
    pub door_type: VldoorType,
    pub sector: usize,
    pub topheight: Fixed,
    pub speed: Fixed,
    pub direction: i32,
    pub topwait: i32,
    pub topcountdown: i32,
}

impl VldoorT {
    pub fn new(sector: usize) -> Self {
        Self {
            thinker: Thinker::new(ActionFn::None),
            door_type: VldoorType::Normal,
            sector,
            topheight: Fixed::ZERO,
            speed: Fixed::ZERO,
            direction: 0,
            topwait: 0,
            topcountdown: 0,
        }
    }
}

/// Floor movement thinker.
#[derive(Debug, Clone)]
pub struct FloorMoveT {
    pub thinker: Thinker,
    pub floor_type: FloorType,
    pub crush: bool,
    pub sector: usize,
    pub direction: i32,
    pub newsecspecial: i32,
    pub newtexture: i16,
    pub floordestheight: Fixed,
    pub speed: Fixed,
}

impl FloorMoveT {
    pub fn new(sector: usize) -> Self {
        Self {
            thinker: Thinker::new(ActionFn::None),
            floor_type: FloorType::LowerFloor,
            crush: false,
            sector,
            direction: 0,
            newsecspecial: 0,
            newtexture: 0,
            floordestheight: Fixed::ZERO,
            speed: Fixed::ZERO,
        }
    }
}

/// Platform thinker.
#[derive(Debug, Clone)]
pub struct PlatT {
    pub thinker: Thinker,
    pub sector: usize,
    pub speed: Fixed,
    pub low: Fixed,
    pub high: Fixed,
    pub wait: i32,
    pub count: i32,
    pub status: PlatStatus,
    pub oldstatus: PlatStatus,
    pub crush: bool,
    pub tag: i32,
    pub plat_type: PlatType,
}

impl PlatT {
    pub fn new(sector: usize) -> Self {
        Self {
            thinker: Thinker::new(ActionFn::None),
            sector,
            speed: Fixed::ZERO,
            low: Fixed::ZERO,
            high: Fixed::ZERO,
            wait: 0,
            count: 0,
            status: PlatStatus::Up,
            oldstatus: PlatStatus::Up,
            crush: false,
            tag: 0,
            plat_type: PlatType::PerpetualRaise,
        }
    }
}

/// Fire flicker light effect thinker.
#[derive(Debug, Clone)]
pub struct FireFlickerT {
    pub thinker: Thinker,
    pub sector: usize,
    pub count: i32,
    pub maxlight: i32,
    pub minlight: i32,
}

impl FireFlickerT {
    pub fn new(sector: usize) -> Self {
        Self {
            thinker: Thinker::new(ActionFn::None),
            sector,
            count: 0,
            maxlight: 0,
            minlight: 0,
        }
    }
}

/// Light flash effect thinker.
#[derive(Debug, Clone)]
pub struct LightFlashT {
    pub thinker: Thinker,
    pub sector: usize,
    pub count: i32,
    pub maxlight: i32,
    pub minlight: i32,
    pub maxtime: i32,
    pub mintime: i32,
}

impl LightFlashT {
    pub fn new(sector: usize) -> Self {
        Self {
            thinker: Thinker::new(ActionFn::None),
            sector,
            count: 0,
            maxlight: 0,
            minlight: 0,
            maxtime: 0,
            mintime: 0,
        }
    }
}

/// Strobe light flash effect thinker.
#[derive(Debug, Clone)]
pub struct StrobeFlashT {
    pub thinker: Thinker,
    pub sector: usize,
    pub count: i32,
    pub minlight: i32,
    pub maxlight: i32,
    pub darktime: i32,
    pub brighttime: i32,
}

impl StrobeFlashT {
    pub fn new(sector: usize) -> Self {
        Self {
            thinker: Thinker::new(ActionFn::None),
            sector,
            count: 0,
            minlight: 0,
            maxlight: 0,
            darktime: 0,
            brighttime: 0,
        }
    }
}

/// Glowing light effect thinker.
#[derive(Debug, Clone)]
pub struct GlowT {
    pub thinker: Thinker,
    pub sector: usize,
    pub minlight: i32,
    pub maxlight: i32,
    pub direction: i32,
}

impl GlowT {
    pub fn new(sector: usize) -> Self {
        Self {
            thinker: Thinker::new(ActionFn::None),
            sector,
            minlight: 0,
            maxlight: 0,
            direction: 0,
        }
    }
}

// ============================================================================
// Animation system types
// ============================================================================

/// Runtime animation definition (computed from source data).
#[derive(Debug, Default, Clone, Copy)]
pub struct AnimDef {
    pub is_texture: bool,
    pub pic_num: i32,
    pub base_pic: i32,
    pub num_pics: i32,
    pub speed: i32,
}

/// Source animation definition (static data from animdefs[] in p_spec.c).
#[derive(Debug, Clone, Copy)]
pub struct AnimDefSource {
    pub is_texture: bool,
    pub endname: &'static str,
    pub startname: &'static str,
    pub speed: i32,
}

/// Animation definition table from p_spec.c (22 entries + terminator).
/// Order preserved exactly from original source: 9 flats then 13 textures.
pub static ANIM_DEFS: &[AnimDefSource] = &[
    // Flats (is_texture = false)
    AnimDefSource {
        is_texture: false,
        endname: "NUKAGE3",
        startname: "NUKAGE1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: false,
        endname: "FWATER4",
        startname: "FWATER1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: false,
        endname: "SWATER4",
        startname: "SWATER1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: false,
        endname: "LAVAFL3",
        startname: "LAVAFL1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: false,
        endname: "BLOOD3",
        startname: "BLOOD1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: false,
        endname: "RROCK08",
        startname: "RROCK05",
        speed: 8,
    },
    AnimDefSource {
        is_texture: false,
        endname: "SLIME04",
        startname: "SLIME01",
        speed: 8,
    },
    AnimDefSource {
        is_texture: false,
        endname: "SLIME08",
        startname: "SLIME05",
        speed: 8,
    },
    AnimDefSource {
        is_texture: false,
        endname: "SLIME12",
        startname: "SLIME09",
        speed: 8,
    },
    // Textures (is_texture = true)
    AnimDefSource {
        is_texture: true,
        endname: "BLODGR4",
        startname: "BLODGR1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "SLADRIP3",
        startname: "SLADRIP1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "BLODRIP4",
        startname: "BLODRIP1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "FIREWALL",
        startname: "FIREWA16",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "GSTFONT3",
        startname: "GSTFONT1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "FIRELAVA",
        startname: "FIRELAV3",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "FIREMAG3",
        startname: "FIREMAG1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "FIREBLU2",
        startname: "FIREBLU1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "ROCKRED3",
        startname: "ROCKRED1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "BFALL4",
        startname: "BFALL1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "SFALL4",
        startname: "SFALL1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "WFALL4",
        startname: "WFALL1",
        speed: 8,
    },
    AnimDefSource {
        is_texture: true,
        endname: "DBRAIN4",
        startname: "DBRAIN1",
        speed: 8,
    },
    // Terminator
    AnimDefSource {
        is_texture: false,
        endname: "",
        startname: "",
        speed: 0,
    },
];

// ============================================================================
// Switch and button types
// ============================================================================

/// Switch texture pair entry.
#[derive(Debug, Default, Clone)]
pub struct SwitchList {
    pub name1: [u8; 9],
    pub name2: [u8; 9],
    pub episode: i32,
}

/// Timed button state for switch texture reversion.
#[derive(Debug, Clone)]
pub struct ButtonT {
    pub line: Option<usize>,
    pub where_pos: BWhere,
    pub btexture: i32,
    pub btimer: i32,
    pub soundorg: Option<usize>,
}

impl Default for ButtonT {
    fn default() -> Self {
        Self {
            line: None,
            where_pos: BWhere::Top,
            btexture: 0,
            btimer: 0,
            soundorg: None,
        }
    }
}

// ============================================================================
// Module-level mutable state
// ============================================================================

/// Mutable state for the map specials system.
/// Aggregates global variables from p_spec.c.
pub struct SpecState {
    /// Runtime animation definitions (computed from ANIM_DEFS at init).
    pub anims: [AnimDef; MAX_ANIMS],
    /// Number of active animations.
    pub last_anim_idx: usize,
    /// Switch texture pair lookup (alternating texture numbers).
    pub switchlist: [i32; MAXSWITCHES * 2],
    /// Number of switch pairs.
    pub numswitches: i32,
    /// Active timed button list.
    pub buttonlist: [ButtonT; MAXBUTTONS],
    /// Whether level timer is active (-timer or -avg command line).
    pub level_timer: bool,
    /// Remaining tics until level timer expires.
    pub level_time_count: i32,
    /// List of line indices with scrolling/animated specials (e.g., type 48).
    pub line_special_list: [usize; MAXLINEANIMS],
    /// Number of entries in line_special_list.
    pub num_line_specials: usize,
}

/// Public re-exports for schema compliance (object exports).
pub fn switchlist(state: &SpecState) -> &[i32] {
    &state.switchlist
}

pub fn numswitches(state: &SpecState) -> i32 {
    state.numswitches
}

pub fn buttonlist(state: &SpecState) -> &[ButtonT] {
    &state.buttonlist
}

impl Default for SpecState {
    fn default() -> Self {
        Self {
            anims: [AnimDef::default(); MAX_ANIMS],
            last_anim_idx: 0,
            switchlist: [0i32; MAXSWITCHES * 2],
            numswitches: 0,
            buttonlist: core::array::from_fn(|_| ButtonT::default()),
            level_timer: false,
            level_time_count: 0,
            line_special_list: [0usize; MAXLINEANIMS],
            num_line_specials: 0,
        }
    }
}

// ============================================================================
// Context trait for cross-module dispatch
// ============================================================================

/// Context trait providing all operations needed by map specials.
///
/// This trait abstracts over game state and cross-module function calls
/// required by P_CrossSpecialLine, P_ShootSpecialLine, P_SpawnSpecials, etc.
/// The concrete implementation wires together the game state subsystems.
pub trait SpecContext {
    // --- Level geometry accessors ---
    fn lines(&self) -> &[LineDef];
    fn lines_mut(&mut self) -> &mut Vec<LineDef>;
    fn sides(&self) -> &[SideDef];
    fn sides_mut(&mut self) -> &mut Vec<SideDef>;
    fn sectors(&self) -> &[Sector];
    fn sectors_mut(&mut self) -> &mut Vec<Sector>;
    fn num_sectors(&self) -> usize;
    fn num_lines(&self) -> usize;

    // --- Spec state ---
    fn spec_state(&self) -> &SpecState;
    fn spec_state_mut(&mut self) -> &mut SpecState;

    // --- Game state ---
    fn leveltime(&self) -> i32;
    fn gameskill(&self) -> i32;
    fn deathmatch(&self) -> i32;
    fn netgame(&self) -> bool;

    // --- Entities ---
    fn mobjs(&self) -> &[MapObject];
    fn mobjs_mut(&mut self) -> &mut Vec<MapObject>;
    fn players(&self) -> &[Player];
    fn players_mut(&mut self) -> &mut Vec<Player>;
    fn playeringame(&self) -> &[bool];

    // --- Global counters ---
    fn totalsecret_inc(&mut self);

    // --- Sound ---
    fn s_start_sound(&mut self, origin: Option<usize>, sfx: SfxEnum);

    // --- Random ---
    fn rng_mut(&mut self) -> &mut DoomRandom;

    // --- Arguments ---
    fn args(&self) -> &Args;

    // --- Cross-module: Doors ---
    fn ev_do_door(&mut self, line_idx: usize, door_type: VldoorType) -> bool;

    // --- Cross-module: Floors ---
    fn ev_do_floor(&mut self, line_idx: usize, floor_type: FloorType) -> bool;

    // --- Cross-module: Ceilings ---
    fn ev_do_ceiling(&mut self, line_idx: usize, ceil_type: CeilingType) -> bool;
    fn ev_ceiling_crush_stop(&mut self, line_idx: usize) -> bool;

    // --- Cross-module: Platforms ---
    fn ev_do_plat(&mut self, line_idx: usize, plat_type: PlatType, amount: i32) -> bool;
    fn ev_stop_plat(&mut self, line_idx: usize);

    // --- Cross-module: Stairs ---
    fn ev_build_stairs(&mut self, line_idx: usize, stair_type: StairType) -> bool;

    // --- Cross-module: Teleport ---
    fn ev_teleport(&mut self, line_idx: usize, side: i32, thing_idx: usize) -> bool;

    // --- Cross-module: Lights ---
    fn ev_light_turn_on(&mut self, line_idx: usize, bright: i32);
    fn ev_turn_tag_lights_off(&mut self, line_idx: usize);
    fn ev_start_light_strobing(&mut self, line_idx: usize);

    // --- Cross-module: Light spawning ---
    fn p_spawn_light_flash(&mut self, sector_idx: usize);
    fn p_spawn_strobe_flash(&mut self, sector_idx: usize, darktime: i32, in_sync: bool);
    fn p_spawn_glowing_light(&mut self, sector_idx: usize);
    fn p_spawn_fire_flicker(&mut self, sector_idx: usize);

    // --- Cross-module: Door spawning ---
    fn p_spawn_door_close_in_30(&mut self, sector_idx: usize);
    fn p_spawn_door_raise_in_5_mins(&mut self, sector_idx: usize, sector_num: i32);

    // --- Cross-module: Switch textures ---
    fn p_change_switch_texture(&mut self, line_idx: usize, use_again: bool);

    // --- Cross-module: Game flow ---
    fn g_exit_level(&mut self);
    fn g_secret_exit_level(&mut self);

    // --- Cross-module: Damage ---
    fn p_damage_mobj(
        &mut self,
        target: usize,
        inflictor: Option<usize>,
        source: Option<usize>,
        damage: i32,
    );

    // --- Cross-module: Thinker management ---
    fn p_add_thinker_floor(&mut self, floor: FloorMoveT) -> usize;
    fn p_add_thinker_door(&mut self, door: VldoorT) -> usize;
    fn p_add_thinker_ceiling(&mut self, ceiling: CeilingT) -> usize;

    // --- Cross-module: Ceiling stasis management ---
    /// Re-activate all ceiling movers in stasis whose tag matches the given tag.
    fn p_activate_in_stasis_ceiling(&mut self, tag: i32);

    // --- Cross-module: Door re-use (bump logic) ---
    /// Returns a mutable reference to the door data for a given thinker
    /// data handle (as stored in `sector.specialdata`). Returns `None` if
    /// the handle does not refer to a door thinker.
    fn get_door_data_mut(&mut self, handle: usize) -> Option<&mut VldoorT>;

    // --- Cross-module: Texture/flat name resolution ---
    fn r_flat_num_for_name(&self, name: &str) -> i32;
    fn r_texture_num_for_name(&self, name: &str) -> i32;
    fn r_check_texture_num_for_name(&self, name: &str) -> i32;

    // --- Cross-module: Texture translation tables ---
    fn flat_translation(&self) -> &[i32];
    fn flat_translation_mut(&mut self) -> &mut Vec<i32>;
    fn texture_translation(&self) -> &[i32];
    fn texture_translation_mut(&mut self) -> &mut Vec<i32>;

    // --- Cross-module: Active ceiling/plat init ---
    fn init_active_ceilings(&mut self);
    fn init_active_plats(&mut self);

    // --- Cross-module: Texture height lookup ---
    /// Return the height of a texture (in 16.16 fixed-point) given a texture
    /// number.  Equivalent to the C global `textureheight[texnum]` in r_data.c.
    /// Used by `EV_DoFloor` for the `raiseToTexture` floor type.
    fn texture_height(&self, texnum: i32) -> Fixed;

    // --- Subsector lookup ---
    /// Returns the sector index for a given subsector index.
    fn subsector_sector(&self, subsector_idx: usize) -> usize;
}

// ============================================================================
// Utility functions (lines 397-490 of p_spec.c)
// ============================================================================

/// Get the SideDef for a given line and side index.
/// Translated from getSide() in p_spec.c.
pub fn get_side<'a>(
    _current_sector: usize,
    line: usize,
    side: usize,
    lines: &[LineDef],
    sides: &'a [SideDef],
) -> &'a SideDef {
    &sides[lines[line].sidenum[side] as usize]
}

/// Get the Sector for a given line and side index.
/// Translated from getSector() in p_spec.c.
pub fn get_sector<'a>(
    _current_sector: usize,
    line: usize,
    side: usize,
    lines: &[LineDef],
    sides: &[SideDef],
    sectors: &'a [Sector],
) -> &'a Sector {
    let side_idx = lines[line].sidenum[side] as usize;
    &sectors[sides[side_idx].sector]
}

/// Check if a line is two-sided.
/// Translated from twoSided() in p_spec.c.
pub fn two_sided(_sector: usize, line: usize, lines: &[LineDef]) -> bool {
    (lines[line].flags & LineFlags::ML_TWOSIDED.bits()) != 0
}

/// Get the sector on the other side of a line from the given sector.
/// Returns None if line is one-sided or sector is not on either side.
/// Translated from getNextSector() in p_spec.c.
pub fn get_next_sector(line: &LineDef, sector_idx: usize) -> Option<usize> {
    if (line.flags & LineFlags::ML_TWOSIDED.bits()) == 0 {
        return None;
    }
    if line.frontsector == Some(sector_idx) {
        line.backsector
    } else {
        line.frontsector
    }
}

/// Find the lowest floor height in surrounding sectors.
/// Initializes to the given sector's floor height.
/// Translated from P_FindLowestFloorSurrounding() in p_spec.c.
pub fn p_find_lowest_floor_surrounding(
    sector_idx: usize,
    sectors: &[Sector],
    lines: &[LineDef],
) -> Fixed {
    let sec = &sectors[sector_idx];
    let mut floor = sec.floorheight;

    for &line_idx in &sec.lines {
        let line = &lines[line_idx];
        if let Some(other_idx) = get_next_sector(line, sector_idx) {
            let other = &sectors[other_idx];
            if other.floorheight < floor {
                floor = other.floorheight;
            }
        }
    }
    floor
}

/// Find the highest floor height in surrounding sectors.
/// Initializes to -500*FRACUNIT per original C source.
/// Translated from P_FindHighestFloorSurrounding() in p_spec.c.
pub fn p_find_highest_floor_surrounding(
    sector_idx: usize,
    sectors: &[Sector],
    lines: &[LineDef],
) -> Fixed {
    let sec = &sectors[sector_idx];
    let mut floor = Fixed::new(-500 * FRACUNIT);

    for &line_idx in &sec.lines {
        let line = &lines[line_idx];
        if let Some(other_idx) = get_next_sector(line, sector_idx) {
            let other = &sectors[other_idx];
            if other.floorheight > floor {
                floor = other.floorheight;
            }
        }
    }
    floor
}

/// Find the next highest floor above currentheight in neighboring sectors.
/// Uses MAX_ADJOINING_SECTORS limit. Returns currentheight if none found.
/// Translated from P_FindNextHighestFloor() in p_spec.c.
pub fn p_find_next_highest_floor(
    sector_idx: usize,
    currentheight: Fixed,
    sectors: &[Sector],
    lines: &[LineDef],
) -> Fixed {
    let sec = &sectors[sector_idx];
    let mut heightlist = [Fixed::ZERO; MAX_ADJOINING_SECTORS];
    let mut h: usize = 0;

    for i in 0..sec.lines.len() {
        let line_idx = sec.lines[i];
        let line = &lines[line_idx];
        if let Some(other_idx) = get_next_sector(line, sector_idx) {
            let other = &sectors[other_idx];
            if other.floorheight > currentheight && h < MAX_ADJOINING_SECTORS {
                heightlist[h] = other.floorheight;
                h += 1;
            }
        }
    }

    if h == 0 {
        return currentheight;
    }

    let mut height = heightlist[0];
    for item in heightlist.iter().take(h).skip(1) {
        if *item < height {
            height = *item;
        }
    }
    height
}

/// Find the lowest ceiling height in surrounding sectors.
/// Initializes to i32::MAX.
/// Translated from P_FindLowestCeilingSurrounding() in p_spec.c.
pub fn p_find_lowest_ceiling_surrounding(
    sector_idx: usize,
    sectors: &[Sector],
    lines: &[LineDef],
) -> Fixed {
    let sec = &sectors[sector_idx];
    let mut height = Fixed::new(i32::MAX);

    for &line_idx in &sec.lines {
        let line = &lines[line_idx];
        if let Some(other_idx) = get_next_sector(line, sector_idx) {
            let other = &sectors[other_idx];
            if other.ceilingheight < height {
                height = other.ceilingheight;
            }
        }
    }
    height
}

/// Find the highest ceiling height in surrounding sectors.
/// Initializes to 0.
/// Translated from P_FindHighestCeilingSurrounding() in p_spec.c.
pub fn p_find_highest_ceiling_surrounding(
    sector_idx: usize,
    sectors: &[Sector],
    lines: &[LineDef],
) -> Fixed {
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

/// Find a sector by tag, starting search from (start+1).
/// Returns sector index or -1 if not found.
/// Translated from P_FindSectorFromLineTag() in p_spec.c.
pub fn p_find_sector_from_line_tag(line: &LineDef, start: i32, sectors: &[Sector]) -> i32 {
    let tag = line.tag;
    let begin = (start + 1) as usize;
    for (i, sector) in sectors.iter().enumerate().skip(begin) {
        if sector.tag == tag {
            return i as i32;
        }
    }
    -1
}

/// Find the minimum light level in surrounding sectors.
/// Initializes to the provided max value.
/// Translated from P_FindMinSurroundingLight() in p_spec.c.
pub fn p_find_min_surrounding_light(
    sector_idx: usize,
    max: i32,
    sectors: &[Sector],
    lines: &[LineDef],
) -> i32 {
    let sec = &sectors[sector_idx];
    let mut min = max;

    for &line_idx in &sec.lines {
        let line = &lines[line_idx];
        if let Some(other_idx) = get_next_sector(line, sector_idx) {
            let other = &sectors[other_idx];
            if (other.lightlevel as i32) < min {
                min = other.lightlevel as i32;
            }
        }
    }
    min
}

// ============================================================================
// P_InitPicAnims — Initialize flat and texture animation sequences
// Translated from lines 288-340 of p_spec.c
// ============================================================================

/// Initialize picture animations from the ANIM_DEFS table.
/// Resolves texture/flat names to numeric indices and validates frame counts.
pub fn p_init_pic_anims(ctx: &mut dyn SpecContext) {
    let mut last_anim_idx: usize = 0;
    let mut anims = [AnimDef::default(); MAX_ANIMS];

    for adef in ANIM_DEFS.iter() {
        // Terminator: empty endname signals end of table
        if adef.endname.is_empty() {
            break;
        }

        let pic_num;
        let base_pic;

        if adef.is_texture {
            // Check if texture exists before requiring it
            let check = ctx.r_check_texture_num_for_name(adef.startname);
            if check == -1 {
                continue;
            }
            pic_num = ctx.r_texture_num_for_name(adef.endname);
            base_pic = ctx.r_texture_num_for_name(adef.startname);
        } else {
            pic_num = ctx.r_flat_num_for_name(adef.endname);
            base_pic = ctx.r_flat_num_for_name(adef.startname);
        }

        let num_pics = pic_num - base_pic + 1;
        if num_pics < 2 {
            // Animation needs at least 2 frames; skip invalid entries.
            continue;
        }

        if last_anim_idx >= MAX_ANIMS {
            break;
        }

        anims[last_anim_idx] = AnimDef {
            is_texture: adef.is_texture,
            pic_num,
            base_pic,
            num_pics,
            speed: adef.speed,
        };
        last_anim_idx += 1;
    }

    let state = ctx.spec_state_mut();
    state.anims = anims;
    state.last_anim_idx = last_anim_idx;
}

// ============================================================================
// P_CrossSpecialLine — Called every time a thing origin is about to cross
//                      a line with a non-zero special.
// Translated from lines 491-950 of p_spec.c
// ============================================================================

/// Called every time a thing crosses a line with a non-zero special.
/// Dispatches to the appropriate subsystem based on the line special number.
///
/// For TRIGGER specials, the line.special is cleared after activation.
/// For RETRIGGER specials, the line.special is preserved.
pub fn p_cross_special_line(
    line_idx: usize,
    side: i32,
    thing_idx: usize,
    ctx: &mut dyn SpecContext,
) {
    let line_special;
    let is_player;
    let thing_type;
    let thing_flags;

    // Read thing properties
    {
        let thing = &ctx.mobjs()[thing_idx];
        is_player = thing.player.is_some();
        thing_type = thing.type_;
        thing_flags = thing.flags;
    }

    // Read line special
    {
        line_special = ctx.lines()[line_idx].special;
    }

    // Monster (non-player) filtering:
    // Only certain line specials can be triggered by monsters.
    if !is_player {
        // Check if this is a missile projectile — missiles cannot trigger specials.
        if thing_flags.contains(MobjFlags::MF_MISSILE) {
            return;
        }

        // Also reject specific projectile types by type ID.
        match thing_type {
            t if t == MobjType::MT_ROCKET as usize => return,
            t if t == MobjType::MT_PLASMA as usize => return,
            t if t == MobjType::MT_BFG as usize => return,
            t if t == MobjType::MT_TROOPSHOT as usize => return,
            t if t == MobjType::MT_HEADSHOT as usize => return,
            t if t == MobjType::MT_BRUISERSHOT as usize => return,
            _ => {}
        }

        // Monsters can only trigger these specific line specials:
        match line_special {
            39 | 97 | 125 | 126 => {} // Teleport specials
            4 | 10 | 88 => {}         // Door/plat specials
            _ => return,              // All others: reject non-player
        }
    }

    match line_special {
        // ================================================================
        // TRIGGERS — line.special cleared after activation
        // ================================================================

        // Case 2: Open Door Stay (W1)
        2 => {
            ctx.ev_do_door(line_idx, VldoorType::Open);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 3: Close Door (W1)
        3 => {
            ctx.ev_do_door(line_idx, VldoorType::Close);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 4: Raise Door (W1)
        4 => {
            ctx.ev_do_door(line_idx, VldoorType::Normal);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 5: Raise Floor (W1)
        5 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloor);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 6: Fast Ceiling Crush & Raise (W1)
        6 => {
            ctx.ev_do_ceiling(line_idx, CeilingType::FastCrushAndRaise);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 8: Build Stairs (W1)
        8 => {
            ctx.ev_build_stairs(line_idx, StairType::Build8);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 10: PlatDownWaitUpStay (W1)
        10 => {
            ctx.ev_do_plat(line_idx, PlatType::DownWaitUpStay, 0);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 12: Light Turn On - brightest adjacent (W1)
        12 => {
            ctx.ev_light_turn_on(line_idx, 0);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 13: Light Turn On 255 (W1)
        13 => {
            ctx.ev_light_turn_on(line_idx, 255);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 16: Close Door 30 (W1)
        16 => {
            ctx.ev_do_door(line_idx, VldoorType::Close30ThenOpen);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 17: Start Light Strobing (W1)
        17 => {
            ctx.ev_start_light_strobing(line_idx);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 19: Lower Floor (W1)
        19 => {
            ctx.ev_do_floor(line_idx, FloorType::LowerFloor);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 22: Plat Raise and Change (W1)
        22 => {
            ctx.ev_do_plat(line_idx, PlatType::RaiseToNearestAndChange, 0);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 25: Ceiling Crush and Raise (W1)
        25 => {
            ctx.ev_do_ceiling(line_idx, CeilingType::CrushAndRaise);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 30: Raise Floor to Shortest Texture (W1)
        30 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseToTexture);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 35: Lights Very Dark (W1)
        35 => {
            ctx.ev_light_turn_on(line_idx, 35);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 36: Lower Floor Turbo (W1)
        36 => {
            ctx.ev_do_floor(line_idx, FloorType::TurboLower);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 37: Lower and Change (W1)
        37 => {
            ctx.ev_do_floor(line_idx, FloorType::LowerAndChange);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 38: Lower Floor To Lowest (W1)
        38 => {
            ctx.ev_do_floor(line_idx, FloorType::LowerFloorToLowest);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 39: Teleport (W1)
        39 => {
            ctx.ev_teleport(line_idx, side, thing_idx);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 40: RaiseCeilingLowerFloor (W1)
        40 => {
            ctx.ev_do_ceiling(line_idx, CeilingType::LowerToFloor);
            ctx.ev_do_floor(line_idx, FloorType::LowerFloorToLowest);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 44: Ceiling Crush (W1) - lower ceiling to 8 above floor
        44 => {
            ctx.ev_do_ceiling(line_idx, CeilingType::LowerAndCrush);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 52: Exit (W1)
        52 => {
            ctx.g_exit_level();
        }

        // Case 53: Perpetual Platform Raise (W1)
        53 => {
            ctx.ev_do_plat(line_idx, PlatType::PerpetualRaise, 0);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 54: Platform Stop (W1)
        54 => {
            ctx.ev_stop_plat(line_idx);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 56: Raise Floor Crush (W1)
        56 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloorCrush);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 57: Ceiling Crush Stop (W1)
        57 => {
            ctx.ev_ceiling_crush_stop(line_idx);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 58: Raise Floor 24 (W1)
        58 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloor24);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 59: Raise Floor 24 And Change (W1)
        59 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloor24AndChange);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 100: Build Stairs Turbo 16 (W1)
        100 => {
            ctx.ev_build_stairs(line_idx, StairType::Turbo16);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 104: Turn Lights Off in Sector Tag (W1)
        104 => {
            ctx.ev_turn_tag_lights_off(line_idx);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 108: Blazing Door Raise (W1)
        108 => {
            ctx.ev_do_door(line_idx, VldoorType::BlazeRaise);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 109: Blazing Door Open (W1)
        109 => {
            ctx.ev_do_door(line_idx, VldoorType::BlazeOpen);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 110: Blazing Door Close (W1)
        110 => {
            ctx.ev_do_door(line_idx, VldoorType::BlazeClose);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 119: Raise Floor To Nearest (W1)
        119 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloorToNearest);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 121: Blazing PlatDownWaitUpStay (W1)
        121 => {
            ctx.ev_do_plat(line_idx, PlatType::BlazeDWUS, 0);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 124: Secret Exit (W1)
        124 => {
            ctx.g_secret_exit_level();
        }

        // Case 125: Teleport MonsterOnly (W1)
        125 => {
            if !is_player {
                ctx.ev_teleport(line_idx, side, thing_idx);
                ctx.lines_mut()[line_idx].special = 0;
            }
        }

        // Case 130: Raise Floor Turbo (W1)
        130 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloorTurbo);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // Case 141: Silent Ceiling Crush & Raise (W1)
        141 => {
            ctx.ev_do_ceiling(line_idx, CeilingType::SilentCrushAndRaise);
            ctx.lines_mut()[line_idx].special = 0;
        }

        // ================================================================
        // RETRIGGERS — line.special is NOT cleared
        // ================================================================

        // Case 72: Ceiling Crush (WR)
        72 => {
            ctx.ev_do_ceiling(line_idx, CeilingType::LowerAndCrush);
        }

        // Case 73: Ceiling Crush and Raise (WR)
        73 => {
            ctx.ev_do_ceiling(line_idx, CeilingType::CrushAndRaise);
        }

        // Case 74: Ceiling Crush Stop (WR)
        74 => {
            ctx.ev_ceiling_crush_stop(line_idx);
        }

        // Case 75: Close Door (WR)
        75 => {
            ctx.ev_do_door(line_idx, VldoorType::Close);
        }

        // Case 76: Close Door 30 (WR)
        76 => {
            ctx.ev_do_door(line_idx, VldoorType::Close30ThenOpen);
        }

        // Case 77: Fast Ceiling Crush & Raise (WR)
        77 => {
            ctx.ev_do_ceiling(line_idx, CeilingType::FastCrushAndRaise);
        }

        // Case 79: Lights Very Dark (WR)
        79 => {
            ctx.ev_light_turn_on(line_idx, 35);
        }

        // Case 80: Light Turn On - brightest adjacent (WR)
        80 => {
            ctx.ev_light_turn_on(line_idx, 0);
        }

        // Case 81: Light Turn On 255 (WR)
        81 => {
            ctx.ev_light_turn_on(line_idx, 255);
        }

        // Case 82: Lower Floor To Lowest (WR)
        82 => {
            ctx.ev_do_floor(line_idx, FloorType::LowerFloorToLowest);
        }

        // Case 83: Lower Floor (WR)
        83 => {
            ctx.ev_do_floor(line_idx, FloorType::LowerFloor);
        }

        // Case 84: LowerAndChange (WR)
        84 => {
            ctx.ev_do_floor(line_idx, FloorType::LowerAndChange);
        }

        // Case 86: Open Door Stay (WR)
        86 => {
            ctx.ev_do_door(line_idx, VldoorType::Open);
        }

        // Case 87: Perpetual Platform Raise (WR)
        87 => {
            ctx.ev_do_plat(line_idx, PlatType::PerpetualRaise, 0);
        }

        // Case 88: PlatDownWaitUpStay (WR)
        88 => {
            ctx.ev_do_plat(line_idx, PlatType::DownWaitUpStay, 0);
        }

        // Case 89: Platform Stop (WR)
        89 => {
            ctx.ev_stop_plat(line_idx);
        }

        // Case 90: Raise Door (WR)
        90 => {
            ctx.ev_do_door(line_idx, VldoorType::Normal);
        }

        // Case 91: Raise Floor (WR)
        91 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloor);
        }

        // Case 92: Raise Floor 24 (WR)
        92 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloor24);
        }

        // Case 93: Raise Floor 24 And Change (WR)
        93 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloor24AndChange);
        }

        // Case 94: Raise Floor Crush (WR)
        94 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloorCrush);
        }

        // Case 95: Raise To Nearest And Change (Plat) (WR)
        95 => {
            ctx.ev_do_plat(line_idx, PlatType::RaiseToNearestAndChange, 0);
        }

        // Case 96: Raise Floor to Shortest Texture (WR)
        96 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseToTexture);
        }

        // Case 97: Teleport (WR)
        97 => {
            ctx.ev_teleport(line_idx, side, thing_idx);
        }

        // Case 98: Lower Floor Turbo (WR)
        98 => {
            ctx.ev_do_floor(line_idx, FloorType::TurboLower);
        }

        // Case 105: Blazing Door Raise (WR)
        105 => {
            ctx.ev_do_door(line_idx, VldoorType::BlazeRaise);
        }

        // Case 106: Blazing Door Open (WR)
        106 => {
            ctx.ev_do_door(line_idx, VldoorType::BlazeOpen);
        }

        // Case 107: Blazing Door Close (WR)
        107 => {
            ctx.ev_do_door(line_idx, VldoorType::BlazeClose);
        }

        // Case 120: Blazing PlatDownWaitUpStay (WR)
        120 => {
            ctx.ev_do_plat(line_idx, PlatType::BlazeDWUS, 0);
        }

        // Case 126: Teleport MonsterOnly (WR)
        126 => {
            if !is_player {
                ctx.ev_teleport(line_idx, side, thing_idx);
            }
        }

        // Case 128: Raise Floor To Nearest (WR)
        128 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloorToNearest);
        }

        // Case 129: Raise Floor Turbo (WR)
        129 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloorTurbo);
        }

        // All other specials: no action on cross
        _ => {}
    }
}

// ============================================================================
// P_ShootSpecialLine — Called when a thing uses a gun on a special line.
// Translated from lines 958-1000 of p_spec.c
// ============================================================================

/// Called when a thing shoots a special line with a gun (hitscan).
/// Only certain line specials can be triggered by gunfire.
pub fn p_shoot_special_line(thing_idx: usize, line_idx: usize, ctx: &mut dyn SpecContext) {
    let is_player;
    let line_special;

    {
        let thing = &ctx.mobjs()[thing_idx];
        is_player = thing.player.is_some();
    }
    {
        line_special = ctx.lines()[line_idx].special;
    }

    // Only players can trigger gun specials, except for line 46 (open door).
    if !is_player {
        match line_special {
            46 => {} // GUN type: open door — monsters allowed
            _ => return,
        }
    }

    match line_special {
        // Case 24: Raise Floor (G1)
        24 => {
            ctx.ev_do_floor(line_idx, FloorType::RaiseFloor);
            ctx.p_change_switch_texture(line_idx, false);
        }

        // Case 46: Open Door (GR) — GUN type, repeatable
        46 => {
            ctx.ev_do_door(line_idx, VldoorType::Open);
            ctx.p_change_switch_texture(line_idx, true);
        }

        // Case 47: Plat Raise To Nearest And Change (G1)
        47 => {
            ctx.ev_do_plat(line_idx, PlatType::RaiseToNearestAndChange, 0);
            ctx.p_change_switch_texture(line_idx, false);
        }

        _ => {}
    }
}

// ============================================================================
// P_PlayerInSpecialSector — Called every tic for each player in a special sector.
// Translated from lines 1009-1071 of p_spec.c
// ============================================================================

/// Handle sector-based special effects on a player.
/// Applies damage, counts secrets, and triggers E1M8 exit.
pub fn p_player_in_special_sector(player_idx: usize, ctx: &mut dyn SpecContext) {
    // Get the player's mobj index and sector info
    let mobj_idx;
    let mobj_z;
    let sector_floorheight;
    let sector_special;
    let sector_idx;

    {
        let player = &ctx.players()[player_idx];
        mobj_idx = match player.mobj {
            Some(idx) => idx,
            None => return,
        };
    }

    {
        let mobj = &ctx.mobjs()[mobj_idx];
        mobj_z = mobj.z;
        let subsector_idx = match mobj.subsector {
            Some(idx) => idx,
            None => return,
        };
        // Resolve subsector → sector via context
        sector_idx = ctx.subsector_sector(subsector_idx);
    }

    {
        let sector = &ctx.sectors()[sector_idx];
        sector_floorheight = sector.floorheight;
        sector_special = sector.special;
    }

    // Player must be on the floor to be affected
    if mobj_z != sector_floorheight {
        return;
    }

    let leveltime = ctx.leveltime();

    match sector_special {
        // Case 5: HELLSLIME DAMAGE — 10 damage every 32 tics
        5 => {
            let player = &ctx.players()[player_idx];
            if player.powers[PowerType::IronFeet as usize] == 0 && (leveltime & 0x1f) == 0 {
                ctx.p_damage_mobj(mobj_idx, None, None, 10);
            }
        }

        // Case 7: NUKAGE DAMAGE — 5 damage every 32 tics
        7 => {
            let player = &ctx.players()[player_idx];
            if player.powers[PowerType::IronFeet as usize] == 0 && (leveltime & 0x1f) == 0 {
                ctx.p_damage_mobj(mobj_idx, None, None, 5);
            }
        }

        // Case 16, 4: SUPER HELLSLIME DAMAGE — 20 damage per tick, more frequent
        16 | 4 => {
            let player = &ctx.players()[player_idx];
            if (player.powers[PowerType::IronFeet as usize] == 0 || ctx.rng_mut().p_random() < 5)
                && (leveltime & 0x1f) == 0
            {
                ctx.p_damage_mobj(mobj_idx, None, None, 20);
            }
        }

        // Case 9: SECRET SECTOR — count and clear
        9 => {
            ctx.players_mut()[player_idx].secretcount += 1;
            ctx.sectors_mut()[sector_idx].special = 0;
        }

        // Case 11: E1M8 EXIT — 20 damage, exit when health drops to 10 or below
        11 => {
            // Clear god mode flag for E1M8 ending
            ctx.players_mut()[player_idx].cheats &= !CheatFlags::CF_GODMODE.bits();

            if (leveltime & 0x1f) == 0 {
                ctx.p_damage_mobj(mobj_idx, None, None, 20);
            }

            let player_health = ctx.players()[player_idx].health;
            if player_health <= 10 {
                ctx.g_exit_level();
            }
        }

        _ => {}
    }
}

// ============================================================================
// P_UpdateSpecials — Called every tic to update animations, scrolling, buttons.
// Translated from lines 1083-1156 of p_spec.c
// ============================================================================

/// Animate flats, textures, scroll line 48, handle button countdowns, level timer.
pub fn p_update_specials(ctx: &mut dyn SpecContext) {
    // --- LEVEL TIMER ---
    {
        let is_timer = ctx.spec_state().level_timer;
        if is_timer {
            ctx.spec_state_mut().level_time_count -= 1;
            if ctx.spec_state().level_time_count == 0 {
                ctx.g_exit_level();
                return;
            }
        }
    }

    // --- ANIMATE FLATS AND TEXTURES GLOBALLY ---
    // Each animation frame in the group gets the same offset for global sync,
    // but with per-frame phase shifting via the `+ i` term.
    {
        let leveltime = ctx.leveltime();
        let state = ctx.spec_state();
        let last_anim = state.last_anim_idx;

        // Collect animation data to avoid borrow issues
        let mut anim_data: Vec<(bool, i32, i32, i32)> = Vec::new();
        for a in 0..last_anim {
            let anim = &state.anims[a];
            if anim.num_pics > 0 && anim.speed > 0 {
                anim_data.push((anim.is_texture, anim.base_pic, anim.num_pics, anim.speed));
            }
        }

        for (is_texture, base_pic, num_pics, speed) in anim_data {
            for i in base_pic..(base_pic + num_pics) {
                let pic = base_pic + ((leveltime / speed + i) % num_pics);

                if is_texture {
                    let trans = ctx.texture_translation_mut();
                    if (i as usize) < trans.len() {
                        trans[i as usize] = pic;
                    }
                } else {
                    let trans = ctx.flat_translation_mut();
                    if (i as usize) < trans.len() {
                        trans[i as usize] = pic;
                    }
                }
            }
        }
    }

    // --- ANIMATE LINE SPECIALS (Scroll line type 48) ---
    {
        let state = ctx.spec_state();
        let num_specials = state.num_line_specials;
        let mut scroll_targets: Vec<usize> = Vec::with_capacity(num_specials);

        for i in 0..num_specials {
            let line_idx = state.line_special_list[i];
            scroll_targets.push(line_idx);
        }

        let lines = ctx.lines();
        let mut side_indices: Vec<usize> = Vec::with_capacity(scroll_targets.len());
        for &line_idx in &scroll_targets {
            if line_idx < lines.len() {
                let sidenum = lines[line_idx].sidenum[0];
                if sidenum >= 0 {
                    side_indices.push(sidenum as usize);
                }
            }
        }

        let sides = ctx.sides_mut();
        for side_idx in side_indices {
            if side_idx < sides.len() {
                sides[side_idx].textureoffset = sides[side_idx].textureoffset + Fixed::from_int(1);
            }
        }
    }

    // --- DO BUTTONS ---
    {
        let state = ctx.spec_state();
        // Gather buttons that need processing
        let mut expired_buttons: Vec<(usize, BWhere, i32, Option<usize>)> = Vec::new();
        let mut decrement_indices: Vec<usize> = Vec::new();

        for i in 0..MAXBUTTONS {
            if state.buttonlist[i].btimer > 0 {
                let remaining = state.buttonlist[i].btimer - 1;
                if remaining == 0 {
                    // Button timer expired — need to restore texture and clear
                    let btn = &state.buttonlist[i];
                    let line_idx = btn.line;
                    let where_pos = btn.where_pos;
                    let btexture = btn.btexture;
                    let soundorg = btn.soundorg;
                    if let Some(li) = line_idx {
                        expired_buttons.push((li, where_pos, btexture, soundorg));
                    }
                }
                decrement_indices.push(i);
            }
        }

        // Decrement timers
        let state_mut = ctx.spec_state_mut();
        for &i in &decrement_indices {
            state_mut.buttonlist[i].btimer -= 1;
        }

        // Process expired buttons: restore texture and play sound
        for (line_idx, where_pos, btexture, soundorg) in &expired_buttons {
            let sidenum = ctx.lines()[*line_idx].sidenum[0];
            if sidenum >= 0 {
                let side_idx = sidenum as usize;
                let sides = ctx.sides_mut();
                if side_idx < sides.len() {
                    match where_pos {
                        BWhere::Top => {
                            sides[side_idx].toptexture = *btexture as i16;
                        }
                        BWhere::Middle => {
                            sides[side_idx].midtexture = *btexture as i16;
                        }
                        BWhere::Bottom => {
                            sides[side_idx].bottomtexture = *btexture as i16;
                        }
                    }
                }
            }
            ctx.s_start_sound(*soundorg, SfxEnum::sfx_swtchn);
        }

        // Clear expired button entries
        if !expired_buttons.is_empty() {
            let state_mut = ctx.spec_state_mut();
            for i in 0..MAXBUTTONS {
                if state_mut.buttonlist[i].btimer == 0 && state_mut.buttonlist[i].line.is_some() {
                    // Check if this was one of the expired ones
                    // Since btimer was just decremented to 0 and line is Some, clear it
                    state_mut.buttonlist[i] = ButtonT::default();
                }
            }
        }
    }
}

// ============================================================================
// EV_DoDonut — Donut special (raise inner, lower outer).
// Translated from lines 1163-1221 of p_spec.c
// ============================================================================

/// Execute a donut special (p_spec.c EV_DoDonut).
///
/// s1 = tagged sector (the "hole" — will be LOWERED to s3's floor height)
/// s2 = donut ring sector (surrounding s1 — will be RAISED to s3's floor height)
/// s3 = outer sector (surrounding s2 — provides target height and texture)
pub fn ev_do_donut(line_idx: usize, ctx: &mut dyn SpecContext) -> bool {
    let mut rtn = false;
    let line_tag;

    {
        line_tag = ctx.lines()[line_idx].tag;
    }

    let mut secnum: i32 = -1;
    let lines_len = ctx.lines().len();

    loop {
        // Find next sector with matching tag
        secnum = {
            let sectors = ctx.sectors();
            p_find_sector_from_line_tag_raw(line_tag, secnum, sectors)
        };

        if secnum < 0 {
            break;
        }

        let s1_idx = secnum as usize;

        // Skip if sector already has active special
        {
            if ctx.sectors()[s1_idx].specialdata.is_some() {
                continue;
            }
        }

        rtn = true;

        // Find s2: first two-sided line of s1 → neighbor sector
        let s2_idx;
        {
            let sector = &ctx.sectors()[s1_idx];
            let mut found_s2 = None;
            for &line_i in &sector.lines {
                if line_i < lines_len {
                    let line = &ctx.lines()[line_i];
                    if line.flags & LineFlags::ML_TWOSIDED.bits() != 0 {
                        // Get next sector (the one that isn't s1)
                        let sectors = ctx.sectors();
                        let lines = ctx.lines();
                        if let Some(ns) = get_next_sector_by_idx(&lines[line_i], s1_idx, sectors) {
                            found_s2 = Some(ns);
                            break;
                        }
                    }
                }
            }

            s2_idx = match found_s2 {
                Some(idx) => idx,
                None => continue,
            };
        }

        // Find s3: first two-sided line of s2 where the other sector is NOT s1
        let s3_idx;
        {
            let sector_s2 = &ctx.sectors()[s2_idx];
            let mut found_s3 = None;
            for &line_i in &sector_s2.lines {
                if line_i < lines_len {
                    let line = &ctx.lines()[line_i];
                    // Note: Original C code has `(!s2->lines[i]->flags & ML_TWOSIDED)` which is
                    // an operator precedence bug (! before &). We replicate the INTENDED behavior:
                    // check if the line IS two-sided.
                    if line.flags & LineFlags::ML_TWOSIDED.bits() == 0 {
                        continue;
                    }
                    let sectors = ctx.sectors();
                    if let Some(ns) = get_next_sector_by_idx(&ctx.lines()[line_i], s2_idx, sectors)
                    {
                        if ns != s1_idx {
                            found_s3 = Some(ns);
                            break;
                        }
                    }
                }
            }

            s3_idx = match found_s3 {
                Some(idx) => idx,
                None => continue,
            };
        }

        // Read target values from s3
        let s3_floorheight;
        let s3_floorpic;

        {
            let s3 = &ctx.sectors()[s3_idx];
            s3_floorheight = s3.floorheight;
            s3_floorpic = s3.floorpic;
        }

        // C: "Spawn rising slime" — floor mover for s2 (ring RAISES to s3 height)
        let floor_ring = FloorMoveT {
            thinker: Thinker {
                prev: None,
                next: None,
                function: ActionFn::MoveFloor,
            },
            sector: s2_idx,
            floor_type: FloorType::DonutRaise,
            crush: false,
            direction: 1,
            newsecspecial: 0,
            newtexture: s3_floorpic,
            floordestheight: s3_floorheight,
            speed: Fixed::new(FLOORSPEED / 2),
        };
        ctx.p_add_thinker_floor(floor_ring);
        ctx.sectors_mut()[s2_idx].specialdata = Some(s2_idx); // Mark as active

        // C: "Spawn lowering donut-hole" — floor mover for s1 (hole LOWERS to s3 height)
        let floor_hole = FloorMoveT {
            thinker: Thinker {
                prev: None,
                next: None,
                function: ActionFn::MoveFloor,
            },
            sector: s1_idx,
            floor_type: FloorType::LowerFloor,
            crush: false,
            direction: -1,
            newsecspecial: 0,
            newtexture: s3_floorpic,
            floordestheight: s3_floorheight,
            speed: Fixed::new(FLOORSPEED / 2),
        };
        ctx.p_add_thinker_floor(floor_hole);
        ctx.sectors_mut()[s1_idx].specialdata = Some(s1_idx); // Mark as active
    }

    rtn
}

/// Internal helper: find next sector across a two-sided line, by sector index.
/// Returns the sector on the other side of the line from `sector_idx`.
fn get_next_sector_by_idx(line: &LineDef, sector_idx: usize, _sectors: &[Sector]) -> Option<usize> {
    if line.flags & LineFlags::ML_TWOSIDED.bits() == 0 {
        return None;
    }
    if line.frontsector == Some(sector_idx) {
        line.backsector
    } else {
        line.frontsector
    }
}

/// Internal helper: find sector from line tag, operating on raw slices.
fn p_find_sector_from_line_tag_raw(tag: i16, start: i32, sectors: &[Sector]) -> i32 {
    let begin = (start + 1) as usize;
    for (i, sec) in sectors.iter().enumerate().skip(begin) {
        if sec.tag == tag {
            return i as i32;
        }
    }
    -1
}

// ============================================================================
// P_SpawnSpecials — Initialize sector specials and line specials at level load.
// Translated from lines 1239-1362 of p_spec.c
// ============================================================================

/// Called at level load after P_SetupLevel. Initializes:
/// - Level timer from -avg or -timer command-line arguments
/// - Active ceiling and platform lists
/// - Button list
/// - Sector light effects and door specials
/// - Line scrolling special list (type 48)
pub fn p_spawn_specials(ctx: &mut dyn SpecContext) {
    // Check for -timer command line (deathmatch timer)
    if ctx.deathmatch() != 0 {
        let args = ctx.args().clone();
        if let Some(minutes_str) = args.parm_value("-timer") {
            if let Ok(minutes) = minutes_str.parse::<i32>() {
                let state = ctx.spec_state_mut();
                state.level_timer = true;
                state.level_time_count = minutes * 60 * TICRATE;
                tracing::info!("Deathmatch timer: {} minutes", minutes);
            }
        }
    }

    // Check for -avg (Austin Virtual Gaming — 20 minute timer)
    {
        let args = ctx.args().clone();
        if args.check_parm("-avg").is_some() {
            let state = ctx.spec_state_mut();
            state.level_timer = true;
            state.level_time_count = 20 * 60 * TICRATE;
            tracing::info!("Austin Virtual Gaming timer: 20 minutes");
        }
    }

    // Init active ceilings and platforms
    ctx.init_active_ceilings();
    ctx.init_active_plats();

    // Clear button list
    {
        let state = ctx.spec_state_mut();
        for i in 0..MAXBUTTONS {
            state.buttonlist[i] = ButtonT::default();
        }
    }

    // Iterate over all sectors and spawn appropriate specials
    let num_sectors = ctx.sectors().len();
    for i in 0..num_sectors {
        let sector_special;
        {
            sector_special = ctx.sectors()[i].special;
        }

        match sector_special {
            // Case 1: FLICKERING LIGHTS — random light flash
            1 => {
                ctx.p_spawn_light_flash(i);
            }

            // Case 2: STROBE FAST (FASTDARK=15 tics dark)
            2 => {
                ctx.p_spawn_strobe_flash(i, FASTDARK, false);
            }

            // Case 3: STROBE SLOW (SLOWDARK=35 tics dark)
            3 => {
                ctx.p_spawn_strobe_flash(i, SLOWDARK, false);
            }

            // Case 4: STROBE FAST + Super Hellslime Damage
            // Spawn strobe, then reassign special to 4 so damage logic is preserved
            4 => {
                ctx.p_spawn_strobe_flash(i, FASTDARK, false);
                ctx.sectors_mut()[i].special = 4;
            }

            // Case 5: HELLSLIME DAMAGE — no light effect, handled in P_PlayerInSpecialSector
            5 => {}

            // Case 7: NUKAGE DAMAGE — no light effect, handled in P_PlayerInSpecialSector
            7 => {}

            // Case 8: GLOWING LIGHT
            8 => {
                ctx.p_spawn_glowing_light(i);
            }

            // Case 9: SECRET SECTOR — increment total secret counter
            9 => {
                ctx.totalsecret_inc();
            }

            // Case 10: DOOR CLOSE IN 30 SECONDS
            10 => {
                ctx.p_spawn_door_close_in_30(i);
            }

            // Case 11: E1M8 EXIT DAMAGE — no light effect, handled in P_PlayerInSpecialSector
            11 => {}

            // Case 12: SYNC STROBE SLOW (in-sync)
            12 => {
                ctx.p_spawn_strobe_flash(i, SLOWDARK, true);
            }

            // Case 13: SYNC STROBE FAST (in-sync)
            13 => {
                ctx.p_spawn_strobe_flash(i, FASTDARK, true);
            }

            // Case 14: DOOR RAISE IN 5 MINUTES
            14 => {
                ctx.p_spawn_door_raise_in_5_mins(i, i as i32);
            }

            // Case 16: SUPER HELLSLIME DAMAGE — no light effect
            16 => {}

            // Case 17: FIRE FLICKER
            17 => {
                ctx.p_spawn_fire_flicker(i);
            }

            _ => {}
        }
    }

    // Init line EFFECTs — build list of scrolling lines (type 48)
    {
        let num_lines = ctx.lines().len();
        let mut list_idx = 0;

        for i in 0..num_lines {
            let special = ctx.lines()[i].special;
            if special == 48 && list_idx < MAXLINEANIMS {
                let state = ctx.spec_state_mut();
                state.line_special_list[list_idx] = i;
                list_idx += 1;
                state.num_line_specials = list_idx;
            }
        }
    }
}
