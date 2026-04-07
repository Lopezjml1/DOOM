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

//! Translated from linuxdoom-1.10/m_random.c and linuxdoom-1.10/m_random.h
//!
//! Deterministic pseudo-random number generator.
//!
//! DOOM uses a 256-byte lookup table for random numbers, with two separate
//! indices: one for gameplay-affecting randomness (P_Random) and one for
//! non-gameplay effects (M_Random). This separation ensures that demo
//! playback produces identical gameplay regardless of non-gameplay random
//! number consumption.
//!
//! **CRITICAL**: The table values and index advancement logic must be
//! bit-identical to the original C implementation for demo compatibility.

/// The deterministic PRNG lookup table. Exactly 256 bytes, copied verbatim
/// from the original id Software DOOM 1.10 source (m_random.c lines 31-51).
///
/// This table is the sole source of pseudo-random values in the engine.
/// Both `P_Random` (gameplay) and `M_Random` (non-gameplay) read from
/// this same table using independent indices.
pub static RNDTABLE: [u8; 256] = [
    0, 8, 109, 220, 222, 241, 149, 107, 75, 248, 254, 140, 16, 66, 74, 21, 211, 47, 80, 242, 154,
    27, 205, 128, 161, 89, 77, 36, 95, 110, 85, 48, 212, 140, 211, 249, 22, 79, 200, 50, 28, 188,
    52, 140, 202, 120, 68, 145, 62, 70, 184, 190, 91, 197, 152, 224, 149, 104, 25, 178, 252, 182,
    202, 182, 141, 197, 4, 81, 181, 242, 145, 42, 39, 227, 156, 198, 225, 193, 219, 93, 122, 175,
    249, 0, 175, 143, 70, 239, 46, 246, 163, 53, 163, 109, 168, 135, 2, 235, 25, 92, 20, 145, 138,
    77, 69, 166, 78, 176, 173, 212, 166, 113, 94, 161, 41, 50, 239, 49, 111, 164, 70, 60, 2, 37,
    171, 75, 136, 156, 11, 56, 42, 146, 138, 229, 73, 146, 77, 61, 98, 196, 135, 106, 63, 197, 195,
    86, 96, 203, 113, 101, 170, 247, 181, 113, 80, 250, 108, 7, 255, 237, 129, 226, 79, 107, 112,
    166, 103, 241, 24, 223, 239, 120, 198, 58, 60, 82, 128, 3, 184, 66, 143, 224, 145, 224, 81,
    206, 163, 45, 63, 90, 168, 114, 59, 33, 159, 95, 28, 139, 123, 98, 125, 196, 15, 70, 194, 253,
    54, 14, 109, 226, 71, 17, 161, 93, 186, 87, 244, 138, 20, 52, 123, 251, 26, 36, 17, 46, 52,
    231, 232, 76, 31, 221, 84, 37, 216, 165, 212, 106, 197, 242, 98, 43, 39, 175, 254, 145, 190,
    84, 118, 222, 187, 136, 120, 163, 236, 249,
];

/// Deterministic PRNG state. Holds two separate indices into [`RNDTABLE`].
///
/// Replaces the C globals: `int rndindex` and `int prndindex` from
/// `m_random.c` lines 53-54.
///
/// # Index Separation
///
/// Two independent indices exist because gameplay-affecting randomness
/// (`p_random`) must produce a deterministic sequence for demo playback,
/// while non-gameplay randomness (`m_random`) — used for visual effects,
/// menu animations, etc. — is not recorded in demos. Sharing a single
/// index would cause non-gameplay calls to perturb the gameplay sequence.
#[derive(Debug, Clone)]
pub struct DoomRandom {
    /// Index for M_Random (non-gameplay random numbers).
    rndindex: i32,
    /// Index for P_Random (gameplay random numbers — deterministic for demos).
    prndindex: i32,
}

impl DoomRandom {
    /// Create a new PRNG state with both indices at 0.
    ///
    /// This is equivalent to the initial state of the C globals
    /// `rndindex = 0` and `prndindex = 0`.
    pub const fn new() -> Self {
        DoomRandom {
            rndindex: 0,
            prndindex: 0,
        }
    }

    /// Gameplay-affecting random number. Used for AI decisions, damage rolls,
    /// spread patterns, and all other gameplay randomness that must be
    /// deterministic for demo playback.
    ///
    /// Uses a separate index from [`m_random`](Self::m_random).
    ///
    /// Equivalent to C function `int P_Random(void)` (m_random.c lines 57-61):
    /// ```c
    /// prndindex = (prndindex+1)&0xff;
    /// return rndtable[prndindex];
    /// ```
    ///
    /// **CRITICAL**: Advances the index BEFORE reading the table (pre-increment).
    /// After [`clear_random`](Self::clear_random), the first call returns
    /// `RNDTABLE[1]` (value 8), not `RNDTABLE[0]`.
    ///
    /// Returns a value in the range 0..=255.
    #[inline]
    pub fn p_random(&mut self) -> u8 {
        self.prndindex = (self.prndindex + 1) & 0xff;
        RNDTABLE[self.prndindex as usize]
    }

    /// Non-gameplay random number. Used for menu effects, visual flourishes,
    /// and other non-deterministic contexts that are not recorded in demos.
    ///
    /// Uses a separate index from [`p_random`](Self::p_random).
    ///
    /// Equivalent to C function `int M_Random(void)` (m_random.c lines 63-67):
    /// ```c
    /// rndindex = (rndindex+1)&0xff;
    /// return rndtable[rndindex];
    /// ```
    ///
    /// Returns a value in the range 0..=255.
    #[inline]
    pub fn m_random(&mut self) -> u8 {
        self.rndindex = (self.rndindex + 1) & 0xff;
        RNDTABLE[self.rndindex as usize]
    }

    /// Reset both PRNG indices to 0. Called at level start to ensure
    /// deterministic gameplay for demo playback.
    ///
    /// Equivalent to C function `void M_ClearRandom(void)` (m_random.c
    /// lines 69-72) which sets `rndindex = prndindex = 0;`
    pub fn clear_random(&mut self) {
        self.rndindex = 0;
        self.prndindex = 0;
    }

    /// Get the current P_Random index (for debugging/testing).
    ///
    /// After [`clear_random`](Self::clear_random), this returns 0.
    /// After one call to [`p_random`](Self::p_random), this returns 1.
    pub fn prnd_index(&self) -> i32 {
        self.prndindex
    }

    /// Get the current M_Random index (for debugging/testing).
    ///
    /// After [`clear_random`](Self::clear_random), this returns 0.
    /// After one call to [`m_random`](Self::m_random), this returns 1.
    pub fn rnd_index(&self) -> i32 {
        self.rndindex
    }
}

impl Default for DoomRandom {
    fn default() -> Self {
        Self::new()
    }
}
