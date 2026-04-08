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

//! Heads-up display management: messages, chat, titles.
//!
//! Translated from linuxdoom-1.10/hu_stuff.c and linuxdoom-1.10/hu_stuff.h.
//!
//! Manages on-screen HUD messages, multiplayer chat input, level title
//! display, and the HUD font. This is the high-level HUD system that uses
//! `hud_lib.rs` widget primitives for actual text rendering.
//!
//! # Architecture
//!
//! All formerly-global mutable state is encapsulated in [`HudState`], which
//! is passed by mutable reference to every public function. This eliminates
//! `static mut` usage and enables the Rust borrow checker to enforce safe
//! state access patterns.
//!
//! # Original C Source Reference
//!
//! - `hu_stuff.c` (759 lines): HU_Init, HU_Start, HU_Responder, HU_Ticker,
//!   HU_Drawer, HU_Erase, HU_queueChatChar, HU_dequeueChatChar
//! - `hu_stuff.h` (67 lines): Constants HU_FONTSTART through HU_MSGTIMEOUT,
//!   function declarations

use crate::game::strings;
use crate::info::sounds::SfxEnum;
use crate::types::doomdef::{
    GameMission, GameMode, Language, KEY_BACKSPACE, KEY_ENTER, KEY_RALT, KEY_RSHIFT, MAXPLAYERS,
    TICRATE,
};
use crate::types::event::{Event, EventType};
use crate::types::player::Player;
use crate::ui::hud_lib::{
    hulib_add_char_to_text_line, hulib_add_message_to_stext, hulib_draw_itext, hulib_draw_stext,
    hulib_draw_text_line, hulib_erase_itext, hulib_erase_stext, hulib_erase_text_line,
    hulib_init_itext, hulib_init_stext, hulib_init_text_line, hulib_key_in_itext,
    hulib_reset_itext, HuInputText, HuScrollText, HuTextLine,
};
use crate::util::swap::short;
use crate::video::video::VideoState;

// doom_wad::PurgeTag is needed for WAD lump cache tag when loading font patches.
use doom_wad::PurgeTag;

// WadProvider trait re-exported from doom-wad via crate::traits::wad.
use crate::traits::wad::WadProvider;

// =============================================================================
// Public constants (from hu_stuff.h)
// =============================================================================

/// First printable character in the HUD font: ASCII '!' (33).
///
/// Font patches in the WAD are named `STCFN033` through `STCFN095`.
pub const HU_FONTSTART: u8 = b'!';

/// Last printable character in the HUD font: ASCII '_' (95).
pub const HU_FONTEND: u8 = b'_';

/// Number of characters in the HUD font: 95 - 33 + 1 = 63.
pub const HU_FONTSIZE: usize = (HU_FONTEND - HU_FONTSTART + 1) as usize;

/// Broadcast destination for chat messages (sent to all players).
///
/// When a chat destination byte is set to `HU_BROADCAST`, the message is
/// visible to all connected players. Player-specific destinations use
/// values 1 through `MAXPLAYERS`.
pub const HU_BROADCAST: i32 = 5;

/// X coordinate for message display (left edge of screen).
pub const HU_MSGX: i32 = 0;

/// Y coordinate for message display (top of screen).
pub const HU_MSGY: i32 = 0;

/// Maximum width of HUD message text in characters.
pub const HU_MSGWIDTH: i32 = 64;

/// Number of message lines displayed simultaneously.
pub const HU_MSGHEIGHT: i32 = 1;

/// Message display timeout in tics: 4 seconds at 35 tics/second = 140 tics.
pub const HU_MSGTIMEOUT: i32 = 4 * TICRATE;

// =============================================================================
// Local constants (from hu_stuff.c)
// =============================================================================

/// Height of the title text display (1 line).
/// Preserved from original C source for reference. Not used directly in Rust
/// code but documents the original design intent.
#[allow(dead_code)]
const HU_TITLEHEIGHT: i32 = 1;

/// X coordinate for the level title display.
const HU_TITLEX: i32 = 0;

/// Key to toggle chat input mode: 't'.
const HU_INPUTTOGGLE: u8 = b't';

/// Width of the chat input widget in characters.
/// Preserved from original C source for reference.
#[allow(dead_code)]
const HU_INPUTWIDTH: i32 = 64;

/// Height of the chat input widget in lines.
const _HU_INPUTHEIGHT: i32 = 1;

/// Size of the chat character ring buffer.
const QUEUESIZE: usize = 128;

/// Message refresh key: Enter key triggers message redisplay.
const HU_MSGREFRESH: i32 = KEY_ENTER;

// =============================================================================
// Static data: shift key transformation tables
// =============================================================================

/// English keyboard shift transformation table.
///
/// Maps ASCII characters to their shifted equivalents when the Shift key is
/// held. For example, 'a' (97) -> 'A' (65), '1' (49) -> '!' (33).
/// Characters 0-31 map to themselves (control characters).
const ENGLISH_SHIFTXFORM: [u8; 128] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, b' ', b'!', b'"', b'#', b'$', b'%', b'&', b'"', // shift-'
    b'(', b')', b'*', b'+', b'<', // shift-,
    b'_', // shift--
    b'>', // shift-.
    b'?', // shift-/
    b')', // shift-0
    b'!', // shift-1
    b'@', // shift-2
    b'#', // shift-3
    b'$', // shift-4
    b'%', // shift-5
    b'^', // shift-6
    b'&', // shift-7
    b'*', // shift-8
    b'(', // shift-9
    b':', b':', // shift-;
    b'<', b'+', // shift-=
    b'>', b'?', b'@', b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L', b'M',
    b'N', b'O', b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X', b'Y', b'Z', b'[', b'!', b']',
    b'^', b'_', b'`', b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L', b'M',
    b'N', b'O', b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X', b'Y', b'Z', b'{', b'|', b'}',
    b'~', 127,
];

/// French keyboard shift transformation table.
///
/// Maps ASCII characters to their shifted equivalents for French (AZERTY)
/// keyboard layout. Differs from English primarily in number row mappings.
const FRENCH_SHIFTXFORM: [u8; 128] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, b' ', b'!', b'"', b'#', b'$', b'%', b'&', b'"', // shift-'
    b'(', b')', b'*', b'+', b'?', // shift-,
    b'_', // shift--
    b'>', // shift-.
    b'?', // shift-/
    b'0', // shift-0
    b'1', // shift-1
    b'2', // shift-2
    b'3', // shift-3
    b'4', // shift-4
    b'5', // shift-5
    b'6', // shift-6
    b'7', // shift-7
    b'8', // shift-8
    b'9', // shift-9
    b'/', b'.', // shift-;
    b'<', b'+', // shift-=
    b'>', b'?', b'@', b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L', b'M',
    b'N', b'O', b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X', b'Y', b'Z', b'[', b'!', b']',
    b'^', b'_', b'`', b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L', b'M',
    b'N', b'O', b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X', b'Y', b'Z', b'{', b'|', b'}',
    b'~', 127,
];

/// French keyboard character translation map (AZERTY -> QWERTY logical mapping).
///
/// Used when `language == French` to translate physical key positions to the
/// expected logical characters for the French AZERTY layout.
const FRENCH_KEY_MAP: [u8; 128] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, b' ', b'!', b'"', b'#', b'$', b'%', b'&', b'%', b'(', b')', b'*', b'+',
    b';', b'-', b':', b'!', b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b':', b'M',
    b'<', b'=', b'>', b'?', b'@', b'Q', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K',
    b'L', b',', b'N', b'O', b'P', b'A', b'R', b'S', b'T', b'U', b'V', b'Z', b'X', b'Y', b'W', b'^',
    b'\\', b'$', b'^', 95, b'@', b'Q', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K',
    b'L', b',', b'N', b'O', b'P', b'A', b'R', b'S', b'T', b'U', b'V', b'Z', b'X', b'Y', b'W', b'^',
    b'\\', b'$', b'^', 127,
];

// =============================================================================
// Static data: map name arrays
// =============================================================================

/// Map names for DOOM 1 shareware/registered/retail (episodes 1-4, maps 1-9).
///
/// Indexed as `[(episode - 1) * 9 + (map - 1)]`. The trailing 9 "NEWLEVEL"
/// entries are safety padding for out-of-range access.
const MAPNAMES: [&str; 45] = [
    // Episode 1
    strings::HUSTR_E1M1,
    strings::HUSTR_E1M2,
    strings::HUSTR_E1M3,
    strings::HUSTR_E1M4,
    strings::HUSTR_E1M5,
    strings::HUSTR_E1M6,
    strings::HUSTR_E1M7,
    strings::HUSTR_E1M8,
    strings::HUSTR_E1M9,
    // Episode 2
    strings::HUSTR_E2M1,
    strings::HUSTR_E2M2,
    strings::HUSTR_E2M3,
    strings::HUSTR_E2M4,
    strings::HUSTR_E2M5,
    strings::HUSTR_E2M6,
    strings::HUSTR_E2M7,
    strings::HUSTR_E2M8,
    strings::HUSTR_E2M9,
    // Episode 3
    strings::HUSTR_E3M1,
    strings::HUSTR_E3M2,
    strings::HUSTR_E3M3,
    strings::HUSTR_E3M4,
    strings::HUSTR_E3M5,
    strings::HUSTR_E3M6,
    strings::HUSTR_E3M7,
    strings::HUSTR_E3M8,
    strings::HUSTR_E3M9,
    // Episode 4
    strings::HUSTR_E4M1,
    strings::HUSTR_E4M2,
    strings::HUSTR_E4M3,
    strings::HUSTR_E4M4,
    strings::HUSTR_E4M5,
    strings::HUSTR_E4M6,
    strings::HUSTR_E4M7,
    strings::HUSTR_E4M8,
    strings::HUSTR_E4M9,
    // Safety padding for out-of-range episodes
    "NEWLEVEL",
    "NEWLEVEL",
    "NEWLEVEL",
    "NEWLEVEL",
    "NEWLEVEL",
    "NEWLEVEL",
    "NEWLEVEL",
    "NEWLEVEL",
    "NEWLEVEL",
];

/// Map names for DOOM II (maps 1-32).
const MAPNAMES2: [&str; 32] = [
    strings::HUSTR_1,
    strings::HUSTR_2,
    strings::HUSTR_3,
    strings::HUSTR_4,
    strings::HUSTR_5,
    strings::HUSTR_6,
    strings::HUSTR_7,
    strings::HUSTR_8,
    strings::HUSTR_9,
    strings::HUSTR_10,
    strings::HUSTR_11,
    strings::HUSTR_12,
    strings::HUSTR_13,
    strings::HUSTR_14,
    strings::HUSTR_15,
    strings::HUSTR_16,
    strings::HUSTR_17,
    strings::HUSTR_18,
    strings::HUSTR_19,
    strings::HUSTR_20,
    strings::HUSTR_21,
    strings::HUSTR_22,
    strings::HUSTR_23,
    strings::HUSTR_24,
    strings::HUSTR_25,
    strings::HUSTR_26,
    strings::HUSTR_27,
    strings::HUSTR_28,
    strings::HUSTR_29,
    strings::HUSTR_30,
    strings::HUSTR_31,
    strings::HUSTR_32,
];

/// Map names for Final DOOM: The Plutonia Experiment (maps 1-32).
const MAPNAMESP: [&str; 32] = [
    strings::PHUSTR_1,
    strings::PHUSTR_2,
    strings::PHUSTR_3,
    strings::PHUSTR_4,
    strings::PHUSTR_5,
    strings::PHUSTR_6,
    strings::PHUSTR_7,
    strings::PHUSTR_8,
    strings::PHUSTR_9,
    strings::PHUSTR_10,
    strings::PHUSTR_11,
    strings::PHUSTR_12,
    strings::PHUSTR_13,
    strings::PHUSTR_14,
    strings::PHUSTR_15,
    strings::PHUSTR_16,
    strings::PHUSTR_17,
    strings::PHUSTR_18,
    strings::PHUSTR_19,
    strings::PHUSTR_20,
    strings::PHUSTR_21,
    strings::PHUSTR_22,
    strings::PHUSTR_23,
    strings::PHUSTR_24,
    strings::PHUSTR_25,
    strings::PHUSTR_26,
    strings::PHUSTR_27,
    strings::PHUSTR_28,
    strings::PHUSTR_29,
    strings::PHUSTR_30,
    strings::PHUSTR_31,
    strings::PHUSTR_32,
];

/// Map names for Final DOOM: TNT Evilution (maps 1-32).
const MAPNAMEST: [&str; 32] = [
    strings::THUSTR_1,
    strings::THUSTR_2,
    strings::THUSTR_3,
    strings::THUSTR_4,
    strings::THUSTR_5,
    strings::THUSTR_6,
    strings::THUSTR_7,
    strings::THUSTR_8,
    strings::THUSTR_9,
    strings::THUSTR_10,
    strings::THUSTR_11,
    strings::THUSTR_12,
    strings::THUSTR_13,
    strings::THUSTR_14,
    strings::THUSTR_15,
    strings::THUSTR_16,
    strings::THUSTR_17,
    strings::THUSTR_18,
    strings::THUSTR_19,
    strings::THUSTR_20,
    strings::THUSTR_21,
    strings::THUSTR_22,
    strings::THUSTR_23,
    strings::THUSTR_24,
    strings::THUSTR_25,
    strings::THUSTR_26,
    strings::THUSTR_27,
    strings::THUSTR_28,
    strings::THUSTR_29,
    strings::THUSTR_30,
    strings::THUSTR_31,
    strings::THUSTR_32,
];

/// Player color name prefixes for chat messages.
///
/// Indexed by player number (0 = Green, 1 = Indigo, 2 = Brown, 3 = Red).
const PLAYER_NAMES: [&str; 4] = [
    strings::HUSTR_PLRGREEN,
    strings::HUSTR_PLRINDIGO,
    strings::HUSTR_PLRBROWN,
    strings::HUSTR_PLRRED,
];

/// Default chat macro strings for Alt+0 through Alt+9.
const DEFAULT_CHAT_MACROS: [&str; 10] = [
    strings::HUSTR_CHATMACRO0,
    strings::HUSTR_CHATMACRO1,
    strings::HUSTR_CHATMACRO2,
    strings::HUSTR_CHATMACRO3,
    strings::HUSTR_CHATMACRO4,
    strings::HUSTR_CHATMACRO5,
    strings::HUSTR_CHATMACRO6,
    strings::HUSTR_CHATMACRO7,
    strings::HUSTR_CHATMACRO8,
    strings::HUSTR_CHATMACRO9,
];

/// Chat destination keys for each player slot.
///
/// Pressing one of these keys in multiplayer starts a directed chat to that
/// player. The keys correspond to player color initials:
/// 'g' (Green), 'i' (Indigo), 'b' (Brown), 'r' (Red).
const DESTINATION_KEYS: [char; 4] = [
    strings::HUSTR_KEYGREEN,
    strings::HUSTR_KEYINDIGO,
    strings::HUSTR_KEYBROWN,
    strings::HUSTR_KEYRED,
];

/// "Talk to self" messages cycled through when chatting in single-player.
const TALK_TO_SELF: [&str; 5] = [
    strings::HUSTR_TALKTOSELF1,
    strings::HUSTR_TALKTOSELF2,
    strings::HUSTR_TALKTOSELF3,
    strings::HUSTR_TALKTOSELF4,
    strings::HUSTR_TALKTOSELF5,
];

// =============================================================================
// HUD State Struct
// =============================================================================

/// Encapsulates all formerly-global HUD state.
///
/// In the original C code, these were file-scoped global variables in
/// `hu_stuff.c`. Here they are collected into a single struct passed by
/// reference to every HUD function, following the AAP §0.7.5 mandate to
/// eliminate `static mut` usage.
pub struct HudState {
    // -- Font data --
    /// HUD font patches loaded from WAD lumps `STCFN033` through `STCFN095`.
    ///
    /// Each entry is raw patch data bytes. Characters outside the printable
    /// range have empty Vecs.
    pub hu_font: Vec<Vec<u8>>,

    // -- Title widget --
    /// Level title text line widget (displays current map name).
    pub w_title: HuTextLine,

    // -- Message display --
    /// Scrolling message widget for player messages (e.g., "Picked up a shotgun").
    pub w_message: HuScrollText,

    /// Whether a message is currently being displayed.
    pub message_on: bool,

    /// When set, the current message overrides the "Messages OFF" setting.
    ///
    /// Preserves the original variable name semantics — this is part of
    /// DOOM's historical character.
    pub message_dontfuckwithme: bool,

    /// When set, the current message cannot be overwritten by lower-priority
    /// messages (e.g., cheat confirmation persists over pickup messages).
    pub message_nottobefuckedwith: bool,

    /// Message display countdown timer in tics. When it reaches 0, the
    /// message is hidden.
    pub message_counter: i32,

    // -- Chat system --
    /// Whether chat input mode is currently active.
    pub chat_on: bool,

    /// Chat input widget for the local player.
    pub w_chat: HuInputText,

    /// Constant false value — the chat widget is never drawn in the original
    /// engine (only the message widget shows chat). Faithfully preserves
    /// the original behavior where `w_chat.on` is set to `&always_off`.
    pub always_off: bool,

    /// Chat destination for each player slot (0 = no chat, HU_BROADCAST = all,
    /// 1..MAXPLAYERS = specific player).
    pub chat_dest: [u8; MAXPLAYERS],

    /// Per-player chat input buffers for receiving chat from other players.
    pub w_inputbuffer: Vec<HuInputText>,

    /// Current chat character being queued for transmission.
    pub chat_char: u8,

    // -- Display player --
    /// Index of the currently displayed player (used for message display).
    pub plr: usize,

    // -- Keyboard mapping --
    /// Shift key character transformation table (128 entries).
    ///
    /// Selected at init time based on language setting (English or French).
    pub shiftxform: [u8; 128],

    /// Chat macro strings (Alt+0 through Alt+9).
    pub chat_macros: [&'static str; 10],

    // -- Internal/private state --
    /// Chat character ring buffer for outgoing characters.
    chatchars: [u8; QUEUESIZE],

    /// Ring buffer write pointer (next position to write).
    head: usize,

    /// Ring buffer read pointer (next position to read).
    tail: usize,

    /// Height of HUD font in pixels (read from first font patch).
    font_height: i32,

    /// Whether the Shift key is currently pressed.
    shift_down: bool,

    /// Whether the Alt key is currently pressed.
    alt_down: bool,

    /// Counter for cycling "talk to self" messages.
    num_nobrainers: usize,

    /// Whether the current display player has the message display enabled.
    /// Mirrors the external `showMessages` config variable.
    show_messages: bool,
}

impl Default for HudState {
    fn default() -> Self {
        Self::new()
    }
}

impl HudState {
    /// Creates a new `HudState` with all fields initialized to defaults.
    pub fn new() -> Self {
        let mut w_inputbuffer = Vec::with_capacity(MAXPLAYERS);
        for _ in 0..MAXPLAYERS {
            w_inputbuffer.push(HuInputText::default());
        }

        Self {
            hu_font: Vec::new(),
            w_title: HuTextLine::default(),
            w_message: HuScrollText::default(),
            message_on: false,
            message_dontfuckwithme: false,
            message_nottobefuckedwith: false,
            message_counter: 0,
            chat_on: false,
            w_chat: HuInputText::default(),
            always_off: false,
            chat_dest: [0u8; MAXPLAYERS],
            w_inputbuffer,
            chat_char: 0,
            plr: 0,
            shiftxform: ENGLISH_SHIFTXFORM,
            chat_macros: DEFAULT_CHAT_MACROS,
            chatchars: [0u8; QUEUESIZE],
            head: 0,
            tail: 0,
            font_height: 0,
            shift_down: false,
            alt_down: false,
            num_nobrainers: 0,
            show_messages: true,
        }
    }
}

// =============================================================================
// Chat character ring buffer (from hu_stuff.c lines 580-615)
// =============================================================================

/// Queue a chat character for transmission to other players.
///
/// Characters are stored in a ring buffer and dequeued by the network
/// layer via [`hu_dequeue_chat_char`].
fn hu_queue_chat_char(state: &mut HudState, ch: u8) {
    let next = (state.head + 1) & (QUEUESIZE - 1);
    if next == state.tail {
        // Queue is full — drop the character (original behavior).
        return;
    }
    state.chatchars[state.head] = ch;
    state.head = next;
}

/// Dequeue the next chat character from the ring buffer.
///
/// Returns 0 if the queue is empty (original `HU_dequeueChatChar` behavior).
pub fn hu_dequeue_chat_char(state: &mut HudState) -> u8 {
    if state.head == state.tail {
        return 0;
    }
    let ch = state.chatchars[state.tail];
    state.tail = (state.tail + 1) & (QUEUESIZE - 1);
    ch
}

// =============================================================================
// Map title lookup (HU_TITLE macro equivalent)
// =============================================================================

/// Returns the display name for the current map.
///
/// This is the Rust equivalent of the `HU_TITLE` / `HU_TITLE2` / `HU_TITLEP` /
/// `HU_TITLET` macros from `hu_stuff.h`.
///
/// # Arguments
///
/// * `episode` — Current episode number (1-based, relevant for DOOM 1 only).
/// * `map` — Current map number (1-based).
/// * `mode` — Current game mode (determines which name array to use).
/// * `mission` — Current game mission (distinguishes Plutonia/TNT from DOOM 2).
///
/// # Returns
///
/// A static string reference containing the map display name.
pub fn get_map_title(episode: i32, map: i32, mode: GameMode, mission: GameMission) -> &'static str {
    match mode {
        GameMode::Commercial => match mission {
            GameMission::PackPlut => {
                let idx = ((map - 1) as usize).min(MAPNAMESP.len() - 1);
                MAPNAMESP[idx]
            }
            GameMission::PackTnt => {
                let idx = ((map - 1) as usize).min(MAPNAMEST.len() - 1);
                MAPNAMEST[idx]
            }
            _ => {
                // DOOM II and any other commercial mission
                let idx = ((map - 1) as usize).min(MAPNAMES2.len() - 1);
                MAPNAMES2[idx]
            }
        },
        _ => {
            // Shareware, registered, or retail (episode-based)
            let idx = (((episode - 1) * 9 + (map - 1)) as usize).min(MAPNAMES.len() - 1);
            MAPNAMES[idx]
        }
    }
}

// =============================================================================
// French keyboard translation
// =============================================================================

/// Translate a key code through the French AZERTY key map.
///
/// Equivalent to `ForeignTranslation()` in hu_stuff.c (line 386).
fn foreign_translation(ch: u8) -> u8 {
    if (ch as usize) < FRENCH_KEY_MAP.len() {
        FRENCH_KEY_MAP[ch as usize]
    } else {
        ch
    }
}

// =============================================================================
// Core HUD functions
// =============================================================================

/// Initialize the HUD system: load font patches from the WAD file.
///
/// Equivalent to `HU_Init()` in hu_stuff.c (line 392). Loads font patches
/// `STCFN033` through `STCFN095` from the WAD and selects the keyboard
/// shift transformation table based on language setting.
///
/// # Arguments
///
/// * `state` — Mutable reference to the HUD state to initialize.
/// * `wad` — WAD file provider for loading font lump data.
/// * `language` — Current language setting (English or French).
pub fn hu_init(state: &mut HudState, wad: &mut dyn WadProvider, language: Language) {
    // Load HUD font patches (STCFN033 through STCFN095).
    let mut font_patches: Vec<Vec<u8>> = Vec::with_capacity(HU_FONTSIZE);
    for i in 0..HU_FONTSIZE {
        let charnum = HU_FONTSTART as usize + i;
        let lump_name = format!("STCFN{:03}", charnum);
        let data = wad.cache_lump_name(&lump_name, PurgeTag::Static);
        font_patches.push(data.to_vec());
    }

    // Determine font height from first patch (read i16 at offset 2 = height field).
    if !font_patches.is_empty() && font_patches[0].len() >= 4 {
        let raw = &font_patches[0];
        let h = short(i16::from_le_bytes([raw[2], raw[3]]));
        state.font_height = h as i32;
    } else {
        state.font_height = 8; // Fallback default height
    }

    state.hu_font = font_patches;

    // Select shift transformation table based on language.
    match language {
        Language::French => {
            state.shiftxform = FRENCH_SHIFTXFORM;
        }
        _ => {
            state.shiftxform = ENGLISH_SHIFTXFORM;
        }
    }

    // Initialize chat macros with defaults.
    state.chat_macros = DEFAULT_CHAT_MACROS;
}

/// Set up the HUD for the current level.
///
/// Equivalent to `HU_Start()` in hu_stuff.c (line 434). Called at the
/// beginning of each level to reset chat state, set the level title, and
/// reinitialize all HUD widgets.
///
/// # Arguments
///
/// * `state` — Mutable reference to the HUD state.
/// * `consoleplayer` — Index of the local player (0-based).
/// * `episode` — Current episode number (1-based).
/// * `map` — Current map number (1-based).
/// * `mode` — Current game mode.
/// * `mission` — Current game mission.
/// * `playeringame` — Array indicating which player slots are active.
/// * `show_messages` — Whether the player has message display enabled.
pub fn hu_start(
    state: &mut HudState,
    consoleplayer: usize,
    episode: i32,
    map: i32,
    mode: GameMode,
    mission: GameMission,
    playeringame: &[bool],
    show_messages: bool,
) {
    state.plr = consoleplayer;
    state.show_messages = show_messages;

    // Reset message state.
    state.message_on = false;
    state.message_dontfuckwithme = false;
    state.message_nottobefuckedwith = false;
    state.message_counter = 0;
    state.chat_on = false;

    // Compute dynamic positioning based on font height.
    let title_y: i32 = 167 - state.font_height;
    let msg_y: i32 = HU_MSGY;
    let input_x: i32 = HU_MSGX;
    let input_y: i32 = HU_MSGY + (HU_MSGHEIGHT * (state.font_height + 1));

    // Initialize the message scroll widget.
    hulib_init_stext(
        &mut state.w_message,
        HU_MSGX,
        msg_y,
        HU_MSGHEIGHT,
        &state.hu_font,
        HU_FONTSTART as i32,
        true, // message_on starts as true conceptually
    );
    state.message_on = false; // but we start with it off until a message arrives

    // Initialize the local player chat input widget.
    // In the original code, `w_chat.on` is set to `&always_off` — the chat
    // widget is never actually drawn (only the message widget shows chat).
    hulib_init_itext(
        &mut state.w_chat,
        input_x,
        input_y,
        &state.hu_font,
        HU_FONTSTART as i32,
        false, // always_off — chat input widget never drawn
    );

    // Initialize per-player input buffers.
    let max_p = playeringame.len().min(MAXPLAYERS);
    for i in 0..max_p {
        state.chat_dest[i] = 0;
        hulib_init_itext(
            &mut state.w_inputbuffer[i],
            0,
            0,
            &state.hu_font,
            HU_FONTSTART as i32,
            false,
        );
    }

    // Initialize the level title text line.
    hulib_init_text_line(
        &mut state.w_title,
        HU_TITLEX,
        title_y,
        &state.hu_font,
        HU_FONTSTART as i32,
    );

    // Add the map title string to the title widget character by character.
    let title_str = get_map_title(episode, map, mode, mission);
    for ch in title_str.bytes() {
        hulib_add_char_to_text_line(&mut state.w_title, ch);
    }
}

/// Draw the HUD overlay.
///
/// Equivalent to `HU_Drawer()` in hu_stuff.c (line 486). Renders the level
/// title, the scrolling message text, and the chat input line (if active).
///
/// # Arguments
///
/// * `state` — Reference to the HUD state.
/// * `video` — Mutable reference to the video state for drawing.
/// * `automapactive` — Whether the automap is currently displayed.
pub fn hu_drawer(state: &mut HudState, video: &mut VideoState, automapactive: bool) {
    // The message widget's `on` field must be synced from our state.
    state.w_message.on = state.message_on;
    hulib_draw_stext(&state.w_message, video);

    // The chat input widget is drawn only when chat mode is active.
    // In the original code, `w_chat.on` is `always_off` (false), so
    // the chat widget is not drawn by `HUlib_drawIText`. Instead, chat
    // text is transmitted via the message system. We set it here for
    // faithfulness.
    state.w_chat.on = state.chat_on;
    hulib_draw_itext(&state.w_chat, video);

    // Draw the level title when automap is active.
    if automapactive {
        hulib_draw_text_line(&state.w_title, false, video);
    }
}

/// Erase HUD text areas (called during screen refresh).
///
/// Equivalent to `HU_Erase()` in hu_stuff.c (line 498). Erases the
/// message text, chat input, and level title areas by restoring the
/// background pixels.
///
/// # Arguments
///
/// * `state` — Mutable reference to the HUD state.
/// * `video` — Mutable reference to the video state for erasing.
/// * `automapactive` — Whether the automap is currently displayed.
/// * `viewwindowx` — Left edge of the 3D view window in pixels.
/// * `viewwindowy` — Top edge of the 3D view window in pixels.
/// * `viewwidth` — Width of the 3D view window in pixels.
/// * `viewheight` — Height of the 3D view window in pixels.
pub fn hu_erase(
    state: &mut HudState,
    video: &mut VideoState,
    automapactive: bool,
    viewwindowx: i32,
    viewwindowy: i32,
    viewwidth: i32,
    viewheight: i32,
) {
    // Sync on state before erasing.
    state.w_message.on = state.message_on;
    hulib_erase_stext(
        &mut state.w_message,
        video,
        automapactive,
        viewwindowx,
        viewwindowy,
        viewwidth,
        viewheight,
    );

    state.w_chat.on = state.chat_on;
    hulib_erase_itext(
        &mut state.w_chat,
        video,
        automapactive,
        viewwindowx,
        viewwindowy,
        viewwidth,
        viewheight,
    );

    hulib_erase_text_line(
        &mut state.w_title,
        video,
        automapactive,
        viewwindowx,
        viewwindowy,
        viewwidth,
        viewheight,
    );
}

/// Process HUD logic per game tic.
///
/// Equivalent to `HU_Ticker()` in hu_stuff.c (line 505). Processes incoming
/// player messages (pickup notifications, cheat confirmations), handles
/// chat character relay from other players, and manages the message display
/// countdown timer.
///
/// # Arguments
///
/// * `state` — Mutable reference to the HUD state.
/// * `players` — Mutable reference to the player state array.
/// * `consoleplayer` — Index of the local player.
/// * `playeringame` — Array indicating which player slots are active.
/// * `netgame` — Whether this is a network multiplayer game.
///
/// # Returns
///
/// A vector of sound effects to be played by the caller (avoids coupling
/// to the sound system). Possible entries:
/// - `SfxEnum::sfx_tink` — A chat character was received.
/// - `SfxEnum::sfx_radio` — A complete chat message was received.
pub fn hu_ticker(
    state: &mut HudState,
    players: &mut [Player],
    consoleplayer: usize,
    playeringame: &[bool],
    netgame: bool,
) -> Vec<SfxEnum> {
    let mut sounds: Vec<SfxEnum> = Vec::new();

    // Decrement the message display counter.
    if state.message_counter > 0 {
        state.message_counter -= 1;
        if state.message_counter == 0 {
            state.message_on = false;
            // Reset message priority flags when message times out.
            state.message_nottobefuckedwith = false;
        }
    }

    // Check if the display player has a new message to show.
    let display_plr = state.plr;
    if display_plr < players.len() {
        // Take the message from the player if present.
        let msg = players[display_plr].message.take();
        if let Some(ref message_text) = msg {
            if !message_text.is_empty() {
                // Priority logic: if `message_nottobefuckedwith` is set,
                // only `message_dontfuckwithme` can override.
                if state.message_nottobefuckedwith && !state.message_dontfuckwithme {
                    // Current critical message takes priority — ignore new message.
                } else {
                    hulib_add_message_to_stext(&mut state.w_message, None, message_text);
                    state.message_on = true;
                    state.message_counter = HU_MSGTIMEOUT;
                    // If the new message has the "dontfuckwithme" flag set,
                    // mark it as critical (not to be fucked with).
                    if state.message_dontfuckwithme {
                        state.message_nottobefuckedwith = true;
                        state.message_dontfuckwithme = false;
                    }
                }
            }
        }
    }

    // Process incoming chat characters from other players in network games.
    if netgame {
        let max_p = playeringame.len().min(MAXPLAYERS);
        for i in 0..max_p {
            if !playeringame[i] {
                continue;
            }
            // Read the chat character from this player's tic command.
            let ch = if i < players.len() {
                players[i].cmd.chatchar
            } else {
                0
            };
            if ch != 0 {
                // Clear the chat character so we don't process it again.
                if i < players.len() {
                    players[i].cmd.chatchar = 0;
                }

                if (ch as i32) <= HU_BROADCAST {
                    // This is a destination-setting character.
                    state.chat_dest[i] = ch;
                } else {
                    // This is actual chat text. Feed it to the player's input buffer.
                    if i != consoleplayer
                        && (state.chat_dest[i] == consoleplayer as u8 + 1
                            || state.chat_dest[i] == HU_BROADCAST as u8)
                    {
                        let rc = if i < state.w_inputbuffer.len() {
                            hulib_key_in_itext(&mut state.w_inputbuffer[i], ch)
                        } else {
                            false
                        };

                        if rc && ch as i32 == KEY_ENTER {
                            // Complete message received — display it with player name prefix.
                            // Build the chat message from the input buffer.
                            let msg_bytes: Vec<u8> = if i < state.w_inputbuffer.len() {
                                state.w_inputbuffer[i].l.l[..state.w_inputbuffer[i].l.len].to_vec()
                            } else {
                                Vec::new()
                            };
                            let msg_text = String::from_utf8_lossy(&msg_bytes);

                            let prefix = if i < PLAYER_NAMES.len() {
                                Some(PLAYER_NAMES[i])
                            } else {
                                None
                            };

                            hulib_add_message_to_stext(&mut state.w_message, prefix, &msg_text);

                            state.message_nottobefuckedwith = true;
                            state.message_on = true;
                            state.message_counter = HU_MSGTIMEOUT;
                            sounds.push(SfxEnum::sfx_radio);

                            // Reset the input buffer for the next message.
                            if i < state.w_inputbuffer.len() {
                                hulib_reset_itext(&mut state.w_inputbuffer[i]);
                            }
                        } else if ch as i32 == KEY_ENTER {
                            // Message was from us echoing back — play tink sound.
                            sounds.push(SfxEnum::sfx_radio);
                        } else {
                            sounds.push(SfxEnum::sfx_tink);
                        }
                    }
                }
            }
        }
    }

    sounds
}

/// Process an input event for the HUD system.
///
/// Equivalent to `HU_Responder()` in hu_stuff.c (line 617). Handles:
/// - Shift/Alt modifier key tracking
/// - Message refresh on Enter key
/// - Chat mode toggle on 't' key (in netgame)
/// - Chat macro insertion on Alt+0..9
/// - Directed chat on destination keys (g/i/b/r)
/// - Character input during chat mode
/// - Chat send on Enter, cancel on Escape
///
/// # Arguments
///
/// * `state` — Mutable reference to the HUD state.
/// * `ev` — The input event to process.
/// * `netgame` — Whether this is a network multiplayer game.
/// * `consoleplayer` — Index of the local player.
/// * `playeringame` — Array indicating which player slots are active.
/// * `language` — Current language setting for keyboard translation.
///
/// # Returns
///
/// `true` if the event was consumed by the HUD (no further processing needed),
/// `false` otherwise.
pub fn hu_responder(
    state: &mut HudState,
    ev: &Event,
    netgame: bool,
    consoleplayer: usize,
    playeringame: &[bool],
    language: Language,
) -> bool {
    let mut eatkey = false;

    // Track Shift and Alt modifier keys.
    match ev.event_type {
        EventType::KeyDown => {
            if ev.data1 == KEY_RSHIFT {
                state.shift_down = true;
                return false;
            }
            if ev.data1 == KEY_RALT {
                state.alt_down = true;
                return false;
            }
        }
        EventType::KeyUp => {
            if ev.data1 == KEY_RSHIFT {
                state.shift_down = false;
                return false;
            }
            if ev.data1 == KEY_RALT {
                state.alt_down = false;
                return false;
            }
            return false;
        }
        _ => {
            return false;
        }
    }

    // From here on, we only process KeyDown events.
    // Get the key code.
    let ch = ev.data1;

    // If currently in chat mode, process the character.
    if state.chat_on {
        // Apply shift transform if Shift is held.
        let mut c = ch;
        if state.shift_down && (c as usize) < state.shiftxform.len() {
            c = state.shiftxform[c as usize] as i32;
        }

        // Apply French keyboard translation.
        if language == Language::French && c >= 0 && (c as usize) < FRENCH_KEY_MAP.len() {
            c = foreign_translation(c as u8) as i32;
        }

        // Feed the character to the chat input widget.
        let rc = hulib_key_in_itext(&mut state.w_chat, c as u8);
        if !rc {
            // Chat rejected the character (e.g., buffer full) — still consume
            // the event to prevent it from propagating.
            eatkey = true;
        }

        if c == KEY_ENTER {
            // Send the chat message.
            // Queue the destination header character.
            hu_queue_chat_char(state, state.chat_dest[consoleplayer]);

            // Queue each character of the chat message.
            for idx in 0..state.w_chat.l.len {
                hu_queue_chat_char(state, state.w_chat.l.l[idx]);
            }
            // Queue the Enter character to signal end-of-message.
            hu_queue_chat_char(state, KEY_ENTER as u8);

            // Exit chat mode.
            state.chat_on = false;

            // Display confirmation or talk-to-self message.
            if playeringame.len() > consoleplayer
                && !playeringame.get(consoleplayer).copied().unwrap_or(false)
                || !netgame
            {
                // Single-player: cycle through "talk to self" messages.
                let msg = TALK_TO_SELF[state.num_nobrainers % TALK_TO_SELF.len()];
                state.num_nobrainers += 1;
                // Store message for the player so ticker picks it up.
                state.message_dontfuckwithme = true;
                hulib_add_message_to_stext(&mut state.w_message, None, msg);
                state.message_on = true;
                state.message_counter = HU_MSGTIMEOUT;
            } else {
                // Multiplayer: show "message sent" confirmation.
                hulib_add_message_to_stext(&mut state.w_message, None, strings::HUSTR_MESSAGESENT);
                state.message_on = true;
                state.message_counter = HU_MSGTIMEOUT;
            }

            // Reset the chat input widget.
            hulib_reset_itext(&mut state.w_chat);

            eatkey = true;
        } else if c == KEY_BACKSPACE {
            // Backspace was already handled by hulib_key_in_itext.
            eatkey = true;
        } else if c > 0 {
            eatkey = true;
        }

        // Check for Escape to cancel chat.
        if ch == 27 {
            // Escape key (raw code 27)
            state.chat_on = false;
            hulib_reset_itext(&mut state.w_chat);
            eatkey = true;
        }

        return eatkey;
    }

    // Not in chat mode — check for chat activation and other HUD keys.

    // Message refresh on Enter key (re-display last message).
    if ch == HU_MSGREFRESH {
        state.message_on = true;
        state.message_counter = HU_MSGTIMEOUT;
        eatkey = true;
    }

    // Chat mode activation (only in netgame).
    if netgame && ch == HU_INPUTTOGGLE as i32 {
        // Check for Alt+0..9 (chat macros).
        if state.alt_down {
            // Alt+t doesn't toggle — ignore.
        } else {
            // Toggle chat mode with broadcast destination.
            state.chat_on = true;
            hulib_reset_itext(&mut state.w_chat);
            state.chat_dest[consoleplayer] = HU_BROADCAST as u8;
            eatkey = true;
        }
    }

    // Check for Alt+0..9 chat macros (only in netgame).
    if netgame && state.alt_down {
        let macro_idx: Option<usize> = match ch {
            48 => Some(0), // '0'
            49 => Some(1), // '1'
            50 => Some(2), // '2'
            51 => Some(3), // '3'
            52 => Some(4), // '4'
            53 => Some(5), // '5'
            54 => Some(6), // '6'
            55 => Some(7), // '7'
            56 => Some(8), // '8'
            57 => Some(9), // '9'
            _ => None,
        };

        if let Some(idx) = macro_idx {
            // Queue the macro as if it were typed character by character.
            // First, set broadcast destination.
            hu_queue_chat_char(state, HU_BROADCAST as u8);

            // Queue each character of the macro string.
            let macro_str = state.chat_macros[idx];
            for byte in macro_str.bytes() {
                hu_queue_chat_char(state, byte);
            }
            // Queue Enter to send.
            hu_queue_chat_char(state, KEY_ENTER as u8);

            eatkey = true;
        }
    }

    // Check for directed chat keys (g/i/b/r in netgame).
    if netgame && !state.chat_on {
        for (i, &dest_key) in DESTINATION_KEYS.iter().enumerate() {
            if ch == dest_key as i32 {
                if i < playeringame.len() && playeringame[i] {
                    state.chat_on = true;
                    hulib_reset_itext(&mut state.w_chat);
                    state.chat_dest[consoleplayer] = (i + 1) as u8;
                    eatkey = true;
                    break;
                } else {
                    // Player not in game — show "unsent" message.
                    let prefix = strings::HUSTR_MSGU;
                    let p_name = if i < PLAYER_NAMES.len() {
                        PLAYER_NAMES[i]
                    } else {
                        "Unknown"
                    };
                    let msg = format!("{}{}", prefix, p_name);
                    hulib_add_message_to_stext(&mut state.w_message, None, &msg);
                    state.message_on = true;
                    state.message_counter = HU_MSGTIMEOUT;
                    eatkey = false;
                    break;
                }
            }
        }
    }

    eatkey
}
