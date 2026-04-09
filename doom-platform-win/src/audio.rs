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

//! SDL2 audio backend — SFX channel mixing and music playback.
//!
//! Translated from `linuxdoom-1.10/i_sound.c` and `sndserv/soundsrv.c`.
//! Replaces the Linux OSS `/dev/dsp` audio backend AND the standalone
//! `sndserv` process pipe model with SDL2 in-process callback-based audio.
//!
//! ## Audio Architecture
//! - 8 internal mixing channels for SFX (matching original NUM_CHANNELS=8)
//! - 16-bit signed stereo output at 11025 Hz (matching SAMPLERATE)
//! - 512-sample mixing buffer (matching SAMPLECOUNT)
//! - Volume lookup table: vol_lookup\[128×256\] for fast mixing
//! - Stereo separation via x² falloff (matching original i_sound.c:357-361)
//!
//! ## AAP Issue Resolution
//! - IR-02: Sound system reimplemented using SDL2 (no dependency on DOS sound library)
//! - IR-07: SNDINTR timer interrupt mode replaced by SDL2 audio callback

use std::sync::{Arc, Mutex};

use sdl2::audio::{AudioCallback, AudioDevice, AudioSpecDesired};
use tracing::{debug, error, info, warn};

use doom_core::info::sounds::{SfxEnum, NUMSFX};
use doom_core::traits::audio::AudioBackend;
use doom_core::types::doomdef::TICRATE;

// =============================================================================
// Audio constants — MUST MATCH ORIGINAL for behavioral parity
// =============================================================================

/// Number of mixing samples per callback (i_sound.c:93, soundsrv.h:38).
pub const SAMPLECOUNT: usize = 512;

/// Number of internal mixing channels (i_sound.c:94).
pub const NUM_CHANNELS: usize = 8;

/// Output sample rate in Hz (i_sound.c:99, soundsrv.h:40).
pub const SAMPLERATE: i32 = 11025;

/// Sample size in bytes: 16-bit = 2 (i_sound.c:100).
pub const SAMPLESIZE: usize = 2;

/// Multiply factor: 2 channels (stereo) × 2 bytes (16-bit) (i_sound.c:96).
pub const BUFMUL: usize = 4;

/// Mix buffer size: SAMPLECOUNT × BUFMUL (i_sound.c:97).
pub const MIXBUFFERSIZE: usize = SAMPLECOUNT * BUFMUL;

// =============================================================================
// Per-channel state — mirrors the parallel arrays in i_sound.c:116-151
// =============================================================================

/// State for a single mixing channel.
///
/// In the original C code, channel state was spread across parallel global
/// arrays: `channels[]`, `channelsend[]`, `channelstep[]`,
/// `channelstepremainder[]`, `channelstart[]`, `channelhandles[]`,
/// `channelids[]`, `channelleftvol_lookup[]`, `channelrightvol_lookup[]`.
/// Here we consolidate into a single struct per channel.
#[derive(Default)]
struct Channel {
    /// Raw unsigned 8-bit sample data for this channel.
    /// `None` means the channel is inactive/empty.
    data: Option<Vec<u8>>,
    /// Current read position in the sample data.
    position: usize,
    /// End position (padded length of sample data).
    end_position: usize,
    /// Pitch step amount in 16.16 fixed-point (i_sound.c:116).
    step: u32,
    /// 0.16 bit remainder of last step advance (i_sound.c:118).
    step_remainder: u32,
    /// Game tic when this channel started playing (i_sound.c:131).
    /// Used for oldest-channel eviction when all channels are busy.
    start_time: i32,
    /// Channel handle returned by `addsfx`, used for identification
    /// (i_sound.c:137).
    handle: i32,
    /// SFX id of the currently playing sound effect (i_sound.c:141).
    /// Used for duplicate detection (chainsaw, pistol, etc.).
    sfx_id: i32,
    /// Left volume index (0-127) into vol_lookup table (i_sound.c:150).
    left_vol: i32,
    /// Right volume index (0-127) into vol_lookup table (i_sound.c:151).
    right_vol: i32,
}

// =============================================================================
// Shared mixer state — accessed by both main thread and SDL2 audio callback
// =============================================================================

/// Shared state between the main game thread and the SDL2 audio callback
/// thread, protected by `Arc<Mutex<>>`.
///
/// The original C code used global variables for all of this state.
/// In Rust, we consolidate into a struct behind a mutex for thread safety.
struct MixerState {
    /// Eight mixing channels (i_sound.c:122-151).
    channels: [Channel; NUM_CHANNELS],
    /// Volume lookup table: vol_lookup\[128 × 256\] (i_sound.c:147).
    /// Maps (volume_level, unsigned_sample) → signed 16-bit amplitude.
    vol_lookup: Vec<i32>,
    /// Monotonically increasing handle counter (i_sound.c:271).
    /// Wraps around through 0 → 100 (matching original unsigned short behavior).
    handle_num: u16,
    /// Current game tic for timestamp tracking. Updated by `update_sound()`.
    gametic: i32,
}

// =============================================================================
// Volume lookup table generation — I_SetChannels() equivalent (i_sound.c:396-424)
// =============================================================================

/// Generate the volume lookup table that converts unsigned 8-bit samples
/// (0-255) into signed amplitudes scaled by volume level (0-127).
///
/// Reproduces the formula from i_sound.c:421-423 exactly:
/// ```text
/// vol_lookup[i*256+j] = (i*(j-128)*256)/127
/// ```
///
/// The table has 128 × 256 = 32,768 entries. Each entry maps a
/// (volume, sample) pair to a signed 16-bit-range amplitude value.
fn init_vol_lookup(vol_lookup: &mut Vec<i32>) {
    vol_lookup.resize(128 * 256, 0);
    for i in 0..128 {
        for j in 0..256 {
            // CRITICAL: This formula MUST be reproduced exactly for audio fidelity.
            // Original: vol_lookup[i*256+j] = (i*(j-128)*256)/127;
            // The (j-128) centers unsigned samples around zero.
            // The *256 scales to 16-bit range.
            // The /127 normalizes by max volume.
            vol_lookup[i * 256 + j] = (i as i32 * (j as i32 - 128) * 256) / 127;
        }
    }
}

/// Generate the pitch-to-step lookup table (i_sound.c:412-415).
///
/// Reproduces the formula from i_sound.c:415:
/// ```text
/// steptablemid[i] = (int)(pow(2.0, (i/64.0)) * 65536.0)
/// ```
/// where steptablemid = steptable + 128 (i.e., index 128 is normal pitch).
///
/// The table maps pitch values (0-255) to 16.16 fixed-point step amounts.
/// - Index 128 = normal pitch (step = 65536 = 1.0 in 16.16 fixed-point)
/// - Lower indices = lower pitch (slower playback)
/// - Higher indices = higher pitch (faster playback)
fn init_step_table(table: &mut [i32; 256]) {
    for (i, entry) in table.iter_mut().enumerate() {
        let exponent = (i as f64 - 128.0) / 64.0;
        *entry = (2.0_f64.powf(exponent) * 65536.0) as i32;
    }
}

// =============================================================================
// Sound loading — getsfx() equivalent (i_sound.c:185-251)
// =============================================================================

/// Process raw WAD sound lump data into a padded sample buffer.
///
/// Reproduces `getsfx()` from i_sound.c:185-251:
/// 1. Skips the 8-byte WAD sound header (format + samplerate + length)
/// 2. Pads the sample data to the next SAMPLECOUNT boundary
/// 3. Fills padding bytes with 128 (silence for unsigned 8-bit audio)
///
/// # Parameters
/// - `raw_lump`: Raw bytes from the WAD sound lump (including 8-byte header)
///
/// # Returns
/// Tuple of (padded sample data without header, padded length).
/// Returns `(vec![], 0)` if the lump is too short.
fn load_sfx(raw_lump: &[u8]) -> (Vec<u8>, usize) {
    // WAD sound lump header is 8 bytes:
    //   bytes 0-1: format number (3)
    //   bytes 2-3: sample rate
    //   bytes 4-7: number of samples
    if raw_lump.len() <= 8 {
        warn!("Sound lump too short ({} bytes), skipping", raw_lump.len());
        return (vec![], 0);
    }

    let raw_size = raw_lump.len() - 8;

    // Pad to SAMPLECOUNT boundary (i_sound.c:230).
    // paddedsize = ((size-8 + (SAMPLECOUNT-1)) / SAMPLECOUNT) * SAMPLECOUNT
    let padded_size = raw_size.div_ceil(SAMPLECOUNT) * SAMPLECOUNT;

    // Allocate padded buffer filled with 128 (silence for unsigned 8-bit audio).
    // Padding value 128 matches i_sound.c:241.
    let mut padded = vec![128u8; padded_size];

    // Copy raw sample data (skipping 8-byte header).
    let copy_len = raw_size.min(padded_size);
    padded[..copy_len].copy_from_slice(&raw_lump[8..8 + copy_len]);

    (padded, padded_size)
}

// =============================================================================
// Channel assignment — addsfx() equivalent (i_sound.c:264-381)
// =============================================================================

/// Add a sound effect to a mixing channel.
///
/// Reproduces `addsfx()` from i_sound.c:264-381:
/// 1. Chainsaw duplicate detection (i_sound.c:284-306)
/// 2. Channel selection: first empty, or evict oldest (i_sound.c:308-325)
/// 3. Stereo separation calculation (i_sound.c:350-373)
/// 4. Volume lookup table assignment
///
/// # Returns
/// Channel handle for identification.
fn add_sfx(
    state: &mut MixerState,
    sfx_id: i32,
    data: Vec<u8>,
    length: usize,
    volume: i32,
    separation: i32,
    step: u32,
    gametic: i32,
) -> i32 {
    // -------------------------------------------------------------------------
    // Chainsaw troubles (i_sound.c:283-306).
    // Play these sound effects only one at a time.
    // -------------------------------------------------------------------------
    if sfx_id == SfxEnum::sfx_sawup as i32
        || sfx_id == SfxEnum::sfx_sawidl as i32
        || sfx_id == SfxEnum::sfx_sawful as i32
        || sfx_id == SfxEnum::sfx_sawhit as i32
        || sfx_id == SfxEnum::sfx_stnmov as i32
        || sfx_id == SfxEnum::sfx_pistol as i32
    {
        for chan in state.channels.iter_mut() {
            if chan.data.is_some() && chan.sfx_id == sfx_id {
                // Reset — stop the duplicate.
                chan.data = None;
                // There will only be one duplicate (original: break).
                break;
            }
        }
    }

    // -------------------------------------------------------------------------
    // Find a channel: first empty, or evict oldest (i_sound.c:308-325).
    // -------------------------------------------------------------------------
    let mut slot = None;
    let mut oldest_time = gametic;
    let mut oldest_slot = 0;

    for (i, chan) in state.channels.iter().enumerate() {
        if chan.data.is_none() {
            // Found an empty channel — use it.
            slot = Some(i);
            break;
        }
        // Track the oldest active channel for eviction.
        if chan.start_time < oldest_time {
            oldest_time = chan.start_time;
            oldest_slot = i;
        }
    }

    // If no empty channel found, evict the oldest one (i_sound.c:322-325).
    let slot = slot.unwrap_or(oldest_slot);

    // -------------------------------------------------------------------------
    // Assign channel state (i_sound.c:329-348).
    // -------------------------------------------------------------------------
    let chan = &mut state.channels[slot];
    chan.data = Some(data);
    chan.position = 0;
    chan.end_position = length;
    chan.step = step;
    chan.step_remainder = 0;
    chan.start_time = gametic;

    // Handle counter: reset to 100 on wrap-around (i_sound.c:335-340).
    if state.handle_num == 0 {
        state.handle_num = 100;
    }
    let handle = state.handle_num as i32;
    state.handle_num = state.handle_num.wrapping_add(1);
    chan.handle = handle;

    // -------------------------------------------------------------------------
    // Stereo separation — x² falloff (i_sound.c:350-373).
    //
    // CRITICAL: This formula MUST match i_sound.c:357-361 exactly.
    //
    // Input separation range: 0-255 (128 = center).
    // After +=1: range becomes 1-256.
    // -------------------------------------------------------------------------
    let sep = separation + 1;

    // Left channel volume: attenuated by separation² (i_sound.c:357-358).
    let left_vol = volume - ((volume * sep * sep) >> 16);

    // Right channel: mirror separation (i_sound.c:359-361).
    let sep_right = sep - 257;
    let right_vol = volume - ((volume * sep_right * sep_right) >> 16);

    // Clamp volumes to valid range [0, 127] (i_sound.c:364-368).
    chan.left_vol = left_vol.clamp(0, 127);
    chan.right_vol = right_vol.clamp(0, 127);

    // Preserve SFX id for duplicate detection (i_sound.c:377).
    chan.sfx_id = sfx_id;

    debug!(
        "add_sfx: id={}, slot={}, handle={}, leftvol={}, rightvol={}",
        sfx_id, slot, handle, chan.left_vol, chan.right_vol
    );

    handle
}

// =============================================================================
// SDL2 audio callback — I_UpdateSound() equivalent (i_sound.c:539-654)
// =============================================================================

/// SDL2 audio callback that performs real-time SFX mixing.
///
/// This callback runs on a separate audio thread managed by SDL2.
/// It locks the shared `MixerState` and mixes all active channels
/// into the output buffer.
///
/// The mixing loop reproduces i_sound.c:576-608 exactly:
/// 1. Read unsigned 8-bit sample from each active channel
/// 2. Apply left/right volume lookup for each channel
/// 3. Accumulate into signed 32-bit accumulators (dl, dr)
/// 4. Advance channel position using 16.16 fixed-point stepping
/// 5. Clamp final mix to 16-bit range [-0x8000, 0x7FFF]
struct DoomAudioCallback {
    mixer_state: Arc<Mutex<MixerState>>,
}

impl AudioCallback for DoomAudioCallback {
    type Channel = i16;

    fn callback(&mut self, out: &mut [i16]) {
        let mut state = match self.mixer_state.lock() {
            Ok(s) => s,
            Err(poisoned) => {
                // If the mutex is poisoned, still try to recover.
                warn!("Audio mixer mutex was poisoned, recovering");
                poisoned.into_inner()
            }
        };

        // Destructure the MixerState to allow simultaneous mutable borrow
        // of channels and immutable borrow of vol_lookup. Rust's borrow
        // checker requires field-level splitting for this pattern.
        let MixerState {
            ref mut channels,
            ref vol_lookup,
            ..
        } = *state;

        let vol_len = vol_lookup.len();

        // Process interleaved stereo frames: [left, right, left, right, ...]
        // Each frame is 2 i16 values (one per physical channel).
        for frame in out.chunks_exact_mut(2) {
            let mut dl: i32 = 0; // Left accumulator (i_sound.c:562)
            let mut dr: i32 = 0; // Right accumulator (i_sound.c:563)

            // Mix all active channels (i_sound.c:576-608).
            for chan in channels.iter_mut() {
                if let Some(ref data) = chan.data {
                    if chan.position < chan.end_position {
                        // Read unsigned 8-bit sample (i_sound.c:583).
                        let sample = data[chan.position] as usize;

                        // Apply volume lookup (i_sound.c:588-589).
                        // channelleftvol_lookup[chan][sample]  →
                        //   vol_lookup[left_vol * 256 + sample]
                        // channelrightvol_lookup[chan][sample] →
                        //   vol_lookup[right_vol * 256 + sample]
                        let lv_idx = chan.left_vol as usize * 256 + sample;
                        let rv_idx = chan.right_vol as usize * 256 + sample;

                        // Bounds-safe access to vol_lookup.
                        if lv_idx < vol_len {
                            dl += vol_lookup[lv_idx];
                        }
                        if rv_idx < vol_len {
                            dr += vol_lookup[rv_idx];
                        }

                        // Advance position with 16.16 fixed-point stepping
                        // (i_sound.c:595-600).
                        chan.step_remainder += chan.step;
                        chan.position += (chan.step_remainder >> 16) as usize;
                        chan.step_remainder &= 0xFFFF; // Keep fractional part

                        // Check if channel has reached end of data
                        // (i_sound.c:602-603).
                        if chan.position >= chan.end_position {
                            chan.data = None;
                        }
                    } else {
                        // Position past end — deactivate channel.
                        chan.data = None;
                    }
                }
            }

            // Clamp to 16-bit signed range (i_sound.c:617-630).
            // Original: if (dl > 0x7fff) *leftout = 0x7fff;
            //           else if (dl < -0x8000) *leftout = -0x8000;
            frame[0] = dl.clamp(-0x8000, 0x7FFF) as i16;
            frame[1] = dr.clamp(-0x8000, 0x7FFF) as i16;
        }
    }
}

// =============================================================================
// SdlAudioBackend — the primary exported struct
// =============================================================================

/// SDL2-based audio backend implementing the `AudioBackend` trait.
///
/// Provides 8-channel SFX mixing via SDL2's callback-based audio device,
/// replacing both the Linux OSS `/dev/dsp` backend from `i_sound.c` and
/// the standalone `sndserv` process pipe model.
///
/// # Thread Safety
///
/// The internal mixer state is wrapped in `Arc<Mutex<>>` to allow
/// safe concurrent access from the main game thread (channel assignment)
/// and the SDL2 audio callback thread (sample mixing).
pub struct SdlAudioBackend {
    /// SDL2 audio subsystem handle for device creation.
    audio_subsystem: Option<sdl2::AudioSubsystem>,
    /// The SDL2 audio device (created in `init_sound()`).
    audio_device: Option<AudioDevice<DoomAudioCallback>>,
    /// Shared mixer state between main thread and audio callback.
    mixer_state: Arc<Mutex<MixerState>>,
    /// Cached sound effect data, indexed by SFX id (up to NUMSFX entries).
    /// Each entry is the processed (header-stripped, padded) sample data.
    sfx_data: Vec<Option<Vec<u8>>>,
    /// Padded lengths of loaded sound effects, indexed by SFX id.
    sfx_lengths: Vec<usize>,
    /// Pitch-to-step lookup table (256 entries, i_sound.c:108-113).
    steptable: [i32; 256],
    /// Local game tic counter, incremented each `update_sound()` call.
    /// Used for `sound_is_playing()` checks and music timing.
    gametic: i32,
    /// Music volume level (0-127), stored for music API compatibility.
    music_volume: i32,
    /// Whether the current music track should loop.
    looping: bool,
    /// Game tic when the current music track should stop
    /// (i_sound.c:835 — `musicdies = gametic + TICRATE*30`).
    musicdies: i32,
}

impl SdlAudioBackend {
    /// Create a new SDL2 audio backend.
    ///
    /// The audio device is NOT started here — call `init_sound()` to
    /// initialize and start audio playback.
    ///
    /// # Parameters
    /// - `audio_subsystem`: SDL2 audio subsystem obtained from `sdl2::init()`
    pub fn new(audio_subsystem: sdl2::AudioSubsystem) -> Self {
        info!("Creating SDL2 audio backend");

        // Initialize the volume lookup table (I_SetChannels equivalent).
        let mut vol_lookup = Vec::new();
        init_vol_lookup(&mut vol_lookup);

        // Initialize the pitch step table (i_sound.c:412-415).
        let mut steptable = [0i32; 256];
        init_step_table(&mut steptable);

        // Build the shared mixer state.
        let mixer_state = Arc::new(Mutex::new(MixerState {
            channels: std::array::from_fn(|_| Channel::default()),
            vol_lookup,
            handle_num: 0,
            gametic: 0,
        }));

        Self {
            audio_subsystem: Some(audio_subsystem),
            audio_device: None,
            mixer_state,
            sfx_data: vec![None; NUMSFX],
            sfx_lengths: vec![0; NUMSFX],
            steptable,
            gametic: 0,
            music_volume: 0,
            looping: false,
            musicdies: -1,
        }
    }

    /// Register a raw WAD sound lump for a given SFX id.
    ///
    /// Processes the raw lump data (strips 8-byte header, pads to
    /// SAMPLECOUNT boundary) and caches it for later playback via
    /// `start_sound()`.
    ///
    /// # Parameters
    /// - `id`: SFX identifier (index into the SfxEnum table, 0..NUMSFX-1)
    /// - `raw_lump`: Raw bytes from the WAD sound lump (including 8-byte header)
    pub fn cache_sound(&mut self, id: usize, raw_lump: &[u8]) {
        if id >= NUMSFX {
            warn!(
                "cache_sound: SFX id {} out of range (max {})",
                id,
                NUMSFX - 1
            );
            return;
        }

        let (data, length) = load_sfx(raw_lump);
        if length == 0 {
            warn!("cache_sound: SFX {} produced empty data", id);
            return;
        }

        debug!("cache_sound: SFX {} loaded, {} samples padded", id, length);
        self.sfx_data[id] = Some(data);
        self.sfx_lengths[id] = length;
    }
}

// =============================================================================
// AudioBackend trait implementation — all 17 methods
// =============================================================================

impl AudioBackend for SdlAudioBackend {
    /// Initialize the SDL2 audio device and start playback.
    ///
    /// Equivalent to `I_InitSound()` from i_sound.c:743-820.
    /// Configures the audio device with:
    /// - Sample rate: 11025 Hz (matching i_sound.c:779 and sndserv/linux.c:88)
    /// - Channels: 2 (stereo, matching linux.c:91)
    /// - Format: signed 16-bit (matching linux.c:94)
    /// - Buffer size: 512 samples (SAMPLECOUNT)
    fn init_sound(&mut self) {
        info!("I_InitSound: Initializing SDL2 audio device");

        let subsystem = match self.audio_subsystem.take() {
            Some(s) => s,
            None => {
                error!("I_InitSound: Audio subsystem not available");
                return;
            }
        };

        let desired_spec = AudioSpecDesired {
            freq: Some(SAMPLERATE),
            channels: Some(2), // Stereo output
            samples: Some(SAMPLECOUNT as u16),
        };

        let mixer_clone = Arc::clone(&self.mixer_state);

        match subsystem.open_playback(None, &desired_spec, |_spec| DoomAudioCallback {
            mixer_state: mixer_clone,
        }) {
            Ok(device) => {
                // Start audio playback (replaces OSS write loop).
                device.resume();
                info!(
                    "I_InitSound: SDL2 audio device opened — {}Hz, stereo, 16-bit, {} samples",
                    SAMPLERATE, SAMPLECOUNT
                );
                self.audio_device = Some(device);
            }
            Err(e) => {
                error!("I_InitSound: Failed to open SDL2 audio device: {}", e);
                // Store subsystem back for potential retry.
                self.audio_subsystem = Some(subsystem);
            }
        }
    }

    /// Shut down the audio device and release resources.
    ///
    /// Equivalent to `I_ShutdownSound()` from i_sound.c:695-739.
    fn shutdown_sound(&mut self) {
        info!("I_ShutdownSound: Shutting down SDL2 audio");
        if let Some(device) = self.audio_device.take() {
            device.pause();
            // Device is dropped here, closing the audio.
            drop(device);
        }
        info!("I_ShutdownSound: Audio device closed");
    }

    /// Start a sound effect on a mixing channel.
    ///
    /// Equivalent to `I_StartSound()` from i_sound.c:464-499.
    /// Looks up the pitch-adjusted step from the step table, clones
    /// the cached SFX data, and assigns it to a channel via `add_sfx()`.
    ///
    /// # Parameters
    /// - `id`: SFX identifier (index into SfxEnum table)
    /// - `vol`: Volume level (0-127)
    /// - `sep`: Stereo separation (0-255, 128=center)
    /// - `pitch`: Pitch adjustment index (0-255, 128=normal)
    /// - `priority`: Priority level (unused in original, matching i_sound.c:464)
    ///
    /// # Returns
    /// Channel handle (>= 0) on success, -1 if the sound cannot be started.
    fn start_sound(&mut self, id: i32, vol: i32, sep: i32, pitch: i32, _priority: i32) -> i32 {
        // Validate SFX id range.
        let sfx_idx = id as usize;
        if sfx_idx >= NUMSFX {
            warn!("I_StartSound: SFX id {} out of range", id);
            return -1;
        }

        // Get cached sound data. If not loaded yet, skip silently.
        let data = match self.sfx_data.get(sfx_idx) {
            Some(Some(d)) => d.clone(),
            _ => {
                debug!("I_StartSound: SFX {} not cached, skipping", id);
                return -1;
            }
        };

        let length = self.sfx_lengths[sfx_idx];
        if length == 0 {
            return -1;
        }

        // Look up pitch-adjusted step (i_sound.c:482-486).
        // Original: steptable[pitch] where pitch is clamped to [0, 255].
        let pitch_idx = (pitch.clamp(0, 255)) as usize;
        let step = self.steptable[pitch_idx] as u32;

        // Lock mixer state and assign channel.
        let mut state = match self.mixer_state.lock() {
            Ok(s) => s,
            Err(poisoned) => {
                warn!("Mixer state mutex poisoned in start_sound");
                poisoned.into_inner()
            }
        };

        add_sfx(&mut state, id, data, length, vol, sep, step, self.gametic)
    }

    /// Stop a sound by its channel handle.
    ///
    /// Equivalent to `I_StopSound()` from i_sound.c:505-513.
    /// Note: The original was marked as UNUSED but had a stub implementation
    /// that cleared the channel. We preserve this behavior.
    fn stop_sound(&mut self, handle: i32) {
        let mut state = match self.mixer_state.lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };

        for chan in state.channels.iter_mut() {
            if chan.handle == handle {
                chan.data = None;
                break;
            }
        }
    }

    /// Check if a sound is still playing.
    ///
    /// Equivalent to `I_SoundIsPlaying()` from i_sound.c:517-521.
    /// Original: `return gametic < handle;` — a crude check that considers
    /// a sound "playing" if the game tic counter hasn't exceeded the handle
    /// value. This is a placeholder heuristic from the original code,
    /// preserved for behavioral parity.
    fn sound_is_playing(&self, handle: i32) -> bool {
        // Matching i_sound.c:520 exactly: return gametic < handle;
        self.gametic < handle
    }

    /// Update sound state (called once per game tic).
    ///
    /// Equivalent to `I_UpdateSound()` from i_sound.c:539-654.
    /// In the SDL2 callback model, the actual mixing happens in the
    /// `DoomAudioCallback::callback()` method on the audio thread.
    /// This method only needs to update the game tic counter.
    fn update_sound(&mut self) {
        // Increment local gametic counter (approximates global gametic).
        self.gametic += 1;

        // Also update the gametic in mixer state for channel timestamping.
        if let Ok(mut state) = self.mixer_state.lock() {
            state.gametic = self.gametic;
        }
    }

    /// Submit the mixed audio buffer to the hardware.
    ///
    /// Equivalent to `I_SubmitSound()` from i_sound.c:660-672.
    /// In the SDL2 callback model, audio submission is handled automatically
    /// by SDL2's audio thread — no explicit write needed. The original code
    /// called `write(audio_fd, mixbuffer, ...)` to push data to OSS `/dev/dsp`.
    /// This is a no-op with SDL2.
    fn submit_sound(&mut self) {
        // No-op: SDL2 callback model handles audio submission automatically.
        // The DoomAudioCallback::callback() method fills the output buffer
        // directly when SDL2's audio thread requests it.
    }

    /// Update sound parameters for an active channel.
    ///
    /// Equivalent to `I_UpdateSoundParams()` from i_sound.c:675-688.
    /// Note: This was UNUSED in the original implementation (marked with
    /// "UNUSED" comment). Preserved as a no-op for behavioral parity.
    fn update_sound_params(&mut self, _handle: i32, _vol: i32, _sep: i32, _pitch: i32) {
        // UNUSED in original (i_sound.c:675-688).
        // The original code had empty parameter lists with comments indicating
        // the function was never called. Preserved as no-op.
    }

    // =========================================================================
    // Music API — dummy implementations matching i_sound.c:835-889
    //
    // The original Linux version had stub/dummy music implementations because
    // the music system relied on external MIDI playback which was not fully
    // integrated. We preserve this exact behavior.
    // =========================================================================

    /// Initialize the music subsystem.
    ///
    /// Equivalent to `I_InitMusic()` from i_sound.c:828-830.
    /// No-op in the original; preserved as no-op.
    fn init_music(&mut self) {
        info!("I_InitMusic: Music subsystem initialized (stub)");
    }

    /// Shut down the music subsystem.
    ///
    /// Equivalent to `I_ShutdownMusic()` from i_sound.c:832-834.
    /// No-op in the original; preserved as no-op.
    fn shutdown_music(&mut self) {
        info!("I_ShutdownMusic: Music subsystem shut down (stub)");
    }

    /// Set the music playback volume.
    ///
    /// Equivalent to `I_SetMusicVolume()` from i_sound.c:836-841.
    /// Stores the volume value for potential future use.
    fn set_music_volume(&mut self, volume: i32) {
        debug!("I_SetMusicVolume: volume={}", volume);
        self.music_volume = volume.clamp(0, 127);
    }

    /// Pause the currently playing music track.
    ///
    /// Equivalent to `I_PauseSong()` from i_sound.c:853-855.
    /// No-op in the original; preserved as no-op.
    fn pause_song(&mut self, _handle: i32) {
        debug!("I_PauseSong: (stub)");
    }

    /// Resume a paused music track.
    ///
    /// Equivalent to `I_ResumeSong()` from i_sound.c:857-859.
    /// No-op in the original; preserved as no-op.
    fn resume_song(&mut self, _handle: i32) {
        debug!("I_ResumeSong: (stub)");
    }

    /// Register a music lump for playback.
    ///
    /// Equivalent to `I_RegisterSong()` from i_sound.c:876-881.
    /// Returns 1 as a dummy handle, matching the original behavior.
    fn register_song(&mut self, _data: &[u8]) -> i32 {
        debug!("I_RegisterSong: returning handle 1 (stub)");
        1
    }

    /// Start playing a registered music track.
    ///
    /// Equivalent to `I_PlaySong()` from i_sound.c:842-846.
    /// Sets the music timeout timer: `musicdies = gametic + TICRATE * 30`.
    fn play_song(&mut self, _handle: i32, looping: bool) {
        debug!("I_PlaySong: looping={}", looping);
        self.looping = looping;
        // Original: musicdies = gametic + TICRATE * 30 (i_sound.c:845).
        // This sets a 30-second timeout for the music track.
        self.musicdies = self.gametic + TICRATE * 30;
    }

    /// Stop the currently playing music track.
    ///
    /// Equivalent to `I_StopSong()` from i_sound.c:861-867.
    /// Resets the looping flag and music timeout.
    fn stop_song(&mut self, _handle: i32) {
        debug!("I_StopSong: stopping music");
        self.looping = false;
        self.musicdies = -1;
    }

    /// Unregister a previously registered music track.
    ///
    /// Equivalent to `I_UnRegisterSong()` from i_sound.c:870-874.
    /// No-op in the original; preserved as no-op.
    fn unregister_song(&mut self, _handle: i32) {
        debug!("I_UnRegisterSong: (stub)");
    }
}
