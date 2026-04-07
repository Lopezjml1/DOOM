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

//! Translated from linuxdoom-1.10/d_ticcmd.h
//!
//! The data sampled per tick (single player) and transmitted to other peers
//! (multiplayer). Mainly movements/button commands per game tick, plus a
//! checksum for internal state consistency.

/// Per-tick input command structure.
///
/// This structure captures the player's input for a single game tick (1/35th
/// of a second). In single-player mode it is sampled directly from input
/// devices; in multiplayer it is also serialized and transmitted to peers.
///
/// The `forwardmove` and `sidemove` fields are scaled by 2048 when applied
/// to the player's momentum. The `angleturn` field is shifted left by 16
/// bits to produce an angle delta in Binary Angle Measurement (BAM) units.
///
/// # Memory Layout
///
/// `#[repr(C)]` is used to guarantee a C-compatible memory layout, which is
/// required for demo file compatibility and potential network serialization.
///
/// # Original C definition (d_ticcmd.h lines 36-44)
///
/// ```c
/// typedef struct {
///     char    forwardmove;    // *2048 for move
///     char    sidemove;       // *2048 for move
///     short   angleturn;      // <<16 for angle delta
///     short   consistancy;    // checks for net game
///     byte    chatchar;
///     byte    buttons;
/// } ticcmd_t;
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct TicCmd {
    /// Forward/backward movement speed.
    ///
    /// Positive values move forward, negative values move backward.
    /// Multiplied by 2048 when applied to player momentum.
    /// Mapped from C `char` (signed 8-bit).
    pub forwardmove: i8,

    /// Left/right strafing movement speed.
    ///
    /// Positive values strafe right, negative values strafe left.
    /// Multiplied by 2048 when applied to player momentum.
    /// Mapped from C `char` (signed 8-bit).
    pub sidemove: i8,

    /// Turning angle delta.
    ///
    /// Left-shifted by 16 bits to produce the actual angle change in
    /// Binary Angle Measurement (BAM) units. Positive values turn left
    /// (counter-clockwise), negative values turn right (clockwise).
    /// Mapped from C `short` (signed 16-bit).
    pub angleturn: i16,

    /// Network consistency check value.
    ///
    /// Used in multiplayer to verify that all peers share the same game
    /// state. The spelling `consistancy` (with an 'a') preserves the
    /// original misspelling from the id Software C source code.
    /// Mapped from C `short` (signed 16-bit).
    pub consistancy: i16,

    /// Chat character.
    ///
    /// When non-zero, this character is appended to the player's chat
    /// message buffer. Only one character can be sent per tick.
    /// Mapped from C `byte` (unsigned 8-bit).
    pub chatchar: u8,

    /// Button state bitfield.
    ///
    /// Encodes the state of action buttons (fire, use/open) for this tick.
    /// Individual bits correspond to specific actions defined by the
    /// button constants (`BT_ATTACK`, `BT_USE`, `BT_SPECIAL`, etc.).
    /// Mapped from C `byte` (unsigned 8-bit).
    pub buttons: u8,
}

impl TicCmd {
    /// Create a new empty `TicCmd` with all fields initialized to zero.
    ///
    /// This represents a "no input" tick — the player is not moving, not
    /// turning, not pressing any buttons, and not sending a chat character.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::types::ticcmd::TicCmd;
    ///
    /// let cmd = TicCmd::new();
    /// assert_eq!(cmd.forwardmove, 0);
    /// assert_eq!(cmd.sidemove, 0);
    /// assert_eq!(cmd.angleturn, 0);
    /// assert_eq!(cmd.consistancy, 0);
    /// assert_eq!(cmd.chatchar, 0);
    /// assert_eq!(cmd.buttons, 0);
    /// ```
    pub const fn new() -> Self {
        TicCmd {
            forwardmove: 0,
            sidemove: 0,
            angleturn: 0,
            consistancy: 0,
            chatchar: 0,
            buttons: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_returns_zeroed_struct() {
        let cmd = TicCmd::new();
        assert_eq!(cmd.forwardmove, 0);
        assert_eq!(cmd.sidemove, 0);
        assert_eq!(cmd.angleturn, 0);
        assert_eq!(cmd.consistancy, 0);
        assert_eq!(cmd.chatchar, 0);
        assert_eq!(cmd.buttons, 0);
    }

    #[test]
    fn test_default_matches_new() {
        let from_new = TicCmd::new();
        let from_default = TicCmd::default();
        assert_eq!(from_new, from_default);
    }

    #[test]
    fn test_clone_and_copy() {
        let cmd = TicCmd {
            forwardmove: 50,
            sidemove: -24,
            angleturn: 1280,
            consistancy: 42,
            chatchar: b'A',
            buttons: 0x03,
        };
        let cloned = cmd;
        assert_eq!(cmd, cloned);
    }

    #[test]
    fn test_field_ranges() {
        // Verify full range of i8 for movement fields
        let cmd = TicCmd {
            forwardmove: i8::MAX,
            sidemove: i8::MIN,
            angleturn: i16::MAX,
            consistancy: i16::MIN,
            chatchar: u8::MAX,
            buttons: u8::MAX,
        };
        assert_eq!(cmd.forwardmove, 127);
        assert_eq!(cmd.sidemove, -128);
        assert_eq!(cmd.angleturn, 32767);
        assert_eq!(cmd.consistancy, -32768);
        assert_eq!(cmd.chatchar, 255);
        assert_eq!(cmd.buttons, 255);
    }

    #[test]
    fn test_repr_c_size() {
        // With #[repr(C)], the struct should have a predictable size:
        // i8 + i8 + i16 + i16 + u8 + u8 = 8 bytes
        assert_eq!(std::mem::size_of::<TicCmd>(), 8);
    }

    #[test]
    fn test_repr_c_alignment() {
        // #[repr(C)] alignment should be 2 (largest field is i16)
        assert_eq!(std::mem::align_of::<TicCmd>(), 2);
    }

    #[test]
    fn test_debug_format() {
        let cmd = TicCmd::new();
        let debug_str = format!("{:?}", cmd);
        assert!(debug_str.contains("TicCmd"));
        assert!(debug_str.contains("forwardmove"));
        assert!(debug_str.contains("consistancy"));
    }
}
