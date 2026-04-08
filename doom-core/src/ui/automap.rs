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

//! The automap code.
//!
//! Translated from linuxdoom-1.10/am_map.c and am_map.h
//!
//! Implements the full automap overlay showing map geometry, player position,
//! things, grid lines, and mark points. Supports zoom, pan, follow mode,
//! mark/clear points, and cheat reveal modes (IDDT).
//!
//! # Architecture
//!
//! All state is stored in [`AutomapState`] — no `static mut` is used. Public
//! API functions accept `&mut AutomapState` plus references to any external
//! game data they need (map geometry, players, map objects, video state).
//!
//! # Color Scheme
//!
//! The automap uses palette indices from the PLAYPAL lump to color different
//! line types. See the internal color constants (`WALLCOLORS`, `FDWALLCOLORS`,
//! etc.) for the exact palette ranges used.

use crate::game::strings::{
    AMSTR_FOLLOWOFF, AMSTR_FOLLOWON, AMSTR_GRIDOFF, AMSTR_GRIDON, AMSTR_MARKEDSPOT,
    AMSTR_MARKSCLEARED,
};
use crate::types::angle::Angle;
use crate::types::doomdef::{
    PowerType, KEY_DOWNARROW, KEY_LEFTARROW, KEY_RIGHTARROW, KEY_TAB, KEY_UPARROW, MAXPLAYERS,
    SCREENHEIGHT, SCREENWIDTH,
};
use crate::types::event::{Event, EventType};
use crate::types::fixed::{Fixed, FRACBITS, FRACUNIT};
use crate::types::map_data::{LineDef, LineFlags, Sector, Vertex};
use crate::types::mobj::MapObject;
use crate::types::player::Player;
use crate::types::tables::{finecosine, FINESINE};
use crate::util::cheat::{scramble, CheatSeq};
use crate::video::video::VideoState;

// =============================================================================
// Public constants (from am_map.h)
// =============================================================================

/// Automap message header — identifies messages from the automap subsystem.
///
/// Constructed from ASCII 'a' and 'm' packed into the upper two bytes.
/// Original C: `#define AM_MSGHEADER (('a'<<24)+('m'<<16))`
pub const AM_MSGHEADER: i32 = ((b'a' as i32) << 24) + ((b'm' as i32) << 16);

/// Message sent when the automap is entered (opened).
///
/// The status bar uses this to know it should stop drawing temporarily.
/// Original C: `#define AM_MSGENTERED (AM_MSGHEADER | ('e'<<8))`
pub const AM_MSGENTERED: i32 = AM_MSGHEADER | ((b'e' as i32) << 8);

/// Message sent when the automap is exited (closed).
///
/// The status bar uses this to resume its normal drawing.
/// Original C: `#define AM_MSGEXITED (AM_MSGHEADER | ('x'<<8))`
pub const AM_MSGEXITED: i32 = AM_MSGHEADER | ((b'x' as i32) << 8);

// =============================================================================
// Internal color constants — palette indices (am_map.c lines 52-85)
// =============================================================================

/// Red range start (palette index 176). Used for solid walls.
const REDS: u8 = (256 - 5 * 16) as u8;
/// Number of red shades available.
const REDRANGE: u8 = 16;
/// Blue range start (palette index 200).
#[allow(dead_code)]
const BLUES: u8 = (256 - 4 * 16 + 8) as u8;
/// Green range start (palette index 112). Used for things.
const GREENS: u8 = (7 * 16) as u8;
/// Gray range start (palette index 96). Used for two-sided same-height lines.
const GRAYS: u8 = (6 * 16) as u8;
/// Number of gray shades.
const GRAYSRANGE: u8 = 16;
/// Brown range start (palette index 64). Used for floor-height-change lines.
const BROWNS: u8 = (4 * 16) as u8;
/// Yellow range start (palette index 231). Used for ceiling-height-change lines.
const YELLOWS: u8 = (256 - 32 + 7) as u8;
/// Black (palette index 0). Used for automap background.
const BLACK: u8 = 0;
/// White (palette index 209). Used for the local player arrow.
const WHITE: u8 = (256 - 47) as u8;

// Semantic color aliases for automap line rendering.
/// Background fill color.
const BACKGROUND: u8 = BLACK;
/// Local player arrow color.
const YOURCOLORS: u8 = WHITE;
/// Solid (one-sided) wall color base.
const WALLCOLORS: u8 = REDS;
/// Wall range — number of shades for wall coloring.
#[allow(dead_code)]
const WALLRANGE: u8 = REDRANGE;
/// Two-sided wall (same-height) color base.
const TSWALLCOLORS: u8 = GRAYS;
/// Floor-different wall color base.
const FDWALLCOLORS: u8 = BROWNS;
/// Ceiling-different wall color base.
const CDWALLCOLORS: u8 = YELLOWS;
/// Thing triangle color base.
const THINGCOLORS: u8 = GREENS;
/// Secret wall color base (same as regular walls, revealed by cheat).
const SECRETWALLCOLORS: u8 = WALLCOLORS;
/// Grid overlay color.
const GRIDCOLORS: u8 = GRAYS + GRAYSRANGE / 2;
/// Crosshair color.
#[allow(dead_code)]
const XHAIRCOLORS: u8 = GRAYS;

// =============================================================================
// Key bindings (am_map.c lines 90-102)
// =============================================================================

const AM_PANDOWNKEY: i32 = KEY_DOWNARROW;
const AM_PANUPKEY: i32 = KEY_UPARROW;
const AM_PANRIGHTKEY: i32 = KEY_RIGHTARROW;
const AM_PANLEFTKEY: i32 = KEY_LEFTARROW;
const AM_ZOOMINKEY: i32 = b'=' as i32;
const AM_ZOOMOUTKEY: i32 = b'-' as i32;
const AM_STARTKEY: i32 = KEY_TAB;
const AM_ENDKEY: i32 = KEY_TAB;
const AM_GOBIGKEY: i32 = b'0' as i32;
const AM_FOLLOWKEY: i32 = b'f' as i32;
const AM_GRIDKEY: i32 = b'g' as i32;
const AM_MARKKEY: i32 = b'm' as i32;
const AM_CLEARMARKKEY: i32 = b'c' as i32;

// =============================================================================
// Scale and movement constants (am_map.c lines 107-116)
// =============================================================================

/// Maximum number of mark points the player can place on the automap.
pub const AM_NUMMARKPOINTS: usize = 10;

/// Player collision radius in fixed-point — 16 map units.
/// Original C: `#define PLAYERRADIUS (16*FRACUNIT)` from p_local.h.
const PLAYERRADIUS: i32 = 16 * FRACUNIT;

/// Initial map-to-framebuffer scale factor (20% of full size).
/// Original C: `#define INITSCALEMTOF (.2*FRACUNIT)` = 13107.
const INITSCALEMTOF: i32 = (0.2_f64 * FRACUNIT as f64) as i32;

/// Pan increment in pixels per tic when arrow keys are held.
const F_PANINC: i32 = 4;

/// Zoom-in multiplier per tic (1.02× in fixed-point).
/// Original C: `(int)(1.02*FRACUNIT)` = 66846.
const M_ZOOMIN: i32 = (1.02_f64 * FRACUNIT as f64) as i32;

/// Zoom-out multiplier per tic (1/1.02× in fixed-point).
/// Original C: `(int)(FRACUNIT/1.02)` = 64250.
const M_ZOOMOUT: i32 = (FRACUNIT as f64 / 1.02_f64) as i32;

/// Blockmap grid spacing in map units (128 units per block).
/// Original C: `#define MAPBLOCKUNITS 128` from p_local.h.
const MAPBLOCKUNITS: i32 = 128;

// =============================================================================
// Coordinate transform functions (am_map.c lines 119-123)
// =============================================================================

/// Frame-buffer to Map coordinate conversion.
/// Original C: `#define FTOM(x) FixedMul(((x)<<16),scale_ftom)`
#[inline]
fn ftom(x: i32, scale_ftom: Fixed) -> Fixed {
    Fixed::new(x << FRACBITS).fixed_mul(scale_ftom)
}

/// Map to Frame-buffer coordinate conversion.
/// Original C: `#define MTOF(x) (FixedMul((x),scale_mtof)>>16)`
#[inline]
fn mtof(x: Fixed, scale_mtof: Fixed) -> i32 {
    x.fixed_mul(scale_mtof).0 >> FRACBITS
}

/// Centered X: Map coordinate to frame-buffer X.
/// Original C: `#define CXMTOF(x) (f_x + MTOF((x)-m_x))`
#[inline]
fn cxmtof(x: Fixed, f_x: i32, m_x: Fixed, scale_mtof: Fixed) -> i32 {
    f_x + mtof(x - m_x, scale_mtof)
}

/// Centered Y: Map coordinate to frame-buffer Y (Y-axis inverted).
/// Original C: `#define CYMTOF(y) (f_y + (f_h - MTOF((y)-m_y)))`
#[inline]
fn cymtof(y: Fixed, f_y: i32, f_h: i32, m_y: Fixed, scale_mtof: Fixed) -> i32 {
    f_y + (f_h - mtof(y - m_y, scale_mtof))
}

// =============================================================================
// Internal type definitions (am_map.c lines 128-151)
// =============================================================================

/// Frame-buffer point (integer pixel coordinates).
#[derive(Debug, Clone, Copy, Default)]
struct FPoint {
    x: i32,
    y: i32,
}

/// Frame-buffer line segment.
#[derive(Debug, Clone, Copy, Default)]
struct FLine {
    a: FPoint,
    b: FPoint,
}

/// Map coordinate point (fixed-point).
#[derive(Debug, Clone, Copy)]
pub struct MPoint {
    pub x: Fixed,
    pub y: Fixed,
}

impl Default for MPoint {
    fn default() -> Self {
        MPoint {
            x: Fixed::ZERO,
            y: Fixed::ZERO,
        }
    }
}

/// Map coordinate line segment.
#[derive(Debug, Clone, Copy)]
struct MLine {
    a: MPoint,
    b: MPoint,
}

// =============================================================================
// Vector graphics data (am_map.c lines 160-211)
// =============================================================================

/// Helper to build an MLine from raw fixed-point integer values.
const fn ml(ax: i32, ay: i32, bx: i32, by: i32) -> MLine {
    MLine {
        a: MPoint {
            x: Fixed(ax),
            y: Fixed(ay),
        },
        b: MPoint {
            x: Fixed(bx),
            y: Fixed(by),
        },
    }
}

/// Arrow radius: `R = 8 * PLAYERRADIUS / 7`.
/// Integer division matches C: `8 * 1048576 / 7 = 1198372`.
const AR: i32 = 8 * PLAYERRADIUS / 7;

/// Player arrow — 7 line segments forming the standard automap arrow.
///
/// Original C: `static mline_t player_arrow[]` (am_map.c lines 162-178).
/// Coordinates are expressed in terms of `AR` (arrow radius).
static PLAYER_ARROW: [MLine; 7] = [
    ml(-AR + AR / 8, 0, AR, 0),                       // shaft
    ml(AR, 0, AR - AR / 2, AR / 4),                   // right arrowhead
    ml(AR, 0, AR - AR / 2, -(AR / 4)),                // left arrowhead
    ml(-AR + AR / 8, 0, -(AR) - AR / 8, AR / 4),      // right tail
    ml(-AR + AR / 8, 0, -(AR) - AR / 8, -(AR / 4)),   // left tail
    ml(-AR + 3 * AR / 8, 0, -AR + AR / 8, AR / 4),    // inner right
    ml(-AR + 3 * AR / 8, 0, -AR + AR / 8, -(AR / 4)), // inner left
];

/// Cheat-mode player arrow — 16 line segments showing "ddt" letters.
///
/// Original C: `static mline_t cheat_player_arrow[]` (am_map.c lines 181-209).
static CHEAT_PLAYER_ARROW: [MLine; 16] = [
    ml(-AR + AR / 8, 0, AR, 0),                       // 0: shaft
    ml(AR, 0, AR - AR / 2, AR / 6),                   // 1: head right
    ml(AR, 0, AR - AR / 2, -(AR / 6)),                // 2: head left
    ml(-AR + AR / 8, 0, -(AR) - AR / 8, AR / 4),      // 3: tail right
    ml(-AR + AR / 8, 0, -(AR) - AR / 8, -(AR / 4)),   // 4: tail left
    ml(-AR + 3 * AR / 8, 0, -AR + AR / 8, AR / 4),    // 5: inner right
    ml(-AR + 3 * AR / 8, 0, -AR + AR / 8, -(AR / 4)), // 6: inner left
    // "d" letter
    ml(-(AR / 2), 0, -(AR / 2) - AR / 6, -(AR / 6)), // 7
    ml(-(AR / 2) - AR / 6, -(AR / 6), -(AR / 2) - AR / 6, AR / 4), // 8
    ml(-(AR / 2) - AR / 6, AR / 4, -(AR / 2) + AR / 6, AR / 4), // 9
    ml(-(AR / 2) + AR / 6, AR / 4, -(AR / 2) + AR / 6, -(AR / 6)), // 10
    ml(-(AR / 2) + AR / 6, -(AR / 6), -(AR / 2) - AR / 6, -(AR / 6)), // 11 (close d)
    // "d" stem
    ml(-(AR / 2) - AR / 6, AR / 4, -(AR / 2) - AR / 4, AR / 4), // 12
    ml(-(AR / 2) - AR / 4, AR / 4, -(AR / 2) - AR / 4, -(AR / 6)), // 13
    ml(-(AR / 2) - AR / 4, -(AR / 6), -(AR / 2), -(AR / 6)),    // 14
    // "t" top
    ml(-(AR / 2) + AR / 6, AR / 4, -(AR / 2) + AR / 4, AR / 4), // 15
];

/// Equilateral triangle marker for things — 3 line segments.
///
/// Original C: `static mline_t triangle_guy[]` (am_map.c lines ~212).
/// Uses `R = FRACUNIT` with vertices at approximately (-0.867, -0.5),
/// (0.867, -0.5), and (0, 1.0) in fractional terms.
#[allow(dead_code)]
static TRIANGLE_GUY: [MLine; 3] = [
    ml(
        (-0.867_f64 * FRACUNIT as f64) as i32,
        (-0.5_f64 * FRACUNIT as f64) as i32,
        (0.867_f64 * FRACUNIT as f64) as i32,
        (-0.5_f64 * FRACUNIT as f64) as i32,
    ),
    ml(
        (0.867_f64 * FRACUNIT as f64) as i32,
        (-0.5_f64 * FRACUNIT as f64) as i32,
        0,
        FRACUNIT,
    ),
    ml(
        0,
        FRACUNIT,
        (-0.867_f64 * FRACUNIT as f64) as i32,
        (-0.5_f64 * FRACUNIT as f64) as i32,
    ),
];

/// Thin triangle marker for things (cheat mode) — 3 line segments.
///
/// Original C: `static mline_t thintriangle_guy[]` (am_map.c lines ~220).
/// Uses `R = FRACUNIT` with vertices at (-0.5, -0.7), (1.0, 0), (-0.5, 0.7).
static THINTRIANGLE_GUY: [MLine; 3] = [
    ml(
        (-0.5_f64 * FRACUNIT as f64) as i32,
        (-0.7_f64 * FRACUNIT as f64) as i32,
        FRACUNIT,
        0,
    ),
    ml(
        FRACUNIT,
        0,
        (-0.5_f64 * FRACUNIT as f64) as i32,
        (0.7_f64 * FRACUNIT as f64) as i32,
    ),
    ml(
        (-0.5_f64 * FRACUNIT as f64) as i32,
        (0.7_f64 * FRACUNIT as f64) as i32,
        (-0.5_f64 * FRACUNIT as f64) as i32,
        (-0.7_f64 * FRACUNIT as f64) as i32,
    ),
];

// =============================================================================
// Multiplayer player colors (am_map.c line ~1243)
// =============================================================================

/// Colors assigned to each player in deathmatch/cooperative automap view.
/// Player 0=Green, 1=Gray, 2=Brown, 3=Red.
static PLAYER_COLORS: [u8; 4] = [GREENS, GRAYS, BROWNS, REDS];

// =============================================================================
// AutomapState — all mutable automap state (replaces C static globals)
// =============================================================================

/// Complete automap state, replacing all `static` variables from am_map.c.
///
/// Per AAP §0.7.5, no `static mut` is used — all automap state lives here
/// and is threaded through function calls by reference.
pub struct AutomapState {
    // --- Public state (exposed per schema) ---
    /// Whether the automap overlay is currently active/visible.
    pub automapactive: bool,
    /// Cheat level: 0=normal, 1=show all walls, 2=show all walls+things.
    pub cheating: i32,
    /// Whether the grid overlay is enabled.
    pub grid: bool,
    /// Set true on first tic of a new level, cleared after first AM_Start.
    pub leveljuststarted: bool,
    /// Whether the automap follows the player's position automatically.
    pub followplayer: bool,
    /// Set true when AM_Stop is called externally (before AM_Start re-opens).
    pub stopped: bool,
    /// Number of mark points currently placed (0..AM_NUMMARKPOINTS).
    pub markpointnum: usize,
    /// Last-seen map number for level-change detection.
    pub lastlevel: i32,
    /// Last-seen episode number for level-change detection.
    pub lastepisode: i32,

    // --- Framebuffer window ---
    finit_width: i32,
    finit_height: i32,
    f_x: i32,
    f_y: i32,
    f_w: i32,
    f_h: i32,
    fb_initialized: bool,

    // --- Animation state ---
    amclock: i32,

    // --- Pan and zoom state ---
    m_paninc: MPoint,
    mtof_zoommul: Fixed,
    ftom_zoommul: Fixed,

    // --- Map window bounds (in map coordinates) ---
    m_x: Fixed,
    m_y: Fixed,
    m_x2: Fixed,
    m_y2: Fixed,
    m_w: Fixed,
    m_h: Fixed,

    // --- Map coordinate bounds (from vertex data) ---
    min_x: Fixed,
    min_y: Fixed,
    max_x: Fixed,
    max_y: Fixed,
    max_w: Fixed,
    max_h: Fixed,

    // --- Scale bounds ---
    min_scale_mtof: Fixed,
    max_scale_mtof: Fixed,
    scale_mtof: Fixed,
    scale_ftom: Fixed,

    // --- Saved state for go-big toggle ---
    old_m_x: Fixed,
    old_m_y: Fixed,
    old_m_w: Fixed,
    old_m_h: Fixed,

    // --- Follow-player state ---
    f_oldloc: MPoint,

    // --- Rendering state ---
    lightlev: i32,

    // --- Mark points ---
    markpoints: [MPoint; AM_NUMMARKPOINTS],
    /// Mark number digit patch data (AMMNUM0..9 from WAD).
    pub marknums: [Option<Vec<u8>>; AM_NUMMARKPOINTS],

    // --- Go-big toggle state ---
    bigstate: bool,

    // --- Cheat sequence detector ---
    cheat_amap: CheatSeq,

    // --- Blockmap origin (for grid alignment) ---
    pub bmaporgx: Fixed,
    pub bmaporgy: Fixed,

    // --- Notification event for status bar communication ---
    /// Pending notification event (AM_MSGENTERED or AM_MSGEXITED).
    pub pending_notify: Option<Event>,
}

impl AutomapState {
    /// Create a new AutomapState with default values.
    pub fn new() -> Self {
        let iddt_sequence = vec![
            scramble(b'i'),
            scramble(b'd'),
            scramble(b'd'),
            scramble(b't'),
            0xff,
        ];

        AutomapState {
            automapactive: false,
            cheating: 0,
            grid: false,
            leveljuststarted: true,
            followplayer: true,
            stopped: true,
            markpointnum: 0,
            lastlevel: -1,
            lastepisode: -1,

            finit_width: SCREENWIDTH,
            finit_height: SCREENHEIGHT - 32,
            f_x: 0,
            f_y: 0,
            f_w: SCREENWIDTH,
            f_h: SCREENHEIGHT - 32,
            fb_initialized: false,

            amclock: 0,

            m_paninc: MPoint::default(),
            mtof_zoommul: Fixed::new(FRACUNIT),
            ftom_zoommul: Fixed::new(FRACUNIT),

            m_x: Fixed::ZERO,
            m_y: Fixed::ZERO,
            m_x2: Fixed::ZERO,
            m_y2: Fixed::ZERO,
            m_w: Fixed::ZERO,
            m_h: Fixed::ZERO,

            min_x: Fixed::ZERO,
            min_y: Fixed::ZERO,
            max_x: Fixed::ZERO,
            max_y: Fixed::ZERO,
            max_w: Fixed::ZERO,
            max_h: Fixed::ZERO,

            min_scale_mtof: Fixed::ZERO,
            max_scale_mtof: Fixed::ZERO,
            scale_mtof: Fixed::new(INITSCALEMTOF),
            scale_ftom: Fixed::ZERO,

            old_m_x: Fixed::ZERO,
            old_m_y: Fixed::ZERO,
            old_m_w: Fixed::ZERO,
            old_m_h: Fixed::ZERO,

            f_oldloc: MPoint {
                x: Fixed::new(i32::MAX),
                y: Fixed::ZERO,
            },

            lightlev: 0,

            markpoints: [MPoint::default(); AM_NUMMARKPOINTS],
            marknums: Default::default(),

            bigstate: false,
            cheat_amap: CheatSeq::new(&iddt_sequence),

            bmaporgx: Fixed::ZERO,
            bmaporgy: Fixed::ZERO,

            pending_notify: None,
        }
    }
}

impl Default for AutomapState {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Internal helper functions
// =============================================================================

/// Compute minimum zoom-out scale: shows entire map in the window.
///
/// Original C: `AM_minOutWindowScale` (am_map.c ~line 306).
fn am_min_out_window_scale(state: &AutomapState) -> Fixed {
    let a = Fixed::new(state.f_w << FRACBITS).fixed_div(state.max_w);
    let b = Fixed::new(state.f_h << FRACBITS).fixed_div(state.max_h);
    if a.0 < b.0 {
        a
    } else {
        b
    }
}

/// Compute maximum zoom-in scale.
///
/// Original C: `AM_maxOutWindowScale` (am_map.c ~line 316).
fn am_max_out_window_scale(state: &AutomapState) -> Fixed {
    Fixed::new(state.f_h << FRACBITS).fixed_div(Fixed::new(2 * PLAYERRADIUS))
}

/// Find the map extents from the vertex array and compute scale bounds.
///
/// Original C: `AM_findMinMaxBoundaries` (am_map.c ~line 326).
fn am_find_min_max_boundaries(state: &mut AutomapState, vertexes: &[Vertex]) {
    state.min_x = Fixed::new(i32::MAX);
    state.min_y = Fixed::new(i32::MAX);
    state.max_x = Fixed::new(i32::MIN);
    state.max_y = Fixed::new(i32::MIN);

    for v in vertexes {
        if v.x.0 < state.min_x.0 {
            state.min_x = v.x;
        } else if v.x.0 > state.max_x.0 {
            state.max_x = v.x;
        }
        if v.y.0 < state.min_y.0 {
            state.min_y = v.y;
        } else if v.y.0 > state.max_y.0 {
            state.max_y = v.y;
        }
    }

    state.max_w = state.max_x - state.min_x;
    state.max_h = state.max_y - state.min_y;

    state.min_scale_mtof = am_min_out_window_scale(state);
    state.max_scale_mtof = am_max_out_window_scale(state);
}

/// Save current scale and location for go-big restore.
///
/// Original C: `AM_saveScaleAndLoc` (am_map.c ~line 294).
fn am_save_scale_and_loc(state: &mut AutomapState) {
    state.old_m_x = state.m_x;
    state.old_m_y = state.m_y;
    state.old_m_w = state.m_w;
    state.old_m_h = state.m_h;
}

/// Restore previously saved scale and location after go-big.
///
/// Original C: `AM_restoreScaleAndLoc` (am_map.c ~line 302).
fn am_restore_scale_and_loc(state: &mut AutomapState) {
    state.m_w = state.old_m_w;
    state.m_h = state.old_m_h;

    if !state.followplayer {
        state.m_x = state.old_m_x;
        state.m_y = state.old_m_y;
    }

    state.m_x2 = state.m_x + state.m_w;
    state.m_y2 = state.m_y + state.m_h;

    // Recompute scale from restored window
    state.scale_mtof = Fixed::new(state.f_w << FRACBITS).fixed_div(state.m_w);
    state.scale_ftom = Fixed::new(FRACUNIT).fixed_div(state.scale_mtof);
}

/// Apply the new scale and recompute the map window from the current center.
///
/// Original C: `AM_activateNewScale` (am_map.c ~line 284).
fn am_activate_new_scale(state: &mut AutomapState) {
    state.m_x = state.m_x + Fixed::new(state.m_w.0 / 2)
        - Fixed::new(
            Fixed::new(state.f_w << FRACBITS)
                .fixed_div(state.scale_mtof)
                .0
                / 2,
        );
    state.m_y = state.m_y + Fixed::new(state.m_h.0 / 2)
        - Fixed::new(
            Fixed::new(state.f_h << FRACBITS)
                .fixed_div(state.scale_mtof)
                .0
                / 2,
        );
    state.m_w = ftom(state.f_w, state.scale_ftom);
    state.m_h = ftom(state.f_h, state.scale_ftom);
    state.m_x2 = state.m_x + state.m_w;
    state.m_y2 = state.m_y + state.m_h;
}

/// Add a mark point at the current map center.
///
/// Original C: `AM_addMark` (am_map.c ~line 509).
fn am_add_mark(state: &mut AutomapState) {
    state.markpoints[state.markpointnum] = MPoint {
        x: state.m_x + Fixed::new(state.m_w.0 / 2),
        y: state.m_y + Fixed::new(state.m_h.0 / 2),
    };
    state.markpointnum = (state.markpointnum + 1) % AM_NUMMARKPOINTS;
}

/// Clear all mark points.
///
/// Original C: `AM_clearMarks` (am_map.c ~line 517).
fn am_clear_marks(state: &mut AutomapState) {
    state.markpointnum = 0;
}

/// Apply panning: move the map window by `m_paninc` and clamp to bounds.
///
/// Original C: `AM_changeWindowLoc` (am_map.c ~line 368).
fn am_change_window_loc(state: &mut AutomapState) {
    if state.m_paninc.x.0 != 0 || state.m_paninc.y.0 != 0 {
        state.followplayer = false;
        state.f_oldloc.x = Fixed::new(i32::MAX);
    }

    state.m_x = state.m_x + state.m_paninc.x;
    state.m_y = state.m_y + state.m_paninc.y;

    // Clamp X
    if state.m_x.0 + state.m_w.0 > state.max_x.0 {
        state.m_x = state.max_x - state.m_w;
    } else if state.m_x.0 < state.min_x.0 {
        state.m_x = state.min_x;
    }

    // Clamp Y
    if state.m_y.0 + state.m_h.0 > state.max_y.0 {
        state.m_y = state.max_y - state.m_h;
    } else if state.m_y.0 < state.min_y.0 {
        state.m_y = state.min_y;
    }

    state.m_x2 = state.m_x + state.m_w;
    state.m_y2 = state.m_y + state.m_h;
}

/// Zoom out to show the entire map.
///
/// Original C: `AM_minOutWindowScale` applied as max zoom-out (am_map.c ~line 310).
fn am_min_out_window_scale_apply(state: &mut AutomapState) {
    state.scale_mtof = state.min_scale_mtof;
    state.scale_ftom = Fixed::new(FRACUNIT).fixed_div(state.scale_mtof);
    am_activate_new_scale(state);
}

/// Apply zoom multipliers per tic.
///
/// Original C: `AM_changeWindowScale` (am_map.c ~line 443).
fn am_change_window_scale(state: &mut AutomapState) {
    state.scale_mtof = state.scale_mtof.fixed_mul(state.mtof_zoommul);
    state.scale_ftom = Fixed::new(FRACUNIT).fixed_div(state.scale_mtof);

    // Clamp to bounds
    if state.scale_mtof.0 < state.min_scale_mtof.0 {
        am_min_out_window_scale_apply(state);
    } else if state.scale_mtof.0 > state.max_scale_mtof.0 {
        state.scale_mtof = state.max_scale_mtof;
        state.scale_ftom = Fixed::new(FRACUNIT).fixed_div(state.scale_mtof);
    }
}

/// Update automap to follow the player position.
///
/// Original C: `AM_doFollowPlayer` (am_map.c ~line 457).
fn am_do_follow_player(state: &mut AutomapState, player_x: Fixed, player_y: Fixed) {
    if state.f_oldloc.x != player_x || state.f_oldloc.y != player_y {
        // Snap to grid for consistent pixel rounding
        state.m_x = Fixed::new(mtof(player_x, state.scale_mtof));
        state.m_x = ftom(state.m_x.0, state.scale_ftom) - Fixed::new(state.m_w.0 / 2);
        state.m_y = Fixed::new(mtof(player_y, state.scale_mtof));
        state.m_y = ftom(state.m_y.0, state.scale_ftom) - Fixed::new(state.m_h.0 / 2);
        state.m_x2 = state.m_x + state.m_w;
        state.m_y2 = state.m_y + state.m_h;
        state.f_oldloc.x = player_x;
        state.f_oldloc.y = player_y;
    }
}

/// Initialize automap variables when opened.
///
/// Sets the map window centered on the player and sends AM_MSGENTERED.
///
/// Original C: `AM_initVariables` (am_map.c ~line 523).
fn am_init_variables(
    state: &mut AutomapState,
    players: &[Player],
    consoleplayer: usize,
    mobjs: &[MapObject],
) {
    state.automapactive = true;

    // Initialize framebuffer window dimensions (first time only)
    if !state.fb_initialized {
        state.fb_initialized = true;
        state.f_x = 0;
        state.f_y = 0;
        state.f_w = state.finit_width;
        state.f_h = state.finit_height;
    }

    state.f_oldloc.x = Fixed::new(i32::MAX);
    state.amclock = 0;
    state.lightlev = 0;

    state.m_paninc = MPoint::default();
    state.ftom_zoommul = Fixed::new(FRACUNIT);
    state.mtof_zoommul = Fixed::new(FRACUNIT);

    state.m_w = ftom(state.f_w, state.scale_ftom);
    state.m_h = ftom(state.f_h, state.scale_ftom);

    // Center on player position
    if let Some(mobj_idx) = players[consoleplayer].mobj {
        let mo = &mobjs[mobj_idx];
        state.m_x = mo.x - Fixed::new(state.m_w.0 / 2);
        state.m_y = mo.y - Fixed::new(state.m_h.0 / 2);
    }

    am_change_window_loc(state);

    state.m_x2 = state.m_x + state.m_w;
    state.m_y2 = state.m_y + state.m_h;

    // Send AM_MSGENTERED notification to status bar
    state.pending_notify = Some(Event {
        event_type: EventType::KeyUp,
        data1: AM_MSGENTERED,
        data2: 0,
        data3: 0,
    });
}

/// Per-level initialization: find map bounds and set initial scale.
///
/// Original C: `AM_LevelInit` (am_map.c ~line 469).
fn am_level_init(state: &mut AutomapState, vertexes: &[Vertex]) {
    state.leveljuststarted = true;
    state.f_x = 0;
    state.f_y = 0;
    state.f_w = state.finit_width;
    state.f_h = state.finit_height;

    am_find_min_max_boundaries(state, vertexes);

    // Set initial scale to ~1.4× the minimum (showing most of the map)
    // Original C: `scale_mtof = FixedDiv(a_to_f, (int)(0.7*FRACUNIT))`
    state.scale_mtof = state
        .min_scale_mtof
        .fixed_div(Fixed::new((0.7_f64 * FRACUNIT as f64) as i32));

    if state.scale_mtof.0 > state.max_scale_mtof.0 {
        state.scale_mtof = state.min_scale_mtof;
    }

    state.scale_ftom = Fixed::new(FRACUNIT).fixed_div(state.scale_mtof);
}

/// Open the automap (AM_Start equivalent).
///
/// Handles level-change detection, calls am_level_init if needed,
/// initializes variables, and loads mark number patches.
///
/// Original C: `AM_Start` (am_map.c ~line 559).
fn am_start(
    state: &mut AutomapState,
    players: &[Player],
    consoleplayer: usize,
    mobjs: &[MapObject],
    vertexes: &[Vertex],
    gameepisode: i32,
    gamemap: i32,
) {
    if !state.stopped {
        am_stop_internal(state);
    }
    state.stopped = false;

    if state.lastlevel != gamemap || state.lastepisode != gameepisode {
        am_level_init(state, vertexes);
        state.lastlevel = gamemap;
        state.lastepisode = gameepisode;
    }

    am_init_variables(state, players, consoleplayer, mobjs);
    // Note: AM_loadPics (loading AMMNUM patches) is handled externally
    // by the game code setting state.marknums before drawing.
}

// =============================================================================
// Public API functions
// =============================================================================

/// Handle an input event for the automap.
///
/// Processes TAB to toggle automap, arrow keys for pan, +/- for zoom,
/// F for follow, G for grid, M for mark, C for clear marks, 0 for go-big,
/// and the IDDT cheat sequence.
///
/// Returns `true` if the event was consumed by the automap and should not
/// be passed to other input handlers.
///
/// Original C: `AM_Responder` (am_map.c lines 613-735).
pub fn am_responder(
    state: &mut AutomapState,
    ev: &Event,
    players: &mut [Player],
    consoleplayer: usize,
    mobjs: &[MapObject],
    vertexes: &[Vertex],
    gameepisode: i32,
    gamemap: i32,
    deathmatch: bool,
) -> bool {
    let mut rc = false;

    match ev.event_type {
        EventType::KeyDown => {
            if !state.automapactive {
                // Automap is closed — only TAB opens it
                if ev.data1 == AM_STARTKEY {
                    am_start(
                        state,
                        players,
                        consoleplayer,
                        mobjs,
                        vertexes,
                        gameepisode,
                        gamemap,
                    );
                    // viewactive = false; // Caller is responsible for this
                    rc = true;
                }
            } else {
                // Automap is open — handle all automap keys
                rc = true;
                match ev.data1 {
                    x if x == AM_PANRIGHTKEY => {
                        if !state.followplayer {
                            state.m_paninc.x = Fixed::new(ftom(F_PANINC, state.scale_ftom).0);
                        } else {
                            rc = false;
                        }
                    }
                    x if x == AM_PANLEFTKEY => {
                        if !state.followplayer {
                            state.m_paninc.x = Fixed::new(-ftom(F_PANINC, state.scale_ftom).0);
                        } else {
                            rc = false;
                        }
                    }
                    x if x == AM_PANUPKEY => {
                        if !state.followplayer {
                            state.m_paninc.y = ftom(F_PANINC, state.scale_ftom);
                        } else {
                            rc = false;
                        }
                    }
                    x if x == AM_PANDOWNKEY => {
                        if !state.followplayer {
                            state.m_paninc.y = Fixed::new(-ftom(F_PANINC, state.scale_ftom).0);
                        } else {
                            rc = false;
                        }
                    }
                    x if x == AM_ZOOMOUTKEY => {
                        state.mtof_zoommul = Fixed::new(M_ZOOMOUT);
                        state.ftom_zoommul = Fixed::new(M_ZOOMIN);
                    }
                    x if x == AM_ZOOMINKEY => {
                        state.mtof_zoommul = Fixed::new(M_ZOOMIN);
                        state.ftom_zoommul = Fixed::new(M_ZOOMOUT);
                    }
                    x if x == AM_ENDKEY => {
                        state.bigstate = false;
                        // viewactive = true; // Caller is responsible for this
                        am_stop_internal(state);
                    }
                    x if x == AM_GOBIGKEY => {
                        state.bigstate = !state.bigstate;
                        if state.bigstate {
                            am_save_scale_and_loc(state);
                            am_min_out_window_scale_apply(state);
                        } else {
                            am_restore_scale_and_loc(state);
                        }
                    }
                    x if x == AM_FOLLOWKEY => {
                        state.followplayer = !state.followplayer;
                        state.f_oldloc.x = Fixed::new(i32::MAX);
                        players[consoleplayer].message = if state.followplayer {
                            Some(AMSTR_FOLLOWON.to_string())
                        } else {
                            Some(AMSTR_FOLLOWOFF.to_string())
                        };
                    }
                    x if x == AM_GRIDKEY => {
                        state.grid = !state.grid;
                        players[consoleplayer].message = if state.grid {
                            Some(AMSTR_GRIDON.to_string())
                        } else {
                            Some(AMSTR_GRIDOFF.to_string())
                        };
                    }
                    x if x == AM_MARKKEY => {
                        am_add_mark(state);
                        players[consoleplayer].message = Some(AMSTR_MARKEDSPOT.to_string());
                    }
                    x if x == AM_CLEARMARKKEY => {
                        am_clear_marks(state);
                        players[consoleplayer].message = Some(AMSTR_MARKSCLEARED.to_string());
                    }
                    _ => {
                        rc = false;
                    }
                }
            }

            // Cheat check: IDDT sequence (happens on every keydown)
            if !deathmatch && state.cheat_amap.check_cheat(ev.data1 as u8) {
                rc = false;
                state.cheating = (state.cheating + 1) % 3;
            }
        }

        EventType::KeyUp => match ev.data1 {
            x if x == AM_PANRIGHTKEY => {
                if !state.followplayer {
                    state.m_paninc.x = Fixed::ZERO;
                }
            }
            x if x == AM_PANLEFTKEY => {
                if !state.followplayer {
                    state.m_paninc.x = Fixed::ZERO;
                }
            }
            x if x == AM_PANUPKEY => {
                if !state.followplayer {
                    state.m_paninc.y = Fixed::ZERO;
                }
            }
            x if x == AM_PANDOWNKEY => {
                if !state.followplayer {
                    state.m_paninc.y = Fixed::ZERO;
                }
            }
            x if x == AM_ZOOMOUTKEY || x == AM_ZOOMINKEY => {
                state.mtof_zoommul = Fixed::new(FRACUNIT);
                state.ftom_zoommul = Fixed::new(FRACUNIT);
            }
            _ => {}
        },

        _ => {}
    }

    rc
}

/// Update automap state per game tic.
///
/// Advances the automap clock, applies follow-player centering, zoom
/// scale changes, and panning increments.
///
/// Original C: `AM_Ticker` (am_map.c lines 805-827).
pub fn am_ticker(
    state: &mut AutomapState,
    players: &[Player],
    consoleplayer: usize,
    mobjs: &[MapObject],
) {
    if !state.automapactive {
        return;
    }

    state.amclock += 1;

    // Follow player position
    if state.followplayer {
        if let Some(mobj_idx) = players[consoleplayer].mobj {
            let mo = &mobjs[mobj_idx];
            am_do_follow_player(state, mo.x, mo.y);
        }
    }

    // Apply zoom (only if zoom keys are held — zoommul != FRACUNIT)
    if state.mtof_zoommul.0 != FRACUNIT {
        am_change_window_scale(state);
    }

    // Apply panning (only if pan keys are held)
    if state.m_paninc.x.0 != 0 || state.m_paninc.y.0 != 0 {
        am_change_window_loc(state);
    }
}

// =============================================================================
// Drawing functions
// =============================================================================

// Cohen-Sutherland clipping region codes
const CS_LEFT: i32 = 1;
const CS_RIGHT: i32 = 2;
const CS_BOTTOM: i32 = 4;
const CS_TOP: i32 = 8;

/// Compute Cohen-Sutherland outcode for a framebuffer point.
#[inline]
fn do_outcode(x: i32, y: i32, f_w: i32, f_h: i32) -> i32 {
    let mut oc = 0;
    if y < 0 {
        oc |= CS_TOP;
    } else if y >= f_h {
        oc |= CS_BOTTOM;
    }
    if x < 0 {
        oc |= CS_LEFT;
    } else if x >= f_w {
        oc |= CS_RIGHT;
    }
    oc
}

/// Clear the automap framebuffer region to a solid color.
///
/// Original C: `AM_clearFB` (am_map.c ~line 832).
fn am_clear_fb(state: &AutomapState, video: &mut VideoState, color: u8) {
    let fb = &mut video.screens[0];
    let start = (state.f_y * state.f_w + state.f_x) as usize;
    let total = (state.f_h * state.f_w) as usize;
    if start + total <= fb.len() {
        for pixel in fb[start..start + total].iter_mut() {
            *pixel = color;
        }
    }
}

/// Cohen-Sutherland line clipping: clip a map-coordinate line to the
/// framebuffer window.
///
/// Returns `true` if the line is visible (and `fl` is set to the clipped
/// framebuffer-coordinate line), or `false` if the line is entirely outside.
///
/// Original C: `AM_clipMline` (am_map.c lines 846-971).
fn am_clip_mline(state: &AutomapState, ml: &MLine, fl: &mut FLine) -> bool {
    // Trivial reject in map coordinates
    if (ml.a.x.0 < state.m_x.0 && ml.b.x.0 < state.m_x.0)
        || (ml.a.x.0 > state.m_x2.0 && ml.b.x.0 > state.m_x2.0)
        || (ml.a.y.0 < state.m_y.0 && ml.b.y.0 < state.m_y.0)
        || (ml.a.y.0 > state.m_y2.0 && ml.b.y.0 > state.m_y2.0)
    {
        return false;
    }

    // Transform to framebuffer coordinates
    fl.a.x = cxmtof(ml.a.x, state.f_x, state.m_x, state.scale_mtof);
    fl.a.y = cymtof(ml.a.y, state.f_y, state.f_h, state.m_y, state.scale_mtof);
    fl.b.x = cxmtof(ml.b.x, state.f_x, state.m_x, state.scale_mtof);
    fl.b.y = cymtof(ml.b.y, state.f_y, state.f_h, state.m_y, state.scale_mtof);

    let mut outcode1 = do_outcode(fl.a.x, fl.a.y, state.f_w, state.f_h);
    let mut outcode2 = do_outcode(fl.b.x, fl.b.y, state.f_w, state.f_h);

    // Cohen-Sutherland clipping loop
    loop {
        if (outcode1 | outcode2) == 0 {
            // Both endpoints inside — accept
            return true;
        }
        if (outcode1 & outcode2) != 0 {
            // Both endpoints on same outside region — reject
            return false;
        }

        // Pick the outside endpoint
        let outside = if outcode1 != 0 { outcode1 } else { outcode2 };
        let mut tmp = FPoint { x: 0, y: 0 };

        if (outside & CS_TOP) != 0 {
            let dy = fl.a.y - fl.b.y;
            let dx = fl.b.x - fl.a.x;
            if dy != 0 {
                tmp.x = fl.a.x + (dx * fl.a.y) / dy;
            }
            tmp.y = 0;
        } else if (outside & CS_BOTTOM) != 0 {
            let dy = fl.a.y - fl.b.y;
            let dx = fl.b.x - fl.a.x;
            if dy != 0 {
                tmp.x = fl.a.x + (dx * (fl.a.y - (state.f_h - 1))) / dy;
            }
            tmp.y = state.f_h - 1;
        } else if (outside & CS_RIGHT) != 0 {
            let dy = fl.b.y - fl.a.y;
            let dx = fl.b.x - fl.a.x;
            if dx != 0 {
                tmp.y = fl.a.y + (dy * (state.f_w - 1 - fl.a.x)) / dx;
            }
            tmp.x = state.f_w - 1;
        } else if (outside & CS_LEFT) != 0 {
            let dy = fl.b.y - fl.a.y;
            let dx = fl.b.x - fl.a.x;
            if dx != 0 {
                tmp.y = fl.a.y + (dy * (-fl.a.x)) / dx;
            }
            tmp.x = 0;
        }

        if outside == outcode1 {
            fl.a = tmp;
            outcode1 = do_outcode(fl.a.x, fl.a.y, state.f_w, state.f_h);
        } else {
            fl.b = tmp;
            outcode2 = do_outcode(fl.b.x, fl.b.y, state.f_w, state.f_h);
        }
    }
}

/// Draw a framebuffer-coordinate line using Bresenham's algorithm.
///
/// The line endpoints must be within the framebuffer bounds (call
/// `am_clip_mline` first). Writes directly to `video.screens[0]`.
///
/// Original C: `AM_drawFline` (am_map.c lines 977-1049).
fn am_draw_fline(state: &AutomapState, video: &mut VideoState, fl: &FLine, color: u8) {
    let f_w = state.f_w;

    // Bounds check — skip lines outside the framebuffer
    if fl.a.x < 0
        || fl.a.x >= state.f_w
        || fl.a.y < 0
        || fl.a.y >= state.f_h
        || fl.b.x < 0
        || fl.b.x >= state.f_w
        || fl.b.y < 0
        || fl.b.y >= state.f_h
    {
        return;
    }

    let dx = fl.b.x - fl.a.x;
    let ax = 2 * dx.abs();
    let sx: i32 = if dx < 0 { -1 } else { 1 };

    let dy = fl.b.y - fl.a.y;
    let ay = 2 * dy.abs();
    let sy: i32 = if dy < 0 { -1 } else { 1 };

    let mut x = fl.a.x;
    let mut y = fl.a.y;

    let fb = &mut video.screens[0];

    if ax > ay {
        // X-major line
        let mut d = ay - ax / 2;
        loop {
            let idx = (y * f_w + x) as usize;
            if idx < fb.len() {
                fb[idx] = color;
            }
            if x == fl.b.x {
                return;
            }
            if d >= 0 {
                y += sy;
                d -= ax;
            }
            x += sx;
            d += ay;
        }
    } else {
        // Y-major line
        let mut d = ax - ay / 2;
        loop {
            let idx = (y * f_w + x) as usize;
            if idx < fb.len() {
                fb[idx] = color;
            }
            if y == fl.b.y {
                return;
            }
            if d >= 0 {
                x += sx;
                d -= ay;
            }
            y += sy;
            d += ax;
        }
    }
}

/// Draw a map-coordinate line: clip and then render with Bresenham.
///
/// Original C: `AM_drawMline` (am_map.c ~line 1055).
fn am_draw_mline(state: &AutomapState, video: &mut VideoState, ml: &MLine, color: u8) {
    let mut fl = FLine::default();
    if am_clip_mline(state, ml, &mut fl) {
        am_draw_fline(state, video, &fl, color);
    }
}

/// Draw the grid overlay — horizontal and vertical lines at MAPBLOCKUNITS
/// intervals aligned to the blockmap origin.
///
/// Original C: `AM_drawGrid` (am_map.c lines 1071-1111).
fn am_draw_grid(state: &AutomapState, video: &mut VideoState, color: u8) {
    let block_size = Fixed::new(MAPBLOCKUNITS << FRACBITS);

    // Draw horizontal lines (varying y)
    let y_offset = (state.m_y - state.bmaporgy).0 % block_size.0;
    let mut start_y = state.m_y.0 - y_offset;
    let end_y = state.m_y.0 + state.m_h.0;

    while start_y < end_y {
        let ml = MLine {
            a: MPoint {
                x: state.m_x,
                y: Fixed::new(start_y),
            },
            b: MPoint {
                x: Fixed::new(state.m_x.0 + state.m_w.0),
                y: Fixed::new(start_y),
            },
        };
        am_draw_mline(state, video, &ml, color);
        start_y += block_size.0;
    }

    // Draw vertical lines (varying x)
    let x_offset = (state.m_x - state.bmaporgx).0 % block_size.0;
    let mut start_x = state.m_x.0 - x_offset;
    let end_x = state.m_x.0 + state.m_w.0;

    while start_x < end_x {
        let ml = MLine {
            a: MPoint {
                x: Fixed::new(start_x),
                y: state.m_y,
            },
            b: MPoint {
                x: Fixed::new(start_x),
                y: Fixed::new(state.m_y.0 + state.m_h.0),
            },
        };
        am_draw_mline(state, video, &ml, color);
        start_x += block_size.0;
    }
}

/// Draw all wall segments on the automap, colored by type.
///
/// Color scheme:
/// - One-sided walls: WALLCOLORS (red)
/// - Teleporter lines (special==39): mid-red
/// - Secret walls: SECRETWALLCOLORS (cheat) or WALLCOLORS (normal)
/// - Floor height difference: FDWALLCOLORS (brown)
/// - Ceiling height difference: CDWALLCOLORS (yellow)
/// - Same-height two-sided: TSWALLCOLORS (gray, cheat only)
/// - Unmapped + allmap powerup: dim gray
///
/// Original C: `AM_drawWalls` (am_map.c lines 1117-1165).
fn am_draw_walls(
    state: &AutomapState,
    video: &mut VideoState,
    lines: &[LineDef],
    vertexes: &[Vertex],
    sectors: &[Sector],
    players: &[Player],
    consoleplayer: usize,
) {
    let lightlev = state.lightlev as u8;
    let plr = &players[consoleplayer];
    let has_allmap = plr.powers[PowerType::AllMap as usize] > 0;

    for line in lines {
        let v1 = &vertexes[line.v1];
        let v2 = &vertexes[line.v2];
        let ml = MLine {
            a: MPoint { x: v1.x, y: v1.y },
            b: MPoint { x: v2.x, y: v2.y },
        };

        let flags = line.flags;
        let is_mapped = (flags & LineFlags::ML_MAPPED.bits()) != 0;
        let is_dontdraw = (flags & LineFlags::ML_DONTDRAW.bits()) != 0;
        let is_secret = (flags & LineFlags::ML_SECRET.bits()) != 0;

        if state.cheating > 0 || is_mapped {
            // Line is revealed (cheat or mapped)
            if is_dontdraw && state.cheating == 0 {
                continue; // Never-see line in normal mode
            }

            if let Some(back_idx) = line.backsector {
                // Two-sided line
                if line.special == 39 {
                    // Teleporter
                    am_draw_mline(state, video, &ml, WALLCOLORS.wrapping_add(REDRANGE / 2));
                } else if is_secret {
                    // Secret wall
                    if state.cheating > 0 {
                        am_draw_mline(state, video, &ml, SECRETWALLCOLORS.wrapping_add(lightlev));
                    } else {
                        am_draw_mline(state, video, &ml, WALLCOLORS.wrapping_add(lightlev));
                    }
                } else if let Some(fidx) = line.frontsector {
                    let front = &sectors[fidx];
                    let back = &sectors[back_idx];

                    if back.floorheight != front.floorheight {
                        // Floor height difference
                        am_draw_mline(state, video, &ml, FDWALLCOLORS.wrapping_add(lightlev));
                    } else if back.ceilingheight != front.ceilingheight {
                        // Ceiling height difference
                        am_draw_mline(state, video, &ml, CDWALLCOLORS.wrapping_add(lightlev));
                    } else if state.cheating > 0 {
                        // Same-height two-sided line (cheat only)
                        am_draw_mline(state, video, &ml, TSWALLCOLORS.wrapping_add(lightlev));
                    }
                }
            } else {
                // One-sided line (solid wall)
                am_draw_mline(state, video, &ml, WALLCOLORS.wrapping_add(lightlev));
            }
        } else if has_allmap {
            // Not yet mapped but player has computer area map powerup
            if !is_dontdraw {
                am_draw_mline(state, video, &ml, GRAYS + 3);
            }
        }
    }
}

/// Rotate a point (x, y) around the origin by angle `a`.
///
/// Uses trigonometric lookup tables for fixed-point sine/cosine.
/// x' = x*cos(a) - y*sin(a)
/// y' = x*sin(a) + y*cos(a)
///
/// Original C: `AM_rotate` (am_map.c lines 1172-1189).
fn am_rotate(x: &mut Fixed, y: &mut Fixed, a: Angle) {
    let fine = a.to_fine_angle();
    let cos_val = finecosine(fine);
    let sin_val = FINESINE[fine];
    let tmpx = x.fixed_mul(cos_val) - y.fixed_mul(sin_val);
    *y = x.fixed_mul(sin_val) + y.fixed_mul(cos_val);
    *x = tmpx;
}

/// Draw a vector shape (array of map-coordinate lines) at a given position,
/// with scaling and rotation.
///
/// Used to draw the player arrow and thing triangles at entity positions.
///
/// Original C: `AM_drawLineCharacter` (am_map.c lines 1191-1238).
fn am_draw_line_character(
    state: &AutomapState,
    video: &mut VideoState,
    lines: &[MLine],
    scale: Fixed,
    angle: Angle,
    color: u8,
    x: Fixed,
    y: Fixed,
) {
    for line in lines {
        let mut l = *line;

        // Scale
        if scale.0 != 0 {
            l.a.x = l.a.x.fixed_mul(scale);
            l.a.y = l.a.y.fixed_mul(scale);
            l.b.x = l.b.x.fixed_mul(scale);
            l.b.y = l.b.y.fixed_mul(scale);
        }

        // Rotate
        if angle.value() != 0 {
            am_rotate(&mut l.a.x, &mut l.a.y, angle);
            am_rotate(&mut l.b.x, &mut l.b.y, angle);
        }

        // Translate to position
        l.a.x = l.a.x + x;
        l.a.y = l.a.y + y;
        l.b.x = l.b.x + x;
        l.b.y = l.b.y + y;

        am_draw_mline(state, video, &l, color);
    }
}

/// Draw all player arrows on the automap.
///
/// In single-player: draws the local player's arrow in white.
/// In deathmatch: draws each in-game player with a unique color.
///
/// Original C: `AM_drawPlayers` (am_map.c lines 1240-1282).
fn am_draw_players(
    state: &AutomapState,
    video: &mut VideoState,
    players: &[Player],
    consoleplayer: usize,
    playeringame: &[bool],
    mobjs: &[MapObject],
    deathmatch: bool,
) {
    let arrow = if state.cheating > 0 {
        &CHEAT_PLAYER_ARROW[..]
    } else {
        &PLAYER_ARROW[..]
    };

    if !deathmatch {
        // Single player — draw local player arrow
        if let Some(mobj_idx) = players[consoleplayer].mobj {
            let mo = &mobjs[mobj_idx];
            am_draw_line_character(
                state,
                video,
                arrow,
                Fixed::ZERO,
                mo.angle,
                YOURCOLORS,
                mo.x,
                mo.y,
            );
        }
    } else {
        // Deathmatch — draw all in-game players
        let max = playeringame.len().min(MAXPLAYERS);
        for i in 0..max {
            if !playeringame[i] {
                continue;
            }
            if let Some(mobj_idx) = players[i].mobj {
                let mo = &mobjs[mobj_idx];
                let color = if players[i].powers[PowerType::Invisibility as usize] > 0 {
                    246_u8 // Near-black for invisible players
                } else {
                    PLAYER_COLORS[i % PLAYER_COLORS.len()]
                };
                am_draw_line_character(
                    state,
                    video,
                    arrow,
                    Fixed::ZERO,
                    mo.angle,
                    color,
                    mo.x,
                    mo.y,
                );
            }
        }
    }
}

/// Draw thing triangles on the automap (cheat mode 2 only).
///
/// Iterates all sectors' thing lists and draws thin triangles at each
/// thing's position and orientation.
///
/// Original C: `AM_drawThings` (am_map.c lines 1284-1303).
fn am_draw_things(
    state: &AutomapState,
    video: &mut VideoState,
    sectors: &[Sector],
    mobjs: &[MapObject],
    color: u8,
    _colorrange: i32,
) {
    for sector in sectors {
        let mut thing_idx = sector.thinglist;
        while let Some(idx) = thing_idx {
            if idx >= mobjs.len() {
                break; // Safety: prevent out-of-bounds access
            }
            let thing = &mobjs[idx];
            am_draw_line_character(
                state,
                video,
                &THINTRIANGLE_GUY,
                Fixed::new(16 << FRACBITS),
                thing.angle,
                color,
                thing.x,
                thing.y,
            );
            thing_idx = thing.snext;
        }
    }
}

/// Draw mark point digits on the automap.
///
/// Each mark is rendered using the AMMNUM digit patches at the mark's
/// map-to-framebuffer-transformed position.
///
/// Original C: `AM_drawMarks` (am_map.c lines 1305-1324).
fn am_draw_marks(state: &AutomapState, video: &mut VideoState) {
    let mark_w: i32 = 5; // Width of mark digit patches
    let mark_h: i32 = 6; // Height of mark digit patches

    for i in 0..AM_NUMMARKPOINTS {
        if state.markpoints[i].x.0 == Fixed::ZERO.0
            && state.markpoints[i].y.0 == Fixed::ZERO.0
            && i >= state.markpointnum
        {
            continue;
        }

        // Check if this mark point index has been placed
        // (all mark points up to markpointnum-1 are valid, wrapping around)
        // The original C just checks if x != f_oldloc.x as sentinel
        let fx = cxmtof(
            state.markpoints[i].x,
            state.f_x,
            state.m_x,
            state.scale_mtof,
        ) - mark_w / 2;
        let fy = cymtof(
            state.markpoints[i].y,
            state.f_y,
            state.f_h,
            state.m_y,
            state.scale_mtof,
        ) - mark_h / 2;

        // Extract individual digits of the mark number
        let mut num = i;
        let mut digits = Vec::new();
        if num == 0 {
            digits.push(0);
        } else {
            while num > 0 {
                digits.push(num % 10);
                num /= 10;
            }
            digits.reverse();
        }

        let mut draw_x = fx;
        for &digit in &digits {
            if digit < state.marknums.len() {
                if let Some(ref patch_data) = state.marknums[digit] {
                    video.draw_patch(draw_x, fy, 0, patch_data);
                }
            }
            draw_x += mark_w;
        }
    }
}

/// Draw the crosshair at the center of the automap framebuffer.
///
/// Original C: `AM_drawCrosshair` (am_map.c lines 1326-1330).
fn am_draw_crosshair(state: &AutomapState, video: &mut VideoState, color: u8) {
    let fb = &mut video.screens[0];
    let idx = ((state.f_w * (state.f_h + 1)) / 2) as usize;
    if idx < fb.len() {
        fb[idx] = color;
    }
}

// =============================================================================
// Main drawer and stop functions
// =============================================================================

/// Render the complete automap overlay to the framebuffer.
///
/// Drawing order:
/// 1. Clear framebuffer to background color
/// 2. Draw grid lines (if grid mode enabled)
/// 3. Draw wall segments (colored by type)
/// 4. Draw player arrows
/// 5. Draw thing triangles (if cheating level 2)
/// 6. Draw crosshair
/// 7. Draw mark point digits
/// 8. Mark the dirty screen region for refresh
///
/// Original C: `AM_Drawer` (am_map.c lines 1332-1349).
pub fn am_drawer(
    state: &AutomapState,
    video: &mut VideoState,
    vertexes: &[Vertex],
    lines: &[LineDef],
    sectors: &[Sector],
    mobjs: &[MapObject],
    players: &[Player],
    consoleplayer: usize,
    playeringame: &[bool],
    deathmatch: bool,
) {
    if !state.automapactive {
        return;
    }

    // 1. Clear background
    am_clear_fb(state, video, BACKGROUND);

    // 2. Grid overlay
    if state.grid {
        am_draw_grid(state, video, GRIDCOLORS);
    }

    // 3. Wall segments
    am_draw_walls(
        state,
        video,
        lines,
        vertexes,
        sectors,
        players,
        consoleplayer,
    );

    // 4. Player arrows
    am_draw_players(
        state,
        video,
        players,
        consoleplayer,
        playeringame,
        mobjs,
        deathmatch,
    );

    // 5. Thing triangles (cheat mode 2)
    if state.cheating == 2 {
        am_draw_things(state, video, sectors, mobjs, THINGCOLORS, 0);
    }

    // 6. Crosshair
    am_draw_crosshair(state, video, XHAIRCOLORS);

    // 7. Mark digits
    am_draw_marks(state, video);

    // 8. Mark the automap framebuffer region as dirty for screen refresh
    video.mark_rect(state.f_x, state.f_y, state.f_w, state.f_h);
}

/// Internal stop — closes the automap and sends AM_MSGEXITED notification.
///
/// Called from both `am_stop` (public) and `am_responder` (TAB key toggle).
fn am_stop_internal(state: &mut AutomapState) {
    state.automapactive = false;
    state.stopped = true;

    // Clear mark number patches
    for marknum in state.marknums.iter_mut() {
        *marknum = None;
    }

    // Send AM_MSGEXITED notification to status bar
    state.pending_notify = Some(Event {
        event_type: EventType::KeyUp,
        data1: AM_MSGEXITED,
        data2: 0,
        data3: 0,
    });
}

/// Force the automap closed.
///
/// Called externally by the game when the level ends or automap must be
/// forcibly dismissed. Sends an AM_MSGEXITED notification event via
/// `state.pending_notify`.
///
/// Original C: `AM_Stop` (am_map.c ~line 549).
pub fn am_stop(state: &mut AutomapState) {
    am_stop_internal(state);
}
