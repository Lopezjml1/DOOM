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
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

//! Game completion, final screen animation.
//!
//! Translated from `linuxdoom-1.10/f_finale.c` and `f_finale.h`.
//!
//! Implements the end-of-episode/game sequences:
//! - Scrolling text over a background flat (all episodes / DOOM II endings)
//! - Bunny picture scroll (Episode 3 ending)
//! - Monster cast call sequence (DOOM II MAP30 ending)
//!
//! Three major subsystems:
//! 1. **Text display** (`f_text_write`): tiles a flat background and reveals
//!    finale text one character at a time using HU font patches.
//! 2. **Bunny scroll** (`f_bunny_scroll`): horizontally scrolls PFUB1/PFUB2
//!    patches and overlays "THE END" titles.
//! 3. **Cast call** (`f_start_cast`, `f_cast_ticker`, `f_cast_responder`,
//!    `f_cast_drawer`, `f_cast_print`): parades every monster in the game
//!    with animations and sound effects.

use crate::game::strings;
use crate::info::mobjinfo::{MobjType, MOBJINFO};
use crate::info::sounds::{MusicEnum, SfxEnum};
use crate::info::sprites::SpriteNum;
use crate::info::states::{ActionFnId, StateNum, STATES};
use crate::types::doomdef::{GameMission, GameMode, GameState, SCREENHEIGHT, SCREENWIDTH};
use crate::types::event::{Event, EventType};
use crate::types::player::Player;
use crate::ui::hud::{HU_FONTSIZE, HU_FONTSTART};
use crate::video::video::VideoState;

// ---------------------------------------------------------------------------
// Constants (from f_finale.c lines 39-40)
// ---------------------------------------------------------------------------

/// Tics per character reveal during the finale text scroll.
///
/// Original C: `#define TEXTSPEED 3` (f_finale.c line 39).
pub const TEXTSPEED: i32 = 3;

/// Tics to wait after all text has been revealed before advancing.
///
/// Original C: `#define TEXTWAIT 250` (f_finale.c line 40).
pub const TEXTWAIT: i32 = 250;

// ---------------------------------------------------------------------------
// CastInfo — cast call monster definition
// ---------------------------------------------------------------------------

/// A single entry in the cast call sequence.
///
/// Pairs a display name (drawn centered on screen) with the map-object type
/// whose animation states drive the cast-call sprite display.
///
/// Original C: `castinfo_t` (f_finale.c lines 330-334).
#[derive(Debug, Clone, Copy)]
pub struct CastInfo {
    /// Human-readable name displayed below the sprite (e.g. "Zombieman").
    pub name: &'static str,
    /// Map-object type providing animation states and sound effects.
    pub mobj_type: MobjType,
}

// ---------------------------------------------------------------------------
// CASTORDER — the exact parade sequence (f_finale.c lines 337-359)
// ---------------------------------------------------------------------------

/// Monster parade order for the DOOM II cast call sequence.
///
/// 17 entries in the original order: zombieman through player ("Our Hero").
/// The original C array has an 18th NULL-terminator entry; Rust uses a
/// fixed-length array with bounds checking instead.
pub const CASTORDER: [CastInfo; 17] = [
    CastInfo {
        name: strings::CC_ZOMBIE,
        mobj_type: MobjType::MT_POSSESSED,
    },
    CastInfo {
        name: strings::CC_SHOTGUN,
        mobj_type: MobjType::MT_SHOTGUY,
    },
    CastInfo {
        name: strings::CC_HEAVY,
        mobj_type: MobjType::MT_CHAINGUY,
    },
    CastInfo {
        name: strings::CC_IMP,
        mobj_type: MobjType::MT_TROOP,
    },
    CastInfo {
        name: strings::CC_DEMON,
        mobj_type: MobjType::MT_SERGEANT,
    },
    CastInfo {
        name: strings::CC_LOST,
        mobj_type: MobjType::MT_SKULL,
    },
    CastInfo {
        name: strings::CC_CACO,
        mobj_type: MobjType::MT_HEAD,
    },
    CastInfo {
        name: strings::CC_HELL,
        mobj_type: MobjType::MT_KNIGHT,
    },
    CastInfo {
        name: strings::CC_BARON,
        mobj_type: MobjType::MT_BRUISER,
    },
    CastInfo {
        name: strings::CC_ARACH,
        mobj_type: MobjType::MT_BABY,
    },
    CastInfo {
        name: strings::CC_PAIN,
        mobj_type: MobjType::MT_PAIN,
    },
    CastInfo {
        name: strings::CC_REVEN,
        mobj_type: MobjType::MT_UNDEAD,
    },
    CastInfo {
        name: strings::CC_MANCU,
        mobj_type: MobjType::MT_FATSO,
    },
    CastInfo {
        name: strings::CC_ARCH,
        mobj_type: MobjType::MT_VILE,
    },
    CastInfo {
        name: strings::CC_SPIDER,
        mobj_type: MobjType::MT_SPIDER,
    },
    CastInfo {
        name: strings::CC_CYBER,
        mobj_type: MobjType::MT_CYBORG,
    },
    CastInfo {
        name: strings::CC_HERO,
        mobj_type: MobjType::MT_PLAYER,
    },
];

// ---------------------------------------------------------------------------
// FinaleState — all mutable finale state consolidated (no static mut)
// ---------------------------------------------------------------------------

/// Holds all mutable state for the finale sequence.
///
/// Per AAP §0.7.5, no `static mut` is used; all formerly-global C variables
/// are consolidated into this struct and passed by mutable reference.
pub struct FinaleState {
    /// Current finale stage: 0 = text scroll, 1 = art screen, 2 = cast call.
    pub finalestage: i32,
    /// Animation frame counter (incremented each tic).
    pub finalecount: i32,
    /// The text string being revealed character-by-character.
    pub finaletext: &'static str,
    /// Name of the flat lump used as the tiled text background.
    pub finaleflat: &'static str,

    // --- Cast call state ---
    /// Index into [`CASTORDER`] for the current cast member.
    pub castnum: usize,
    /// Remaining tics for the current cast animation frame.
    pub casttics: i32,
    /// Current animation state (index into the state table).
    pub caststate: StateNum,
    /// `true` when showing the death animation for the current cast member.
    pub castdeath: bool,
    /// Number of animation frames displayed in the current sequence.
    pub castframes: i32,
    /// Alternates between melee and missile attack animations.
    pub castonmelee: bool,
    /// `true` when the cast member is performing an attack animation.
    pub castattacking: bool,
}

impl Default for FinaleState {
    fn default() -> Self {
        Self {
            finalestage: 0,
            finalecount: 0,
            finaletext: "",
            finaleflat: "",
            castnum: 0,
            casttics: 0,
            caststate: StateNum::S_NULL,
            castdeath: false,
            castframes: 0,
            castonmelee: false,
            castattacking: false,
        }
    }
}

// ---------------------------------------------------------------------------
// f_start_finale — initialize the finale sequence
// ---------------------------------------------------------------------------

/// Initialize the finale sequence for the current episode/map.
///
/// Selects the appropriate background flat and text string based on the
/// game mode, mission pack, episode number, and map number. Returns a
/// tuple of `(music_to_play, game_state)` so the caller can trigger the
/// appropriate music change and game-state transition.
///
/// Original C: `F_StartFinale` (f_finale.c lines 96-191).
///
/// # Arguments
/// * `state` — Mutable reference to the finale state to initialize.
/// * `gamemode` — Current game mode (Shareware, Registered, Commercial, Retail).
/// * `gamemission` — Current mission pack (Doom, Doom2, PackTnt, PackPlut).
/// * `gameepisode` — Episode number (1-based, used for DOOM 1).
/// * `gamemap` — Map number (1-based, used for DOOM II).
///
/// # Returns
/// A `MusicEnum` value for the music track to start, plus `GameState::Finale`.
pub fn f_start_finale(
    state: &mut FinaleState,
    gamemode: GameMode,
    gamemission: GameMission,
    gameepisode: i32,
    gamemap: i32,
) -> (MusicEnum, GameState) {
    // Reset finale state
    state.finalestage = 0;
    state.finalecount = 0;
    state.castnum = 0;
    state.casttics = 0;
    state.caststate = StateNum::S_NULL;
    state.castdeath = false;
    state.castframes = 0;
    state.castonmelee = false;
    state.castattacking = false;

    let music;

    // Select text, flat, and music based on game mode
    // Original C: f_finale.c lines 104-189
    match gamemode {
        GameMode::Shareware | GameMode::Registered | GameMode::Retail => {
            // DOOM 1 episodes
            music = MusicEnum::mus_victor;
            match gameepisode {
                1 => {
                    state.finaleflat = "FLOOR4_8";
                    state.finaletext = strings::E1TEXT;
                }
                2 => {
                    state.finaleflat = "SFLR6_1";
                    state.finaletext = strings::E2TEXT;
                }
                3 => {
                    state.finaleflat = "MFLR8_4";
                    state.finaletext = strings::E3TEXT;
                }
                4 => {
                    state.finaleflat = "MFLR8_3";
                    state.finaletext = strings::E4TEXT;
                }
                _ => {
                    // Fallback for unexpected episode values
                    state.finaleflat = "FLOOR4_8";
                    state.finaletext = strings::E1TEXT;
                }
            }
        }
        GameMode::Commercial => {
            // DOOM II / Final Doom — text/flat varies by mission and map
            music = MusicEnum::mus_read_m;
            match gamemission {
                GameMission::PackTnt => {
                    // TNT: Evilution
                    match gamemap {
                        6 => {
                            state.finaleflat = "SLIME16";
                            state.finaletext = strings::T1TEXT;
                        }
                        11 => {
                            state.finaleflat = "RROCK14";
                            state.finaletext = strings::T2TEXT;
                        }
                        20 => {
                            state.finaleflat = "RROCK07";
                            state.finaletext = strings::T3TEXT;
                        }
                        30 => {
                            state.finaleflat = "RROCK17";
                            state.finaletext = strings::T4TEXT;
                        }
                        15 => {
                            state.finaleflat = "RROCK13";
                            state.finaletext = strings::T5TEXT;
                        }
                        31 => {
                            state.finaleflat = "RROCK19";
                            state.finaletext = strings::T6TEXT;
                        }
                        _ => {
                            // FIXME - other text, music?
                            state.finaleflat = "F_SKY1";
                            state.finaletext = strings::C1TEXT;
                        }
                    }
                }
                GameMission::PackPlut => {
                    // The Plutonia Experiment
                    match gamemap {
                        6 => {
                            state.finaleflat = "SLIME16";
                            state.finaletext = strings::P1TEXT;
                        }
                        11 => {
                            state.finaleflat = "RROCK14";
                            state.finaletext = strings::P2TEXT;
                        }
                        20 => {
                            state.finaleflat = "RROCK07";
                            state.finaletext = strings::P3TEXT;
                        }
                        30 => {
                            state.finaleflat = "RROCK17";
                            state.finaletext = strings::P4TEXT;
                        }
                        15 => {
                            state.finaleflat = "RROCK13";
                            state.finaletext = strings::P5TEXT;
                        }
                        31 => {
                            state.finaleflat = "RROCK19";
                            state.finaletext = strings::P6TEXT;
                        }
                        _ => {
                            // FIXME - other text, music?
                            state.finaleflat = "F_SKY1";
                            state.finaletext = strings::C1TEXT;
                        }
                    }
                }
                _ => {
                    // DOOM II (standard)
                    match gamemap {
                        6 => {
                            state.finaleflat = "SLIME16";
                            state.finaletext = strings::C1TEXT;
                        }
                        11 => {
                            state.finaleflat = "RROCK14";
                            state.finaletext = strings::C2TEXT;
                        }
                        20 => {
                            state.finaleflat = "RROCK07";
                            state.finaletext = strings::C3TEXT;
                        }
                        30 => {
                            state.finaleflat = "RROCK17";
                            state.finaletext = strings::C4TEXT;
                        }
                        15 => {
                            state.finaleflat = "RROCK13";
                            state.finaletext = strings::C5TEXT;
                        }
                        31 => {
                            state.finaleflat = "RROCK19";
                            state.finaletext = strings::C6TEXT;
                        }
                        _ => {
                            // FIXME - other text, music?
                            state.finaleflat = "F_SKY1";
                            state.finaletext = strings::C1TEXT;
                        }
                    }
                }
            }
        }
        _ => {
            // Indetermined mode fallback
            music = MusicEnum::mus_read_m;
            state.finaleflat = "F_SKY1";
            state.finaletext = strings::C1TEXT;
        }
    }

    (music, GameState::Finale)
}

// ---------------------------------------------------------------------------
// f_responder — handle input events during the finale
// ---------------------------------------------------------------------------

/// Process an input event during the finale.
///
/// During the cast call stage (stage 2), delegates to [`f_cast_responder`].
/// All other stages do not consume events.
///
/// Original C: `F_Responder` (f_finale.c lines 195-201).
///
/// # Returns
/// `true` if the event was consumed, `false` otherwise.
pub fn f_responder(state: &mut FinaleState, ev: &Event) -> bool {
    if state.finalestage == 2 {
        return f_cast_responder(state, ev);
    }
    false
}

// ---------------------------------------------------------------------------
// f_ticker — advance the finale one tic
// ---------------------------------------------------------------------------

/// Advance the finale animation by one game tic.
///
/// Handles three stages:
/// - **Stage 0** (text scroll): increments `finalecount`; when any player
///   presses a button in commercial mode (after tic 50), either starts the
///   cast call (MAP30) or signals the caller to advance the world.
/// - **Stage 1** (art screen): no ticker logic; display only.
/// - **Stage 2** (cast call): delegates to [`f_cast_ticker`].
///
/// Original C: `F_Ticker` (f_finale.c lines 207-259).
///
/// # Arguments
/// * `state` — Mutable finale state.
/// * `gamemode` — Current game mode.
/// * `gameepisode` — Episode number (1-based).
/// * `gamemap` — Map number (1-based, needed for DOOM II cast call check).
/// * `players` — Player array for reading button state.
/// * `maxplayers` — Number of player slots to check for button presses.
///
/// # Returns
/// A `TickerResult` indicating what action the caller should take.
pub fn f_ticker(
    state: &mut FinaleState,
    gamemode: GameMode,
    gameepisode: i32,
    gamemap: i32,
    players: &[Player],
    maxplayers: usize,
) -> TickerResult {
    // Check for skipping (commercial mode only, after tic 50)
    // Original C: f_finale.c lines 215-232
    if gamemode == GameMode::Commercial && state.finalecount > 50 {
        // Check if any player is pressing a button
        let mut skip = false;
        let count = maxplayers.min(players.len());
        for player in players.iter().take(count) {
            if player.cmd.buttons != 0 {
                skip = true;
                break;
            }
        }

        if skip {
            if gamemap == 30 {
                f_start_cast(state);
                return TickerResult::CastStarted;
            } else {
                return TickerResult::WorldDone;
            }
        }
    }

    // Advance animation counter
    state.finalecount += 1;

    // If already in cast call stage, run the cast ticker
    if state.finalestage == 2 {
        let sfx = f_cast_ticker(state);
        return TickerResult::CastTick(sfx);
    }

    // For commercial mode, no further stage transitions in ticker
    // (cast call is entered via the skip logic above)
    if gamemode == GameMode::Commercial {
        return TickerResult::None;
    }

    // Redundant check from original C (line 252): if (finalestage == 2) return
    if state.finalestage == 2 {
        return TickerResult::None;
    }

    // Check if text display is complete and we should advance to art screen
    // Original C: f_finale.c lines 254-259
    let text_len = state.finaletext.len() as i32;
    let text_complete_tic = text_len * TEXTSPEED + TEXTWAIT;

    if state.finalecount > text_complete_tic {
        // Text phase complete — advance to art screen stage
        state.finalecount = 0;
        state.finalestage = 1;
        // wipegamestate = -1 in C; the caller handles wipe state

        // For DOOM 1 episode 3, play bunny scroll music
        if gameepisode == 3 {
            return TickerResult::MusicChange(MusicEnum::mus_bunny);
        }
    }

    TickerResult::None
}

/// Result of a [`f_ticker`] call, indicating what the caller should do.
#[derive(Debug, Clone)]
pub enum TickerResult {
    /// No special action needed.
    None,
    /// A music track change is requested.
    MusicChange(MusicEnum),
    /// The cast call sequence was started (DOOM II MAP30).
    CastStarted,
    /// The world should advance to the next level (commercial non-MAP30 skip).
    WorldDone,
    /// Cast call animation advanced; optional sound effect to play.
    CastTick(Option<SfxEnum>),
}

// ---------------------------------------------------------------------------
// f_text_write — render the scrolling text over a tiled flat
// ---------------------------------------------------------------------------

/// Draw the finale text screen: tiled flat background with character-by-
/// character text reveal.
///
/// The background flat is tiled across the entire 320×200 screen. Text is
/// revealed one character per `TEXTSPEED` tics, starting at pixel position
/// (10, 10). Newlines advance `cy` by 11 pixels. Non-printable characters
/// (outside the HU font range) advance `cx` by 4 pixels.
///
/// Original C: `F_TextWrite` (f_finale.c lines 261-327).
///
/// # Arguments
/// * `state` — Current finale state (for `finalecount` and `finaletext`).
/// * `video` — Video state for screen buffer access and patch drawing.
/// * `flat_data` — Raw 4096-byte flat lump data (64×64 pixels) for tiling.
/// * `hu_font` — Array of HU font patch data slices, indexed by
///   `(character - HU_FONTSTART)`.
pub fn f_text_write(
    state: &FinaleState,
    video: &mut VideoState,
    flat_data: &[u8],
    hu_font: &[&[u8]],
) {
    // Tile the flat across the entire screen (screen 0)
    // The flat is 64x64 pixels. We tile it to fill SCREENWIDTH x SCREENHEIGHT.
    // Original C: f_finale.c lines 276-299
    let src = flat_data;
    let screen_size = (SCREENWIDTH * SCREENHEIGHT) as usize;

    // Only tile if we have valid flat data (64*64 = 4096 bytes)
    if src.len() >= 4096 {
        // Ensure screen 0 has enough space
        if video.screens[0].len() >= screen_size {
            for y in 0..SCREENHEIGHT {
                for x in 0..SCREENWIDTH {
                    let src_x = (x & 63) as usize;
                    let src_y = (y & 63) as usize;
                    let dest_idx = (y * SCREENWIDTH + x) as usize;
                    let src_idx = src_y * 64 + src_x;
                    if src_idx < src.len() && dest_idx < video.screens[0].len() {
                        video.screens[0][dest_idx] = src[src_idx];
                    }
                }
            }
        }
    }

    // Determine how many characters to reveal
    // Original C: count = (finalecount - 10) / TEXTSPEED
    let count = (state.finalecount - 10) / TEXTSPEED;
    if count < 0 {
        return; // Not time to start showing text yet
    }
    let count = count as usize;

    // Draw revealed characters
    // Original C: f_finale.c lines 304-327
    let mut cx = 10;
    let mut cy = 10;
    let text_bytes = state.finaletext.as_bytes();

    for (i, &ch) in text_bytes.iter().enumerate() {
        if i >= count {
            break;
        }

        // Handle newline
        if ch == b'\n' {
            cx = 10;
            cy += 11;
            continue;
        }

        // Character index into the HU font
        let c = ch.wrapping_sub(HU_FONTSTART) as usize;
        if c >= HU_FONTSIZE || c >= hu_font.len() {
            // Non-printable or out-of-range character — advance by 4 pixels
            cx += 4;
            continue;
        }

        // Get the font patch width from the patch header
        let patch = hu_font[c];
        if patch.len() < 8 {
            cx += 4;
            continue;
        }
        let w = i16::from_le_bytes([patch[0], patch[1]]) as i32;

        // Don't draw if off-screen right
        if cx + w > SCREENWIDTH {
            break;
        }

        // Draw the character patch
        video.draw_patch(cx, cy, 0, patch);
        cx += w;
    }
}

// ---------------------------------------------------------------------------
// f_start_cast — begin the cast call sequence
// ---------------------------------------------------------------------------

/// Initialize the cast call sequence (DOOM II ending).
///
/// Sets the cast to the first monster and initializes animation state from
/// the monster's see-state. Called when the DOOM II finale text phase
/// finishes on MAP30.
///
/// Original C: `F_StartCast` (f_finale.c lines 377-389).
pub fn f_start_cast(state: &mut FinaleState) {
    // wipegamestate = -1 in C; the caller handles wipe state
    state.finalestage = 2;
    state.castnum = 0;
    state.castdeath = false;
    state.castframes = 0;
    state.castonmelee = false;
    state.castattacking = false;

    // Start with the first cast member's see state
    let mtype = CASTORDER[0].mobj_type as usize;
    state.caststate = MOBJINFO[mtype].seestate;
    state.casttics = STATES[state.caststate as usize].tics;
}

// ---------------------------------------------------------------------------
// f_cast_ticker — advance cast call animation by one tic
// ---------------------------------------------------------------------------

/// Advance the cast call animation by one game tic.
///
/// Handles frame timing, attack-frame transitions, death sequences, and
/// advancing to the next cast member. Sound effects are dispatched based
/// on the current animation state's action function.
///
/// Original C: `F_CastTicker` (f_finale.c lines 395-500).
///
/// # Returns
/// An optional `SfxEnum` for a sound effect that should be played this tic.
fn f_cast_ticker(state: &mut FinaleState) -> Option<SfxEnum> {
    // Decrement remaining tics for current frame
    state.casttics -= 1;
    if state.casttics > 0 {
        return None; // Not time to advance yet
    }

    // Determine sound effect for the current state's action function
    // Original C: f_finale.c lines 404-459 (massive switch statement)
    let st = &STATES[state.caststate as usize];
    let sfx: Option<SfxEnum> = match st.action {
        ActionFnId::A_PosAttack => Some(SfxEnum::sfx_pistol),
        ActionFnId::A_SPosAttack => Some(SfxEnum::sfx_shotgn),
        ActionFnId::A_CPosAttack => Some(SfxEnum::sfx_shotgn),
        ActionFnId::A_VileTarget => Some(SfxEnum::sfx_vilatk),
        ActionFnId::A_SkelWhoosh => Some(SfxEnum::sfx_skeswg),
        ActionFnId::A_SkelFist => Some(SfxEnum::sfx_skepch),
        ActionFnId::A_SkelMissile => Some(SfxEnum::sfx_skeatk),
        ActionFnId::A_FatAttack1 | ActionFnId::A_FatAttack2 | ActionFnId::A_FatAttack3 => {
            Some(SfxEnum::sfx_firsht)
        }
        ActionFnId::A_Scream => {
            let mtype = CASTORDER[state.castnum].mobj_type as usize;
            Some(MOBJINFO[mtype].deathsound)
        }
        ActionFnId::A_TroopAttack => Some(SfxEnum::sfx_claw),
        ActionFnId::A_SargAttack => Some(SfxEnum::sfx_sgtatk),
        ActionFnId::A_HeadAttack => Some(SfxEnum::sfx_firsht),
        ActionFnId::A_BruisAttack => Some(SfxEnum::sfx_firsht),
        ActionFnId::A_BspiAttack => Some(SfxEnum::sfx_plasma),
        ActionFnId::A_CyberAttack => Some(SfxEnum::sfx_rlaunc),
        ActionFnId::A_PainAttack => Some(SfxEnum::sfx_sklatk),
        ActionFnId::A_SkullAttack => Some(SfxEnum::sfx_sklatk),
        ActionFnId::A_SpidRefire | ActionFnId::A_Metal => Some(SfxEnum::sfx_metal),
        ActionFnId::A_BabyMetal => Some(SfxEnum::sfx_bspwlk),
        ActionFnId::A_Hoof => Some(SfxEnum::sfx_hoof),
        _ => None,
    };

    // Advance to the next state
    // Original C: f_finale.c lines 461-500
    if state.castdeath {
        // In death sequence — advance through death states
        let next = STATES[state.caststate as usize].nextstate;
        state.caststate = next;
        state.castframes += 1;

        if next == StateNum::S_NULL {
            // Death animation complete — move to next cast member
            state.castnum += 1;
            state.castdeath = false;

            if state.castnum >= CASTORDER.len() {
                // Wrapped around — restart from first monster
                state.castnum = 0;
            }

            let mtype = CASTORDER[state.castnum].mobj_type as usize;
            state.caststate = MOBJINFO[mtype].seestate;
        }
    } else {
        // Normal (see/attack) animation sequence
        state.castframes += 1;

        // At frame 12, enter attack state
        // Original C: castframes == 12
        if state.castframes == 12 && !state.castattacking {
            state.castattacking = true;
            let mtype = CASTORDER[state.castnum].mobj_type as usize;

            // Alternate between melee and missile attacks
            if state.castonmelee {
                let melee = MOBJINFO[mtype].meleestate;
                if melee != StateNum::S_NULL {
                    state.caststate = melee;
                } else {
                    // No melee — try missile
                    let missile = MOBJINFO[mtype].missilestate;
                    if missile != StateNum::S_NULL {
                        state.caststate = missile;
                    }
                    // If neither exists, just keep going with see state
                }
            } else {
                let missile = MOBJINFO[mtype].missilestate;
                if missile != StateNum::S_NULL {
                    state.caststate = missile;
                } else {
                    // No missile — try melee
                    let melee = MOBJINFO[mtype].meleestate;
                    if melee != StateNum::S_NULL {
                        state.caststate = melee;
                    }
                }
            }
            state.castonmelee = !state.castonmelee;
        }

        // At frame 24, stop attacking and return to see state
        // Original C: castframes == 24 || (castattacking && ...)
        if state.castframes == 24
            || (state.castattacking
                && STATES[state.caststate as usize].nextstate == StateNum::S_NULL)
        {
            // Attack sequence done — return to see state
            state.castattacking = false;
            state.castframes = 0;
            let mtype = CASTORDER[state.castnum].mobj_type as usize;
            state.caststate = MOBJINFO[mtype].seestate;
        } else {
            // Advance to next animation frame
            state.caststate = STATES[state.caststate as usize].nextstate;
        }
    }

    state.casttics = STATES[state.caststate as usize].tics;
    if state.casttics == -1 {
        // -1 means infinite duration; use a reasonable default
        state.casttics = 15;
    }

    sfx
}

// ---------------------------------------------------------------------------
// f_cast_responder — handle input during cast call
// ---------------------------------------------------------------------------

/// Handle keyboard input during the cast call sequence.
///
/// When a key is pressed, the current cast member enters its death sequence.
/// A sound effect is played for the monster's death sound.
///
/// Original C: `F_CastResponder` (f_finale.c lines 502-520).
///
/// # Returns
/// `true` if the event was consumed.
fn f_cast_responder(state: &mut FinaleState, ev: &Event) -> bool {
    if ev.event_type != EventType::KeyDown {
        return false;
    }

    if state.castdeath {
        return true; // Already dying, eat the event
    }

    // Trigger death sequence for current cast member
    state.castdeath = true;
    state.castframes = 0;
    state.castattacking = false;

    let mtype = CASTORDER[state.castnum].mobj_type as usize;
    state.caststate = MOBJINFO[mtype].deathstate;
    state.casttics = STATES[state.caststate as usize].tics;

    // Note: The caller is responsible for playing MOBJINFO[mtype].deathsound
    // (the original C calls S_StartSound here)

    true
}

// ---------------------------------------------------------------------------
// f_cast_print — draw cast member name centered on screen
// ---------------------------------------------------------------------------

/// Draw a cast member's name centered horizontally on the screen.
///
/// Uses the HU font patches to render the text string at the bottom of the
/// screen (y = 180 in the original). The text is first measured to determine
/// its pixel width, then drawn centered.
///
/// Original C: `F_CastPrint` (f_finale.c lines 523-571).
///
/// # Arguments
/// * `video` — Video state for patch drawing.
/// * `hu_font` — Array of HU font patch data slices.
/// * `text` — The name string to draw.
fn f_cast_print(video: &mut VideoState, hu_font: &[&[u8]], text: &str) {
    let text_bytes = text.as_bytes();

    // First pass: measure total width
    let mut width = 0i32;
    for &ch in text_bytes {
        let c = ch.wrapping_sub(HU_FONTSTART) as usize;
        if c >= HU_FONTSIZE || c >= hu_font.len() {
            width += 4;
            continue;
        }
        let patch = hu_font[c];
        if patch.len() < 8 {
            width += 4;
            continue;
        }
        let w = i16::from_le_bytes([patch[0], patch[1]]) as i32;
        width += w;
    }

    // Center horizontally
    let mut cx = (SCREENWIDTH - width) / 2;
    let cy = 180;

    // Second pass: draw characters
    for &ch in text_bytes {
        let c = ch.wrapping_sub(HU_FONTSTART) as usize;
        if c >= HU_FONTSIZE || c >= hu_font.len() {
            cx += 4;
            continue;
        }
        let patch = hu_font[c];
        if patch.len() < 8 {
            cx += 4;
            continue;
        }
        let w = i16::from_le_bytes([patch[0], patch[1]]) as i32;

        video.draw_patch(cx, cy, 0, patch);
        cx += w;
    }
}

// ---------------------------------------------------------------------------
// f_cast_drawer — draw the current cast member
// ---------------------------------------------------------------------------

/// Draw the cast call screen: background, monster sprite, and name text.
///
/// Renders the "BOSSBACK" background patch, the current monster's sprite
/// centered on screen, and the monster's name at the bottom.
///
/// Original C: `F_CastDrawer` (f_finale.c lines 579-607).
///
/// # Arguments
/// * `state` — Current finale state (for cast animation state).
/// * `video` — Video state for drawing operations.
/// * `hu_font` — HU font patches for name text.
/// * `bossback_patch` — Raw patch data for the BOSSBACK lump.
/// * `get_sprite_patch` — Callback to retrieve sprite frame patch data.
///   Takes `(sprite: SpriteNum, frame: i32)` and returns the patch data
///   and whether the sprite should be flipped horizontally.
pub fn f_cast_drawer<F>(
    state: &FinaleState,
    video: &mut VideoState,
    hu_font: &[&[u8]],
    bossback_patch: &[u8],
    mut get_sprite_patch: F,
) where
    F: FnMut(SpriteNum, i32) -> Option<(Vec<u8>, bool)>,
{
    // Draw the background
    video.draw_patch(0, 0, 0, bossback_patch);

    // Get current state info
    let st = &STATES[state.caststate as usize];
    let sprite = st.sprite;
    let frame = st.frame & 0x7FFF; // Mask off FF_FULLBRIGHT

    // Get the sprite patch
    if let Some((patch_data, flip)) = get_sprite_patch(sprite, frame) {
        // Draw sprite centered on screen
        if flip {
            video.draw_patch_flipped(160, 170, 0, &patch_data);
        } else {
            video.draw_patch(160, 170, 0, &patch_data);
        }
    }

    // Draw the cast member name
    if state.castnum < CASTORDER.len() {
        f_cast_print(video, hu_font, CASTORDER[state.castnum].name);
    }
}

// ---------------------------------------------------------------------------
// f_draw_patch_col — column-based patch drawing for bunny scroll
// ---------------------------------------------------------------------------

/// Draw a single column from a patch at a specific screen x coordinate.
///
/// Used by [`f_bunny_scroll`] for the column-by-column scrolling of the
/// PFUB1 and PFUB2 patches.
///
/// Original C: `F_DrawPatchCol` (f_finale.c lines 609-638).
///
/// # Arguments
/// * `video` — Video state for screen buffer access.
/// * `x` — Destination screen x coordinate.
/// * `patch_data` — Raw patch lump data.
/// * `col` — Source column index within the patch.
fn f_draw_patch_col(video: &mut VideoState, x: i32, patch_data: &[u8], col: i32) {
    if patch_data.len() < 8 {
        return;
    }

    // Parse patch header for bounds checking
    let width = i16::from_le_bytes([patch_data[0], patch_data[1]]) as i32;
    if col < 0 || col >= width {
        return;
    }

    // Get column offset from the patch's column offset table
    let col_ofs_pos = (8 + col * 4) as usize;
    if col_ofs_pos + 4 > patch_data.len() {
        return;
    }
    let col_ofs = i32::from_le_bytes([
        patch_data[col_ofs_pos],
        patch_data[col_ofs_pos + 1],
        patch_data[col_ofs_pos + 2],
        patch_data[col_ofs_pos + 3],
    ]) as usize;

    // Walk through the column's post list
    let sw = SCREENWIDTH as usize;
    let mut post_offset = col_ofs;

    loop {
        if post_offset >= patch_data.len() {
            break;
        }

        let topdelta = patch_data[post_offset];
        if topdelta == 0xFF {
            break; // End of column posts
        }

        if post_offset + 1 >= patch_data.len() {
            break;
        }
        let length = patch_data[post_offset + 1] as usize;

        // Source pixel data starts 3 bytes after post header (skip pad byte)
        let source_start = post_offset + 3;
        let dest_start = (topdelta as usize) * sw + (x as usize);

        for i in 0..length {
            if source_start + i < patch_data.len() {
                let dest_idx = dest_start + i * sw;
                if dest_idx < video.screens[0].len() {
                    video.screens[0][dest_idx] = patch_data[source_start + i];
                }
            }
        }

        // Move to next post (length + 4 for header/pad bytes)
        post_offset += length + 4;
    }
}

// ---------------------------------------------------------------------------
// f_bunny_scroll — Episode 3 ending bunny scroll
// ---------------------------------------------------------------------------

/// Draw the Episode 3 bunny scroll screen.
///
/// Horizontally scrolls PFUB1 and PFUB2 patches from right to left. After
/// the scroll completes (finalecount >= 1130), overlays "THE END" patches
/// (END0 through END6) which blink in sequence.
///
/// Original C: `F_BunnyScroll` (f_finale.c lines 644-694).
///
/// # Arguments
/// * `state` — Current finale state (for `finalecount`).
/// * `video` — Video state for screen buffer access and patch drawing.
/// * `pfub1` — Raw patch data for the PFUB1 lump (left bunny image).
/// * `pfub2` — Raw patch data for the PFUB2 lump (right bunny image).
/// * `end_patches` — Array of 7 patch data slices for END0 through END6.
pub fn f_bunny_scroll(
    state: &FinaleState,
    video: &mut VideoState,
    pfub1: &[u8],
    pfub2: &[u8],
    end_patches: &[&[u8]; 7],
) {
    // Calculate scroll position
    // Original C: scrolled = 320 - ((finalecount-230)/2)
    // Clamp to valid range [0, 320]
    let scrolled = {
        let raw = 320 - ((state.finalecount - 230) / 2);
        raw.clamp(0, 320)
    };

    // Draw the two bunny patches column-by-column
    // PFUB2 is on the left (scrolled out), PFUB1 is on the right (scrolled in)
    for x in 0..SCREENWIDTH {
        let source_col = x + scrolled;
        if source_col < 320 {
            // Draw from PFUB2
            f_draw_patch_col(video, x, pfub2, source_col);
        } else {
            // Draw from PFUB1
            f_draw_patch_col(video, x, pfub1, source_col - 320);
        }
    }

    // Overlay "THE END" after scroll completes
    // Original C: if (finalecount >= 1130) — first END0, then staged END0-END6
    if state.finalecount >= 1130 {
        let stage = if state.finalecount < 1180 {
            0 // Just show END0
        } else {
            let s = (state.finalecount - 1180) / 5;
            if s > 6 {
                6
            } else {
                s
            }
        };

        let stage_idx = stage as usize;
        if stage_idx < end_patches.len() {
            video.draw_patch(
                (SCREENWIDTH - 13 * 8) / 2,
                (SCREENHEIGHT - 8 * 8) / 2,
                0,
                end_patches[stage_idx],
            );
        }
    }
}

// ---------------------------------------------------------------------------
// f_drawer — main finale drawing dispatch
// ---------------------------------------------------------------------------

/// Main drawing dispatch for the finale screen.
///
/// Routes to the appropriate drawing function based on the current
/// `finalestage`:
/// - Stage 0: text scroll ([`f_text_write`])
/// - Stage 1: art screen (episode-specific) or bunny scroll
/// - Stage 2: cast call ([`f_cast_drawer`])
///
/// For stage 1 art screens, the appropriate patch is drawn depending on
/// the game mode and episode:
/// - Episode 1: CREDIT (registered/retail) or HELP2 (shareware)
/// - Episode 2: VICTORY2
/// - Episode 3: Bunny scroll (PFUB1/PFUB2/END0-END6)
/// - Episode 4: ENDPIC
/// - DOOM II / Commercial: BOSSBACK (handled by cast call in stage 2)
///
/// Original C: `F_Drawer` (f_finale.c lines 700-737).
pub fn f_drawer<F>(
    state: &FinaleState,
    video: &mut VideoState,
    gamemode: GameMode,
    gameepisode: i32,
    flat_data: &[u8],
    hu_font: &[&[u8]],
    art_patch: &[u8],
    bunny_data: Option<(&[u8], &[u8], &[&[u8]; 7])>,
    bossback_patch: &[u8],
    get_sprite_patch: F,
) where
    F: FnMut(SpriteNum, i32) -> Option<(Vec<u8>, bool)>,
{
    match state.finalestage {
        2 => {
            // Cast call
            f_cast_drawer(state, video, hu_font, bossback_patch, get_sprite_patch);
        }
        0 => {
            // Text scroll
            f_text_write(state, video, flat_data, hu_font);
        }
        1 => {
            // Art screen — depends on game mode and episode
            match gamemode {
                GameMode::Shareware | GameMode::Registered | GameMode::Retail => {
                    match gameepisode {
                        3 => {
                            // Bunny scroll
                            if let Some((pfub1, pfub2, end_patches)) = bunny_data {
                                f_bunny_scroll(state, video, pfub1, pfub2, end_patches);
                            }
                        }
                        _ => {
                            // Art patch (CREDIT, VICTORY2, ENDPIC, HELP2, etc.)
                            // The caller passes the appropriate patch
                            video.draw_patch(0, 0, 0, art_patch);
                        }
                    }
                }
                GameMode::Commercial => {
                    // DOOM II stage 1 should not normally be reached — the
                    // finale goes directly from text (stage 0) to cast call
                    // (stage 2). If it does, just show the text.
                    f_text_write(state, video, flat_data, hu_font);
                }
                _ => {
                    // Fallback — show art patch
                    video.draw_patch(0, 0, 0, art_patch);
                }
            }
        }
        _ => {
            // Unknown stage — do nothing
        }
    }
}
