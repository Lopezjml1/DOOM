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

//! High-resolution timing module — `I_GetTime` and `I_WaitVBL` equivalents.
//!
//! Translated from `linuxdoom-1.10/i_system.c` (lines 84–137).
//!
//! This module provides the fundamental timing heartbeat for DOOM's game loop,
//! converting wall-clock elapsed time into 35-tic-per-second game time units.
//!
//! # Original C implementation
//!
//! The original `I_GetTime()` (lines 88–100) used POSIX `gettimeofday()` to
//! read wall-clock time, captured the first call's `tv_sec` in a `static int
//! basetime`, and computed elapsed tics as:
//!
//! ```c
//! newtics = (tp.tv_sec - basetime) * TICRATE + tp.tv_usec * TICRATE / 1000000;
//! ```
//!
//! The original `I_WaitVBL(count)` (lines 126–137) called
//! `usleep(count * (1000000/70))` to simulate a vertical-blank delay at 70 Hz
//! (the DOS VGA refresh rate), distinct from the 35 Hz game tic rate.
//!
//! # Rust replacement
//!
//! This module replaces both functions with [`Timer`], a struct that captures a
//! monotonic epoch via [`std::time::Instant`] at construction time and provides:
//!
//! - [`Timer::get_time()`] — returns elapsed game tics (at [`TICRATE`] = 35 Hz)
//! - [`Timer::wait_vbl()`] — sleeps for `count` vertical-blank intervals (at 70 Hz)
//!
//! Using `Instant` instead of `gettimeofday()` provides:
//! - Monotonic guarantees (immune to NTP adjustments and wall-clock jumps)
//! - Nanosecond resolution on Windows (via `QueryPerformanceCounter`)
//! - No FFI overhead (pure Rust standard library)

use std::thread;
use std::time::{Duration, Instant};

/// Game simulation ticks per second.
///
/// This matches the `TICRATE` constant defined in `linuxdoom-1.10/doomdef.h`.
/// The game loop advances one simulation tic every 1/35th of a second
/// (approximately 28.57 ms per tic).
///
/// Note: the original C code comment on `I_GetTime` says "returns time in
/// 1/70th second tics", but the *actual computation* in the code uses `TICRATE`
/// which is 35, not 70. The comment is misleading and does not match the
/// implementation. We follow the code, not the comment.
const TICRATE: i32 = 35;

/// DOS VGA vertical blanking rate in Hz.
///
/// The original `I_WaitVBL` uses 70, **not** `TICRATE` (35). This is a
/// deliberate constant representing the DOS VGA refresh rate at which the
/// original engine synchronized screen updates. The game runs at half this
/// rate (35 tics/second), but certain wait operations (palette fades, screen
/// wipes) use the full 70 Hz VBL timing.
const VBL_RATE: u64 = 70;

/// High-resolution timer for DOOM's game loop.
///
/// Replaces the `gettimeofday()`-based `I_GetTime()` and the `usleep()`-based
/// `I_WaitVBL()` from `linuxdoom-1.10/i_system.c`.
///
/// # Construction
///
/// Call [`Timer::new()`] once at engine startup. This captures a monotonic
/// epoch equivalent to the original C code's `static int basetime` variable
/// that was set on the first call to `I_GetTime()`.
///
/// # Thread Safety
///
/// `Timer` is `Send + Sync` because [`Instant`] is `Send + Sync`. The timer
/// reads only immutable state after construction, so concurrent `get_time()`
/// calls from multiple threads are safe.
///
/// # Example
///
/// ```rust
/// use doom_platform_win::timer::Timer;
///
/// let timer = Timer::new();
/// // Immediately after creation, elapsed tics should be 0 or very close to it.
/// let tics = timer.get_time();
/// assert!(tics >= 0);
/// ```
pub struct Timer {
    /// The monotonic epoch captured at construction time.
    ///
    /// This is the Rust equivalent of the C code's `static int basetime = 0`
    /// variable in `I_GetTime()` (i_system.c line 93). The original code set
    /// `basetime = tp.tv_sec` on the first call; here we capture the full
    /// `Instant` at construction for higher precision.
    start_time: Instant,
}

impl Timer {
    /// Creates a new [`Timer`], capturing the current instant as the epoch.
    ///
    /// This is equivalent to the first call to the original C `I_GetTime()`
    /// which set `basetime = tp.tv_sec`. All subsequent [`get_time()`] calls
    /// measure elapsed tics relative to this epoch.
    ///
    /// [`get_time()`]: Timer::get_time
    #[must_use]
    pub fn new() -> Self {
        let timer = Self {
            start_time: Instant::now(),
        };
        tracing::debug!("Timer initialized: epoch captured via std::time::Instant");
        timer
    }

    /// Returns the current time in game tics (35 tics per second).
    ///
    /// This is the Rust equivalent of `I_GetTime()` from
    /// `linuxdoom-1.10/i_system.c` lines 88–100.
    ///
    /// # Original C computation
    ///
    /// ```c
    /// gettimeofday(&tp, &tzp);
    /// if (!basetime) basetime = tp.tv_sec;
    /// newtics = (tp.tv_sec - basetime) * TICRATE
    ///         + tp.tv_usec * TICRATE / 1000000;
    /// return newtics;
    /// ```
    ///
    /// # Rust equivalent
    ///
    /// We compute:
    /// ```text
    /// elapsed_micros * TICRATE / 1_000_000
    /// ```
    /// which is algebraically equivalent to the C expression
    /// `seconds * TICRATE + microseconds * TICRATE / 1_000_000` since:
    /// ```text
    /// elapsed_micros = seconds * 1_000_000 + microseconds
    /// ```
    ///
    /// We use `i64` intermediate arithmetic to avoid overflow (elapsed
    /// microseconds can exceed `i32::MAX` after ~35 minutes), then truncate
    /// to `i32`. Integer division truncates toward zero, matching C99/Rust
    /// semantics.
    ///
    /// # Returns
    ///
    /// Elapsed game tics as a signed 32-bit integer, matching the original
    /// C function's `int` return type. The value is always non-negative under
    /// normal operation (monotonic clock guarantees forward progress).
    #[must_use]
    pub fn get_time(&self) -> i32 {
        let elapsed = self.start_time.elapsed();
        // Convert to microseconds for precision matching gettimeofday().
        // as_micros() returns u128; we cast to i64 which can hold ~292,000 years
        // of microseconds, far exceeding any realistic DOOM session.
        let micros = elapsed.as_micros() as i64;
        // Compute tics: (microseconds * 35) / 1_000_000
        // This is algebraically equivalent to the original C expression:
        //   (seconds * TICRATE) + (microseconds * TICRATE / 1_000_000)
        // Integer division truncates toward zero in both C and Rust.
        let ticrate = TICRATE as i64;
        let newtics = (micros * ticrate) / 1_000_000;
        newtics as i32
    }

    /// Waits for the specified number of vertical-blank intervals.
    ///
    /// This is the Rust equivalent of `I_WaitVBL(int count)` from
    /// `linuxdoom-1.10/i_system.c` lines 126–137.
    ///
    /// # Original C implementation
    ///
    /// ```c
    /// void I_WaitVBL(int count) {
    ///     usleep(count * (1000000/70));
    /// }
    /// ```
    ///
    /// # Timing
    ///
    /// Each VBL interval is `1_000_000 / 70 ≈ 14_285` microseconds (~14.3 ms),
    /// based on the DOS VGA 70 Hz refresh rate. This is **not** the game tic
    /// rate (35 Hz / ~28.6 ms); the VBL rate is used for timing-sensitive
    /// operations like palette fades and screen wipes.
    ///
    /// # Parameters
    ///
    /// - `count`: Number of VBL intervals to wait. A value of 1 sleeps for
    ///   approximately 14.3 ms. A value of 0 or negative returns immediately
    ///   (matching `usleep(0)` behavior).
    pub fn wait_vbl(&self, count: i32) {
        if count <= 0 {
            return;
        }
        // Original C: usleep(count * (1000000/70))
        // 1_000_000 / 70 = 14285 microseconds (integer division, matching C)
        let micros_per_vbl: u64 = 1_000_000 / VBL_RATE;
        let total_micros = (count as u64) * micros_per_vbl;
        thread::sleep(Duration::from_micros(total_micros));
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    /// Verify that `get_time()` returns 0 (or very close to 0) immediately
    /// after construction.
    #[test]
    fn test_get_time_initial_is_zero() {
        let timer = Timer::new();
        let tics = timer.get_time();
        // Immediately after creation, no significant time should have elapsed.
        // Allow a margin of 1 tic (~28.6 ms) for slow CI environments.
        assert!(tics < 2, "Expected initial tics near 0, got {tics}");
    }

    /// Verify that `get_time()` returns approximately `TICRATE` (35) after
    /// sleeping for 1 second.
    #[test]
    fn test_get_time_after_one_second() {
        let timer = Timer::new();
        thread::sleep(Duration::from_secs(1));
        let tics = timer.get_time();
        // Allow ±3 tics tolerance for thread scheduling jitter.
        assert!(
            (32..=38).contains(&tics),
            "Expected ~35 tics after 1s, got {tics}"
        );
    }

    /// Verify that `get_time()` returns non-negative values (monotonic).
    #[test]
    fn test_get_time_monotonic() {
        let timer = Timer::new();
        let t1 = timer.get_time();
        thread::sleep(Duration::from_millis(50));
        let t2 = timer.get_time();
        assert!(t2 >= t1, "Timer should be monotonic: t1={t1}, t2={t2}");
    }

    /// Verify that `wait_vbl(1)` sleeps for approximately 14.3 ms.
    #[test]
    fn test_wait_vbl_duration() {
        let timer = Timer::new();
        let before = Instant::now();
        timer.wait_vbl(1);
        let elapsed = before.elapsed();
        // 1_000_000/70 = 14285 µs ≈ 14.3 ms
        // Allow generous bounds: 10 ms to 50 ms (thread scheduling can overshoot).
        assert!(
            elapsed.as_millis() >= 10 && elapsed.as_millis() <= 50,
            "Expected wait_vbl(1) to sleep ~14ms, got {}ms",
            elapsed.as_millis()
        );
    }

    /// Verify that `wait_vbl(0)` returns immediately without sleeping.
    #[test]
    fn test_wait_vbl_zero_no_sleep() {
        let timer = Timer::new();
        let before = Instant::now();
        timer.wait_vbl(0);
        let elapsed = before.elapsed();
        assert!(
            elapsed.as_millis() < 5,
            "wait_vbl(0) should return immediately, took {}ms",
            elapsed.as_millis()
        );
    }

    /// Verify that `wait_vbl` with a negative count returns immediately.
    #[test]
    fn test_wait_vbl_negative_no_sleep() {
        let timer = Timer::new();
        let before = Instant::now();
        timer.wait_vbl(-1);
        let elapsed = before.elapsed();
        assert!(
            elapsed.as_millis() < 5,
            "wait_vbl(-1) should return immediately, took {}ms",
            elapsed.as_millis()
        );
    }

    /// Verify that the `Default` trait implementation works.
    #[test]
    fn test_default_trait() {
        let timer = Timer::default();
        let tics = timer.get_time();
        assert!(tics < 2, "Default timer should start near 0, got {tics}");
    }

    /// Verify the TICRATE constant matches the engine specification.
    #[test]
    fn test_ticrate_constant() {
        assert_eq!(TICRATE, 35, "TICRATE must be 35 per doomdef.h");
    }

    /// Verify the VBL_RATE constant matches the DOS VGA refresh rate.
    #[test]
    fn test_vbl_rate_constant() {
        assert_eq!(VBL_RATE, 70, "VBL_RATE must be 70 Hz (DOS VGA refresh)");
    }
}
