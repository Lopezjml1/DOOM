// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 1993-1996 Id Software, Inc.
// Copyright (C) 2024 DOOM Rust Port Contributors

//! Light effect thinkers — flicker, flash, strobe, glow.
//!
//! Translated from linuxdoom-1.10/p_lights.c
//!
//! # Original C functions → Rust mapping
//!
//! | C function | Rust function | Description |
//! |---|---|---|
//! | `T_FireFlicker` | `t_fire_flicker` | Fire flicker light effect |
//! | `P_SpawnFireFlicker` | `p_spawn_fire_flicker` | Spawn fire flicker for sector |
//! | `T_LightFlash` | `t_light_flash` | Random light flash effect |
//! | `P_SpawnLightFlash` | `p_spawn_light_flash` | Spawn light flash for sector |
//! | `T_StrobeFlash` | `t_strobe_flash` | Periodic strobe flash effect |
//! | `P_SpawnStrobeFlash` | `p_spawn_strobe_flash` | Spawn strobe flash for sector |
//! | `EV_StartLightStrobing` | `ev_start_light_strobing` | Start strobing by line tag |
//! | `EV_TurnTagLightsOff` | `ev_turn_tag_lights_off` | Turn tagged lights off |
//! | `EV_LightTurnOn` | `ev_light_turn_on` | Turn tagged lights on |
//! | `T_Glow` | `t_glow` | Pulsing glow light effect |
//! | `P_SpawnGlowingLight` | `p_spawn_glowing_light` | Spawn glow for sector |

use crate::play::spec::{
    get_next_sector, p_find_min_surrounding_light, p_find_sector_from_line_tag, FireFlickerT,
    GlowT, LightFlashT, StrobeFlashT, GLOWSPEED, STROBEBRIGHT,
};
use crate::types::map_data::{LineDef, Sector};
use crate::util::random::DoomRandom;

// ============================================================================
// T_FireFlicker — Fire flicker light effect
// Translated from lines 35-65 of p_lights.c
// ============================================================================

/// Process one tic of fire flicker for a sector.
///
/// The fire flicker randomly adjusts light between `minlight` and
/// `maxlight`, changing every 4 tics with a random variation.
///
/// Returns the new count and light level to apply.
pub fn t_fire_flicker_step(
    count: i32,
    maxlight: i32,
    minlight: i32,
    rng: &mut DoomRandom,
) -> (i32, Option<i32>) {
    if count > 0 {
        return (count - 1, None);
    }
    // Reset count to 4 tics
    let amount = ((rng.p_random() as i32) & 3) * 16;
    let new_light = if maxlight - amount < minlight {
        minlight
    } else {
        maxlight - amount
    };
    (4, Some(new_light))
}

/// Create a new FireFlickerT for a sector.
///
/// Initializes maxlight from sector's current light level and minlight
/// from the minimum surrounding light level + 16.
pub fn make_fire_flicker(sector_idx: usize, sectors: &[Sector], lines: &[LineDef]) -> FireFlickerT {
    let sec = &sectors[sector_idx];
    let mut flicker = FireFlickerT::new(sector_idx);
    flicker.maxlight = sec.lightlevel as i32;
    flicker.minlight =
        p_find_min_surrounding_light(sector_idx, sec.lightlevel as i32, sectors, lines) + 16;
    flicker.count = 4;
    flicker
}

// ============================================================================
// T_LightFlash — Random light flash effect
// Translated from lines 70-100 of p_lights.c
// ============================================================================

/// Process one tic of random light flash for a sector.
///
/// Toggles between maxlight and minlight with random timing.
///
/// Returns the new count, new maxtime/mintime, and whether to set max or min light.
pub fn t_light_flash_step(
    count: i32,
    current_light: i32,
    maxlight: i32,
    minlight: i32,
    maxtime: i32,
    mintime: i32,
    rng: &mut DoomRandom,
) -> (i32, Option<i32>) {
    if count > 0 {
        return (count - 1, None);
    }
    if current_light == maxlight {
        let new_count = ((rng.p_random() as i32) & mintime) + 1;
        (new_count, Some(minlight))
    } else {
        let new_count = ((rng.p_random() as i32) & maxtime) + 1;
        (new_count, Some(maxlight))
    }
}

/// Create a new LightFlashT for a sector.
pub fn make_light_flash(
    sector_idx: usize,
    sectors: &[Sector],
    lines: &[LineDef],
    rng: &mut DoomRandom,
) -> LightFlashT {
    let sec = &sectors[sector_idx];
    let mut flash = LightFlashT::new(sector_idx);
    flash.maxlight = sec.lightlevel as i32;
    flash.minlight =
        p_find_min_surrounding_light(sector_idx, sec.lightlevel as i32, sectors, lines);
    flash.maxtime = 64;
    flash.mintime = 7;
    flash.count = (rng.p_random() as i32 & flash.maxtime) + 1;
    flash
}

// ============================================================================
// T_StrobeFlash — Periodic strobe flash effect
// Translated from lines 105-140 of p_lights.c
// ============================================================================

/// Process one tic of strobe flash for a sector.
///
/// Returns the new count and the light level to apply.
pub fn t_strobe_flash_step(
    count: i32,
    current_light: i32,
    maxlight: i32,
    minlight: i32,
    brighttime: i32,
    darktime: i32,
    _rng: &mut DoomRandom,
) -> (i32, Option<i32>) {
    if count > 0 {
        return (count - 1, None);
    }
    if current_light == minlight {
        (brighttime, Some(maxlight))
    } else {
        (darktime, Some(minlight))
    }
}

/// Create a new StrobeFlashT for a sector.
pub fn make_strobe_flash(
    sector_idx: usize,
    darktime: i32,
    in_sync: bool,
    sectors: &[Sector],
    lines: &[LineDef],
    rng: &mut DoomRandom,
) -> StrobeFlashT {
    let sec = &sectors[sector_idx];
    let mut strobe = StrobeFlashT::new(sector_idx);
    strobe.darktime = darktime;
    strobe.brighttime = STROBEBRIGHT;
    strobe.maxlight = sec.lightlevel as i32;
    strobe.minlight =
        p_find_min_surrounding_light(sector_idx, sec.lightlevel as i32, sectors, lines);
    if strobe.minlight == strobe.maxlight {
        strobe.minlight = 0;
    }
    if !in_sync {
        strobe.count = (rng.p_random() as i32 & 7) + 1;
    } else {
        strobe.count = 1;
    }
    strobe
}

// ============================================================================
// EV_StartLightStrobing — Start strobing by line tag
// Translated from lines 145-170 of p_lights.c
// ============================================================================

/// Find all sectors matching the trigger line's tag and prepare them
/// for strobe lighting.
///
/// Returns a list of sector indices that should have strobe flashes spawned.
pub fn find_sectors_for_strobing(
    line_idx: usize,
    lines: &[LineDef],
    sectors: &[Sector],
) -> Vec<usize> {
    let mut result = Vec::new();
    let mut secnum: i32 = -1;
    loop {
        secnum = p_find_sector_from_line_tag(&lines[line_idx], secnum, sectors);
        if secnum < 0 {
            break;
        }
        result.push(secnum as usize);
    }
    result
}

// ============================================================================
// EV_TurnTagLightsOff — Turn all lights off in tagged sectors
// Translated from lines 175-210 of p_lights.c
// ============================================================================

/// Turn lights off in all sectors matching the trigger line's tag.
///
/// Sets each tagged sector's light level to the minimum light level
/// found in surrounding sectors.
pub fn ev_turn_tag_lights_off(line_idx: usize, lines: &[LineDef], sectors: &mut [Sector]) {
    let tag = lines[line_idx].tag;

    for j in 0..sectors.len() {
        if sectors[j].tag == tag {
            let mut min = sectors[j].lightlevel;
            // Find minimum surrounding light
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
// EV_LightTurnOn — Turn lights on in tagged sectors
// Translated from lines 215-260 of p_lights.c
// ============================================================================

/// Turn lights on in all sectors matching the trigger line's tag.
///
/// If `bright` is 0, find the maximum neighboring light level.
/// Otherwise, set to `bright`.
pub fn ev_light_turn_on(line_idx: usize, bright: i32, lines: &[LineDef], sectors: &mut [Sector]) {
    let tag = lines[line_idx].tag;

    for i in 0..sectors.len() {
        if sectors[i].tag == tag {
            if bright == 0 {
                // Find maximum neighbor light
                let mut max_light: i32 = 0;
                let num_lines = sectors[i].lines.len();
                for k in 0..num_lines {
                    let line_k = sectors[i].lines[k];
                    let li = &lines[line_k];
                    if let Some(other_idx) = get_next_sector(li, i) {
                        let other_light = sectors[other_idx].lightlevel as i32;
                        if other_light > max_light {
                            max_light = other_light;
                        }
                    }
                }
                sectors[i].lightlevel = max_light as i16;
            } else {
                sectors[i].lightlevel = bright as i16;
            }
        }
    }
}

// ============================================================================
// T_Glow — Pulsing glow light effect
// Translated from lines 265-300 of p_lights.c
// ============================================================================

/// Process one tic of glow effect for a sector.
///
/// The glow smoothly oscillates the light level between `minlight` and
/// `maxlight` by incrementing/decrementing by `GLOWSPEED` each tic.
///
/// Returns the new light level and new direction.
pub fn t_glow_step(current_light: i32, minlight: i32, maxlight: i32, direction: i32) -> (i32, i32) {
    match direction {
        // Going down
        -1 => {
            let new_light = current_light - GLOWSPEED;
            if new_light <= minlight {
                (minlight, 1)
            } else {
                (new_light, -1)
            }
        }
        // Going up
        1 => {
            let new_light = current_light + GLOWSPEED;
            if new_light >= maxlight {
                (maxlight, -1)
            } else {
                (new_light, 1)
            }
        }
        _ => (current_light, direction),
    }
}

/// Create a new GlowT for a sector.
pub fn make_glow(sector_idx: usize, sectors: &[Sector], lines: &[LineDef]) -> GlowT {
    let sec = &sectors[sector_idx];
    let mut glow = GlowT::new(sector_idx);
    glow.minlight = p_find_min_surrounding_light(sector_idx, sec.lightlevel as i32, sectors, lines);
    glow.maxlight = sec.lightlevel as i32;
    glow.direction = -1;
    glow
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fire_flicker_countdown() {
        let mut rng = DoomRandom::new();
        let (count, light) = t_fire_flicker_step(3, 200, 100, &mut rng);
        assert_eq!(count, 2);
        assert!(light.is_none());
    }

    #[test]
    fn test_fire_flicker_trigger() {
        let mut rng = DoomRandom::new();
        let (count, light) = t_fire_flicker_step(0, 200, 100, &mut rng);
        assert_eq!(count, 4);
        assert!(light.is_some());
        let l = light.unwrap();
        assert!((100..=200).contains(&l));
    }

    #[test]
    fn test_light_flash_toggle_to_min() {
        let mut rng = DoomRandom::new();
        let (count, light) = t_light_flash_step(0, 200, 200, 100, 64, 7, &mut rng);
        // Current == max, so switch to min
        assert!(count >= 1);
        assert_eq!(light, Some(100));
    }

    #[test]
    fn test_light_flash_toggle_to_max() {
        let mut rng = DoomRandom::new();
        let (count, light) = t_light_flash_step(0, 100, 200, 100, 64, 7, &mut rng);
        // Current == min, so switch to max
        assert!(count >= 1);
        assert_eq!(light, Some(200));
    }

    #[test]
    fn test_strobe_flash_toggle() {
        let mut rng = DoomRandom::new();
        // At minlight, should go to maxlight
        let (count, light) = t_strobe_flash_step(0, 50, 200, 50, 5, 15, &mut rng);
        assert_eq!(count, 5); // brighttime
        assert_eq!(light, Some(200));

        // At maxlight, should go to minlight
        let (count2, light2) = t_strobe_flash_step(0, 200, 200, 50, 5, 15, &mut rng);
        assert_eq!(count2, 15); // darktime
        assert_eq!(light2, Some(50));
    }

    #[test]
    fn test_glow_down() {
        let (light, dir) = t_glow_step(100, 50, 200, -1);
        assert_eq!(light, 100 - GLOWSPEED);
        assert_eq!(dir, -1);
    }

    #[test]
    fn test_glow_reverses_at_min() {
        let (light, dir) = t_glow_step(50 + GLOWSPEED - 1, 50, 200, -1);
        assert_eq!(light, 50);
        assert_eq!(dir, 1);
    }

    #[test]
    fn test_glow_up() {
        let (light, dir) = t_glow_step(100, 50, 200, 1);
        assert_eq!(light, 100 + GLOWSPEED);
        assert_eq!(dir, 1);
    }

    #[test]
    fn test_glow_reverses_at_max() {
        let (light, dir) = t_glow_step(200 - GLOWSPEED + 1, 50, 200, 1);
        assert_eq!(light, 200);
        assert_eq!(dir, -1);
    }

    #[test]
    fn test_ev_turn_tag_lights_off() {
        // Sector 0: tag=1, light=200, neighbors sector 1 (light=50)
        let lines = vec![LineDef {
            tag: 1,
            flags: 0x04, // ML_TWOSIDED
            frontsector: Some(0),
            backsector: Some(1),
            ..LineDef::default()
        }];
        let mut sectors = vec![
            Sector {
                tag: 1,
                lightlevel: 200,
                lines: vec![0],
                ..Sector::default()
            },
            Sector {
                tag: 0,
                lightlevel: 50,
                lines: vec![],
                ..Sector::default()
            },
        ];
        // Create a trigger line with tag=1
        ev_turn_tag_lights_off(0, &lines, &mut sectors);
        assert_eq!(sectors[0].lightlevel, 50);
    }

    #[test]
    fn test_ev_light_turn_on_bright() {
        let lines = vec![LineDef {
            tag: 2,
            ..LineDef::default()
        }];
        let mut sectors = vec![Sector {
            tag: 2,
            lightlevel: 50,
            lines: vec![],
            ..Sector::default()
        }];
        ev_light_turn_on(0, 255, &lines, &mut sectors);
        assert_eq!(sectors[0].lightlevel, 255);
    }
}
