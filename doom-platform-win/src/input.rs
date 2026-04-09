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

//! Keyboard and mouse input translation — SDL2 events to DOOM Event enum.
//!
//! Translated from `linuxdoom-1.10/i_video.c` (lines 97-338).
//! Replaces X11 XEvent handling (KeyPress, KeyRelease, ButtonPress,
//! ButtonRelease, MotionNotify) with SDL2 event pump polling.
//!
//! ## Key Mapping
//! The `xlatekey()` function (i_video.c:97-162) maps X11 keysyms to DOOM's
//! internal key constants. This module maps SDL2 keycodes to the same DOOM
//! key constants, preserving the exact same key bindings.
//!
//! ## Mouse Handling
//! Mouse button state is tracked as a 3-bit bitmask (left=bit 0, middle=bit 1,
//! right=bit 2), matching the original X11 button mask encoding in
//! `I_GetEvent()` (i_video.c:215-268). Mouse motion deltas are scaled by `<< 2`
//! with Y-axis inverted (screen Y-down → DOOM Y-up), matching i_video.c:250-251.
//!
//! ## Event Flow
//! The platform layer calls [`process_sdl_events`] each tic with the batch of
//! SDL2 events collected from the event pump. The function returns a `Vec<Event>`
//! that the caller posts into the DOOM event queue via `D_PostEvent`.

use sdl2::event::Event as SdlEvent;
use sdl2::keyboard::Keycode;
use sdl2::mouse::MouseButton;
use tracing::debug;

use doom_core::types::event::{Event, EventType};

// =============================================================================
// DOOM keyboard scan-code constants
// =============================================================================
//
// These values are defined locally for platform self-containment. They MUST
// exactly match the constants in doom-core/src/types/doomdef.rs, which in turn
// match the original `doomdef.h` lines 250-280 from linuxdoom-1.10.
//
// The `xlatekey()` function in i_video.c:97-162 translates X11 keysyms into
// these constants. This module translates SDL2 keycodes into the same values.

/// Right arrow key scan code. `#define KEY_RIGHTARROW 0xae`
pub const KEY_RIGHTARROW: i32 = 0xae;
/// Left arrow key scan code. `#define KEY_LEFTARROW 0xac`
pub const KEY_LEFTARROW: i32 = 0xac;
/// Up arrow key scan code. `#define KEY_UPARROW 0xad`
pub const KEY_UPARROW: i32 = 0xad;
/// Down arrow key scan code. `#define KEY_DOWNARROW 0xaf`
pub const KEY_DOWNARROW: i32 = 0xaf;

/// Escape key. `#define KEY_ESCAPE 27`
pub const KEY_ESCAPE: i32 = 27;
/// Enter / Return key. `#define KEY_ENTER 13`
pub const KEY_ENTER: i32 = 13;
/// Tab key. `#define KEY_TAB 9`
pub const KEY_TAB: i32 = 9;

/// Function key F1. `#define KEY_F1 (0x80+0x3b)`
pub const KEY_F1: i32 = 0x80 + 0x3b;
/// Function key F2. `#define KEY_F2 (0x80+0x3c)`
pub const KEY_F2: i32 = 0x80 + 0x3c;
/// Function key F3. `#define KEY_F3 (0x80+0x3d)`
pub const KEY_F3: i32 = 0x80 + 0x3d;
/// Function key F4. `#define KEY_F4 (0x80+0x3e)`
pub const KEY_F4: i32 = 0x80 + 0x3e;
/// Function key F5. `#define KEY_F5 (0x80+0x3f)`
pub const KEY_F5: i32 = 0x80 + 0x3f;
/// Function key F6. `#define KEY_F6 (0x80+0x40)`
pub const KEY_F6: i32 = 0x80 + 0x40;
/// Function key F7. `#define KEY_F7 (0x80+0x41)`
pub const KEY_F7: i32 = 0x80 + 0x41;
/// Function key F8. `#define KEY_F8 (0x80+0x42)`
pub const KEY_F8: i32 = 0x80 + 0x42;
/// Function key F9. `#define KEY_F9 (0x80+0x43)`
pub const KEY_F9: i32 = 0x80 + 0x43;
/// Function key F10. `#define KEY_F10 (0x80+0x44)`
pub const KEY_F10: i32 = 0x80 + 0x44;
/// Function key F11. `#define KEY_F11 (0x80+0x57)`
pub const KEY_F11: i32 = 0x80 + 0x57;
/// Function key F12. `#define KEY_F12 (0x80+0x58)`
pub const KEY_F12: i32 = 0x80 + 0x58;

/// Backspace / Delete key. `#define KEY_BACKSPACE 127`
pub const KEY_BACKSPACE: i32 = 127;
/// Pause key. `#define KEY_PAUSE 0xff`
pub const KEY_PAUSE: i32 = 0xff;

/// Equals sign key ('='). `#define KEY_EQUALS 0x3d`
pub const KEY_EQUALS: i32 = 0x3d;
/// Minus / hyphen key ('-'). `#define KEY_MINUS 0x2d`
pub const KEY_MINUS: i32 = 0x2d;

/// Right Shift key. `#define KEY_RSHIFT (0x80+0x36)`
pub const KEY_RSHIFT: i32 = 0x80 + 0x36;
/// Right Control key. `#define KEY_RCTRL (0x80+0x1d)`
pub const KEY_RCTRL: i32 = 0x80 + 0x1d;
/// Right Alt key. `#define KEY_RALT (0x80+0x38)`
pub const KEY_RALT: i32 = 0x80 + 0x38;
/// Left Alt key (aliased to KEY_RALT in the original engine).
/// `#define KEY_LALT KEY_RALT`
pub const KEY_LALT: i32 = KEY_RALT;

// =============================================================================
// Compile-time assertions: verify local KEY_* values match doomdef.h exactly
// =============================================================================

const _: () = assert!(KEY_RIGHTARROW == 0xae, "KEY_RIGHTARROW must be 0xae");
const _: () = assert!(KEY_LEFTARROW == 0xac, "KEY_LEFTARROW must be 0xac");
const _: () = assert!(KEY_UPARROW == 0xad, "KEY_UPARROW must be 0xad");
const _: () = assert!(KEY_DOWNARROW == 0xaf, "KEY_DOWNARROW must be 0xaf");
const _: () = assert!(KEY_ESCAPE == 27, "KEY_ESCAPE must be 27");
const _: () = assert!(KEY_ENTER == 13, "KEY_ENTER must be 13");
const _: () = assert!(KEY_TAB == 9, "KEY_TAB must be 9");
const _: () = assert!(KEY_F1 == 0xbb, "KEY_F1 must be 0xbb");
const _: () = assert!(KEY_F12 == 0xd8, "KEY_F12 must be 0xd8");
const _: () = assert!(KEY_BACKSPACE == 127, "KEY_BACKSPACE must be 127");
const _: () = assert!(KEY_PAUSE == 0xff, "KEY_PAUSE must be 0xff");
const _: () = assert!(KEY_EQUALS == 0x3d, "KEY_EQUALS must be 0x3d");
const _: () = assert!(KEY_MINUS == 0x2d, "KEY_MINUS must be 0x2d");
const _: () = assert!(KEY_RSHIFT == 0xb6, "KEY_RSHIFT must be 0xb6");
const _: () = assert!(KEY_RCTRL == 0x9d, "KEY_RCTRL must be 0x9d");
const _: () = assert!(KEY_RALT == 0xb8, "KEY_RALT must be 0xb8");
const _: () = assert!(KEY_LALT == KEY_RALT, "KEY_LALT must equal KEY_RALT");

// =============================================================================
// Mouse button bitmask constants
// =============================================================================
// Matches the X11 button mask encoding in I_GetEvent() (i_video.c:215-268):
//   Button1 (left)   → bit 0 = 0x01
//   Button2 (middle) → bit 1 = 0x02
//   Button3 (right)  → bit 2 = 0x04

/// Left mouse button bitmask bit (bit 0).
const MOUSE_BUTTON_LEFT: u32 = 1;
/// Middle mouse button bitmask bit (bit 1). Used as strafe in DOOM.
const MOUSE_BUTTON_MIDDLE: u32 = 2;
/// Right mouse button bitmask bit (bit 2). Used as forward in DOOM.
const MOUSE_BUTTON_RIGHT: u32 = 4;

// =============================================================================
// InputState — persistent mouse tracking state
// =============================================================================

/// Tracks persistent input state between event processing calls.
///
/// The original C implementation used file-scope static variables
/// `lastmousex`, `lastmousey` (i_video.c:189-190) and derived button state
/// from the X11 event's modifier mask. This struct consolidates that state
/// into a single owned value.
pub struct InputState {
    /// Last known absolute mouse X position. Updated on each
    /// `MouseMotion` event. Corresponds to `lastmousex` in i_video.c:189.
    pub last_mouse_x: i32,
    /// Last known absolute mouse Y position. Updated on each
    /// `MouseMotion` event. Corresponds to `lastmousey` in i_video.c:190.
    pub last_mouse_y: i32,
    /// Current mouse button bitmask: bit 0 = left, bit 1 = middle,
    /// bit 2 = right. Updated on `MouseButtonDown` and `MouseButtonUp`
    /// events. Used as `event.data1` for mouse events.
    pub mouse_button_state: u32,
}

impl InputState {
    /// Creates a new `InputState` with all fields zeroed, matching the
    /// original static variable initialization in i_video.c:189-190
    /// (`lastmousex = 0; lastmousey = 0;`).
    #[inline]
    pub fn new() -> Self {
        Self {
            last_mouse_x: 0,
            last_mouse_y: 0,
            mouse_button_state: 0,
        }
    }
}

impl Default for InputState {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// translate_key — SDL2 Keycode → DOOM key constant
// =============================================================================

/// Translates an SDL2 [`Keycode`] into the corresponding DOOM internal key
/// constant.
///
/// This is the Rust equivalent of the `xlatekey()` function in
/// `linuxdoom-1.10/i_video.c` (lines 97-162). The original function translated
/// X11 keysyms (`XK_Left`, `XK_Right`, etc.) into DOOM's `KEY_*` constants
/// defined in `doomdef.h`. This function performs the same mapping from SDL2
/// keycodes.
///
/// # Mapping Rules
///
/// | SDL2 Keycode            | DOOM Key Constant |
/// |-------------------------|-------------------|
/// | `Left`                  | `KEY_LEFTARROW`   |
/// | `Right`                 | `KEY_RIGHTARROW`  |
/// | `Down`                  | `KEY_DOWNARROW`   |
/// | `Up`                    | `KEY_UPARROW`     |
/// | `Escape`                | `KEY_ESCAPE`      |
/// | `Return`                | `KEY_ENTER`       |
/// | `Tab`                   | `KEY_TAB`         |
/// | `F1` – `F12`           | `KEY_F1` – `KEY_F12` |
/// | `Backspace` / `Delete`  | `KEY_BACKSPACE`   |
/// | `Pause`                 | `KEY_PAUSE`       |
/// | `KpEquals` / `Equals`   | `KEY_EQUALS`      |
/// | `KpMinus` / `Minus`     | `KEY_MINUS`       |
/// | `LShift` / `RShift`     | `KEY_RSHIFT`      |
/// | `LCtrl` / `RCtrl`       | `KEY_RCTRL`       |
/// | `LAlt` / `RAlt`         | `KEY_RALT`        |
/// | Printable ASCII (space–tilde) | Lowercase ASCII value |
///
/// Unmapped keys return `0`.
///
/// # Behavioral Parity
///
/// The original `xlatekey()` converts uppercase ASCII to lowercase
/// (i_video.c:155-156). This function preserves that behavior: SDL2 keycodes
/// for letter keys are already lowercase (`Keycode::A.into_i32()` = 97 = `'a'`),
/// but the explicit conversion is retained as a safety net for any edge cases.
pub fn translate_key(keycode: Keycode) -> i32 {
    // The match arms below mirror the switch/case in xlatekey() exactly.
    // Each X11 keysym → DOOM key mapping has a 1:1 SDL2 keycode equivalent.
    if keycode == Keycode::Left {
        KEY_LEFTARROW
    } else if keycode == Keycode::Right {
        KEY_RIGHTARROW
    } else if keycode == Keycode::Down {
        KEY_DOWNARROW
    } else if keycode == Keycode::Up {
        KEY_UPARROW
    } else if keycode == Keycode::Escape {
        KEY_ESCAPE
    } else if keycode == Keycode::Return {
        KEY_ENTER
    } else if keycode == Keycode::Tab {
        KEY_TAB
    } else if keycode == Keycode::F1 {
        KEY_F1
    } else if keycode == Keycode::F2 {
        KEY_F2
    } else if keycode == Keycode::F3 {
        KEY_F3
    } else if keycode == Keycode::F4 {
        KEY_F4
    } else if keycode == Keycode::F5 {
        KEY_F5
    } else if keycode == Keycode::F6 {
        KEY_F6
    } else if keycode == Keycode::F7 {
        KEY_F7
    } else if keycode == Keycode::F8 {
        KEY_F8
    } else if keycode == Keycode::F9 {
        KEY_F9
    } else if keycode == Keycode::F10 {
        KEY_F10
    } else if keycode == Keycode::F11 {
        KEY_F11
    } else if keycode == Keycode::F12 {
        KEY_F12
    } else if keycode == Keycode::Backspace || keycode == Keycode::Delete {
        // XK_BackSpace / XK_Delete both map to KEY_BACKSPACE (i_video.c:124-125)
        KEY_BACKSPACE
    } else if keycode == Keycode::Pause {
        KEY_PAUSE
    } else if keycode == Keycode::KpEquals || keycode == Keycode::Equals {
        // XK_KP_Equal / XK_equal both map to KEY_EQUALS (i_video.c:129-130)
        KEY_EQUALS
    } else if keycode == Keycode::KpMinus || keycode == Keycode::Minus {
        // XK_KP_Subtract / XK_minus both map to KEY_MINUS (i_video.c:132-133)
        KEY_MINUS
    } else if keycode == Keycode::LShift || keycode == Keycode::RShift {
        // XK_Shift_L / XK_Shift_R both map to KEY_RSHIFT (i_video.c:135-138)
        KEY_RSHIFT
    } else if keycode == Keycode::LCtrl || keycode == Keycode::RCtrl {
        // XK_Control_L / XK_Control_R both map to KEY_RCTRL (i_video.c:140-143)
        KEY_RCTRL
    } else if keycode == Keycode::LAlt || keycode == Keycode::RAlt {
        // XK_Alt_L / XK_Meta_L / XK_Alt_R / XK_Meta_R all map to KEY_RALT
        // (i_video.c:145-150). SDL2 does not distinguish Meta from Alt.
        KEY_RALT
    } else {
        // Default case: handle ASCII printable range (i_video.c:152-157).
        //
        // The original code:
        //   if (rc >= XK_space && rc <= XK_asciitilde)
        //       rc = rc - XK_space + ' ';
        //   if (rc >= 'A' && rc <= 'Z')
        //       rc = rc - 'A' + 'a';
        //
        // Since XK_space == ' ' == 0x20 and XK_asciitilde == '~' == 0x7e,
        // the first line is an identity transformation. SDL2 keycodes for
        // printable ASCII characters also match their ASCII values, so we
        // apply the same identity + uppercase-to-lowercase conversion.
        let k = keycode.into_i32();
        if k >= b' ' as i32 && k <= b'~' as i32 {
            // Convert uppercase to lowercase, matching i_video.c:155-156.
            // SDL2 keycodes for letter keys are already lowercase, but this
            // conversion is retained as a safety net.
            if k >= b'A' as i32 && k <= b'Z' as i32 {
                k - b'A' as i32 + b'a' as i32
            } else {
                k
            }
        } else {
            // Unmapped key — return 0 to indicate no DOOM key equivalent.
            0
        }
    }
}

// =============================================================================
// mouse_button_bit — SDL2 MouseButton → bitmask bit
// =============================================================================

/// Returns the bitmask bit for the given SDL2 mouse button, matching
/// the X11 button mask encoding used in the original `I_GetEvent()`.
///
/// - `MouseButton::Left`   → bit 0 (0x01) — fire in DOOM
/// - `MouseButton::Middle` → bit 1 (0x02) — strafe in DOOM
/// - `MouseButton::Right`  → bit 2 (0x04) — forward in DOOM
/// - Other buttons → 0 (no mapping)
fn mouse_button_bit(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => MOUSE_BUTTON_LEFT,
        MouseButton::Middle => MOUSE_BUTTON_MIDDLE,
        MouseButton::Right => MOUSE_BUTTON_RIGHT,
        _ => 0,
    }
}

// =============================================================================
// process_sdl_events — SDL2 event batch → DOOM Event list
// =============================================================================

/// Processes a batch of SDL2 events and returns a list of DOOM [`Event`]s.
///
/// This function is the Rust equivalent of `I_GetEvent()` (i_video.c:194-279)
/// combined with the polling loop in `I_StartTic()` (i_video.c:309-338).
/// The caller collects SDL2 events from the event pump and passes them here;
/// the returned DOOM events should be posted to the game engine via
/// `D_PostEvent`.
///
/// # Event Mapping
///
/// | SDL2 Event          | DOOM EventType | data1               | data2         | data3          |
/// |---------------------|----------------|----------------------|---------------|----------------|
/// | `KeyDown`           | `KeyDown`      | translated key code  | 0             | 0              |
/// | `KeyUp`             | `KeyUp`        | translated key code  | 0             | 0              |
/// | `MouseButtonDown`   | `Mouse`        | button bitmask       | 0             | 0              |
/// | `MouseButtonUp`     | `Mouse`        | button bitmask       | 0             | 0              |
/// | `MouseMotion`       | `Mouse`        | button bitmask       | `xrel << 2`   | `-(yrel) << 2` |
/// | `Quit`              | `KeyDown`      | `KEY_ESCAPE`         | 0             | 0              |
///
/// # Mouse Motion Scaling
///
/// Mouse motion deltas are left-shifted by 2 bits (`<< 2`), matching the
/// original scaling in i_video.c:250-251:
/// ```c
/// event.data2 = (X_event.xmotion.x - lastmousex) << 2;
/// event.data3 = (lastmousey - X_event.xmotion.y) << 2;
/// ```
///
/// The Y-axis is inverted because screen coordinates increase downward while
/// DOOM's internal coordinate system expects positive Y for upward movement.
///
/// # Arguments
///
/// * `sdl_events` — Slice of SDL2 events collected from the event pump this tic.
/// * `state` — Mutable reference to [`InputState`] for tracking persistent
///   mouse position and button state across calls.
///
/// # Returns
///
/// A `Vec<Event>` containing zero or more DOOM events to be posted to the
/// game engine's event queue.
pub fn process_sdl_events(sdl_events: &[SdlEvent], state: &mut InputState) -> Vec<Event> {
    let mut doom_events: Vec<Event> = Vec::with_capacity(sdl_events.len());

    for sdl_event in sdl_events {
        match sdl_event {
            // -----------------------------------------------------------------
            // Keyboard events — KeyPress / KeyRelease (i_video.c:203-213)
            // -----------------------------------------------------------------
            SdlEvent::KeyDown {
                keycode: Some(kc),
                repeat,
                ..
            } => {
                // Filter out OS key repeats. DOOM manages its own key state
                // persistence internally — repeated KeyDown events from the OS
                // would cause double-fire and other input glitches. The original
                // X11 implementation relied on auto-repeat being managed by the
                // X server; SDL2 explicitly flags repeats via the `repeat` field.
                if *repeat {
                    continue;
                }

                let key = translate_key(*kc);
                if key != 0 {
                    debug!(
                        sdl_keycode = ?kc,
                        doom_key = key,
                        "Key down"
                    );
                    doom_events.push(Event::new(EventType::KeyDown, key, 0, 0));
                }
            }

            SdlEvent::KeyUp {
                keycode: Some(kc), ..
            } => {
                let key = translate_key(*kc);
                if key != 0 {
                    debug!(
                        sdl_keycode = ?kc,
                        doom_key = key,
                        "Key up"
                    );
                    doom_events.push(Event::new(EventType::KeyUp, key, 0, 0));
                }
            }

            // -----------------------------------------------------------------
            // Mouse button events — ButtonPress / ButtonRelease
            // (i_video.c:215-243)
            // -----------------------------------------------------------------
            SdlEvent::MouseButtonDown { mouse_btn, .. } => {
                let bit = mouse_button_bit(*mouse_btn);
                if bit != 0 {
                    // Set the pressed button's bit in the state bitmask.
                    // This mirrors the OR logic in i_video.c:217-223 where
                    // the current X11 button state is combined with the
                    // newly pressed button.
                    state.mouse_button_state |= bit;
                    debug!(
                        button = ?mouse_btn,
                        state = state.mouse_button_state,
                        "Mouse button down"
                    );
                    doom_events.push(Event::new(
                        EventType::Mouse,
                        state.mouse_button_state as i32,
                        0,
                        0,
                    ));
                }
            }

            SdlEvent::MouseButtonUp { mouse_btn, .. } => {
                let bit = mouse_button_bit(*mouse_btn);
                if bit != 0 {
                    // Clear the released button's bit from the state bitmask.
                    // This mirrors the XOR logic in i_video.c:235-239 where
                    // the released button is toggled off from the state.
                    state.mouse_button_state &= !bit;
                    debug!(
                        button = ?mouse_btn,
                        state = state.mouse_button_state,
                        "Mouse button up"
                    );
                    doom_events.push(Event::new(
                        EventType::Mouse,
                        state.mouse_button_state as i32,
                        0,
                        0,
                    ));
                }
            }

            // -----------------------------------------------------------------
            // Mouse motion event — MotionNotify (i_video.c:244-268)
            // -----------------------------------------------------------------
            SdlEvent::MouseMotion {
                x, y, xrel, yrel, ..
            } => {
                // Scale mouse deltas by << 2, matching i_video.c:250-251:
                //   event.data2 = (X_event.xmotion.x - lastmousex) << 2;
                //   event.data3 = (lastmousey - X_event.xmotion.y) << 2;
                //
                // SDL2 provides relative deltas directly via xrel/yrel,
                // eliminating the need to manually compute differences from
                // absolute positions. The Y-axis is inverted because screen
                // coordinates increase downward while DOOM expects positive Y
                // for upward movement.
                let data2 = *xrel << 2;
                let data3 = (-*yrel) << 2;

                // Update last known absolute position for external consumers
                // (corresponds to lastmousex/lastmousey in i_video.c:255-256).
                state.last_mouse_x = *x;
                state.last_mouse_y = *y;

                // Only post the event if there was actual movement, matching
                // the guard condition in i_video.c:253:
                //   if (event.data2 || event.data3)
                if data2 != 0 || data3 != 0 {
                    doom_events.push(Event::new(
                        EventType::Mouse,
                        state.mouse_button_state as i32,
                        data2,
                        data3,
                    ));
                }
            }

            // -----------------------------------------------------------------
            // Quit event — SDL_QUIT (window close, etc.)
            // -----------------------------------------------------------------
            SdlEvent::Quit { .. } => {
                // Map the quit request to an Escape key press so that DOOM's
                // existing menu system handles the quit confirmation dialog.
                // The original X11 code did not explicitly handle window close
                // (the WM_DELETE_WINDOW protocol was not set up), but modern
                // windowed DOOM ports conventionally map Quit → Escape.
                debug!("Quit event received, mapping to KEY_ESCAPE");
                doom_events.push(Event::new(EventType::KeyDown, KEY_ESCAPE, 0, 0));
            }

            // Ignore all other SDL2 events (window resize, focus, etc.)
            _ => {}
        }
    }

    doom_events
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Key constant value verification
    // -------------------------------------------------------------------------

    #[test]
    fn key_constants_match_doomdef() {
        // Verify against the canonical values from doomdef.h lines 250-280.
        assert_eq!(KEY_RIGHTARROW, 0xae);
        assert_eq!(KEY_LEFTARROW, 0xac);
        assert_eq!(KEY_UPARROW, 0xad);
        assert_eq!(KEY_DOWNARROW, 0xaf);
        assert_eq!(KEY_ESCAPE, 27);
        assert_eq!(KEY_ENTER, 13);
        assert_eq!(KEY_TAB, 9);
        assert_eq!(KEY_F1, 0x80 + 0x3b);
        assert_eq!(KEY_F2, 0x80 + 0x3c);
        assert_eq!(KEY_F3, 0x80 + 0x3d);
        assert_eq!(KEY_F4, 0x80 + 0x3e);
        assert_eq!(KEY_F5, 0x80 + 0x3f);
        assert_eq!(KEY_F6, 0x80 + 0x40);
        assert_eq!(KEY_F7, 0x80 + 0x41);
        assert_eq!(KEY_F8, 0x80 + 0x42);
        assert_eq!(KEY_F9, 0x80 + 0x43);
        assert_eq!(KEY_F10, 0x80 + 0x44);
        assert_eq!(KEY_F11, 0x80 + 0x57);
        assert_eq!(KEY_F12, 0x80 + 0x58);
        assert_eq!(KEY_BACKSPACE, 127);
        assert_eq!(KEY_PAUSE, 0xff);
        assert_eq!(KEY_EQUALS, 0x3d);
        assert_eq!(KEY_MINUS, 0x2d);
        assert_eq!(KEY_RSHIFT, 0x80 + 0x36);
        assert_eq!(KEY_RCTRL, 0x80 + 0x1d);
        assert_eq!(KEY_RALT, 0x80 + 0x38);
        assert_eq!(KEY_LALT, KEY_RALT);
    }

    // -------------------------------------------------------------------------
    // InputState construction
    // -------------------------------------------------------------------------

    #[test]
    fn input_state_new_zeroed() {
        let state = InputState::new();
        assert_eq!(state.last_mouse_x, 0);
        assert_eq!(state.last_mouse_y, 0);
        assert_eq!(state.mouse_button_state, 0);
    }

    #[test]
    fn input_state_default_matches_new() {
        let from_new = InputState::new();
        let from_default = InputState::default();
        assert_eq!(from_new.last_mouse_x, from_default.last_mouse_x);
        assert_eq!(from_new.last_mouse_y, from_default.last_mouse_y);
        assert_eq!(from_new.mouse_button_state, from_default.mouse_button_state);
    }

    // -------------------------------------------------------------------------
    // translate_key — arrow keys
    // -------------------------------------------------------------------------

    #[test]
    fn translate_arrow_keys() {
        assert_eq!(translate_key(Keycode::Left), KEY_LEFTARROW);
        assert_eq!(translate_key(Keycode::Right), KEY_RIGHTARROW);
        assert_eq!(translate_key(Keycode::Up), KEY_UPARROW);
        assert_eq!(translate_key(Keycode::Down), KEY_DOWNARROW);
    }

    // -------------------------------------------------------------------------
    // translate_key — common keys
    // -------------------------------------------------------------------------

    #[test]
    fn translate_common_keys() {
        assert_eq!(translate_key(Keycode::Escape), KEY_ESCAPE);
        assert_eq!(translate_key(Keycode::Return), KEY_ENTER);
        assert_eq!(translate_key(Keycode::Tab), KEY_TAB);
        assert_eq!(translate_key(Keycode::Backspace), KEY_BACKSPACE);
        assert_eq!(translate_key(Keycode::Delete), KEY_BACKSPACE);
        assert_eq!(translate_key(Keycode::Pause), KEY_PAUSE);
    }

    // -------------------------------------------------------------------------
    // translate_key — function keys
    // -------------------------------------------------------------------------

    #[test]
    fn translate_function_keys() {
        assert_eq!(translate_key(Keycode::F1), KEY_F1);
        assert_eq!(translate_key(Keycode::F2), KEY_F2);
        assert_eq!(translate_key(Keycode::F3), KEY_F3);
        assert_eq!(translate_key(Keycode::F4), KEY_F4);
        assert_eq!(translate_key(Keycode::F5), KEY_F5);
        assert_eq!(translate_key(Keycode::F6), KEY_F6);
        assert_eq!(translate_key(Keycode::F7), KEY_F7);
        assert_eq!(translate_key(Keycode::F8), KEY_F8);
        assert_eq!(translate_key(Keycode::F9), KEY_F9);
        assert_eq!(translate_key(Keycode::F10), KEY_F10);
        assert_eq!(translate_key(Keycode::F11), KEY_F11);
        assert_eq!(translate_key(Keycode::F12), KEY_F12);
    }

    // -------------------------------------------------------------------------
    // translate_key — modifier keys
    // -------------------------------------------------------------------------

    #[test]
    fn translate_modifier_keys() {
        // Both left and right shift map to KEY_RSHIFT
        assert_eq!(translate_key(Keycode::LShift), KEY_RSHIFT);
        assert_eq!(translate_key(Keycode::RShift), KEY_RSHIFT);
        // Both left and right ctrl map to KEY_RCTRL
        assert_eq!(translate_key(Keycode::LCtrl), KEY_RCTRL);
        assert_eq!(translate_key(Keycode::RCtrl), KEY_RCTRL);
        // Both left and right alt map to KEY_RALT
        assert_eq!(translate_key(Keycode::LAlt), KEY_RALT);
        assert_eq!(translate_key(Keycode::RAlt), KEY_RALT);
    }

    // -------------------------------------------------------------------------
    // translate_key — equals and minus (regular + keypad)
    // -------------------------------------------------------------------------

    #[test]
    fn translate_equals_minus() {
        assert_eq!(translate_key(Keycode::Equals), KEY_EQUALS);
        assert_eq!(translate_key(Keycode::KpEquals), KEY_EQUALS);
        assert_eq!(translate_key(Keycode::Minus), KEY_MINUS);
        assert_eq!(translate_key(Keycode::KpMinus), KEY_MINUS);
    }

    // -------------------------------------------------------------------------
    // translate_key — ASCII printable range
    // -------------------------------------------------------------------------

    #[test]
    fn translate_ascii_printable() {
        // Space character
        assert_eq!(translate_key(Keycode::Space), b' ' as i32);
        // Lowercase letter — should pass through as-is
        // SDL2 keycodes for letters are already lowercase ASCII
        assert_eq!(translate_key(Keycode::A), b'a' as i32);
        assert_eq!(translate_key(Keycode::Z), b'z' as i32);
        // Numeric keys
        assert_eq!(translate_key(Keycode::Num0), b'0' as i32);
        assert_eq!(translate_key(Keycode::Num9), b'9' as i32);
    }

    // -------------------------------------------------------------------------
    // translate_key — unmapped key returns 0
    // -------------------------------------------------------------------------

    #[test]
    fn translate_unmapped_key_returns_zero() {
        // CapsLock is not in DOOM's key mapping
        assert_eq!(translate_key(Keycode::CapsLock), 0);
    }

    // -------------------------------------------------------------------------
    // mouse_button_bit helper
    // -------------------------------------------------------------------------

    #[test]
    fn mouse_button_bits() {
        assert_eq!(mouse_button_bit(MouseButton::Left), 1);
        assert_eq!(mouse_button_bit(MouseButton::Middle), 2);
        assert_eq!(mouse_button_bit(MouseButton::Right), 4);
        assert_eq!(mouse_button_bit(MouseButton::Unknown), 0);
        assert_eq!(mouse_button_bit(MouseButton::X1), 0);
        assert_eq!(mouse_button_bit(MouseButton::X2), 0);
    }
}
