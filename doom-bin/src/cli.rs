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

//! Command-line argument definitions — translated from `linuxdoom-1.10/m_argv.h`.
//!
//! Replaces the `myargc`/`myargv` global argument system and `M_CheckParm()`
//! string-matching function with typed, validated CLI arguments via clap derive
//! macros.
//!
//! ## Original C Argument System
//!
//! The original DOOM used `M_CheckParm("-foo")` scattered throughout `d_main.c`
//! to check for command-line flags. Arguments were positional and untyped:
//! ```text
//! ./linuxxdoom -iwad doom2.wad -file mypwad.wad -warp 1 -skill 4
//! ```
//!
//! The C implementation was simple string matching over `argv`:
//! ```text
//! // m_argv.c
//! int M_CheckParm(char *check) {
//!     for (int i = 1; i < myargc; i++)
//!         if (!strcasecmp(check, myargv[i])) return i;
//!     return 0;
//! }
//! ```
//!
//! ## Rust CLI Interface
//!
//! The Rust version uses clap's derive API for structured, self-documenting
//! argument parsing with automatic `--help` and validation:
//! ```text
//! doom-rust --iwad DOOM2.WAD --pwad mypwad.wad --warp 1 1 --skill 4
//! ```
//!
//! ## Argument Mapping
//!
//! | Original C         | Rust CLI                          | Notes                              |
//! |---------------------|-----------------------------------|------------------------------------|
//! | `-iwad` (path search) | `--iwad <PATH>` (required)     | Explicit path, no auto-search here |
//! | `-file foo.wad`     | `--pwad foo.wad`                  | Repeatable `--pwad` flag           |
//! | `-warp 1 1`         | `--warp 1 1`                      | Typed `u32` parsing                |
//! | `-skill 4`          | `--skill 4`                       | Range-validated 1–5                |
//! | *(no equivalent)*   | `--verbose` / `-v`                | Controls tracing log level         |

use clap::Parser;

/// DOOM 1.10 Rust Port — Run DOOM on Windows 11
///
/// A faithful translation of the id Software DOOM 1.10 source code
/// from ANSI C / Linux / X11 to Rust with native Windows 11 support
/// via SDL2.
///
/// Requires a legally-owned DOOM IWAD file (DOOM.WAD, DOOM2.WAD, etc.)
/// from Steam or other legal source.
#[derive(Parser, Debug)]
#[command(name = "doom-rust")]
#[command(version)]
#[command(about = "DOOM 1.10 — Rust port for Windows 11")]
#[command(long_about = None)]
pub struct Cli {
    /// Path to IWAD file (DOOM.WAD, DOOM2.WAD, TNT.WAD, PLUTONIA.WAD)
    ///
    /// The IWAD (Internal WAD) contains the complete game data.
    /// You must provide your own legally-owned copy.
    ///
    /// Common Steam locations:
    ///   "C:\Program Files (x86)\Steam\steamapps\common\Ultimate Doom\base\DOOM.WAD"
    ///   "C:\Program Files (x86)\Steam\steamapps\common\Doom 2\base\DOOM2.WAD"
    ///
    /// Replaces the IWAD path search logic in d_main.c IdentifyVersion() which
    /// searched hardcoded Unix paths via the DOOMWADDIR environment variable.
    #[arg(long, required = true, value_name = "PATH")]
    pub iwad: String,

    /// Path to PWAD patch file(s) for user modifications
    ///
    /// PWADs (Patch WADs) contain user-created content that overrides
    /// or extends the base IWAD data. Multiple PWADs can be specified
    /// by repeating the flag: --pwad foo.wad --pwad bar.wad
    ///
    /// Each PWAD is loaded after the IWAD, with later PWADs overriding
    /// earlier ones (matching the original WAD loading order semantics
    /// from w_wad.c W_InitMultipleFiles).
    ///
    /// Replaces the original `-file` argument from d_main.c which used
    /// M_CheckParm("-file") and iterated subsequent argv entries.
    #[arg(long, value_name = "PATH")]
    pub pwad: Vec<String>,

    /// Warp directly to a specific level [episode map]
    ///
    /// For DOOM 1: --warp <episode> <map> (e.g., --warp 1 1 for E1M1)
    /// For DOOM 2: --warp <map> (e.g., --warp 1 for MAP01)
    ///
    /// Replaces the original `-warp` argument from d_main.c which used
    /// M_CheckParm("-warp") followed by myargv[p+1] and myargv[p+2].
    #[arg(long, num_args = 1..=2, value_name = "LEVEL")]
    pub warp: Option<Vec<u32>>,

    /// Difficulty skill level (1-5)
    ///
    /// 1 = I'm Too Young to Die  (sk_baby)
    /// 2 = Hey, Not Too Rough    (sk_easy)
    /// 3 = Hurt Me Plenty        (sk_medium) [default]
    /// 4 = Ultra-Violence        (sk_hard)
    /// 5 = Nightmare!            (sk_nightmare)
    ///
    /// The skill value maps to the internal GameSkill enum as (skill - 1),
    /// matching the original C conversion: myargv[p+1][0] - '1'.
    ///
    /// Replaces the original `-skill` argument from d_main.c.
    #[arg(
        long,
        default_value_t = 3,
        value_parser = clap::value_parser!(u8).range(1..=5),
        value_name = "LEVEL"
    )]
    pub skill: u8,

    /// Enable verbose/debug logging output
    ///
    /// When set, the default tracing log level is DEBUG instead of INFO.
    /// Can be overridden by the RUST_LOG environment variable for
    /// fine-grained control (e.g., RUST_LOG=doom_core=trace).
    ///
    /// This flag has no equivalent in the original C codebase, which used
    /// printf/fprintf(stderr, ...) with no runtime verbosity control.
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Helper to parse args using try_parse_from, which doesn't call
    /// process::exit on error (unlike parse()).
    fn parse_args(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn test_required_iwad() {
        // Missing --iwad should produce an error
        let result = parse_args(&["doom-rust"]);
        assert!(result.is_err(), "Should fail without --iwad");
    }

    #[test]
    fn test_iwad_only() {
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM2.WAD"]).unwrap();
        assert_eq!(cli.iwad, "DOOM2.WAD");
        assert!(cli.pwad.is_empty());
        assert!(cli.warp.is_none());
        assert_eq!(cli.skill, 3); // default
        assert!(!cli.verbose);
    }

    #[test]
    fn test_iwad_with_path() {
        let cli = parse_args(&[
            "doom-rust",
            "--iwad",
            r"C:\Program Files (x86)\Steam\steamapps\common\Doom 2\base\DOOM2.WAD",
        ])
        .unwrap();
        assert!(cli.iwad.contains("DOOM2.WAD"));
    }

    #[test]
    fn test_single_pwad() {
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM.WAD", "--pwad", "mypwad.wad"]).unwrap();
        assert_eq!(cli.pwad, vec!["mypwad.wad"]);
    }

    #[test]
    fn test_multiple_pwads() {
        let cli = parse_args(&[
            "doom-rust",
            "--iwad",
            "DOOM.WAD",
            "--pwad",
            "first.wad",
            "--pwad",
            "second.wad",
        ])
        .unwrap();
        assert_eq!(cli.pwad, vec!["first.wad", "second.wad"]);
    }

    #[test]
    fn test_warp_single_arg_doom2() {
        // DOOM 2 style: --warp <map>
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM2.WAD", "--warp", "15"]).unwrap();
        assert_eq!(cli.warp, Some(vec![15]));
    }

    #[test]
    fn test_warp_two_args_doom1() {
        // DOOM 1 style: --warp <episode> <map>
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM.WAD", "--warp", "3", "5"]).unwrap();
        assert_eq!(cli.warp, Some(vec![3, 5]));
    }

    #[test]
    fn test_warp_none_by_default() {
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM.WAD"]).unwrap();
        assert!(cli.warp.is_none());
    }

    #[test]
    fn test_skill_default() {
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM.WAD"]).unwrap();
        assert_eq!(cli.skill, 3); // "Hurt Me Plenty"
    }

    #[test]
    fn test_skill_valid_range() {
        for skill in 1..=5u8 {
            let cli = parse_args(&[
                "doom-rust",
                "--iwad",
                "DOOM.WAD",
                "--skill",
                &skill.to_string(),
            ])
            .unwrap();
            assert_eq!(cli.skill, skill);
        }
    }

    #[test]
    fn test_skill_zero_rejected() {
        let result = parse_args(&["doom-rust", "--iwad", "DOOM.WAD", "--skill", "0"]);
        assert!(result.is_err(), "Skill 0 should be rejected (range 1..=5)");
    }

    #[test]
    fn test_skill_six_rejected() {
        let result = parse_args(&["doom-rust", "--iwad", "DOOM.WAD", "--skill", "6"]);
        assert!(result.is_err(), "Skill 6 should be rejected (range 1..=5)");
    }

    #[test]
    fn test_skill_non_numeric_rejected() {
        let result = parse_args(&["doom-rust", "--iwad", "DOOM.WAD", "--skill", "hard"]);
        assert!(result.is_err(), "Non-numeric skill should be rejected");
    }

    #[test]
    fn test_verbose_long() {
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM.WAD", "--verbose"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn test_verbose_short() {
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM.WAD", "-v"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn test_verbose_default_false() {
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM.WAD"]).unwrap();
        assert!(!cli.verbose);
    }

    #[test]
    fn test_all_args_combined() {
        let cli = parse_args(&[
            "doom-rust",
            "--iwad",
            "DOOM.WAD",
            "--pwad",
            "patch.wad",
            "--warp",
            "1",
            "1",
            "--skill",
            "4",
            "--verbose",
        ])
        .unwrap();
        assert_eq!(cli.iwad, "DOOM.WAD");
        assert_eq!(cli.pwad, vec!["patch.wad"]);
        assert_eq!(cli.warp, Some(vec![1, 1]));
        assert_eq!(cli.skill, 4);
        assert!(cli.verbose);
    }

    #[test]
    fn test_debug_derive() {
        // Verify Debug is derived by formatting the struct
        let cli = parse_args(&["doom-rust", "--iwad", "DOOM.WAD"]).unwrap();
        let debug_output = format!("{:?}", cli);
        assert!(debug_output.contains("Cli"));
        assert!(debug_output.contains("DOOM.WAD"));
    }
}
