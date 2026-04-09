// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Translated from linuxdoom-1.10/p_lights.c
//!
//! Handle sector-based lighting effects: fire flicker, broken light flash,
//! strobe flash, and smooth glow. These are spawned during level setup
//! based on sector special types and by line trigger events.
//!
//! # Light Effect Thinkers
//!
//! Each light effect is driven by a thinker that runs once per game tic
//! (35 Hz). The thinker modifies its associated sector's `lightlevel`
//! field to produce the visual effect.
//!
//! | C function              | Rust function              | Description                     |
//! |-------------------------|----------------------------|---------------------------------|
//! | `T_FireFlicker`         | [`t_fire_flicker`]         | Fire flicker light effect       |
//! | `P_SpawnFireFlicker`    | [`p_spawn_fire_flicker`]   | Spawn fire flicker for sector   |
//! | `T_LightFlash`          | [`t_light_flash`]          | Random light flash effect       |
//! | `P_SpawnLightFlash`     | [`p_spawn_light_flash`]    | Spawn light flash for sector    |
//! | `T_StrobeFlash`         | [`t_strobe_flash`]         | Periodic strobe flash effect    |
//! | `P_SpawnStrobeFlash`    | [`p_spawn_strobe_flash`]   | Spawn strobe flash for sector   |
//! | `EV_StartLightStrobing` | [`ev_start_light_strobing`]| Start strobing by line tag      |
//! | `EV_TurnTagLightsOff`   | [`ev_turn_tag_lights_off`] | Turn tagged lights off          |
//! | `EV_LightTurnOn`        | [`ev_light_turn_on`]       | Turn tagged lights on           |
//! | `T_Glow`                | [`t_glow`]                 | Pulsing glow light effect       |
//! | `P_SpawnGlowingLight`   | [`p_spawn_glowing_light`]  | Spawn glow for sector           |

use crate::play::spec::{
    get_next_sector, p_find_min_surrounding_light, p_find_sector_from_line_tag, FireFlickerT,
    GlowT, LightFlashT, StrobeFlashT,
};
use crate::play::tick::{self, ThinkerList};
use crate::types::map_data::{LineDef, Sector};
use crate::types::thinker::ActionFn;
use crate::util::random::DoomRandom;

// ============================================================================
// Constants — originally from p_local.h, used exclusively by light functions
// ============================================================================

/// Glow light change per tic. The glow effect adds or subtracts this value
/// from the sector's light level each game tic to create a smooth oscillation.
pub const GLOWSPEED: i32 = 8;

/// Bright time for strobe flash (in tics). Determines how many tics the
/// strobe stays at maximum brightness before switching to darkness.
pub const STROBEBRIGHT: i32 = 5;

/// Fast strobe dark time (in tics). Used for rapid strobe effects, spending
/// 15 tics in the dark phase.
pub const FASTDARK: i32 = 15;

/// Slow strobe dark time (in tics). Used for slow strobe effects, spending
/// 35 tics (one full second) in the dark phase.
pub const SLOWDARK: i32 = 35;

// ============================================================================
// FIRELIGHT FLICKER
// ============================================================================

// ----------------------------------------------------------------------------
// T_FireFlicker — Fire flicker light effect
// Translated from p_lights.c lines 46-61
// ----------------------------------------------------------------------------

/// Process one tic of fire flicker for a sector.
///
/// The fire flicker randomly adjusts the sector's light level between
/// `minlight` and `maxlight`. A countdown timer (`count`) ensures the
/// effect only changes every 4 tics. When the timer expires, a random
/// amount of 0, 16, 32, or 48 is subtracted from `maxlight`. If the
/// current sector light minus the amount would fall below `minlight`,
/// `minlight` is used instead; otherwise `maxlight - amount` is applied.
///
/// The comparison uses the sector's current light level (not maxlight),
/// faithfully preserving the original C behavior where the condition
/// depends on the sector's live state rather than the thinker's cached
/// maximum.
///
/// Translated from `T_FireFlicker` in p_lights.c lines 46-61.
pub fn t_fire_flicker(flicker: &mut FireFlickerT, sectors: &mut [Sector], rng: &mut DoomRandom) {
    // Pre-decrement count; if non-zero, wait
    flicker.count -= 1;
    if flicker.count != 0 {
        return;
    }

    // Random amount: (P_Random() & 3) * 16 → 0, 16, 32, or 48
    let amount = ((rng.p_random() as i32) & 3) * 16;

    // Apply flicker: check current sector lightlevel against minimum
    let current_light = sectors[flicker.sector].lightlevel as i32;
    if current_light - amount < flicker.minlight {
        sectors[flicker.sector].lightlevel = flicker.minlight as i16;
    } else {
        sectors[flicker.sector].lightlevel = (flicker.maxlight - amount) as i16;
    }

    // Reset countdown
    flicker.count = 4;
}

// ----------------------------------------------------------------------------
// P_SpawnFireFlicker — Spawn fire flicker for sector
// Translated from p_lights.c lines 68-85
// ----------------------------------------------------------------------------

/// Spawn a fire flicker light effect for a sector.
///
/// Clears the sector's special type (preventing re-spawn), creates a
/// [`FireFlickerT`] thinker with the sector's current light level as
/// `maxlight` and the minimum surrounding light level + 16 as `minlight`,
/// and registers it with the thinker list.
///
/// Translated from `P_SpawnFireFlicker` in p_lights.c lines 68-85.
pub fn p_spawn_fire_flicker(
    sector_idx: usize,
    sectors: &mut [Sector],
    lines: &[LineDef],
    thinkers: &mut ThinkerList,
    storage: &mut Vec<FireFlickerT>,
) {
    // Read values before mutation (immutable borrow of sectors)
    let lightlevel = sectors[sector_idx].lightlevel as i32;
    let minlight = p_find_min_surrounding_light(sector_idx, lightlevel, sectors, lines) + 16;

    // Clear sector special to prevent re-spawn
    sectors[sector_idx].special = 0;

    // Construct the fire flicker thinker
    let mut flicker = FireFlickerT::new(sector_idx);
    flicker.thinker.function = ActionFn::FireFlicker;
    flicker.maxlight = lightlevel;
    flicker.minlight = minlight;
    flicker.count = 4;

    // Store in arena and register with thinker list
    let data_idx = storage.len();
    storage.push(flicker);
    tick::p_add_thinker(thinkers, ActionFn::FireFlicker, data_idx);
}

// ============================================================================
// BROKEN LIGHT FLASHING
// ============================================================================

// ----------------------------------------------------------------------------
// T_LightFlash — Random light flash effect
// Translated from p_lights.c lines 98-114
// ----------------------------------------------------------------------------

/// Process one tic of random light flash for a sector.
///
/// Toggles the sector between `maxlight` and `minlight` with randomized
/// timing intervals. When the sector is at maximum brightness, it switches
/// to minimum with a delay of `(P_Random() & mintime) + 1` tics. When at
/// minimum, it switches back to maximum with `(P_Random() & maxtime) + 1`
/// tics.
///
/// Translated from `T_LightFlash` in p_lights.c lines 98-114.
pub fn t_light_flash(flash: &mut LightFlashT, sectors: &mut [Sector], rng: &mut DoomRandom) {
    // Pre-decrement count; if non-zero, wait
    flash.count -= 1;
    if flash.count != 0 {
        return;
    }

    if (sectors[flash.sector].lightlevel as i32) == flash.maxlight {
        // Currently at max — switch to min, set random delay based on mintime
        sectors[flash.sector].lightlevel = flash.minlight as i16;
        flash.count = ((rng.p_random() as i32) & flash.mintime) + 1;
    } else {
        // Currently at min (or other) — switch to max, set random delay based on maxtime
        sectors[flash.sector].lightlevel = flash.maxlight as i16;
        flash.count = ((rng.p_random() as i32) & flash.maxtime) + 1;
    }
}

// ----------------------------------------------------------------------------
// P_SpawnLightFlash — Spawn light flash for sector
// Translated from p_lights.c lines 124-143
// ----------------------------------------------------------------------------

/// Spawn a random light flash effect for a sector.
///
/// Clears the sector's special type, creates a [`LightFlashT`] thinker
/// with `maxtime = 64` and `mintime = 7`, and registers it with the
/// thinker list. The initial countdown is randomized using
/// `(P_Random() & maxtime) + 1`.
///
/// Translated from `P_SpawnLightFlash` in p_lights.c lines 124-143.
pub fn p_spawn_light_flash(
    sector_idx: usize,
    sectors: &mut [Sector],
    lines: &[LineDef],
    rng: &mut DoomRandom,
    thinkers: &mut ThinkerList,
    storage: &mut Vec<LightFlashT>,
) {
    // Read values before mutation
    let lightlevel = sectors[sector_idx].lightlevel as i32;
    let minlight = p_find_min_surrounding_light(sector_idx, lightlevel, sectors, lines);

    // Clear sector special
    sectors[sector_idx].special = 0;

    // Construct the light flash thinker
    let mut flash = LightFlashT::new(sector_idx);
    flash.thinker.function = ActionFn::LightFlash;
    flash.maxlight = lightlevel;
    flash.minlight = minlight;
    flash.maxtime = 64;
    flash.mintime = 7;
    flash.count = ((rng.p_random() as i32) & flash.maxtime) + 1;

    // Store and register
    let data_idx = storage.len();
    storage.push(flash);
    tick::p_add_thinker(thinkers, ActionFn::LightFlash, data_idx);
}

// ============================================================================
// STROBE LIGHT FLASHING
// ============================================================================

// ----------------------------------------------------------------------------
// T_StrobeFlash — Periodic strobe flash effect
// Translated from p_lights.c lines 155-171
// ----------------------------------------------------------------------------

/// Process one tic of strobe flash for a sector.
///
/// Toggles between `maxlight` and `minlight` on a fixed schedule. The
/// bright phase lasts `brighttime` tics and the dark phase lasts
/// `darktime` tics. Unlike fire flicker and light flash, strobe timing
/// is deterministic (no randomness per toggle).
///
/// Translated from `T_StrobeFlash` in p_lights.c lines 155-171.
pub fn t_strobe_flash(strobe: &mut StrobeFlashT, sectors: &mut [Sector]) {
    // Pre-decrement count; if non-zero, wait
    strobe.count -= 1;
    if strobe.count != 0 {
        return;
    }

    if (sectors[strobe.sector].lightlevel as i32) == strobe.minlight {
        // Currently dark — switch to bright
        sectors[strobe.sector].lightlevel = strobe.maxlight as i16;
        strobe.count = strobe.brighttime;
    } else {
        // Currently bright — switch to dark
        sectors[strobe.sector].lightlevel = strobe.minlight as i16;
        strobe.count = strobe.darktime;
    }
}

// ----------------------------------------------------------------------------
// P_SpawnStrobeFlash — Spawn strobe flash for sector
// Translated from p_lights.c lines 180-209
// ----------------------------------------------------------------------------

/// Spawn a periodic strobe flash effect for a sector.
///
/// Creates a [`StrobeFlashT`] thinker with the given dark time and
/// [`STROBEBRIGHT`] as the bright time. If `in_sync` is false, the
/// initial count is randomized with `(P_Random() & 7) + 1` to
/// desynchronize multiple strobes. If true, count starts at 1.
///
/// If the minimum surrounding light equals the sector's current light
/// level, `minlight` is forced to 0 to ensure visible flashing.
///
/// Translated from `P_SpawnStrobeFlash` in p_lights.c lines 180-209.
pub fn p_spawn_strobe_flash(
    sector_idx: usize,
    fast_or_slow: i32,
    in_sync: bool,
    sectors: &mut [Sector],
    lines: &[LineDef],
    rng: &mut DoomRandom,
    thinkers: &mut ThinkerList,
    storage: &mut Vec<StrobeFlashT>,
) {
    // Read values before mutation
    let lightlevel = sectors[sector_idx].lightlevel as i32;
    let mut minlight = p_find_min_surrounding_light(sector_idx, lightlevel, sectors, lines);

    // Construct the strobe flash thinker
    let mut strobe = StrobeFlashT::new(sector_idx);
    strobe.thinker.function = ActionFn::StrobeFlash;
    strobe.darktime = fast_or_slow;
    strobe.brighttime = STROBEBRIGHT;
    strobe.maxlight = lightlevel;

    // If min surrounding light equals sector light, force minlight to 0
    // to ensure visible flashing
    if minlight == lightlevel {
        minlight = 0;
    }
    strobe.minlight = minlight;

    // Clear sector special
    sectors[sector_idx].special = 0;

    // Set initial count: randomized if not synchronized
    if !in_sync {
        strobe.count = ((rng.p_random() as i32) & 7) + 1;
    } else {
        strobe.count = 1;
    }

    // Store and register
    let data_idx = storage.len();
    storage.push(strobe);
    tick::p_add_thinker(thinkers, ActionFn::StrobeFlash, data_idx);
}

// ----------------------------------------------------------------------------
// EV_StartLightStrobing — Start strobing by line tag
// Translated from p_lights.c lines 215-229
// ----------------------------------------------------------------------------

/// Start strobe lighting in all sectors matching the trigger line's tag.
///
/// Iterates through sectors using [`p_find_sector_from_line_tag`]. Sectors
/// that already have an active special (non-None `specialdata`) are skipped
/// to prevent double-spawning. Each qualifying sector gets a slow
/// ([`SLOWDARK`]) strobe flash spawned without synchronization.
///
/// Translated from `EV_StartLightStrobing` in p_lights.c lines 215-229.
pub fn ev_start_light_strobing(
    line: &LineDef,
    sectors: &mut [Sector],
    lines: &[LineDef],
    rng: &mut DoomRandom,
    thinkers: &mut ThinkerList,
    strobes: &mut Vec<StrobeFlashT>,
) {
    let mut secnum: i32 = -1;
    loop {
        secnum = p_find_sector_from_line_tag(line, secnum, sectors);
        if secnum < 0 {
            break;
        }
        let sec_idx = secnum as usize;

        // Skip sectors that already have active specials
        if sectors[sec_idx].specialdata.is_some() {
            continue;
        }

        p_spawn_strobe_flash(
            sec_idx, SLOWDARK, false, sectors, lines, rng, thinkers, strobes,
        );
    }
}

// ============================================================================
// TURN LINE'S TAG LIGHTS OFF
// ============================================================================

// ----------------------------------------------------------------------------
// EV_TurnTagLightsOff — Turn all lights off in tagged sectors
// Translated from p_lights.c lines 236-264
// ----------------------------------------------------------------------------

/// Turn lights off in all sectors matching the trigger line's tag.
///
/// For each sector whose tag matches the trigger line's tag, finds the
/// minimum light level among all adjacent sectors (accessed via the
/// sector's bounding lines and [`get_next_sector`]) and sets the sector's
/// light level to that minimum.
///
/// Uses a manual iteration over all sectors (not [`p_find_sector_from_line_tag`])
/// to match the original C implementation exactly.
///
/// Translated from `EV_TurnTagLightsOff` in p_lights.c lines 236-264.
pub fn ev_turn_tag_lights_off(line: &LineDef, sectors: &mut [Sector], lines: &[LineDef]) {
    let tag = line.tag;

    for j in 0..sectors.len() {
        if sectors[j].tag == tag {
            // Start with the sector's own light level as the minimum candidate
            let mut min = sectors[j].lightlevel;

            // Search all adjacent sectors for a lower light level
            let num_lines = sectors[j].lines.len();
            for k in 0..num_lines {
                let line_k = sectors[j].lines[k];
                let li = &lines[line_k];
                if let Some(other_idx) = get_next_sector(li, j) {
                    if sectors[other_idx].lightlevel < min {
                        min = sectors[other_idx].lightlevel;
                    }
                }
            }

            sectors[j].lightlevel = min;
        }
    }
}

// ============================================================================
// TURN LINE'S TAG LIGHTS ON
// ============================================================================

// ----------------------------------------------------------------------------
// EV_LightTurnOn — Turn lights on in tagged sectors
// Translated from p_lights.c lines 270-307
// ----------------------------------------------------------------------------

/// Turn lights on in all sectors matching the trigger line's tag.
///
/// If `bright` is 0, searches adjacent sectors of each tagged sector for
/// the maximum light level and uses that value. If `bright` is non-zero,
/// sets the sector's light level to `bright` directly.
///
/// **Behavioral parity note**: The original C code reuses the `bright`
/// parameter as a mutable local across loop iterations. Once a non-zero
/// value is found for the first tagged sector's neighbor search, subsequent
/// tagged sectors receive the same value without performing their own
/// neighbor search. This behavior is preserved exactly.
///
/// Translated from `EV_LightTurnOn` in p_lights.c lines 270-307.
pub fn ev_light_turn_on(line: &LineDef, bright: i32, sectors: &mut [Sector], lines: &[LineDef]) {
    let tag = line.tag;
    // Mutable local matching C parameter behavior: once set non-zero,
    // subsequent tagged sectors skip the neighbor search
    let mut bright = bright;

    for i in 0..sectors.len() {
        if sectors[i].tag == tag {
            // bright == 0 means search for highest surrounding light level
            if bright == 0 {
                let num_lines = sectors[i].lines.len();
                for k in 0..num_lines {
                    let line_k = sectors[i].lines[k];
                    let li = &lines[line_k];
                    if let Some(other_idx) = get_next_sector(li, i) {
                        if (sectors[other_idx].lightlevel as i32) > bright {
                            bright = sectors[other_idx].lightlevel as i32;
                        }
                    }
                }
            }
            sectors[i].lightlevel = bright as i16;
        }
    }
}

// ============================================================================
// GLOW EFFECT
// ============================================================================

// ----------------------------------------------------------------------------
// T_Glow — Pulsing glow light effect
// Translated from p_lights.c lines 314-338
// ----------------------------------------------------------------------------

/// Process one tic of glow effect for a sector.
///
/// Smoothly oscillates the sector's light level between `minlight` and
/// `maxlight` by adding or subtracting [`GLOWSPEED`] each tic. When the
/// light reaches or passes a boundary, the change is **undone** (the light
/// level is restored to its pre-change value) and the direction is
/// reversed. This produces a ping-pong effect where the light bounces
/// between the two extremes without overshoot.
///
/// Translated from `T_Glow` in p_lights.c lines 314-338.
pub fn t_glow(glow: &mut GlowT, sectors: &mut [Sector]) {
    match glow.direction {
        -1 => {
            // Darkening: subtract GLOWSPEED
            sectors[glow.sector].lightlevel -= GLOWSPEED as i16;
            if (sectors[glow.sector].lightlevel as i32) <= glow.minlight {
                // Undo the subtraction (bounce back) and reverse direction
                sectors[glow.sector].lightlevel += GLOWSPEED as i16;
                glow.direction = 1;
            }
        }
        1 => {
            // Brightening: add GLOWSPEED
            sectors[glow.sector].lightlevel += GLOWSPEED as i16;
            if (sectors[glow.sector].lightlevel as i32) >= glow.maxlight {
                // Undo the addition (bounce back) and reverse direction
                sectors[glow.sector].lightlevel -= GLOWSPEED as i16;
                glow.direction = -1;
            }
        }
        _ => {
            // Defensive guard: invalid direction is a no-op
        }
    }
}

// ----------------------------------------------------------------------------
// P_SpawnGlowingLight — Spawn glow for sector
// Translated from p_lights.c lines 341-356
// ----------------------------------------------------------------------------

/// Spawn a smooth glowing light effect for a sector.
///
/// Creates a [`GlowT`] thinker that oscillates between the sector's
/// current light level (`maxlight`) and the minimum surrounding light
/// level (`minlight`), starting in the darkening direction (-1).
/// Clears the sector's special type to prevent re-spawn.
///
/// Translated from `P_SpawnGlowingLight` in p_lights.c lines 341-356.
pub fn p_spawn_glowing_light(
    sector_idx: usize,
    sectors: &mut [Sector],
    lines: &[LineDef],
    thinkers: &mut ThinkerList,
    storage: &mut Vec<GlowT>,
) {
    // Read values before mutation
    let lightlevel = sectors[sector_idx].lightlevel as i32;
    let minlight = p_find_min_surrounding_light(sector_idx, lightlevel, sectors, lines);

    // Construct the glow thinker
    let mut glow = GlowT::new(sector_idx);
    glow.thinker.function = ActionFn::Glow;
    glow.minlight = minlight;
    glow.maxlight = lightlevel;
    glow.direction = -1; // Start darkening

    // Clear sector special
    sectors[sector_idx].special = 0;

    // Store and register
    let data_idx = storage.len();
    storage.push(glow);
    tick::p_add_thinker(thinkers, ActionFn::Glow, data_idx);
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::spec::FireFlickerT;
    use crate::play::tick::ThinkerList;
    use crate::types::map_data::{LineDef, LineFlags, Sector};
    use crate::types::thinker::ActionFn;

    // ---- Constant validation ------------------------------------------------

    #[test]
    fn test_constants() {
        assert_eq!(GLOWSPEED, 8);
        assert_eq!(STROBEBRIGHT, 5);
        assert_eq!(FASTDARK, 15);
        assert_eq!(SLOWDARK, 35);
    }

    // ---- Helper: create a two-sided line connecting two sectors ---------------

    fn make_two_sided_line(tag: i16, front: usize, back: usize) -> LineDef {
        LineDef {
            tag,
            flags: LineFlags::ML_TWOSIDED.bits(),
            frontsector: Some(front),
            backsector: Some(back),
            ..LineDef::default()
        }
    }

    fn make_sector(tag: i16, lightlevel: i16, line_indices: Vec<usize>) -> Sector {
        Sector {
            tag,
            lightlevel,
            linecount: line_indices.len() as i32,
            lines: line_indices,
            ..Sector::default()
        }
    }

    // ---- T_FireFlicker tests ------------------------------------------------

    #[test]
    fn test_fire_flicker_countdown() {
        let mut rng = DoomRandom::new();
        let mut sectors = vec![make_sector(0, 200, vec![])];
        let mut flicker = FireFlickerT::new(0);
        flicker.maxlight = 200;
        flicker.minlight = 100;
        flicker.count = 3;

        t_fire_flicker(&mut flicker, &mut sectors, &mut rng);

        // Count decremented from 3 to 2, no light change
        assert_eq!(flicker.count, 2);
        assert_eq!(sectors[0].lightlevel, 200);
    }

    #[test]
    fn test_fire_flicker_triggers_at_zero() {
        let mut rng = DoomRandom::new();
        let mut sectors = vec![make_sector(0, 200, vec![])];
        let mut flicker = FireFlickerT::new(0);
        flicker.maxlight = 200;
        flicker.minlight = 100;
        flicker.count = 1; // Will decrement to 0 → trigger

        t_fire_flicker(&mut flicker, &mut sectors, &mut rng);

        // Count should be reset to 4
        assert_eq!(flicker.count, 4);
        // Light should be between minlight and maxlight
        let light = sectors[0].lightlevel as i32;
        assert!(
            (100..=200).contains(&light),
            "Light {} out of range [100, 200]",
            light
        );
    }

    #[test]
    fn test_fire_flicker_clamps_to_minlight() {
        // Force a specific RNG state where amount would push below minlight.
        // We create a scenario where maxlight - amount < minlight, so
        // the result should be minlight.
        let mut rng = DoomRandom::new();
        let mut sectors = vec![make_sector(0, 180, vec![])];
        let mut flicker = FireFlickerT::new(0);
        flicker.maxlight = 180;
        flicker.minlight = 170;
        flicker.count = 1;

        // Run multiple times to exercise clamping. At least some iterations
        // will produce amount > 10 (maxlight - minlight) which forces clamping.
        for _ in 0..20 {
            flicker.count = 1;
            t_fire_flicker(&mut flicker, &mut sectors, &mut rng);
            let light = sectors[0].lightlevel as i32;
            assert!(
                (170..=180).contains(&light),
                "Light {} out of range [170, 180]",
                light
            );
        }
    }

    // ---- T_LightFlash tests ------------------------------------------------

    #[test]
    fn test_light_flash_countdown() {
        let mut rng = DoomRandom::new();
        let mut sectors = vec![make_sector(0, 200, vec![])];
        let mut flash = LightFlashT::new(0);
        flash.maxlight = 200;
        flash.minlight = 100;
        flash.maxtime = 64;
        flash.mintime = 7;
        flash.count = 5;

        t_light_flash(&mut flash, &mut sectors, &mut rng);

        assert_eq!(flash.count, 4);
        assert_eq!(sectors[0].lightlevel, 200);
    }

    #[test]
    fn test_light_flash_toggle_to_min() {
        let mut rng = DoomRandom::new();
        let mut sectors = vec![make_sector(0, 200, vec![])];
        let mut flash = LightFlashT::new(0);
        flash.maxlight = 200;
        flash.minlight = 100;
        flash.maxtime = 64;
        flash.mintime = 7;
        flash.count = 1; // Will trigger

        t_light_flash(&mut flash, &mut sectors, &mut rng);

        // Was at maxlight (200), should switch to minlight
        assert_eq!(sectors[0].lightlevel, 100);
        // Count should be (P_Random() & mintime) + 1, range [1, 8]
        assert!(flash.count >= 1 && flash.count <= 8);
    }

    #[test]
    fn test_light_flash_toggle_to_max() {
        let mut rng = DoomRandom::new();
        let mut sectors = vec![make_sector(0, 100, vec![])];
        let mut flash = LightFlashT::new(0);
        flash.maxlight = 200;
        flash.minlight = 100;
        flash.maxtime = 64;
        flash.mintime = 7;
        flash.count = 1;

        t_light_flash(&mut flash, &mut sectors, &mut rng);

        // Was at minlight (100), should switch to maxlight
        assert_eq!(sectors[0].lightlevel, 200);
        // Count should be (P_Random() & maxtime) + 1, range [1, 65]
        assert!(flash.count >= 1 && flash.count <= 65);
    }

    // ---- T_StrobeFlash tests -----------------------------------------------

    #[test]
    fn test_strobe_flash_countdown() {
        let mut sectors = vec![make_sector(0, 200, vec![])];
        let mut strobe = StrobeFlashT::new(0);
        strobe.maxlight = 200;
        strobe.minlight = 50;
        strobe.brighttime = STROBEBRIGHT;
        strobe.darktime = SLOWDARK;
        strobe.count = 3;

        t_strobe_flash(&mut strobe, &mut sectors);

        assert_eq!(strobe.count, 2);
        assert_eq!(sectors[0].lightlevel, 200);
    }

    #[test]
    fn test_strobe_flash_dark_to_bright() {
        let mut sectors = vec![make_sector(0, 50, vec![])];
        let mut strobe = StrobeFlashT::new(0);
        strobe.maxlight = 200;
        strobe.minlight = 50;
        strobe.brighttime = STROBEBRIGHT;
        strobe.darktime = SLOWDARK;
        strobe.count = 1; // Will trigger

        t_strobe_flash(&mut strobe, &mut sectors);

        // Was at minlight, should switch to maxlight
        assert_eq!(sectors[0].lightlevel, 200);
        assert_eq!(strobe.count, STROBEBRIGHT);
    }

    #[test]
    fn test_strobe_flash_bright_to_dark() {
        let mut sectors = vec![make_sector(0, 200, vec![])];
        let mut strobe = StrobeFlashT::new(0);
        strobe.maxlight = 200;
        strobe.minlight = 50;
        strobe.brighttime = STROBEBRIGHT;
        strobe.darktime = SLOWDARK;
        strobe.count = 1;

        t_strobe_flash(&mut strobe, &mut sectors);

        // Was at maxlight (not minlight), should switch to minlight
        assert_eq!(sectors[0].lightlevel, 50);
        assert_eq!(strobe.count, SLOWDARK);
    }

    // ---- T_Glow tests ------------------------------------------------------

    #[test]
    fn test_glow_darkening() {
        let mut sectors = vec![make_sector(0, 160, vec![])];
        let mut glow = GlowT::new(0);
        glow.minlight = 50;
        glow.maxlight = 200;
        glow.direction = -1;

        t_glow(&mut glow, &mut sectors);

        // 160 - 8 = 152, which is > minlight (50), so no reverse
        assert_eq!(sectors[0].lightlevel, 152);
        assert_eq!(glow.direction, -1);
    }

    #[test]
    fn test_glow_darkening_reverses_with_undo() {
        // Light at 58, min at 50. After subtract 8 → 50, which is <= minlight.
        // Undo: 50 + 8 = 58. Direction reverses to +1.
        let mut sectors = vec![make_sector(0, 58, vec![])];
        let mut glow = GlowT::new(0);
        glow.minlight = 50;
        glow.maxlight = 200;
        glow.direction = -1;

        t_glow(&mut glow, &mut sectors);

        // Light should be undone to 58 (not set to minlight 50!)
        assert_eq!(sectors[0].lightlevel, 58);
        assert_eq!(glow.direction, 1);
    }

    #[test]
    fn test_glow_darkening_reverses_past_min() {
        // Light at 55, min at 50. After subtract 8 → 47, which is <= minlight.
        // Undo: 47 + 8 = 55. Direction reverses to +1.
        let mut sectors = vec![make_sector(0, 55, vec![])];
        let mut glow = GlowT::new(0);
        glow.minlight = 50;
        glow.maxlight = 200;
        glow.direction = -1;

        t_glow(&mut glow, &mut sectors);

        // Light should be undone to 55 (original value)
        assert_eq!(sectors[0].lightlevel, 55);
        assert_eq!(glow.direction, 1);
    }

    #[test]
    fn test_glow_brightening() {
        let mut sectors = vec![make_sector(0, 100, vec![])];
        let mut glow = GlowT::new(0);
        glow.minlight = 50;
        glow.maxlight = 200;
        glow.direction = 1;

        t_glow(&mut glow, &mut sectors);

        // 100 + 8 = 108, which is < maxlight (200), so no reverse
        assert_eq!(sectors[0].lightlevel, 108);
        assert_eq!(glow.direction, 1);
    }

    #[test]
    fn test_glow_brightening_reverses_with_undo() {
        // Light at 192, max at 200. After add 8 → 200, which is >= maxlight.
        // Undo: 200 - 8 = 192. Direction reverses to -1.
        let mut sectors = vec![make_sector(0, 192, vec![])];
        let mut glow = GlowT::new(0);
        glow.minlight = 50;
        glow.maxlight = 200;
        glow.direction = 1;

        t_glow(&mut glow, &mut sectors);

        // Light should be undone to 192 (not set to maxlight 200!)
        assert_eq!(sectors[0].lightlevel, 192);
        assert_eq!(glow.direction, -1);
    }

    #[test]
    fn test_glow_brightening_reverses_past_max() {
        // Light at 196, max at 200. After add 8 → 204, which is >= maxlight.
        // Undo: 204 - 8 = 196. Direction reverses to -1.
        let mut sectors = vec![make_sector(0, 196, vec![])];
        let mut glow = GlowT::new(0);
        glow.minlight = 50;
        glow.maxlight = 200;
        glow.direction = 1;

        t_glow(&mut glow, &mut sectors);

        assert_eq!(sectors[0].lightlevel, 196);
        assert_eq!(glow.direction, -1);
    }

    // ---- EV_TurnTagLightsOff tests -----------------------------------------

    #[test]
    fn test_ev_turn_tag_lights_off_basic() {
        // Sector 0: tag=1, light=200, neighbor sector 1 with light=50
        let lines = vec![make_two_sided_line(1, 0, 1)];
        let mut sectors = vec![make_sector(1, 200, vec![0]), make_sector(0, 50, vec![])];

        let trigger = LineDef {
            tag: 1,
            ..LineDef::default()
        };

        ev_turn_tag_lights_off(&trigger, &mut sectors, &lines);

        // Sector 0 should be set to the minimum neighbor light (50)
        assert_eq!(sectors[0].lightlevel, 50);
        // Sector 1 is untagged, should be unchanged
        assert_eq!(sectors[1].lightlevel, 50);
    }

    #[test]
    fn test_ev_turn_tag_lights_off_no_neighbor() {
        // Sector 0: tag=1, light=200, no two-sided lines
        let lines = vec![LineDef {
            tag: 1,
            flags: 0, // One-sided
            ..LineDef::default()
        }];
        let mut sectors = vec![make_sector(1, 200, vec![0])];

        let trigger = LineDef {
            tag: 1,
            ..LineDef::default()
        };

        ev_turn_tag_lights_off(&trigger, &mut sectors, &lines);

        // No neighbors found, light stays at own level
        assert_eq!(sectors[0].lightlevel, 200);
    }

    // ---- EV_LightTurnOn tests ----------------------------------------------

    #[test]
    fn test_ev_light_turn_on_with_bright_value() {
        let lines = vec![LineDef::default()];
        let mut sectors = vec![make_sector(2, 50, vec![])];

        let trigger = LineDef {
            tag: 2,
            ..LineDef::default()
        };

        ev_light_turn_on(&trigger, 255, &mut sectors, &lines);

        assert_eq!(sectors[0].lightlevel, 255);
    }

    #[test]
    fn test_ev_light_turn_on_find_max_neighbor() {
        // Sector 0: tag=3, light=50, neighbor sector 1 with light=180
        let lines = vec![make_two_sided_line(3, 0, 1)];
        let mut sectors = vec![make_sector(3, 50, vec![0]), make_sector(0, 180, vec![])];

        let trigger = LineDef {
            tag: 3,
            ..LineDef::default()
        };

        // bright=0 → search neighbors for max
        ev_light_turn_on(&trigger, 0, &mut sectors, &lines);

        assert_eq!(sectors[0].lightlevel, 180);
    }

    #[test]
    fn test_ev_light_turn_on_bright_persists_across_sectors() {
        // Two tagged sectors. Sector 0 has neighbor with light 180.
        // Sector 2 has neighbor with light 100.
        // Due to C behavior, bright found for sector 0 (180) persists,
        // so sector 2 also gets 180 instead of 100.
        let lines = vec![
            make_two_sided_line(4, 0, 1), // line 0: sector 0 ↔ sector 1
            make_two_sided_line(4, 2, 3), // line 1: sector 2 ↔ sector 3
        ];
        let mut sectors = vec![
            make_sector(4, 50, vec![0]), // sector 0: tagged, neighbor is sector 1
            make_sector(0, 180, vec![]), // sector 1: neighbor, light=180
            make_sector(4, 50, vec![1]), // sector 2: tagged, neighbor is sector 3
            make_sector(0, 100, vec![]), // sector 3: neighbor, light=100
        ];

        let trigger = LineDef {
            tag: 4,
            ..LineDef::default()
        };

        ev_light_turn_on(&trigger, 0, &mut sectors, &lines);

        // Both tagged sectors should get 180 (the max from sector 0's neighbor)
        // due to the C-faithful bright variable persistence
        assert_eq!(sectors[0].lightlevel, 180);
        assert_eq!(sectors[2].lightlevel, 180);
    }

    // ---- Spawn function tests -----------------------------------------------

    #[test]
    fn test_spawn_fire_flicker() {
        let lines = vec![make_two_sided_line(0, 0, 1)];
        let mut sectors = vec![make_sector(0, 200, vec![0]), make_sector(0, 100, vec![])];
        let mut thinkers = ThinkerList::new();
        let mut storage: Vec<FireFlickerT> = Vec::new();

        // Sector 0 has special=1 before spawn
        sectors[0].special = 1;

        p_spawn_fire_flicker(0, &mut sectors, &lines, &mut thinkers, &mut storage);

        // Special should be cleared
        assert_eq!(sectors[0].special, 0);
        // Storage should have one entry
        assert_eq!(storage.len(), 1);
        assert_eq!(storage[0].maxlight, 200);
        // minlight = P_FindMinSurroundingLight(200) + 16 = 100 + 16 = 116
        assert_eq!(storage[0].minlight, 116);
        assert_eq!(storage[0].count, 4);
        assert_eq!(storage[0].thinker.function, ActionFn::FireFlicker);
    }

    #[test]
    fn test_spawn_light_flash() {
        let lines = vec![make_two_sided_line(0, 0, 1)];
        let mut sectors = vec![make_sector(0, 200, vec![0]), make_sector(0, 80, vec![])];
        let mut rng = DoomRandom::new();
        let mut thinkers = ThinkerList::new();
        let mut storage: Vec<LightFlashT> = Vec::new();

        sectors[0].special = 1;

        p_spawn_light_flash(
            0,
            &mut sectors,
            &lines,
            &mut rng,
            &mut thinkers,
            &mut storage,
        );

        assert_eq!(sectors[0].special, 0);
        assert_eq!(storage.len(), 1);
        assert_eq!(storage[0].maxlight, 200);
        assert_eq!(storage[0].minlight, 80);
        assert_eq!(storage[0].maxtime, 64);
        assert_eq!(storage[0].mintime, 7);
        assert!(storage[0].count >= 1 && storage[0].count <= 65);
        assert_eq!(storage[0].thinker.function, ActionFn::LightFlash);
    }

    #[test]
    fn test_spawn_strobe_flash_not_synced() {
        let lines = vec![make_two_sided_line(0, 0, 1)];
        let mut sectors = vec![make_sector(0, 200, vec![0]), make_sector(0, 60, vec![])];
        let mut rng = DoomRandom::new();
        let mut thinkers = ThinkerList::new();
        let mut storage: Vec<StrobeFlashT> = Vec::new();

        sectors[0].special = 1;

        p_spawn_strobe_flash(
            0,
            FASTDARK,
            false,
            &mut sectors,
            &lines,
            &mut rng,
            &mut thinkers,
            &mut storage,
        );

        assert_eq!(sectors[0].special, 0);
        assert_eq!(storage.len(), 1);
        assert_eq!(storage[0].maxlight, 200);
        assert_eq!(storage[0].minlight, 60);
        assert_eq!(storage[0].darktime, FASTDARK);
        assert_eq!(storage[0].brighttime, STROBEBRIGHT);
        // Not synced: count = (P_Random() & 7) + 1, range [1, 8]
        assert!(storage[0].count >= 1 && storage[0].count <= 8);
    }

    #[test]
    fn test_spawn_strobe_flash_synced() {
        let lines = vec![make_two_sided_line(0, 0, 1)];
        let mut sectors = vec![make_sector(0, 200, vec![0]), make_sector(0, 60, vec![])];
        let mut rng = DoomRandom::new();
        let mut thinkers = ThinkerList::new();
        let mut storage: Vec<StrobeFlashT> = Vec::new();

        p_spawn_strobe_flash(
            0,
            SLOWDARK,
            true,
            &mut sectors,
            &lines,
            &mut rng,
            &mut thinkers,
            &mut storage,
        );

        // Synced: count = 1
        assert_eq!(storage[0].count, 1);
    }

    #[test]
    fn test_spawn_strobe_flash_minlight_equals_maxlight() {
        // When no neighbor has lower light, minlight should be forced to 0
        let lines = vec![make_two_sided_line(0, 0, 1)];
        let mut sectors = vec![
            make_sector(0, 200, vec![0]),
            make_sector(0, 200, vec![]), // Same light level
        ];
        let mut rng = DoomRandom::new();
        let mut thinkers = ThinkerList::new();
        let mut storage: Vec<StrobeFlashT> = Vec::new();

        p_spawn_strobe_flash(
            0,
            SLOWDARK,
            true,
            &mut sectors,
            &lines,
            &mut rng,
            &mut thinkers,
            &mut storage,
        );

        // minlight forced to 0 because it equaled maxlight
        assert_eq!(storage[0].minlight, 0);
        assert_eq!(storage[0].maxlight, 200);
    }

    #[test]
    fn test_spawn_glowing_light() {
        let lines = vec![make_two_sided_line(0, 0, 1)];
        let mut sectors = vec![make_sector(0, 200, vec![0]), make_sector(0, 80, vec![])];
        let mut thinkers = ThinkerList::new();
        let mut storage: Vec<GlowT> = Vec::new();

        sectors[0].special = 1;

        p_spawn_glowing_light(0, &mut sectors, &lines, &mut thinkers, &mut storage);

        assert_eq!(sectors[0].special, 0);
        assert_eq!(storage.len(), 1);
        assert_eq!(storage[0].maxlight, 200);
        assert_eq!(storage[0].minlight, 80);
        assert_eq!(storage[0].direction, -1); // Starts darkening
        assert_eq!(storage[0].thinker.function, ActionFn::Glow);
    }

    // ---- EV_StartLightStrobing tests ----------------------------------------

    #[test]
    fn test_ev_start_light_strobing() {
        let lines = vec![make_two_sided_line(5, 0, 1)];
        let mut sectors = vec![make_sector(5, 200, vec![0]), make_sector(0, 80, vec![])];
        let mut rng = DoomRandom::new();
        let mut thinkers = ThinkerList::new();
        let mut strobes: Vec<StrobeFlashT> = Vec::new();

        let trigger = LineDef {
            tag: 5,
            ..LineDef::default()
        };

        ev_start_light_strobing(
            &trigger,
            &mut sectors,
            &lines,
            &mut rng,
            &mut thinkers,
            &mut strobes,
        );

        // Should have spawned one strobe for sector 0
        assert_eq!(strobes.len(), 1);
        assert_eq!(strobes[0].darktime, SLOWDARK);
        assert_eq!(sectors[0].special, 0);
    }

    #[test]
    fn test_ev_start_light_strobing_skips_active_special() {
        let lines = vec![make_two_sided_line(5, 0, 1)];
        let mut sectors = vec![make_sector(5, 200, vec![0]), make_sector(0, 80, vec![])];
        // Mark sector 0 as having an active special
        sectors[0].specialdata = Some(42);

        let mut rng = DoomRandom::new();
        let mut thinkers = ThinkerList::new();
        let mut strobes: Vec<StrobeFlashT> = Vec::new();

        let trigger = LineDef {
            tag: 5,
            ..LineDef::default()
        };

        ev_start_light_strobing(
            &trigger,
            &mut sectors,
            &lines,
            &mut rng,
            &mut thinkers,
            &mut strobes,
        );

        // Should NOT have spawned any strobes (sector has active special)
        assert_eq!(strobes.len(), 0);
    }
}
