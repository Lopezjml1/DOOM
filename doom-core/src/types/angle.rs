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

//! Translated from linuxdoom-1.10/tables.h (angle definitions)
//!
//! Binary Angle Measurement (BAM) type and constants.
//! A full circle is 2^32 units. u32 wrapping arithmetic naturally handles
//! angle wraparound, making this representation ideal for game engine use.
//!
//! In the original C source, `angle_t` is defined as `typedef unsigned angle_t;`
//! (tables.h line 78). The BAM system uses the entire 32-bit unsigned range to
//! represent 360 degrees, so addition and subtraction naturally wrap around at
//! the full-circle boundary without any explicit modulus operation.
//!
//! The fine angle constants (`FINEANGLES`, `FINEMASK`, `ANGLETOFINESHIFT`) map
//! BAM angles to indices in the trigonometric lookup tables (`finesine`,
//! `finetangent`, `tantoangle`) defined in the companion `tables` module.

use std::fmt;
use std::ops::{Add, BitAnd, Neg, Shr, Sub};

// ---------------------------------------------------------------------------
// Angle newtype — wraps a u32 BAM value
// ---------------------------------------------------------------------------

/// Binary Angle Measurement (BAM) newtype.
///
/// Wraps a `u32` where the full `0..=u32::MAX` range represents a complete
/// 360-degree circle. Arithmetic operators use wrapping semantics so that
/// angle addition and subtraction naturally handle the 360° → 0° boundary.
///
/// # Original C equivalent
/// ```c
/// typedef unsigned angle_t;  // tables.h line 78
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Angle(pub u32);

// ---------------------------------------------------------------------------
// BAM angle constants (tables.h lines 68-71)
// ---------------------------------------------------------------------------

/// 45 degrees in BAM units.
pub const ANG45: Angle = Angle(0x20000000);

/// 90 degrees in BAM units.
pub const ANG90: Angle = Angle(0x40000000);

/// 180 degrees in BAM units.
pub const ANG180: Angle = Angle(0x80000000);

/// 270 degrees in BAM units.
pub const ANG270: Angle = Angle(0xc0000000);

// ---------------------------------------------------------------------------
// Fine angle constants (tables.h lines 50-55)
// ---------------------------------------------------------------------------

/// Number of entries in the fine-angle lookup tables.
///
/// The `finesine` and `finetangent` arrays use 8192 angular subdivisions
/// covering 360 degrees (for sine) or 180 degrees (for tangent).
pub const FINEANGLES: u32 = 8192;

/// Bitmask for fine-angle indices: `FINEANGLES - 1` = 8191.
///
/// Used to wrap fine-angle indices into the valid `0..8191` range.
pub const FINEMASK: u32 = FINEANGLES - 1;

/// Right-shift amount to convert a BAM angle to a fine-angle index.
///
/// `BAM >> 19` maps the 32-bit BAM range `0..0xFFFFFFFF` onto the 13-bit
/// fine-angle range `0..8191`. This effectively divides the angle by
/// `0x100000000 / 8192 = 0x80000` (524288).
pub const ANGLETOFINESHIFT: u32 = 19;

// ---------------------------------------------------------------------------
// Operator trait implementations — wrapping BAM arithmetic
// ---------------------------------------------------------------------------

impl Add for Angle {
    type Output = Angle;

    /// Add two BAM angles with wrapping semantics.
    ///
    /// Wrapping is correct behavior: adding past 360° should wrap to 0°.
    #[inline]
    fn add(self, rhs: Angle) -> Angle {
        Angle(self.0.wrapping_add(rhs.0))
    }
}

impl Sub for Angle {
    type Output = Angle;

    /// Subtract two BAM angles with wrapping semantics.
    ///
    /// Wrapping is correct behavior: subtracting past 0° should wrap to 360°.
    #[inline]
    fn sub(self, rhs: Angle) -> Angle {
        Angle(self.0.wrapping_sub(rhs.0))
    }
}

impl Neg for Angle {
    type Output = Angle;

    /// Negate a BAM angle (compute the complementary angle).
    ///
    /// For BAM angles, negation is equivalent to `360° - angle`.
    #[inline]
    fn neg(self) -> Angle {
        Angle(self.0.wrapping_neg())
    }
}

impl BitAnd<u32> for Angle {
    type Output = Angle;

    /// Bitwise AND between a BAM angle and a raw mask value.
    ///
    /// Commonly used to mask fine-angle bits or extract angle components.
    #[inline]
    fn bitand(self, rhs: u32) -> Angle {
        Angle(self.0 & rhs)
    }
}

impl Shr<u32> for Angle {
    type Output = Angle;

    /// Right-shift a BAM angle by a given number of bits.
    ///
    /// Used primarily via `ANGLETOFINESHIFT` to convert BAM → fine-angle index.
    #[inline]
    fn shr(self, rhs: u32) -> Angle {
        Angle(self.0 >> rhs)
    }
}

// ---------------------------------------------------------------------------
// Utility methods
// ---------------------------------------------------------------------------

impl Angle {
    /// Create a new angle from a raw u32 BAM value.
    ///
    /// # Examples
    /// ```
    /// use doom_core::types::angle::{Angle, ANG90};
    /// let a = Angle::new(0x40000000);
    /// assert_eq!(a, ANG90);
    /// ```
    #[inline]
    pub const fn new(value: u32) -> Self {
        Angle(value)
    }

    /// Get the raw u32 BAM value.
    ///
    /// # Examples
    /// ```
    /// use doom_core::types::angle::ANG180;
    /// assert_eq!(ANG180.value(), 0x80000000);
    /// ```
    #[inline]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// Convert BAM angle to fine angle index (0..8191) for table lookups.
    ///
    /// This performs `self.0 >> ANGLETOFINESHIFT`, mapping the full 32-bit
    /// BAM range onto the 13-bit fine-angle range used by the `finesine`,
    /// `finetangent`, and related trigonometric lookup tables.
    ///
    /// # Examples
    /// ```
    /// use doom_core::types::angle::{Angle, ANG90, FINEANGLES};
    /// // ANG90 (0x40000000) >> 19 = 0x2000 = 8192 / 4 = 2048
    /// assert_eq!(ANG90.to_fine_angle(), (FINEANGLES / 4) as usize);
    /// ```
    #[inline]
    pub const fn to_fine_angle(self) -> usize {
        (self.0 >> ANGLETOFINESHIFT) as usize
    }
}

// ---------------------------------------------------------------------------
// Display implementation
// ---------------------------------------------------------------------------

impl fmt::Display for Angle {
    /// Format the angle as a hexadecimal BAM value for human-readable output.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Angle(0x{:08x})", self.0)
    }
}

// ---------------------------------------------------------------------------
// From / Into conversions
// ---------------------------------------------------------------------------

impl From<u32> for Angle {
    /// Convert a raw `u32` BAM value into an [`Angle`].
    #[inline]
    fn from(v: u32) -> Self {
        Angle(v)
    }
}

impl From<Angle> for u32 {
    /// Extract the raw `u32` BAM value from an [`Angle`].
    #[inline]
    fn from(a: Angle) -> Self {
        a.0
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_angle_constants_match_original() {
        // tables.h lines 68-71: exact hex values
        assert_eq!(ANG45.0, 0x20000000);
        assert_eq!(ANG90.0, 0x40000000);
        assert_eq!(ANG180.0, 0x80000000);
        assert_eq!(ANG270.0, 0xc0000000);
    }

    #[test]
    fn test_fine_angle_constants() {
        // tables.h lines 50-55
        assert_eq!(FINEANGLES, 8192);
        assert_eq!(FINEMASK, 8191);
        assert_eq!(ANGLETOFINESHIFT, 19);
    }

    #[test]
    fn test_angle_addition_wrapping() {
        // 270° + 180° should wrap to 90°
        let result = ANG270 + ANG180;
        assert_eq!(result, ANG90);
    }

    #[test]
    fn test_angle_subtraction_wrapping() {
        // 45° - 90° should wrap to 315° (0x20000000 - 0x40000000 = 0xe0000000)
        let result = ANG45 - ANG90;
        assert_eq!(result.0, 0xe0000000);
    }

    #[test]
    fn test_angle_negation() {
        // -90° in BAM is 270°
        let result = -ANG90;
        assert_eq!(result, ANG270);
    }

    #[test]
    fn test_angle_bitand() {
        // Masking ANG90 with FINEMASK should give the lower 13 bits
        let result = ANG90 & FINEMASK;
        assert_eq!(result.0, 0x40000000 & 8191);
    }

    #[test]
    fn test_angle_shr() {
        // ANG90 >> ANGLETOFINESHIFT should give 2048 (= FINEANGLES / 4)
        let result = ANG90 >> ANGLETOFINESHIFT;
        assert_eq!(result.0, 2048);
    }

    #[test]
    fn test_to_fine_angle() {
        // ANG90 maps to fine-angle index 2048
        assert_eq!(ANG90.to_fine_angle(), 2048);
        // ANG180 maps to fine-angle index 4096
        assert_eq!(ANG180.to_fine_angle(), 4096);
        // ANG270 maps to fine-angle index 6144
        assert_eq!(ANG270.to_fine_angle(), 6144);
        // 0° maps to fine-angle index 0
        assert_eq!(Angle(0).to_fine_angle(), 0);
    }

    #[test]
    fn test_new_and_value() {
        let a = Angle::new(0xDEADBEEF);
        assert_eq!(a.value(), 0xDEADBEEF);
    }

    #[test]
    fn test_from_u32() {
        let a: Angle = 0x12345678u32.into();
        assert_eq!(a.0, 0x12345678);
    }

    #[test]
    fn test_into_u32() {
        let v: u32 = ANG45.into();
        assert_eq!(v, 0x20000000);
    }

    #[test]
    fn test_display() {
        let s = format!("{}", ANG90);
        assert_eq!(s, "Angle(0x40000000)");
    }

    #[test]
    fn test_default_is_zero() {
        let a = Angle::default();
        assert_eq!(a.0, 0);
    }

    #[test]
    fn test_full_circle_wrapping() {
        // Adding four ANG90 values should wrap back to 0
        let full = ANG90 + ANG90 + ANG90 + ANG90;
        assert_eq!(full.0, 0);
    }

    #[test]
    fn test_ordering() {
        assert!(ANG45 < ANG90);
        assert!(ANG90 < ANG180);
        assert!(ANG180 < ANG270);
    }

    #[test]
    fn test_equality() {
        assert_eq!(Angle::new(100), Angle::new(100));
        assert_ne!(Angle::new(100), Angle::new(200));
    }

    #[test]
    fn test_clone_copy() {
        let a = ANG45;
        let b = a; // Copy
        #[allow(clippy::clone_on_copy)]
        let c = a.clone(); // Clone (explicit, testing the trait)
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn test_angletofineshift_maps_full_range() {
        // The maximum BAM value (0xFFFFFFFF) should map to FINEANGLES-1 (8191)
        // after right-shifting by 19 bits
        let max_angle = Angle(u32::MAX);
        assert_eq!(max_angle.to_fine_angle(), 8191);
    }

    #[test]
    fn test_neg_zero_is_zero() {
        let zero = Angle(0);
        assert_eq!(-zero, Angle(0));
    }

    #[test]
    fn test_sub_self_is_zero() {
        assert_eq!((ANG90 - ANG90).0, 0);
        assert_eq!((ANG180 - ANG180).0, 0);
        assert_eq!((ANG270 - ANG270).0, 0);
    }
}
