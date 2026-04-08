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

//! Switch texture change logic and P_UseSpecialLine dispatcher.
//!
//! Translated from linuxdoom-1.10/p_switch.c
//!
//! This module implements switch/button activation: swapping wall textures to
//! indicate activation and timing buttons that automatically reset after a delay.
//! It also implements `P_UseSpecialLine`, the massive dispatcher for all player
//! "use" actions on linedefs (doors, switches, stairs, lifts, etc.).
//!
//! # Original C functions translated
//!
//! | Rust function              | C function             | Description                              |
//! |----------------------------|------------------------|------------------------------------------|
//! | `p_init_switch_list`       | `P_InitSwitchList`     | Initialize switch texture pair table      |
//! | `p_start_button`           | `P_StartButton`        | Start a timed button reset (internal)     |
//! | `p_change_switch_texture`  | `P_ChangeSwitchTexture`| Swap switch texture + optional timer      |
//! | `p_use_special_line`       | `P_UseSpecialLine`     | Dispatcher for line "use" actions         |

use crate::info::sounds::SfxEnum;
use crate::play::doors::{ev_do_locked_door, ev_vertical_door};
use crate::play::spec::{
    ev_do_donut, BWhere, CeilingType, FloorType, PlatType, SpecContext, StairType, VldoorType,
    BUTTONTIME, MAXBUTTONS, MAXSWITCHES,
};
use crate::types::doomdef::GameMode;
use crate::types::map_data::LineFlags;

// =============================================================================
// Static switch texture pair table (p_switch.c lines 48-97)
// =============================================================================

/// A single entry in the built-in switch texture pair table.
///
/// Each entry maps two texture names (off/on pair) and the minimum episode
/// tier required for the pair to be active.  Episode 1 = shareware,
/// 2 = registered, 3 = commercial (DOOM II).
struct AlphSwitchEntry {
    name1: &'static str,
    name2: &'static str,
    episode: i32,
}

/// The built-in switch texture pair table — 40 real entries + 1 terminator.
///
/// Translated from p_switch.c `alphSwitchList[]`.  The terminator entry has
/// `episode == 0` and empty names.
static ALPH_SWITCH_LIST: [AlphSwitchEntry; 41] = [
    // ---- Episode 1 — Shareware (19 entries) ----
    AlphSwitchEntry {
        name1: "SW1BRCOM",
        name2: "SW2BRCOM",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1BRN1",
        name2: "SW2BRN1",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1BRN2",
        name2: "SW2BRN2",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1BRNGN",
        name2: "SW2BRNGN",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1BROWN",
        name2: "SW2BROWN",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1COMM",
        name2: "SW2COMM",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1COMP",
        name2: "SW2COMP",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1DIRT",
        name2: "SW2DIRT",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1EXIT",
        name2: "SW2EXIT",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1GRAY",
        name2: "SW2GRAY",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1GRAY1",
        name2: "SW2GRAY1",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1METAL",
        name2: "SW2METAL",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1PIPE",
        name2: "SW2PIPE",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1SLAD",
        name2: "SW2SLAD",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1STARG",
        name2: "SW2STARG",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1STON1",
        name2: "SW2STON1",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1STON2",
        name2: "SW2STON2",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1STONE",
        name2: "SW2STONE",
        episode: 1,
    },
    AlphSwitchEntry {
        name1: "SW1STRTN",
        name2: "SW2STRTN",
        episode: 1,
    },
    // ---- Episode 2 — Registered (10 entries) ----
    AlphSwitchEntry {
        name1: "SW1BLUE",
        name2: "SW2BLUE",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1CMT",
        name2: "SW2CMT",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1GARG",
        name2: "SW2GARG",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1GSTON",
        name2: "SW2GSTON",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1HOT",
        name2: "SW2HOT",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1LION",
        name2: "SW2LION",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1SATYR",
        name2: "SW2SATYR",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1SKIN",
        name2: "SW2SKIN",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1VINE",
        name2: "SW2VINE",
        episode: 2,
    },
    AlphSwitchEntry {
        name1: "SW1WOOD",
        name2: "SW2WOOD",
        episode: 2,
    },
    // ---- Episode 3 — DOOM II / Commercial (11 entries) ----
    AlphSwitchEntry {
        name1: "SW1PANEL",
        name2: "SW2PANEL",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1ROCK",
        name2: "SW2ROCK",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1MET2",
        name2: "SW2MET2",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1WDMET",
        name2: "SW2WDMET",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1BRIK",
        name2: "SW2BRIK",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1MOD1",
        name2: "SW2MOD1",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1ZIM",
        name2: "SW2ZIM",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1STON6",
        name2: "SW2STON6",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1TEK",
        name2: "SW2TEK",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1MARB",
        name2: "SW2MARB",
        episode: 3,
    },
    AlphSwitchEntry {
        name1: "SW1SKULL",
        name2: "SW2SKULL",
        episode: 3,
    },
    // ---- Terminator ----
    AlphSwitchEntry {
        name1: "",
        name2: "",
        episode: 0,
    },
];

// =============================================================================
// P_InitSwitchList (p_switch.c lines 107-148)
// =============================================================================

/// Initialize the runtime switch texture pair list from the built-in table.
///
/// Filters `ALPH_SWITCH_LIST` by the current game mode's episode tier:
/// - Shareware / Retail → episode 1 only
/// - Registered → episodes 1-2
/// - Commercial (DOOM II) → episodes 1-3
///
/// Resolved texture numbers are stored as alternating pairs in
/// `SpecState.switchlist[]`.  The entry count is stored in
/// `SpecState.numswitches`.
///
/// Translated from p_switch.c `P_InitSwitchList`.
pub fn p_init_switch_list(game_mode: GameMode, ctx: &mut dyn SpecContext) {
    // Determine the maximum episode tier to include.
    // Original C: registered→2, commercial→3, else→1.
    let episode = match game_mode {
        GameMode::Registered | GameMode::Retail => 2,
        GameMode::Commercial => 3,
        _ => 1, // Shareware, Indetermined
    };

    let mut index: usize = 0;

    for entry in ALPH_SWITCH_LIST.iter() {
        // Terminator: episode == 0 marks end of list.
        if entry.episode == 0 {
            // Store the pair count and sentinel.
            ctx.spec_state_mut().numswitches = (index / 2) as i32;
            if index < MAXSWITCHES * 2 {
                ctx.spec_state_mut().switchlist[index] = -1;
            }
            break;
        }

        if entry.episode <= episode {
            // Resolve both texture names to runtime texture numbers.
            let tex1 = ctx.r_texture_num_for_name(entry.name1);
            let tex2 = ctx.r_texture_num_for_name(entry.name2);

            if index + 1 < MAXSWITCHES * 2 {
                ctx.spec_state_mut().switchlist[index] = tex1;
                ctx.spec_state_mut().switchlist[index + 1] = tex2;
                index += 2;
            }
        }
    }
}

// =============================================================================
// P_StartButton (p_switch.c lines 154-190)
// =============================================================================

/// Register a button timer so that the switch texture will revert after a delay.
///
/// If the line already has an active button (btimer > 0 on a matching line),
/// the call is silently ignored.  If no free button slot exists, panics —
/// matching the original `I_Error("P_StartButton: no button slots left!")`.
///
/// Translated from p_switch.c `P_StartButton`.
fn p_start_button(
    line_idx: usize,
    where_pos: BWhere,
    texture: i32,
    time: i32,
    ctx: &mut dyn SpecContext,
) {
    // Check if this line already has an active button — if so, do nothing.
    // Original C (lines 160-167): scan buttonlist for matching line with btimer > 0.
    for i in 0..MAXBUTTONS {
        let btn = &ctx.spec_state().buttonlist[i];
        if btn.btimer > 0 && btn.line == Some(line_idx) {
            return;
        }
    }

    // Read the front sector index before mutating spec state.
    let frontsector = ctx.lines()[line_idx].frontsector;

    // Find the first free slot (btimer == 0).
    for i in 0..MAXBUTTONS {
        if ctx.spec_state().buttonlist[i].btimer <= 0 {
            let spec = ctx.spec_state_mut();
            spec.buttonlist[i].line = Some(line_idx);
            spec.buttonlist[i].where_pos = where_pos;
            spec.buttonlist[i].btexture = texture;
            spec.buttonlist[i].btimer = time;
            // Sound origin is the front sector (degenmobj_t soundorg).
            // Original C: buttonlist[i].soundorg = (mobj_t *)&line->frontsector->soundorg
            spec.buttonlist[i].soundorg = frontsector;
            return;
        }
    }

    // No free slots — fatal error, matches original I_Error.
    panic!("P_StartButton: no button slots left!");
}

// =============================================================================
// P_ChangeSwitchTexture (p_switch.c lines 200-263)
// =============================================================================

/// Swap a switch's wall texture to its partner and optionally start a button
/// timer for automatic revert.
///
/// Scans the front sidedef's top/middle/bottom textures against the runtime
/// switch list.  When a match is found, the texture is replaced with the
/// XOR-1 partner (`switchlist[i ^ 1]`) and an appropriate sound is played.
///
/// If `use_again` is `true` (button), a timer is started via `p_start_button`
/// so the texture reverts after `BUTTONTIME` tics.  If `false` (one-shot
/// switch), the line's special is cleared to 0.
///
/// **Behavioral note (preserved from original):** The line's special is
/// cleared *before* the exit-sound check, so exit switches (special 11)
/// that are non-reusable will always play `sfx_swtchn` instead of
/// `sfx_swtchx`.  This matches the original engine behavior.
///
/// Sound is played at `buttonlist[0].soundorg` regardless of which button
/// entry is used — this is the original behavior and is preserved.
///
/// Translated from p_switch.c `P_ChangeSwitchTexture`.
pub fn p_change_switch_texture(line_idx: usize, use_again: bool, ctx: &mut dyn SpecContext) {
    // One-shot switch: clear line special so it cannot trigger again.
    // Original C (line 207): if (!useAgain) line->special = 0;
    if !use_again {
        ctx.lines_mut()[line_idx].special = 0;
    }

    // Read front sidedef textures.
    let side_idx = ctx.lines()[line_idx].sidenum[0] as usize;
    let tex_top = ctx.sides()[side_idx].toptexture as i32;
    let tex_mid = ctx.sides()[side_idx].midtexture as i32;
    let tex_bot = ctx.sides()[side_idx].bottomtexture as i32;

    // Determine sound effect.
    // Default is sfx_swtchn; exit switches (special 11) use sfx_swtchx.
    // NOTE: For non-reusable switches, special was already cleared to 0 above,
    // so this check will be false.  This is the original behavior.
    let sound = if ctx.lines()[line_idx].special == 11 {
        SfxEnum::sfx_swtchx
    } else {
        SfxEnum::sfx_swtchn
    };

    let numswitches = ctx.spec_state().numswitches;

    // Scan the runtime switch list for a matching texture.
    // Original C: loop from 0 to numswitches*2.
    for i in 0..(numswitches as usize * 2) {
        let sw_tex = ctx.spec_state().switchlist[i];

        if sw_tex == tex_top {
            // Match on top texture.
            let soundorg = ctx.spec_state().buttonlist[0].soundorg;
            ctx.s_start_sound(soundorg, sound);
            let partner = ctx.spec_state().switchlist[i ^ 1];
            ctx.sides_mut()[side_idx].toptexture = partner as i16;
            if use_again {
                p_start_button(line_idx, BWhere::Top, sw_tex, BUTTONTIME, ctx);
            }
            return;
        }

        if sw_tex == tex_mid {
            // Match on middle texture.
            let soundorg = ctx.spec_state().buttonlist[0].soundorg;
            ctx.s_start_sound(soundorg, sound);
            let partner = ctx.spec_state().switchlist[i ^ 1];
            ctx.sides_mut()[side_idx].midtexture = partner as i16;
            if use_again {
                p_start_button(line_idx, BWhere::Middle, sw_tex, BUTTONTIME, ctx);
            }
            return;
        }

        if sw_tex == tex_bot {
            // Match on bottom texture.
            let soundorg = ctx.spec_state().buttonlist[0].soundorg;
            ctx.s_start_sound(soundorg, sound);
            let partner = ctx.spec_state().switchlist[i ^ 1];
            ctx.sides_mut()[side_idx].bottomtexture = partner as i16;
            if use_again {
                p_start_button(line_idx, BWhere::Bottom, sw_tex, BUTTONTIME, ctx);
            }
            return;
        }
    }
}

// =============================================================================
// P_UseSpecialLine (p_switch.c lines 275-653) — massive switch dispatcher
// =============================================================================

/// Dispatch a player "use" action on a linedef.
///
/// This is the central dispatcher for all line-special triggers activated by
/// the player pressing "use" (spacebar).  It checks the line's special field
/// and delegates to the appropriate subsystem: doors, floors, ceilings,
/// platforms, stairs, lights, exits, etc.
///
/// Returns `true` if the line has a recognized use-special (even if it didn't
/// actually trigger), `false` only for back-side access on non-124 specials
/// or monster access on disallowed specials.
///
/// Translated from p_switch.c `P_UseSpecialLine`.
///
/// # Parameters
///
/// - `thing_idx`: Arena index of the MapObject activating the line.
/// - `line_idx`: Arena index of the LineDef being activated.
/// - `side`: 0 = front side, 1 = back side.
/// - `ctx`: Mutable reference to the SpecContext for cross-module dispatch.
pub fn p_use_special_line(
    thing_idx: usize,
    line_idx: usize,
    side: i32,
    ctx: &mut dyn SpecContext,
) -> bool {
    // ------------------------------------------------------------------
    // Back-side check (p_switch.c lines 282-297)
    // Only case 124 (sliding door, UNUSED) is allowed from the back side.
    // ------------------------------------------------------------------
    if side != 0 {
        match ctx.lines()[line_idx].special {
            124 => { /* Sliding door open & close — UNUSED, fall through */ }
            _ => return false,
        }
    }

    // ------------------------------------------------------------------
    // Non-player monster check (p_switch.c lines 300-318)
    // Monsters may only activate a very limited set of specials.
    // ------------------------------------------------------------------
    let is_player = ctx.mobjs()[thing_idx].player.is_some();

    if !is_player {
        // Monsters never open secret doors.
        let flags = ctx.lines()[line_idx].flags;
        if flags & LineFlags::ML_SECRET.bits() != 0 {
            return false;
        }

        // Monsters can only activate manual doors (1, 32, 33, 34).
        match ctx.lines()[line_idx].special {
            1 | 32 | 33 | 34 => { /* Allowed — fall through to main dispatch */ }
            _ => return false,
        }
    }

    // ------------------------------------------------------------------
    // Main switch on line.special (p_switch.c lines 322-650)
    // ------------------------------------------------------------------
    let special = ctx.lines()[line_idx].special;

    match special {
        // ==============================================================
        // MANUAL DOORS (cases 1, 26-28, 31-34, 117-118)
        // All call EV_VerticalDoor directly.
        // ==============================================================
        1   // Vertical Door
        | 26  // Blue Door / Locked
        | 27  // Yellow Door / Locked
        | 28  // Red Door / Locked
        | 31  // Manual door open
        | 32  // Blue locked door open
        | 33  // Red locked door open
        | 34  // Yellow locked door open
        | 117 // Blazing door raise
        | 118 // Blazing door open
        => {
            ev_vertical_door(line_idx, thing_idx, ctx);
        }

        // ==============================================================
        // SWITCHES — one-time activation (S1)
        // P_ChangeSwitchTexture(line, 0) called only if EV_ returns true,
        // EXCEPT for cases 11 and 51 which always change.
        // ==============================================================

        // Case 7: Build Stairs (build8)
        7 => {
            if ctx.ev_build_stairs(line_idx, StairType::Build8) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 9: Change Donut
        9 => {
            if ev_do_donut(line_idx, ctx) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 11: Exit level
        // CRITICAL: Always change texture, then exit.
        11 => {
            p_change_switch_texture(line_idx, false, ctx);
            ctx.g_exit_level();
        }

        // Case 14: Raise Floor 32 and change texture
        14 => {
            if ctx.ev_do_plat(line_idx, PlatType::RaiseAndChange, 32) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 15: Raise Floor 24 and change texture
        15 => {
            if ctx.ev_do_plat(line_idx, PlatType::RaiseAndChange, 24) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 18: Raise Floor to next highest floor
        18 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloorToNearest) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 20: Raise Plat to next highest and change texture
        20 => {
            if ctx.ev_do_plat(line_idx, PlatType::RaiseToNearestAndChange, 0) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 21: PlatDownWaitUpStay
        21 => {
            if ctx.ev_do_plat(line_idx, PlatType::DownWaitUpStay, 0) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 23: Lower Floor to Lowest
        23 => {
            if ctx.ev_do_floor(line_idx, FloorType::LowerFloorToLowest) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 29: Raise Door (normal)
        29 => {
            if ctx.ev_do_door(line_idx, VldoorType::Normal) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 41: Lower Ceiling to Floor
        41 => {
            if ctx.ev_do_ceiling(line_idx, CeilingType::LowerToFloor) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 71: Turbo Lower Floor
        71 => {
            if ctx.ev_do_floor(line_idx, FloorType::TurboLower) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 49: Ceiling Crush And Raise
        49 => {
            if ctx.ev_do_ceiling(line_idx, CeilingType::CrushAndRaise) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 50: Close Door
        50 => {
            if ctx.ev_do_door(line_idx, VldoorType::Close) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 51: Secret EXIT
        // CRITICAL: Always change texture, then secret exit.
        51 => {
            p_change_switch_texture(line_idx, false, ctx);
            ctx.g_secret_exit_level();
        }

        // Case 55: Raise Floor Crush
        55 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloorCrush) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 101: Raise Floor
        101 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloor) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 102: Lower Floor to Surrounding floor height
        102 => {
            if ctx.ev_do_floor(line_idx, FloorType::LowerFloor) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 103: Open Door
        103 => {
            if ctx.ev_do_door(line_idx, VldoorType::Open) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 111: Blazing Door Raise
        111 => {
            if ctx.ev_do_door(line_idx, VldoorType::BlazeRaise) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 112: Blazing Door Open
        112 => {
            if ctx.ev_do_door(line_idx, VldoorType::BlazeOpen) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 113: Blazing Door Close
        113 => {
            if ctx.ev_do_door(line_idx, VldoorType::BlazeClose) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 122: Blazing PlatDownWaitUpStay
        122 => {
            if ctx.ev_do_plat(line_idx, PlatType::BlazeDWUS, 0) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 127: Build Stairs Turbo 16
        127 => {
            if ctx.ev_build_stairs(line_idx, StairType::Turbo16) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 131: Raise Floor Turbo
        131 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloorTurbo) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Cases 133, 135, 137: Blazing locked door (Blue/Red/Yellow) — one-shot
        133 | 135 | 137 => {
            if ev_do_locked_door(line_idx, VldoorType::BlazeOpen, thing_idx, ctx) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // Case 140: Raise Floor 512
        140 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloor512) {
                p_change_switch_texture(line_idx, false, ctx);
            }
        }

        // ==============================================================
        // BUTTONS — reusable activation (SR)
        // P_ChangeSwitchTexture(line, 1) called only if EV_ returns true,
        // EXCEPT for cases 138 and 139 which always change.
        // ==============================================================

        // Case 42: Close Door (button)
        42 => {
            if ctx.ev_do_door(line_idx, VldoorType::Close) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 43: Lower Ceiling to Floor (button)
        43 => {
            if ctx.ev_do_ceiling(line_idx, CeilingType::LowerToFloor) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 45: Lower Floor (button)
        45 => {
            if ctx.ev_do_floor(line_idx, FloorType::LowerFloor) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 60: Lower Floor to Lowest (button)
        60 => {
            if ctx.ev_do_floor(line_idx, FloorType::LowerFloorToLowest) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 61: Open Door (button)
        61 => {
            if ctx.ev_do_door(line_idx, VldoorType::Open) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 62: PlatDownWaitUpStay (button)
        // NOTE: amount is 1 here, NOT 0 — this matches the original C code.
        62 => {
            if ctx.ev_do_plat(line_idx, PlatType::DownWaitUpStay, 1) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 63: Raise Door (normal, button)
        63 => {
            if ctx.ev_do_door(line_idx, VldoorType::Normal) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 64: Raise Floor to ceiling (button)
        64 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloor) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 66: Raise Floor 24 and change texture (button)
        66 => {
            if ctx.ev_do_plat(line_idx, PlatType::RaiseAndChange, 24) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 67: Raise Floor 32 and change texture (button)
        67 => {
            if ctx.ev_do_plat(line_idx, PlatType::RaiseAndChange, 32) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 65: Raise Floor Crush (button)
        65 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloorCrush) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 68: Raise Plat to next highest and change (button)
        68 => {
            if ctx.ev_do_plat(line_idx, PlatType::RaiseToNearestAndChange, 0) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 69: Raise Floor to next highest floor (button)
        69 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloorToNearest) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 70: Turbo Lower Floor (button)
        70 => {
            if ctx.ev_do_floor(line_idx, FloorType::TurboLower) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 114: Blazing Door Raise (button)
        114 => {
            if ctx.ev_do_door(line_idx, VldoorType::BlazeRaise) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 115: Blazing Door Open (button)
        115 => {
            if ctx.ev_do_door(line_idx, VldoorType::BlazeOpen) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 116: Blazing Door Close (button)
        116 => {
            if ctx.ev_do_door(line_idx, VldoorType::BlazeClose) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 123: Blazing PlatDownWaitUpStay (button)
        123 => {
            if ctx.ev_do_plat(line_idx, PlatType::BlazeDWUS, 0) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 132: Raise Floor Turbo (button)
        132 => {
            if ctx.ev_do_floor(line_idx, FloorType::RaiseFloorTurbo) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Cases 99, 134, 136: Blazing locked door (Blue/Red/Yellow) — button
        99 | 134 | 136 => {
            if ev_do_locked_door(line_idx, VldoorType::BlazeOpen, thing_idx, ctx) {
                p_change_switch_texture(line_idx, true, ctx);
            }
        }

        // Case 138: Light Turn On (brightness 255, button)
        // CRITICAL: Always change texture regardless of action result.
        138 => {
            ctx.ev_light_turn_on(line_idx, 255);
            p_change_switch_texture(line_idx, true, ctx);
        }

        // Case 139: Light Turn Off (brightness 35, button)
        // CRITICAL: Always change texture regardless of action result.
        139 => {
            ctx.ev_light_turn_on(line_idx, 35);
            p_change_switch_texture(line_idx, true, ctx);
        }

        // Unrecognized special — no specific action, but still returns true.
        _ => {}
    }

    true
}
