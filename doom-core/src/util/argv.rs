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

//! Translated from linuxdoom-1.10/m_argv.c and linuxdoom-1.10/m_argv.h
//!
//! Command-line argument parsing utilities.
//! Provides argument storage and lookup functionality used throughout the engine
//! for `-devparm`, `-warp`, `-file`, `-nomonsters`, `-respawn`, `-fast`, etc.
//!
//! In the original C code, `myargc` and `myargv` were global variables, and
//! `M_CheckParm` performed case-insensitive lookup returning a 1-based index
//! (0 if not found). This module replaces those globals with the [`Args`] struct,
//! which is passed by reference through the call chain per the global state
//! consolidation strategy (AAP §0.7.5).

/// Command-line argument storage and lookup.
///
/// Replaces the original C globals `myargc` / `myargv` and the `M_CheckParm`
/// function from `linuxdoom-1.10/m_argv.c`. The struct is intended to be
/// constructed once at program startup (typically from `std::env::args()`) and
/// then passed by shared reference (`&Args`) to any subsystem that needs to
/// inspect command-line parameters.
///
/// # Examples
///
/// ```
/// use doom_core::util::argv::Args;
///
/// let args = Args::new(["doom", "-skill", "4", "-warp", "1", "8"]);
/// assert_eq!(args.argc(), 6);
/// assert_eq!(args.check_parm("-skill"), Some(1));
/// assert_eq!(args.parm_value("-skill"), Some("4"));
/// assert!(args.has_parm("-warp"));
/// assert!(!args.has_parm("-nomonsters"));
/// ```
#[derive(Debug, Clone)]
pub struct Args {
    /// Stored arguments, including the program name at index 0.
    args: Vec<String>,
}

impl Args {
    /// Create a new [`Args`] from an iterator of string arguments.
    ///
    /// Typically constructed from `std::env::args()` at program startup, or
    /// from a literal slice in tests.
    ///
    /// # Arguments
    ///
    /// * `args` — An iterator yielding items convertible to `String`. The first
    ///   element is expected to be the program name (matching the C convention
    ///   where `argv[0]` is the executable path).
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::util::argv::Args;
    ///
    /// // From a fixed list (useful in tests):
    /// let args = Args::new(["doom", "-devparm"]);
    /// assert_eq!(args.argc(), 2);
    /// ```
    pub fn new<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Args {
            args: args.into_iter().map(|s| s.into()).collect(),
        }
    }

    /// Check for the given parameter in the program's command line arguments.
    ///
    /// Returns `Some(index)` with the 1-based argument number (1 to `argc - 1`),
    /// or `None` if the parameter is not present.
    ///
    /// This is the Rust equivalent of the C function:
    /// ```c
    /// int M_CheckParm(char *check);
    /// ```
    /// which returned 0 when not found. In Rust, `None` replaces the sentinel
    /// zero value.
    ///
    /// # Behavioral Details
    ///
    /// * The search is **case-insensitive**, matching the original `strcasecmp`
    ///   behavior in the C source.
    /// * The loop starts at index 1 (skipping `argv[0]`, the program name),
    ///   exactly as the original C implementation:
    ///   `for (i = 1; i < myargc; i++)`.
    /// * If the same parameter appears multiple times, the **first** occurrence
    ///   is returned (matching the original C behavior of returning immediately
    ///   on the first match).
    ///
    /// # Arguments
    ///
    /// * `check` — The parameter string to search for (e.g. `"-devparm"`).
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::util::argv::Args;
    ///
    /// let args = Args::new(["doom", "-devparm", "-warp", "1", "8"]);
    /// assert_eq!(args.check_parm("-devparm"), Some(1));
    /// assert_eq!(args.check_parm("-warp"), Some(2));
    /// assert_eq!(args.check_parm("-nomonsters"), None);
    ///
    /// // Case-insensitive matching:
    /// assert_eq!(args.check_parm("-DEVPARM"), Some(1));
    /// assert_eq!(args.check_parm("-DevParm"), Some(1));
    /// ```
    pub fn check_parm(&self, check: &str) -> Option<usize> {
        (1..self.args.len()).find(|&i| self.args[i].eq_ignore_ascii_case(check))
    }

    /// Number of arguments (including the program name at index 0).
    ///
    /// Equivalent to the C global `myargc`.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::util::argv::Args;
    ///
    /// let args = Args::new(["doom", "-devparm"]);
    /// assert_eq!(args.argc(), 2);
    ///
    /// let empty = Args::default();
    /// assert_eq!(empty.argc(), 0);
    /// ```
    #[inline]
    pub fn argc(&self) -> usize {
        self.args.len()
    }

    /// Get the argument at `index`, or `None` if the index is out of bounds.
    ///
    /// Equivalent to a bounds-checked read of the C global `myargv[index]`.
    ///
    /// # Arguments
    ///
    /// * `index` — Zero-based index into the argument list.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::util::argv::Args;
    ///
    /// let args = Args::new(["doom", "-skill", "4"]);
    /// assert_eq!(args.argv(0), Some("doom"));
    /// assert_eq!(args.argv(1), Some("-skill"));
    /// assert_eq!(args.argv(2), Some("4"));
    /// assert_eq!(args.argv(3), None);
    /// ```
    #[inline]
    pub fn argv(&self, index: usize) -> Option<&str> {
        self.args.get(index).map(|s| s.as_str())
    }

    /// Get the argument at `index`, panicking if out of bounds.
    ///
    /// Use this when the caller has already verified the index is valid (e.g.
    /// after receiving a `Some(i)` from [`check_parm`](Self::check_parm)).
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.argc()`.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::util::argv::Args;
    ///
    /// let args = Args::new(["doom", "-skill", "4"]);
    /// assert_eq!(args.argv_unchecked(0), "doom");
    /// assert_eq!(args.argv_unchecked(2), "4");
    /// ```
    #[inline]
    pub fn argv_unchecked(&self, index: usize) -> &str {
        &self.args[index]
    }

    /// Returns `true` if the given parameter is present in the arguments.
    ///
    /// This is a convenience wrapper around [`check_parm`](Self::check_parm)
    /// for callers that only need a boolean check (e.g. `-devparm`, `-nomonsters`).
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::util::argv::Args;
    ///
    /// let args = Args::new(["doom", "-nomonsters", "-fast"]);
    /// assert!(args.has_parm("-nomonsters"));
    /// assert!(args.has_parm("-fast"));
    /// assert!(!args.has_parm("-respawn"));
    /// ```
    #[inline]
    pub fn has_parm(&self, check: &str) -> bool {
        self.check_parm(check).is_some()
    }

    /// Returns the value of a parameter (the argument immediately after it),
    /// if the parameter is present and has a following argument.
    ///
    /// This is a convenience method for parameters that take a value, such as
    /// `-skill 4` or `-warp 1 8`. For multi-value parameters like `-warp`,
    /// use [`check_parm`](Self::check_parm) to get the index and then read
    /// subsequent arguments with [`argv`](Self::argv).
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::util::argv::Args;
    ///
    /// let args = Args::new(["doom", "-skill", "4", "-devparm"]);
    /// assert_eq!(args.parm_value("-skill"), Some("4"));
    /// // `-devparm` is the last argument — no value follows:
    /// assert_eq!(args.parm_value("-devparm"), None);
    /// // Parameter not present at all:
    /// assert_eq!(args.parm_value("-warp"), None);
    /// ```
    pub fn parm_value(&self, check: &str) -> Option<&str> {
        self.check_parm(check).and_then(|i| self.argv(i + 1))
    }
}

impl Default for Args {
    /// Create an empty [`Args`] with no arguments.
    ///
    /// Useful as a placeholder or in tests where no command-line arguments
    /// are needed.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::util::argv::Args;
    ///
    /// let args = Args::default();
    /// assert_eq!(args.argc(), 0);
    /// assert_eq!(args.check_parm("-anything"), None);
    /// ```
    fn default() -> Self {
        Args { args: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_from_slice() {
        let args = Args::new(["doom", "-skill", "4"]);
        assert_eq!(args.argc(), 3);
        assert_eq!(args.argv(0), Some("doom"));
        assert_eq!(args.argv(1), Some("-skill"));
        assert_eq!(args.argv(2), Some("4"));
    }

    #[test]
    fn test_new_from_strings() {
        let v = vec![
            String::from("doom"),
            String::from("-devparm"),
            String::from("-warp"),
        ];
        let args = Args::new(v);
        assert_eq!(args.argc(), 3);
    }

    #[test]
    fn test_default_is_empty() {
        let args = Args::default();
        assert_eq!(args.argc(), 0);
        assert_eq!(args.argv(0), None);
        assert_eq!(args.check_parm("-anything"), None);
        assert!(!args.has_parm("-anything"));
        assert_eq!(args.parm_value("-anything"), None);
    }

    #[test]
    fn test_check_parm_found() {
        let args = Args::new(["doom", "-devparm", "-warp", "1", "8"]);
        assert_eq!(args.check_parm("-devparm"), Some(1));
        assert_eq!(args.check_parm("-warp"), Some(2));
    }

    #[test]
    fn test_check_parm_not_found() {
        let args = Args::new(["doom", "-devparm"]);
        assert_eq!(args.check_parm("-nomonsters"), None);
        assert_eq!(args.check_parm("-respawn"), None);
    }

    #[test]
    fn test_check_parm_case_insensitive() {
        let args = Args::new(["doom", "-devparm", "-NoMonsters"]);
        // Original is lowercase, search uppercase:
        assert_eq!(args.check_parm("-DEVPARM"), Some(1));
        // Original is mixed case, search lowercase:
        assert_eq!(args.check_parm("-nomonsters"), Some(2));
        // Mixed case search:
        assert_eq!(args.check_parm("-DevParm"), Some(1));
    }

    #[test]
    fn test_check_parm_skips_argv0() {
        // The program name "doom" should never be matched by check_parm,
        // even if searched for, because the loop starts at index 1.
        let args = Args::new(["doom"]);
        assert_eq!(args.check_parm("doom"), None);
    }

    #[test]
    fn test_check_parm_returns_first_occurrence() {
        let args = Args::new(["doom", "-file", "a.wad", "-file", "b.wad"]);
        // Should return the first occurrence at index 1, not 3.
        assert_eq!(args.check_parm("-file"), Some(1));
    }

    #[test]
    fn test_has_parm() {
        let args = Args::new(["doom", "-nomonsters", "-fast"]);
        assert!(args.has_parm("-nomonsters"));
        assert!(args.has_parm("-fast"));
        assert!(!args.has_parm("-respawn"));
    }

    #[test]
    fn test_parm_value_with_value() {
        let args = Args::new(["doom", "-skill", "4", "-warp", "1"]);
        assert_eq!(args.parm_value("-skill"), Some("4"));
        assert_eq!(args.parm_value("-warp"), Some("1"));
    }

    #[test]
    fn test_parm_value_at_end() {
        // `-devparm` is the last argument, no value follows.
        let args = Args::new(["doom", "-skill", "4", "-devparm"]);
        assert_eq!(args.parm_value("-devparm"), None);
    }

    #[test]
    fn test_parm_value_not_present() {
        let args = Args::new(["doom", "-skill", "4"]);
        assert_eq!(args.parm_value("-warp"), None);
    }

    #[test]
    fn test_argv_bounds() {
        let args = Args::new(["doom", "-devparm"]);
        assert_eq!(args.argv(0), Some("doom"));
        assert_eq!(args.argv(1), Some("-devparm"));
        assert_eq!(args.argv(2), None);
        assert_eq!(args.argv(100), None);
    }

    #[test]
    fn test_argv_unchecked() {
        let args = Args::new(["doom", "-skill", "4"]);
        assert_eq!(args.argv_unchecked(0), "doom");
        assert_eq!(args.argv_unchecked(1), "-skill");
        assert_eq!(args.argv_unchecked(2), "4");
    }

    #[test]
    #[should_panic]
    fn test_argv_unchecked_panics_on_out_of_bounds() {
        let args = Args::new(["doom"]);
        let _ = args.argv_unchecked(5);
    }

    #[test]
    fn test_clone() {
        let args = Args::new(["doom", "-devparm"]);
        let cloned = args.clone();
        assert_eq!(cloned.argc(), 2);
        assert_eq!(cloned.check_parm("-devparm"), Some(1));
    }

    #[test]
    fn test_debug_format() {
        let args = Args::new(["doom", "-skill", "4"]);
        let debug_str = format!("{:?}", args);
        assert!(debug_str.contains("Args"));
        assert!(debug_str.contains("doom"));
        assert!(debug_str.contains("-skill"));
    }

    #[test]
    fn test_single_arg_program_name_only() {
        let args = Args::new(["doom"]);
        assert_eq!(args.argc(), 1);
        assert_eq!(args.argv(0), Some("doom"));
        assert_eq!(args.check_parm("-anything"), None);
        assert!(!args.has_parm("-anything"));
    }

    #[test]
    fn test_parm_value_case_insensitive() {
        let args = Args::new(["doom", "-Skill", "4"]);
        assert_eq!(args.parm_value("-skill"), Some("4"));
        assert_eq!(args.parm_value("-SKILL"), Some("4"));
    }
}
