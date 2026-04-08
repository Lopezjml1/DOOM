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

//! Switch texture change logic.
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
//! | Rust function | C function | Description |
//! |---------------|------------|-------------|
//! | `p_init_switch_list` | `P_InitSwitchList` | Initialize switch texture pair table |
//! | `p_start_button` | `P_StartButton` | Start a timed button reset |
//! | `p_change_switch_texture` | `P_ChangeSwitchTexture` | Swap switch texture + optional timer |
//! | `p_use_special_line` | `P_UseSpecialLine` | Dispatcher for line "use" actions |

// Imports from crate::play::spec are available via cross-module dispatch
// in the SwitchContext trait when a full game state is available.

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of switch textures in the game.
/// Original C: `MAXSWITCHES` (50)
pub const MAXSWITCHES: usize = 50;

/// Number of ticks before a button resets (1 second = 35 tics).
/// Original C: `BUTTONTIME` (35)
pub const BUTTONTIME: i32 = 35;

/// Maximum number of active buttons at once.
/// Original C: `MAXBUTTONS` (16)
pub const MAXBUTTONS: usize = 16;

// =============================================================================
// Switch texture pair list
// =============================================================================

/// A pair of switch texture names (off/on). When a switch is activated, its
/// texture is changed from one to the other.
///
/// Translated from p_switch.c `alphSwitchList[]`.
#[derive(Debug, Clone)]
pub struct SwitchPair {
    /// Texture name in the "off" state.
    pub name1: &'static str,
    /// Texture name in the "on" state.
    pub name2: &'static str,
    /// Minimum episode for this switch pair to be active.
    pub episode: i32,
}

/// The built-in switch texture pair table.
///
/// Translated from p_switch.c `alphSwitchList[]`. Each entry contains two texture
/// names that form a switch pair, and the episode requirement.
pub static ALPHA_SWITCH_LIST: &[SwitchPair] = &[
    // Episode 1 switches
    SwitchPair {
        name1: "SW1BRCOM",
        name2: "SW2BRCOM",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1BRN1",
        name2: "SW2BRN1",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1BRN2",
        name2: "SW2BRN2",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1BRNGN",
        name2: "SW2BRNGN",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1BROWN",
        name2: "SW2BROWN",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1COMM",
        name2: "SW2COMM",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1COMP",
        name2: "SW2COMP",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1DIRT",
        name2: "SW2DIRT",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1EXIT",
        name2: "SW2EXIT",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1GRAY",
        name2: "SW2GRAY",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1GRAY1",
        name2: "SW2GRAY1",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1METAL",
        name2: "SW2METAL",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1PIPE",
        name2: "SW2PIPE",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1SLAD",
        name2: "SW2SLAD",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1STARG",
        name2: "SW2STARG",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1STON1",
        name2: "SW2STON1",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1STON2",
        name2: "SW2STON2",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1STONE",
        name2: "SW2STONE",
        episode: 1,
    },
    SwitchPair {
        name1: "SW1STRTN",
        name2: "SW2STRTN",
        episode: 1,
    },
    // Episode 2 switches
    SwitchPair {
        name1: "SW1BLUE",
        name2: "SW2BLUE",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1CMT",
        name2: "SW2CMT",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1GARG",
        name2: "SW2GARG",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1GSTON",
        name2: "SW2GSTON",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1HOT",
        name2: "SW2HOT",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1LION",
        name2: "SW2LION",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1SATYR",
        name2: "SW2SATYR",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1SKIN",
        name2: "SW2SKIN",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1VINE",
        name2: "SW2VINE",
        episode: 2,
    },
    SwitchPair {
        name1: "SW1WOOD",
        name2: "SW2WOOD",
        episode: 2,
    },
    // DOOM II switches
    SwitchPair {
        name1: "SW1PANEL",
        name2: "SW2PANEL",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1ROCK",
        name2: "SW2ROCK",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1MET2",
        name2: "SW2MET2",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1WDMET",
        name2: "SW2WDMET",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1BRIK",
        name2: "SW2BRIK",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1MOD1",
        name2: "SW2MOD1",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1ZIM",
        name2: "SW2ZIM",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1STON6",
        name2: "SW2STON6",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1TEK",
        name2: "SW2TEK",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1MARB",
        name2: "SW2MARB",
        episode: 3,
    },
    SwitchPair {
        name1: "SW1SKULL",
        name2: "SW2SKULL",
        episode: 3,
    },
];

// =============================================================================
// Active button tracking
// =============================================================================

/// An active button waiting to reset its texture.
///
/// Translated from p_switch.c `button_t`.
#[derive(Debug, Clone)]
pub struct Button {
    /// Index of the line with the switch texture.
    pub line: usize,
    /// Which texture position was changed (top, middle, bottom).
    pub position: SwitchPosition,
    /// Texture number to revert to when the timer expires.
    pub texture: i32,
    /// Countdown timer in tics.
    pub timer: i32,
    /// Sound origin sector for the revert click.
    pub sound_origin: usize,
}

/// Which part of the sidedef holds the switch texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchPosition {
    /// Top texture.
    Top,
    /// Middle texture.
    Middle,
    /// Bottom texture.
    Bottom,
}

/// Tracks all active buttons that need to revert their textures.
#[derive(Debug)]
pub struct ButtonList {
    /// Active buttons awaiting timer expiry.
    pub buttons: [Option<Button>; MAXBUTTONS],
}

impl ButtonList {
    /// Create a new empty button list.
    pub fn new() -> Self {
        const NONE: Option<Button> = None;
        Self {
            buttons: [NONE; MAXBUTTONS],
        }
    }

    /// Clear all active buttons (done at level start).
    pub fn clear(&mut self) {
        for slot in self.buttons.iter_mut() {
            *slot = None;
        }
    }
}

impl Default for ButtonList {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Switch state management
// =============================================================================

/// Runtime switch list built from `ALPHA_SWITCH_LIST` filtered by episode.
///
/// The switch list is a flat array of texture number pairs. `switchlist[i]` and
/// `switchlist[i+1]` form a pair (where i is even).
#[derive(Debug)]
pub struct SwitchList {
    /// Flat list of texture number pairs: [off1, on1, off2, on2, ...].
    pub switchlist: Vec<i32>,
    /// Number of switch pairs.
    pub numswitches: usize,
}

impl SwitchList {
    /// Create an empty switch list.
    pub fn new() -> Self {
        Self {
            switchlist: Vec::new(),
            numswitches: 0,
        }
    }
}

impl Default for SwitchList {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Context trait for switch operations
// =============================================================================

/// Context trait providing game state access needed by switch/button logic.
pub trait SwitchContext {
    /// Look up a texture number by name. Returns -1 if not found.
    fn texture_num_for_name(&self, name: &str) -> i32;

    /// Get the top texture of the front sidedef of a line.
    fn line_front_top_texture(&self, line_idx: usize) -> i32;

    /// Get the middle texture of the front sidedef of a line.
    fn line_front_mid_texture(&self, line_idx: usize) -> i32;

    /// Get the bottom texture of the front sidedef of a line.
    fn line_front_bottom_texture(&self, line_idx: usize) -> i32;

    /// Set the top texture of the front sidedef of a line.
    fn set_line_front_top_texture(&mut self, line_idx: usize, tex: i32);

    /// Set the middle texture of the front sidedef of a line.
    fn set_line_front_mid_texture(&mut self, line_idx: usize, tex: i32);

    /// Set the bottom texture of the front sidedef of a line.
    fn set_line_front_bottom_texture(&mut self, line_idx: usize, tex: i32);

    /// Get the sector index for the front side of a line.
    fn line_front_sector(&self, line_idx: usize) -> usize;

    /// Get the game episode number for switch list filtering.
    fn game_episode(&self) -> i32;

    /// Start a sound at a sector.
    fn start_sector_sound(&mut self, sector_idx: usize, sfx: i32);
}

// =============================================================================
// P_InitSwitchList — build runtime switch pair table
// =============================================================================

/// Initialize the switch texture pair list from the built-in table.
///
/// Filters `ALPHA_SWITCH_LIST` based on the current episode and resolves
/// texture names to texture numbers.
///
/// Translated from p_switch.c `P_InitSwitchList`.
pub fn p_init_switch_list(switch_list: &mut SwitchList, ctx: &dyn SwitchContext) {
    let episode = ctx.game_episode();

    // Determine which episode max to include
    let max_ep = if episode >= 3 { 3 } else { episode };

    switch_list.switchlist.clear();
    switch_list.numswitches = 0;

    for pair in ALPHA_SWITCH_LIST {
        if pair.episode <= max_ep {
            let tex1 = ctx.texture_num_for_name(pair.name1);
            let tex2 = ctx.texture_num_for_name(pair.name2);

            // Only add if both textures exist
            if tex1 >= 0 && tex2 >= 0 {
                switch_list.switchlist.push(tex1);
                switch_list.switchlist.push(tex2);
                switch_list.numswitches += 1;
            }
        }
    }
}

// =============================================================================
// P_StartButton — start a timed button reset
// =============================================================================

/// Register a button that will revert its texture after a delay.
///
/// Translated from p_switch.c `P_StartButton`.
pub fn p_start_button(
    button_list: &mut ButtonList,
    line: usize,
    position: SwitchPosition,
    texture: i32,
    timer: i32,
    sound_origin: usize,
) {
    for slot in button_list.buttons.iter_mut() {
        if slot.is_none() {
            *slot = Some(Button {
                line,
                position,
                texture,
                timer,
                sound_origin,
            });
            return;
        }
    }

    // All button slots full — in the original C code this is an I_Error,
    // but we'll log and drop the oldest button to avoid crashing.
    tracing::warn!("P_StartButton: no free button slots, dropping oldest");
    button_list.buttons[0] = Some(Button {
        line,
        position,
        texture,
        timer,
        sound_origin,
    });
}

// =============================================================================
// P_ChangeSwitchTexture — swap switch texture on activation
// =============================================================================

/// Sound effect for switch activation.
/// Original C: `sfx_swtchn`
pub const SFX_SWTCHN: i32 = 50;

/// Sound effect for switch exit.
/// Original C: `sfx_swtchx`
pub const SFX_SWTCHX: i32 = 51;

/// Change a switch's wall texture and optionally start a button timer.
///
/// This scans the three texture slots (top, middle, bottom) of the line's front
/// sidedef, looking for a match in the switch list. When found, the texture is
/// replaced with its counterpart and an appropriate sound is played.
///
/// If `use_again` is true, a button timer is started so the texture reverts
/// after `BUTTONTIME` ticks.
///
/// Translated from p_switch.c `P_ChangeSwitchTexture`.
pub fn p_change_switch_texture(
    line_idx: usize,
    use_again: bool,
    switch_list: &SwitchList,
    button_list: &mut ButtonList,
    ctx: &mut dyn SwitchContext,
) {
    let sound_origin = ctx.line_front_sector(line_idx);

    // Determine which sound to play (exit switches play different sound)
    let sound = if !use_again { SFX_SWTCHX } else { SFX_SWTCHN };

    // Search all three texture positions for a switch match
    for i in 0..switch_list.numswitches {
        let tex_off = switch_list.switchlist[i * 2];
        let tex_on = switch_list.switchlist[i * 2 + 1];

        // Check top texture
        let top = ctx.line_front_top_texture(line_idx);
        if top == tex_off || top == tex_on {
            ctx.start_sector_sound(sound_origin, sound);
            let new_tex = if top == tex_off { tex_on } else { tex_off };
            ctx.set_line_front_top_texture(line_idx, new_tex);
            if use_again {
                p_start_button(
                    button_list,
                    line_idx,
                    SwitchPosition::Top,
                    top,
                    BUTTONTIME,
                    sound_origin,
                );
            }
            return;
        }

        // Check middle texture
        let mid = ctx.line_front_mid_texture(line_idx);
        if mid == tex_off || mid == tex_on {
            ctx.start_sector_sound(sound_origin, sound);
            let new_tex = if mid == tex_off { tex_on } else { tex_off };
            ctx.set_line_front_mid_texture(line_idx, new_tex);
            if use_again {
                p_start_button(
                    button_list,
                    line_idx,
                    SwitchPosition::Middle,
                    mid,
                    BUTTONTIME,
                    sound_origin,
                );
            }
            return;
        }

        // Check bottom texture
        let bot = ctx.line_front_bottom_texture(line_idx);
        if bot == tex_off || bot == tex_on {
            ctx.start_sector_sound(sound_origin, sound);
            let new_tex = if bot == tex_off { tex_on } else { tex_off };
            ctx.set_line_front_bottom_texture(line_idx, new_tex);
            if use_again {
                p_start_button(
                    button_list,
                    line_idx,
                    SwitchPosition::Bottom,
                    bot,
                    BUTTONTIME,
                    sound_origin,
                );
            }
            return;
        }
    }
}

// =============================================================================
// P_UseSpecialLine — dispatcher for player "use" actions on linedefs
// =============================================================================

/// Result of a P_UseSpecialLine call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseLineResult {
    /// The line was successfully activated.
    Activated,
    /// The line has no use special or was already activated (one-shot).
    NoEffect,
    /// The player lacks a key for a locked door.
    Locked,
}

/// Dispatch a player "use" action on a linedef.
///
/// This is the central dispatcher for all line-special triggers activated by the
/// player pressing "use". It checks the line's special field and delegates to the
/// appropriate module function (door, floor, ceiling, platform, etc.).
///
/// Translated from p_switch.c `P_UseSpecialLine`. The original function is a
/// massive switch statement (~350 lines) covering all use-activated line specials.
///
/// Due to the circular dependencies between modules (doors, floors, platforms,
/// ceilings, etc.), this function uses line special numbers directly and returns
/// `UseLineResult` indicating what happened. The actual game loop dispatches the
/// results to the appropriate module handlers.
///
/// The line specials are divided into:
/// - **Manual doors** (1, 26–28, 31–34, 117–118): Direct door open/close
/// - **Switches (repeatable, SR/S1)**: Activate once or repeatedly
/// - Each special number maps to a specific action type and parameters
///
/// In this translation, we provide the dispatcher structure matching the original
/// C special numbers. The actual module function calls are performed by the
/// game loop using the returned information.
pub fn p_use_special_line(line_special: i16, is_from_front: bool) -> UseLineResult {
    // All use-activated specials must be triggered from the front side
    if !is_from_front {
        return UseLineResult::NoEffect;
    }

    // Check if this is a recognized use-activated special
    match line_special {
        // ======= MANUAL DOORS =======
        // Manual door open (player push, any type)
        1 => UseLineResult::Activated,
        // Blue key manual door
        26 => UseLineResult::Activated,
        // Yellow key manual door
        27 => UseLineResult::Activated,
        // Red key manual door
        28 => UseLineResult::Activated,
        // Manual door open stay
        31 => UseLineResult::Activated,
        // Blue key door open stay
        32 => UseLineResult::Activated,
        // Red key door open stay
        33 => UseLineResult::Activated,
        // Yellow key door open stay
        34 => UseLineResult::Activated,

        // ======= SWITCHES (S1 = use once) =======
        // S1 Door Open
        103 => UseLineResult::Activated,
        // S1 Door Close
        50 => UseLineResult::Activated,
        // S1 Raise stairs (8 unit steps)
        7 => UseLineResult::Activated,
        // S1 Floor Raise to Lowest Ceiling
        18 => UseLineResult::Activated,
        // S1 Floor Lower to Highest Floor
        23 => UseLineResult::Activated,
        // S1 Floor Raise to Next Higher Floor
        55 => UseLineResult::Activated,
        // S1 Platform Down Wait Up Stay
        14 => UseLineResult::Activated,
        // S1 Lights to brightest adjacent
        35 => UseLineResult::Activated,
        // S1 Lights to darkest adjacent
        36 => UseLineResult::Activated,
        // S1 Floor Lower to Lowest Floor
        38 => UseLineResult::Activated,
        // S1 Floor Lower to Lowest Floor (changes texture)
        37 => UseLineResult::Activated,
        // S1 Ceiling Crush and Raise
        49 => UseLineResult::Activated,
        // S1 Ceiling lower to floor
        110 => UseLineResult::Activated,
        // S1 Blazing Door Raise
        111 => UseLineResult::Activated,
        // S1 Blazing Door Open
        112 => UseLineResult::Activated,
        // S1 Blazing Door Close
        113 => UseLineResult::Activated,
        // S1 Door Open/Close wait 30s
        29 => UseLineResult::Activated,
        // S1 Floor Raise to Shortest Lower Texture
        22 => UseLineResult::Activated,
        // S1 Blue locked door open (blazing)
        99 => UseLineResult::Activated,
        // S1 Red locked door open (blazing)
        134 | 135 => UseLineResult::Activated,
        // S1 Yellow locked door open (blazing)
        136 | 137 => UseLineResult::Activated,
        // S1 Blue locked door open
        133 => UseLineResult::Activated,
        // S1 Turbo stairs
        100 => UseLineResult::Activated,
        // S1 Floor raise 512
        140 => UseLineResult::Activated,
        // S1 Exit level
        11 => UseLineResult::Activated,
        // S1 Secret exit
        51 => UseLineResult::Activated,
        // S1 Floor raise to nearest + change texture
        71 => UseLineResult::Activated,

        // ======= SWITCHES (SR = repeatable) =======
        // SR Door close
        42 => UseLineResult::Activated,
        // SR Ceiling lower to floor
        43 => UseLineResult::Activated,
        // SR Floor lower to highest floor
        45 => UseLineResult::Activated,
        // SR Lights to darkest adjacent
        60 => UseLineResult::Activated,
        // SR Floor lower to lowest floor
        61 => UseLineResult::Activated,
        // SR Platform Down-Wait-Up-Stay
        62 => UseLineResult::Activated,
        // SR Door open wait close
        63 => UseLineResult::Activated,
        // SR Floor raise to lowest ceiling
        64 => UseLineResult::Activated,
        // SR Floor raise crush
        65 => UseLineResult::Activated,
        // SR Floor raise 24 change texture
        66 => UseLineResult::Activated,
        // SR Floor raise 32 change texture
        67 => UseLineResult::Activated,
        // SR Floor lower to lowest floor (change texture + type)
        68 => UseLineResult::Activated,
        // SR Lights to brightest adjacent
        69 => UseLineResult::Activated,
        // SR Floor raise to next higher
        70 => UseLineResult::Activated,
        // SR Floor lower to 8 above highest floor
        72 => UseLineResult::Activated,
        // SR Ceiling crush stop
        73 => UseLineResult::Activated,
        // SR Platform stop
        74 => UseLineResult::Activated,
        // SR Door close stay
        75 => UseLineResult::Activated,
        // SR Close door 30 then open
        76 => UseLineResult::Activated,
        // SR Ceiling crush and raise
        77 => UseLineResult::Activated,
        // SR Blazing door raise
        114 => UseLineResult::Activated,
        // SR Blazing door open
        115 => UseLineResult::Activated,
        // SR Blazing door close
        116 => UseLineResult::Activated,
        // SR Blazing door open wait close (repeatable)
        117 | 118 => UseLineResult::Activated,
        // SR Floor raise to shortest lower texture
        102 => UseLineResult::Activated,
        // SR Floor raise 24
        58 => UseLineResult::Activated,
        // SR Floor raise 512
        141 => UseLineResult::Activated,

        // Unrecognized special — no effect
        _ => UseLineResult::NoEffect,
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_button_list_new_empty() {
        let bl = ButtonList::new();
        for slot in &bl.buttons {
            assert!(slot.is_none());
        }
    }

    #[test]
    fn test_button_list_clear() {
        let mut bl = ButtonList::new();
        bl.buttons[0] = Some(Button {
            line: 0,
            position: SwitchPosition::Middle,
            texture: 42,
            timer: BUTTONTIME,
            sound_origin: 0,
        });
        bl.clear();
        for slot in &bl.buttons {
            assert!(slot.is_none());
        }
    }

    #[test]
    fn test_start_button_fills_first_slot() {
        let mut bl = ButtonList::new();
        p_start_button(&mut bl, 5, SwitchPosition::Top, 10, BUTTONTIME, 3);
        assert!(bl.buttons[0].is_some());
        let btn = bl.buttons[0].as_ref().unwrap();
        assert_eq!(btn.line, 5);
        assert_eq!(btn.texture, 10);
        assert_eq!(btn.timer, BUTTONTIME);
    }

    #[test]
    fn test_use_special_line_manual_doors() {
        assert_eq!(p_use_special_line(1, true), UseLineResult::Activated);
        assert_eq!(p_use_special_line(26, true), UseLineResult::Activated);
        assert_eq!(p_use_special_line(27, true), UseLineResult::Activated);
        assert_eq!(p_use_special_line(28, true), UseLineResult::Activated);
        assert_eq!(p_use_special_line(31, true), UseLineResult::Activated);
    }

    #[test]
    fn test_use_special_line_from_back() {
        // All use specials must be from front side
        assert_eq!(p_use_special_line(1, false), UseLineResult::NoEffect);
        assert_eq!(p_use_special_line(11, false), UseLineResult::NoEffect);
    }

    #[test]
    fn test_use_special_line_unknown() {
        assert_eq!(p_use_special_line(9999, true), UseLineResult::NoEffect);
        assert_eq!(p_use_special_line(0, true), UseLineResult::NoEffect);
    }

    #[test]
    fn test_switch_list_default() {
        let sl = SwitchList::default();
        assert_eq!(sl.numswitches, 0);
        assert!(sl.switchlist.is_empty());
    }

    #[test]
    fn test_alpha_switch_list_entries() {
        // Verify the list has the expected number of entries
        assert_eq!(ALPHA_SWITCH_LIST.len(), 40);

        // Verify episode 1 switches come first
        assert_eq!(ALPHA_SWITCH_LIST[0].name1, "SW1BRCOM");
        assert_eq!(ALPHA_SWITCH_LIST[0].episode, 1);

        // Verify last entry is a DOOM II switch
        let last = &ALPHA_SWITCH_LIST[ALPHA_SWITCH_LIST.len() - 1];
        assert_eq!(last.episode, 3);
    }

    #[test]
    fn test_switch_position_variants() {
        let positions = [
            SwitchPosition::Top,
            SwitchPosition::Middle,
            SwitchPosition::Bottom,
        ];
        assert_eq!(positions.len(), 3);
        assert_ne!(SwitchPosition::Top, SwitchPosition::Bottom);
    }
}
