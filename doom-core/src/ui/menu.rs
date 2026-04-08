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

//! DOOM selection menu, options, episode etc.
//!
//! Translated from linuxdoom-1.10/m_menu.c and m_menu.h
//!
//! Implements the complete DOOM in-game menu system including:
//! - Main menu (New Game, Options, Load/Save, Read This, Quit)
//! - Episode selection (DOOM 1)
//! - Skill selection
//! - Options menu (detail, screen size, mouse sensitivity, volume)
//! - Sound volume sliders (SFX and Music)
//! - Save/Load game with 6 slots
//! - Quick save/load
//! - Help/Read This screens
//! - Message box system for confirmations
//!
//! All state is stored in [`MenuState`] — no `static mut` is used.
//! Menu callbacks use enum-based dispatch rather than C-style function
//! pointers, enabling safe access to the consolidated state.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use crate::game::game_ctrl::{self, GameCtrl};
use crate::game::strings;
use crate::info::sounds::SfxEnum;
use crate::traits::audio::AudioBackend;
use crate::traits::platform::PlatformHost;
use crate::traits::renderer::Renderer;
use crate::traits::wad::WadProvider;
use crate::types::doomdef::{self, GameMode, GameState, Skill, SCREENHEIGHT, SCREENWIDTH};
use crate::types::event::{Event, EventType, GameAction};
#[allow(unused_imports)]
// Schema-required import; Fixed is the canonical type for coordinate math.
use crate::types::fixed::Fixed;
#[allow(unused_imports)]
// Imported for type completeness per schema — accessed via GameCtrl.players.
use crate::types::player::Player;
use crate::ui::automap::AutomapState;
use crate::ui::hud::{HudState, HU_FONTSIZE, HU_FONTSTART};
use crate::util::argv::Args;
use crate::util::swap;
use crate::video::video::{VideoState, GAMMATABLE};
use doom_wad::PurgeTag;

// =============================================================================
// Constants — translated from m_menu.c
// =============================================================================

/// Maximum length of a save game description string (24 characters).
/// Original C: `#define SAVESTRINGSIZE 24` (m_menu.c line 76)
pub const SAVESTRINGSIZE: usize = 24;

/// Horizontal offset for the skull cursor from the menu item position.
/// Original C: `#define SKULLXOFF -32` (m_menu.c line 99)
pub const SKULLXOFF: i32 = -32;

/// Vertical spacing between menu items in pixels.
/// Original C: `#define LINEHEIGHT 16` (m_menu.c line 100)
pub const LINEHEIGHT: i32 = 16;

/// Number of save game slots.
const LOAD_END: usize = 6;

/// Gamma correction level messages displayed when the user presses F11.
/// Original C: `char* gammamsg[5]` (m_menu.c lines 91-98)
const GAMMAMSG: [&str; 5] = [
    strings::GAMMALVL0,
    strings::GAMMALVL1,
    strings::GAMMALVL2,
    strings::GAMMALVL3,
    strings::GAMMALVL4,
];

/// Quit sounds for DOOM 1 — played on quit confirmation.
/// Original C: `int quitsounds[8]` (m_menu.c lines 1002-1006)
const QUITSOUNDS: [SfxEnum; 8] = [
    SfxEnum::sfx_pldeth,
    SfxEnum::sfx_dmpain,
    SfxEnum::sfx_popain,
    SfxEnum::sfx_slop,
    SfxEnum::sfx_posit1,
    SfxEnum::sfx_posit3,
    SfxEnum::sfx_sgtsit,
    SfxEnum::sfx_vilact,
];

/// Quit sounds for DOOM 2 — played on quit confirmation.
/// Original C: `int quitsounds2[8]` (m_menu.c lines 1008-1012)
const QUITSOUNDS2: [SfxEnum; 8] = [
    SfxEnum::sfx_vilact,
    SfxEnum::sfx_getpow,
    SfxEnum::sfx_bspsit,
    SfxEnum::sfx_sgtatk,
    SfxEnum::sfx_skeact,
    SfxEnum::sfx_skepch,
    SfxEnum::sfx_vilatk,
    SfxEnum::sfx_sgtatk,
];

/// Detail level display names (patch lump names).
/// Original C: `char* detailNames[2]` (m_menu.c line 901)
const DETAIL_NAMES: [&str; 2] = ["M_GDHIGH", "M_GDLOW"];

/// Messages toggle display names (patch lump names).
/// Original C: `char* msgNames[2]` (m_menu.c line 902)
const MSG_NAMES: [&str; 2] = ["M_MSGOFF", "M_MSGON"];

// =============================================================================
// Menu indices — identify menus within the menus array
// =============================================================================

/// Index of the main menu in `MenuState.menus`.
const MENU_MAIN: usize = 0;
/// Index of the episode selection menu.
const MENU_EPISODE: usize = 1;
/// Index of the new game (skill selection) menu.
const MENU_NEWGAME: usize = 2;
/// Index of the options menu.
const MENU_OPTIONS: usize = 3;
/// Index of the first "Read This" help screen menu.
const MENU_READ1: usize = 4;
/// Index of the second "Read This" help screen menu.
const MENU_READ2: usize = 5;
/// Index of the sound volume settings menu.
const MENU_SOUND: usize = 6;
/// Index of the load game menu.
const MENU_LOAD: usize = 7;
/// Index of the save game menu.
const MENU_SAVE: usize = 8;

// =============================================================================
// Enum types for menu callback and draw dispatch
// =============================================================================

/// Identifies which menu callback to invoke when a menu item is selected
/// or adjusted. Replaces C-style function pointers from `menuitem_t.routine`.
///
/// Each variant corresponds to one of the original `M_*` callback functions
/// in m_menu.c. The dispatch is centralized in [`dispatch_callback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCallback {
    /// No action (empty/separator items).
    None,
    /// Start a new game — `M_NewGame` (m_menu.c line 832).
    NewGame,
    /// Open options menu — `M_Options` (m_menu.c line 882).
    Options,
    /// Open load game menu — `M_LoadGame` (m_menu.c line 584).
    LoadGame,
    /// Open save game menu — `M_SaveGame` (m_menu.c line 640).
    SaveGame,
    /// Open "Read This" help screen — `M_ReadThis` (m_menu.c line 1067).
    ReadThis,
    /// Quit DOOM — `M_QuitDOOM` (m_menu.c line 1037).
    QuitDoom,
    /// Select episode — `M_Episode` (m_menu.c line 860).
    Episode,
    /// Choose skill level — `M_ChooseSkill` (m_menu.c line 849).
    ChooseSkill,
    /// Confirm nightmare difficulty — `M_VerifyNightmare` (m_menu.c line 841).
    VerifyNightmare,
    /// End current game — `M_EndGame` (m_menu.c line 1048).
    EndGame,
    /// End game confirmation response — `M_EndGameResponse` (m_menu.c line 1030).
    EndGameResponse,
    /// Toggle messages on/off — `M_ChangeMessages` (m_menu.c line 884).
    ChangeMessages,
    /// Toggle detail level — `M_ChangeDetail` (m_menu.c line 1101).
    ChangeDetail,
    /// Change screen size — `M_SizeDisplay` (m_menu.c line 1120).
    SizeDisplay,
    /// Change mouse sensitivity — `M_ChangeSensitivity` (m_menu.c line 1089).
    ChangeSensitivity,
    /// Open sound menu — `M_Sound` (m_menu.c line 768).
    Sound,
    /// Adjust SFX volume — `M_SfxVol` (m_menu.c line 774).
    SfxVol,
    /// Adjust music volume — `M_MusicVol` (m_menu.c line 783).
    MusicVol,
    /// Load game from slot — `M_LoadSelect` (m_menu.c line 566).
    LoadSelect,
    /// Save game to slot — `M_SaveSelect` (m_menu.c line 622).
    SaveSelect,
    /// Go to second help screen — `M_ReadThis2` (m_menu.c line 1073).
    ReadThis2,
    /// Finish reading help — `M_FinishReadThis` (m_menu.c line 1079).
    FinishReadThis,
    /// Quick save response — `M_QuickSaveResponse` (m_menu.c line 656).
    QuickSaveResponse,
    /// Quick load response — `M_QuickLoadResponse` (m_menu.c line 679).
    QuickLoadResponse,
    /// Quit game response — `M_QuitResponse` (m_menu.c line 1018).
    QuitResponse,
}

/// Identifies which drawing routine to call for a menu screen.
/// Replaces C-style function pointers from `menu_t.routine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuDraw {
    MainMenu,
    Episode,
    NewGame,
    Options,
    ReadThis1,
    ReadThis2,
    Sound,
    Load,
    Save,
}

// =============================================================================
// MenuItem and Menu structs
// =============================================================================

/// A single item within a menu screen.
///
/// Translated from C `menuitem_t` (m_menu.c lines 140-146).
///
/// * `status`: 0 = no cursor allowed, 1 = selectable, 2 = slider (arrows ok)
/// * `name`: Patch lump name for the menu item graphic (up to 8 chars)
/// * `routine`: Callback enum dispatched when the item is selected or adjusted
/// * `alpha_key`: Keyboard hotkey character for this item
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub status: i16,
    pub name: &'static str,
    pub routine: MenuCallback,
    pub alpha_key: u8,
}

impl MenuItem {
    /// Return the lump name as a string for WAD lookup.
    pub fn name_str(&self) -> &str {
        self.name
    }
}

/// A complete menu screen definition.
///
/// Translated from C `menu_t` (m_menu.c lines 148-157).
///
/// * `num_items`: Number of active items in this menu
/// * `prev_menu`: Index of the parent menu (None for root)
/// * `menu_items`: The items in this menu
/// * `draw_routine`: Which draw function to invoke
/// * `x`, `y`: Screen position of the menu
/// * `last_on`: Index of the last selected item (preserved across visits)
#[derive(Debug, Clone)]
pub struct Menu {
    pub num_items: i16,
    pub prev_menu: Option<usize>,
    pub menu_items: Vec<MenuItem>,
    pub draw_routine: MenuDraw,
    pub x: i16,
    pub y: i16,
    pub last_on: i16,
}

// =============================================================================
// MenuState — consolidated mutable state for the menu system
// =============================================================================

/// Consolidated menu state struct holding all formerly-global variables
/// from m_menu.c. Replaces C global mutable state per AAP §0.7.5.
#[derive(Debug)]
pub struct MenuState {
    // -- Input sensitivity --
    /// Mouse sensitivity (0-9). Original C: `int mouseSensitivity` (m_menu.c line 76)
    pub mouse_sensitivity: i32,

    // -- Display options --
    /// Messages on/off (0=off, 1=on). Original C: `int showMessages` (m_menu.c line 79)
    pub show_messages: i32,
    /// Detail level (0=high, 1=low/blocky). Original C: `int detailLevel` (m_menu.c line 84)
    pub detail_level: i32,
    /// Current screen size setting (3-11). Original C: `int screenblocks` (m_menu.c line 85)
    pub screen_blocks: i32,
    /// Temporary screen size for menu (0-8). Original C: `int screenSize` (m_menu.c line 86)
    pub screen_size: i32,

    // -- Quick save state --
    /// Quick save slot index (-1 = not set). Original C: `int quickSaveSlot` (m_menu.c line 89)
    pub quick_save_slot: i32,

    // -- Message box state --
    /// Whether a message box is being displayed. Original C: `int messageToPrint` (m_menu.c line 78)
    pub message_to_print: i32,
    /// The message string to display. Original C: `char* messageString` (m_menu.c line 80)
    pub message_string: Option<String>,
    /// Whether the message requires Y/N input. Original C: `int messageNeedsInput` (m_menu.c line 82)
    pub message_needs_input: bool,
    /// Callback to invoke when message is acknowledged.
    /// Original C: `void (*messageRoutine)(int response)` (m_menu.c line 83)
    message_routine: Option<MenuCallback>,
    /// Menu active state when message was shown.
    /// Original C: `int messageLastMenuActive` (m_menu.c line 81)
    message_last_menu_active: bool,

    // -- Save string editing state --
    /// Whether save string input is active. Original C: `int saveStringEnter` (m_menu.c line 93)
    pub save_string_enter: i32,
    /// Which save slot is being edited. Original C: `int saveSlot` (m_menu.c line 94)
    pub save_slot: i32,
    /// Current cursor position in save string. Original C: `int saveCharIndex` (m_menu.c line 95)
    pub save_char_index: i32,
    /// Backup of save string before editing.
    /// Original C: `char saveOldString[SAVESTRINGSIZE]` (m_menu.c line 96)
    save_old_string: [u8; SAVESTRINGSIZE],

    // -- Navigation state --
    /// Whether we are displaying a help screen. Original C: `boolean inhelpscreens` (m_menu.c line 97)
    pub inhelpscreens: bool,
    /// Whether the menu is currently active/visible. Original C: `boolean menuactive` (m_menu.c line 98)
    pub menu_active: bool,

    // -- Skull cursor state --
    /// Current item index (skull position). Original C: `short itemOn` (m_menu.c line 102)
    pub item_on: i16,
    /// Skull animation countdown timer. Original C: `short skullAnimCounter` (m_menu.c line 103)
    pub skull_anim_counter: i16,
    /// Current skull frame (0 or 1). Original C: `short whichSkull` (m_menu.c line 104)
    pub which_skull: i16,

    // -- Current menu tracking --
    /// Index of the current menu in the `menus` array.
    /// Original C: `menu_t* currentMenu` (pointer into static array)
    pub current_menu: usize,

    // -- Save game strings --
    /// Save game description strings for all 6+4 slots.
    /// Original C: `char savegamestrings[10][SAVESTRINGSIZE]`
    pub savegame_strings: [[u8; SAVESTRINGSIZE]; 10],

    // -- Menu definitions (mutable, modified during init) --
    /// All menu screen definitions. Indices defined by MENU_* constants.
    menus: Vec<Menu>,

    // -- Responder internal state (C static locals in M_Responder) --
    /// Joystick input wait timer. Original C: `static int joywait` (m_menu.c line 1351)
    joywait: i32,
    /// Mouse input wait timer. Original C: `static int mousewait` (m_menu.c line 1352)
    mousewait: i32,
    /// Accumulated mouse Y movement. Original C: `static int mousey` (m_menu.c line 1353)
    mousey: i32,
    /// Last mouse Y value. Original C: `static int lasty` (m_menu.c line 1354)
    lasty: i32,
    /// Accumulated mouse X movement. Original C: `static int mousex` (m_menu.c line 1355)
    mousex: i32,
    /// Last mouse X value. Original C: `static int lastx` (m_menu.c line 1356)
    lastx: i32,

    // -- Tic counter used by responder for input rate limiting --
    /// Current input tic counter, updated each frame for mouse/joy rate-limiting.
    input_tic: i32,

    // -- Episode selection (file-scope variable) --
    /// Selected episode index (0-based). Original C: `static int epi` (m_menu.c line 857)
    epi: i32,
    /// Temporary string buffer for formatted messages.
    /// Original C: `char tempstring[80]` (m_menu.c)
    temp_string: String,
    /// End string buffer for quit messages.
    /// Original C: `char endstring[160]` (m_menu.c)
    end_string: String,

    // -- SFX/Music volume for the Sound menu --
    /// Current SFX volume (0-15). Stored here for slider display.
    sfx_volume: i32,
    /// Current music volume (0-15). Stored here for slider display.
    music_volume: i32,
}

impl MenuState {
    /// Create a new `MenuState` with default values.
    /// Menu definitions are populated by [`m_init`].
    pub fn new() -> Self {
        MenuState {
            mouse_sensitivity: 5,
            show_messages: 1,
            detail_level: 0,
            screen_blocks: 9,
            screen_size: 0,
            quick_save_slot: -1,
            message_to_print: 0,
            message_string: None,
            message_needs_input: false,
            message_routine: None,
            message_last_menu_active: false,
            save_string_enter: 0,
            save_slot: 0,
            save_char_index: 0,
            save_old_string: [0u8; SAVESTRINGSIZE],
            inhelpscreens: false,
            menu_active: false,
            item_on: 0,
            skull_anim_counter: 10,
            which_skull: 0,
            current_menu: MENU_MAIN,
            savegame_strings: [[0u8; SAVESTRINGSIZE]; 10],
            menus: Vec::new(),
            joywait: 0,
            mousewait: 0,
            mousey: 0,
            lasty: 0,
            mousex: 0,
            lastx: 0,
            input_tic: 0,
            epi: 0,
            temp_string: String::new(),
            end_string: String::new(),
            sfx_volume: 0,
            music_volume: 0,
        }
    }
}

impl Default for MenuState {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Menu definition construction
// =============================================================================

/// Build all menu definitions. Called during [`m_init`].
///
/// Recreates the static C menu arrays (MainMenu, EpisodeMenu, NewGameMenu,
/// OptionsMenu, ReadMenu1, ReadMenu2, SoundMenu, LoadMenu, SaveMenu) and
/// their corresponding `menu_t` definitions (MainDef, EpiDef, NewDef, etc.).
fn build_menus() -> Vec<Menu> {
    // -- Main Menu (m_menu.c lines 239-249) --
    // MainMenu: 6 items — NewGame, Options, LoadGame, SaveGame, ReadThis, QuitDoom
    let main_items = vec![
        MenuItem {
            status: 1,
            name: "M_NGAME",
            routine: MenuCallback::NewGame,
            alpha_key: b'n',
        },
        MenuItem {
            status: 1,
            name: "M_OPTION",
            routine: MenuCallback::Options,
            alpha_key: b'o',
        },
        MenuItem {
            status: 1,
            name: "M_LOADG",
            routine: MenuCallback::LoadGame,
            alpha_key: b'l',
        },
        MenuItem {
            status: 1,
            name: "M_SAVEG",
            routine: MenuCallback::SaveGame,
            alpha_key: b's',
        },
        MenuItem {
            status: 1,
            name: "M_RDTHIS",
            routine: MenuCallback::ReadThis,
            alpha_key: b'r',
        },
        MenuItem {
            status: 1,
            name: "M_QUITG",
            routine: MenuCallback::QuitDoom,
            alpha_key: b'q',
        },
    ];
    let main_def = Menu {
        num_items: 6,
        prev_menu: None,
        menu_items: main_items,
        draw_routine: MenuDraw::MainMenu,
        x: 97,
        y: 64,
        last_on: 0,
    };

    // -- Episode Menu (m_menu.c lines 266-273) --
    let episode_items = vec![
        MenuItem {
            status: 1,
            name: "M_EPI1",
            routine: MenuCallback::Episode,
            alpha_key: b'k',
        },
        MenuItem {
            status: 1,
            name: "M_EPI2",
            routine: MenuCallback::Episode,
            alpha_key: b't',
        },
        MenuItem {
            status: 1,
            name: "M_EPI3",
            routine: MenuCallback::Episode,
            alpha_key: b'i',
        },
        MenuItem {
            status: 1,
            name: "M_EPI4",
            routine: MenuCallback::Episode,
            alpha_key: b't',
        },
    ];
    let episode_def = Menu {
        num_items: 4,
        prev_menu: Some(MENU_MAIN),
        menu_items: episode_items,
        draw_routine: MenuDraw::Episode,
        x: 48,
        y: 63,
        last_on: 0, // ep1
    };

    // -- New Game (Skill) Menu (m_menu.c lines 290-298) --
    let newgame_items = vec![
        MenuItem {
            status: 1,
            name: "M_JKILL",
            routine: MenuCallback::ChooseSkill,
            alpha_key: b'i',
        },
        MenuItem {
            status: 1,
            name: "M_ROUGH",
            routine: MenuCallback::ChooseSkill,
            alpha_key: b'h',
        },
        MenuItem {
            status: 1,
            name: "M_HURT",
            routine: MenuCallback::ChooseSkill,
            alpha_key: b'h',
        },
        MenuItem {
            status: 1,
            name: "M_ULTRA",
            routine: MenuCallback::ChooseSkill,
            alpha_key: b'u',
        },
        MenuItem {
            status: 1,
            name: "M_NMARE",
            routine: MenuCallback::ChooseSkill,
            alpha_key: b'n',
        },
    ];
    let newgame_def = Menu {
        num_items: 5,
        prev_menu: Some(MENU_EPISODE),
        menu_items: newgame_items,
        draw_routine: MenuDraw::NewGame,
        x: 48,
        y: 63,
        last_on: 2, // hurt me plenty
    };

    // -- Options Menu (m_menu.c lines 341-352) --
    // endgame, messages, detail, -, scrnsize, -, mousesens, -, sfxvol, -, musicvol
    let options_items = vec![
        MenuItem {
            status: 1,
            name: "M_ENDGAM",
            routine: MenuCallback::EndGame,
            alpha_key: b'e',
        },
        MenuItem {
            status: 1,
            name: "M_MESSG",
            routine: MenuCallback::ChangeMessages,
            alpha_key: b'm',
        },
        MenuItem {
            status: 1,
            name: "M_DETAIL",
            routine: MenuCallback::ChangeDetail,
            alpha_key: b'g',
        },
        MenuItem {
            status: 2,
            name: "M_SCRNSZ",
            routine: MenuCallback::SizeDisplay,
            alpha_key: b's',
        },
        MenuItem {
            status: -1,
            name: "",
            routine: MenuCallback::None,
            alpha_key: 0,
        },
        MenuItem {
            status: 2,
            name: "M_MSENS",
            routine: MenuCallback::ChangeSensitivity,
            alpha_key: b'm',
        },
        MenuItem {
            status: -1,
            name: "",
            routine: MenuCallback::None,
            alpha_key: 0,
        },
        MenuItem {
            status: 1,
            name: "M_SVOL",
            routine: MenuCallback::Sound,
            alpha_key: b's',
        },
    ];
    let options_def = Menu {
        num_items: 8,
        prev_menu: Some(MENU_MAIN),
        menu_items: options_items,
        draw_routine: MenuDraw::Options,
        x: 60,
        y: 37,
        last_on: 0,
    };

    // -- Read This 1 (m_menu.c lines 387-393) --
    let read1_items = vec![MenuItem {
        status: 1,
        name: "",
        routine: MenuCallback::ReadThis2,
        alpha_key: 0,
    }];
    let read1_def = Menu {
        num_items: 1,
        prev_menu: Some(MENU_MAIN),
        menu_items: read1_items,
        draw_routine: MenuDraw::ReadThis1,
        x: 280,
        y: 185,
        last_on: 0,
    };

    // -- Read This 2 (m_menu.c lines 410-416) --
    let read2_items = vec![MenuItem {
        status: 1,
        name: "",
        routine: MenuCallback::FinishReadThis,
        alpha_key: 0,
    }];
    let read2_def = Menu {
        num_items: 1,
        prev_menu: Some(MENU_READ1),
        menu_items: read2_items,
        draw_routine: MenuDraw::ReadThis2,
        x: 330,
        y: 175,
        last_on: 0,
    };

    // -- Sound Menu (m_menu.c lines 440-447) --
    let sound_items = vec![
        MenuItem {
            status: 2,
            name: "M_SFXVOL",
            routine: MenuCallback::SfxVol,
            alpha_key: b's',
        },
        MenuItem {
            status: -1,
            name: "",
            routine: MenuCallback::None,
            alpha_key: 0,
        },
        MenuItem {
            status: 2,
            name: "M_MUSVOL",
            routine: MenuCallback::MusicVol,
            alpha_key: b'm',
        },
        MenuItem {
            status: -1,
            name: "",
            routine: MenuCallback::None,
            alpha_key: 0,
        },
    ];
    let sound_def = Menu {
        num_items: 4,
        prev_menu: Some(MENU_OPTIONS),
        menu_items: sound_items,
        draw_routine: MenuDraw::Sound,
        x: 80,
        y: 64,
        last_on: 0,
    };

    // -- Load Game Menu (m_menu.c lines 468-477) --
    let load_items: Vec<MenuItem> = (0..LOAD_END)
        .map(|_| MenuItem {
            status: 1,
            name: "",
            routine: MenuCallback::LoadSelect,
            alpha_key: 0,
        })
        .collect();
    let load_def = Menu {
        num_items: LOAD_END as i16,
        prev_menu: Some(MENU_MAIN),
        menu_items: load_items,
        draw_routine: MenuDraw::Load,
        x: 80,
        y: 54,
        last_on: 0,
    };

    // -- Save Game Menu (m_menu.c lines 494-503) --
    let save_items: Vec<MenuItem> = (0..LOAD_END)
        .map(|_| MenuItem {
            status: 1,
            name: "",
            routine: MenuCallback::SaveSelect,
            alpha_key: 0,
        })
        .collect();
    let save_def = Menu {
        num_items: LOAD_END as i16,
        prev_menu: Some(MENU_MAIN),
        menu_items: save_items,
        draw_routine: MenuDraw::Save,
        x: 80,
        y: 54,
        last_on: 0,
    };

    // Order MUST match MENU_* constants:
    // 0=Main, 1=Episode, 2=NewGame, 3=Options, 4=Read1, 5=Read2, 6=Sound, 7=Load, 8=Save
    vec![
        main_def,    // MENU_MAIN = 0
        episode_def, // MENU_EPISODE = 1
        newgame_def, // MENU_NEWGAME = 2
        options_def, // MENU_OPTIONS = 3
        read1_def,   // MENU_READ1 = 4
        read2_def,   // MENU_READ2 = 5
        sound_def,   // MENU_SOUND = 6
        load_def,    // MENU_LOAD = 7
        save_def,    // MENU_SAVE = 8
    ]
}

// =============================================================================
// Save game file path construction
// =============================================================================

/// Build the file path for a save game slot.
///
/// Original C used `sprintf(name, SAVEGAMENAME "%d.dsg", slot)` with
/// an optional `-cdrom` prefix of `"c:\\doomdata\\"`.
fn build_save_name(slot: usize, args: &Args) -> PathBuf {
    let filename = format!("{}{}.dsg", strings::SAVEGAMENAME, slot);
    if args.check_parm("-cdrom").is_some() {
        let mut path = PathBuf::from("c:\\doomdata");
        path.push(&filename);
        path
    } else {
        PathBuf::from(&filename)
    }
}

/// Read save game description strings from disk for all slots.
///
/// Original C: `M_ReadSaveStrings` (m_menu.c lines 506-530)
fn m_read_save_strings(state: &mut MenuState, args: &Args) {
    for i in 0..LOAD_END {
        let name = build_save_name(i, args);
        match fs::File::open(&name) {
            Ok(mut f) => {
                let mut buf = [0u8; SAVESTRINGSIZE];
                if f.read_exact(&mut buf).is_ok() {
                    state.savegame_strings[i] = buf;
                    state.menus[MENU_LOAD].menu_items[i].status = 1;
                } else {
                    copy_empty_string(&mut state.savegame_strings[i]);
                    state.menus[MENU_LOAD].menu_items[i].status = 0;
                }
            }
            Err(_) => {
                copy_empty_string(&mut state.savegame_strings[i]);
                state.menus[MENU_LOAD].menu_items[i].status = 0;
            }
        }
    }
}

/// Copy the EMPTYSTRING constant into a save game string buffer.
fn copy_empty_string(dest: &mut [u8; SAVESTRINGSIZE]) {
    let src = strings::EMPTYSTRING.as_bytes();
    let len = src.len().min(SAVESTRINGSIZE);
    dest[..len].copy_from_slice(&src[..len]);
    if len < SAVESTRINGSIZE {
        dest[len..].fill(0);
    }
}

/// Convert a save game string buffer to a displayable string.
fn save_string_to_str(buf: &[u8; SAVESTRINGSIZE]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(SAVESTRINGSIZE);
    String::from_utf8_lossy(&buf[..end]).to_string()
}

// =============================================================================
// Core public API functions
// =============================================================================

/// Initialize the menu system.
///
/// Equivalent of `M_Init` (m_menu.c lines 1846-1893). Builds all menu
/// definitions, sets the initial state, and applies gamemode-specific
/// adjustments (commercial mode removes "Read This", shareware/registered
/// reduce episode count).
pub fn m_init(state: &mut MenuState, gamemode: GameMode) {
    state.current_menu = MENU_MAIN;
    state.menu_active = false;
    state.item_on = state.menus.get(MENU_MAIN).map_or(0, |m| m.last_on);
    state.skull_anim_counter = 10;
    state.which_skull = 0;
    state.screen_size = state.screen_blocks.saturating_sub(3).max(0);
    state.message_to_print = 0;
    state.message_string = None;
    state.message_needs_input = false;
    state.quick_save_slot = -1;

    if state.menus.is_empty() {
        state.menus = build_menus();
    }

    // Gamemode adjustments — matches M_Init in m_menu.c lines 1862-1893.
    match gamemode {
        GameMode::Commercial => {
            // Commercial (DOOM II): remove "Read This" from main menu.
            if state.menus[MENU_MAIN].menu_items.len() >= 6 {
                state.menus[MENU_MAIN].menu_items[4] = state.menus[MENU_MAIN].menu_items[5].clone();
                state.menus[MENU_MAIN].num_items = 5;
                state.menus[MENU_MAIN].y += 8;
            }
            state.menus[MENU_NEWGAME].prev_menu = Some(MENU_MAIN);
            state.menus[MENU_READ1].x = 330;
            state.menus[MENU_READ1].y = 165;
            state.menus[MENU_READ2].x = 330;
            state.menus[MENU_READ2].y = 165;
        }
        GameMode::Shareware => {
            state.menus[MENU_EPISODE].num_items = 1;
        }
        GameMode::Registered => {
            state.menus[MENU_EPISODE].num_items = 3;
        }
        GameMode::Retail | GameMode::Indetermined => {}
    }
}

/// Force the menu to appear (e.g., when pressing Escape).
///
/// Equivalent of `M_StartControlPanel` (m_menu.c lines 1722-1736).
pub fn m_start_control_panel(state: &mut MenuState) {
    state.menu_active = true;
    state.current_menu = MENU_MAIN;
    state.item_on = state.menus[MENU_MAIN].last_on;
}

/// Display a message box with optional Yes/No input.
///
/// Equivalent of `M_StartMessage` (m_menu.c lines 1218-1228).
pub fn m_start_message(
    state: &mut MenuState,
    string: &str,
    routine: Option<MenuCallback>,
    input: bool,
) {
    state.message_last_menu_active = state.menu_active;
    state.message_to_print = 1;
    state.message_string = Some(string.to_string());
    state.message_routine = routine;
    state.message_needs_input = input;
    state.menu_active = true;
}

/// Dismiss the current message box.
/// Available as a public utility for external callers (e.g., game loop reset).
pub fn m_stop_message(state: &mut MenuState) {
    state.menu_active = state.message_last_menu_active;
    state.message_to_print = 0;
}

/// Close all menus.
fn m_clear_menus(state: &mut MenuState) {
    state.menu_active = false;
}

/// Switch to the specified menu screen.
fn m_setup_next_menu(state: &mut MenuState, menu_index: usize) {
    state.current_menu = menu_index;
    state.item_on = state.menus[menu_index].last_on;
}

// =============================================================================
// Menu action callbacks
// =============================================================================

/// Central callback dispatch. Called when a menu item is activated or adjusted.
fn dispatch_callback(
    callback: MenuCallback,
    choice: i32,
    state: &mut MenuState,
    game: &mut GameCtrl,
    _video: &mut VideoState,
    audio: &mut dyn AudioBackend,
    renderer: &mut dyn Renderer,
    platform: &mut dyn PlatformHost,
    _hud: &HudState,
    args: &Args,
) {
    match callback {
        MenuCallback::None => {}
        MenuCallback::NewGame => cb_new_game(state, game),
        MenuCallback::Options => cb_options(state),
        MenuCallback::LoadGame => cb_load_game(state, game, args),
        MenuCallback::SaveGame => cb_save_game(state, game, args),
        MenuCallback::ReadThis => cb_read_this(state, game),
        MenuCallback::QuitDoom => cb_quit_doom(state, game, audio),
        MenuCallback::Episode => cb_episode(choice, state, game),
        MenuCallback::ChooseSkill => cb_choose_skill(choice, state, game),
        MenuCallback::VerifyNightmare => cb_verify_nightmare(choice, state, game),
        MenuCallback::EndGame => cb_end_game(state, game, audio),
        MenuCallback::EndGameResponse => cb_end_game_response(choice, state, game),
        MenuCallback::ChangeMessages => cb_change_messages(state, game),
        MenuCallback::ChangeDetail => cb_change_detail(state, game),
        MenuCallback::SizeDisplay => cb_size_display(choice, state, renderer),
        MenuCallback::ChangeSensitivity => cb_change_sensitivity(choice, state),
        MenuCallback::Sound => cb_sound(state),
        MenuCallback::SfxVol => cb_sfx_vol(choice, state, audio),
        MenuCallback::MusicVol => cb_music_vol(choice, state, audio),
        MenuCallback::LoadSelect => cb_load_select(choice, state, game, args),
        MenuCallback::SaveSelect => cb_save_select(choice, state),
        MenuCallback::ReadThis2 => cb_read_this2(state),
        MenuCallback::FinishReadThis => cb_finish_read_this(state),
        MenuCallback::QuickSaveResponse => {
            cb_quick_save_response(choice, state, game, args);
        }
        MenuCallback::QuickLoadResponse => {
            cb_quick_load_response(choice, state, game, args);
        }
        MenuCallback::QuitResponse => {
            cb_quit_response(choice, state, game, audio, platform);
        }
    }
}

fn cb_new_game(state: &mut MenuState, game: &GameCtrl) {
    if game.netgame && !game.demoplayback {
        m_start_message(state, strings::NEWGAME, None, false);
        return;
    }
    if game.gamemode == GameMode::Commercial {
        m_setup_next_menu(state, MENU_NEWGAME);
    } else {
        m_setup_next_menu(state, MENU_EPISODE);
    }
}

fn cb_options(state: &mut MenuState) {
    m_setup_next_menu(state, MENU_OPTIONS);
}

fn cb_load_game(state: &mut MenuState, game: &GameCtrl, args: &Args) {
    if game.netgame {
        m_start_message(state, strings::LOADNET, None, false);
        return;
    }
    m_setup_next_menu(state, MENU_LOAD);
    m_read_save_strings(state, args);
}

fn cb_save_game(state: &mut MenuState, game: &GameCtrl, args: &Args) {
    if !game.usergame {
        m_start_message(state, strings::SAVEDEAD, None, false);
        return;
    }
    if game.gamestate != GameState::Level {
        return;
    }
    m_setup_next_menu(state, MENU_SAVE);
    m_read_save_strings(state, args);
}

fn cb_read_this(state: &mut MenuState, game: &GameCtrl) {
    match game.gamemode {
        GameMode::Retail => m_setup_next_menu(state, MENU_READ2),
        _ => m_setup_next_menu(state, MENU_READ1),
    }
}

fn cb_quit_doom(state: &mut MenuState, game: &GameCtrl, _audio: &mut dyn AudioBackend) {
    let endmsg_index = if game.gamemode == GameMode::Commercial {
        (((game.gametic >> 2) & 7) + 8) as usize
    } else {
        ((game.gametic >> 2) & 7) as usize
    };
    let msg_index = endmsg_index.min(strings::ENDMSG.len().saturating_sub(1));
    state.end_string = format!("{}\n\n{}", strings::ENDMSG[msg_index], strings::DOSY);
    let msg = state.end_string.clone();
    m_start_message(state, &msg, Some(MenuCallback::QuitResponse), true);
}

fn cb_episode(choice: i32, state: &mut MenuState, game: &GameCtrl) {
    if game.gamemode == GameMode::Shareware && choice != 0 {
        m_start_message(state, strings::SWSTRING, None, false);
        m_setup_next_menu(state, MENU_READ1);
        return;
    }
    if game.gamemode == GameMode::Registered && choice > 2 {
        return;
    }
    state.epi = choice;
    m_setup_next_menu(state, MENU_NEWGAME);
}

fn cb_choose_skill(choice: i32, state: &mut MenuState, game: &mut GameCtrl) {
    if choice == Skill::Nightmare as i32 {
        m_start_message(
            state,
            strings::NIGHTMARE,
            Some(MenuCallback::VerifyNightmare),
            true,
        );
        return;
    }
    let skill = match choice {
        0 => Skill::Baby,
        1 => Skill::Easy,
        2 => Skill::Medium,
        3 => Skill::Hard,
        4 => Skill::Nightmare,
        _ => Skill::Medium,
    };
    game_ctrl::g_defered_init_new(game, skill, state.epi + 1, 1);
    m_clear_menus(state);
}

fn cb_verify_nightmare(choice: i32, state: &mut MenuState, game: &mut GameCtrl) {
    if choice != b'y' as i32 {
        return;
    }
    game_ctrl::g_defered_init_new(game, Skill::Nightmare, state.epi + 1, 1);
    m_clear_menus(state);
}

fn cb_end_game(state: &mut MenuState, game: &GameCtrl, audio: &mut dyn AudioBackend) {
    if !game.usergame {
        audio.start_sound(SfxEnum::sfx_oof as i32, 128, 128, 128, 0);
        return;
    }
    if game.netgame {
        m_start_message(state, strings::NETEND, None, false);
        return;
    }
    m_start_message(
        state,
        strings::ENDGAME,
        Some(MenuCallback::EndGameResponse),
        true,
    );
}

fn cb_end_game_response(choice: i32, state: &mut MenuState, game: &mut GameCtrl) {
    if choice != b'y' as i32 {
        return;
    }
    state.menus[state.current_menu].last_on = state.item_on;
    m_clear_menus(state);
    game.gameaction = GameAction::Nothing;
    game.gamestate = GameState::DemoScreen;
}

fn cb_change_messages(state: &mut MenuState, game: &mut GameCtrl) {
    state.show_messages = 1 - state.show_messages;
    if let Some(player) = game.players.get_mut(game.consoleplayer) {
        if state.show_messages == 0 {
            player.message = Some(strings::MSGOFF.to_string());
        } else {
            player.message = Some(strings::MSGON.to_string());
        }
    }
}

fn cb_change_detail(state: &mut MenuState, game: &mut GameCtrl) {
    state.detail_level = 1 - state.detail_level;
    if let Some(player) = game.players.get_mut(game.consoleplayer) {
        if state.detail_level == 0 {
            player.message = Some(strings::DETAILHI.to_string());
        } else {
            player.message = Some(strings::DETAILLO.to_string());
        }
    }
}

fn cb_size_display(choice: i32, state: &mut MenuState, renderer: &mut dyn Renderer) {
    match choice {
        0 => {
            if state.screen_size > 0 {
                state.screen_blocks -= 1;
                state.screen_size -= 1;
            }
        }
        1 => {
            if state.screen_size < 8 {
                state.screen_blocks += 1;
                state.screen_size += 1;
            }
        }
        _ => {}
    }
    renderer.set_view_size(state.screen_blocks, state.detail_level);
}

fn cb_change_sensitivity(choice: i32, state: &mut MenuState) {
    match choice {
        0 => {
            if state.mouse_sensitivity > 0 {
                state.mouse_sensitivity -= 1;
            }
        }
        1 => {
            if state.mouse_sensitivity < 9 {
                state.mouse_sensitivity += 1;
            }
        }
        _ => {}
    }
}

fn cb_sound(state: &mut MenuState) {
    m_setup_next_menu(state, MENU_SOUND);
}

fn cb_sfx_vol(choice: i32, state: &mut MenuState, _audio: &mut dyn AudioBackend) {
    match choice {
        0 => {
            if state.sfx_volume > 0 {
                state.sfx_volume -= 1;
            }
        }
        1 => {
            if state.sfx_volume < 15 {
                state.sfx_volume += 1;
            }
        }
        _ => {}
    }
}

fn cb_music_vol(choice: i32, state: &mut MenuState, audio: &mut dyn AudioBackend) {
    match choice {
        0 => {
            if state.music_volume > 0 {
                state.music_volume -= 1;
            }
        }
        1 => {
            if state.music_volume < 15 {
                state.music_volume += 1;
            }
        }
        _ => {}
    }
    audio.set_music_volume(state.music_volume);
}

fn cb_load_select(choice: i32, state: &mut MenuState, game: &mut GameCtrl, args: &Args) {
    let name = build_save_name(choice as usize, args);
    game_ctrl::g_load_game(game, &name.to_string_lossy());
    m_clear_menus(state);
}

fn cb_save_select(choice: i32, state: &mut MenuState) {
    let idx = (choice as usize).min(state.savegame_strings.len() - 1);
    state.save_slot = choice;
    state.save_old_string = state.savegame_strings[idx];
    state.save_string_enter = 1;
    let buf = &state.savegame_strings[idx];
    let end = buf.iter().position(|&b| b == 0).unwrap_or(SAVESTRINGSIZE);
    state.save_char_index = end as i32;
}

fn m_do_save(state: &mut MenuState, game: &mut GameCtrl, args: &Args, slot: i32) {
    let si = (slot as usize).min(state.savegame_strings.len() - 1);
    let _name = build_save_name(si, args);
    let desc = save_string_to_str(&state.savegame_strings[si]);
    game_ctrl::g_save_game(game, slot, &desc);
    m_clear_menus(state);
    if state.quick_save_slot == -1 {
        state.quick_save_slot = slot;
    }
}

fn cb_read_this2(state: &mut MenuState) {
    m_setup_next_menu(state, MENU_READ2);
}

fn cb_finish_read_this(state: &mut MenuState) {
    m_setup_next_menu(state, MENU_MAIN);
}

fn cb_quick_save(
    state: &mut MenuState,
    game: &GameCtrl,
    audio: &mut dyn AudioBackend,
    args: &Args,
) {
    if !game.usergame {
        audio.start_sound(SfxEnum::sfx_oof as i32, 128, 128, 128, 0);
        return;
    }
    if game.gamestate != GameState::Level {
        return;
    }
    if state.quick_save_slot < 0 {
        m_start_message(state, strings::QSAVESPOT, None, false);
        m_setup_next_menu(state, MENU_SAVE);
        m_read_save_strings(state, args);
        return;
    }
    let qsi = (state.quick_save_slot as usize).min(state.savegame_strings.len() - 1);
    let desc = save_string_to_str(&state.savegame_strings[qsi]);
    state.temp_string = format!("{}{}\n\n{}", strings::QSPROMPT, desc, strings::PRESSYN);
    let msg = state.temp_string.clone();
    m_start_message(state, &msg, Some(MenuCallback::QuickSaveResponse), true);
}

fn cb_quick_save_response(choice: i32, state: &mut MenuState, game: &mut GameCtrl, args: &Args) {
    if choice != b'y' as i32 {
        return;
    }
    let slot = state.quick_save_slot;
    m_do_save(state, game, args, slot);
}

fn cb_quick_load(
    state: &mut MenuState,
    game: &GameCtrl,
    _audio: &mut dyn AudioBackend,
    _args: &Args,
) {
    if game.netgame {
        m_start_message(state, strings::QLOADNET, None, false);
        return;
    }
    if state.quick_save_slot < 0 {
        m_start_message(state, strings::QSAVESPOT, None, false);
        return;
    }
    let qsi = (state.quick_save_slot as usize).min(state.savegame_strings.len() - 1);
    let desc = save_string_to_str(&state.savegame_strings[qsi]);
    state.temp_string = format!("{}{}\n\n{}", strings::QLPROMPT, desc, strings::PRESSYN);
    let msg = state.temp_string.clone();
    m_start_message(state, &msg, Some(MenuCallback::QuickLoadResponse), true);
}

fn cb_quick_load_response(choice: i32, state: &mut MenuState, game: &mut GameCtrl, args: &Args) {
    if choice != b'y' as i32 {
        return;
    }
    let name = build_save_name(state.quick_save_slot as usize, args);
    game_ctrl::g_load_game(game, &name.to_string_lossy());
    m_clear_menus(state);
}

fn cb_quit_response(
    choice: i32,
    _state: &mut MenuState,
    game: &GameCtrl,
    audio: &mut dyn AudioBackend,
    platform: &mut dyn PlatformHost,
) {
    if choice != b'y' as i32 {
        return;
    }
    if game.gamemode == GameMode::Commercial {
        let idx = ((game.gametic >> 2) & 7) as usize;
        let sfx = QUITSOUNDS2[idx.min(QUITSOUNDS2.len() - 1)];
        audio.start_sound(sfx as i32, 128, 128, 128, 0);
    } else {
        let idx = ((game.gametic >> 2) & 7) as usize;
        let sfx = QUITSOUNDS[idx.min(QUITSOUNDS.len() - 1)];
        audio.start_sound(sfx as i32, 128, 128, 128, 0);
    }
    platform.quit();
}

// =============================================================================
// Text rendering helpers
// =============================================================================

/// Compute the pixel width of a string rendered with the HUD font.
///
/// Equivalent of `M_StringWidth` (m_menu.c lines 1254-1270).
pub fn m_string_width(s: &str, hud: &HudState) -> i32 {
    let mut width: i32 = 0;
    for ch in s.bytes() {
        let c = ch as i32;
        if c == b'\n' as i32 {
            break;
        }
        let idx = c - HU_FONTSTART as i32;
        if idx < 0 || idx >= HU_FONTSIZE as i32 {
            width += 4; // space character
        } else if let Some(patch_data) = hud.hu_font.get(idx as usize) {
            if patch_data.len() >= 8 {
                let w = i16::from_le_bytes([patch_data[0], patch_data[1]]);
                width += swap::short(w) as i32;
            } else {
                width += 4;
            }
        } else {
            width += 4;
        }
    }
    width
}

/// Compute the pixel height of a (possibly multi-line) string.
///
/// Equivalent of `M_StringHeight` (m_menu.c lines 1274-1290).
pub fn m_string_height(s: &str, _hud: &HudState) -> i32 {
    let mut height: i32 = 8; // patch height of font character (hu_font)
    for ch in s.bytes() {
        if ch == b'\n' {
            height += 8; // linefeed increases height
        }
    }
    height
}

/// Draw a text string at (x, y) using HUD font patches.
///
/// Equivalent of `M_WriteText` (m_menu.c lines 1293-1338).
/// Supports '\n' for line breaks. Characters outside the HUD font range
/// are treated as spaces (4-pixel advance).
pub fn m_write_text(x: i32, y: i32, string: &str, video: &mut VideoState, hud: &HudState) {
    let mut cx = x;
    let mut cy = y;
    for ch in string.bytes() {
        let c = ch;
        if c == b'\n' {
            cx = x;
            cy += 12;
            continue;
        }
        if c == b'\t' {
            // Tab → skip ahead.
            cx += 4;
            continue;
        }
        let idx = (c as i32) - HU_FONTSTART as i32;
        if idx < 0 || idx >= HU_FONTSIZE as i32 {
            cx += 4; // space
            continue;
        }
        if let Some(patch_data) = hud.hu_font.get(idx as usize) {
            if patch_data.len() >= 8 {
                let w = i16::from_le_bytes([patch_data[0], patch_data[1]]);
                let pw = swap::short(w) as i32;
                if cx + pw > SCREENWIDTH {
                    break; // don't draw past screen edge
                }
                video.draw_patch_direct(cx, cy, 0, patch_data);
                cx += pw;
            } else {
                cx += 4;
            }
        } else {
            cx += 4;
        }
    }
}

// =============================================================================
// Drawing helper functions
// =============================================================================

/// Draw a thermometer/slider widget.
///
/// Equivalent of `M_DrawThermo` (m_menu.c lines 1182-1210).
/// Draws the left cap, middle segments, right cap, and position gem.
pub fn m_draw_thermo(
    x: i32,
    y: i32,
    therm_width: i32,
    therm_dot: i32,
    video: &mut VideoState,
    wad: &mut dyn WadProvider,
) {
    // Left cap
    let left = wad.cache_lump_name("M_THERML", PurgeTag::Cache);
    video.draw_patch_direct(x, y, 0, left);

    // Middle segments
    for i in 0..therm_width {
        let mid = wad.cache_lump_name("M_THERMM", PurgeTag::Cache);
        video.draw_patch_direct(x + 8 + i * 8, y, 0, mid);
    }

    // Right cap
    let right = wad.cache_lump_name("M_THERMR", PurgeTag::Cache);
    video.draw_patch_direct(x + 8 + therm_width * 8, y, 0, right);

    // Position gem
    let gem = wad.cache_lump_name("M_THERMO", PurgeTag::Cache);
    video.draw_patch_direct(x + 8 + therm_dot * 8, y, 0, gem);
}

/// Draw an unselected (empty) menu cell graphic.
///
/// Equivalent of `M_DrawEmptyCell` (m_menu.c lines 1164-1170).
pub fn m_draw_empty_cell(
    menu: &Menu,
    item: i16,
    video: &mut VideoState,
    wad: &mut dyn WadProvider,
) {
    let patch = wad.cache_lump_name("M_CELL1", PurgeTag::Cache);
    video.draw_patch_direct(
        menu.x as i32 - 10,
        menu.y as i32 + (item as i32) * LINEHEIGHT - 1,
        0,
        patch,
    );
}

/// Draw a selected (filled) menu cell graphic.
///
/// Equivalent of `M_DrawSelCell` (m_menu.c lines 1174-1180).
pub fn m_draw_sel_cell(menu: &Menu, item: i16, video: &mut VideoState, wad: &mut dyn WadProvider) {
    let patch = wad.cache_lump_name("M_CELL2", PurgeTag::Cache);
    video.draw_patch_direct(
        menu.x as i32 - 10,
        menu.y as i32 + (item as i32) * LINEHEIGHT - 1,
        0,
        patch,
    );
}

/// Draw the save/load border decoration.
///
/// Equivalent of `M_DrawSaveLoadBorder` (m_menu.c lines 507-530).
pub fn m_draw_save_load_border(x: i32, y: i32, video: &mut VideoState, wad: &mut dyn WadProvider) {
    let left = wad.cache_lump_name("M_LSLEFT", PurgeTag::Cache);
    video.draw_patch_direct(x - 8, y + 7, 0, left);

    for i in 0..24 {
        let mid = wad.cache_lump_name("M_LSCNTR", PurgeTag::Cache);
        video.draw_patch_direct(x + i * 8, y + 7, 0, mid);
    }

    let right = wad.cache_lump_name("M_LSRGHT", PurgeTag::Cache);
    video.draw_patch_direct(x + 24 * 8, y + 7, 0, right);
}

// =============================================================================
// Menu-specific draw routines
// =============================================================================

/// Draw the main menu screen.
///
/// Equivalent of `M_DrawMainMenu` (m_menu.c lines 750-756).
fn draw_main_menu(video: &mut VideoState, wad: &mut dyn WadProvider) {
    let patch = wad.cache_lump_name("M_DOOM", PurgeTag::Cache);
    video.draw_patch_direct(94, 2, 0, patch);
}

/// Draw the new game skill selection menu.
///
/// Equivalent of `M_DrawNewGame` (m_menu.c lines 792-800).
fn draw_new_game(video: &mut VideoState, wad: &mut dyn WadProvider) {
    let patch = wad.cache_lump_name("M_NEWG", PurgeTag::Cache);
    video.draw_patch_direct(96, 14, 0, patch);

    let skill_patch = wad.cache_lump_name("M_SKILL", PurgeTag::Cache);
    video.draw_patch_direct(54, 38, 0, skill_patch);
}

/// Draw the episode selection menu.
///
/// Equivalent of `M_DrawEpisode` (m_menu.c lines 828-831).
fn draw_episode(video: &mut VideoState, wad: &mut dyn WadProvider) {
    let patch = wad.cache_lump_name("M_EPISOD", PurgeTag::Cache);
    video.draw_patch_direct(54, 38, 0, patch);
}

/// Draw the options menu.
///
/// Equivalent of `M_DrawOptions` (m_menu.c lines 970-994).
fn draw_options(
    state: &MenuState,
    video: &mut VideoState,
    wad: &mut dyn WadProvider,
    hud: &HudState,
) {
    let patch = wad.cache_lump_name("M_OPTTTL", PurgeTag::Cache);
    video.draw_patch_direct(108, 15, 0, patch);

    // Sensitivity thermometer
    m_draw_thermo(
        state.menus[MENU_OPTIONS].x as i32,
        state.menus[MENU_OPTIONS].y as i32 + LINEHEIGHT * 5 + 2,
        10,
        state.mouse_sensitivity,
        video,
        wad,
    );

    // Screen size thermometer
    m_draw_thermo(
        state.menus[MENU_OPTIONS].x as i32,
        state.menus[MENU_OPTIONS].y as i32 + LINEHEIGHT * 3 + 2,
        9,
        state.screen_size,
        video,
        wad,
    );

    // Messages and Detail text
    m_write_text(
        state.menus[MENU_OPTIONS].x as i32 + 120,
        state.menus[MENU_OPTIONS].y as i32 + LINEHEIGHT,
        MSG_NAMES[state.show_messages.clamp(0, 1) as usize],
        video,
        hud,
    );

    m_write_text(
        state.menus[MENU_OPTIONS].x as i32 + 133,
        state.menus[MENU_OPTIONS].y as i32 + LINEHEIGHT * 2,
        DETAIL_NAMES[state.detail_level.clamp(0, 1) as usize],
        video,
        hud,
    );
}

/// Draw the sound volume menu.
///
/// Equivalent of `M_DrawSound` (m_menu.c lines 753-770).
fn draw_sound(state: &MenuState, video: &mut VideoState, wad: &mut dyn WadProvider) {
    let patch = wad.cache_lump_name("M_SVOL", PurgeTag::Cache);
    video.draw_patch_direct(60, 38, 0, patch);

    // SFX volume thermometer
    m_draw_thermo(
        state.menus[MENU_SOUND].x as i32,
        state.menus[MENU_SOUND].y as i32 + LINEHEIGHT + 2,
        16,
        state.sfx_volume,
        video,
        wad,
    );

    // Music volume thermometer
    m_draw_thermo(
        state.menus[MENU_SOUND].x as i32,
        state.menus[MENU_SOUND].y as i32 + LINEHEIGHT * 3 + 2,
        16,
        state.music_volume,
        video,
        wad,
    );
}

/// Draw the load game menu.
///
/// Equivalent of `M_DrawLoad` (m_menu.c lines 535-558).
fn draw_load(state: &MenuState, video: &mut VideoState, wad: &mut dyn WadProvider, hud: &HudState) {
    let patch = wad.cache_lump_name("M_LOADG", PurgeTag::Cache);
    video.draw_patch_direct(72, 28, 0, patch);

    for i in 0..6i32 {
        m_draw_save_load_border(
            state.menus[MENU_LOAD].x as i32,
            state.menus[MENU_LOAD].y as i32 + LINEHEIGHT * i,
            video,
            wad,
        );
        let text = save_string_to_str(&state.savegame_strings[i as usize]);
        m_write_text(
            state.menus[MENU_LOAD].x as i32,
            state.menus[MENU_LOAD].y as i32 + LINEHEIGHT * i,
            &text,
            video,
            hud,
        );
    }
}

/// Draw the save game menu.
///
/// Equivalent of `M_DrawSave` (m_menu.c lines 610-640).
fn draw_save(state: &MenuState, video: &mut VideoState, wad: &mut dyn WadProvider, hud: &HudState) {
    let patch = wad.cache_lump_name("M_SAVEG", PurgeTag::Cache);
    video.draw_patch_direct(72, 28, 0, patch);

    for i in 0..6i32 {
        m_draw_save_load_border(
            state.menus[MENU_SAVE].x as i32,
            state.menus[MENU_SAVE].y as i32 + LINEHEIGHT * i,
            video,
            wad,
        );
        let text = save_string_to_str(&state.savegame_strings[i as usize]);
        m_write_text(
            state.menus[MENU_SAVE].x as i32,
            state.menus[MENU_SAVE].y as i32 + LINEHEIGHT * i,
            &text,
            video,
            hud,
        );
    }

    // Draw cursor during string editing
    if state.save_string_enter != 0 {
        let slot = state.save_slot as usize;
        let text = save_string_to_str(&state.savegame_strings[slot]);
        let cursor_x = state.menus[MENU_SAVE].x as i32 + m_string_width(&text, hud);
        let cursor_y = state.menus[MENU_SAVE].y as i32 + LINEHEIGHT * state.save_slot;
        // Draw underscore cursor
        m_write_text(cursor_x, cursor_y, "_", video, hud);
    }
}

/// Draw the "Read This 1" help screen.
///
/// Equivalent of `M_DrawReadThis1` (m_menu.c lines 1067-1072).
fn draw_read_this1(
    state: &mut MenuState,
    game: &GameCtrl,
    video: &mut VideoState,
    wad: &mut dyn WadProvider,
) {
    state.inhelpscreens = true;
    let lump_name = match game.gamemode {
        GameMode::Commercial => "HELP",
        GameMode::Retail => "HELP1",
        _ => "HELP1",
    };
    let patch = wad.cache_lump_name(lump_name, PurgeTag::Cache);
    video.draw_patch_direct(0, 0, 0, patch);
}

/// Draw the "Read This 2" help screen.
///
/// Equivalent of `M_DrawReadThis2` (m_menu.c lines 1074-1078).
fn draw_read_this2(
    state: &mut MenuState,
    game: &GameCtrl,
    video: &mut VideoState,
    wad: &mut dyn WadProvider,
) {
    state.inhelpscreens = true;
    let lump_name = match game.gamemode {
        GameMode::Commercial => "HELP",
        GameMode::Retail => "CREDIT",
        _ => "HELP2",
    };
    let patch = wad.cache_lump_name(lump_name, PurgeTag::Cache);
    video.draw_patch_direct(0, 0, 0, patch);
}

// =============================================================================
// m_ticker — Skull cursor animation
// =============================================================================

/// Animate the skull cursor.
///
/// Equivalent of `M_Ticker` (m_menu.c lines 1827-1842).
/// Called once per game tic. Decrements `skull_anim_counter` and toggles
/// `which_skull` between 0 and 1 every 8 tics.
pub fn m_ticker(state: &mut MenuState) {
    state.skull_anim_counter -= 1;
    if state.skull_anim_counter <= 0 {
        state.which_skull ^= 1;
        state.skull_anim_counter = 8;
    }
}

// =============================================================================
// m_drawer — Draw the current menu
// =============================================================================

/// Skull cursor patch names.
const SKULL_NAMES: [&str; 2] = ["M_SKULL1", "M_SKULL2"];

/// Draw the entire menu overlay.
///
/// Equivalent of `M_Drawer` (m_menu.c lines 1755-1824).
/// If a message is pending, draws the message box. Otherwise draws the
/// current menu: background, title, item names, skull cursor, and any
/// active thermometers or text fields.
pub fn m_drawer(
    state: &mut MenuState,
    game: &GameCtrl,
    video: &mut VideoState,
    wad: &mut dyn WadProvider,
    hud: &HudState,
) {
    state.inhelpscreens = false;

    // Message box takes priority.
    if state.message_to_print != 0 {
        if let Some(ref msg) = state.message_string.clone() {
            // Center the message on screen.
            let lines: Vec<&str> = msg.split('\n').collect();
            let mut y = SCREENHEIGHT / 2 - (m_string_height(msg, hud)) / 2;
            for line in &lines {
                let x = SCREENWIDTH / 2 - m_string_width(line, hud) / 2;
                m_write_text(x, y, line, video, hud);
                y += 8; // hu_font character height
            }
        }
        return;
    }

    if !state.menu_active {
        return;
    }

    // Dispatch the menu-specific draw routine.
    let draw_routine = if state.current_menu < state.menus.len() {
        state.menus[state.current_menu].draw_routine
    } else {
        MenuDraw::MainMenu
    };

    match draw_routine {
        MenuDraw::MainMenu => draw_main_menu(video, wad),
        MenuDraw::NewGame => draw_new_game(video, wad),
        MenuDraw::Episode => draw_episode(video, wad),
        MenuDraw::Options => draw_options(state, video, wad, hud),
        MenuDraw::Sound => draw_sound(state, video, wad),
        MenuDraw::Load => draw_load(state, video, wad, hud),
        MenuDraw::Save => draw_save(state, video, wad, hud),
        MenuDraw::ReadThis1 => draw_read_this1(state, game, video, wad),
        MenuDraw::ReadThis2 => draw_read_this2(state, game, video, wad),
    }

    // Draw menu items.
    let menu_idx = state.current_menu;
    if menu_idx < state.menus.len() {
        let mx = state.menus[menu_idx].x as i32;
        let my = state.menus[menu_idx].y as i32;
        let num = state.menus[menu_idx].num_items as usize;
        for i in 0..num {
            if i < state.menus[menu_idx].menu_items.len() {
                let name = state.menus[menu_idx].menu_items[i].name_str();
                if !name.is_empty() {
                    let patch = wad.cache_lump_name(name, PurgeTag::Cache);
                    video.draw_patch_direct(mx, my + (i as i32) * LINEHEIGHT, 0, patch);
                }
            }
        }

        // Draw the skull cursor.
        let skull_idx = (state.which_skull as usize) & 1;
        let skull_name = SKULL_NAMES[skull_idx];
        let skull = wad.cache_lump_name(skull_name, PurgeTag::Cache);
        video.draw_patch_direct(
            mx + SKULLXOFF,
            my + (state.item_on as i32) * LINEHEIGHT - 5,
            0,
            skull,
        );
    }
}

// =============================================================================
// m_responder — Main event handler
// =============================================================================

/// Process an input event for the menu system.
///
/// Equivalent of `M_Responder` (m_menu.c lines 1349-1716).
/// Returns `true` if the event was consumed by the menu, `false` otherwise.
///
/// Handles:
/// - Joystick/mouse axis-to-key conversion
/// - Save game string editing
/// - Message box acknowledgement/response
/// - F-key global shortcuts (F1-F12)
/// - Menu navigation (arrows, Enter, Escape)
/// - Alphanumeric hotkey selection
pub fn m_responder(
    ev: &Event,
    state: &mut MenuState,
    game: &mut GameCtrl,
    video: &mut VideoState,
    audio: &mut dyn AudioBackend,
    renderer: &mut dyn Renderer,
    platform: &mut dyn PlatformHost,
    hud: &HudState,
    automap: &AutomapState,
    args: &Args,
) -> bool {
    // Determine the key code from the event. Only key-down and mouse-motion
    // events are handled. Mouse motion is converted to arrow key equivalents
    // for menu navigation.
    let ch: i32 = match ev.event_type {
        EventType::KeyDown => ev.data1,
        EventType::Mouse => {
            if !state.menu_active {
                return false;
            }
            // Mouse Y axis → up/down.
            state.mousey += ev.data3; // data3 = dy
            if state.mousey < state.lasty - 30 {
                state.mousewait = state.input_tic + 5;
                state.lasty -= 30;
                doomdef::KEY_DOWNARROW
            } else if state.mousey > state.lasty + 30 {
                state.mousewait = state.input_tic + 5;
                state.lasty += 30;
                doomdef::KEY_UPARROW
            } else {
                // Mouse X axis → left/right (for sliders).
                state.mousex += ev.data2; // data2 = dx
                if state.mousex < state.lastx - 30 {
                    state.mousewait = state.input_tic + 5;
                    state.lastx -= 30;
                    doomdef::KEY_LEFTARROW
                } else if state.mousex > state.lastx + 30 {
                    state.mousewait = state.input_tic + 5;
                    state.lastx += 30;
                    doomdef::KEY_RIGHTARROW
                } else {
                    return false;
                }
            }
        }
        EventType::Joystick => {
            // Joystick axis/button → key equivalents (rate-limited).
            // Original C: m_menu.c lines 1359-1389.
            let cur_time = state.input_tic;
            if state.joywait >= cur_time {
                return false;
            }
            let mut jch: i32 = -1;
            // Y axis
            if ev.data3 == -1 {
                jch = doomdef::KEY_UPARROW;
                state.joywait = cur_time + 5;
            } else if ev.data3 == 1 {
                jch = doomdef::KEY_DOWNARROW;
                state.joywait = cur_time + 5;
            }
            // X axis
            if ev.data2 == -1 {
                jch = doomdef::KEY_LEFTARROW;
                state.joywait = cur_time + 5;
            } else if ev.data2 == 1 {
                jch = doomdef::KEY_RIGHTARROW;
                state.joywait = cur_time + 5;
            }
            // Buttons
            if ev.data1 & 1 != 0 {
                jch = doomdef::KEY_ENTER;
                state.joywait = cur_time + 5;
            }
            if ev.data1 & 2 != 0 {
                jch = doomdef::KEY_BACKSPACE;
                state.joywait = cur_time + 5;
            }
            if jch < 0 {
                return false;
            }
            jch
        }
        _ => {
            return false;
        }
    };

    // -------------------------------------------------------------------
    // Save string editing mode
    // -------------------------------------------------------------------
    if state.save_string_enter != 0 {
        match ch {
            k if k == doomdef::KEY_BACKSPACE => {
                if state.save_char_index > 0 {
                    state.save_char_index -= 1;
                    let idx = state.save_char_index as usize;
                    let slot = state.save_slot as usize;
                    state.savegame_strings[slot][idx] = 0;
                }
                return true;
            }
            k if k == doomdef::KEY_ESCAPE => {
                // Cancel editing — restore old string.
                state.save_string_enter = 0;
                let slot = state.save_slot as usize;
                state.savegame_strings[slot] = state.save_old_string;
                return true;
            }
            k if k == doomdef::KEY_ENTER => {
                // Confirm save.
                state.save_string_enter = 0;
                let slot = state.save_slot;
                m_do_save(state, game, args, slot);
                return true;
            }
            _ => {
                // Add character if printable and there's room.
                let c = ch as u8;
                if (32..=127).contains(&c) && (state.save_char_index as usize) < SAVESTRINGSIZE - 1
                {
                    let idx = state.save_char_index as usize;
                    let slot = state.save_slot as usize;
                    state.savegame_strings[slot][idx] = c.to_ascii_uppercase();
                    state.save_char_index += 1;
                    state.savegame_strings[slot][state.save_char_index as usize] = 0;
                }
                return true;
            }
        }
    }

    // -------------------------------------------------------------------
    // Message response mode
    // -------------------------------------------------------------------
    if state.message_to_print != 0 {
        if state.message_needs_input {
            // Needs Y/N or Enter/Escape.
            if ch != b'y' as i32
                && ch != b'n' as i32
                && ch != b' ' as i32
                && ch != doomdef::KEY_ESCAPE
                && ch != doomdef::KEY_ENTER
            {
                return false;
            }
        }

        state.message_to_print = 0;
        let routine = state.message_routine;
        // If there's a callback, dispatch it with the key as the choice.
        if let Some(cb) = routine {
            state.menu_active = state.message_last_menu_active;
            // Must dispatch before clearing, so clone anything needed.
            dispatch_callback(
                cb, ch, state, game, video, audio, renderer, platform, hud, args,
            );
        } else {
            state.menu_active = state.message_last_menu_active;
        }

        audio.start_sound(SfxEnum::sfx_swtchx as i32, 128, 128, 128, 0);
        return true;
    }

    // -------------------------------------------------------------------
    // F-key shortcuts (work regardless of menu state, during gameplay)
    // -------------------------------------------------------------------
    if !state.menu_active {
        match ch {
            k if k == doomdef::KEY_MINUS => {
                // Decrease screen size.
                if automap.automapactive || hud.chat_on {
                    return false;
                }
                cb_size_display(0, state, renderer);
                audio.start_sound(SfxEnum::sfx_stnmov as i32, 128, 128, 128, 0);
                return true;
            }
            k if k == doomdef::KEY_EQUALS => {
                // Increase screen size.
                if automap.automapactive || hud.chat_on {
                    return false;
                }
                cb_size_display(1, state, renderer);
                audio.start_sound(SfxEnum::sfx_stnmov as i32, 128, 128, 128, 0);
                return true;
            }
            k if k == doomdef::KEY_F1 => {
                // Help screen.
                cb_read_this(state, game);
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                return true;
            }
            k if k == doomdef::KEY_F2 => {
                // Save game.
                cb_save_game(state, game, args);
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                return true;
            }
            k if k == doomdef::KEY_F3 => {
                // Load game.
                cb_load_game(state, game, args);
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                return true;
            }
            k if k == doomdef::KEY_F4 => {
                // Sound volume.
                cb_sound(state);
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                return true;
            }
            k if k == doomdef::KEY_F5 => {
                // Toggle detail.
                cb_change_detail(state, game);
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                return true;
            }
            k if k == doomdef::KEY_F6 => {
                // Quick save.
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                cb_quick_save(state, game, audio, args);
                return true;
            }
            k if k == doomdef::KEY_F7 => {
                // End game.
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                cb_end_game(state, game, audio);
                return true;
            }
            k if k == doomdef::KEY_F8 => {
                // Toggle messages.
                cb_change_messages(state, game);
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                return true;
            }
            k if k == doomdef::KEY_F9 => {
                // Quick load.
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                cb_quick_load(state, game, audio, args);
                return true;
            }
            k if k == doomdef::KEY_F10 => {
                // Quit game.
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
                cb_quit_doom(state, game, audio);
                return true;
            }
            k if k == doomdef::KEY_F11 => {
                // Gamma correction toggle.
                video.usegamma = (video.usegamma + 1) % (GAMMATABLE.len() as i32);
                let gi = (video.usegamma as usize).min(GAMMAMSG.len() - 1);
                let msg = GAMMAMSG[gi];
                if let Some(player) = game.players.get_mut(game.consoleplayer) {
                    player.message = Some(msg.to_string());
                }
                // I_SetPalette would be called here — the platform layer
                // will apply gamma on next V_FinishUpdate. Palette refresh
                // is driven by the caller.
                return true;
            }
            k if k == doomdef::KEY_F12 => {
                // Screenshot.
                game_ctrl::g_screen_shot(game);
                return true;
            }
            k if k == doomdef::KEY_PAUSE => {
                // Pause toggle.
                game.sendpause = true;
                return true;
            }
            _ => {
                return false;
            }
        }
    }

    // -------------------------------------------------------------------
    // Menu navigation (menu is active)
    // -------------------------------------------------------------------
    let num_items = if state.current_menu < state.menus.len() {
        state.menus[state.current_menu].num_items
    } else {
        0
    };

    match ch {
        k if k == doomdef::KEY_DOWNARROW => {
            // Move cursor down.
            loop {
                state.item_on += 1;
                if state.item_on >= num_items {
                    state.item_on = 0;
                }
                // Skip items with status -1 (not selectable).
                let idx = state.item_on as usize;
                if idx < state.menus[state.current_menu].menu_items.len()
                    && state.menus[state.current_menu].menu_items[idx].status != -1
                {
                    break;
                }
            }
            audio.start_sound(SfxEnum::sfx_pstop as i32, 128, 128, 128, 0);
            true
        }
        k if k == doomdef::KEY_UPARROW => {
            // Move cursor up.
            loop {
                state.item_on -= 1;
                if state.item_on < 0 {
                    state.item_on = num_items - 1;
                }
                let idx = state.item_on as usize;
                if idx < state.menus[state.current_menu].menu_items.len()
                    && state.menus[state.current_menu].menu_items[idx].status != -1
                {
                    break;
                }
            }
            audio.start_sound(SfxEnum::sfx_pstop as i32, 128, 128, 128, 0);
            true
        }
        k if k == doomdef::KEY_LEFTARROW => {
            // Adjust slider left.
            let idx = state.item_on as usize;
            if idx < state.menus[state.current_menu].menu_items.len() {
                let status = state.menus[state.current_menu].menu_items[idx].status;
                if status == 2 {
                    // This is a slider item.
                    let cb = state.menus[state.current_menu].menu_items[idx].routine;
                    audio.start_sound(SfxEnum::sfx_stnmov as i32, 128, 128, 128, 0);
                    dispatch_callback(
                        cb, 0, state, game, video, audio, renderer, platform, hud, args,
                    );
                }
            }
            true
        }
        k if k == doomdef::KEY_RIGHTARROW => {
            // Adjust slider right.
            let idx = state.item_on as usize;
            if idx < state.menus[state.current_menu].menu_items.len() {
                let status = state.menus[state.current_menu].menu_items[idx].status;
                if status == 2 {
                    let cb = state.menus[state.current_menu].menu_items[idx].routine;
                    audio.start_sound(SfxEnum::sfx_stnmov as i32, 128, 128, 128, 0);
                    dispatch_callback(
                        cb, 1, state, game, video, audio, renderer, platform, hud, args,
                    );
                }
            }
            true
        }
        k if k == doomdef::KEY_ENTER => {
            // Select the current item.
            let idx = state.item_on as usize;
            if idx < state.menus[state.current_menu].menu_items.len() {
                let status = state.menus[state.current_menu].menu_items[idx].status;
                if status != 0 {
                    state.menus[state.current_menu].last_on = state.item_on;
                    let cb = state.menus[state.current_menu].menu_items[idx].routine;
                    audio.start_sound(SfxEnum::sfx_pistol as i32, 128, 128, 128, 0);
                    dispatch_callback(
                        cb,
                        state.item_on as i32,
                        state,
                        game,
                        video,
                        audio,
                        renderer,
                        platform,
                        hud,
                        args,
                    );
                }
            }
            true
        }
        k if k == doomdef::KEY_ESCAPE => {
            // Go back to previous menu or close.
            state.menus[state.current_menu].last_on = state.item_on;
            let prev = if state.current_menu < state.menus.len() {
                state.menus[state.current_menu].prev_menu
            } else {
                None
            };
            if let Some(prev_idx) = prev {
                state.current_menu = prev_idx;
                state.item_on = state.menus[prev_idx].last_on;
                audio.start_sound(SfxEnum::sfx_swtchn as i32, 128, 128, 128, 0);
            } else {
                m_clear_menus(state);
                audio.start_sound(SfxEnum::sfx_swtchx as i32, 128, 128, 128, 0);
            }
            true
        }
        _ => {
            // Check for alphanumeric hotkey match.
            let hotkey = (ch as u8).to_ascii_lowercase();
            let menu_idx = state.current_menu;
            if menu_idx < state.menus.len() {
                let n = state.menus[menu_idx].num_items as usize;
                for i in (state.item_on as usize + 1)..n {
                    if i < state.menus[menu_idx].menu_items.len()
                        && state.menus[menu_idx].menu_items[i].alpha_key == hotkey
                    {
                        state.item_on = i as i16;
                        audio.start_sound(SfxEnum::sfx_pstop as i32, 128, 128, 128, 0);
                        return true;
                    }
                }
                // Wrap around from the beginning.
                for i in 0..=(state.item_on as usize) {
                    if i < state.menus[menu_idx].menu_items.len()
                        && state.menus[menu_idx].menu_items[i].alpha_key == hotkey
                    {
                        state.item_on = i as i16;
                        audio.start_sound(SfxEnum::sfx_pstop as i32, 128, 128, 128, 0);
                        return true;
                    }
                }
            }
            false
        }
    }
}
