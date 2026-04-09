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

//! Translated from linuxdoom-1.10/m_cheat.c and linuxdoom-1.10/m_cheat.h
//!
//! Cheat code sequence checking. Implements a state machine that detects
//! when the player has typed a cheat code sequence (IDDQD, IDKFA, etc.).
//! Used by the status bar module (st_stuff).
//!
//! The cheat system works by storing encoded (scrambled) byte sequences.
//! As the player types keys, each key is translated via a lookup table
//! and compared against the expected sequence. Special byte values in
//! the sequence control the state machine:
//! - `0`: Parameter input slot — stores the raw key value
//! - `1`: Parameter delimiter — marks the boundary between the cheat
//!   code and its parameters
//! - `0xff`: End-of-sequence marker — triggers cheat activation

// ---------------------------------------------------------------------------
// SCRAMBLE — bit-manipulation cipher
// ---------------------------------------------------------------------------

/// Scramble a byte value by rearranging its bits.
///
/// This is a bijective (one-to-one) bit permutation. Each input bit maps
/// to exactly one output bit position:
///
/// | Input bit | Output bit |
/// |-----------|------------|
/// | 0         | 7          |
/// | 1         | 6          |
/// | 2         | 2          |
/// | 3         | 4          |
/// | 4         | 3          |
/// | 5         | 5          |
/// | 6         | 1          |
/// | 7         | 0          |
///
/// Equivalent to the C macro from m_cheat.h:
/// ```text
/// #define SCRAMBLE(a) \
/// ((((a)&1)<<7) + (((a)&2)<<5) + ((a)&4) + (((a)&8)<<1) \
///  + (((a)&16)>>1) + ((a)&32) + (((a)&64)>>5) + (((a)&128)>>7))
/// ```
#[inline]
pub const fn scramble(a: u8) -> u8 {
    ((a & 1) << 7)
        .wrapping_add((a & 2) << 5)
        .wrapping_add(a & 4)
        .wrapping_add((a & 8) << 1)
        .wrapping_add((a & 16) >> 1)
        .wrapping_add(a & 32)
        .wrapping_add((a & 64) >> 5)
        .wrapping_add((a & 128) >> 7)
}

// ---------------------------------------------------------------------------
// Cheat translation table — pre-computed at compile time
// ---------------------------------------------------------------------------

/// Build the 256-byte cheat translation table at compile time.
///
/// In the original C code, this was a lazily-initialized static array
/// (`static unsigned char cheat_xlate_table[256]`) populated on the first
/// call to `cht_CheckCheat` via `SCRAMBLE(i)` for `i` in `0..256`.
///
/// In Rust we build it as a `const fn` so the table is embedded directly
/// in the binary with zero runtime initialization cost.
const fn build_xlate_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0u16;
    while i < 256 {
        table[i as usize] = scramble(i as u8);
        i += 1;
    }
    table
}

/// Pre-computed cheat sequence translation table.
///
/// Maps each raw key byte to its scrambled equivalent. Used by
/// [`CheatSeq::check_cheat`] to compare incoming key presses against
/// the encoded cheat sequence.
///
/// Replaces the C global `static unsigned char cheat_xlate_table[256]`.
static CHEAT_XLATE_TABLE: [u8; 256] = build_xlate_table();

// ---------------------------------------------------------------------------
// CheatSeq — cheat sequence state tracker
// ---------------------------------------------------------------------------

/// Cheat sequence state tracker.
///
/// Equivalent to the C struct:
/// ```c
/// typedef struct {
///     unsigned char* sequence;
///     unsigned char* p;
/// } cheatseq_t;
/// ```
///
/// The `sequence` field holds the encoded cheat bytes (produced via
/// [`scramble`]), terminated by `0xff`. The `pos` field tracks how far
/// the player has progressed through the sequence (replacing the C
/// pointer `p`).
///
/// # Special sequence byte values
///
/// | Value  | Meaning                                      |
/// |--------|----------------------------------------------|
/// | `0`    | Parameter input slot — raw key stored here    |
/// | `1`    | Parameter delimiter — skipped automatically   |
/// | `0xff` | End of sequence — cheat activation marker     |
///
/// # Example
///
/// ```
/// use doom_core::util::cheat::{CheatSeq, scramble};
///
/// // Build a trivial two-character cheat "AB" + end marker
/// let seq = [scramble(b'a'), scramble(b'b'), 0xff];
/// let mut cheat = CheatSeq::new(&seq);
///
/// assert!(!cheat.check_cheat(b'a'));
/// assert!(cheat.check_cheat(b'b')); // cheat complete
/// ```
#[derive(Debug, Clone)]
pub struct CheatSeq {
    /// The encoded cheat sequence bytes.
    ///
    /// This vector is mutable because parameter input slots (bytes with
    /// value `0`) are overwritten in-place with the raw key values typed
    /// by the player, matching the original C behavior where
    /// `*(cht->p++) = key` stores into the sequence buffer.
    sequence: Vec<u8>,

    /// Current match position index within `sequence`.
    ///
    /// Replaces the C pointer `p` — starts at `0` (equivalent to
    /// `p = sequence`) and advances toward the end marker.
    pos: usize,
}

impl CheatSeq {
    /// Create a new `CheatSeq` from a raw encoded sequence.
    ///
    /// The sequence should consist of scrambled bytes (via [`scramble`]),
    /// optionally containing a parameter section (delimiter `1` followed
    /// by zero-valued input slots), and terminated by `0xff`.
    ///
    /// # Panics
    ///
    /// Does not panic, but the caller is responsible for providing a
    /// well-formed sequence. A sequence missing the `0xff` terminator
    /// will never trigger cheat completion.
    pub fn new(sequence: &[u8]) -> Self {
        CheatSeq {
            sequence: sequence.to_vec(),
            pos: 0,
        }
    }

    /// Check if a key press advances or completes this cheat sequence.
    ///
    /// Returns `true` if the entire cheat sequence has been matched
    /// (the state machine reached the `0xff` end marker), `false`
    /// otherwise.
    ///
    /// Equivalent to C: `int cht_CheckCheat(cheatseq_t* cht, char key)`
    ///
    /// # State machine behaviour
    ///
    /// 1. If the current sequence byte is `0` (parameter slot), the raw
    ///    `key` value is stored in-place and the position advances.
    /// 2. Otherwise, the key is translated through [`CHEAT_XLATE_TABLE`]
    ///    and compared to the current sequence byte. On match the
    ///    position advances; on mismatch the position resets to `0`.
    /// 3. After advancing, if the new position is `1` (parameter
    ///    delimiter) it is skipped automatically.
    /// 4. If the new position is `0xff` (end marker), the cheat is
    ///    complete: the position resets and `true` is returned.
    pub fn check_cheat(&mut self, key: u8) -> bool {
        // Defensive bounds check — should never happen with a well-formed
        // sequence, but prevents a panic in case of corruption.
        if self.pos >= self.sequence.len() {
            self.pos = 0;
            return false;
        }

        // --- First block: match or store the current byte ---
        if self.sequence[self.pos] == 0 {
            // Parameter input slot: store the raw key and advance.
            // Equivalent to C: `*(cht->p++) = key;`
            self.sequence[self.pos] = key;
            self.pos += 1;
        } else if CHEAT_XLATE_TABLE[key as usize] == self.sequence[self.pos] {
            // Scrambled key matches the expected sequence byte — advance.
            self.pos += 1;
        } else {
            // Mismatch: reset to the beginning of the sequence.
            self.pos = 0;
            return false;
        }

        // --- Second block: handle special post-advance values ---
        // These two checks mirror the C `if / else if` structure exactly.
        if self.pos < self.sequence.len() && self.sequence[self.pos] == 1 {
            // Parameter delimiter — skip past it automatically.
            self.pos += 1;
        } else if self.pos < self.sequence.len() && self.sequence[self.pos] == 0xff {
            // End-of-sequence marker — cheat is complete.
            self.pos = 0;
            return true;
        }

        false
    }

    /// Extract parameter characters from the sequence after a cheat
    /// completes.
    ///
    /// Walks the sequence to find the parameter delimiter (byte value
    /// `1`), then copies all subsequent bytes (the user-entered
    /// parameters) into the returned vector. Each extracted byte is
    /// zeroed in the sequence so that the parameter slots are ready for
    /// re-use on the next cheat activation.
    ///
    /// Equivalent to C: `void cht_GetParam(cheatseq_t* cht, char* buffer)`
    ///
    /// # Returns
    ///
    /// A `Vec<u8>` containing the parameter bytes. The vector will be
    /// empty if there is no parameter delimiter in the sequence.
    pub fn get_param(&mut self) -> Vec<u8> {
        let mut buffer = Vec::new();

        // Walk the sequence to find the parameter delimiter (value 1).
        // Equivalent to C: `while (*(p++) != 1);`
        let mut p = 0;
        while p < self.sequence.len() && self.sequence[p] != 1 {
            p += 1;
        }
        // Skip past the delimiter itself.
        if p < self.sequence.len() {
            p += 1;
        }

        // Extract parameter bytes in a do-while equivalent, zeroing each
        // slot as we go so the sequence is ready for re-use.
        // Equivalent to C:
        //   do {
        //       c = *p;
        //       *(buffer++) = c;
        //       *(p++) = 0;
        //   } while (c && *p != 0xff);
        if p < self.sequence.len() {
            loop {
                let c = self.sequence[p];
                buffer.push(c);
                self.sequence[p] = 0; // Clear the stored parameter
                p += 1;

                // Exit conditions matching C: `while (c && *p != 0xff)`
                if c == 0 {
                    break;
                }
                if p >= self.sequence.len() {
                    break;
                }
                if self.sequence[p] == 0xff {
                    break;
                }
            }
        }

        // C appends a null terminator if we stopped at 0xff:
        //   `if (*p == 0xff) *buffer = 0;`
        // In Rust we include this for behavioral parity — callers that
        // process the Vec may rely on the trailing zero.
        if p < self.sequence.len() && self.sequence[p] == 0xff {
            buffer.push(0);
        }

        buffer
    }

    /// Reset the cheat sequence match position to the beginning.
    ///
    /// This does **not** clear any stored parameter bytes; it only
    /// resets the match cursor so the cheat can be re-triggered from
    /// the first character.
    pub fn reset(&mut self) {
        self.pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // scramble() tests
    // ------------------------------------------------------------------

    #[test]
    fn test_scramble_zero() {
        // All bits zero → all output bits zero.
        assert_eq!(scramble(0), 0);
    }

    #[test]
    fn test_scramble_all_ones() {
        // All bits set → all output bits set (bijective permutation).
        assert_eq!(scramble(0xFF), 0xFF);
    }

    #[test]
    fn test_scramble_single_bits() {
        // Verify each input bit maps to the documented output bit.
        assert_eq!(scramble(0x01), 0x80); // bit 0 → bit 7
        assert_eq!(scramble(0x02), 0x40); // bit 1 → bit 6
        assert_eq!(scramble(0x04), 0x04); // bit 2 → bit 2
        assert_eq!(scramble(0x08), 0x10); // bit 3 → bit 4
        assert_eq!(scramble(0x10), 0x08); // bit 4 → bit 3
        assert_eq!(scramble(0x20), 0x20); // bit 5 → bit 5
        assert_eq!(scramble(0x40), 0x02); // bit 6 → bit 1
        assert_eq!(scramble(0x80), 0x01); // bit 7 → bit 0
    }

    #[test]
    fn test_scramble_is_bijective() {
        // Every output value in 0..256 must appear exactly once.
        let mut seen = [false; 256];
        for i in 0u16..256 {
            let s = scramble(i as u8) as usize;
            assert!(
                !seen[s],
                "scramble produced duplicate output {} for input {}",
                s, i
            );
            seen[s] = true;
        }
        assert!(seen.iter().all(|&b| b), "not all outputs were produced");
    }

    #[test]
    fn test_scramble_known_value() {
        // scramble('i') = scramble(0x69)
        // 0x69 = 0b01101001
        // bit0(1)→bit7, bit3(1)→bit4, bit5(1)→bit5, bit6(1)→bit1
        // = 128 + 16 + 32 + 2 = 178 = 0xB2
        assert_eq!(scramble(b'i'), 0xB2);
    }

    // ------------------------------------------------------------------
    // CHEAT_XLATE_TABLE tests
    // ------------------------------------------------------------------

    #[test]
    fn test_xlate_table_length() {
        assert_eq!(CHEAT_XLATE_TABLE.len(), 256);
    }

    #[test]
    fn test_xlate_table_matches_scramble() {
        for i in 0u16..256 {
            assert_eq!(
                CHEAT_XLATE_TABLE[i as usize],
                scramble(i as u8),
                "table mismatch at index {}",
                i
            );
        }
    }

    // ------------------------------------------------------------------
    // CheatSeq basic tests
    // ------------------------------------------------------------------

    #[test]
    fn test_simple_cheat_sequence() {
        // Build a cheat for "ab" — two scrambled characters + end marker.
        let seq = [scramble(b'a'), scramble(b'b'), 0xff];
        let mut cheat = CheatSeq::new(&seq);

        assert!(!cheat.check_cheat(b'a')); // first char matches
        assert!(cheat.check_cheat(b'b')); // second char completes
    }

    #[test]
    fn test_mismatch_resets_sequence() {
        let seq = [scramble(b'a'), scramble(b'b'), scramble(b'c'), 0xff];
        let mut cheat = CheatSeq::new(&seq);

        assert!(!cheat.check_cheat(b'a')); // match
        assert!(!cheat.check_cheat(b'x')); // mismatch — resets

        // Must re-enter the full sequence from the beginning.
        assert!(!cheat.check_cheat(b'a'));
        assert!(!cheat.check_cheat(b'b'));
        assert!(cheat.check_cheat(b'c')); // complete
    }

    #[test]
    fn test_cheat_with_parameters() {
        // Simulate a cheat like IDMUS: coded part + delimiter + 2 param slots + end.
        let seq = [
            scramble(b'i'),
            scramble(b'd'),
            1, // parameter delimiter
            0, // param slot 1
            0, // param slot 2
            0xff,
        ];
        let mut cheat = CheatSeq::new(&seq);

        // Type the coded part.
        assert!(!cheat.check_cheat(b'i'));
        // After matching 'd', the delimiter (1) is skipped automatically.
        assert!(!cheat.check_cheat(b'd'));

        // Now we are at the first parameter slot (0).
        // Typing '1' stores it.
        assert!(!cheat.check_cheat(b'1'));
        // Typing '2' stores it and reaches 0xff — cheat complete.
        assert!(cheat.check_cheat(b'2'));
    }

    #[test]
    fn test_get_param_extracts_and_clears() {
        // Build a sequence with parameter section already filled in.
        let mut cheat = CheatSeq::new(&[
            scramble(b'x'),
            1,    // delimiter
            b'A', // stored param 1
            b'B', // stored param 2
            0xff,
        ]);

        let params = cheat.get_param();
        // Should contain ['A', 'B', 0] — the two params plus null terminator.
        assert_eq!(params, vec![b'A', b'B', 0]);

        // The parameter slots should now be zeroed in the sequence.
        assert_eq!(cheat.sequence[2], 0);
        assert_eq!(cheat.sequence[3], 0);
    }

    #[test]
    fn test_get_param_empty_slots() {
        // Parameters not filled in (still zeros).
        let mut cheat = CheatSeq::new(&[scramble(b'x'), 1, 0, 0, 0xff]);

        let params = cheat.get_param();
        // First slot is 0 — the do-while reads it, then exits because c == 0.
        assert_eq!(params, vec![0]);
    }

    #[test]
    fn test_reset() {
        let seq = [scramble(b'a'), scramble(b'b'), 0xff];
        let mut cheat = CheatSeq::new(&seq);

        assert!(!cheat.check_cheat(b'a')); // advance to pos 1
        cheat.reset();
        // After reset, must start over.
        assert!(!cheat.check_cheat(b'a'));
        assert!(cheat.check_cheat(b'b'));
    }

    #[test]
    fn test_cheat_reusable_after_completion() {
        let seq = [scramble(b'a'), scramble(b'b'), 0xff];
        let mut cheat = CheatSeq::new(&seq);

        // Complete the cheat once.
        assert!(!cheat.check_cheat(b'a'));
        assert!(cheat.check_cheat(b'b'));

        // The cheat should be reusable — pos was reset on completion.
        assert!(!cheat.check_cheat(b'a'));
        assert!(cheat.check_cheat(b'b'));
    }

    #[test]
    fn test_full_idkfa_like_sequence() {
        // Simulate a 5-character cheat like "idkfa".
        let seq = [
            scramble(b'i'),
            scramble(b'd'),
            scramble(b'k'),
            scramble(b'f'),
            scramble(b'a'),
            0xff,
        ];
        let mut cheat = CheatSeq::new(&seq);

        assert!(!cheat.check_cheat(b'i'));
        assert!(!cheat.check_cheat(b'd'));
        assert!(!cheat.check_cheat(b'k'));
        assert!(!cheat.check_cheat(b'f'));
        assert!(cheat.check_cheat(b'a'));
    }

    #[test]
    fn test_param_round_trip() {
        // Build a cheat with 2 parameter slots, activate it, extract params.
        let seq = [
            scramble(b'i'),
            scramble(b'd'),
            1, // delimiter
            0, // param slot 1
            0, // param slot 2
            0xff,
        ];
        let mut cheat = CheatSeq::new(&seq);

        // Activate the cheat.
        assert!(!cheat.check_cheat(b'i'));
        assert!(!cheat.check_cheat(b'd'));
        assert!(!cheat.check_cheat(b'3'));
        assert!(cheat.check_cheat(b'5'));

        // Extract parameters.
        let params = cheat.get_param();
        assert_eq!(params, vec![b'3', b'5', 0]);

        // After extraction, param slots are zeroed — cheat can be re-used.
        assert!(!cheat.check_cheat(b'i'));
        assert!(!cheat.check_cheat(b'd'));
        assert!(!cheat.check_cheat(b'7'));
        assert!(cheat.check_cheat(b'1'));

        let params2 = cheat.get_param();
        assert_eq!(params2, vec![b'7', b'1', 0]);
    }

    #[test]
    fn test_bounds_safety_empty_sequence() {
        // Edge case: empty sequence should not panic.
        let mut cheat = CheatSeq::new(&[]);
        assert!(!cheat.check_cheat(b'a'));
    }

    #[test]
    fn test_single_char_cheat() {
        // Minimal cheat: one character + end marker.
        let seq = [scramble(b'z'), 0xff];
        let mut cheat = CheatSeq::new(&seq);
        assert!(cheat.check_cheat(b'z'));
    }
}
