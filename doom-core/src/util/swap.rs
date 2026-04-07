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

//! Translated from linuxdoom-1.10/m_swap.c and linuxdoom-1.10/m_swap.h
//!
//! Endian byte-swap utilities. WAD files are stored in little-endian format.
//!
//! On little-endian systems (x86 Windows — our target), the [`short`] and
//! [`long`] conversion functions are identity operations. The explicit swap
//! functions ([`swap_short`], [`swap_long`]) are provided for completeness
//! and potential future cross-platform use.
//!
//! # Original C Design
//!
//! The original C code used preprocessor macros to select behavior at
//! compile time:
//!
//! ```c
//! // m_swap.h — on little-endian (default):
//! #define SHORT(x)  (x)
//! #define LONG(x)   (x)
//!
//! // m_swap.h — on big-endian (__BIG_ENDIAN__):
//! #define SHORT(x)  ((short)SwapSHORT((unsigned short)(x)))
//! #define LONG(x)   ((long)SwapLONG((unsigned long)(x)))
//! ```
//!
//! # Rust Approach
//!
//! We use [`i16::from_le`] and [`i32::from_le`] which compile to no-ops on
//! little-endian targets and automatically perform the byte swap on
//! big-endian targets. This is strictly superior to the C `#ifdef` approach
//! because the same code is correct on all architectures without conditional
//! compilation.

/// Convert a 16-bit value from WAD format (little-endian) to native endian.
///
/// On x86_64 Windows (little-endian), this is an identity operation.
///
/// Equivalent to C: `#define SHORT(x) (x)` on little-endian,
/// or `SwapSHORT(x)` on big-endian.
///
/// # Examples
///
/// ```
/// use doom_core::util::swap::short;
/// assert_eq!(short(0x0102_i16), 0x0102_i16); // identity on little-endian
/// ```
#[inline]
pub fn short(x: i16) -> i16 {
    // WAD files are little-endian. On little-endian targets, this is identity.
    // On big-endian targets, this would swap bytes automatically.
    i16::from_le(x)
}

/// Convert a 32-bit value from WAD format (little-endian) to native endian.
///
/// On x86_64 Windows (little-endian), this is an identity operation.
///
/// Equivalent to C: `#define LONG(x) (x)` on little-endian,
/// or `SwapLONG(x)` on big-endian.
///
/// # Examples
///
/// ```
/// use doom_core::util::swap::long;
/// assert_eq!(long(0x01020304_i32), 0x01020304_i32); // identity on little-endian
/// ```
#[inline]
pub fn long(x: i32) -> i32 {
    // WAD files are little-endian. On little-endian targets, this is identity.
    // On big-endian targets, this would swap bytes automatically.
    i32::from_le(x)
}

/// Byte-swap a 16-bit value (MSB ↔ LSB).
///
/// Unconditionally swaps the two bytes regardless of platform endianness.
///
/// Equivalent to C: `unsigned short SwapSHORT(unsigned short x)`
/// ```c
/// return (x>>8) | (x<<8);
/// ```
///
/// # Examples
///
/// ```
/// use doom_core::util::swap::swap_short;
/// assert_eq!(swap_short(0x0102_i16), 0x0201_i16);
/// ```
#[inline]
pub fn swap_short(x: i16) -> i16 {
    x.swap_bytes()
}

/// Byte-swap a 32-bit value.
///
/// Unconditionally reverses the byte order regardless of platform endianness.
///
/// Equivalent to C: `unsigned long SwapLONG(unsigned long x)`
/// ```c
/// return (x>>24) | ((x>>8) & 0xff00) | ((x<<8) & 0xff0000) | (x<<24);
/// ```
///
/// # Examples
///
/// ```
/// use doom_core::util::swap::swap_long;
/// assert_eq!(swap_long(0x01020304_i32), 0x04030201_i32);
/// ```
#[inline]
pub fn swap_long(x: i32) -> i32 {
    x.swap_bytes()
}

/// Convert an unsigned 16-bit value from WAD format (little-endian) to native
/// endian.
///
/// This is the unsigned counterpart to [`short`]. On little-endian systems,
/// this is an identity operation.
///
/// # Examples
///
/// ```
/// use doom_core::util::swap::short_unsigned;
/// assert_eq!(short_unsigned(0x0102_u16), 0x0102_u16);
/// ```
#[inline]
pub fn short_unsigned(x: u16) -> u16 {
    u16::from_le(x)
}

/// Convert an unsigned 32-bit value from WAD format (little-endian) to native
/// endian.
///
/// This is the unsigned counterpart to [`long`]. On little-endian systems,
/// this is an identity operation.
///
/// # Examples
///
/// ```
/// use doom_core::util::swap::long_unsigned;
/// assert_eq!(long_unsigned(0x01020304_u32), 0x01020304_u32);
/// ```
#[inline]
pub fn long_unsigned(x: u32) -> u32 {
    u32::from_le(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // short() — WAD little-endian to native (identity on x86_64)
    // -----------------------------------------------------------------------

    #[test]
    fn test_short_identity() {
        // On little-endian, short(x) == x for all values.
        assert_eq!(short(0x0102_i16), 0x0102_i16);
    }

    #[test]
    fn test_short_zero() {
        assert_eq!(short(0_i16), 0_i16);
    }

    #[test]
    fn test_short_negative() {
        assert_eq!(short(-1_i16), -1_i16);
    }

    #[test]
    fn test_short_min_max() {
        assert_eq!(short(i16::MIN), i16::MIN);
        assert_eq!(short(i16::MAX), i16::MAX);
    }

    // -----------------------------------------------------------------------
    // long() — WAD little-endian to native (identity on x86_64)
    // -----------------------------------------------------------------------

    #[test]
    fn test_long_identity() {
        // On little-endian, long(x) == x for all values.
        assert_eq!(long(0x01020304_i32), 0x01020304_i32);
    }

    #[test]
    fn test_long_zero() {
        assert_eq!(long(0_i32), 0_i32);
    }

    #[test]
    fn test_long_negative() {
        assert_eq!(long(-1_i32), -1_i32);
    }

    #[test]
    fn test_long_min_max() {
        assert_eq!(long(i32::MIN), i32::MIN);
        assert_eq!(long(i32::MAX), i32::MAX);
    }

    // -----------------------------------------------------------------------
    // swap_short() — unconditional byte swap
    // -----------------------------------------------------------------------

    #[test]
    fn test_swap_short_basic() {
        assert_eq!(swap_short(0x0102_i16), 0x0201_i16);
    }

    #[test]
    fn test_swap_short_zero() {
        assert_eq!(swap_short(0_i16), 0_i16);
    }

    #[test]
    fn test_swap_short_roundtrip() {
        // Swapping twice returns the original value.
        let original = 0x1234_i16;
        assert_eq!(swap_short(swap_short(original)), original);
    }

    #[test]
    fn test_swap_short_ff00() {
        assert_eq!(swap_short(0x00FF_i16), -256_i16); // 0xFF00 as i16
    }

    // -----------------------------------------------------------------------
    // swap_long() — unconditional byte swap
    // -----------------------------------------------------------------------

    #[test]
    fn test_swap_long_basic() {
        assert_eq!(swap_long(0x01020304_i32), 0x04030201_i32);
    }

    #[test]
    fn test_swap_long_zero() {
        assert_eq!(swap_long(0_i32), 0_i32);
    }

    #[test]
    fn test_swap_long_roundtrip() {
        // Swapping twice returns the original value.
        let original = 0x12345678_i32;
        assert_eq!(swap_long(swap_long(original)), original);
    }

    #[test]
    fn test_swap_long_matches_c_formula() {
        // Verify against the explicit C formula:
        // (x>>24) | ((x>>8)&0xff00) | ((x<<8)&0xff0000) | (x<<24)
        let x: u32 = 0xDEADBEEF;
        let c_result = (x >> 24) | ((x >> 8) & 0xff00) | ((x << 8) & 0xff_0000) | (x << 24);
        assert_eq!(swap_long(x as i32), c_result as i32);
    }

    // -----------------------------------------------------------------------
    // short_unsigned() — unsigned WAD little-endian to native
    // -----------------------------------------------------------------------

    #[test]
    fn test_short_unsigned_identity() {
        assert_eq!(short_unsigned(0x0102_u16), 0x0102_u16);
    }

    #[test]
    fn test_short_unsigned_zero() {
        assert_eq!(short_unsigned(0_u16), 0_u16);
    }

    #[test]
    fn test_short_unsigned_max() {
        assert_eq!(short_unsigned(u16::MAX), u16::MAX);
    }

    // -----------------------------------------------------------------------
    // long_unsigned() — unsigned WAD little-endian to native
    // -----------------------------------------------------------------------

    #[test]
    fn test_long_unsigned_identity() {
        assert_eq!(long_unsigned(0x01020304_u32), 0x01020304_u32);
    }

    #[test]
    fn test_long_unsigned_zero() {
        assert_eq!(long_unsigned(0_u32), 0_u32);
    }

    #[test]
    fn test_long_unsigned_max() {
        assert_eq!(long_unsigned(u32::MAX), u32::MAX);
    }

    // -----------------------------------------------------------------------
    // Cross-function consistency checks
    // -----------------------------------------------------------------------

    #[test]
    fn test_short_unsigned_matches_short_for_positive() {
        // For non-negative i16 values, short() and short_unsigned() should
        // produce the same bit pattern.
        let val: i16 = 0x1234;
        let signed_result = short(val);
        let unsigned_result = short_unsigned(val as u16);
        assert_eq!(signed_result as u16, unsigned_result);
    }

    #[test]
    fn test_long_unsigned_matches_long_for_positive() {
        // For non-negative i32 values, long() and long_unsigned() should
        // produce the same bit pattern.
        let val: i32 = 0x12345678;
        let signed_result = long(val);
        let unsigned_result = long_unsigned(val as u32);
        assert_eq!(signed_result as u32, unsigned_result);
    }

    #[test]
    #[allow(clippy::manual_rotate)]
    fn test_swap_short_matches_c_formula() {
        // Verify against the explicit C formula: (x>>8) | (x<<8)
        // Using unsigned arithmetic to match the C behavior exactly.
        // The allow is intentional — we want to express the C formula verbatim.
        let x: u16 = 0xABCD;
        let c_result = (x >> 8) | (x << 8);
        assert_eq!(swap_short(x as i16), c_result as i16);
    }
}
