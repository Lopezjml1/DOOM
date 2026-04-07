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

//! Translated from linuxdoom-1.10/m_fixed.h and linuxdoom-1.10/m_fixed.c
//!
//! Fixed point, 32bit as 16.16.
//! The `Fixed` newtype wraps `i32` with the lower 16 bits as the fractional part.
//! All arithmetic must produce bit-identical results to the original C
//! implementation for demo playback compatibility.
//!
//! # Fixed-Point Format
//!
//! The DOOM engine uses a 16.16 fixed-point number format where the upper 16 bits
//! represent the integer part and the lower 16 bits represent the fractional part.
//! This means `FRACUNIT` (65536) represents 1.0, `FRACUNIT * 2` represents 2.0,
//! and `FRACUNIT / 2` (32768) represents 0.5.
//!
//! # Determinism Mandate
//!
//! Every arithmetic operation in this module MUST produce bit-identical results
//! to the original C implementation. This is critical for demo playback
//! compatibility and behavioral parity. Specifically:
//!
//! - `FixedMul` uses 64-bit intermediate multiplication to prevent overflow
//! - `FixedDiv2` uses `f64` double-precision arithmetic (matching the active C code
//!   path; the `#if 0`'d integer shift path in `m_fixed.c` is NOT used)
//! - Wrapping arithmetic (`wrapping_add`, `wrapping_sub`, `wrapping_neg`) is used
//!   where the original C relies on signed integer overflow (which is undefined
//!   behavior in C but works on 2's complement hardware)

use std::fmt;
use std::ops::{Add, Neg, Sub};

// ---------------------------------------------------------------------------
// Constants (from m_fixed.h lines 35-36)
// ---------------------------------------------------------------------------

/// Number of fractional bits in the 16.16 fixed-point format.
/// Original C: `#define FRACBITS 16`
pub const FRACBITS: i32 = 16;

/// The value representing 1.0 in 16.16 fixed-point format (= 65536).
/// Original C: `#define FRACUNIT (1<<FRACBITS)`
pub const FRACUNIT: i32 = 1 << FRACBITS;

// ---------------------------------------------------------------------------
// Fixed Newtype
// ---------------------------------------------------------------------------

/// A 16.16 fixed-point number stored as an `i32`.
///
/// The upper 16 bits are the integer part and the lower 16 bits are the
/// fractional part. For example:
/// - `Fixed(65536)` = 1.0
/// - `Fixed(32768)` = 0.5
/// - `Fixed(131072)` = 2.0
/// - `Fixed(-65536)` = -1.0
///
/// This is the most fundamental numeric type in the DOOM engine. Virtually
/// every calculation — movement, collision, rendering, trigonometry — uses
/// this format.
///
/// Original C: `typedef int fixed_t;`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fixed(pub i32);

// ---------------------------------------------------------------------------
// Associated Constants and Constructors
// ---------------------------------------------------------------------------

impl Fixed {
    /// Zero value (0.0 in fixed-point).
    pub const ZERO: Fixed = Fixed(0);

    /// One (1.0 in fixed-point) = `FRACUNIT` = 65536.
    pub const ONE: Fixed = Fixed(FRACUNIT);

    /// Create a `Fixed` from a raw 16.16 value.
    ///
    /// The raw value is used directly — no shifting is performed.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::types::fixed::{Fixed, FRACUNIT};
    /// let one = Fixed::new(FRACUNIT); // 1.0
    /// assert_eq!(one, Fixed::ONE);
    /// ```
    #[inline]
    pub const fn new(raw: i32) -> Self {
        Fixed(raw)
    }

    /// Create a `Fixed` from a whole integer by shifting left by `FRACBITS`.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::types::fixed::Fixed;
    /// let two = Fixed::from_int(2);
    /// assert_eq!(two.0, 131072); // 2 << 16
    /// ```
    #[inline]
    pub const fn from_int(n: i32) -> Self {
        Fixed(n << FRACBITS)
    }

    /// Get the raw 16.16 `i32` value.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::types::fixed::Fixed;
    /// let val = Fixed::from_int(3);
    /// assert_eq!(val.raw(), 3 << 16);
    /// ```
    #[inline]
    pub const fn raw(self) -> i32 {
        self.0
    }

    // -----------------------------------------------------------------------
    // Core Arithmetic (from m_fixed.c)
    // -----------------------------------------------------------------------

    /// Fixed-point multiplication.
    ///
    /// Multiplies two 16.16 fixed-point values and returns a 16.16 result.
    /// Uses a 64-bit intermediate to prevent overflow.
    ///
    /// Original C (m_fixed.c lines 43-49):
    /// ```c
    /// fixed_t FixedMul(fixed_t a, fixed_t b) {
    ///     return ((long long) a * (long long) b) >> FRACBITS;
    /// }
    /// ```
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::types::fixed::Fixed;
    /// // 1.0 * 1.0 = 1.0
    /// assert_eq!(Fixed::new(65536).fixed_mul(Fixed::new(65536)), Fixed::new(65536));
    /// // 2.0 * 0.5 = 1.0
    /// assert_eq!(Fixed::new(131072).fixed_mul(Fixed::new(32768)), Fixed::new(65536));
    /// ```
    #[inline]
    pub fn fixed_mul(self, other: Fixed) -> Fixed {
        Fixed((((self.0 as i64) * (other.0 as i64)) >> FRACBITS) as i32)
    }

    /// Fixed-point division with overflow protection.
    ///
    /// If `|a| >> 14 >= |b|`, the result would overflow a 32-bit integer, so
    /// the function returns `i32::MIN` or `i32::MAX` depending on the sign of
    /// the operands. Otherwise, delegates to [`fixed_div2`](Self::fixed_div2).
    ///
    /// Original C (m_fixed.c lines 57-65):
    /// ```c
    /// fixed_t FixedDiv(fixed_t a, fixed_t b) {
    ///     if ((abs(a)>>14) >= abs(b))
    ///         return (a^b)<0 ? MININT : MAXINT;
    ///     return FixedDiv2(a,b);
    /// }
    /// ```
    ///
    /// Uses `wrapping_abs()` because `abs(i32::MIN)` is undefined behavior in C
    /// (and panics in Rust debug mode). On 2's complement hardware — which is all
    /// hardware DOOM has ever run on — `abs(MININT)` wraps to `MININT`, and
    /// `wrapping_abs()` provides exactly this behavior.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::types::fixed::Fixed;
    /// // 1.0 / 1.0 = 1.0
    /// assert_eq!(Fixed::new(65536).fixed_div(Fixed::new(65536)), Fixed::new(65536));
    /// // 2.0 / 1.0 = 2.0
    /// assert_eq!(Fixed::new(131072).fixed_div(Fixed::new(65536)), Fixed::new(131072));
    /// ```
    #[inline]
    pub fn fixed_div(self, other: Fixed) -> Fixed {
        if (self.0.wrapping_abs() >> 14) >= other.0.wrapping_abs() {
            // Overflow — return MININT or MAXINT based on sign
            if (self.0 ^ other.0) < 0 {
                Fixed(i32::MIN) // MININT = 0x80000000
            } else {
                Fixed(i32::MAX) // MAXINT = 0x7fffffff
            }
        } else {
            self.fixed_div2(other)
        }
    }

    /// Fixed-point division (direct double-precision calculation).
    ///
    /// Uses `f64` arithmetic to compute the division, matching the original C
    /// implementation exactly. The `#if 0`'d integer-shift version in `m_fixed.c`
    /// (lines 74-77) is NOT active — the double-precision path (lines 80-86) is
    /// the one that ships.
    ///
    /// Original C (m_fixed.c lines 69-87):
    /// ```c
    /// fixed_t FixedDiv2(fixed_t a, fixed_t b) {
    ///     double c;
    ///     c = ((double)a) / ((double)b) * FRACUNIT;
    ///     if (c >= 2147483648.0 || c < -2147483648.0)
    ///         I_Error("FixedDiv: divide by zero");
    ///     return (fixed_t) c;
    /// }
    /// ```
    ///
    /// In the original C, overflow calls `I_Error` (a terminal abort). In the Rust
    /// port, we return a saturated value instead, keeping the engine running.
    /// This deviation is acceptable because `FixedDiv` always guards against
    /// overflow before calling `FixedDiv2`, so this path should never be reached
    /// in practice.
    ///
    /// # Examples
    ///
    /// ```
    /// use doom_core::types::fixed::Fixed;
    /// // 1.0 / 2.0 = 0.5
    /// assert_eq!(Fixed::new(65536).fixed_div2(Fixed::new(131072)), Fixed::new(32768));
    /// ```
    #[inline]
    pub fn fixed_div2(self, other: Fixed) -> Fixed {
        let c: f64 = (self.0 as f64) / (other.0 as f64) * (FRACUNIT as f64);

        if !(-2147483648.0..2147483648.0).contains(&c) {
            // Overflow or divide-by-zero: saturate instead of aborting.
            // The original C calls I_Error("FixedDiv: divide by zero") here.
            if (self.0 ^ other.0) < 0 {
                Fixed(i32::MIN)
            } else {
                Fixed(i32::MAX)
            }
        } else {
            Fixed(c as i32)
        }
    }
}

// ---------------------------------------------------------------------------
// Operator Implementations
// ---------------------------------------------------------------------------
// Use wrapping arithmetic where C relies on signed integer overflow.
// The AAP §0.8.2 mandates: "use wrapping_mul, wrapping_add, etc. where needed"

impl Add for Fixed {
    type Output = Fixed;

    /// Fixed-point addition. Uses wrapping semantics to match C signed integer
    /// overflow behavior.
    #[inline]
    fn add(self, rhs: Fixed) -> Fixed {
        Fixed(self.0.wrapping_add(rhs.0))
    }
}

impl Sub for Fixed {
    type Output = Fixed;

    /// Fixed-point subtraction. Uses wrapping semantics to match C signed integer
    /// overflow behavior.
    #[inline]
    fn sub(self, rhs: Fixed) -> Fixed {
        Fixed(self.0.wrapping_sub(rhs.0))
    }
}

impl Neg for Fixed {
    type Output = Fixed;

    /// Fixed-point negation. Uses wrapping semantics because negating `i32::MIN`
    /// wraps to `i32::MIN` in C (undefined behavior, but consistent on 2's
    /// complement hardware).
    #[inline]
    fn neg(self) -> Fixed {
        Fixed(self.0.wrapping_neg())
    }
}

// ---------------------------------------------------------------------------
// Display Implementation
// ---------------------------------------------------------------------------

impl fmt::Display for Fixed {
    /// Displays the fixed-point value as a decimal number with 4 fractional digits.
    ///
    /// For example, `Fixed(65536)` displays as `"1.0000"`, and `Fixed(98304)`
    /// displays as `"1.5000"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 >> FRACBITS;
        let frac = (self.0 & (FRACUNIT - 1)) as f64 / FRACUNIT as f64;
        write!(f, "{:.4}", whole as f64 + frac)
    }
}

// ---------------------------------------------------------------------------
// From/Into Conversions
// ---------------------------------------------------------------------------

impl From<i32> for Fixed {
    /// Creates a `Fixed` from a raw `i32` value (no shifting).
    ///
    /// This converts the raw 16.16 representation directly. To convert a whole
    /// integer to fixed-point, use [`Fixed::from_int`] instead.
    #[inline]
    fn from(v: i32) -> Self {
        Fixed(v)
    }
}

impl From<Fixed> for i32 {
    /// Extracts the raw `i32` value from a `Fixed`.
    #[inline]
    fn from(f: Fixed) -> Self {
        f.0
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Constants --

    #[test]
    fn test_fracbits_value() {
        assert_eq!(FRACBITS, 16);
    }

    #[test]
    fn test_fracunit_value() {
        assert_eq!(FRACUNIT, 65536);
        assert_eq!(FRACUNIT, 1 << FRACBITS);
    }

    // -- Constructors --

    #[test]
    fn test_new() {
        let f = Fixed::new(12345);
        assert_eq!(f.0, 12345);
        assert_eq!(f.raw(), 12345);
    }

    #[test]
    fn test_from_int() {
        assert_eq!(Fixed::from_int(0), Fixed(0));
        assert_eq!(Fixed::from_int(1), Fixed(FRACUNIT));
        assert_eq!(Fixed::from_int(2), Fixed(131072));
        assert_eq!(Fixed::from_int(-1), Fixed(-FRACUNIT));
    }

    #[test]
    fn test_zero_and_one_constants() {
        assert_eq!(Fixed::ZERO, Fixed(0));
        assert_eq!(Fixed::ONE, Fixed(FRACUNIT));
        assert_eq!(Fixed::ONE, Fixed(65536));
    }

    // -- FixedMul (behavioral parity with m_fixed.c lines 43-49) --

    #[test]
    fn test_fixed_mul_one_times_one() {
        // 1.0 * 1.0 = 1.0
        let result = Fixed(FRACUNIT).fixed_mul(Fixed(FRACUNIT));
        assert_eq!(result, Fixed(FRACUNIT));
    }

    #[test]
    fn test_fixed_mul_two_times_half() {
        // 2.0 * 0.5 = 1.0
        let result = Fixed(131072).fixed_mul(Fixed(32768));
        assert_eq!(result, Fixed(FRACUNIT));
    }

    #[test]
    fn test_fixed_mul_zero() {
        // 0.0 * anything = 0.0
        let result = Fixed(0).fixed_mul(Fixed(FRACUNIT));
        assert_eq!(result, Fixed(0));
    }

    #[test]
    fn test_fixed_mul_negative() {
        // 1.0 * -1.0 = -1.0
        let result = Fixed(FRACUNIT).fixed_mul(Fixed(-FRACUNIT));
        assert_eq!(result, Fixed(-FRACUNIT));
    }

    #[test]
    fn test_fixed_mul_negative_times_negative() {
        // -1.0 * -1.0 = 1.0
        let result = Fixed(-FRACUNIT).fixed_mul(Fixed(-FRACUNIT));
        assert_eq!(result, Fixed(FRACUNIT));
    }

    #[test]
    fn test_fixed_mul_large_values() {
        // Tests that 64-bit intermediate prevents overflow
        // 100.0 * 100.0 = 10000.0
        let a = Fixed::from_int(100);
        let b = Fixed::from_int(100);
        let result = a.fixed_mul(b);
        assert_eq!(result, Fixed::from_int(10000));
    }

    #[test]
    fn test_fixed_mul_fractional() {
        // 0.5 * 0.5 = 0.25
        let half = Fixed(FRACUNIT / 2);
        let result = half.fixed_mul(half);
        assert_eq!(result, Fixed(FRACUNIT / 4));
    }

    // -- FixedDiv (behavioral parity with m_fixed.c lines 57-65) --

    #[test]
    fn test_fixed_div_one_by_one() {
        // 1.0 / 1.0 = 1.0
        let result = Fixed(FRACUNIT).fixed_div(Fixed(FRACUNIT));
        assert_eq!(result, Fixed(FRACUNIT));
    }

    #[test]
    fn test_fixed_div_two_by_one() {
        // 2.0 / 1.0 = 2.0
        let result = Fixed(131072).fixed_div(Fixed(FRACUNIT));
        assert_eq!(result, Fixed(131072));
    }

    #[test]
    fn test_fixed_div_one_by_two() {
        // 1.0 / 2.0 = 0.5
        let result = Fixed(FRACUNIT).fixed_div(Fixed(131072));
        assert_eq!(result, Fixed(32768));
    }

    #[test]
    fn test_fixed_div_negative() {
        // 1.0 / -1.0 = -1.0
        let result = Fixed(FRACUNIT).fixed_div(Fixed(-FRACUNIT));
        assert_eq!(result, Fixed(-FRACUNIT));
    }

    #[test]
    fn test_fixed_div_overflow_positive() {
        // MAXINT / 1 should overflow and return MAXINT
        let result = Fixed(i32::MAX).fixed_div(Fixed(1));
        assert_eq!(result, Fixed(i32::MAX));
    }

    #[test]
    fn test_fixed_div_overflow_negative_sign() {
        // MAXINT / -1 should overflow and return MININT (different signs)
        let result = Fixed(i32::MAX).fixed_div(Fixed(-1));
        assert_eq!(result, Fixed(i32::MIN));
    }

    #[test]
    fn test_fixed_div_by_zero() {
        // Division by zero → overflow check triggers, returns MAXINT
        let result = Fixed(FRACUNIT).fixed_div(Fixed(0));
        assert_eq!(result, Fixed(i32::MAX));
    }

    // -- FixedDiv2 (behavioral parity with m_fixed.c lines 69-87, double path) --

    #[test]
    fn test_fixed_div2_basic() {
        // 1.0 / 2.0 = 0.5
        let result = Fixed(FRACUNIT).fixed_div2(Fixed(131072));
        assert_eq!(result, Fixed(32768));
    }

    #[test]
    fn test_fixed_div2_identity() {
        // 1.0 / 1.0 = 1.0
        let result = Fixed(FRACUNIT).fixed_div2(Fixed(FRACUNIT));
        assert_eq!(result, Fixed(FRACUNIT));
    }

    // -- Operator Tests --

    #[test]
    fn test_add() {
        let a = Fixed(100);
        let b = Fixed(200);
        assert_eq!(a + b, Fixed(300));
    }

    #[test]
    fn test_add_wrapping() {
        // i32::MAX + 1 should wrap (matching C behavior)
        let a = Fixed(i32::MAX);
        let b = Fixed(1);
        assert_eq!(a + b, Fixed(i32::MIN));
    }

    #[test]
    fn test_sub() {
        let a = Fixed(300);
        let b = Fixed(100);
        assert_eq!(a - b, Fixed(200));
    }

    #[test]
    fn test_sub_wrapping() {
        // i32::MIN - 1 should wrap
        let a = Fixed(i32::MIN);
        let b = Fixed(1);
        assert_eq!(a - b, Fixed(i32::MAX));
    }

    #[test]
    fn test_neg() {
        assert_eq!(-Fixed(100), Fixed(-100));
        assert_eq!(-Fixed(-100), Fixed(100));
        assert_eq!(-Fixed(0), Fixed(0));
    }

    #[test]
    fn test_neg_wrapping_min() {
        // -i32::MIN should wrap to i32::MIN (2's complement behavior)
        assert_eq!(-Fixed(i32::MIN), Fixed(i32::MIN));
    }

    // -- Display --

    #[test]
    fn test_display_one() {
        assert_eq!(format!("{}", Fixed(FRACUNIT)), "1.0000");
    }

    #[test]
    fn test_display_half() {
        assert_eq!(format!("{}", Fixed(FRACUNIT / 2)), "0.5000");
    }

    #[test]
    fn test_display_zero() {
        assert_eq!(format!("{}", Fixed(0)), "0.0000");
    }

    #[test]
    fn test_display_two_point_five() {
        let val = Fixed::from_int(2) + Fixed(FRACUNIT / 2);
        assert_eq!(format!("{}", val), "2.5000");
    }

    // -- From/Into --

    #[test]
    fn test_from_i32() {
        let f: Fixed = 42.into();
        assert_eq!(f, Fixed(42));
    }

    #[test]
    fn test_into_i32() {
        let f = Fixed(42);
        let v: i32 = f.into();
        assert_eq!(v, 42);
    }

    // -- Default --

    #[test]
    fn test_default() {
        assert_eq!(Fixed::default(), Fixed(0));
    }

    // -- Ordering --

    #[test]
    fn test_ordering() {
        assert!(Fixed(100) > Fixed(50));
        assert!(Fixed(-100) < Fixed(100));
        assert!(Fixed(100) == Fixed(100));
    }
}
