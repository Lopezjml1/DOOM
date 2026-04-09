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

//! Integration tests for WAD lump name lookup and directory management.
//! Validates behavior derived from linuxdoom-1.10/w_wad.c functions:
//! W_CheckNumForName (lines 351-390), W_GetNumForName (lines 399-409),
//! W_AddFile (lines 141-226), W_InitMultipleFiles (lines 292-316).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use doom_wad::{WadError, WadFile, WadProvider};

// =============================================================================
// Test Helpers
// =============================================================================

/// Global counter to ensure unique temp file names across parallel test runs.
static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Construct a complete WAD file binary with named lumps.
///
/// Each lump entry is `(name, data_bytes)`.
/// Lump names are padded/truncated to 8 bytes and uppercased, matching
/// the original w_wad.c:221 behavior: `strncpy(lump_p->name, fileinfo->name, 8)`.
///
/// # WAD Binary Layout
///
/// ```text
/// Offset 0:      Header (12 bytes)
///   [0..4]       identification: "IWAD" or "PWAD"
///   [4..8]       numlumps:      i32 LE
///   [8..12]      infotableofs:  i32 LE
/// Offset 12:     Lump data (contiguous, variable length)
/// Offset N:      Directory (16 bytes per entry)
///   [0..4]       filepos: i32 LE
///   [4..8]       size:    i32 LE
///   [8..16]      name:    8 bytes, null-padded, uppercase
/// ```
fn make_wad_with_lumps(is_iwad: bool, lumps: &[(&str, &[u8])]) -> Vec<u8> {
    let identification: &[u8; 4] = if is_iwad { b"IWAD" } else { b"PWAD" };
    let num_lumps = lumps.len() as i32;

    // Calculate total lump data size to determine where the directory starts.
    let total_data_size: i32 = lumps.iter().map(|(_, d)| d.len() as i32).sum();
    let infotableofs = 12 + total_data_size;

    let mut data = Vec::new();

    // --- Write WAD header (12 bytes) ---
    data.extend_from_slice(identification);
    data.extend_from_slice(&num_lumps.to_le_bytes());
    data.extend_from_slice(&infotableofs.to_le_bytes());

    // --- Write lump data sequentially after header ---
    for (_, lump_data) in lumps {
        data.extend_from_slice(lump_data);
    }

    // --- Write directory entries (16 bytes each) ---
    let mut current_offset = 12i32;
    for (name, lump_data) in lumps {
        // filepos: where this lump's data begins in the file (i32 LE)
        data.extend_from_slice(&current_offset.to_le_bytes());
        // size: byte length of lump data (i32 LE)
        data.extend_from_slice(&(lump_data.len() as i32).to_le_bytes());

        // name: 8 bytes, null-padded, uppercase — w_wad.c:112 toupper
        let mut name_bytes = [0u8; 8];
        for (i, ch) in name.bytes().take(8).enumerate() {
            name_bytes[i] = ch.to_ascii_uppercase();
        }
        data.extend_from_slice(&name_bytes);

        current_offset += lump_data.len() as i32;
    }

    data
}

/// RAII helper struct for temporary WAD files on disk.
///
/// Creates a uniquely-named temp file in the system temp directory on
/// construction and removes it on drop, ensuring test cleanup even on
/// panic (unwinding).
struct TestWadFile {
    path: PathBuf,
}

impl TestWadFile {
    /// Write `data` to a new temp file with a unique name incorporating `label`.
    fn new(label: &str, data: &[u8]) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let filename = format!("doom_lump_test_{}_{}.wad", label, id);
        let path = std::env::temp_dir().join(filename);
        std::fs::write(&path, data).expect("Failed to write test WAD file");
        Self { path }
    }

    /// Write `data` to a temp file with a custom extension (for non-WAD tests).
    ///
    /// IMPORTANT: For single-lump file tests, the filename stem must be ≤ 8
    /// characters because `ExtractFileBase` (w_wad.c:85-114) extracts up to 8
    /// uppercase characters from the filename. The `label` parameter IS the
    /// stem — keep it at most 8 chars.
    fn with_extension(label: &str, ext: &str, data: &[u8]) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        // Use label directly as the stem to keep it ≤ 8 chars.
        // Append numeric ID as a subfolder component to avoid collisions
        // without lengthening the filename stem.
        let dir = std::env::temp_dir().join(format!("dlump{}", id));
        let _ = std::fs::create_dir_all(&dir);
        let filename = format!("{}.{}", label, ext);
        let path = dir.join(filename);
        std::fs::write(&path, data).expect("Failed to write test file");
        Self { path }
    }

    /// Returns the file path as a `&str`.
    fn path_str(&self) -> &str {
        self.path.to_str().expect("Test path is not valid UTF-8")
    }
}

impl Drop for TestWadFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        // Also try to remove the parent directory if it was a temp subfolder
        // created by with_extension(). Fails silently if it's the system
        // temp dir or if the directory is not empty.
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Helper to create a `WadFile` from a single in-memory WAD binary.
fn wad_from_bytes(is_iwad: bool, lumps: &[(&str, &[u8])], label: &str) -> (WadFile, TestWadFile) {
    let data = make_wad_with_lumps(is_iwad, lumps);
    let tmp = TestWadFile::new(label, &data);
    let wad = WadFile::init_multiple_files(&[tmp.path_str()]).expect("Failed to load test WAD");
    (wad, tmp)
}

// =============================================================================
// Phase 2: Case-Insensitive Lump Name Matching Tests
// =============================================================================
// Validates the 8-byte case-insensitive name comparison from w_wad.c:353-386:
//
//   strncpy(name8.s, name, 8);
//   name8.s[8] = 0;
//   strupr(name8.s);      // case insensitive
//   v1 = name8.x[0];
//   v2 = name8.x[1];
//   ...compare as two ints...

/// Exact uppercase match: "PLAYPAL" stored → "PLAYPAL" queried.
#[test]
fn test_exact_case_lump_lookup() {
    let (wad, _tmp) = wad_from_bytes(true, &[("PLAYPAL", &[0u8; 16])], "exact_case");
    let result = wad.check_num_for_name("PLAYPAL");
    assert_eq!(result, Some(0), "Exact uppercase name should match");
}

/// Lowercase query matches uppercase stored name.
/// Validates: w_wad.c:370 `strupr(name8.s)` — query names are uppercased.
#[test]
fn test_case_insensitive_lump_lookup_lowercase() {
    let (wad, _tmp) = wad_from_bytes(true, &[("PLAYPAL", &[0u8; 16])], "lower_case");
    let result = wad.check_num_for_name("playpal");
    assert_eq!(
        result,
        Some(0),
        "Lowercase query should match uppercase lump"
    );
}

/// Mixed case query matches uppercase stored name.
#[test]
fn test_case_insensitive_lump_lookup_mixed() {
    let (wad, _tmp) = wad_from_bytes(true, &[("PLAYPAL", &[0u8; 16])], "mixed_case");
    let result = wad.check_num_for_name("PlayPal");
    assert_eq!(
        result,
        Some(0),
        "Mixed case query should match uppercase lump"
    );
}

/// 8-character name exact match — tests full 8-byte comparison path.
/// Validates: Lump names are 8 bytes, null-padded (w_wad.h:49: `char name[8]`).
#[test]
fn test_eight_char_name_match() {
    let (wad, _tmp) = wad_from_bytes(true, &[("TEXTURE1", &[0u8; 32])], "eight_char");
    let result = wad.check_num_for_name("TEXTURE1");
    assert_eq!(result, Some(0), "8-character name should match exactly");
}

/// Short name (4 chars) padded with null bytes in WAD still matches query.
#[test]
fn test_short_name_null_padded() {
    let (wad, _tmp) = wad_from_bytes(true, &[("E1M1", &[0u8; 8])], "short_name");
    let result = wad.check_num_for_name("E1M1");
    assert_eq!(
        result,
        Some(0),
        "Short name should match with null-padded storage"
    );
}

// =============================================================================
// Phase 3: W_CheckNumForName Equivalent Tests
// =============================================================================
// Validates the `check_num_for_name` method (W_CheckNumForName, w_wad.c:351-390).

/// check_num_for_name returns None when the lump name does not exist.
/// Validates: w_wad.c:389 `return -1;` → Rust returns `None`.
#[test]
fn test_check_num_for_name_not_found() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[("PLAYPAL", &[0u8; 16]), ("COLORMAP", &[0u8; 32])],
        "not_found",
    );
    let result = wad.check_num_for_name("MISSING");
    assert_eq!(result, None, "Missing lump should return None");
}

/// check_num_for_name returns the correct index for each found lump.
#[test]
fn test_check_num_for_name_found() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[
            ("PLAYPAL", &[0u8; 16]),
            ("COLORMAP", &[0u8; 32]),
            ("DEMO1", &[0u8; 8]),
        ],
        "found",
    );
    assert_eq!(wad.check_num_for_name("PLAYPAL"), Some(0));
    assert_eq!(wad.check_num_for_name("COLORMAP"), Some(1));
    assert_eq!(wad.check_num_for_name("DEMO1"), Some(2));
}

/// check_num_for_name with empty name returns None.
#[test]
fn test_check_num_for_name_empty() {
    let (wad, _tmp) = wad_from_bytes(true, &[("PLAYPAL", &[0u8; 16])], "empty_name");
    let result = wad.check_num_for_name("");
    assert_eq!(result, None, "Empty name should not match any lump");
}

// =============================================================================
// Phase 4: W_GetNumForName Equivalent Tests
// =============================================================================
// Validates the `get_num_for_name` method (W_GetNumForName, w_wad.c:399-409).

/// get_num_for_name returns Ok(index) for a found lump.
#[test]
fn test_get_num_for_name_found() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[("PLAYPAL", &[0u8; 16]), ("COLORMAP", &[0u8; 32])],
        "get_found",
    );
    let result = wad.get_num_for_name("PLAYPAL");
    assert!(result.is_ok(), "Found lump should return Ok");
    assert_eq!(result.unwrap(), 0);
}

/// get_num_for_name returns Err(WadError::LumpNotFound) for a missing lump.
/// Validates: w_wad.c:406 `I_Error("W_GetNumForName: %s not found!", name)`
/// → Rust returns `Err(WadError::LumpNotFound(name))`.
#[test]
fn test_get_num_for_name_not_found_error() {
    let (wad, _tmp) = wad_from_bytes(true, &[("PLAYPAL", &[0u8; 16])], "get_not_found");
    let result = wad.get_num_for_name("NONEXIST");
    assert!(result.is_err(), "Missing lump should return Err");
    match result {
        Err(WadError::LumpNotFound(name)) => {
            assert_eq!(name, "NONEXIST", "Error should contain the queried name");
        }
        other => panic!("Expected WadError::LumpNotFound, got {:?}", other),
    }
}

// =============================================================================
// Phase 5: Backward Scan Semantics Tests
// =============================================================================
// Validates the CRITICAL backward scan from w_wad.c:377-386:
//
//   // scan backwards so patch lump files take precedence
//   lump_p = lumpinfo + numlumps;
//   while (lump_p-- != lumpinfo) { ... }
//
// This is the mechanism by which PWAD files override IWAD lumps.
// w_wad.c:289-290: "The name searcher looks backwards, so a later file
// does override all earlier ones."

/// Later WAD overrides earlier WAD for a lump with the same name.
/// The IWAD's "PLAYPAL" is at a lower index; the PWAD's "PLAYPAL" is at a
/// higher index. Backward scan should return the higher (PWAD) index.
#[test]
fn test_backward_scan_later_overrides_earlier() {
    // IWAD: PLAYPAL (data A), COLORMAP
    let iwad_data =
        make_wad_with_lumps(true, &[("PLAYPAL", &[0xAA; 16]), ("COLORMAP", &[0xBB; 32])]);
    let iwad_tmp = TestWadFile::new("bscan_iwad", &iwad_data);

    // PWAD: PLAYPAL (data B) — this should override the IWAD's PLAYPAL
    let pwad_data = make_wad_with_lumps(false, &[("PLAYPAL", &[0xCC; 16])]);
    let pwad_tmp = TestWadFile::new("bscan_pwad", &pwad_data);

    // Load IWAD first, then PWAD — mimics standard DOOM loading order
    let wad = WadFile::init_multiple_files(&[iwad_tmp.path_str(), pwad_tmp.path_str()])
        .expect("Failed to load test WADs");

    // Total lumps: 2 (from IWAD) + 1 (from PWAD) = 3
    assert_eq!(wad.num_lumps(), 3, "Total lump count should be 3");

    // Backward scan: PLAYPAL from PWAD (index 2) should be found,
    // not PLAYPAL from IWAD (index 0).
    let idx = wad.check_num_for_name("PLAYPAL");
    assert_eq!(idx, Some(2), "PWAD's PLAYPAL should override IWAD's");

    // Verify the data is from the PWAD (0xCC), not the IWAD (0xAA)
    let data = wad.read_lump(idx.unwrap());
    assert_eq!(data[0], 0xCC, "Data should come from the PWAD override");
}

/// Duplicate lump names within a single WAD — backward scan returns the last.
#[test]
fn test_backward_scan_within_single_wad() {
    // Single WAD with two lumps named "DEMO1":
    //   index 0: DEMO1 (data A)
    //   index 1: DEMO1 (data B)
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[("DEMO1", &[0xAA; 8]), ("DEMO1", &[0xBB; 8])],
        "bscan_dup",
    );

    // Backward scan should find the later occurrence (index 1)
    let idx = wad.check_num_for_name("DEMO1");
    assert_eq!(
        idx,
        Some(1),
        "Backward scan should return the last occurrence"
    );

    // Verify data is from the second entry (0xBB)
    let data = wad.read_lump(idx.unwrap());
    assert_eq!(data[0], 0xBB, "Data should be from the later duplicate");
}

/// Non-overridden lumps remain accessible even when another lump is overridden.
#[test]
fn test_non_overridden_lumps_accessible() {
    // IWAD: PLAYPAL, COLORMAP, DEMO1
    let iwad_data = make_wad_with_lumps(
        true,
        &[
            ("PLAYPAL", &[0xAA; 16]),
            ("COLORMAP", &[0xBB; 32]),
            ("DEMO1", &[0xCC; 8]),
        ],
    );
    let iwad_tmp = TestWadFile::new("nonovr_iwad", &iwad_data);

    // PWAD: only overrides PLAYPAL
    let pwad_data = make_wad_with_lumps(false, &[("PLAYPAL", &[0xDD; 16])]);
    let pwad_tmp = TestWadFile::new("nonovr_pwad", &pwad_data);

    let wad = WadFile::init_multiple_files(&[iwad_tmp.path_str(), pwad_tmp.path_str()])
        .expect("Failed to load test WADs");

    // COLORMAP and DEMO1 should still be accessible at their IWAD indices
    let colormap_idx = wad.check_num_for_name("COLORMAP");
    assert_eq!(
        colormap_idx,
        Some(1),
        "COLORMAP should remain at IWAD index 1"
    );

    let demo1_idx = wad.check_num_for_name("DEMO1");
    assert_eq!(demo1_idx, Some(2), "DEMO1 should remain at IWAD index 2");

    // PLAYPAL should be overridden to PWAD index 3
    let playpal_idx = wad.check_num_for_name("PLAYPAL");
    assert_eq!(
        playpal_idx,
        Some(3),
        "PLAYPAL should be overridden to PWAD index"
    );
}

// =============================================================================
// Phase 6: Lump Length and Data Access Tests
// =============================================================================

/// lump_length returns the correct size for each lump.
/// Validates: w_wad.c:416-422 W_LumpLength.
#[test]
fn test_lump_length() {
    let test_data = vec![0xDEu8; 100];
    let (wad, _tmp) = wad_from_bytes(true, &[("TEST", &test_data)], "lump_len");
    let idx = wad
        .check_num_for_name("TEST")
        .expect("TEST lump should exist");
    assert_eq!(wad.lump_length(idx), 100, "Lump length should be 100 bytes");
}

/// read_lump returns the correct data for a lump.
/// Validates: w_wad.c:431-467 W_ReadLump.
#[test]
fn test_read_lump_data() {
    let expected = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let (wad, _tmp) = wad_from_bytes(true, &[("DATA", &expected)], "read_data");
    let idx = wad
        .check_num_for_name("DATA")
        .expect("DATA lump should exist");
    let actual = wad.read_lump(idx);
    assert_eq!(actual, expected, "read_lump should return exact lump bytes");
}

/// num_lumps returns the total count across all loaded files.
/// Validates: w_wad.c:339-342 W_NumLumps.
#[test]
fn test_num_lumps_count() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[
            ("L1", &[0u8; 4]),
            ("L2", &[0u8; 4]),
            ("L3", &[0u8; 4]),
            ("L4", &[0u8; 4]),
            ("L5", &[0u8; 4]),
        ],
        "num_lumps",
    );
    assert_eq!(wad.num_lumps(), 5, "num_lumps should return 5");
}

/// read_lump with multiple lumps returns the correct data for each.
#[test]
fn test_read_lump_multiple() {
    let data_a = vec![0x01, 0x02, 0x03];
    let data_b = vec![0xAA, 0xBB, 0xCC, 0xDD];
    let data_c = vec![0xFF];
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[("LUMPA", &data_a), ("LUMPB", &data_b), ("LUMPC", &data_c)],
        "read_multi",
    );

    assert_eq!(wad.read_lump(0), data_a);
    assert_eq!(wad.read_lump(1), data_b);
    assert_eq!(wad.read_lump(2), data_c);
}

/// Zero-length lumps (marker lumps) are handled correctly.
#[test]
fn test_zero_length_lump() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[
            ("S_START", &[]), // marker lump, 0 bytes
            ("SOMEDATA", &[0xAB; 16]),
            ("S_END", &[]), // marker lump, 0 bytes
        ],
        "zero_len",
    );
    let idx = wad
        .check_num_for_name("S_START")
        .expect("S_START should exist");
    assert_eq!(
        wad.lump_length(idx),
        0,
        "Marker lump should have zero length"
    );
    let data = wad.read_lump(idx);
    assert!(data.is_empty(), "Marker lump data should be empty");
}

// =============================================================================
// Phase 7: MAXWADFILES Limit Awareness Test
// =============================================================================
// MAXWADFILES = 20 is defined in d_main.h and enforced at the application
// level (d_main.c), NOT by w_wad.c. The doom-wad crate itself does not
// enforce this limit. This test documents that contract awareness.

/// Verify that the doom-wad crate can load multiple WAD files without an
/// artificial limit. The MAXWADFILES=20 constraint is enforced by doom-core
/// (d_main.c), not by the WAD loading layer.
#[test]
fn test_maxwadfiles_not_enforced_by_wad_crate() {
    // Create 5 small WAD files and load them all — the WAD crate should
    // accept any number of files without complaint.
    let mut temps = Vec::new();
    let mut paths = Vec::new();

    for i in 0..5 {
        let name = format!("LMP{:02}", i);
        let data = make_wad_with_lumps(true, &[(&name, &[i as u8; 4])]);
        let tmp = TestWadFile::new(&format!("maxwad_{}", i), &data);
        paths.push(tmp.path_str().to_string());
        temps.push(tmp);
    }

    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let wad = WadFile::init_multiple_files(&path_refs).expect("Should load multiple WAD files");

    // Each WAD has 1 lump → 5 lumps total
    assert_eq!(wad.num_lumps(), 5, "Should have 5 lumps from 5 files");
}

// =============================================================================
// Phase 8: Single-Lump File Support Tests
// =============================================================================
// Validates the non-.wad file handling from w_wad.c:172-179:
//
//   if (strcmpi(filename + strlen(filename) - 3, "wad"))
//   {
//       // single lump file
//       fileinfo = &singleinfo;
//       singleinfo.filepos = 0;
//       singleinfo.size = LONG(filelength(handle));
//       ExtractFileBase(filename, singleinfo.name);
//       numlumps++;
//   }

/// Non-.wad files are treated as single-lump files.
/// The lump name is extracted from the filename base, uppercased.
/// Validates: w_wad.c:172-179 and ExtractFileBase (lines 85-114).
#[test]
fn test_single_lump_file() {
    let file_content = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x42];
    // Stem must be ≤ 8 chars for ExtractFileBase (w_wad.c:85-114)
    let tmp = TestWadFile::with_extension("TDATA", "lmp", &file_content);

    let mut wad = WadFile::new();
    wad.add_file(tmp.path_str())
        .expect("Should load single-lump file");

    // Verify lump count increased by 1
    assert_eq!(
        wad.num_lumps(),
        1,
        "Single-lump file should add exactly 1 lump"
    );

    // Verify lump size matches file size
    assert_eq!(
        wad.lump_length(0),
        file_content.len(),
        "Lump size should equal file size"
    );

    // Verify lump data matches file content
    let data = wad.read_lump(0);
    assert_eq!(data, file_content, "Lump data should match file content");
}

/// Single-lump file: the lump name is derived from the filename stem.
/// Validates: w_wad.c:178 `ExtractFileBase(filename, singleinfo.name)`.
#[test]
fn test_single_lump_name_from_filename() {
    let file_content = vec![0x01, 0x02, 0x03];
    // Stem "MYDATA" is 6 chars ≤ 8 — ExtractFileBase uppercases it
    let tmp = TestWadFile::with_extension("MYDATA", "lmp", &file_content);

    let mut wad = WadFile::new();
    wad.add_file(tmp.path_str())
        .expect("Should load single-lump file");

    // The lump name should be "MYDATA" (uppercase of filename stem).
    assert_eq!(wad.num_lumps(), 1);
    assert_eq!(wad.lump_length(0), 3);

    // Look up by the name derived from the filename stem
    let idx = wad.check_num_for_name("MYDATA");
    assert_eq!(
        idx,
        Some(0),
        "Lump name should match uppercase filename stem"
    );
}

// =============================================================================
// Phase 9: Edge Cases
// =============================================================================

/// Lump with all 8 characters used (no null padding in the name field).
/// Validates: w_wad.c:366-367 `strncpy(name8.s, name, 8); name8.s[8] = 0;`
#[test]
fn test_full_eight_char_name() {
    let (wad, _tmp) = wad_from_bytes(true, &[("ABCDEFGH", &[0u8; 4])], "full8");
    let result = wad.check_num_for_name("ABCDEFGH");
    assert_eq!(result, Some(0), "Full 8-char name should match");
}

/// Full 8-char name with case-insensitive lookup.
#[test]
fn test_full_eight_char_case_insensitive() {
    let (wad, _tmp) = wad_from_bytes(true, &[("ABCDEFGH", &[0u8; 4])], "full8_ci");
    let result = wad.check_num_for_name("abcdefgh");
    assert_eq!(
        result,
        Some(0),
        "Case-insensitive match on full 8-char name"
    );
}

/// Multiple lumps with similar name prefixes are correctly distinguished.
/// Validates correct 8-byte comparison, not prefix matching.
#[test]
fn test_similar_prefix_names() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[
            ("MAP01", &[0x01; 4]),
            ("MAP02", &[0x02; 4]),
            ("MAP03", &[0x03; 4]),
        ],
        "similar_prefix",
    );
    assert_eq!(wad.check_num_for_name("MAP01"), Some(0));
    assert_eq!(wad.check_num_for_name("MAP02"), Some(1));
    assert_eq!(wad.check_num_for_name("MAP03"), Some(2));

    // Ensure no false matches
    assert_eq!(wad.check_num_for_name("MAP04"), None);
    assert_eq!(wad.check_num_for_name("MAP0"), None);
}

/// Lump index out of bounds triggers a panic.
/// Validates: w_wad.c:418-419 bounds check:
/// `if (lump >= numlumps) I_Error("W_LumpLength: %i >= numlumps", lump);`
#[test]
#[should_panic(expected = "W_LumpLength")]
fn test_lump_index_out_of_bounds() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[("L1", &[0u8; 4]), ("L2", &[0u8; 4]), ("L3", &[0u8; 4])],
        "oob",
    );
    // Index 999 is way out of bounds for a 3-lump WAD
    let _ = wad.lump_length(999);
}

/// read_lump with out-of-bounds index triggers a panic.
/// Validates: w_wad.c:440-441 `if (lump >= numlumps) I_Error(...)`.
#[test]
#[should_panic(expected = "W_ReadLump")]
fn test_read_lump_out_of_bounds() {
    let (wad, _tmp) = wad_from_bytes(true, &[("ONLY", &[0u8; 4])], "read_oob");
    let _ = wad.read_lump(999);
}

/// Lumps with only 1-character name work correctly.
#[test]
fn test_single_char_name() {
    let (wad, _tmp) = wad_from_bytes(true, &[("A", &[0x42; 2])], "single_char");
    assert_eq!(wad.check_num_for_name("A"), Some(0));
    assert_eq!(wad.check_num_for_name("a"), Some(0)); // case insensitive
    assert_eq!(wad.lump_length(0), 2);
}

/// WadProvider trait methods work on WadFile via trait dispatch.
/// Ensures the trait implementation delegates correctly.
#[test]
fn test_wad_provider_trait_dispatch() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[("PLAYPAL", &[0u8; 768]), ("COLORMAP", &[0u8; 256])],
        "trait_dispatch",
    );

    // Use trait methods via the trait object interface
    let provider: &dyn WadProvider = &wad;
    assert_eq!(provider.num_lumps(), 2);
    assert_eq!(provider.check_num_for_name("PLAYPAL"), Some(0));
    assert_eq!(provider.check_num_for_name("COLORMAP"), Some(1));
    assert_eq!(provider.lump_length(0), 768);
    assert_eq!(provider.lump_length(1), 256);

    let result = provider.get_num_for_name("MISSING");
    assert!(result.is_err());
}

/// get_num_for_name returns correct index via WadProvider trait.
#[test]
fn test_wad_provider_get_num_for_name() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[("DEMO1", &[0u8; 64]), ("DEMO2", &[0u8; 64])],
        "trait_get",
    );
    let provider: &dyn WadProvider = &wad;
    assert_eq!(provider.get_num_for_name("DEMO1").unwrap(), 0);
    assert_eq!(provider.get_num_for_name("DEMO2").unwrap(), 1);
}

/// Multiple add_file calls accumulate lumps in the directory.
#[test]
fn test_add_file_accumulates_lumps() {
    let wad1_data = make_wad_with_lumps(true, &[("LUMPA", &[0x01; 4])]);
    let wad2_data = make_wad_with_lumps(false, &[("LUMPB", &[0x02; 4])]);

    let tmp1 = TestWadFile::new("accum1", &wad1_data);
    let tmp2 = TestWadFile::new("accum2", &wad2_data);

    let mut wad = WadFile::new();
    wad.add_file(tmp1.path_str())
        .expect("First file should load");
    assert_eq!(wad.num_lumps(), 1);

    wad.add_file(tmp2.path_str())
        .expect("Second file should load");
    assert_eq!(wad.num_lumps(), 2);

    assert_eq!(wad.check_num_for_name("LUMPA"), Some(0));
    assert_eq!(wad.check_num_for_name("LUMPB"), Some(1));
}

/// Large lump data (8 KiB) is read correctly.
#[test]
fn test_large_lump_data() {
    let large_data: Vec<u8> = (0..8192).map(|i| (i % 256) as u8).collect();
    let (wad, _tmp) = wad_from_bytes(true, &[("BIGDATA", &large_data)], "large_data");
    let idx = wad.check_num_for_name("BIGDATA").unwrap();
    assert_eq!(wad.lump_length(idx), 8192);
    let read_data = wad.read_lump(idx);
    assert_eq!(
        read_data, large_data,
        "Large lump data should be read exactly"
    );
}

/// Numeric characters in lump names work correctly.
#[test]
fn test_numeric_lump_names() {
    let (wad, _tmp) = wad_from_bytes(
        true,
        &[
            ("12345678", &[0u8; 4]),
            ("E1M1", &[0u8; 4]),
            ("MAP01", &[0u8; 4]),
        ],
        "numeric_names",
    );
    assert_eq!(wad.check_num_for_name("12345678"), Some(0));
    assert_eq!(wad.check_num_for_name("E1M1"), Some(1));
    assert_eq!(wad.check_num_for_name("MAP01"), Some(2));
}
