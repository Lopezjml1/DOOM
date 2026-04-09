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

//! Translated from linuxdoom-1.10/doomtype.h
//!
//! Simple basic typedefs, isolated here to make it easier separating modules.
//! Most of these have direct Rust equivalents (bool, u8, i32::MAX, etc.)
//! but are preserved here for source mapping clarity.

// Note: C's `typedef enum {false, true} boolean;` maps directly to Rust's `bool`.
// No type alias needed — use `bool` everywhere.

/// Unsigned byte type. Equivalent to C: `typedef unsigned char byte;`
pub type Byte = u8;

// ---------------------------------------------------------------------------
// Max/Min value constants
// ---------------------------------------------------------------------------
// These match the C `#define` constants from the non-LINUX path in doomtype.h
// (lines 44-55). On Linux, these were provided by <values.h>; in Rust, the
// standard library provides them as associated constants on primitive types.
// We preserve named constants here for source-mapping clarity and because
// MAXINT / MININT are used directly in FixedDiv overflow checking
// (m_fixed.c line 63: `return (a^b)<0 ? MININT : MAXINT;`).
// ---------------------------------------------------------------------------

/// Maximum value for a signed 8-bit integer.
/// C equivalent: `#define MAXCHAR ((char)0x7f)`
pub const MAXCHAR: i8 = i8::MAX; // 0x7f = 127

/// Maximum value for a signed 16-bit integer.
/// C equivalent: `#define MAXSHORT ((short)0x7fff)`
pub const MAXSHORT: i16 = i16::MAX; // 0x7fff = 32767

/// Maximum value for a signed 32-bit integer.
/// C equivalent: `#define MAXINT ((int)0x7fffffff)`
///
/// CRITICAL: Used in `FixedDiv` overflow checking — value must be exactly `0x7fffffff`.
pub const MAXINT: i32 = i32::MAX; // 0x7fffffff = 2_147_483_647

/// Maximum value for a signed 32-bit integer (long alias).
/// C equivalent: `#define MAXLONG ((long)0x7fffffff)`
///
/// In the original C code, `long` was 32-bit on the target platform.
pub const MAXLONG: i32 = i32::MAX; // 0x7fffffff = 2_147_483_647

/// Minimum value for a signed 8-bit integer.
/// C equivalent: `#define MINCHAR ((char)0x80)`
pub const MINCHAR: i8 = i8::MIN; // 0x80 as signed = -128

/// Minimum value for a signed 16-bit integer.
/// C equivalent: `#define MINSHORT ((short)0x8000)`
pub const MINSHORT: i16 = i16::MIN; // 0x8000 as signed = -32768

/// Minimum value for a signed 32-bit integer.
/// C equivalent: `#define MININT ((int)0x80000000)`
///
/// CRITICAL: Used in `FixedDiv` overflow checking — value must be exactly `0x80000000`
/// interpreted as a signed 32-bit integer (`-2_147_483_648`).
pub const MININT: i32 = i32::MIN; // 0x80000000 as signed = -2_147_483_648

/// Minimum value for a signed 32-bit integer (long alias).
/// C equivalent: `#define MINLONG ((long)0x80000000)`
///
/// In the original C code, `long` was 32-bit on the target platform.
pub const MINLONG: i32 = i32::MIN; // 0x80000000 as signed = -2_147_483_648
