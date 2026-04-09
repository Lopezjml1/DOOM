//! DOOM network game communication and tic synchronization.
//!
//! Translated from linuxdoom-1.10/d_net.c and linuxdoom-1.10/d_net.h
//!
//! In this phase, networking is stubbed for single-player only.
//! The tic synchronization logic is preserved for demo playback compatibility.
//!
//! The tic sync functions (`net_update` and `try_run_tics`) are the master
//! timing controllers for the DOOM game loop. `net_update` builds tic commands
//! (ticcmds) from player input at the correct rate, and `try_run_tics` runs the
//! game simulation for the correct number of tics each frame.
//!
//! # Architecture
//!
//! To decouple this module from subsystem types (VideoState, AudioBackend, etc.)
//! that live outside its direct dependency set, game operations are abstracted
//! through the [`NetCallbacks`] trait. The binary crate (doom-bin) implements
//! this trait to bridge all subsystems together.
//!
//! Copyright (C) 1993-1996 by id Software, Inc.
//! Copyright (C) 2024 Rust DOOM Contributors
//!
//! This program is free software; you can redistribute it and/or modify
//! it under the terms of the GNU General Public License as published by
//! the Free Software Foundation; either version 2 of the License, or
//! (at your option) any later version.

use tracing::{debug, info};

use crate::types::doomdef::{KEY_ESCAPE, MAXPLAYERS, TICRATE, VERSION};
use crate::types::event::{Event, EventType, BT_SPECIAL};
use crate::types::net::{Command, DoomCom, DoomData, BACKUPTICS, DOOMCOM_ID, MAXNETNODES};
use crate::types::ticcmd::TicCmd;

// Note: The following are depended upon transitively through NetCallbacks:
// - crate::traits::platform::PlatformHost (get_time, start_tic, error)
// - crate::game::game_ctrl::{GameCtrl, g_build_ticcmd, g_ticker, g_responder}
// - crate::game::game_main::{GameMain, d_process_events, d_do_advance_demo}
// - crate::ui::menu::{MenuState, m_ticker}

// ---------------------------------------------------------------------------
// Constants from d_net.c lines 37-58
// ---------------------------------------------------------------------------

/// Network command flag: exit game (d_net.c line 37).
const NCMD_EXIT: u32 = 0x8000_0000;

/// Network command flag: request retransmission (d_net.c line 38).
const NCMD_RETRANSMIT: u32 = 0x4000_0000;

/// Network command flag: setup packet (d_net.c line 39).
const NCMD_SETUP: u32 = 0x2000_0000;

/// Network command flag: kill game (d_net.c line 40).
const NCMD_KILL: u32 = 0x1000_0000;

/// Mask for checksum bits in network commands (d_net.c line 41).
/// Used in `netbuffer_checksum` and packet validation.
#[allow(dead_code)]
const NCMD_CHECKSUM: u32 = 0x0FFF_FFFF;

/// Resend count threshold before packet is considered lost (d_net.c line 57).
#[allow(dead_code)]
const RESENDCOUNT: i32 = 10;

/// Drone player flag — indicates this node is a spectator (d_net.c line 58).
#[allow(dead_code)]
const PL_DRONE: u8 = 0x80;

// ---------------------------------------------------------------------------
// NetCallbacks trait — bridges game_net to external subsystems
// ---------------------------------------------------------------------------

/// Callback trait for game subsystem operations needed by the network/tic
/// synchronization layer.
///
/// This trait decouples `game_net` from the concrete subsystem types
/// (`VideoState`, `AudioBackend`, `Renderer`, etc.) that are needed by
/// event processing and the game ticker. The binary crate (or integration
/// tests) implement this trait to wire all subsystems together.
///
/// # Contract
///
/// Implementations must guarantee:
/// - `get_time()` returns monotonically non-decreasing tic counts at 35/sec
/// - `poll_and_build_ticcmd()` polls input, processes events, and builds one
///   ticcmd from the resulting key state
/// - `game_ticker()` advances the game simulation by exactly one tic
/// - `menu_ticker()` advances the menu skull animation by one tic
/// - `do_advance_demo()` cycles to the next demo in the attract sequence
/// - `error()` never returns (divergent)
pub trait NetCallbacks {
    /// Returns current time in game tics (35 per second).
    ///
    /// Replaces C `I_GetTime()` from `i_system.h`.
    fn get_time(&self) -> i32;

    /// Polls platform input, processes all pending events, and builds
    /// a tic command from the resulting input state.
    ///
    /// This single callback replaces the C sequence:
    /// ```c
    /// I_StartTic();
    /// D_ProcessEvents();
    /// G_BuildTiccmd(&localcmds[maketic % BACKUPTICS]);
    /// ```
    ///
    /// The built ticcmd must be written into `cmd`.
    fn poll_and_build_ticcmd(&mut self, cmd: &mut TicCmd);

    /// Runs the game ticker for one simulation tic.
    ///
    /// Replaces C `G_Ticker()` from `g_game.h`.
    fn game_ticker(&mut self);

    /// Runs the menu ticker for one animation tic.
    ///
    /// Replaces C `M_Ticker()` from `m_menu.h`.
    fn menu_ticker(&mut self);

    /// Returns `true` if the attract-mode demo advance is pending.
    ///
    /// Replaces C `extern boolean advancedemo` from `d_main.c`.
    /// The value is checked inside the tic execution loop in `try_run_tics`.
    fn is_advance_demo(&self) -> bool;

    /// Advances to the next demo in the attract sequence.
    ///
    /// Replaces C `D_DoAdvanceDemo()` from `d_main.h`.
    /// Implementations should clear the advance-demo flag after processing.
    fn do_advance_demo(&mut self);

    /// Reports a fatal error and terminates the process.
    ///
    /// Replaces C `I_Error()` from `i_system.h`.
    fn error(&self, msg: &str) -> !;
}

// ---------------------------------------------------------------------------
// NetState — all former d_net.c global variables
// ---------------------------------------------------------------------------

/// Network state: all mutable state for the tic synchronization and
/// (stubbed) network protocol layer.
///
/// In the original C code, these were file-scope global variables in
/// `d_net.c`. In the Rust port they are consolidated into this struct
/// to eliminate `static mut` usage and enable the borrow checker to
/// enforce safe state access.
///
/// # Single-player behaviour
///
/// When `doomcom.numnodes == 1` (single-player), the network send/receive
/// paths are no-ops. Only the local rebound packet mechanism is active,
/// which feeds the local player's ticcmds back to `try_run_tics`.
pub struct NetState {
    // -- Core communication block (d_net.c line 44) --
    /// Network communication control block. Contains protocol fields,
    /// player counts, and the embedded `DoomData` payload.
    /// Replaces C `doomcom_t* doomcom`.
    pub doomcom: DoomCom,

    // -- Tic command buffers (d_net.c lines 60-63) --
    /// Local player's tic commands, indexed by `maketic % BACKUPTICS`.
    /// Replaces C `ticcmd_t localcmds[BACKUPTICS]`.
    pub localcmds: [TicCmd; BACKUPTICS],

    /// Per-player tic command buffers received from the network.
    /// `netcmds[player][tic % BACKUPTICS]`.
    /// Replaces C `ticcmd_t netcmds[MAXPLAYERS][BACKUPTICS]`.
    pub netcmds: [[TicCmd; BACKUPTICS]; MAXPLAYERS],

    // -- Per-node tracking (d_net.c lines 65-69) --
    /// Highest tic number received from each node.
    /// Replaces C `int nettics[MAXNETNODES]`.
    pub nettics: [i32; MAXNETNODES],

    /// Whether each network node is currently in the game.
    /// Replaces C `boolean nodeingame[MAXNETNODES]`.
    pub nodeingame: [bool; MAXNETNODES],

    /// Whether we need to request retransmission from each node.
    /// Replaces C `boolean remoteresend[MAXNETNODES]`.
    pub remoteresend: [bool; MAXNETNODES],

    /// Tic number we need retransmitted from each node.
    /// Replaces C `int resendto[MAXNETNODES]`.
    pub resendto: [i32; MAXNETNODES],

    /// Countdown for resend requests per node.
    /// Replaces C `int resendcount[MAXNETNODES]`.
    pub resendcount: [i32; MAXNETNODES],

    /// Maps player index → network node index.
    /// Replaces C `int nodeforplayer[MAXPLAYERS]`.
    pub nodeforplayer: [i32; MAXPLAYERS],

    // -- Tic timing (d_net.c lines 71-75) --
    /// Next tic number to be built by the local player.
    /// Replaces C `int maketic`.
    pub maketic: i32,

    /// Last tic number received from the network (for display lag calc).
    /// Replaces C `int lastnettic`.
    pub lastnettic: i32,

    /// Number of tics to skip (for network sync catch-up).
    /// Replaces C `int skiptics`.
    pub skiptics: i32,

    /// Tic duplication factor (1 = no duplication, 2+ = duplicate each input).
    /// Replaces C `int ticdup`.
    pub ticdup: i32,

    /// Maximum number of tics to send ahead in network mode.
    /// `BACKUPTICS / (2 * ticdup) - 1`.
    /// Replaces C `int maxsend`.
    pub maxsend: i32,

    // -- Rebound packet for local node (d_net.c lines 82-83) --
    /// Whether a rebound packet is pending for the local node.
    /// Replaces C `boolean reboundpacket`.
    pub reboundpacket: bool,

    /// Storage for the rebound packet data.
    /// Replaces C `doomdata_t reboundstore`.
    pub reboundstore: DoomData,

    // -- Frame timing for adaptive frame skipping (d_net.c lines 629-633) --
    /// Current game time in tics (aligned to ticdup).
    /// Replaces C `int gametime`.
    pub gametime: i32,

    /// Ring buffer of recent frame tic counts for frame adaptation.
    /// Replaces C `int frametics[4]`.
    pub frametics: [i32; 4],

    /// Current index into `frametics` ring buffer.
    /// Replaces C `int frameon`.
    pub frameon: i32,

    /// Frame skip decisions derived from frame adaptation.
    /// Replaces C `int frameskip[4]`.
    pub frameskip: [i32; 4],

    /// Previous value of nettics for frame adaptation comparison.
    /// Replaces C `int oldnettics`.
    pub oldnettics: i32,

    // -- Internal: entertic tracker for TryRunTics (was static local) --
    /// Previous enter-tic value for `try_run_tics` (was `static int
    /// oldentertics` in C). Moved to struct to avoid `static mut`.
    oldentertics: i32,
}

impl NetState {
    /// Creates a new `NetState` with all fields zeroed/defaulted.
    ///
    /// This is equivalent to the zero-initialization of the C global
    /// variables at program startup.
    pub fn new() -> Self {
        Self {
            doomcom: DoomCom::default(),
            localcmds: [TicCmd::new(); BACKUPTICS],
            netcmds: [[TicCmd::new(); BACKUPTICS]; MAXPLAYERS],
            nettics: [0; MAXNETNODES],
            nodeingame: [false; MAXNETNODES],
            remoteresend: [false; MAXNETNODES],
            resendto: [0; MAXNETNODES],
            resendcount: [0; MAXNETNODES],
            nodeforplayer: [0; MAXPLAYERS],
            maketic: 0,
            lastnettic: 0,
            skiptics: 0,
            ticdup: 1,
            maxsend: (BACKUPTICS as i32) / 2 - 1,
            reboundpacket: false,
            reboundstore: DoomData::default(),
            gametime: 0,
            frametics: [0; 4],
            frameon: 0,
            frameskip: [0; 4],
            oldnettics: 0,
            oldentertics: 0,
        }
    }
}

impl Default for NetState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helper functions (translated from d_net.c lines 90-186)
// ---------------------------------------------------------------------------

/// Calculates the byte size of the network buffer payload.
///
/// Translated from `NetbufferSize()` — d_net.c lines 90-93.
/// Returns the size of the `DoomData` header plus `numtics` ticcmds.
fn netbuffer_size(numtics: usize) -> usize {
    // DoomData header: checksum(4) + retransmitfrom(1) + starttic(1) +
    //                  player(1) + numtics(1) = 8 bytes
    // Plus numtics * sizeof(TicCmd)
    let header_size = 8usize;
    header_size + numtics * core::mem::size_of::<TicCmd>()
}

/// Computes a checksum for the current network buffer contents.
///
/// Translated from `NetbufferChecksum()` — d_net.c lines 98-115.
///
/// Under `#ifdef NORMALUNIX` the original always returned 0 due to byte
/// order problems on Linux. Since Windows x86_64 is little-endian (matching
/// the original DOS target), we implement the actual checksum.
///
/// However, for maximum compatibility with the single-player stub, we
/// return 0 (matching the NORMALUNIX path).
fn netbuffer_checksum(_data: &DoomData) -> u32 {
    // d_net.c line 99: #ifdef NORMALUNIX → return 0
    // We follow the NORMALUNIX convention for now (byte-order safe).
    0
}

/// Expands a truncated (8-bit) tic number back to a full tic number
/// by combining it with the current `maketic` high bits.
///
/// Translated from `ExpandTics()` — d_net.c lines 120-135.
///
/// The network protocol transmits only the low 8 bits of tic numbers
/// to save bandwidth. This function reconstructs the full value.
fn expand_tics(maketic: i32, low: i32) -> i32 {
    let delta = low.wrapping_sub(maketic & 0xff);

    if delta >= 64 {
        // Wrapped backwards — the tic is from the previous 256-tic window
        (maketic & !0xff) + low - 256
    } else if delta <= -64 {
        // Wrapped forwards — the tic is from the next 256-tic window
        (maketic & !0xff) + low + 256
    } else {
        // Normal case — same 256-tic window
        (maketic & !0xff) + low
    }
}

/// Sends a network packet to the specified node.
///
/// Translated from `HSendPacket()` — d_net.c lines 142-186.
///
/// For node 0 (the local node), the packet is stored as a "rebound packet"
/// instead of being sent over the network. For all other nodes in a netgame,
/// the packet would be transmitted via `I_NetCmd()` (stubbed in single-player).
fn h_send_packet(net: &mut NetState, node: i32, flags: u32, demoplayback: bool, netgame: bool) {
    let numtics = net.doomcom.data.numtics as usize;

    // d_net.c line 145: Apply checksum and flags
    net.doomcom.data.checksum = netbuffer_checksum(&net.doomcom.data) | flags;

    if node == 0 {
        // d_net.c lines 149-153: Local node — store as rebound packet
        net.reboundstore = net.doomcom.data.clone();
        net.reboundpacket = true;
        return;
    }

    if demoplayback {
        return;
    }

    if !netgame {
        tracing::error!("HSendPacket: attempted to transmit in non-netgame mode");
        return;
    }

    // d_net.c lines 162-166: Set up doomcom for transmission
    net.doomcom.command = Command::Send as i16;
    net.doomcom.remotenode = node as i16;
    net.doomcom.datalength = netbuffer_size(numtics) as i16;

    tracing::debug!(
        "HSendPacket: node={} flags={:#010x} tics={}",
        node,
        flags,
        numtics
    );

    // d_net.c line 186: I_NetCmd() — STUB: not implemented for single-player
    // In a networked game, this would call through to the platform network layer.
}

/// Attempts to receive a network packet.
///
/// Translated from `HGetPacket()` — d_net.c lines 192-253.
///
/// First checks for a pending rebound packet (from the local node).
/// In network games, would also call `I_NetCmd()` to receive from
/// remote nodes (stubbed in single-player).
///
/// Returns `true` if a packet was received and is ready in `doomcom.data`,
/// `false` if no packet is available.
fn h_get_packet(net: &mut NetState, demoplayback: bool, netgame: bool) -> bool {
    // d_net.c lines 194-202: Check for rebound packet first
    if net.reboundpacket {
        net.doomcom.data = net.reboundstore.clone();
        net.doomcom.remotenode = 0;
        net.reboundpacket = false;
        return true;
    }

    if !netgame {
        return false;
    }

    if demoplayback {
        return false;
    }

    // d_net.c lines 213-253: I_NetCmd(CMD_GET) — STUB for single-player
    // Would receive a packet from the network and validate doomcom_id.
    net.doomcom.command = Command::Get as i16;

    // In a real networked game, I_NetCmd() would fill doomcom with received data.
    // For single-player, no remote packets are ever available.
    false
}

/// Processes all available incoming network packets.
///
/// Translated from `GetPackets()` — d_net.c lines 261-358.
///
/// For each received packet, validates the contents and copies ticcmds
/// from the packet into the `netcmds` buffer for the appropriate player(s).
/// Handles retransmission requests, exit notifications, and out-of-order
/// packet detection.
///
/// In single-player mode, this processes only local rebound packets,
/// which effectively copies `localcmds` into `netcmds[consoleplayer]`.
fn get_packets(
    net: &mut NetState,
    playeringame: &[bool; MAXPLAYERS],
    _consoleplayer: usize,
    demoplayback: bool,
    netgame: bool,
) {
    loop {
        if !h_get_packet(net, demoplayback, netgame) {
            break;
        }

        let netbuf_checksum = net.doomcom.data.checksum;

        // d_net.c line 265-269: Check for special command flags
        if netbuf_checksum & NCMD_SETUP != 0 {
            continue; // Setup packets are only for D_ArbitrateNetStart
        }

        if netbuf_checksum & NCMD_KILL != 0 {
            tracing::error!("Killed by network driver");
            // In a full implementation this would call I_Error. For single-player
            // rebound packets this flag is never set.
            continue;
        }

        let nodenum = net.doomcom.remotenode as usize;
        if nodenum >= MAXNETNODES {
            tracing::warn!("GetPackets: invalid node number {}", nodenum);
            continue;
        }

        // d_net.c lines 274-280: Handle exit notification
        if netbuf_checksum & NCMD_EXIT != 0 {
            if !net.nodeingame[nodenum] {
                continue;
            }
            net.nodeingame[nodenum] = false;
            // Mark all players on this node as not in game
            for i in 0..MAXPLAYERS {
                if net.nodeforplayer[i] == nodenum as i32 {
                    tracing::info!("Player {} left the game", i);
                }
            }
            continue;
        }

        // d_net.c lines 286-291: Handle retransmission request
        if netbuf_checksum & NCMD_RETRANSMIT != 0 {
            let resendto_tic = expand_tics(net.maketic, net.doomcom.data.retransmitfrom as i32);
            tracing::debug!(
                "GetPackets: retransmit request from node {} for tic {}",
                nodenum,
                resendto_tic
            );
            net.resendto[nodenum] = resendto_tic;
            net.resendcount[nodenum] = RESENDCOUNT;
        }

        // d_net.c line 295: Expand the start tic from 8-bit to full
        let realstart = expand_tics(net.maketic, net.doomcom.data.starttic as i32);
        let realend = realstart + net.doomcom.data.numtics as i32;
        let netnode = nodenum;

        // d_net.c lines 300-315: Check for out-of-order or duplicate packets
        if realend <= net.nettics[netnode] {
            // Already have these tics — skip
            continue;
        }
        if realstart > net.nettics[netnode] {
            // Missed some tics — request retransmission
            tracing::debug!(
                "GetPackets: missed tics {}-{} from node {}",
                net.nettics[netnode],
                realstart - 1,
                netnode
            );
            net.remoteresend[netnode] = true;
            continue;
        }

        // d_net.c lines 320-355: Copy ticcmds from packet into netcmds buffer
        net.remoteresend[netnode] = false;

        // Determine which player this packet is from
        let player = net.doomcom.data.player as usize;
        if player >= MAXPLAYERS || !playeringame[player] {
            tracing::warn!(
                "GetPackets: invalid player {} from node {}",
                player,
                netnode
            );
            continue;
        }

        // Copy new tics from the packet into the netcmds buffer
        let start_offset = (net.nettics[netnode] - realstart) as usize;
        let end_offset = net.doomcom.data.numtics as usize;

        for i in start_offset..end_offset {
            let tic_idx = (net.nettics[netnode] as usize + i - start_offset) % BACKUPTICS;
            if i < BACKUPTICS {
                net.netcmds[player][tic_idx] = net.doomcom.data.cmds[i];
            }
        }

        // Update the highest tic number received from this node
        net.nettics[netnode] = realend;
    }
}

// ---------------------------------------------------------------------------
// net_update — master tic command builder
// ---------------------------------------------------------------------------

/// Builds new tic commands for the local player and listens for network
/// packets.
///
/// Translated from `NetUpdate()` — d_net.c lines 368-446.
///
/// This is called at the top of each frame by `try_run_tics` and may be
/// called multiple times per frame during the tic wait loop. It:
///
/// 1. Computes how many new tics have elapsed since the last call.
/// 2. For each new tic, invokes `cb.poll_and_build_ticcmd()` to poll input,
///    process events, and build a `TicCmd`.
/// 3. Sends the new ticcmds to network nodes (stubbed in single-player).
/// 4. Listens for incoming packets via `get_packets`.
///
/// # Parameters
///
/// - `net`: Network state (ticcmd buffers, timing, etc.).
/// - `gametic`: Current game tic from `GameCtrl.gametic`.
/// - `consoleplayer`: Local player index from `GameCtrl.consoleplayer`.
/// - `displayplayer`: Display player index from `GameCtrl.displayplayer`.
/// - `demoplayback`: Whether demo playback is active (`GameCtrl.demoplayback`).
/// - `netgame`: Whether this is a network game (`GameCtrl.netgame`).
/// - `singletics`: Debug single-tic mode flag (`GameMain.singletics`).
/// - `playeringame`: Which players are active (`GameCtrl.playeringame`).
/// - `paused`: Whether the game is paused (`GameCtrl.paused`).
/// - `sendpause`: Whether a pause command is pending (`GameCtrl.sendpause`).
/// - `cb`: Callbacks for platform timing and game operations.
pub fn net_update(
    net: &mut NetState,
    gametic: i32,
    consoleplayer: usize,
    _displayplayer: usize,
    demoplayback: bool,
    netgame: bool,
    singletics: bool,
    playeringame: &[bool; MAXPLAYERS],
    _paused: bool,
    _sendpause: bool,
    cb: &mut dyn NetCallbacks,
) {
    // d_net.c line 378: Get current time, divided by ticdup
    let nowtime = cb.get_time() / net.ticdup;
    let mut newtics = nowtime - net.gametime;

    // d_net.c line 381: Clamp to non-negative
    if newtics <= 0 {
        // No new tics — jump to listen section
        // d_net.c: "goto listen"
        get_packets(net, playeringame, consoleplayer, demoplayback, netgame);
        return;
    }

    net.gametime = nowtime;

    // d_net.c lines 384-393: Handle skiptics (network sync catch-up)
    if net.skiptics <= newtics {
        newtics -= net.skiptics;
        net.skiptics = 0;
    } else {
        net.skiptics -= newtics;
        newtics = 0;
    }

    // d_net.c line 396: Set player in network buffer
    net.doomcom.data.player = consoleplayer as u8;

    // d_net.c lines 400-414: Build new ticcmds for console player
    let gameticdiv = gametic / net.ticdup;

    for _i in 0..newtics {
        // d_net.c line 404: Check if we're too far ahead
        if net.maketic - gameticdiv >= (BACKUPTICS as i32) / 2 - 1 {
            break;
        }

        // d_net.c lines 407-409: Poll input, process events, build ticcmd
        // I_StartTic() + D_ProcessEvents() + G_BuildTiccmd()
        let idx = (net.maketic as usize) % BACKUPTICS;
        cb.poll_and_build_ticcmd(&mut net.localcmds[idx]);

        net.maketic += 1;
    }

    // d_net.c line 413: In single-tic debug mode, skip network send/listen
    if singletics {
        return;
    }

    // d_net.c lines 417-441: Send packets to other nodes
    // STUB: In single-player, we only send to node 0 (local rebound).
    //
    // For each active node, we would:
    // 1. Check if we have new tics to send
    // 2. Check for retransmission requests
    // 3. Build and send the packet
    //
    // For node 0 (local node), HSendPacket stores the packet as a
    // rebound packet which is picked up by get_packets below.
    if net.nodeingame[0] {
        let send_start = net.resendto[0];
        let send_end = net.maketic;

        if send_end > send_start {
            // d_net.c line 428: Build the outgoing packet
            net.doomcom.data.starttic = (send_start & 0xff) as u8;
            net.doomcom.data.numtics = ((send_end - send_start).min(BACKUPTICS as i32)) as u8;

            // d_net.c line 430: Copy localcmds into packet cmds
            for j in 0..(net.doomcom.data.numtics as i32) {
                let src_idx = ((send_start + j) as usize) % BACKUPTICS;
                let dst_idx = j as usize;
                if dst_idx < BACKUPTICS {
                    net.doomcom.data.cmds[dst_idx] = net.localcmds[src_idx];
                }
            }

            // d_net.c line 437: Attach retransmit request if needed
            let flags = if net.remoteresend[0] {
                net.remoteresend[0] = false;
                net.doomcom.data.retransmitfrom = (net.nettics[0] & 0xff) as u8;
                NCMD_RETRANSMIT
            } else {
                0
            };

            h_send_packet(net, 0, flags, demoplayback, netgame);

            // Update resend tracking
            net.resendto[0] = send_end;
            net.resendcount[0] = 0;
        }
    }

    // d_net.c line 443: "listen:" label — receive incoming packets
    get_packets(net, playeringame, consoleplayer, demoplayback, netgame);
}

// ---------------------------------------------------------------------------
// check_abort — check for Escape key during network wait
// ---------------------------------------------------------------------------

/// Checks for an Escape key press during network arbitration, aborting with
/// an error if detected.
///
/// Translated from `CheckAbort()` — d_net.c lines 453-470.
///
/// Waits 2 tics then polls the event queue for `KEY_ESCAPE`. If found,
/// calls `cb.error()` to abort the game. This prevents the player from
/// being stuck in an infinite network wait.
/// Scans a slice of pending events for an Escape key press, aborting the
/// network synchronization if found.
///
/// Translated from `CheckAbort()` — d_net.c lines 453-470.
///
/// In the original C code, this busy-waits for 2 tics, then polls input and
/// scans the global event queue. In this Rust port, the caller passes a
/// snapshot of pending events since events are not globally accessible.
///
/// In single-player mode, this function is never called because
/// `d_arbitrate_net_start` is a stub. It is preserved for structural
/// completeness and to satisfy the protocol contract.
#[allow(dead_code)]
fn check_abort(events: &[Event], cb: &mut dyn NetCallbacks) {
    // d_net.c line 458: Wait 2 tics
    let stoptic = cb.get_time() + 2;
    while cb.get_time() < stoptic {
        // busy-wait for 2 tics
    }

    // d_net.c lines 462-469: Scan event queue for Escape key
    for ev in events {
        if ev.event_type == EventType::KeyDown && ev.data1 == KEY_ESCAPE {
            cb.error("Network game synchronization aborted.");
        }
    }

    debug!("check_abort: scanned {} events, no abort", events.len());
}

// ---------------------------------------------------------------------------
// d_arbitrate_net_start — network startup handshake
// ---------------------------------------------------------------------------

/// Performs the network startup handshake between all nodes.
///
/// Translated from `D_ArbitrateNetStart()` — d_net.c lines 476-547.
///
/// STUB: In single-player, this function is never called. The multi-player
/// arbitration protocol requires node-to-node packet exchange to negotiate
/// game parameters (skill, episode, map, deathmatch, etc.).
///
/// The key player (consoleplayer == 0) sends setup info to all other nodes,
/// while other nodes listen for the setup packet.
#[allow(dead_code)]
fn d_arbitrate_net_start(
    _net: &mut NetState,
    _playeringame: &mut [bool; MAXPLAYERS],
    _consoleplayer: usize,
    _demoplayback: bool,
    _netgame: bool,
    _cb: &mut dyn NetCallbacks,
) {
    // STUB: Single-player only — no network arbitration needed.
    //
    // In a full multiplayer implementation, this would:
    // 1. If consoleplayer == 0 (key player):
    //    - Set up game parameters in a DoomData packet with NCMD_SETUP flag
    //    - Wait until all other nodes report ready
    //    - Send final start packet
    // 2. If consoleplayer != 0:
    //    - Wait for setup packet from key player
    //    - Extract game parameters (skill, episode, map, etc.)
    //    - Send ready acknowledgement
    //
    // Both sides call check_abort() periodically to allow escape.
    debug!("d_arbitrate_net_start: skipped (single-player mode)");
}

// ---------------------------------------------------------------------------
// d_check_net_game — initialize network state
// ---------------------------------------------------------------------------

/// Initializes the network subsystem and detects whether this is a network
/// game or a single-player session.
///
/// Translated from `D_CheckNetGame()` — d_net.c lines 555-594.
///
/// For single-player, this sets up the minimal network state: one node, one
/// player, ticdup = 1, and marks node 0 and player 0 as active.
///
/// # Parameters
///
/// - `net`: Network state to initialize.
/// - `playeringame`: Mutable reference to the player-active array. Player 0
///   is marked active.
/// - `consoleplayer`: Set to 0 for single-player.
/// - `displayplayer`: Set to 0 for single-player.
/// - `cb`: Callbacks for platform timing.
///
/// # Returns
///
/// A tuple `(consoleplayer, displayplayer)` indicating the assigned player
/// indices.
pub fn d_check_net_game(
    net: &mut NetState,
    playeringame: &mut [bool; MAXPLAYERS],
    cb: &mut dyn NetCallbacks,
) -> (usize, usize) {
    // d_net.c lines 560-567: Initialize per-node tracking arrays
    for i in 0..MAXNETNODES {
        net.nodeingame[i] = false;
        net.nettics[i] = 0;
        net.remoteresend[i] = false;
        net.resendto[i] = 0;
        net.resendcount[i] = 0;
    }

    // d_net.c lines 569-570: Initialize player-to-node mapping
    for i in 0..MAXPLAYERS {
        net.nodeforplayer[i] = 0;
    }

    // d_net.c line 573: Single-player defaults
    // Set doomcom for single-player: 1 node, 1 player, player 0
    net.doomcom.id = DOOMCOM_ID as i32;
    net.doomcom.numnodes = 1;
    net.doomcom.numplayers = 1;
    net.doomcom.consoleplayer = 0;

    let consoleplayer = net.doomcom.consoleplayer as usize;
    let displayplayer = consoleplayer;

    info!(
        "d_check_net_game: version={}, ticrate={}, consoleplayer={}, numplayers={}",
        VERSION, TICRATE, consoleplayer, net.doomcom.numplayers
    );

    // d_net.c line 579: Get ticdup from doomcom (default 1)
    net.ticdup = if net.doomcom.ticdup > 0 {
        net.doomcom.ticdup as i32
    } else {
        1
    };

    // d_net.c line 582: Calculate maxsend — max tics we can buffer ahead
    net.maxsend = (BACKUPTICS as i32) / (2 * net.ticdup) - 1;
    if net.maxsend < 1 {
        net.maxsend = 1;
    }

    // d_net.c lines 585-590: Mark player 0 and node 0 as active
    for p in playeringame.iter_mut() {
        *p = false;
    }
    playeringame[consoleplayer] = true;
    net.nodeingame[0] = true;

    // d_net.c line 592: Initialize timing
    net.gametime = cb.get_time() / net.ticdup;
    net.oldentertics = net.gametime;

    info!(
        "d_check_net_game: startskill = N/A, deathmatch = false, \
         startmap = N/A, startepisode = N/A"
    );

    (consoleplayer, displayplayer)
}

// ---------------------------------------------------------------------------
// d_quit_net_game — clean shutdown
// ---------------------------------------------------------------------------

/// Sends exit notification to all network nodes and shuts down the network
/// subsystem.
///
/// Translated from `D_QuitNetGame()` — d_net.c lines 602-622.
///
/// In single-player, this is effectively a no-op because there is only one
/// node (the local node). In a multiplayer game, it would send NCMD_EXIT
/// packets to all connected nodes.
pub fn d_quit_net_game(net: &mut NetState, demoplayback: bool, netgame: bool, usergame: bool) {
    debug!("d_quit_net_game: shutting down network");

    // d_net.c line 607: If not in a real game, nothing to do
    if !netgame || !usergame || demoplayback {
        return;
    }

    // d_net.c lines 610-620: Send exit packets to all nodes
    // (4 retransmissions for reliability)
    for _retry in 0..4_i32 {
        for i in 1..(net.doomcom.numnodes as usize) {
            h_send_packet(net, i as i32, NCMD_EXIT, demoplayback, netgame);
        }

        // d_net.c line 618: Wait 1 tic between retransmissions
        // In the original C: I_WaitVBL(1);
        // In the Rust port, we skip the wait for clean shutdown
    }
}

// ---------------------------------------------------------------------------
// try_run_tics — master tic scheduler
// ---------------------------------------------------------------------------

/// The master tic scheduler — determines how many game tics to run each
/// frame and executes them.
///
/// Translated from `TryRunTics()` — d_net.c lines 636-767.
///
/// **This function is the heart of DOOM's game timing model and must be
/// preserved exactly for demo playback compatibility.**
///
/// Algorithm summary:
///
/// 1. Compute real elapsed tics since the last call.
/// 2. Call `net_update` to build new ticcmds.
/// 3. Find `lowtic` — the minimum tic count across all active nodes.
/// 4. Decide how many tics (`counts`) to run based on real time vs available
///    tics.
/// 5. Apply frame adaptation for non-key players (skip/slow logic).
/// 6. Wait for new tics if the available count is insufficient (with a 20-tic
///    timeout to keep the menu responsive).
/// 7. Execute `counts × ticdup` simulation tics, calling `G_Ticker()` for
///    each and clearing special buttons on duplicated tics.
///
/// # Parameters
///
/// - `net`: Network state (ticcmd buffers, timing, etc.).
/// - `gametic`: Current game tic — incremented by this function.
/// - `consoleplayer`: Local player index.
/// - `displayplayer`: Display player index.
/// - `demoplayback`: Whether demo playback is active.
/// - `netgame`: Whether this is a network game.
/// - `singletics`: Debug single-tic mode.
/// - `playeringame`: Per-player active flags.
/// - `paused`: Whether the game is paused.
/// - `sendpause`: Whether a pause command is pending.
/// - `cb`: Callbacks for timing and game operations.
///
/// # Returns
///
/// The new value of `gametic` after executing tics. The caller must store
/// this back into `GameCtrl.gametic`.
pub fn try_run_tics(
    net: &mut NetState,
    gametic: i32,
    consoleplayer: usize,
    displayplayer: usize,
    demoplayback: bool,
    netgame: bool,
    singletics: bool,
    playeringame: &[bool; MAXPLAYERS],
    paused: bool,
    sendpause: bool,
    cb: &mut dyn NetCallbacks,
) -> i32 {
    let mut current_gametic = gametic;

    // d_net.c line 650: Get real tics elapsed since last call
    let entertic = cb.get_time() / net.ticdup;
    let realtics = entertic - net.oldentertics;
    net.oldentertics = entertic;

    // d_net.c line 654: Build new ticcmds
    net_update(
        net,
        current_gametic,
        consoleplayer,
        displayplayer,
        demoplayback,
        netgame,
        singletics,
        playeringame,
        paused,
        sendpause,
        cb,
    );

    // d_net.c lines 656-665: Find lowtic across all active nodes
    let mut lowtic = i32::MAX;
    let mut _numplaying: i32 = 0;
    for i in 0..(net.doomcom.numnodes as usize) {
        if net.nodeingame[i] {
            _numplaying += 1;
            if net.nettics[i] < lowtic {
                lowtic = net.nettics[i];
            }
        }
    }

    let availabletics = lowtic - current_gametic / net.ticdup;

    // d_net.c lines 668-678: Decide how many tics to run
    let mut counts = if realtics < availabletics - 1 {
        realtics + 1
    } else if realtics < availabletics {
        realtics
    } else {
        availabletics
    };

    if counts < 1 {
        counts = 1;
    }

    net.frameon += 1;

    debug!(
        "try_run_tics: real={} avail={} counts={}",
        realtics, availabletics, counts
    );

    // d_net.c lines 686-711: Frame adaptation for non-key players
    //
    // In a multiplayer game, if the local player is not the "key player"
    // (player 0), the engine adjusts timing to stay in sync:
    // - If local node is behind, slow down (decrement gametime)
    // - If local node is consistently behind, skip tics
    //
    // In single-player, consoleplayer == 0 is always the key player,
    // so this block effectively does nothing.
    if !demoplayback {
        // Find the first active player
        let mut key_player_idx: usize = 0;
        for (i, &active) in playeringame.iter().enumerate() {
            if active {
                key_player_idx = i;
                break;
            }
        }

        if consoleplayer != key_player_idx {
            // Non-key player adaptation
            let key_node = net.nodeforplayer[key_player_idx] as usize;
            if key_node < MAXNETNODES {
                if net.nettics[0] <= net.nettics[key_node] {
                    net.gametime -= 1;
                    debug!("try_run_tics: slowing down (behind key player)");
                }

                let behind = if net.oldnettics > net.nettics[key_node] {
                    1
                } else {
                    0
                };
                net.frameskip[(net.frameon & 3) as usize] = behind;
                net.oldnettics = net.nettics[0];

                if net.frameskip[0] != 0
                    && net.frameskip[1] != 0
                    && net.frameskip[2] != 0
                    && net.frameskip[3] != 0
                {
                    net.skiptics = 1;
                    debug!("try_run_tics: skipping tics (consistently behind)");
                }
            }
        }
    }

    // d_net.c lines 715-733: Wait for new tics if needed
    //
    // If we don't have enough tics available, poll the network repeatedly
    // until either new tics arrive or a 20-tic timeout expires (to keep
    // the menu responsive during network waits).
    while lowtic < current_gametic / net.ticdup + counts {
        net_update(
            net,
            current_gametic,
            consoleplayer,
            displayplayer,
            demoplayback,
            netgame,
            singletics,
            playeringame,
            paused,
            sendpause,
            cb,
        );

        // Recalculate lowtic
        lowtic = i32::MAX;
        for i in 0..(net.doomcom.numnodes as usize) {
            if net.nodeingame[i] && net.nettics[i] < lowtic {
                lowtic = net.nettics[i];
            }
        }

        // d_net.c line 726: Sanity check
        if lowtic < current_gametic / net.ticdup {
            cb.error("TryRunTics: lowtic < gametic");
        }

        // d_net.c lines 729-733: 20-tic timeout to keep menu alive
        if cb.get_time() / net.ticdup - entertic >= 20 {
            cb.menu_ticker();
            return current_gametic;
        }
    }

    // d_net.c lines 736-766: Run the tic simulation
    //
    // Execute `counts` batches of `ticdup` tics each. For each tic:
    // 1. Check for demo advance
    // 2. Run menu ticker
    // 3. Run game ticker (advances the simulation)
    // 4. Increment gametic
    // 5. On duplicated tics, clear chatchar and BT_SPECIAL buttons
    while counts > 0 {
        counts -= 1;

        for i in 0..net.ticdup {
            // d_net.c line 740: Sanity check
            if current_gametic / net.ticdup > lowtic {
                cb.error("gametic>lowtic");
            }

            // d_net.c line 741: Check for demo advance
            if cb.is_advance_demo() {
                cb.do_advance_demo();
            }

            // d_net.c lines 742-743: Tick menu and game
            cb.menu_ticker();
            cb.game_ticker();
            current_gametic += 1;

            // d_net.c lines 749-762: Modify commands for duplicated tics
            //
            // When ticdup > 1, all tics in the batch use the same ticcmd.
            // On non-final duplicates, clear chat characters and special
            // button commands to prevent them from being processed multiple
            // times.
            if i != net.ticdup - 1 {
                let buf = ((current_gametic / net.ticdup) as usize) % BACKUPTICS;
                for j in 0..MAXPLAYERS {
                    let cmd = &mut net.netcmds[j][buf];
                    cmd.chatchar = 0;
                    if cmd.buttons & BT_SPECIAL != 0 {
                        cmd.buttons = 0;
                    }
                }
            }
        }

        // d_net.c line 764: Check for new console commands between batches
        net_update(
            net,
            current_gametic,
            consoleplayer,
            displayplayer,
            demoplayback,
            netgame,
            singletics,
            playeringame,
            paused,
            sendpause,
            cb,
        );
    }

    current_gametic
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::doomdef::MAXPLAYERS;
    use crate::types::net::{BACKUPTICS, MAXNETNODES};

    /// Mock implementation of NetCallbacks for testing.
    struct MockCallbacks {
        time: i32,
        build_count: i32,
        game_ticker_count: i32,
        menu_ticker_count: i32,
        advance_demo: bool,
        advance_demo_called: bool,
    }

    impl MockCallbacks {
        fn new(time: i32) -> Self {
            Self {
                time,
                build_count: 0,
                game_ticker_count: 0,
                menu_ticker_count: 0,
                advance_demo: false,
                advance_demo_called: false,
            }
        }
    }

    impl NetCallbacks for MockCallbacks {
        fn get_time(&self) -> i32 {
            self.time
        }

        fn poll_and_build_ticcmd(&mut self, cmd: &mut TicCmd) {
            self.build_count += 1;
            cmd.forwardmove = 1;
        }

        fn game_ticker(&mut self) {
            self.game_ticker_count += 1;
        }

        fn menu_ticker(&mut self) {
            self.menu_ticker_count += 1;
        }

        fn is_advance_demo(&self) -> bool {
            self.advance_demo
        }

        fn do_advance_demo(&mut self) {
            self.advance_demo_called = true;
            self.advance_demo = false;
        }

        fn error(&self, msg: &str) -> ! {
            panic!("Test error: {}", msg);
        }
    }

    #[test]
    fn test_netstate_new() {
        let net = NetState::new();
        assert_eq!(net.maketic, 0);
        assert_eq!(net.lastnettic, 0);
        assert_eq!(net.skiptics, 0);
        assert_eq!(net.ticdup, 1);
        // maxsend defaults to BACKUPTICS/2 - 1 = 5
        assert_eq!(net.maxsend, (BACKUPTICS as i32) / 2 - 1);
        assert!(!net.reboundpacket);
        assert_eq!(net.gametime, 0);
        assert_eq!(net.frameon, 0);
        assert_eq!(net.oldnettics, 0);
        assert_eq!(net.localcmds.len(), BACKUPTICS);
        assert_eq!(net.netcmds.len(), MAXPLAYERS);
        assert_eq!(net.nettics.len(), MAXNETNODES);
        assert_eq!(net.nodeingame.len(), MAXNETNODES);
    }

    #[test]
    fn test_netstate_default_matches_new() {
        let net = NetState::default();
        assert_eq!(net.ticdup, 1);
        assert_eq!(net.maketic, 0);
    }

    #[test]
    fn test_d_check_net_game_single_player() {
        let mut net = NetState::new();
        let mut playeringame = [false; MAXPLAYERS];
        let mut cb = MockCallbacks::new(35);

        let (console, display) = d_check_net_game(&mut net, &mut playeringame, &mut cb);

        assert_eq!(console, 0);
        assert_eq!(display, 0);
        assert!(playeringame[0]);
        assert!(net.nodeingame[0]);
        assert_eq!(net.doomcom.numnodes, 1);
        assert_eq!(net.doomcom.numplayers, 1);
        assert_eq!(net.doomcom.consoleplayer, 0);
        assert_eq!(net.ticdup, 1);
        assert!(net.maxsend >= 1);
        assert_eq!(net.gametime, 35);
    }

    #[test]
    fn test_d_quit_net_game_noop_single_player() {
        let mut net = NetState::new();
        d_quit_net_game(&mut net, false, false, false);
        // no panic = success for single-player
    }

    #[test]
    fn test_net_update_builds_ticcmds() {
        let mut net = NetState::new();
        let mut playeringame = [false; MAXPLAYERS];
        let mut cb = MockCallbacks::new(0);
        let _ = d_check_net_game(&mut net, &mut playeringame, &mut cb);

        cb.time = 3;
        net_update(
            &mut net,
            0,
            0,
            0,
            false,
            false,
            false,
            &playeringame,
            false,
            false,
            &mut cb,
        );

        assert_eq!(net.maketic, 3);
        assert_eq!(cb.build_count, 3);
        for i in 0..3 {
            assert_eq!(net.localcmds[i].forwardmove, 1);
        }
    }

    #[test]
    fn test_net_update_respects_backuptics_limit() {
        let mut net = NetState::new();
        let mut playeringame = [false; MAXPLAYERS];
        let mut cb = MockCallbacks::new(0);
        let _ = d_check_net_game(&mut net, &mut playeringame, &mut cb);

        cb.time = 100;
        net_update(
            &mut net,
            0,
            0,
            0,
            false,
            false,
            false,
            &playeringame,
            false,
            false,
            &mut cb,
        );

        let max_ahead = (BACKUPTICS as i32) / 2 - 1;
        assert!(
            net.maketic <= max_ahead,
            "maketic ({}) should be <= {}",
            net.maketic,
            max_ahead
        );
    }

    #[test]
    fn test_net_update_singletics_returns_early() {
        let mut net = NetState::new();
        let mut playeringame = [false; MAXPLAYERS];
        let mut cb = MockCallbacks::new(0);
        let _ = d_check_net_game(&mut net, &mut playeringame, &mut cb);

        cb.time = 3;
        // singletics = true — should build ticcmds but not send/listen
        net_update(
            &mut net,
            0,
            0,
            0,
            false,
            false,
            true,
            &playeringame,
            false,
            false,
            &mut cb,
        );

        assert_eq!(net.maketic, 3);
        assert_eq!(cb.build_count, 3);
    }

    #[test]
    fn test_try_run_tics_single_tic() {
        let mut net = NetState::new();
        let mut playeringame = [false; MAXPLAYERS];
        let mut cb = MockCallbacks::new(0);
        let _ = d_check_net_game(&mut net, &mut playeringame, &mut cb);

        cb.time = 1;
        let new_gametic = try_run_tics(
            &mut net,
            0,
            0,
            0,
            false,
            false,
            false,
            &playeringame,
            false,
            false,
            &mut cb,
        );

        assert!(
            new_gametic >= 1,
            "gametic should advance, got {}",
            new_gametic
        );
        assert!(cb.game_ticker_count >= 1);
        assert!(cb.menu_ticker_count >= 1);
    }

    #[test]
    fn test_try_run_tics_advance_demo() {
        let mut net = NetState::new();
        let mut playeringame = [false; MAXPLAYERS];
        let mut cb = MockCallbacks::new(0);
        let _ = d_check_net_game(&mut net, &mut playeringame, &mut cb);

        cb.advance_demo = true;
        cb.time = 1;

        let _new_gametic = try_run_tics(
            &mut net,
            0,
            0,
            0,
            false,
            false,
            false,
            &playeringame,
            false,
            false,
            &mut cb,
        );

        assert!(
            cb.advance_demo_called,
            "do_advance_demo should have been called"
        );
    }

    #[test]
    fn test_netstate_all_fields_accessible() {
        let net = NetState::new();
        let _: &crate::types::net::DoomCom = &net.doomcom;
        let _: &[TicCmd; BACKUPTICS] = &net.localcmds;
        let _: &[[TicCmd; BACKUPTICS]; MAXPLAYERS] = &net.netcmds;
        let _: &[i32; MAXNETNODES] = &net.nettics;
        let _: &[bool; MAXNETNODES] = &net.nodeingame;
        let _: &[bool; MAXNETNODES] = &net.remoteresend;
        let _: &[i32; MAXNETNODES] = &net.resendto;
        let _: &[i32; MAXNETNODES] = &net.resendcount;
        let _: &[i32; MAXPLAYERS] = &net.nodeforplayer;
        let _: i32 = net.maketic;
        let _: i32 = net.lastnettic;
        let _: i32 = net.skiptics;
        let _: i32 = net.ticdup;
        let _: i32 = net.maxsend;
        let _: bool = net.reboundpacket;
        let _: &crate::types::net::DoomData = &net.reboundstore;
        let _: i32 = net.gametime;
        let _: &[i32; 4] = &net.frametics;
        let _: i32 = net.frameon;
        let _: &[i32; 4] = &net.frameskip;
        let _: i32 = net.oldnettics;
    }

    #[test]
    fn test_net_update_no_new_tics() {
        let mut net = NetState::new();
        let mut playeringame = [false; MAXPLAYERS];
        let mut cb = MockCallbacks::new(0);
        let _ = d_check_net_game(&mut net, &mut playeringame, &mut cb);

        // Time doesn't advance — no new tics
        net_update(
            &mut net,
            0,
            0,
            0,
            false,
            false,
            false,
            &playeringame,
            false,
            false,
            &mut cb,
        );

        // No ticcmds should be built
        assert_eq!(net.maketic, 0);
        assert_eq!(cb.build_count, 0);
    }
}
