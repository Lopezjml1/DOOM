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

//! WAD file loading and lump management — translated from linuxdoom-1.10/w_wad.c.
//! Handles WAD file header parsing, directory construction, lump I/O.
//!
//! This module contains the primary [`WadFile`] struct that implements the
//! complete WAD loading pipeline: file opening, header validation, directory
//! parsing, lump name lookup (with backward scan for PWAD override semantics),
//! lump data reading, and the [`WadProvider`] trait integration with
//! [`LumpCache`] for cached access.
//!
//! # Architecture
//!
//! The original C engine used global mutable state (`lumpinfo`, `numlumps`,
//! `lumpcache`, `reloadname`, `reloadlump`) spread across `w_wad.c`. In the
//! Rust port, all state is consolidated into the owned [`WadFile`] struct,
//! eliminating global variables and enabling the Rust borrow checker to
//! enforce safe access patterns.
//!
//! # Source Mapping
//!
//! | Rust Method/Function | C Function (w_wad.c) | Lines |
//! |---|---|---|
//! | [`extract_file_base`] | `ExtractFileBase` | 85–114 |
//! | [`WadFile::new`] | *(constructor)* | — |
//! | [`WadFile::add_file`] | `W_AddFile` | 141–226 |
//! | [`WadFile::init_multiple_files`] | `W_InitMultipleFiles` | 292–316 |
//! | [`WadFile::reload`] | `W_Reload` | 236–275 |
//! | [`WadFile::check_num_for_name`] | `W_CheckNumForName` | 351–390 |
//! | [`WadFile::get_num_for_name`] | `W_GetNumForName` | 399–409 |
//! | [`WadFile::lump_length`] | `W_LumpLength` | 416–422 |
//! | [`WadFile::num_lumps`] | `W_NumLumps` | 339–342 |
//! | [`WadFile::read_lump`] | `W_ReadLump` | 431–467 |
//! | [`WadFile::cache_lump_num`] | `W_CacheLumpNum` | 475–500 |
//! | [`WadFile::cache_lump_name`] | `W_CacheLumpName` | 507–513 |

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use tracing::{debug, error, info, warn};

use crate::lump_cache::LumpCache;
use crate::types::{FileLump, LumpInfo, LumpNum, PurgeTag, WadError, WadInfo, WadType};
use crate::wad_provider::WadProvider;

// =============================================================================
// Internal helper functions
// =============================================================================

/// Extracts the filename base (up to 8 uppercase characters, no extension)
/// from a file path — equivalent of `ExtractFileBase` at `w_wad.c:85-114`.
///
/// The original C code walked backwards from the end of the string to find the
/// last `\` or `/`, then copied up to 8 characters before `.`, converting each
/// to uppercase. In Rust, we use [`Path::file_stem`] for cross-platform path
/// handling and then apply the 8-character limit with uppercase conversion.
///
/// # Errors
///
/// Returns [`WadError::FilenameTooLong`] if the stem exceeds 8 characters,
/// matching `I_Error("Filename base of %s >8 chars", path)` at `w_wad.c:110`.
fn extract_file_base(path: &str) -> Result<[u8; 8], WadError> {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    if stem.len() > 8 {
        return Err(WadError::FilenameTooLong(path.to_string()));
    }

    let mut name = [0u8; 8];
    for (i, byte) in stem.bytes().enumerate() {
        if i >= 8 {
            break;
        }
        name[i] = byte.to_ascii_uppercase();
    }

    Ok(name)
}

/// Converts a lump name string to an 8-byte uppercase, null-padded array
/// suitable for comparison against stored lump names.
///
/// This replaces the `strncpy` + `strupr` sequence at `w_wad.c:364-370`:
/// ```c
/// strncpy(name8.s, name, 8);
/// name8.s[8] = 0;
/// strupr(name8.s);
/// ```
fn name_to_8bytes(name: &str) -> [u8; 8] {
    let mut result = [0u8; 8];
    for (i, byte) in name.bytes().enumerate() {
        if i >= 8 {
            break;
        }
        result[i] = byte.to_ascii_uppercase();
    }
    result
}

/// Identifies the WAD type from the 4-byte header identification field.
///
/// Returns [`WadType::Iwad`] for `"IWAD"`, [`WadType::Pwad`] for `"PWAD"`,
/// or `None` for unrecognized identifications. Replaces the `strncmp` checks
/// at `w_wad.c:185-192`.
fn identify_wad_type(identification: &[u8; 4]) -> Option<WadType> {
    match identification {
        b"IWAD" => Some(WadType::Iwad),
        b"PWAD" => Some(WadType::Pwad),
        _ => None,
    }
}

// =============================================================================
// WadFile — Primary WAD loading struct
// =============================================================================

/// WAD file loader and lump manager — the primary translation of `w_wad.c`.
///
/// Consolidates the three global variables (`lumpinfo`, `numlumps`,
/// `lumpcache`) plus the reload state (`reloadname`, `reloadlump`) from the
/// original C code into a single owned struct.
///
/// # Usage
///
/// ```no_run
/// use doom_wad::wad_file::WadFile;
/// use doom_wad::WadProvider;
/// use doom_wad::PurgeTag;
///
/// let mut wad = WadFile::init_multiple_files(&["DOOM.WAD"])
///     .expect("failed to load WAD files");
///
/// // Look up a lump by name (backward scan for PWAD override semantics)
/// if let Some(idx) = wad.check_num_for_name("PLAYPAL") {
///     let data = wad.read_lump(idx);
///     assert_eq!(data.len(), wad.lump_length(idx));
/// }
///
/// // Cached access with tag-based eviction
/// let palette = wad.cache_lump_name("PLAYPAL", PurgeTag::Cache);
/// assert_eq!(palette.len(), 768 * 14); // 14 palettes × 768 bytes
/// ```
pub struct WadFile {
    /// Runtime lump directory — replaces global `lumpinfo_t* lumpinfo`
    /// (`w_wad.c:61`) and `int numlumps` (`w_wad.c:62`).
    lump_info: Vec<LumpInfo>,

    /// HashMap-based lump cache — replaces global `void** lumpcache`
    /// (`w_wad.c:64`) and zone allocator tag tracking.
    lump_cache: LumpCache,

    /// Open file handles indexed by handle number. Each `add_file` call
    /// pushes one entry: `Some(File)` for normal files, `None` for reload
    /// files (replacing the C convention of `handle = -1`).
    file_handles: Vec<Option<File>>,

    /// Path to the reloadable file, if any. Replaces global
    /// `char* reloadname` (`w_wad.c:138`). When set, lumps with
    /// `handle = None` are read by opening this file on demand.
    reload_name: Option<String>,

    /// First lump index belonging to the reload file. Replaces global
    /// `int reloadlump` (`w_wad.c:137`). Used by [`reload`](WadFile::reload)
    /// to identify which lump directory entries to update.
    reload_lump: usize,
}

impl WadFile {
    // =========================================================================
    // Construction
    // =========================================================================

    /// Creates a new, empty `WadFile` with no lumps loaded.
    ///
    /// Use [`add_file`](WadFile::add_file) to load WAD files, or
    /// [`init_multiple_files`](WadFile::init_multiple_files) for batch loading.
    pub fn new() -> Self {
        WadFile {
            lump_info: Vec::new(),
            lump_cache: LumpCache::new(),
            file_handles: Vec::new(),
            reload_name: None,
            reload_lump: 0,
        }
    }

    // =========================================================================
    // File loading — W_AddFile (w_wad.c:141-226)
    // =========================================================================

    /// Adds a single WAD or lump file to the directory.
    ///
    /// Equivalent of `W_AddFile` at `w_wad.c:141-226`. All files are optional,
    /// but at least one file must be found (checked by
    /// [`init_multiple_files`](WadFile::init_multiple_files)).
    ///
    /// # File Type Detection (w_wad.c:172-203)
    ///
    /// - Files ending with `"wad"` (case-insensitive) are treated as WAD files
    ///   containing multiple lumps with a directory header.
    /// - All other files are treated as single-lump files, with the filename
    ///   base (up to 8 uppercase characters, no extension) as the lump name.
    ///
    /// # Reload Support (w_wad.c:156-161)
    ///
    /// If the filename starts with `'~'`, the tilde is stripped and the file is
    /// registered as the reload file. Its lumps are stored with a sentinel
    /// handle (`None`), and [`reload`](WadFile::reload) can re-read the
    /// directory and invalidate cached data.
    ///
    /// # Errors
    ///
    /// Returns [`WadError::FileOpen`] if the file cannot be opened,
    /// [`WadError::InvalidWad`] if a `.wad` file has an unrecognized header,
    /// [`WadError::FilenameTooLong`] if a single-lump file's base name exceeds
    /// 8 characters, or [`WadError::Io`] for other I/O failures.
    pub fn add_file(&mut self, filename: &str) -> Result<(), WadError> {
        let mut actual_filename = filename;

        // Handle reload indicator (w_wad.c:156-161):
        // "If filename starts with a tilde, the file is handled specially
        //  to allow map reloads."
        if actual_filename.starts_with('~') {
            actual_filename = &actual_filename[1..];
            self.reload_name = Some(actual_filename.to_string());
            self.reload_lump = self.lump_info.len();
        }

        // Open the file (w_wad.c:163-167)
        // Replaces: open(filename, O_RDONLY | O_BINARY)
        let file = match File::open(actual_filename) {
            Ok(f) => f,
            Err(_) => {
                // w_wad.c:165: printf(" couldn't open %s\n", filename);
                warn!("couldn't open {}", actual_filename);
                return Err(WadError::FileOpen(actual_filename.to_string()));
            }
        };

        // Detect WAD vs single lump (w_wad.c:172)
        // Original C: strcmpi(filename + strlen(filename) - 3, "wad")
        let is_wad = actual_filename.to_ascii_lowercase().ends_with("wad");

        let file_lumps: Vec<FileLump> = if !is_wad {
            // Single lump file (w_wad.c:174-179)
            let file_size = file.metadata().map_err(WadError::Io)?.len() as i32;
            let name = extract_file_base(actual_filename)?;

            // w_wad.c:169: printf(" adding %s\n", filename);
            info!("adding {} (single lump)", actual_filename);

            vec![FileLump {
                filepos: 0,
                size: file_size,
                name,
            }]
        } else {
            // WAD file (w_wad.c:183-203)
            let mut reader = BufReader::new(&file);

            // Read wadinfo_t header — 12 bytes (w_wad.c:184)
            // Replaces: read(handle, &header, sizeof(header))
            let mut identification = [0u8; 4];
            reader.read_exact(&mut identification)?;
            let numlumps = reader.read_i32::<LittleEndian>()?;
            let infotableofs = reader.read_i32::<LittleEndian>()?;

            // Construct WadInfo for validation and logging
            let header = WadInfo {
                identification,
                numlumps,
                infotableofs,
            };

            // Validate identification (w_wad.c:185-192)
            let wad_type = identify_wad_type(&header.identification)
                .ok_or_else(|| WadError::InvalidWad(actual_filename.to_string()))?;

            // w_wad.c:169: printf(" adding %s\n", filename);
            info!(
                "adding {} ({}, {} lumps)",
                actual_filename, wad_type, header.numlumps
            );

            // Validate infotableofs before using as seek offset.
            // A negative i32 wraps to a huge u64, causing a seek error
            // or reading garbage from a malformed WAD file.
            if header.infotableofs < 0 {
                return Err(WadError::InvalidWad(actual_filename.to_string()));
            }

            // Validate numlumps for the same reason — a negative count
            // would wrap to a huge usize in the loop below.
            if header.numlumps < 0 {
                return Err(WadError::InvalidWad(actual_filename.to_string()));
            }

            // Seek to directory and read entries (w_wad.c:200-201)
            // Replaces: lseek(handle, header.infotableofs, SEEK_SET)
            //           read(handle, fileinfo, length)
            reader.seek(SeekFrom::Start(header.infotableofs as u64))?;

            let count = header.numlumps as usize;
            let mut lumps = Vec::with_capacity(count);
            for _ in 0..count {
                // Each filelump_t is 16 bytes: filepos(4) + size(4) + name(8)
                // Replaces LONG() macro from m_swap.h for endian conversion
                let filepos = reader.read_i32::<LittleEndian>()?;
                let size = reader.read_i32::<LittleEndian>()?;
                let mut name = [0u8; 8];
                reader.read_exact(&mut name)?;
                lumps.push(FileLump {
                    filepos,
                    size,
                    name,
                });
            }

            lumps
            // reader (BufReader<&File>) dropped here — releases borrow on file
        };

        // Determine handle storage (w_wad.c:214):
        // storehandle = reloadname ? -1 : handle;
        // When reload is active, ALL files get sentinel handle (None).
        let handle_idx = self.file_handles.len();
        let store_handle = if self.reload_name.is_some() {
            None
        } else {
            Some(handle_idx)
        };

        // Store or drop file handle (w_wad.c:224-225)
        if self.reload_name.is_some() {
            self.file_handles.push(None);
            // file is dropped here — equivalent to close(handle) at w_wad.c:225
            drop(file);
        } else {
            self.file_handles.push(Some(file));
        }

        // Build lump directory (w_wad.c:216-222)
        for file_lump in &file_lumps {
            // Copy name and uppercase for consistent comparison
            // w_wad.c:221: strncpy(lump_p->name, fileinfo->name, 8);
            let mut name = file_lump.name;
            name.iter_mut().for_each(|b| *b = b.to_ascii_uppercase());

            self.lump_info.push(LumpInfo {
                name,
                handle: store_handle,
                position: file_lump.filepos,
                size: file_lump.size,
            });
        }

        debug!(
            "loaded {} lumps (total: {})",
            file_lumps.len(),
            self.lump_info.len()
        );

        Ok(())
    }

    // =========================================================================
    // Batch loading — W_InitMultipleFiles (w_wad.c:292-316)
    // =========================================================================

    /// Initializes a `WadFile` from multiple file paths.
    ///
    /// Equivalent of `W_InitMultipleFiles` at `w_wad.c:292-316`. Calls
    /// [`add_file`](WadFile::add_file) for each filename and returns an error
    /// if no lumps were loaded.
    ///
    /// # Errors
    ///
    /// Returns [`WadError::NoFilesFound`] if zero lumps are loaded across all
    /// files, matching `I_Error("W_InitFiles: no files found")` at
    /// `w_wad.c:305-306`. Individual file open failures are propagated from
    /// [`add_file`](WadFile::add_file).
    pub fn init_multiple_files(filenames: &[&str]) -> Result<Self, WadError> {
        let mut wad = WadFile::new();

        // w_wad.c:302-303: Iterate and add each file
        for filename in filenames {
            wad.add_file(filename)?;
        }

        // w_wad.c:305-306: if (!numlumps) I_Error("W_InitFiles: no files found");
        if wad.lump_info.is_empty() {
            return Err(WadError::NoFilesFound);
        }

        // w_wad.c:309-315: Initialize lumpcache (replaced by LumpCache::new()
        // which was already called in WadFile::new())
        info!(
            "W_InitMultipleFiles: {} lumps loaded from {} files",
            wad.lump_info.len(),
            filenames.len()
        );

        Ok(wad)
    }

    // =========================================================================
    // Reload — W_Reload (w_wad.c:236-275)
    // =========================================================================

    /// Reloads the reload file and invalidates cached lumps in its range.
    ///
    /// Equivalent of `W_Reload` at `w_wad.c:236-275`. This is designed for
    /// development use: a PWAD file prefixed with `'~'` can be reloaded at
    /// runtime after being modified externally (e.g., by a map editor).
    ///
    /// # Behavior
    ///
    /// 1. If no reload file was registered (no `'~'` prefix), returns
    ///    immediately with `Ok(())`.
    /// 2. Opens the reload file and re-reads its WAD directory.
    /// 3. For each lump in the reload range, invalidates the cache entry
    ///    (equivalent of `Z_Free(lumpcache[i])` at `w_wad.c:267-268`).
    /// 4. Updates stored positions and sizes for the reloaded lumps.
    ///
    /// # Errors
    ///
    /// Returns [`WadError::ReloadError`] if the reload file cannot be opened.
    pub fn reload(&mut self) -> Result<(), WadError> {
        // w_wad.c:246-247: if (!reloadname) return;
        let reload_name = match &self.reload_name {
            Some(name) => name.clone(),
            None => return Ok(()),
        };

        debug!("reloading {}", reload_name);

        // w_wad.c:249: Open reload file
        let file =
            File::open(&reload_name).map_err(|_| WadError::ReloadError(reload_name.clone()))?;
        let mut reader = BufReader::new(&file);

        // w_wad.c:252-258: Read header
        let mut identification = [0u8; 4];
        reader
            .read_exact(&mut identification)
            .map_err(|_| WadError::ReloadError(reload_name.clone()))?;
        let lumpcount = reader
            .read_i32::<LittleEndian>()
            .map_err(|_| WadError::ReloadError(reload_name.clone()))?
            as usize;
        let infotableofs = reader
            .read_i32::<LittleEndian>()
            .map_err(|_| WadError::ReloadError(reload_name.clone()))?;

        // Validate infotableofs — a negative offset wraps to a huge u64.
        if infotableofs < 0 {
            return Err(WadError::ReloadError(reload_name.clone()));
        }

        // w_wad.c:260: Seek to directory
        reader
            .seek(SeekFrom::Start(infotableofs as u64))
            .map_err(|_| WadError::ReloadError(reload_name.clone()))?;

        // Read directory entries
        let mut file_lumps = Vec::with_capacity(lumpcount);
        for _ in 0..lumpcount {
            let filepos = reader
                .read_i32::<LittleEndian>()
                .map_err(|_| WadError::ReloadError(reload_name.clone()))?;
            let size = reader
                .read_i32::<LittleEndian>()
                .map_err(|_| WadError::ReloadError(reload_name.clone()))?;
            let mut name = [0u8; 8];
            reader
                .read_exact(&mut name)
                .map_err(|_| WadError::ReloadError(reload_name.clone()))?;
            file_lumps.push(FileLump {
                filepos,
                size,
                name,
            });
        }

        // w_wad.c:261-271: Update lump info and invalidate cache entries
        let reload_lump = self.reload_lump;
        for (i, file_lump) in file_lumps.iter().enumerate() {
            let lump_idx = reload_lump + i;
            if lump_idx < self.lump_info.len() {
                // w_wad.c:267-268: if (lumpcache[i]) Z_Free(lumpcache[i]);
                self.lump_cache.invalidate(lump_idx);

                // w_wad.c:270-271: Update position and size
                self.lump_info[lump_idx].position = file_lump.filepos;
                self.lump_info[lump_idx].size = file_lump.size;
            }
        }

        info!("reloaded {} lumps from {}", file_lumps.len(), reload_name);

        // File is automatically closed when `file` goes out of scope
        // (equivalent of close(handle) at w_wad.c:274)
        Ok(())
    }

    // =========================================================================
    // Lump lookup — W_CheckNumForName / W_GetNumForName (w_wad.c:351-409)
    // =========================================================================

    /// Checks if a lump with the given name exists, returning its index.
    ///
    /// Equivalent of `W_CheckNumForName` at `w_wad.c:351-390`.
    ///
    /// # Critical Algorithm — Backward Scan
    ///
    /// The scan proceeds **backward** from the last lump to the first
    /// (`w_wad.c:377`: "scan backwards so patch lump files take precedence").
    /// This is essential for correct PWAD override behavior: if both the IWAD
    /// and a PWAD contain a lump named `"MAP01"`, the PWAD's lump (loaded
    /// later, at a higher index) is returned.
    ///
    /// # Name Comparison
    ///
    /// The name is converted to uppercase, padded to 8 bytes with nulls, and
    /// compared against stored lump names using 8-byte equality. This replaces
    /// the union/int-cast trick at `w_wad.c:353-357, 372-373, 381-382`.
    pub fn check_num_for_name(&self, name: &str) -> Option<usize> {
        // w_wad.c:364-370: Convert search name to uppercase 8-byte array
        let search = name_to_8bytes(name);

        // w_wad.c:377-386: Backward scan
        // "scan backwards so patch lump files take precedence"
        (0..self.lump_info.len())
            .rev()
            .find(|&i| self.lump_info[i].name == search)
    }

    /// Returns the lump index for a given name, or an error if not found.
    ///
    /// Equivalent of `W_GetNumForName` at `w_wad.c:399-409`. This is the
    /// strict version of [`check_num_for_name`](WadFile::check_num_for_name):
    /// it returns a [`WadError::LumpNotFound`] instead of `None` when the
    /// lump does not exist, matching `I_Error("W_GetNumForName: %s not found!")`.
    ///
    /// # Errors
    ///
    /// Returns [`WadError::LumpNotFound`] if the lump name is not in the
    /// directory.
    pub fn get_num_for_name(&self, name: &str) -> Result<usize, WadError> {
        self.check_num_for_name(name)
            .ok_or_else(|| WadError::LumpNotFound(name.to_string()))
    }

    // =========================================================================
    // Lump info — W_LumpLength / W_NumLumps (w_wad.c:339-422)
    // =========================================================================

    /// Returns the uncompressed size of the specified lump in bytes.
    ///
    /// Equivalent of `W_LumpLength` at `w_wad.c:416-422`.
    ///
    /// # Panics
    ///
    /// Panics if `lump` is out of bounds, matching the original
    /// `I_Error("W_LumpLength: %i >= numlumps", lump)` at `w_wad.c:419-420`.
    pub fn lump_length(&self, lump: usize) -> usize {
        if lump >= self.lump_info.len() {
            // w_wad.c:419-420
            error!(
                "W_LumpLength: {} >= numlumps ({})",
                lump,
                self.lump_info.len()
            );
            panic!(
                "W_LumpLength: {} >= numlumps ({})",
                lump,
                self.lump_info.len()
            );
        }
        self.lump_info[lump].size as usize
    }

    /// Returns the total number of lumps loaded across all WAD files.
    ///
    /// Equivalent of `W_NumLumps` at `w_wad.c:339-342`:
    /// ```c
    /// int W_NumLumps(void) { return numlumps; }
    /// ```
    pub fn num_lumps(&self) -> usize {
        self.lump_info.len()
    }

    // =========================================================================
    // Lump reading — W_ReadLump (w_wad.c:431-467)
    // =========================================================================

    /// Reads the full contents of a lump from disk into a new `Vec<u8>`.
    ///
    /// Equivalent of `W_ReadLump` at `w_wad.c:431-467`. For lumps from
    /// reload files (handle = `None`), the reload file is opened, read,
    /// and closed on each call. For normal lumps, the stored file handle
    /// is used with seek + read.
    ///
    /// # Panics
    ///
    /// Panics if the lump index is out of bounds or if the disk read fails,
    /// matching the original `I_Error` calls.
    pub fn read_lump(&self, lump: usize) -> Vec<u8> {
        Self::read_lump_raw(&self.lump_info, &self.file_handles, &self.reload_name, lump)
    }

    // =========================================================================
    // Cached lump access — W_CacheLumpNum / W_CacheLumpName (w_wad.c:475-513)
    // =========================================================================

    /// Returns cached lump data by index, reading from disk on cache miss.
    ///
    /// Equivalent of `W_CacheLumpNum` at `w_wad.c:475-500`. Uses the
    /// [`LumpCache`] (replacing `lumpcache[]` + zone allocator) to store
    /// previously read data with tag-based eviction semantics.
    ///
    /// # Split Borrow Pattern
    ///
    /// This method uses Rust's split borrow capability: `lump_cache` is
    /// borrowed mutably while `lump_info`, `file_handles`, and `reload_name`
    /// are borrowed immutably. These are disjoint struct fields, which the
    /// borrow checker accepts.
    ///
    /// # Panics
    ///
    /// Panics if `lump` is out of bounds.
    pub fn cache_lump_num(&mut self, lump: usize, tag: PurgeTag) -> &[u8] {
        if lump >= self.lump_info.len() {
            error!(
                "W_CacheLumpNum: {} >= numlumps ({})",
                lump,
                self.lump_info.len()
            );
            panic!(
                "W_CacheLumpNum: {} >= numlumps ({})",
                lump,
                self.lump_info.len()
            );
        }

        // Use LumpNum for structured logging
        let _lump_num = LumpNum::from(lump);
        debug!("cache_lump_num: lump={}, tag={:?}", lump, tag);

        // Split borrows: immutable refs to file access fields,
        // mutable ref to cache. Rust allows disjoint field borrows.
        let lump_info = &self.lump_info;
        let file_handles = &self.file_handles;
        let reload_name = &self.reload_name;

        self.lump_cache.cache_lump_num(lump, tag, |l| {
            Self::read_lump_raw(lump_info, file_handles, reload_name, l)
        })
    }

    /// Returns cached lump data by name, reading from disk on cache miss.
    ///
    /// Equivalent of `W_CacheLumpName` at `w_wad.c:507-513`. Resolves the
    /// name to an index via [`check_num_for_name`](WadFile::check_num_for_name)
    /// and delegates to [`cache_lump_num`](WadFile::cache_lump_num).
    ///
    /// # Panics
    ///
    /// Panics if the lump name is not found, matching the original
    /// `I_Error` behavior.
    pub fn cache_lump_name(&mut self, name: &str, tag: PurgeTag) -> &[u8] {
        let lump = self
            .check_num_for_name(name)
            .unwrap_or_else(|| panic!("W_CacheLumpName: {} not found!", name));
        self.cache_lump_num(lump, tag)
    }

    // =========================================================================
    // Internal helper — raw lump reading for split-borrow pattern
    // =========================================================================

    /// Internal lump reading function that works with split struct borrows.
    ///
    /// Takes individual references to the relevant struct fields instead of
    /// `&self`, allowing it to be called from within a closure that already
    /// holds `&mut self.lump_cache`. This avoids borrow checker conflicts.
    ///
    /// Replaces `W_ReadLump` at `w_wad.c:431-467` with the same logic:
    /// - For lumps with a valid file handle (`Some(idx)`): seek to position
    ///   and read the full lump.
    /// - For reload lumps (handle = `None`): open the reload file, seek,
    ///   read, and let the file close when dropped.
    fn read_lump_raw(
        lump_info: &[LumpInfo],
        file_handles: &[Option<File>],
        reload_name: &Option<String>,
        lump: usize,
    ) -> Vec<u8> {
        // w_wad.c:440-441: Bounds check
        if lump >= lump_info.len() {
            error!("W_ReadLump: {} >= numlumps ({})", lump, lump_info.len());
            panic!("W_ReadLump: {} >= numlumps ({})", lump, lump_info.len());
        }

        let info = &lump_info[lump];

        // Guard against malformed WAD entries with negative size values.
        // A negative i32 cast to usize wraps to a huge value (~18 EB on 64-bit),
        // causing an OOM panic.  The original C code never performed this check
        // because negative sizes do not appear in legitimate WAD files, but we
        // add it here for robustness against user-provided PWADs.
        if info.size < 0 {
            error!(
                "W_ReadLump: lump {} has negative size ({})",
                lump, info.size
            );
            panic!(
                "W_ReadLump: lump {} has negative size ({})",
                lump, info.size
            );
        }

        let size = info.size as usize;
        let position = info.position as u64;
        let mut buf = vec![0u8; size];

        // Short-circuit for zero-length lumps (e.g., marker lumps)
        if size == 0 {
            return buf;
        }

        match info.handle {
            Some(handle_idx) => {
                // Normal file — use stored handle (w_wad.c:454-457)
                // Replaces: lseek(l->handle, l->position, SEEK_SET);
                //           c = read(l->handle, dest, l->size);
                match &file_handles[handle_idx] {
                    Some(file) => {
                        // Use &File for Read/Seek (impl Read for &File and
                        // impl Seek for &File are available in std).
                        let mut reader: &File = file;
                        reader.seek(SeekFrom::Start(position)).unwrap_or_else(|e| {
                            panic!("W_ReadLump: seek failed on lump {} ({})", lump, e)
                        });
                        reader.read_exact(&mut buf).unwrap_or_else(|e| {
                            panic!(
                                "W_ReadLump: only read partial data on lump {} ({})",
                                lump, e
                            )
                        });
                    }
                    None => {
                        // Handle index points to a None entry — programming error
                        panic!(
                            "W_ReadLump: file handle {} is not open for lump {}",
                            handle_idx, lump
                        );
                    }
                }
            }
            None => {
                // Reload file — open, read, close (w_wad.c:447-452, 463-464)
                // Replaces: handle = open(reloadname, O_RDONLY | O_BINARY);
                //           lseek(handle, l->position, SEEK_SET);
                //           c = read(handle, dest, l->size);
                //           close(handle);
                let name = reload_name.as_ref().unwrap_or_else(|| {
                    panic!("W_ReadLump: lump {} has no handle and no reload name", lump)
                });
                let file = File::open(name).unwrap_or_else(|e| {
                    panic!("W_ReadLump: couldn't open reload file {} ({})", name, e)
                });
                let mut reader = BufReader::new(file);
                reader.seek(SeekFrom::Start(position)).unwrap_or_else(|e| {
                    panic!("W_ReadLump: seek failed on reload lump {} ({})", lump, e)
                });
                reader.read_exact(&mut buf).unwrap_or_else(|e| {
                    panic!(
                        "W_ReadLump: only read partial data on reload lump {} ({})",
                        lump, e
                    )
                });
                // file is dropped here — equivalent of close(handle)
            }
        }

        buf
    }
}

// =============================================================================
// Default implementation
// =============================================================================

impl Default for WadFile {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// WadProvider trait implementation
// =============================================================================

/// [`WadProvider`] trait implementation for [`WadFile`].
///
/// Delegates all trait methods to the corresponding inherent methods on
/// `WadFile`. This allows both direct usage (via `WadFile` methods) and
/// trait-object usage (via `dyn WadProvider`).
///
/// The trait methods with `&self` (non-caching) and `&mut self` (caching)
/// match the original `w_wad.c` API: lookup and read operations are
/// side-effect-free, while caching operations modify the internal cache.
impl WadProvider for WadFile {
    /// See [`WadFile::check_num_for_name`].
    fn check_num_for_name(&self, name: &str) -> Option<usize> {
        WadFile::check_num_for_name(self, name)
    }

    /// See [`WadFile::get_num_for_name`].
    fn get_num_for_name(&self, name: &str) -> Result<usize, WadError> {
        WadFile::get_num_for_name(self, name)
    }

    /// See [`WadFile::lump_length`].
    fn lump_length(&self, lump: usize) -> usize {
        WadFile::lump_length(self, lump)
    }

    /// See [`WadFile::num_lumps`].
    fn num_lumps(&self) -> usize {
        WadFile::num_lumps(self)
    }

    /// See [`WadFile::read_lump`].
    fn read_lump(&self, lump: usize) -> Vec<u8> {
        WadFile::read_lump(self, lump)
    }

    /// See [`WadFile::cache_lump_num`].
    fn cache_lump_num(&mut self, lump: usize, tag: PurgeTag) -> &[u8] {
        WadFile::cache_lump_num(self, lump, tag)
    }

    /// See [`WadFile::cache_lump_name`].
    fn cache_lump_name(&mut self, name: &str, tag: PurgeTag) -> &[u8] {
        WadFile::cache_lump_name(self, name, tag)
    }
}
