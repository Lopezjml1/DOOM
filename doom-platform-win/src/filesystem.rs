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

//! Windows file system operations — IWAD discovery, config paths, save directories.
//!
//! Translated from `linuxdoom-1.10/d_main.c` (IWAD search) and
//! `linuxdoom-1.10/m_misc.c` (config/save path handling).
//!
//! Replaces Unix-specific paths (`/usr/local/share/games/doom/`, `~/.doomrc`)
//! with Windows-native equivalents using the `dirs` crate for known folder
//! resolution and hardcoded Steam common installation paths.
//!
//! ## Original C Behavior (d_main.c `IdentifyVersion`)
//!
//! The original engine searched for IWADs using the `DOOMWADDIR` environment
//! variable (defaulting to `"."`) and checked for `doom2.wad`, `doomu.wad`,
//! `doom.wad`, `doom1.wad`, `plutonia.wad`, `tnt.wad`, and `doom2f.wad` in
//! that directory via `access(path, R_OK)`.
//!
//! Config files were stored at `$HOME/.doomrc` (d_main.c line 614).
//!
//! ## Rust/Windows Replacement
//!
//! - **IWAD discovery**: CLI `--iwad` path (highest priority), then current
//!   working directory, then common Steam installation paths on Windows.
//! - **Config directory**: `dirs::config_dir()` → `AppData/Roaming/doom-rust`
//!   (replaces `$HOME/.doomrc`).
//! - **Save directory**: `<config_dir>/savegames` subdirectory.
//!
//! ## AAP Issue Resolution
//!
//! - IR-10: IWAD search paths are now Windows-specific with clear error messages.
//! - §0.8.2: Deterministic startup path with clear diagnostics.

use std::fs;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Common Steam installation directories where DOOM IWAD files are typically
/// located on Windows systems. These paths cover the most common retail
/// releases available through Steam.
///
/// Replaces the Unix `DOOMWADDIR` environment variable and default `"."`
/// directory from `d_main.c` line 578-580.
const STEAM_DOOM_PATHS: &[&str] = &[
    r"C:\Program Files (x86)\Steam\steamapps\common\Ultimate Doom\base",
    r"C:\Program Files (x86)\Steam\steamapps\common\Doom 2\base",
    r"C:\Program Files (x86)\Steam\steamapps\common\DOOM 3 BFG Edition\base\wads",
    r"C:\Program Files (x86)\Steam\steamapps\common\Final Doom\base",
];

/// Known IWAD filenames in priority order. The list includes both lowercase
/// and uppercase variants because Windows filesystems are case-insensitive
/// but some WAD tools produce specific casings.
///
/// This mirrors the search order from `d_main.c` `IdentifyVersion()`:
/// doom2.wad (commercial), plutonia.wad (commercial), tnt.wad (commercial),
/// doomu.wad (retail), doom.wad (registered), doom1.wad (shareware).
///
/// The French `doom2f.wad` from the original C code is included as well.
const IWAD_NAMES: &[&str] = &[
    "doom2.wad",
    "DOOM2.WAD",
    "doom2f.wad",
    "DOOM2F.WAD",
    "plutonia.wad",
    "PLUTONIA.WAD",
    "tnt.wad",
    "TNT.WAD",
    "doom.wad",
    "DOOM.WAD",
    "doom1.wad",
    "DOOM1.WAD",
    "doomu.wad",
    "DOOMU.WAD",
];

/// Application configuration directory name under the platform config root.
/// On Windows this resolves to `%APPDATA%/doom-rust` (i.e.
/// `C:\Users\<user>\AppData\Roaming\doom-rust`).
///
/// Replaces the Unix `$HOME/.doomrc` convention from `d_main.c` line 614.
const CONFIG_DIR_NAME: &str = "doom-rust";

/// Subdirectory name for save game files within the config directory.
const SAVE_DIR_NAME: &str = "savegames";

/// IWAD magic bytes: the ASCII string "IWAD" that must appear at offset 0
/// in a valid Internal WAD file. PWAD files begin with "PWAD" instead.
const IWAD_MAGIC: &[u8; 4] = b"IWAD";

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur during filesystem operations.
///
/// Per AAP §0.8.2 these error messages are designed to be user-facing and
/// include actionable fix suggestions.
#[derive(Debug, thiserror::Error)]
pub enum FileSystemError {
    /// The specified IWAD file could not be found at the given path.
    #[error(
        "IWAD file not found at path: {path}. \
         Please provide a valid path using --iwad <path>. \
         Common locations include:\n\
         - C:\\Program Files (x86)\\Steam\\steamapps\\common\\Ultimate Doom\\base\\DOOM.WAD\n\
         - C:\\Program Files (x86)\\Steam\\steamapps\\common\\Doom 2\\base\\DOOM2.WAD"
    )]
    IwadNotFound {
        /// The path that was searched or specified by the user.
        path: String,
    },

    /// Failed to create or access a configuration/save directory.
    #[error("Failed to create config directory: {0}")]
    ConfigDirCreation(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// IWAD discovery
// ---------------------------------------------------------------------------

/// Searches for a valid DOOM IWAD file across multiple locations.
///
/// The search order is:
/// 1. **CLI-provided path** (`--iwad` argument) — highest priority.
/// 2. **`DOOMWADDIR` environment variable** — if set, search that directory
///    for known IWAD filenames (preserves behavioral parity with the
///    original C engine's `getenv("DOOMWADDIR")` from d_main.c:578).
/// 3. **Current working directory** — scan for known IWAD filenames.
/// 4. **Steam common installation paths** — check well-known Steam
///    directories on Windows for DOOM IWADs.
///
/// Returns `Some(path)` with the first valid IWAD found, or `None` if no
/// IWAD could be located. The caller is responsible for producing a
/// user-facing error message (see [`FileSystemError::IwadNotFound`]).
///
/// # Arguments
///
/// * `cli_path` — Optional path provided via the `--iwad` command-line
///   argument. When present, only this specific path is checked (no
///   further search is performed).
///
/// # Examples
///
/// ```no_run
/// use doom_platform_win::filesystem::find_iwad;
///
/// // With an explicit CLI path
/// let wad = find_iwad(Some(r"C:\Games\DOOM2.WAD"));
///
/// // Automatic search (current dir + Steam paths)
/// let wad = find_iwad(None);
/// ```
pub fn find_iwad(cli_path: Option<&str>) -> Option<PathBuf> {
    // Priority 1: CLI-provided path (exact match).
    if let Some(path_str) = cli_path {
        let path = PathBuf::from(path_str);
        debug!("Checking CLI-provided IWAD path: {}", path.display());

        if validate_iwad_path(&path) {
            info!("IWAD found via --iwad: {}", path.display());
            return Some(path);
        }

        // The CLI path might be a directory rather than a file — scan it for
        // known IWAD names.
        if path.is_dir() {
            debug!(
                "CLI path is a directory; scanning for IWADs: {}",
                path.display()
            );
            if let Some(found) = search_directory_for_iwad(&path) {
                info!("IWAD found in CLI directory: {}", found.display());
                return Some(found);
            }
        }

        warn!(
            "CLI-provided IWAD path not valid: {}. Continuing search...",
            path.display()
        );
    }

    // Priority 2: DOOMWADDIR environment variable (behavioral parity with
    // the original C engine: d_main.c line 578).
    if let Ok(wad_dir) = std::env::var("DOOMWADDIR") {
        let dir = PathBuf::from(&wad_dir);
        debug!(
            "Checking DOOMWADDIR environment variable: {}",
            dir.display()
        );
        if dir.is_dir() {
            if let Some(found) = search_directory_for_iwad(&dir) {
                info!("IWAD found via DOOMWADDIR: {}", found.display());
                return Some(found);
            }
        } else {
            warn!(
                "DOOMWADDIR is set but is not a valid directory: {}",
                wad_dir
            );
        }
    }

    // Priority 3: Current working directory.
    if let Ok(cwd) = std::env::current_dir() {
        debug!("Searching current directory for IWADs: {}", cwd.display());
        if let Some(found) = search_directory_for_iwad(&cwd) {
            info!("IWAD found in current directory: {}", found.display());
            return Some(found);
        }
    } else {
        warn!("Could not determine current working directory");
    }

    // Priority 4: Steam common installation paths.
    for steam_path in STEAM_DOOM_PATHS {
        let dir = Path::new(steam_path);
        debug!("Searching Steam path for IWADs: {}", dir.display());

        if !dir.is_dir() {
            debug!("Steam directory does not exist: {}", dir.display());
            continue;
        }

        if let Some(found) = search_directory_for_iwad(dir) {
            info!("IWAD found in Steam directory: {}", found.display());
            return Some(found);
        }
    }

    warn!(
        "No IWAD file found in any search location. \
         Use --iwad <path> to specify the IWAD location."
    );
    None
}

// ---------------------------------------------------------------------------
// IWAD validation
// ---------------------------------------------------------------------------

/// Validates that the given path points to a readable IWAD file.
///
/// Performs two checks:
/// 1. **Existence and readability** — the file must exist and its metadata
///    must be accessible (equivalent to the original C `access(path, R_OK)`
///    check from `d_main.c` lines 658-709).
/// 2. **Magic bytes** — the first 4 bytes of the file must be the ASCII
///    string `"IWAD"`, confirming it is an Internal WAD and not a PWAD or
///    unrelated file.
///
/// # Arguments
///
/// * `path` — Path to the file to validate.
///
/// # Returns
///
/// `true` if the file exists, is readable, and begins with the IWAD magic
/// bytes. `false` otherwise.
pub fn validate_iwad_path(path: &Path) -> bool {
    // Check that the path points to an existing file (not a directory).
    match fs::metadata(path) {
        Ok(meta) => {
            if !meta.is_file() {
                debug!("Path is not a regular file: {}", path.display());
                return false;
            }
        }
        Err(e) => {
            debug!("Cannot access file metadata for {}: {}", path.display(), e);
            return false;
        }
    }

    // Read the first 4 bytes and check for the IWAD magic.
    match fs::read(path) {
        Ok(data) => {
            if data.len() < 4 {
                debug!(
                    "File too small to be a valid WAD ({}  bytes): {}",
                    data.len(),
                    path.display()
                );
                return false;
            }
            if &data[..4] == IWAD_MAGIC {
                debug!("Valid IWAD header confirmed: {}", path.display());
                true
            } else {
                debug!(
                    "File does not have IWAD magic bytes (got {:?}): {}",
                    &data[..4],
                    path.display()
                );
                false
            }
        }
        Err(e) => {
            debug!("Cannot read file {}: {}", path.display(), e);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Config directory
// ---------------------------------------------------------------------------

/// Returns the path to the application configuration directory.
///
/// On Windows this resolves to `%APPDATA%\doom-rust`
/// (e.g. `C:\Users\<user>\AppData\Roaming\doom-rust`).
///
/// Replaces the Unix `$HOME/.doomrc` convention from `d_main.c` line 614
/// and the `getenv("HOME")` call.
///
/// The directory is created if it does not already exist. If the platform
/// config directory cannot be determined (unlikely on Windows), the current
/// working directory is used as a fallback.
///
/// # Panics
///
/// Does not panic. Falls back to the current directory on error.
pub fn get_config_dir() -> PathBuf {
    let base = match dirs::config_dir() {
        Some(dir) => {
            debug!("Platform config root: {}", dir.display());
            dir
        }
        None => {
            warn!(
                "Could not determine platform config directory; \
                 falling back to current directory"
            );
            PathBuf::from(".")
        }
    };

    let config_path = base.join(CONFIG_DIR_NAME);

    // Create the directory tree if it doesn't exist yet.
    if !config_path.exists() {
        match fs::create_dir_all(&config_path) {
            Ok(()) => {
                info!("Created config directory: {}", config_path.display());
            }
            Err(e) => {
                warn!(
                    "Failed to create config directory {}: {}. \
                     Falling back to current directory.",
                    config_path.display(),
                    e
                );
                return PathBuf::from(".");
            }
        }
    }

    info!("Config directory: {}", config_path.display());
    config_path
}

// ---------------------------------------------------------------------------
// Save game directory
// ---------------------------------------------------------------------------

/// Returns the path to the save game directory.
///
/// This is a `savegames` subdirectory under the config directory returned
/// by [`get_config_dir`]. The directory is created if it does not exist.
///
/// # Examples
///
/// ```no_run
/// use doom_platform_win::filesystem::get_save_dir;
///
/// let save_dir = get_save_dir();
/// // e.g. C:\Users\<user>\AppData\Roaming\doom-rust\savegames
/// ```
pub fn get_save_dir() -> PathBuf {
    let base = get_config_dir();
    let save_path = base.join(SAVE_DIR_NAME);

    if !save_path.exists() {
        match fs::create_dir_all(&save_path) {
            Ok(()) => {
                info!("Created save directory: {}", save_path.display());
            }
            Err(e) => {
                warn!(
                    "Failed to create save directory {}: {}. \
                     Save games will use the config directory.",
                    save_path.display(),
                    e
                );
                return base;
            }
        }
    }

    info!("Save directory: {}", save_path.display());
    save_path
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Searches a directory for any of the known IWAD filenames.
///
/// Iterates through [`IWAD_NAMES`] and checks whether each file exists in
/// `dir` and passes [`validate_iwad_path`] validation.
///
/// Returns `Some(path)` for the first valid IWAD found, or `None`.
fn search_directory_for_iwad(dir: &Path) -> Option<PathBuf> {
    for name in IWAD_NAMES {
        let candidate = dir.join(name);
        debug!("  Checking: {}", candidate.display());
        if validate_iwad_path(&candidate) {
            return Some(candidate);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Creates a unique temporary directory for test isolation using the
    /// standard library only (no `tempfile` crate dependency).
    fn make_test_dir(suffix: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("doom_fs_test_{}_{}", std::process::id(), suffix));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    /// Cleans up a temporary test directory.
    fn cleanup_test_dir(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    /// Helper: creates a temporary IWAD file with valid magic bytes.
    fn create_temp_iwad(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = fs::File::create(&path).expect("create temp iwad");
        // Write IWAD header: magic (4 bytes) + numlumps (4 bytes) + infotableofs (4 bytes)
        f.write_all(b"IWAD").expect("write magic");
        f.write_all(&[0u8; 8]).expect("write header padding");
        f.flush().expect("flush");
        path
    }

    /// Helper: creates a temporary PWAD file (not an IWAD).
    fn create_temp_pwad(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = fs::File::create(&path).expect("create temp pwad");
        f.write_all(b"PWAD").expect("write magic");
        f.write_all(&[0u8; 8]).expect("write header padding");
        f.flush().expect("flush");
        path
    }

    #[test]
    fn validate_iwad_path_accepts_valid_iwad() {
        let tmp = make_test_dir("valid_iwad");
        let wad = create_temp_iwad(&tmp, "DOOM2.WAD");
        assert!(validate_iwad_path(&wad));
        cleanup_test_dir(&tmp);
    }

    #[test]
    fn validate_iwad_path_rejects_pwad() {
        let tmp = make_test_dir("reject_pwad");
        let wad = create_temp_pwad(&tmp, "extra.wad");
        assert!(!validate_iwad_path(&wad));
        cleanup_test_dir(&tmp);
    }

    #[test]
    fn validate_iwad_path_rejects_nonexistent() {
        let path = PathBuf::from("/nonexistent/DOOM.WAD");
        assert!(!validate_iwad_path(&path));
    }

    #[test]
    fn validate_iwad_path_rejects_directory() {
        let tmp = make_test_dir("reject_dir");
        assert!(!validate_iwad_path(&tmp));
        cleanup_test_dir(&tmp);
    }

    #[test]
    fn validate_iwad_path_rejects_too_small() {
        let tmp = make_test_dir("too_small");
        let path = tmp.join("tiny.wad");
        fs::write(&path, b"IW").expect("write tiny file");
        assert!(!validate_iwad_path(&path));
        cleanup_test_dir(&tmp);
    }

    #[test]
    fn find_iwad_cli_path_returns_valid() {
        let tmp = make_test_dir("find_cli");
        let wad = create_temp_iwad(&tmp, "DOOM.WAD");
        let result = find_iwad(Some(wad.to_str().unwrap()));
        assert_eq!(result, Some(wad));
        cleanup_test_dir(&tmp);
    }

    #[test]
    fn find_iwad_cli_directory_searches_names() {
        let tmp = make_test_dir("find_cli_dir");
        let _wad = create_temp_iwad(&tmp, "doom2.wad");
        let result = find_iwad(Some(tmp.to_str().unwrap()));
        assert!(result.is_some());
        assert!(result
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .eq_ignore_ascii_case("doom2.wad"));
        cleanup_test_dir(&tmp);
    }

    #[test]
    fn find_iwad_returns_none_when_nothing_found() {
        // Use a non-existent path to ensure no accidental match.
        let result = find_iwad(Some("/this/path/does/not/exist/DOOM.WAD"));
        assert!(result.is_none());
    }

    #[test]
    fn get_config_dir_returns_path() {
        let dir = get_config_dir();
        // The directory should either exist or have been created.
        assert!(dir.exists() || dir == PathBuf::from("."));
    }

    #[test]
    fn get_save_dir_returns_path() {
        let dir = get_save_dir();
        assert!(dir.exists() || dir == PathBuf::from("."));
    }

    #[test]
    fn filesystem_error_display() {
        let err = FileSystemError::IwadNotFound {
            path: "C:\\test\\DOOM.WAD".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("IWAD file not found"));
        assert!(msg.contains("--iwad"));
        assert!(msg.contains("C:\\test\\DOOM.WAD"));
    }

    #[test]
    fn filesystem_error_config_dir_creation() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err = FileSystemError::ConfigDirCreation(io_err);
        let msg = format!("{}", err);
        assert!(msg.contains("Failed to create config directory"));
        assert!(msg.contains("access denied"));
    }

    #[test]
    fn config_dir_name_constant() {
        assert_eq!(CONFIG_DIR_NAME, "doom-rust");
    }

    #[test]
    fn iwad_names_not_empty() {
        assert!(!IWAD_NAMES.is_empty());
        // Verify at least the core DOOM IWADs are present.
        let names: Vec<&str> = IWAD_NAMES.iter().copied().collect();
        assert!(names.contains(&"doom2.wad") || names.contains(&"DOOM2.WAD"));
        assert!(names.contains(&"doom.wad") || names.contains(&"DOOM.WAD"));
    }
}
