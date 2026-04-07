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

//! AudioBackend trait definition — the sound system platform abstraction.
//!
//! Translated from `linuxdoom-1.10/i_sound.h`. Defines the interface for
//! sound effect playback, music playback, and audio subsystem lifecycle.
//!
//! The original C code declared these as free functions in `i_sound.h`:
//! - SFX: `I_InitSound`, `I_ShutdownSound`, `I_StartSound`, `I_StopSound`,
//!   `I_SoundIsPlaying`, `I_UpdateSound`, `I_UpdateSoundParams`, `I_SubmitSound`
//! - Music: `I_InitMusic`, `I_ShutdownMusic`, `I_SetMusicVolume`, `I_PauseSong`,
//!   `I_ResumeSong`, `I_RegisterSong`, `I_PlaySong`, `I_StopSong`, `I_UnRegisterSong`
//!
//! The Linux implementation in `i_sound.c` used either:
//! - OSS `/dev/dsp` direct audio output (when `SNDINTR` was defined)
//! - External `sndserver` process communicated via pipe (`SNDSERV` mode, default)
//!
//! Both are replaced by SDL2 in-process audio in `doom-platform-win`.
//!
//! ## Functions NOT included in this trait (with justification)
//!
//! - `I_SetChannels()` (`i_sound.h:56`) — Internal initialization detail, not a
//!   platform abstraction method. Channel setup is an implementation detail of
//!   the `AudioBackend`.
//! - `I_GetSfxLumpNum(sfxinfo_t*)` (`i_sound.h:59`) — Looks up SFX lump names
//!   in the WAD. This is game logic (belongs in `s_sound`/`info`), not a platform
//!   function. The original just calls `W_GetNumForName()` which is WAD access,
//!   not audio.

/// Platform-independent audio backend interface.
///
/// Abstracts sound effect playback, music playback, and audio subsystem
/// lifecycle management. Implementations provide platform-specific audio
/// output (e.g., SDL2 on Windows 11).
///
/// All game logic in `doom-core` calls audio through this trait, never
/// through platform-specific APIs directly. This is the Rust equivalent
/// of the C function declarations in `linuxdoom-1.10/i_sound.h`.
///
/// # Design Notes
///
/// - Uses `&self` for read-only queries (`sound_is_playing`) and `&mut self`
///   for all mutating operations.
/// - `register_song` takes `&[u8]` (borrowed byte slice) instead of C's
///   `void *data` — safe, no ownership transfer needed.
/// - `play_song` uses `bool` for `looping` instead of C's `int looping` —
///   more idiomatic Rust.
/// - All parameter types are primitives (`i32`, `bool`, `&[u8]`) — no
///   platform-specific types in signatures.
/// - The trait is object-safe (no generic methods, no unsized `Self` in
///   return position) to enable dynamic dispatch via `dyn AudioBackend`.
/// - `submit_sound` is included for behavioral parity even though it may
///   be a no-op with SDL2's callback-based audio model.
pub trait AudioBackend {
    // ===== Sound System Lifecycle =====

    /// Initialize the sound subsystem.
    ///
    /// Equivalent of `I_InitSound()` from `i_sound.h:41`.
    /// Replaces Linux OSS `/dev/dsp` initialization or SNDSERV pipe setup.
    ///
    /// Called once at program startup to set up audio devices, allocate
    /// mixing buffers, and prepare sound channel state. The original C
    /// implementation opened `/dev/dsp` with the following parameters:
    /// - Sample rate: 11025 Hz
    /// - Sample size: 16-bit
    /// - Channels: 2 (stereo)
    /// - Mix buffer: 512 samples × 4 bytes = 2048 bytes
    fn init_sound(&mut self);

    /// Shut down the sound subsystem and release all resources.
    ///
    /// Equivalent of `I_ShutdownSound()` from `i_sound.h:48`.
    /// Called at program termination to close audio devices, free mixing
    /// buffers, and stop any active sound channels or music playback.
    fn shutdown_sound(&mut self);

    // ===== SFX Playback =====

    /// Start a sound effect in a mixing channel.
    ///
    /// Equivalent of `I_StartSound(id, vol, sep, pitch, priority)` from
    /// `i_sound.h:63-69`.
    ///
    /// Finds an available (or lowest-priority) mixing channel, loads the
    /// sound data, and begins playback with the specified parameters.
    ///
    /// # Parameters
    /// - `id`: Sound effect identifier (SFX lump index or enum value).
    ///   Corresponds to the `sfxenum_t` values from `sounds.h`.
    /// - `vol`: Volume level (0–127, as per `s_sound.c` mixing conventions).
    ///   127 is maximum volume.
    /// - `sep`: Stereo separation (0 = full right, 128 = center, 255 = full
    ///   left, as per `s_sound.c`). The original engine uses this for
    ///   positional audio based on the listener's angle to the sound source.
    /// - `pitch`: Pitch adjustment (normal = 128, range varies per sound).
    ///   Values above 128 increase pitch, below 128 decrease it.
    /// - `priority`: Sound priority for channel allocation. Higher values
    ///   have higher priority and can preempt lower-priority sounds when
    ///   all channels are occupied.
    ///
    /// # Returns
    /// A handle (channel number) for subsequent `stop_sound`,
    /// `sound_is_playing`, and `update_sound_params` operations.
    /// The original C function returns `int` — the channel handle.
    fn start_sound(&mut self, id: i32, vol: i32, sep: i32, pitch: i32, priority: i32) -> i32;

    /// Stop a sound channel immediately.
    ///
    /// Equivalent of `I_StopSound(handle)` from `i_sound.h:73`.
    /// Silences the specified channel and marks it as available for reuse.
    ///
    /// # Parameters
    /// - `handle`: Channel handle returned by `start_sound`.
    fn stop_sound(&mut self, handle: i32);

    /// Check if a sound channel is still playing.
    ///
    /// Equivalent of `I_SoundIsPlaying(handle)` from `i_sound.h:78`.
    /// Called by `S_*()` functions in the high-level sound code to determine
    /// whether a channel is still actively producing audio output.
    ///
    /// The original C function returns 0 if no longer playing, 1 if playing.
    /// In Rust we use `bool` for clarity.
    ///
    /// # Parameters
    /// - `handle`: Channel handle returned by `start_sound`.
    ///
    /// # Returns
    /// `true` if the channel is still actively playing audio, `false` otherwise.
    fn sound_is_playing(&self, handle: i32) -> bool;

    /// Update the sound output buffer — mix all active channels.
    ///
    /// Equivalent of `I_UpdateSound()` from `i_sound.h:44`.
    /// Called once per game tic (35 times per second) to mix all active
    /// sound channels into the stereo output buffer. The original C
    /// implementation performed 16-bit stereo mixing with volume and
    /// separation applied per-channel.
    ///
    /// On SDL2's callback-based audio model, this may prepare the next
    /// buffer of mixed samples for the audio callback to consume.
    fn update_sound(&mut self);

    /// Submit the mixed sound buffer to the audio device.
    ///
    /// Equivalent of `I_SubmitSound()` from `i_sound.h:45`.
    /// On the original Linux implementation, this wrote the mixed buffer
    /// to `/dev/dsp`. On modern audio APIs (SDL2 callback model), this
    /// may be a no-op since audio is consumed by the device callback
    /// asynchronously.
    ///
    /// Included for behavioral parity with the original engine's audio
    /// pipeline: `I_UpdateSound()` → `I_SubmitSound()`.
    fn submit_sound(&mut self);

    /// Update volume, stereo separation, and pitch of a currently playing sound.
    ///
    /// Equivalent of `I_UpdateSoundParams(handle, vol, sep, pitch)` from
    /// `i_sound.h:82-87`.
    ///
    /// Called by the high-level sound code (`s_sound.c`) when the listener
    /// or sound source moves, requiring real-time parameter adjustment of
    /// an active channel.
    ///
    /// # Parameters
    /// - `handle`: Channel handle returned by `start_sound`.
    /// - `vol`: Updated volume (0–127).
    /// - `sep`: Updated stereo separation (0 = right, 128 = center, 255 = left).
    /// - `pitch`: Updated pitch adjustment (normal = 128).
    fn update_sound_params(&mut self, handle: i32, vol: i32, sep: i32, pitch: i32);

    // ===== Music Playback =====

    /// Initialize the music subsystem.
    ///
    /// Equivalent of `I_InitMusic()` from `i_sound.h:93`.
    /// Called once at startup to prepare the music playback engine.
    /// The original DOOM music format is MUS (a compact MIDI variant).
    fn init_music(&mut self);

    /// Shut down the music subsystem and release all resources.
    ///
    /// Equivalent of `I_ShutdownMusic()` from `i_sound.h:94`.
    /// Called at program termination to stop any playing music,
    /// unregister all songs, and free music-related resources.
    fn shutdown_music(&mut self);

    /// Set the music volume.
    ///
    /// Equivalent of `I_SetMusicVolume(volume)` from `i_sound.h:96`.
    /// Called when the user adjusts the music volume slider in the
    /// options menu.
    ///
    /// # Parameters
    /// - `volume`: Music volume level (0–127, as per `s_sound.c` conventions).
    ///   0 is silence, 127 is maximum volume.
    fn set_music_volume(&mut self, volume: i32);

    /// Pause the currently playing song.
    ///
    /// Equivalent of `I_PauseSong(handle)` from `i_sound.h:98`.
    /// Called when the game is paused (e.g., pressing Pause key or
    /// opening the menu in single-player). Music resumes from the
    /// same position when `resume_song` is called.
    ///
    /// # Parameters
    /// - `handle`: Music handle returned by `register_song`.
    fn pause_song(&mut self, handle: i32);

    /// Resume a paused song from where it was paused.
    ///
    /// Equivalent of `I_ResumeSong(handle)` from `i_sound.h:99`.
    /// Called when the game is unpaused after a `pause_song` call.
    ///
    /// # Parameters
    /// - `handle`: Music handle returned by `register_song`.
    fn resume_song(&mut self, handle: i32);

    /// Register song data and return a handle for playback.
    ///
    /// Equivalent of `I_RegisterSong(data)` from `i_sound.h:101`.
    /// Accepts raw music lump bytes from the WAD file and prepares
    /// them for playback. The data is typically in MUS format (DOOM's
    /// compact MIDI representation) and may need conversion to standard
    /// MIDI for the platform's music synthesizer.
    ///
    /// # Parameters
    /// - `data`: Raw music lump bytes from the WAD. Ownership is not
    ///   transferred — the implementation must copy any data it needs
    ///   to retain beyond the lifetime of this borrow.
    ///
    /// # Returns
    /// A music handle (integer identifier) used for subsequent
    /// `play_song`, `pause_song`, `resume_song`, `stop_song`, and
    /// `unregister_song` operations.
    fn register_song(&mut self, data: &[u8]) -> i32;

    /// Start playing a registered song, optionally looping.
    ///
    /// Equivalent of `I_PlaySong(handle, looping)` from `i_sound.h:106-109`.
    /// The original C comment reads: "plays a song, and when the song is
    /// done, starts playing it again in an endless loop."
    ///
    /// # Parameters
    /// - `handle`: Music handle returned by `register_song`.
    /// - `looping`: When `true`, the song restarts automatically when it
    ///   reaches the end. When `false`, playback stops at the end.
    fn play_song(&mut self, handle: i32, looping: bool);

    /// Stop the currently playing song.
    ///
    /// Equivalent of `I_StopSong(handle)` from `i_sound.h:111`.
    /// The original C comment notes this "stops a song over 3 seconds,"
    /// though actual behavior varies by implementation.
    ///
    /// # Parameters
    /// - `handle`: Music handle returned by `register_song`.
    fn stop_song(&mut self, handle: i32);

    /// Unregister a song and free its resources.
    ///
    /// Equivalent of `I_UnRegisterSong(handle)` from `i_sound.h:113`.
    /// Releases all memory and resources associated with a previously
    /// registered song. The handle becomes invalid after this call.
    ///
    /// # Parameters
    /// - `handle`: Music handle returned by `register_song`.
    fn unregister_song(&mut self, handle: i32);
}
