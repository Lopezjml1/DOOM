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

//! Translated from linuxdoom-1.10/d_net.h
//!
//! Network play related structures. Multiplayer is stubbed for single-player
//! only in this phase. These structures define the communication protocol
//! between DOOM peers and the driver program. Even in single-player mode,
//! the tic synchronization loop ([`TryRunTics`]) relies on [`DoomCom`] and
//! [`DoomData`] to manage the game timing pipeline.
//!
//! # Original C types
//!
//! | C type         | Rust type   | Purpose                           |
//! |----------------|-------------|-----------------------------------|
//! | `command_t`    | [`Command`] | Send/Get packet command codes     |
//! | `doomdata_t`   | [`DoomData`]| Network packet payload            |
//! | `doomcom_t`    | [`DoomCom`] | Full driver communication block   |
//!
//! # Constants
//!
//! | C define       | Rust constant       | Value        |
//! |----------------|---------------------|--------------|
//! | `DOOMCOM_ID`   | [`DOOMCOM_ID`]      | `0x12345678` |
//! | `MAXNETNODES`  | [`MAXNETNODES`]     | `8`          |
//! | `BACKUPTICS`   | [`BACKUPTICS`]      | `12`         |

use super::ticcmd::TicCmd;

// ---------------------------------------------------------------------------
// Constants — exact values from d_net.h lines 42-49
// ---------------------------------------------------------------------------

/// Magic number identifying a valid [`DoomCom`] structure.
///
/// The driver writes this value into [`DoomCom::id`] so that DOOM can verify
/// it is communicating through a properly initialized control block.
///
/// Original C: `#define DOOMCOM_ID 0x12345678l`
pub const DOOMCOM_ID: u32 = 0x12345678;

/// Maximum number of computers (nodes) in a networked game.
///
/// Each node can host one player. Node 0 is always the local console.
///
/// Original C: `#define MAXNETNODES 8`
pub const MAXNETNODES: usize = 8;

/// Number of backup tic commands stored in each network packet.
///
/// Each [`DoomData`] packet carries up to `BACKUPTICS` worth of [`TicCmd`]
/// entries for synchronization between the game timing loop and player input.
/// This value also sizes the command ring buffer used by [`TryRunTics`].
///
/// Original C: `#define BACKUPTICS 12`
pub const BACKUPTICS: usize = 12;

// ---------------------------------------------------------------------------
// Command enum — from d_net.h lines 51-56
// ---------------------------------------------------------------------------

/// Driver command codes exchanged between DOOM and the network driver.
///
/// DOOM writes one of these values into [`DoomCom::command`] and then signals
/// the driver (originally via a software interrupt on DOS, or a function call
/// on Unix/Windows). The driver inspects the command and either transmits or
/// receives a packet.
///
/// Original C: `command_t` enum (`CMD_SEND = 1`, `CMD_GET = 2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum Command {
    /// Send the packet stored in [`DoomCom::data`] to [`DoomCom::remotenode`].
    ///
    /// The driver reads [`DoomCom::datalength`] bytes from the embedded
    /// [`DoomData`] payload and transmits them to the specified remote node.
    Send = 1,

    /// Receive the next available packet into [`DoomCom::data`].
    ///
    /// If a packet is available the driver fills [`DoomCom::data`] and sets
    /// [`DoomCom::remotenode`] to the sender's node index. If no packet is
    /// available, [`DoomCom::remotenode`] is set to `-1`.
    Get = 2,
}

// ---------------------------------------------------------------------------
// DoomData — from d_net.h lines 62-74
// ---------------------------------------------------------------------------

/// Network packet payload.
///
/// Each packet carries a batch of [`TicCmd`] entries produced by one player.
/// The receiver feeds these commands into its local simulation to keep all
/// peers in lock-step.
///
/// # Fields
///
/// * `checksum` — CRC-style checksum; the high bit doubles as a retransmit
///   request flag (`NCMD_RETRANSMIT`).
/// * `retransmitfrom` — When the retransmit flag is set in `checksum`, this
///   field indicates the starting tic number to retransmit from.
/// * `starttic` — The first game tic represented by `cmds[0]`.
/// * `player` — The originating player index.
/// * `numtics` — How many entries in `cmds` are valid (1..=[`BACKUPTICS`]).
/// * `cmds` — Array of [`TicCmd`] entries, one per tic.
///
/// Original C: `doomdata_t`
#[derive(Debug, Clone)]
pub struct DoomData {
    /// Packet checksum. High bit is a retransmit request flag.
    pub checksum: u32,

    /// Starting tic to retransmit from. Only meaningful when the high bit
    /// of [`checksum`](Self::checksum) is set (`NCMD_RETRANSMIT`).
    pub retransmitfrom: u8,

    /// First game tic number covered by this packet.
    pub starttic: u8,

    /// Originating player index (0-based).
    pub player: u8,

    /// Number of valid [`TicCmd`] entries in [`cmds`](Self::cmds).
    pub numtics: u8,

    /// Per-tic input commands. Up to [`BACKUPTICS`] entries; the first
    /// [`numtics`](Self::numtics) entries are valid.
    pub cmds: [TicCmd; BACKUPTICS],
}

impl Default for DoomData {
    /// Returns a zeroed `DoomData` with all [`TicCmd`] entries defaulted.
    fn default() -> Self {
        Self {
            checksum: 0,
            retransmitfrom: 0,
            starttic: 0,
            player: 0,
            numtics: 0,
            cmds: [TicCmd::default(); BACKUPTICS],
        }
    }
}

// ---------------------------------------------------------------------------
// DoomCom — from d_net.h lines 79-127
// ---------------------------------------------------------------------------

/// Full communication block shared between DOOM and the network driver.
///
/// In the original DOS version, DOOM and the network driver shared a single
/// instance of this structure at a well-known memory address. DOOM would fill
/// in a [`Command`] and signal the driver via a software interrupt. On
/// Unix/Windows the driver is called directly, but the structure contract is
/// preserved for compatibility.
///
/// The structure contains both per-session configuration (node count, game
/// settings) and the mutable packet buffer ([`DoomData`]).
///
/// # Sections
///
/// 1. **Header** — magic ID, interrupt number, command/remote node.
/// 2. **Common info** — settings shared by all nodes (ticdup, deathmatch,
///    episode, map, skill, etc.).
/// 3. **Local info** — this node's console player, number of players.
/// 4. **3-display mode** — historical leftover for multi-screen setups.
/// 5. **Packet payload** — embedded [`DoomData`].
///
/// Original C: `doomcom_t`
#[derive(Debug, Clone)]
pub struct DoomCom {
    /// Magic identifier — should equal [`DOOMCOM_ID`] (`0x12345678`).
    ///
    /// The game checks this field at startup to verify that the driver has
    /// properly initialized the communication block.
    ///
    /// Mapped from C `long` (32-bit signed integer on the original platforms).
    pub id: i32,

    /// Software interrupt number used to invoke the driver (DOS legacy).
    ///
    /// Not used on modern platforms but preserved for structural compatibility.
    /// Mapped from C `short`.
    pub intnum: i16,

    /// Current command: [`Command::Send`] or [`Command::Get`].
    ///
    /// DOOM writes this field before signaling the driver. Mapped from C
    /// `short`.
    pub command: i16,

    /// Destination node for [`Command::Send`]; source node set by driver on
    /// [`Command::Get`]. A value of `-1` after a Get indicates no packet was
    /// available. Mapped from C `short`.
    pub remotenode: i16,

    /// Number of bytes of [`data`](Self::data) to transmit on a Send.
    /// Mapped from C `short`.
    pub datalength: i16,

    // -- Info common to all nodes --
    /// Total number of nodes in the game. Console (local player) is always
    /// node 0. Mapped from C `short`.
    pub numnodes: i16,

    /// Tic duplication factor. `1` = no duplication; `2`–`5` = duplicate each
    /// tic that many times (used to reduce bandwidth on slow networks).
    /// Mapped from C `short`.
    pub ticdup: i16,

    /// Extra tics flag. When `1`, a backup tic is included in every packet.
    /// Mapped from C `short`.
    pub extratics: i16,

    /// Deathmatch flag. `1` = deathmatch mode enabled. Mapped from C `short`.
    pub deathmatch: i16,

    /// Save-game slot flag. `-1` = start a new game; `0`–`5` = load the
    /// indicated save slot. Mapped from C `short`.
    pub savegame: i16,

    /// Episode number (`1`–`3` for DOOM, `1` for DOOM II).
    /// Mapped from C `short`.
    pub episode: i16,

    /// Map number (`1`–`9` for DOOM, `1`–`32` for DOOM II).
    /// Mapped from C `short`.
    pub map: i16,

    /// Skill level (`1`–`5`, corresponding to the five difficulty settings).
    /// Mapped from C `short`.
    pub skill: i16,

    // -- Info specific to this node --
    /// Local player index (0-based). Mapped from C `short`.
    pub consoleplayer: i16,

    /// Total number of players in the game. Mapped from C `short`.
    pub numplayers: i16,

    // -- 3-display mode (historical) --
    /// Angle offset for 3-display mode: `1` = left screen, `0` = center,
    /// `-1` = right screen. Probably not operational anymore.
    /// Mapped from C `short`.
    pub angleoffset: i16,

    /// Drone flag. `1` = this node is a passive viewer (drone), not an active
    /// player. Mapped from C `short`.
    pub drone: i16,

    // -- Packet payload --
    /// The embedded packet data to be sent or received.
    pub data: DoomData,
}

impl Default for DoomCom {
    /// Returns a `DoomCom` with [`id`](Self::id) set to [`DOOMCOM_ID`] and
    /// all other fields zeroed / defaulted.
    fn default() -> Self {
        Self {
            id: DOOMCOM_ID as i32,
            intnum: 0,
            command: 0,
            remotenode: 0,
            datalength: 0,
            numnodes: 0,
            ticdup: 0,
            extratics: 0,
            deathmatch: 0,
            savegame: 0,
            episode: 0,
            map: 0,
            skill: 0,
            consoleplayer: 0,
            numplayers: 0,
            angleoffset: 0,
            drone: 0,
            data: DoomData::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Constant value tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_doomcom_id_value() {
        assert_eq!(DOOMCOM_ID, 0x12345678);
    }

    #[test]
    fn test_maxnetnodes_value() {
        assert_eq!(MAXNETNODES, 8);
    }

    #[test]
    fn test_backuptics_value() {
        assert_eq!(BACKUPTICS, 12);
    }

    // -----------------------------------------------------------------------
    // Command enum tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_command_send_discriminant() {
        assert_eq!(Command::Send as i16, 1);
    }

    #[test]
    fn test_command_get_discriminant() {
        assert_eq!(Command::Get as i16, 2);
    }

    #[test]
    fn test_command_clone_copy() {
        let cmd = Command::Send;
        let copied = cmd;
        assert_eq!(cmd, copied);
    }

    #[test]
    fn test_command_debug_format() {
        let debug_str = format!("{:?}", Command::Send);
        assert!(debug_str.contains("Send"));
        let debug_str = format!("{:?}", Command::Get);
        assert!(debug_str.contains("Get"));
    }

    #[test]
    fn test_command_ne() {
        assert_ne!(Command::Send, Command::Get);
    }

    // -----------------------------------------------------------------------
    // DoomData tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_doomdata_default_checksum_zero() {
        let dd = DoomData::default();
        assert_eq!(dd.checksum, 0);
    }

    #[test]
    fn test_doomdata_default_retransmitfrom_zero() {
        let dd = DoomData::default();
        assert_eq!(dd.retransmitfrom, 0);
    }

    #[test]
    fn test_doomdata_default_starttic_zero() {
        let dd = DoomData::default();
        assert_eq!(dd.starttic, 0);
    }

    #[test]
    fn test_doomdata_default_player_zero() {
        let dd = DoomData::default();
        assert_eq!(dd.player, 0);
    }

    #[test]
    fn test_doomdata_default_numtics_zero() {
        let dd = DoomData::default();
        assert_eq!(dd.numtics, 0);
    }

    #[test]
    fn test_doomdata_default_cmds_length() {
        let dd = DoomData::default();
        assert_eq!(dd.cmds.len(), BACKUPTICS);
    }

    #[test]
    fn test_doomdata_default_cmds_all_zeroed() {
        let dd = DoomData::default();
        let default_cmd = TicCmd::default();
        for cmd in &dd.cmds {
            assert_eq!(*cmd, default_cmd);
        }
    }

    #[test]
    fn test_doomdata_clone() {
        let dd = DoomData {
            checksum: 0xDEADBEEF,
            player: 3,
            numtics: 5,
            ..Default::default()
        };
        let cloned = dd.clone();
        assert_eq!(cloned.checksum, 0xDEADBEEF);
        assert_eq!(cloned.player, 3);
        assert_eq!(cloned.numtics, 5);
    }

    #[test]
    fn test_doomdata_debug_format() {
        let dd = DoomData::default();
        let debug_str = format!("{:?}", dd);
        assert!(debug_str.contains("DoomData"));
        assert!(debug_str.contains("checksum"));
        assert!(debug_str.contains("cmds"));
    }

    // -----------------------------------------------------------------------
    // DoomCom tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_doomcom_default_id() {
        let dc = DoomCom::default();
        assert_eq!(dc.id, DOOMCOM_ID as i32);
        assert_eq!(dc.id, 0x12345678_i32);
    }

    #[test]
    fn test_doomcom_default_intnum_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.intnum, 0);
    }

    #[test]
    fn test_doomcom_default_command_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.command, 0);
    }

    #[test]
    fn test_doomcom_default_remotenode_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.remotenode, 0);
    }

    #[test]
    fn test_doomcom_default_datalength_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.datalength, 0);
    }

    #[test]
    fn test_doomcom_default_numnodes_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.numnodes, 0);
    }

    #[test]
    fn test_doomcom_default_ticdup_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.ticdup, 0);
    }

    #[test]
    fn test_doomcom_default_extratics_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.extratics, 0);
    }

    #[test]
    fn test_doomcom_default_deathmatch_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.deathmatch, 0);
    }

    #[test]
    fn test_doomcom_default_savegame_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.savegame, 0);
    }

    #[test]
    fn test_doomcom_default_episode_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.episode, 0);
    }

    #[test]
    fn test_doomcom_default_map_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.map, 0);
    }

    #[test]
    fn test_doomcom_default_skill_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.skill, 0);
    }

    #[test]
    fn test_doomcom_default_consoleplayer_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.consoleplayer, 0);
    }

    #[test]
    fn test_doomcom_default_numplayers_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.numplayers, 0);
    }

    #[test]
    fn test_doomcom_default_angleoffset_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.angleoffset, 0);
    }

    #[test]
    fn test_doomcom_default_drone_zero() {
        let dc = DoomCom::default();
        assert_eq!(dc.drone, 0);
    }

    #[test]
    fn test_doomcom_default_data_is_default() {
        let dc = DoomCom::default();
        assert_eq!(dc.data.checksum, 0);
        assert_eq!(dc.data.numtics, 0);
        assert_eq!(dc.data.cmds.len(), BACKUPTICS);
    }

    #[test]
    fn test_doomcom_clone() {
        let dc = DoomCom {
            numnodes: 4,
            numplayers: 4,
            deathmatch: 1,
            episode: 1,
            map: 7,
            skill: 3,
            ..Default::default()
        };
        let cloned = dc.clone();
        assert_eq!(cloned.id, DOOMCOM_ID as i32);
        assert_eq!(cloned.numnodes, 4);
        assert_eq!(cloned.numplayers, 4);
        assert_eq!(cloned.deathmatch, 1);
        assert_eq!(cloned.episode, 1);
        assert_eq!(cloned.map, 7);
        assert_eq!(cloned.skill, 3);
    }

    #[test]
    fn test_doomcom_debug_format() {
        let dc = DoomCom::default();
        let debug_str = format!("{:?}", dc);
        assert!(debug_str.contains("DoomCom"));
        assert!(debug_str.contains("id"));
        assert!(debug_str.contains("consoleplayer"));
        assert!(debug_str.contains("data"));
    }

    // -----------------------------------------------------------------------
    // Cross-structure integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_doomcom_embedded_data_modification() {
        let mut dc = DoomCom::default();
        dc.data.checksum = 0xCAFEBABE;
        dc.data.player = 2;
        dc.data.numtics = 3;
        dc.data.starttic = 100;
        dc.data.cmds[0].forwardmove = 50;
        dc.data.cmds[1].angleturn = -1000;
        assert_eq!(dc.data.checksum, 0xCAFEBABE);
        assert_eq!(dc.data.player, 2);
        assert_eq!(dc.data.numtics, 3);
        assert_eq!(dc.data.starttic, 100);
        assert_eq!(dc.data.cmds[0].forwardmove, 50);
        assert_eq!(dc.data.cmds[1].angleturn, -1000);
    }

    #[test]
    fn test_doomcom_id_matches_constant() {
        let dc = DoomCom::default();
        assert_eq!(dc.id as u32, DOOMCOM_ID);
    }

    #[test]
    fn test_doomdata_retransmit_flag_in_checksum() {
        // High bit set indicates retransmit request
        let dd = DoomData {
            checksum: 0x80000000,
            retransmitfrom: 5,
            ..Default::default()
        };
        assert!(dd.checksum & 0x80000000 != 0);
        assert_eq!(dd.retransmitfrom, 5);
    }

    #[test]
    fn test_command_values_match_original_c() {
        // CMD_SEND = 1, CMD_GET = 2 in the original C source
        assert_eq!(Command::Send as i16, 1_i16);
        assert_eq!(Command::Get as i16, 2_i16);
    }
}
