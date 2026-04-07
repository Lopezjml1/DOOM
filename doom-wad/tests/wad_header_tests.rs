//! Integration tests for WAD file header parsing and identification.
//! Validates behavior derived from linuxdoom-1.10/w_wad.c W_AddFile (lines 141-226)
//! and the wadinfo_t structure from linuxdoom-1.10/w_wad.h (lines 35-42).
//!
//! The WAD binary format stores a 12-byte header:
//!   - `identification` (4 bytes): ASCII "IWAD" or "PWAD"
//!   - `numlumps`       (4 bytes): little-endian i32 — number of lumps
//!   - `infotableofs`   (4 bytes): little-endian i32 — file offset to the directory
//!
//! Each directory entry (filelump_t, w_wad.h:45-51) is 16 bytes:
//!   - `filepos` (4 bytes): i32 LE — file offset to lump data
//!   - `size`    (4 bytes): i32 LE — byte length of lump data
//!   - `name`    (8 bytes): null-padded, uppercase ASCII lump name

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use doom_wad::{WadError, WadFile, WadInfo, WadProvider, WadType};

// =============================================================================
// Constants — WAD binary format (from w_wad.h)
// =============================================================================

/// WAD header size in bytes: 4 (identification) + 4 (numlumps) + 4 (infotableofs).
/// Derived from wadinfo_t at w_wad.h:35-42.
const WAD_HEADER_SIZE: i32 = 12;

/// Directory entry size in bytes: 4 (filepos) + 4 (size) + 8 (name).
/// Derived from filelump_t at w_wad.h:45-51.
const DIR_ENTRY_SIZE: usize = 16;

// =============================================================================
// Test Fixture Helpers
// =============================================================================

/// Global counter to ensure unique temp file names across parallel test runs.
static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// RAII wrapper for temporary WAD test files on disk.
///
/// Creates a file on construction and removes it on drop, ensuring test
/// cleanup even if a test panics (unwinding). Uses an atomic counter and
/// process ID to generate unique filenames for parallel test safety.
struct TestWadFile {
    path: PathBuf,
}

impl TestWadFile {
    /// Creates a new temporary test file with `.wad` extension.
    ///
    /// The file is placed in the system temp directory with a unique name
    /// derived from a global atomic counter and the provided `label` for
    /// human-readable identification.
    ///
    /// The `.wad` extension is required so that `WadFile::add_file` treats
    /// it as a WAD file (parsing the identification header) rather than a
    /// single-lump file. See w_wad.c line ~168:
    /// `if (strcmpi(filename+strlen(filename)-3, "wad"))`.
    fn new(label: &str, data: &[u8]) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let filename = format!("doom_hdr_test_{}_{}.wad", label, id);
        let path = std::env::temp_dir().join(filename);
        std::fs::write(&path, data).expect("Failed to write test WAD file");
        Self { path }
    }

    /// Returns the file path as a `&str` for passing to `WadFile` methods.
    fn path_str(&self) -> &str {
        self.path.to_str().expect("Test path is not valid UTF-8")
    }
}

impl Drop for TestWadFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Construct a minimal WAD file header (12 bytes) with the given fields.
///
/// This builds ONLY the header — no lump data or directory entries.
/// All integer fields are written in little-endian format to match the
/// WAD on-disk format. See w_wad.c:196-197 where the `LONG()` macro
/// performs endian conversion on `numlumps` and `infotableofs`.
///
/// # Parameters
///
/// - `identification`: 4-byte magic string (e.g., `b"IWAD"` or `b"PWAD"`)
/// - `numlumps`: Number of lumps (stored as little-endian i32)
/// - `infotableofs`: File offset to directory (stored as little-endian i32)
fn make_wad_bytes(identification: &[u8; 4], numlumps: i32, infotableofs: i32) -> Vec<u8> {
    let mut data = Vec::with_capacity(WAD_HEADER_SIZE as usize);
    data.extend_from_slice(identification);
    data.extend_from_slice(&numlumps.to_le_bytes());
    data.extend_from_slice(&infotableofs.to_le_bytes());
    data
}

/// Construct a complete, valid WAD file with header, lump data, and directory.
///
/// The binary layout follows the original WAD format:
///
/// ```text
/// [Header:    12 bytes]
/// [Lump data: variable, concatenated sequentially]
/// [Directory: 16 bytes × numlumps]
/// ```
///
/// Each directory entry (filelump_t from w_wad.h:45-51) contains:
/// - `filepos` (i32 LE): offset to lump data in the file
/// - `size`    (i32 LE): size of lump data in bytes
/// - `name`    (8 bytes): lump name, null-padded, uppercased
///
/// This matches the on-disk layout that `W_AddFile` (w_wad.c:141-226)
/// parses when loading WAD files.
///
/// # Parameters
///
/// - `identification`: 4-byte magic (`b"IWAD"` or `b"PWAD"`)
/// - `lumps`: Slice of `(name, data)` tuples defining each lump
fn make_complete_wad(identification: &[u8; 4], lumps: &[(&str, &[u8])]) -> Vec<u8> {
    let num_lumps = lumps.len() as i32;

    // Calculate total lump data size to determine directory offset.
    let total_data_size: i32 = lumps.iter().map(|(_, d)| d.len() as i32).sum();
    let infotableofs = WAD_HEADER_SIZE + total_data_size;

    // Pre-allocate buffer: header + data + directory.
    let total_size =
        (WAD_HEADER_SIZE + total_data_size) as usize + (num_lumps as usize * DIR_ENTRY_SIZE);
    let mut data = Vec::with_capacity(total_size);

    // --- Write WAD header (12 bytes) ---
    data.extend_from_slice(identification);
    data.extend_from_slice(&num_lumps.to_le_bytes());
    data.extend_from_slice(&infotableofs.to_le_bytes());

    // --- Write lump data (concatenated after header) ---
    for (_, lump_data) in lumps {
        data.extend_from_slice(lump_data);
    }

    // --- Write directory entries (16 bytes each, after all lump data) ---
    let mut current_offset = WAD_HEADER_SIZE;
    for (name, lump_data) in lumps {
        // filepos (i32 LE): offset to this lump's data in the file.
        data.extend_from_slice(&current_offset.to_le_bytes());
        // size (i32 LE): byte length of this lump's data.
        data.extend_from_slice(&(lump_data.len() as i32).to_le_bytes());
        // name (8 bytes, null-padded, uppercase).
        // Matches w_wad.c:221: strncpy(lump_p->name, fileinfo->name, 8)
        // and w_wad.c:112: toupper() applied during ExtractFileBase.
        let mut name_bytes = [0u8; 8];
        for (i, ch) in name.bytes().take(8).enumerate() {
            name_bytes[i] = ch.to_ascii_uppercase();
        }
        data.extend_from_slice(&name_bytes);

        current_offset += lump_data.len() as i32;
    }

    data
}

// =============================================================================
// IWAD / PWAD Identification Tests
//
// These tests validate the 4-byte magic identification parsing from
// w_wad.c lines 185-195:
//   if (strncmp(header.identification,"IWAD",4))
//   {
//       if (strncmp(header.identification,"PWAD",4))
//       {
//           I_Error("Wad file %s doesn't have IWAD or PWAD id\n", filename);
//       }
//   }
// =============================================================================

/// Validates that a WAD file with "IWAD" 4-byte magic loads successfully.
///
/// References: w_wad.h:37 `char identification[4]; // Should be "IWAD" or "PWAD"`
///             w_wad.c:185 `strncmp(header.identification, "IWAD", 4)`
#[test]
fn test_valid_iwad_identification() {
    let wad_data = make_complete_wad(b"IWAD", &[("DUMMY", &[0u8; 4])]);
    let test_file = TestWadFile::new("iwad_ident", &wad_data);

    // Load via init_multiple_files — should succeed without error.
    let result = WadFile::init_multiple_files(&[test_file.path_str()]);
    assert!(
        result.is_ok(),
        "IWAD file should load successfully: {:?}",
        result.err()
    );

    let wad = result.unwrap();
    assert_eq!(wad.num_lumps(), 1, "IWAD should contain 1 lump");
}

/// Validates that a WAD file with "PWAD" 4-byte magic loads successfully.
///
/// References: w_wad.h:37-38, w_wad.c:188 `strncmp(header.identification, "PWAD", 4)`
#[test]
fn test_valid_pwad_identification() {
    let wad_data = make_complete_wad(b"PWAD", &[("PATCH", &[0u8; 8])]);
    let test_file = TestWadFile::new("pwad_ident", &wad_data);

    let result = WadFile::init_multiple_files(&[test_file.path_str()]);
    assert!(
        result.is_ok(),
        "PWAD file should load successfully: {:?}",
        result.err()
    );

    let wad = result.unwrap();
    assert_eq!(wad.num_lumps(), 1, "PWAD should contain 1 lump");
}

/// Validates that `WadType::Iwad` and `WadType::Pwad` are distinct enum
/// variants with correct equality and copy semantics.
#[test]
fn test_wad_type_enum_discrimination() {
    // WadType::Iwad and WadType::Pwad must be distinct.
    assert_ne!(
        WadType::Iwad,
        WadType::Pwad,
        "Iwad and Pwad must be distinct variants"
    );

    // Self-equality must hold.
    assert_eq!(WadType::Iwad, WadType::Iwad, "Iwad must equal itself");
    assert_eq!(WadType::Pwad, WadType::Pwad, "Pwad must equal itself");

    // Copy semantics (WadType derives Copy).
    let wad_type = WadType::Iwad;
    let copy = wad_type;
    assert_eq!(wad_type, copy, "WadType must support Copy");
}

// =============================================================================
// WAD Header Field Parsing Tests
//
// These tests validate correct parsing of wadinfo_t fields in
// little-endian format. See w_wad.c:196-197:
//   header.numlumps   = LONG(header.numlumps);
//   header.infotableofs = LONG(header.infotableofs);
// =============================================================================

/// Validates that the `numlumps` field is correctly parsed as a little-endian
/// i32, and the returned lump count matches.
///
/// References: w_wad.c:196 `header.numlumps = LONG(header.numlumps)` — the
///             LONG() macro performs little-endian to host-endian conversion.
#[test]
fn test_numlumps_little_endian() {
    // Create an IWAD with exactly 5 lumps.
    let lumps: Vec<(&str, &[u8])> = vec![
        ("LUMP0", &[1, 2]),
        ("LUMP1", &[3, 4]),
        ("LUMP2", &[5, 6]),
        ("LUMP3", &[7, 8]),
        ("LUMP4", &[9, 10]),
    ];
    let wad_data = make_complete_wad(b"IWAD", &lumps);

    // Verify the numlumps field in raw binary is 5 in little-endian.
    let raw_numlumps = i32::from_le_bytes([wad_data[4], wad_data[5], wad_data[6], wad_data[7]]);
    assert_eq!(raw_numlumps, 5, "Raw numlumps LE bytes should decode to 5");

    let test_file = TestWadFile::new("numlumps_le", &wad_data);
    let wad = WadFile::init_multiple_files(&[test_file.path_str()]).expect("WAD should load");
    assert_eq!(wad.num_lumps(), 5, "numlumps should be parsed as 5");
}

/// Validates that `infotableofs` is correctly parsed and the directory is
/// read from the correct file offset.
///
/// References: w_wad.c:197 `header.infotableofs = LONG(header.infotableofs)`
///             w_wad.c:200 `lseek(handle, header.infotableofs, SEEK_SET)`
#[test]
fn test_infotableofs_little_endian() {
    // Create a WAD with specific lump data so infotableofs is at a known offset.
    // Header = 12 bytes, two lumps with 10 bytes each = 20 bytes of data.
    // infotableofs = 12 + 20 = 32
    let lump_data_a = [0xAAu8; 10];
    let lump_data_b = [0xBBu8; 10];
    let lumps: Vec<(&str, &[u8])> = vec![("ALPHA", &lump_data_a), ("BETA", &lump_data_b)];
    let wad_data = make_complete_wad(b"IWAD", &lumps);

    // Verify the infotableofs value in the raw binary is 32 (LE).
    let expected_infotableofs: i32 = 12 + 20;
    let stored_infotableofs =
        i32::from_le_bytes([wad_data[8], wad_data[9], wad_data[10], wad_data[11]]);
    assert_eq!(
        stored_infotableofs, expected_infotableofs,
        "Raw infotableofs LE bytes should decode to {}",
        expected_infotableofs
    );

    // Load the WAD and verify lumps are correctly read from the directory
    // positioned at the infotableofs offset.
    let test_file = TestWadFile::new("infotableofs_le", &wad_data);
    let wad = WadFile::init_multiple_files(&[test_file.path_str()]).expect("WAD should load");

    assert_eq!(wad.num_lumps(), 2, "Should have 2 lumps");

    // Verify lump data was read from the correct file positions.
    let data_a = wad.read_lump(0);
    assert_eq!(data_a, lump_data_a, "First lump data should match");
    let data_b = wad.read_lump(1);
    assert_eq!(data_b, lump_data_b, "Second lump data should match");
}

/// Validates that a WAD file with zero lumps can be loaded via `add_file`
/// without error. A zero-lump WAD is technically valid per the WAD format —
/// it has an empty directory.
///
/// Note: `init_multiple_files` would reject a zero-lump WAD with
/// `WadError::NoFilesFound` because the post-load check requires at least
/// one lump (w_wad.c:305-306). Using `add_file` directly bypasses that check.
///
/// References: w_wad.c:202 `numlumps += header.numlumps` — adding 0.
#[test]
fn test_zero_lumps_wad() {
    // WAD header with numlumps=0, infotableofs=12 (directory at end of header).
    let wad_data = make_wad_bytes(b"IWAD", 0, WAD_HEADER_SIZE);
    let test_file = TestWadFile::new("zero_lumps", &wad_data);

    // Use add_file directly (init_multiple_files would return NoFilesFound).
    let mut wad = WadFile::new();
    let result = wad.add_file(test_file.path_str());
    assert!(
        result.is_ok(),
        "Zero-lump WAD should load via add_file: {:?}",
        result.err()
    );
    assert_eq!(wad.num_lumps(), 0, "Zero-lump WAD should have 0 lumps");
}

/// Validates that `WadInfo` struct fields match the `wadinfo_t` layout from
/// w_wad.h:35-42.
///
/// ```c
/// typedef struct {
///     char identification[4]; // "IWAD" or "PWAD"
///     int  numlumps;
///     int  infotableofs;
/// } wadinfo_t;
/// ```
#[test]
fn test_wadinfo_struct_fields() {
    // Verify WadInfo can be constructed with correct field types and values.
    let info = WadInfo {
        identification: *b"IWAD",
        numlumps: 42,
        infotableofs: 12,
    };

    assert_eq!(
        &info.identification, b"IWAD",
        "identification should be IWAD bytes"
    );
    assert_eq!(info.numlumps, 42, "numlumps should be 42");
    assert_eq!(info.infotableofs, 12, "infotableofs should be 12");

    // Verify PWAD identification works too.
    let pwad_info = WadInfo {
        identification: *b"PWAD",
        numlumps: 0,
        infotableofs: 12,
    };
    assert_eq!(
        &pwad_info.identification, b"PWAD",
        "identification should be PWAD bytes"
    );
    assert_eq!(pwad_info.numlumps, 0, "numlumps should be 0");
    assert_eq!(pwad_info.infotableofs, 12, "infotableofs should be 12");
}

/// Validates that `WadInfo` supports `Clone` and `Debug` traits.
#[test]
fn test_wadinfo_clone_and_debug() {
    let info = WadInfo {
        identification: *b"IWAD",
        numlumps: 10,
        infotableofs: 100,
    };

    // Clone semantics.
    let cloned = info.clone();
    assert_eq!(cloned.identification, info.identification);
    assert_eq!(cloned.numlumps, info.numlumps);
    assert_eq!(cloned.infotableofs, info.infotableofs);

    // Debug formatting should produce non-empty output and not panic.
    let debug_str = format!("{:?}", info);
    assert!(
        !debug_str.is_empty(),
        "Debug format should produce non-empty output"
    );
}

// =============================================================================
// Invalid WAD Rejection Tests
//
// These tests validate error handling for malformed WAD files.
// See w_wad.c:188-191:
//   if (strncmp(header.identification,"PWAD",4))
//   {
//       I_Error("Wad file %s doesn't have IWAD or PWAD id\n", filename);
//   }
// =============================================================================

/// Validates that a file with unrecognized 4-byte magic is rejected with
/// `WadError::InvalidWad`.
///
/// References: w_wad.c:188-191 — files that are neither IWAD nor PWAD
///             trigger `I_Error`.
#[test]
fn test_invalid_magic_rejected() {
    // "XWAD" is neither "IWAD" nor "PWAD".
    let wad_data = make_wad_bytes(b"XWAD", 0, WAD_HEADER_SIZE);
    let test_file = TestWadFile::new("invalid_magic", &wad_data);

    let mut wad = WadFile::new();
    let result = wad.add_file(test_file.path_str());
    assert!(
        matches!(result, Err(WadError::InvalidWad(_))),
        "XWAD magic should produce InvalidWad error, got: {:?}",
        result
    );
}

/// Validates that a truncated WAD file (8 bytes — missing `infotableofs`)
/// is rejected with an I/O error.
///
/// The WAD header requires 12 bytes. A file with only 8 bytes will fail
/// during the `read_i32` call for `infotableofs`, producing a
/// `WadError::Io` wrapping `UnexpectedEof`.
#[test]
fn test_truncated_header_rejected() {
    // Only 8 bytes: valid "IWAD" magic + numlumps, but missing infotableofs.
    let mut data = Vec::new();
    data.extend_from_slice(b"IWAD");
    data.extend_from_slice(&5i32.to_le_bytes());
    assert_eq!(data.len(), 8, "Truncated header should be 8 bytes");

    let test_file = TestWadFile::new("truncated_hdr", &data);

    let mut wad = WadFile::new();
    let result = wad.add_file(test_file.path_str());
    assert!(result.is_err(), "Truncated header should produce an error");
    // The read for infotableofs fails with io::ErrorKind::UnexpectedEof.
    assert!(
        matches!(result, Err(WadError::Io(_))),
        "Truncated header should produce Io error, got: {:?}",
        result
    );
}

/// Validates that a zero-byte (empty) WAD file is rejected.
///
/// An empty file cannot contain even the 4-byte identification field,
/// so reading the header will fail immediately with an I/O error.
#[test]
fn test_empty_file_rejected() {
    let test_file = TestWadFile::new("empty_wad", &[]);

    let mut wad = WadFile::new();
    let result = wad.add_file(test_file.path_str());
    assert!(result.is_err(), "Empty file should produce an error");
    // Empty file fails on the first read_exact for the 4-byte identification.
    assert!(
        matches!(result, Err(WadError::Io(_))),
        "Empty file should produce Io error, got: {:?}",
        result
    );
}

/// Validates that null bytes as identification are rejected with
/// `WadError::InvalidWad`.
///
/// A 12-byte file with `[0, 0, 0, 0]` as identification is neither
/// "IWAD" nor "PWAD", so it must be rejected.
#[test]
fn test_null_identification_rejected() {
    let wad_data = make_wad_bytes(&[0, 0, 0, 0], 0, WAD_HEADER_SIZE);
    let test_file = TestWadFile::new("null_ident", &wad_data);

    let mut wad = WadFile::new();
    let result = wad.add_file(test_file.path_str());
    assert!(
        matches!(result, Err(WadError::InvalidWad(_))),
        "Null identification should produce InvalidWad error, got: {:?}",
        result
    );
}

/// Validates that random garbage bytes in the identification field are rejected.
#[test]
fn test_garbage_identification_rejected() {
    let wad_data = make_wad_bytes(&[0xFF, 0xFE, 0xFD, 0xFC], 1, WAD_HEADER_SIZE);
    let test_file = TestWadFile::new("garbage_ident", &wad_data);

    let mut wad = WadFile::new();
    let result = wad.add_file(test_file.path_str());
    assert!(
        matches!(result, Err(WadError::InvalidWad(_))),
        "Garbage identification should produce InvalidWad error, got: {:?}",
        result
    );
}

/// Validates that "DWAD" (visually similar but incorrect) magic is rejected.
#[test]
fn test_similar_but_wrong_magic_rejected() {
    let wad_data = make_wad_bytes(b"DWAD", 0, WAD_HEADER_SIZE);
    let test_file = TestWadFile::new("dwad_magic", &wad_data);

    let mut wad = WadFile::new();
    let result = wad.add_file(test_file.path_str());
    assert!(
        matches!(result, Err(WadError::InvalidWad(_))),
        "DWAD magic should produce InvalidWad error, got: {:?}",
        result
    );
}

// =============================================================================
// Multi-WAD Loading Tests
//
// These tests validate W_InitMultipleFiles behavior (w_wad.c:292-316),
// which iterates filenames and calls W_AddFile for each.
// =============================================================================

/// Validates that multiple WAD files can be loaded sequentially and their
/// lump counts are combined.
///
/// References: W_InitMultipleFiles at w_wad.c:292-316 — iterates filenames
///             and calls W_AddFile for each, accumulating into a unified
///             lump directory.
#[test]
fn test_multiple_wad_files() {
    // Create an IWAD with 3 lumps.
    let iwad_data = make_complete_wad(
        b"IWAD",
        &[
            ("PLAYPAL", &[0u8; 768]),
            ("COLORMAP", &[0u8; 256]),
            ("DEMO1", &[0u8; 32]),
        ],
    );
    let iwad_file = TestWadFile::new("multi_iwad", &iwad_data);

    // Create a PWAD with 2 lumps.
    let pwad_data = make_complete_wad(b"PWAD", &[("MAP01", &[0u8; 16]), ("THINGS", &[0u8; 64])]);
    let pwad_file = TestWadFile::new("multi_pwad", &pwad_data);

    // Load both files via init_multiple_files.
    let wad = WadFile::init_multiple_files(&[iwad_file.path_str(), pwad_file.path_str()])
        .expect("Multi-WAD loading should succeed");

    // Total lumps should be 3 + 2 = 5.
    assert_eq!(
        wad.num_lumps(),
        5,
        "Total lump count should be 3 (IWAD) + 2 (PWAD) = 5"
    );
}

/// Validates that `init_multiple_files` with an empty list returns
/// `WadError::NoFilesFound`.
///
/// References: w_wad.c:305-306:
/// ```c
/// if (!numlumps)
///     I_Error("W_InitFiles: no files found");
/// ```
#[test]
fn test_no_files_found_error() {
    let result = WadFile::init_multiple_files(&[]);
    assert!(result.is_err(), "Empty file list should produce an error");
    // Use .err() to extract the error variant. We avoid .unwrap_err()
    // because WadFile does not implement Debug (required by unwrap_err).
    let err = result.err().expect("already checked is_err");
    assert!(
        matches!(err, WadError::NoFilesFound),
        "Empty file list should produce NoFilesFound error, got: {}",
        err
    );
}

/// Validates that `add_file` correctly accumulates lumps across multiple
/// sequential calls on the same `WadFile`.
///
/// References: w_wad.c:170 `startlump = numlumps`
///             w_wad.c:202 `numlumps += header.numlumps`
#[test]
fn test_add_file_accumulates_lumps() {
    let wad1_data = make_complete_wad(b"IWAD", &[("FIRST", &[1, 2, 3])]);
    let wad2_data = make_complete_wad(b"PWAD", &[("SECOND", &[4, 5]), ("THIRD", &[6])]);
    let file1 = TestWadFile::new("accum1", &wad1_data);
    let file2 = TestWadFile::new("accum2", &wad2_data);

    let mut wad = WadFile::new();
    wad.add_file(file1.path_str())
        .expect("First WAD should load");
    assert_eq!(wad.num_lumps(), 1, "After first WAD: 1 lump");

    wad.add_file(file2.path_str())
        .expect("Second WAD should load");
    assert_eq!(wad.num_lumps(), 3, "After second WAD: 1 + 2 = 3 lumps");
}

/// Validates that lumps from loaded WAD files can be looked up by name
/// using `check_num_for_name`.
///
/// References: W_CheckNumForName at w_wad.c:351-390 — backward scan
///             through the lump directory to find a matching 8-byte name.
#[test]
fn test_loaded_lumps_searchable_by_name() {
    let wad_data = make_complete_wad(
        b"IWAD",
        &[
            ("PLAYPAL", &[0u8; 768]),
            ("COLORMAP", &[0u8; 256]),
            ("ENDOOM", &[0u8; 80]),
        ],
    );
    let test_file = TestWadFile::new("lump_search", &wad_data);

    let wad = WadFile::init_multiple_files(&[test_file.path_str()]).expect("WAD should load");

    // All three lumps should be findable by name.
    assert!(
        wad.check_num_for_name("PLAYPAL").is_some(),
        "PLAYPAL should be found"
    );
    assert!(
        wad.check_num_for_name("COLORMAP").is_some(),
        "COLORMAP should be found"
    );
    assert!(
        wad.check_num_for_name("ENDOOM").is_some(),
        "ENDOOM should be found"
    );

    // Non-existent lump should return None.
    assert!(
        wad.check_num_for_name("NOEXIST").is_none(),
        "Non-existent lump should not be found"
    );
}

/// Validates that lump data read back from a loaded WAD matches the
/// original written payload.
///
/// References: W_ReadLump at w_wad.c:431-467.
#[test]
fn test_lump_data_integrity() {
    let payload = [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE];
    let wad_data = make_complete_wad(b"IWAD", &[("DATA", &payload)]);
    let test_file = TestWadFile::new("data_integrity", &wad_data);

    let wad = WadFile::init_multiple_files(&[test_file.path_str()]).expect("WAD should load");

    let data = wad.read_lump(0);
    assert_eq!(data, payload, "Lump data should match written payload");
    assert_eq!(
        wad.lump_length(0),
        payload.len(),
        "Lump length should match payload size"
    );
}

/// Validates that the `WadProvider` trait is correctly implemented for
/// `WadFile` by exercising trait methods through a trait object reference.
///
/// This ensures the `WadProvider` trait's public API contract is functional
/// for downstream consumers that depend on trait-based abstractions rather
/// than the concrete `WadFile` type.
#[test]
fn test_wad_provider_trait_impl() {
    let wad_data = make_complete_wad(b"IWAD", &[("TESTLMP", &[42u8; 16])]);
    let test_file = TestWadFile::new("trait_impl", &wad_data);

    let wad = WadFile::init_multiple_files(&[test_file.path_str()]).expect("WAD should load");

    // Exercise WadProvider trait methods through a trait object reference.
    let provider: &dyn WadProvider = &wad;
    assert_eq!(
        provider.num_lumps(),
        1,
        "WadProvider::num_lumps should return 1"
    );
    assert!(
        provider.check_num_for_name("TESTLMP").is_some(),
        "WadProvider::check_num_for_name should find TESTLMP"
    );
    assert!(
        provider.check_num_for_name("MISSING").is_none(),
        "WadProvider::check_num_for_name should return None for missing lump"
    );

    // Verify lump_length through trait.
    assert_eq!(
        provider.lump_length(0),
        16,
        "WadProvider::lump_length should return 16"
    );
}

/// Validates that a WAD with many lumps (100) is handled correctly,
/// exercising directory parsing at scale.
#[test]
fn test_many_lumps() {
    // Create 100 lumps with unique names and data.
    let lump_data: Vec<(String, Vec<u8>)> = (0..100)
        .map(|i| {
            let name = format!("L{:05}", i); // "L00000".."L00099" — 6 chars, fits in 8
            let data = vec![(i & 0xFF) as u8; 4];
            (name, data)
        })
        .collect();

    let lumps: Vec<(&str, &[u8])> = lump_data
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();

    let wad_bytes = make_complete_wad(b"IWAD", &lumps);
    let test_file = TestWadFile::new("many_lumps", &wad_bytes);

    let wad = WadFile::init_multiple_files(&[test_file.path_str()]).expect("WAD should load");

    assert_eq!(wad.num_lumps(), 100, "Should have 100 lumps");

    // Spot-check a few lumps by name.
    assert!(wad.check_num_for_name("L00000").is_some(), "First lump");
    assert!(wad.check_num_for_name("L00050").is_some(), "Middle lump");
    assert!(wad.check_num_for_name("L00099").is_some(), "Last lump");
    assert!(
        wad.check_num_for_name("L00100").is_none(),
        "Out-of-range should be None"
    );
}
