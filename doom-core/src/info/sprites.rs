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

//! Translated from linuxdoom-1.10/info.h and linuxdoom-1.10/info.c
//!
//! Sprite name identifiers and lookup table.
//! Each sprite has a 4-character name used to locate sprite frames in WAD
//! lumps.
//!
//! Contains 138 sprite identifiers ([`NUMSPRITES`]) covering:
//! - Weapon sprites (SHTG, PUNG, PISG, CHGG, MISG, SAWG, PLSG, BFGG, etc.)
//! - Monster sprites (TROO, POSS, SPOS, VILE, SKEL, FATT, CYBR, SPID, etc.)
//! - Projectile sprites (BAL1, BAL2, PLSS, MISL, BFS1, etc.)
//! - Item/pickup sprites (ARM1, ARM2, BKEY, STIM, MEDI, SOUL, CLIP, etc.)
//! - Decoration sprites (COLU, COL1-COL6, TRE1, ELEC, GOR1-GOR5, etc.)

// ---------------------------------------------------------------------------
// SpriteNum enum (info.h lines 30-172)
// ---------------------------------------------------------------------------

/// Sprite identifiers for all in-game sprites.
///
/// Matches the C `spritenum_t` exactly. 138 entries (SPR_TROO=0 through
/// SPR_TLP2=137). The sentinel `NUMSPRITES` is provided as a separate
/// constant.
///
/// Each variant corresponds to a 4-character WAD lump name prefix stored
/// in the [`SPRITE_NAMES`] array. The renderer uses these to locate
/// sprite frames via the WAD file's sprite namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
#[allow(non_camel_case_types)]
pub enum SpriteNum {
    SPR_TROO = 0,
    SPR_SHTG = 1,
    SPR_PUNG = 2,
    SPR_PISG = 3,
    SPR_PISF = 4,
    SPR_SHTF = 5,
    SPR_SHT2 = 6,
    SPR_CHGG = 7,
    SPR_CHGF = 8,
    SPR_MISG = 9,
    SPR_MISF = 10,
    SPR_SAWG = 11,
    SPR_PLSG = 12,
    SPR_PLSF = 13,
    SPR_BFGG = 14,
    SPR_BFGF = 15,
    SPR_BLUD = 16,
    SPR_PUFF = 17,
    SPR_BAL1 = 18,
    SPR_BAL2 = 19,
    SPR_PLSS = 20,
    SPR_PLSE = 21,
    SPR_MISL = 22,
    SPR_BFS1 = 23,
    SPR_BFE1 = 24,
    SPR_BFE2 = 25,
    SPR_TFOG = 26,
    SPR_IFOG = 27,
    SPR_PLAY = 28,
    SPR_POSS = 29,
    SPR_SPOS = 30,
    SPR_VILE = 31,
    SPR_FIRE = 32,
    SPR_FATB = 33,
    SPR_FBXP = 34,
    SPR_SKEL = 35,
    SPR_MANF = 36,
    SPR_FATT = 37,
    SPR_CPOS = 38,
    SPR_SARG = 39,
    SPR_HEAD = 40,
    SPR_BAL7 = 41,
    SPR_BOSS = 42,
    SPR_BOS2 = 43,
    SPR_SKUL = 44,
    SPR_SPID = 45,
    SPR_BSPI = 46,
    SPR_APLS = 47,
    SPR_APBX = 48,
    SPR_CYBR = 49,
    SPR_PAIN = 50,
    SPR_SSWV = 51,
    SPR_KEEN = 52,
    SPR_BBRN = 53,
    SPR_BOSF = 54,
    SPR_ARM1 = 55,
    SPR_ARM2 = 56,
    SPR_BAR1 = 57,
    SPR_BEXP = 58,
    SPR_FCAN = 59,
    SPR_BON1 = 60,
    SPR_BON2 = 61,
    SPR_BKEY = 62,
    SPR_RKEY = 63,
    SPR_YKEY = 64,
    SPR_BSKU = 65,
    SPR_RSKU = 66,
    SPR_YSKU = 67,
    SPR_STIM = 68,
    SPR_MEDI = 69,
    SPR_SOUL = 70,
    SPR_PINV = 71,
    SPR_PSTR = 72,
    SPR_PINS = 73,
    SPR_MEGA = 74,
    SPR_SUIT = 75,
    SPR_PMAP = 76,
    SPR_PVIS = 77,
    SPR_CLIP = 78,
    SPR_AMMO = 79,
    SPR_ROCK = 80,
    SPR_BROK = 81,
    SPR_CELL = 82,
    SPR_CELP = 83,
    SPR_SHEL = 84,
    SPR_SBOX = 85,
    SPR_BPAK = 86,
    SPR_BFUG = 87,
    SPR_MGUN = 88,
    SPR_CSAW = 89,
    SPR_LAUN = 90,
    SPR_PLAS = 91,
    SPR_SHOT = 92,
    SPR_SGN2 = 93,
    SPR_COLU = 94,
    SPR_SMT2 = 95,
    SPR_GOR1 = 96,
    SPR_POL2 = 97,
    SPR_POL5 = 98,
    SPR_POL4 = 99,
    SPR_POL3 = 100,
    SPR_POL1 = 101,
    SPR_POL6 = 102,
    SPR_GOR2 = 103,
    SPR_GOR3 = 104,
    SPR_GOR4 = 105,
    SPR_GOR5 = 106,
    SPR_SMIT = 107,
    SPR_COL1 = 108,
    SPR_COL2 = 109,
    SPR_COL3 = 110,
    SPR_COL4 = 111,
    SPR_CAND = 112,
    SPR_CBRA = 113,
    SPR_COL6 = 114,
    SPR_TRE1 = 115,
    SPR_TRE2 = 116,
    SPR_ELEC = 117,
    SPR_CEYE = 118,
    SPR_FSKU = 119,
    SPR_COL5 = 120,
    SPR_TBLU = 121,
    SPR_TGRN = 122,
    SPR_TRED = 123,
    SPR_SMBT = 124,
    SPR_SMGT = 125,
    SPR_SMRT = 126,
    SPR_HDB1 = 127,
    SPR_HDB2 = 128,
    SPR_HDB3 = 129,
    SPR_HDB4 = 130,
    SPR_HDB5 = 131,
    SPR_HDB6 = 132,
    SPR_POB1 = 133,
    SPR_POB2 = 134,
    SPR_BRS1 = 135,
    SPR_TLMP = 136,
    SPR_TLP2 = 137,
}

/// Total number of sprites (sentinel value of the C enum).
pub const NUMSPRITES: usize = 138;

// ---------------------------------------------------------------------------
// SPRITE_NAMES — 4-character sprite lump name lookup (info.c lines 40-55)
// ---------------------------------------------------------------------------

/// The 4-character WAD lump name for each sprite identifier.
///
/// Indexed by [`SpriteNum`] as `usize`. Every name is exactly 4 uppercase
/// ASCII characters matching the original `char *sprnames[NUMSPRITES]`
/// array from `info.c`.
///
/// The renderer prepends each name with frame/rotation characters to form
/// the complete lump name (e.g., "TROO" → "TROOA0" for frame A, rotation
/// 0).
pub static SPRITE_NAMES: [&str; NUMSPRITES] = [
    "TROO", "SHTG", "PUNG", "PISG", "PISF", "SHTF", "SHT2", "CHGG", "CHGF", "MISG", "MISF", "SAWG",
    "PLSG", "PLSF", "BFGG", "BFGF", "BLUD", "PUFF", "BAL1", "BAL2", "PLSS", "PLSE", "MISL", "BFS1",
    "BFE1", "BFE2", "TFOG", "IFOG", "PLAY", "POSS", "SPOS", "VILE", "FIRE", "FATB", "FBXP", "SKEL",
    "MANF", "FATT", "CPOS", "SARG", "HEAD", "BAL7", "BOSS", "BOS2", "SKUL", "SPID", "BSPI", "APLS",
    "APBX", "CYBR", "PAIN", "SSWV", "KEEN", "BBRN", "BOSF", "ARM1", "ARM2", "BAR1", "BEXP", "FCAN",
    "BON1", "BON2", "BKEY", "RKEY", "YKEY", "BSKU", "RSKU", "YSKU", "STIM", "MEDI", "SOUL", "PINV",
    "PSTR", "PINS", "MEGA", "SUIT", "PMAP", "PVIS", "CLIP", "AMMO", "ROCK", "BROK", "CELL", "CELP",
    "SHEL", "SBOX", "BPAK", "BFUG", "MGUN", "CSAW", "LAUN", "PLAS", "SHOT", "SGN2", "COLU", "SMT2",
    "GOR1", "POL2", "POL5", "POL4", "POL3", "POL1", "POL6", "GOR2", "GOR3", "GOR4", "GOR5", "SMIT",
    "COL1", "COL2", "COL3", "COL4", "CAND", "CBRA", "COL6", "TRE1", "TRE2", "ELEC", "CEYE", "FSKU",
    "COL5", "TBLU", "TGRN", "TRED", "SMBT", "SMGT", "SMRT", "HDB1", "HDB2", "HDB3", "HDB4", "HDB5",
    "HDB6", "POB1", "POB2", "BRS1", "TLMP", "TLP2",
];

impl SpriteNum {
    /// Get the 4-character sprite name for this sprite identifier.
    ///
    /// Returns the WAD lump name prefix used to locate sprite frames.
    pub fn name(self) -> &'static str {
        SPRITE_NAMES[self as usize]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numsprites() {
        assert_eq!(NUMSPRITES, 138);
    }

    #[test]
    fn test_sprite_names_count() {
        assert_eq!(SPRITE_NAMES.len(), NUMSPRITES);
    }

    #[test]
    fn test_sprite_names_first() {
        assert_eq!(SPRITE_NAMES[SpriteNum::SPR_TROO as usize], "TROO");
    }

    #[test]
    fn test_sprite_names_last() {
        assert_eq!(SPRITE_NAMES[SpriteNum::SPR_TLP2 as usize], "TLP2");
    }

    #[test]
    fn test_sprite_names_key_entries() {
        assert_eq!(SPRITE_NAMES[SpriteNum::SPR_PLAY as usize], "PLAY");
        assert_eq!(SPRITE_NAMES[SpriteNum::SPR_CYBR as usize], "CYBR");
        assert_eq!(SPRITE_NAMES[SpriteNum::SPR_COLU as usize], "COLU");
        assert_eq!(SPRITE_NAMES[SpriteNum::SPR_POSS as usize], "POSS");
        assert_eq!(SPRITE_NAMES[SpriteNum::SPR_SPID as usize], "SPID");
    }

    #[test]
    fn test_sprite_name_method() {
        assert_eq!(SpriteNum::SPR_TROO.name(), "TROO");
        assert_eq!(SpriteNum::SPR_PLAY.name(), "PLAY");
        assert_eq!(SpriteNum::SPR_TLP2.name(), "TLP2");
    }

    #[test]
    fn test_all_names_are_four_chars() {
        for (i, name) in SPRITE_NAMES.iter().enumerate() {
            assert_eq!(
                name.len(),
                4,
                "Sprite name at index {} is '{}' ({} chars, expected 4)",
                i,
                name,
                name.len()
            );
        }
    }

    #[test]
    fn test_all_names_are_ascii_uppercase() {
        for (i, name) in SPRITE_NAMES.iter().enumerate() {
            for ch in name.chars() {
                assert!(
                    ch.is_ascii_uppercase() || ch.is_ascii_digit(),
                    "Sprite name at index {} ('{}') has non-uppercase char '{}'",
                    i,
                    name,
                    ch
                );
            }
        }
    }

    #[test]
    fn test_enum_contiguous() {
        // Verify the enum values are contiguous from 0 to NUMSPRITES-1
        assert_eq!(SpriteNum::SPR_TROO as usize, 0);
        assert_eq!(SpriteNum::SPR_TLP2 as usize, NUMSPRITES - 1);
    }
}
