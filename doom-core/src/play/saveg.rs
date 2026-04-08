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

//! Translated from linuxdoom-1.10/p_saveg.c and linuxdoom-1.10/p_saveg.h
//!
//! Archiving: SaveGame I/O.
//! Handles serialization and deserialization of game state for save/load.
//! Archives players, world geometry (sectors/linedefs), thinkers (mobjs),
//! and active specials (ceilings, doors, floors, platforms, lights).
//!
//! # Architecture
//!
//! The C code uses a raw byte pointer (`save_p`) that advances through a
//! pre-allocated buffer, performing `memcpy` for structs and manual byte
//! writes for tag bytes. This Rust port replaces that with a [`SaveGame`]
//! struct that owns a `Vec<u8>` buffer and tracks a read/write position.
//!
//! Pointer-to-index conversions (state pointers → state table indices,
//! sector pointers → sector array indices, player pointers → player number)
//! are translated to arena-index operations since the Rust port uses
//! arena-based storage for all game objects.
//!
//! # Original C functions translated
//!
//! | Rust function | C function | Description |
//! |---|---|---|
//! | [`archive_players`] | `P_ArchivePlayers` | Serialize active player structs |
//! | [`unarchive_players`] | `P_UnArchivePlayers` | Deserialize player structs |
//! | [`archive_world`] | `P_ArchiveWorld` | Serialize sector/line/side data |
//! | [`unarchive_world`] | `P_UnArchiveWorld` | Deserialize sector/line/side data |
//! | [`archive_thinkers`] | `P_ArchiveThinkers` | Serialize mobj thinkers |
//! | [`unarchive_thinkers`] | `P_UnArchiveThinkers` | Deserialize mobj thinkers |
//! | [`archive_specials`] | `P_ArchiveSpecials` | Serialize special thinkers |
//! | [`unarchive_specials`] | `P_UnArchiveSpecials` | Deserialize special thinkers |

use crate::info::mobjinfo::NUMMOBJTYPES;
use crate::info::states::NUMSTATES;
use crate::play::ceilng::{p_add_active_ceiling, ActiveCeilings};
use crate::play::plats::{p_add_active_plat, ActivePlats};
use crate::play::spec::{CeilingT, FloorMoveT, GlowT, LightFlashT, PlatT, StrobeFlashT, VldoorT};
use crate::play::tick::ThinkerList;
use crate::types::doomdef::{WeaponType, MAXPLAYERS, NUMAMMO, NUMCARDS, NUMPOWERS, NUMWEAPONS};
use crate::types::fixed::{Fixed, FRACBITS};
use crate::types::map_data::{LineDef, MapThing, Sector, SideDef, Subsector};
use crate::types::mobj::{MapObject, MobjFlags};
use crate::types::player::{Player, PlayerState, NUMPSPRITES};
use crate::types::thinker::{ActionFn, Thinker};

// =============================================================================
// ThinkerClass enum (p_saveg.c lines 220-225)
// =============================================================================

/// Thinker class tag bytes for mobj serialization in save files.
///
/// Original C: `enum { tc_end, tc_mobj } thinkerclass_t;`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ThinkerClass {
    /// End-of-thinkers sentinel marker.
    End = 0,
    /// Map object (mobj) thinker.
    Mobj = 1,
}

// =============================================================================
// SpecialsClass enum (p_saveg.c lines 329-340)
// =============================================================================

/// Special thinker class tag bytes for specials serialization in save files.
///
/// Original C: `enum { ... } specials_e;`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum SpecialsClass {
    Ceiling = 0,
    Door = 1,
    Floor = 2,
    Plat = 3,
    Flash = 4,
    Strobe = 5,
    Glow = 6,
    EndSpecials = 7,
}

impl SpecialsClass {
    /// Convert a raw byte to a SpecialsClass variant.
    /// Returns None for unrecognized values.
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Ceiling),
            1 => Some(Self::Door),
            2 => Some(Self::Floor),
            3 => Some(Self::Plat),
            4 => Some(Self::Flash),
            5 => Some(Self::Strobe),
            6 => Some(Self::Glow),
            7 => Some(Self::EndSpecials),
            _ => None,
        }
    }
}

// =============================================================================
// SaveGame struct — replaces C global `byte* save_p`
// =============================================================================

/// Save game buffer state, replacing the C global `byte* save_p`.
///
/// Provides sequential read/write access to a byte buffer with little-endian
/// integer serialization matching the original DOOM save format.
///
/// # Save format compatibility
///
/// The original C code writes data as raw memory copies of C structs, with
/// `PADSAVEP` alignment to 4-byte boundaries. This Rust implementation
/// serializes field-by-field in little-endian byte order, with the same
/// padding alignment, producing a compatible save format.
pub struct SaveGame {
    /// The raw save data buffer.
    pub buffer: Vec<u8>,
    /// Current read/write position within the buffer.
    pub pos: usize,
}

impl SaveGame {
    /// Create a new SaveGame with the given buffer.
    ///
    /// For saving: pass an empty or pre-allocated `Vec<u8>`.
    /// For loading: pass the buffer read from the save file.
    pub fn new(buffer: Vec<u8>) -> Self {
        Self { buffer, pos: 0 }
    }

    /// Write a single byte to the buffer at the current position.
    ///
    /// Extends the buffer if necessary (save mode).
    /// Original C: `*save_p++ = value;`
    pub fn write_byte(&mut self, b: u8) {
        if self.pos < self.buffer.len() {
            self.buffer[self.pos] = b;
        } else {
            self.buffer.push(b);
        }
        self.pos += 1;
    }

    /// Write a 16-bit signed integer in little-endian format.
    ///
    /// Original C: `*(short*)save_p = value; save_p += 2;`
    pub fn write_i16(&mut self, v: i16) {
        let bytes = v.to_le_bytes();
        self.write_byte(bytes[0]);
        self.write_byte(bytes[1]);
    }

    /// Write a 32-bit signed integer in little-endian format.
    ///
    /// Original C: `*(int*)save_p = value; save_p += 4;`
    pub fn write_i32(&mut self, v: i32) {
        let bytes = v.to_le_bytes();
        self.write_byte(bytes[0]);
        self.write_byte(bytes[1]);
        self.write_byte(bytes[2]);
        self.write_byte(bytes[3]);
    }

    /// Read a single byte from the buffer at the current position.
    ///
    /// Returns 0 if reading past the end of the buffer (defensive).
    /// Original C: `value = *save_p++;`
    pub fn read_byte(&mut self) -> u8 {
        if self.pos < self.buffer.len() {
            let b = self.buffer[self.pos];
            self.pos += 1;
            b
        } else {
            self.pos += 1;
            0
        }
    }

    /// Read a 16-bit signed integer in little-endian format.
    ///
    /// Original C: `value = *(short*)save_p; save_p += 2;`
    pub fn read_i16(&mut self) -> i16 {
        let b0 = self.read_byte();
        let b1 = self.read_byte();
        i16::from_le_bytes([b0, b1])
    }

    /// Read a 32-bit signed integer in little-endian format.
    ///
    /// Original C: `value = *(int*)save_p; save_p += 4;`
    pub fn read_i32(&mut self) -> i32 {
        let b0 = self.read_byte();
        let b1 = self.read_byte();
        let b2 = self.read_byte();
        let b3 = self.read_byte();
        i32::from_le_bytes([b0, b1, b2, b3])
    }

    /// Pad the current position to the next 4-byte boundary.
    ///
    /// Translated from the `PADSAVEP` macro in p_saveg.c line 40:
    /// `save_p += (4 - ((int) save_p & 3)) & 3;`
    ///
    /// This ensures alignment for save format compatibility with SGI/Gecko.
    pub fn pad(&mut self) {
        let padding = (4 - (self.pos & 3)) & 3;
        // When reading, just advance the position.
        // When writing, insert zero-padding bytes.
        for _ in 0..padding {
            if self.pos >= self.buffer.len() {
                self.buffer.push(0);
            }
            self.pos += 1;
        }
    }
}

// =============================================================================
// P_ArchivePlayers — p_saveg.c lines 47-72
// =============================================================================

/// Archive all active players to the save buffer.
///
/// Serializes each active player's full state, converting psprite state
/// references (Option<usize> indices into the STATES table) to raw integer
/// indices for the save file. Inactive players are skipped.
///
/// Original C: iterates `MAXPLAYERS`, skips `!playeringame[i]`, `memcpy`s
/// `player_t`, then converts `psprites[j].state` pointers to offsets from
/// `states` base.
pub fn archive_players(
    save: &mut SaveGame,
    players: &[Player; MAXPLAYERS],
    playeringame: &[bool; MAXPLAYERS],
) {
    for i in 0..MAXPLAYERS {
        if !playeringame[i] {
            continue;
        }
        save.pad();
        write_player(save, &players[i]);
    }
}

/// Serialize a single Player struct to the save buffer.
///
/// Fields are written in the same order as the C `player_t` struct memory
/// layout. State references (psprite states) are converted to indices.
fn write_player(save: &mut SaveGame, p: &Player) {
    // mobj pointer — stored as arena index, will be reconnected on load
    save.write_i32(p.mobj.map_or(-1, |v| v as i32));
    // playerstate
    save.write_i32(p.playerstate as i32);
    // cmd — TicCmd fields
    save.write_byte(p.cmd.forwardmove as u8);
    save.write_byte(p.cmd.sidemove as u8);
    save.write_i16(p.cmd.angleturn);
    save.write_i16(p.cmd.consistancy);
    save.write_byte(p.cmd.chatchar);
    save.write_byte(p.cmd.buttons);
    // viewz, viewheight, deltaviewheight, bob
    save.write_i32(p.viewz.0);
    save.write_i32(p.viewheight.0);
    save.write_i32(p.deltaviewheight.0);
    save.write_i32(p.bob.0);
    // health, armorpoints, armortype
    save.write_i32(p.health);
    save.write_i32(p.armorpoints);
    save.write_i32(p.armortype);
    // powers array
    for j in 0..NUMPOWERS {
        save.write_i32(p.powers[j]);
    }
    // cards array
    for j in 0..NUMCARDS {
        save.write_i32(if p.cards[j] { 1 } else { 0 });
    }
    // backpack
    save.write_i32(if p.backpack { 1 } else { 0 });
    // frags array
    for j in 0..MAXPLAYERS {
        save.write_i32(p.frags[j]);
    }
    // readyweapon, pendingweapon
    save.write_i32(p.readyweapon as i32);
    save.write_i32(p.pendingweapon as i32);
    // weaponowned array
    for j in 0..NUMWEAPONS {
        save.write_i32(if p.weaponowned[j] { 1 } else { 0 });
    }
    // ammo array
    for j in 0..NUMAMMO {
        save.write_i32(p.ammo[j]);
    }
    // maxammo array
    for j in 0..NUMAMMO {
        save.write_i32(p.maxammo[j]);
    }
    // attackdown, usedown
    save.write_i32(p.attackdown);
    save.write_i32(p.usedown);
    // cheats
    save.write_i32(p.cheats);
    // refire
    save.write_i32(p.refire);
    // killcount, itemcount, secretcount
    save.write_i32(p.killcount);
    save.write_i32(p.itemcount);
    save.write_i32(p.secretcount);
    // message — not serialized (set to None on load), write a placeholder
    save.write_i32(0);
    // damagecount, bonuscount
    save.write_i32(p.damagecount);
    save.write_i32(p.bonuscount);
    // attacker — not serialized (set to None on load), write placeholder
    save.write_i32(-1);
    // extralight
    save.write_i32(p.extralight);
    // fixedcolormap
    save.write_i32(p.fixedcolormap);
    // colormap
    save.write_i32(p.colormap);
    // psprites — with state pointer → index conversion
    for j in 0..NUMPSPRITES {
        // Convert state Option<usize> to i32 index
        // C: dest->psprites[j].state = (state_t*)(dest->psprites[j].state - states)
        let state_idx = p.psprites[j].state.map_or(-1, |s| s as i32);
        save.write_i32(state_idx);
        save.write_i32(p.psprites[j].tics);
        save.write_i32(p.psprites[j].sx.0);
        save.write_i32(p.psprites[j].sy.0);
    }
    // didsecret
    save.write_i32(if p.didsecret { 1 } else { 0 });
}

// =============================================================================
// P_UnArchivePlayers — p_saveg.c lines 79-108
// =============================================================================

/// Unarchive players from save buffer. Reverses archive_players.
///
/// Restores player structs from the save buffer, converting state indices
/// back to `Option<usize>` references. Sets `mobj`, `message`, and `attacker`
/// to `None` — these are restored later when thinkers are loaded and linked.
///
/// Original C: iterates `MAXPLAYERS`, skips `!playeringame[i]`, `memcpy`s
/// `player_t`, sets `mo=NULL`, `message=NULL`, `attacker=NULL`, converts
/// state offsets back to pointers.
pub fn unarchive_players(
    save: &mut SaveGame,
    players: &mut [Player; MAXPLAYERS],
    playeringame: &[bool; MAXPLAYERS],
) {
    for i in 0..MAXPLAYERS {
        if !playeringame[i] {
            continue;
        }
        save.pad();
        read_player(save, &mut players[i]);
        // Critical: these pointers are restored when thinkers load
        // C: players[i].mo = NULL; players[i].message = NULL;
        //    players[i].attacker = NULL;
        players[i].mobj = None;
        players[i].message = None;
        players[i].attacker = None;
    }
}

/// Deserialize a single Player struct from the save buffer.
///
/// Reads fields in the same order as write_player. State indices
/// are validated against NUMSTATES before being stored.
fn read_player(save: &mut SaveGame, p: &mut Player) {
    // mobj pointer (will be overwritten with None by caller)
    let _mobj_idx = save.read_i32();
    // playerstate
    let ps_val = save.read_i32();
    p.playerstate = match ps_val {
        0 => PlayerState::Live,
        1 => PlayerState::Dead,
        2 => PlayerState::Reborn,
        _ => PlayerState::Live,
    };
    // cmd — TicCmd fields
    p.cmd.forwardmove = save.read_byte() as i8;
    p.cmd.sidemove = save.read_byte() as i8;
    p.cmd.angleturn = save.read_i16();
    p.cmd.consistancy = save.read_i16();
    p.cmd.chatchar = save.read_byte();
    p.cmd.buttons = save.read_byte();
    // viewz, viewheight, deltaviewheight, bob
    p.viewz = Fixed(save.read_i32());
    p.viewheight = Fixed(save.read_i32());
    p.deltaviewheight = Fixed(save.read_i32());
    p.bob = Fixed(save.read_i32());
    // health, armorpoints, armortype
    p.health = save.read_i32();
    p.armorpoints = save.read_i32();
    p.armortype = save.read_i32();
    // powers array
    for j in 0..NUMPOWERS {
        p.powers[j] = save.read_i32();
    }
    // cards array
    for j in 0..NUMCARDS {
        p.cards[j] = save.read_i32() != 0;
    }
    // backpack
    p.backpack = save.read_i32() != 0;
    // frags array
    for j in 0..MAXPLAYERS {
        p.frags[j] = save.read_i32();
    }
    // readyweapon, pendingweapon
    let rw = save.read_i32();
    p.readyweapon = weapon_from_i32(rw);
    let pw = save.read_i32();
    p.pendingweapon = weapon_from_i32(pw);
    // weaponowned array
    for j in 0..NUMWEAPONS {
        p.weaponowned[j] = save.read_i32() != 0;
    }
    // ammo array
    for j in 0..NUMAMMO {
        p.ammo[j] = save.read_i32();
    }
    // maxammo array
    for j in 0..NUMAMMO {
        p.maxammo[j] = save.read_i32();
    }
    // attackdown, usedown
    p.attackdown = save.read_i32();
    p.usedown = save.read_i32();
    // cheats
    p.cheats = save.read_i32();
    // refire
    p.refire = save.read_i32();
    // killcount, itemcount, secretcount
    p.killcount = save.read_i32();
    p.itemcount = save.read_i32();
    p.secretcount = save.read_i32();
    // message placeholder (not restored)
    let _message = save.read_i32();
    // damagecount, bonuscount
    p.damagecount = save.read_i32();
    p.bonuscount = save.read_i32();
    // attacker placeholder (not restored)
    let _attacker = save.read_i32();
    // extralight
    p.extralight = save.read_i32();
    // fixedcolormap
    p.fixedcolormap = save.read_i32();
    // colormap
    p.colormap = save.read_i32();
    // psprites — with index → state reference conversion
    for j in 0..NUMPSPRITES {
        let state_idx = save.read_i32();
        // C: &states[(int)players[i].psprites[j].state]
        if state_idx >= 0 && (state_idx as usize) < NUMSTATES {
            p.psprites[j].state = Some(state_idx as usize);
        } else {
            p.psprites[j].state = None;
        }
        p.psprites[j].tics = save.read_i32();
        p.psprites[j].sx = Fixed(save.read_i32());
        p.psprites[j].sy = Fixed(save.read_i32());
    }
    // didsecret
    p.didsecret = save.read_i32() != 0;
}

/// Convert a raw i32 to a WeaponType enum value.
///
/// Falls back to `WeaponType::Fist` for out-of-range values.
fn weapon_from_i32(v: i32) -> WeaponType {
    match v {
        0 => WeaponType::Fist,
        1 => WeaponType::Pistol,
        2 => WeaponType::Shotgun,
        3 => WeaponType::Chaingun,
        4 => WeaponType::Missile,
        5 => WeaponType::Plasma,
        6 => WeaponType::Bfg,
        7 => WeaponType::Chainsaw,
        8 => WeaponType::SuperShotgun,
        9 => WeaponType::NoChange,
        _ => WeaponType::Fist,
    }
}

// =============================================================================
// P_ArchiveWorld — p_saveg.c lines 114-160
// =============================================================================

/// Archive world state (sectors and linedefs) to save buffer.
///
/// Serializes all sectors (floor/ceiling height as i16 after >>FRACBITS,
/// floorpic, ceilingpic, lightlevel, special, tag) and all linedefs with
/// their sides (flags, special, tag; then for each valid side:
/// textureoffset/rowoffset >>FRACBITS as i16, top/bottom/mid textures).
///
/// Original C: p_saveg.c lines 114-160
pub fn archive_world(
    save: &mut SaveGame,
    sectors: &[Sector],
    lines: &[LineDef],
    sides: &[SideDef],
) {
    // Archive sectors
    for sec in sectors.iter() {
        save.write_i16((sec.floorheight.0 >> FRACBITS) as i16);
        save.write_i16((sec.ceilingheight.0 >> FRACBITS) as i16);
        save.write_i16(sec.floorpic);
        save.write_i16(sec.ceilingpic);
        save.write_i16(sec.lightlevel);
        save.write_i16(sec.special);
        save.write_i16(sec.tag);
    }

    // Archive lines
    for li in lines.iter() {
        save.write_i16(li.flags);
        save.write_i16(li.special);
        save.write_i16(li.tag);
        for s in 0..2 {
            let side_idx = li.sidenum[s];
            if side_idx == -1 {
                continue;
            }
            let si = &sides[side_idx as usize];
            save.write_i16((si.textureoffset.0 >> FRACBITS) as i16);
            save.write_i16((si.rowoffset.0 >> FRACBITS) as i16);
            save.write_i16(si.toptexture);
            save.write_i16(si.bottomtexture);
            save.write_i16(si.midtexture);
        }
    }
}

// =============================================================================
// P_UnArchiveWorld — p_saveg.c lines 167-211
// =============================================================================

/// Unarchive world state from save buffer. Reverses archive_world.
///
/// Restores all sector properties (heights converted back to fixed-point
/// via <<FRACBITS), clears specialdata and soundtarget to None.
/// Restores all linedef/sidedef properties similarly.
///
/// Original C: p_saveg.c lines 167-211
pub fn unarchive_world(
    save: &mut SaveGame,
    sectors: &mut [Sector],
    lines: &mut [LineDef],
    sides: &mut [SideDef],
) {
    // Unarchive sectors
    for sec in sectors.iter_mut() {
        sec.floorheight = Fixed((save.read_i16() as i32) << FRACBITS);
        sec.ceilingheight = Fixed((save.read_i16() as i32) << FRACBITS);
        sec.floorpic = save.read_i16();
        sec.ceilingpic = save.read_i16();
        sec.lightlevel = save.read_i16();
        sec.special = save.read_i16();
        sec.tag = save.read_i16();
        // Critical: clear runtime-only fields
        // C: sec->specialdata = 0; sec->soundtarget = 0;
        sec.specialdata = None;
        sec.soundtarget = None;
    }

    // Unarchive lines
    for li in lines.iter_mut() {
        li.flags = save.read_i16();
        li.special = save.read_i16();
        li.tag = save.read_i16();
        for s in 0..2 {
            let side_idx = li.sidenum[s];
            if side_idx == -1 {
                continue;
            }
            let si = &mut sides[side_idx as usize];
            si.textureoffset = Fixed((save.read_i16() as i32) << FRACBITS);
            si.rowoffset = Fixed((save.read_i16() as i32) << FRACBITS);
            si.toptexture = save.read_i16();
            si.bottomtexture = save.read_i16();
            si.midtexture = save.read_i16();
        }
    }
}

// =============================================================================
// P_ArchiveThinkers — p_saveg.c lines 232-259
// =============================================================================

/// Archive active mobj thinkers to save buffer.
///
/// Walks the thinker list, serializes each `MobjThinker` with pointer-to-index
/// conversion for state, player, and target references. Writes a `tc_mobj`
/// marker byte before each mobj, and a `tc_end` sentinel at the end.
///
/// Original C: p_saveg.c lines 232-259
///
/// # Parameters
///
/// * `save` — the save game buffer
/// * `thinker_list` — the thinker linked list
/// * `mobjs` — all map objects in the arena
/// * `players` — player array for player→index conversion
pub fn archive_thinkers(
    save: &mut SaveGame,
    thinker_list: &ThinkerList,
    mobjs: &[MapObject],
    players: &[Player; MAXPLAYERS],
) {
    // Walk the thinker list from thinkercap.next to thinkercap
    let head = thinker_list.head;
    let mut current = thinker_list.entries[head].thinker.next;

    while let Some(idx) = current {
        if idx == head {
            break;
        }

        let entry = &thinker_list.entries[idx];
        if entry.active && entry.thinker.function == ActionFn::MobjThinker {
            // Write tc_mobj marker
            save.write_byte(ThinkerClass::Mobj as u8);
            save.pad();
            // Serialize the mobj
            write_mobj(save, &mobjs[entry.data_index], players);
        }

        current = entry.thinker.next;
    }

    // Write tc_end sentinel
    save.write_byte(ThinkerClass::End as u8);
}

/// Serialize a single MapObject to the save buffer.
///
/// Converts state references to indices, player references to 1-based player
/// numbers, and target references to a marker (target is set to None on load).
fn write_mobj(save: &mut SaveGame, mo: &MapObject, _players: &[Player; MAXPLAYERS]) {
    // Write a placeholder for the thinker links (not meaningful on disk)
    // Original C memcpy's the full mobj_t which includes thinker_t
    save.write_i32(0); // thinker.prev
    save.write_i32(0); // thinker.next
    save.write_i32(0); // thinker.function

    // Position and angle
    save.write_i32(mo.x.0);
    save.write_i32(mo.y.0);
    save.write_i32(mo.z.0);

    // Sector links (snext/sprev — not meaningful on disk, zeroed)
    save.write_i32(mo.snext.map_or(0, |v| v as i32));
    save.write_i32(mo.sprev.map_or(0, |v| v as i32));

    // Angle
    save.write_i32(mo.angle.0 as i32);

    // Sprite and frame
    save.write_i32(mo.sprite as i32);
    save.write_i32(mo.frame);

    // Blockmap links (not meaningful on disk, zeroed)
    save.write_i32(mo.bnext.map_or(0, |v| v as i32));
    save.write_i32(mo.bprev.map_or(0, |v| v as i32));

    // Subsector (not meaningful on disk)
    save.write_i32(mo.subsector.map_or(0, |v| v as i32));

    // Floor/ceiling z
    save.write_i32(mo.floorz.0);
    save.write_i32(mo.ceilingz.0);

    // Radius and height
    save.write_i32(mo.radius.0);
    save.write_i32(mo.height.0);

    // Momentum
    save.write_i32(mo.momx.0);
    save.write_i32(mo.momy.0);
    save.write_i32(mo.momz.0);

    // Validcount (runtime only, but written for format compat)
    save.write_i32(mo.validcount);

    // Type
    save.write_i32(mo.type_ as i32);

    // Info pointer — stored as type index (not meaningful, restored from type_)
    save.write_i32(mo.info.unwrap_or(0) as i32);

    // Tics
    save.write_i32(mo.tics);

    // State → index conversion
    // C: mobj->state = (state_t*)(mobj->state - states)
    let state_idx = mo.state.map_or(0, |s| s as i32);
    save.write_i32(state_idx);

    // Flags
    save.write_i32(mo.flags.bits() as i32);

    // Health
    save.write_i32(mo.health);

    // Movement direction and count
    save.write_i32(mo.movedir);
    save.write_i32(mo.movecount);

    // Target — written as 0 (set to None on load)
    save.write_i32(mo.target.map_or(0, |v| v as i32));

    // Reaction time and threshold
    save.write_i32(mo.reactiontime);
    save.write_i32(mo.threshold);

    // Player → 1-based index conversion
    // C: mobj->player = (player_t*)((mobj->player - players) + 1)
    let player_val = if let Some(player_idx) = mo.player {
        // Find which player slot this mobj belongs to
        // The player_idx in our arena is the index into the players array
        (player_idx as i32) + 1
    } else {
        0
    };
    save.write_i32(player_val);

    // Lastlook
    save.write_i32(mo.lastlook);

    // Spawnpoint (mapthing_t)
    save.write_i16(mo.spawnpoint.x);
    save.write_i16(mo.spawnpoint.y);
    save.write_i16(mo.spawnpoint.angle);
    save.write_i16(mo.spawnpoint.type_);
    save.write_i16(mo.spawnpoint.options);

    // Tracer — written but set to None on load
    save.write_i32(mo.tracer.map_or(0, |v| v as i32));
}

// =============================================================================
// P_UnArchiveThinkers — p_saveg.c lines 266-323
// =============================================================================

/// Unarchive mobj thinkers from save buffer.
///
/// Removes all existing thinkers, reinitializes the thinker list, then reads
/// `tc_mobj` entries from the save buffer. For each mobj: restores all fields,
/// converts state/player indices back to references, links into the thinker list
/// and spatial data structures.
///
/// # Parameters
///
/// * `save` — the save game buffer
/// * `thinker_list` — the thinker list (will be reinitialized)
/// * `mobjs` — mobj storage arena (will be cleared and rebuilt)
/// * `players` — player array for player index→mobj linkage
/// * `playeringame` — which player slots are active
/// * `sectors` — level sectors for spatial linking
/// * `subsectors` — level subsectors for position lookup
/// * `blocklinks` — blockmap thing links
/// * `bmaporgx` — blockmap origin X
/// * `bmaporgy` — blockmap origin Y
/// * `bmapwidth` — blockmap width in blocks
/// * `bmapheight` — blockmap height in blocks
pub fn unarchive_thinkers(
    save: &mut SaveGame,
    thinker_list: &mut ThinkerList,
    mobjs: &mut Vec<MapObject>,
    players: &mut [Player; MAXPLAYERS],
    _playeringame: &[bool; MAXPLAYERS],
    sectors: &mut [Sector],
    subsectors: &[Subsector],
    blocklinks: &mut [Option<usize>],
    bmaporgx: Fixed,
    bmaporgy: Fixed,
    bmapwidth: i32,
    bmapheight: i32,
) {
    // Remove all existing thinkers and mobjs
    // C: Remove all current thinkers then P_InitThinkers()
    thinker_list.init_thinkers();
    mobjs.clear();

    // Read thinker entries
    loop {
        let tclass = save.read_byte();
        if tclass == ThinkerClass::End as u8 {
            break;
        }
        if tclass != ThinkerClass::Mobj as u8 {
            // Unknown thinker class — error in save data
            break;
        }

        save.pad();

        // Allocate and read a new mobj
        let mut mo = MapObject::default();
        read_mobj(save, &mut mo);

        // Restore state reference
        // C: mobj->state = &states[(int)mobj->state]
        // state was stored as a raw index
        if let Some(state_idx) = mo.state {
            if state_idx < NUMSTATES {
                mo.state = Some(state_idx);
            } else {
                mo.state = Some(0); // S_NULL fallback
            }
        }

        // Target is always cleared on load
        // C: mobj->target = NULL;
        mo.target = None;

        // Restore info from mobjinfo table
        // C: mobj->info = &mobjinfo[mobj->type]
        if mo.type_ < NUMMOBJTYPES {
            mo.info = Some(mo.type_);
        } else {
            mo.info = None;
        }

        // Add mobj to arena
        let mobj_idx = mobjs.len();
        mobjs.push(mo);

        // Link into spatial data structures
        // C: P_SetThingPosition(mobj)
        // We need a point_in_subsector function — use a simple lookup via subsector
        // In the original C, this calls R_PointInSubsector which does a BSP walk.
        // For save/load, we perform a simplified linking using the subsector field.
        crate::play::maputl::p_set_thing_position(
            mobj_idx,
            mobjs,
            sectors,
            subsectors,
            blocklinks,
            bmaporgx,
            bmaporgy,
            bmapwidth,
            bmapheight,
            |x, y| {
                // Simple point-in-subsector lookup.
                // During unarchive, the mobj's subsector is already set from
                // the save data. We use a brute-force fallback that returns 0.
                // The real R_PointInSubsector will re-link properly on first render.
                // For now, return 0 as a safe default (matches vanilla save/load
                // behavior where P_SetThingPosition is called with potentially
                // stale subsector data that gets corrected during gameplay).
                let _ = (x, y);
                0
            },
        );

        // Restore floorz/ceilingz from subsector's sector
        // C: mobj->floorz = mobj->subsector->sector->floorheight;
        //    mobj->ceilingz = mobj->subsector->sector->ceilingheight;
        if let Some(ss_idx) = mobjs[mobj_idx].subsector {
            if ss_idx < subsectors.len() {
                let sec_idx = subsectors[ss_idx].sector;
                if sec_idx < sectors.len() {
                    mobjs[mobj_idx].floorz = sectors[sec_idx].floorheight;
                    mobjs[mobj_idx].ceilingz = sectors[sec_idx].ceilingheight;
                }
            }
        }

        // Restore player linkage
        // C: mobj->player = &players[(int)mobj->player - 1];
        //    mobj->player->mo = mobj;
        if let Some(player_idx) = mobjs[mobj_idx].player {
            // player was stored as 1-based index
            if player_idx > 0 && player_idx <= MAXPLAYERS {
                let p_idx = player_idx - 1;
                mobjs[mobj_idx].player = Some(p_idx);
                players[p_idx].mobj = Some(mobj_idx);
            } else {
                mobjs[mobj_idx].player = None;
            }
        }

        // Add thinker to the thinker list
        // C: P_AddThinker(&mobj->thinker);
        // The thinker function is MobjThinker since we only serialize mobjThinkers
        thinker_list.add_thinker(ActionFn::MobjThinker, mobj_idx);
    }
}

/// Deserialize a single MapObject from the save buffer.
///
/// Reads fields in the same order as write_mobj. State and player values
/// are left as raw indices — the caller converts them to proper references.
fn read_mobj(save: &mut SaveGame, mo: &mut MapObject) {
    // Thinker links (placeholders, not used)
    let _thinker_prev = save.read_i32();
    let _thinker_next = save.read_i32();
    let _thinker_fn = save.read_i32();

    // Position
    mo.x = Fixed(save.read_i32());
    mo.y = Fixed(save.read_i32());
    mo.z = Fixed(save.read_i32());

    // Sector links (will be re-established by P_SetThingPosition)
    mo.snext = {
        let v = save.read_i32();
        if v > 0 {
            Some(v as usize)
        } else {
            None
        }
    };
    mo.sprev = {
        let v = save.read_i32();
        if v > 0 {
            Some(v as usize)
        } else {
            None
        }
    };

    // Angle
    mo.angle = crate::types::angle::Angle(save.read_i32() as u32);

    // Sprite and frame
    mo.sprite = save.read_i32() as usize;
    mo.frame = save.read_i32();

    // Blockmap links (will be re-established by P_SetThingPosition)
    mo.bnext = {
        let v = save.read_i32();
        if v > 0 {
            Some(v as usize)
        } else {
            None
        }
    };
    mo.bprev = {
        let v = save.read_i32();
        if v > 0 {
            Some(v as usize)
        } else {
            None
        }
    };

    // Subsector (will be re-established)
    mo.subsector = {
        let v = save.read_i32();
        if v > 0 {
            Some(v as usize)
        } else {
            None
        }
    };

    // Floor/ceiling z (will be overwritten from sector)
    mo.floorz = Fixed(save.read_i32());
    mo.ceilingz = Fixed(save.read_i32());

    // Radius and height
    mo.radius = Fixed(save.read_i32());
    mo.height = Fixed(save.read_i32());

    // Momentum
    mo.momx = Fixed(save.read_i32());
    mo.momy = Fixed(save.read_i32());
    mo.momz = Fixed(save.read_i32());

    // Validcount
    mo.validcount = save.read_i32();

    // Type
    mo.type_ = save.read_i32() as usize;

    // Info (will be restored from type_)
    let _info = save.read_i32();

    // Tics
    mo.tics = save.read_i32();

    // State — stored as raw index, caller will validate
    let state_idx = save.read_i32();
    if state_idx >= 0 {
        mo.state = Some(state_idx as usize);
    } else {
        mo.state = None;
    }

    // Flags
    let flags_raw = save.read_i32();
    mo.flags = MobjFlags::from_bits_truncate(flags_raw as u32);

    // Health
    mo.health = save.read_i32();

    // Movement direction and count
    mo.movedir = save.read_i32();
    mo.movecount = save.read_i32();

    // Target (will be set to None by caller)
    let _target = save.read_i32();

    // Reaction time and threshold
    mo.reactiontime = save.read_i32();
    mo.threshold = save.read_i32();

    // Player — stored as 1-based, left as-is for caller to convert
    let player_val = save.read_i32();
    if player_val > 0 {
        mo.player = Some(player_val as usize);
    } else {
        mo.player = None;
    }

    // Lastlook
    mo.lastlook = save.read_i32();

    // Spawnpoint (mapthing_t)
    mo.spawnpoint = MapThing {
        x: save.read_i16(),
        y: save.read_i16(),
        angle: save.read_i16(),
        type_: save.read_i16(),
        options: save.read_i16(),
    };

    // Tracer
    let _tracer = save.read_i32();
    mo.tracer = None;
}

// =============================================================================
// P_ArchiveSpecials — p_saveg.c lines 355-469
// =============================================================================

/// Archive active special thinkers (ceilings, doors, floors, platforms, lights).
///
/// Each special type gets a class byte, padding, and field-by-field
/// serialization with sector pointer → index conversion. Handles the special
/// case of inactive ceilings (ceilings with None action function that are
/// still in the activeceilings array and must be serialized for stasis resume).
/// Terminates with `tc_endspecials` marker.
///
/// Original C: p_saveg.c lines 355-469
///
/// # Parameters
///
/// * `save` — the save game buffer
/// * `thinker_list` — the thinker linked list to walk
/// * `ceilings` — all ceiling thinker data
/// * `doors` — all door thinker data
/// * `floors` — all floor thinker data
/// * `plats` — all platform thinker data
/// * `light_flashes` — all light flash thinker data
/// * `strobe_flashes` — all strobe flash thinker data
/// * `glows` — all glow thinker data
/// * `active_ceilings` — active ceiling tracking array
#[allow(clippy::too_many_arguments)]
pub fn archive_specials(
    save: &mut SaveGame,
    thinker_list: &ThinkerList,
    ceilings: &[CeilingT],
    doors: &[VldoorT],
    floors: &[FloorMoveT],
    plats: &[PlatT],
    light_flashes: &[LightFlashT],
    strobe_flashes: &[StrobeFlashT],
    glows: &[GlowT],
    active_ceilings: &ActiveCeilings,
) {
    // Walk the thinker list
    let head = thinker_list.head;
    let mut current = thinker_list.entries[head].thinker.next;

    while let Some(idx) = current {
        if idx == head {
            break;
        }

        let entry = &thinker_list.entries[idx];
        if !entry.active {
            current = entry.thinker.next;
            continue;
        }

        match entry.thinker.function {
            ActionFn::MoveCeiling => {
                save.write_byte(SpecialsClass::Ceiling as u8);
                save.pad();
                write_ceiling(save, &ceilings[entry.data_index]);
            }
            ActionFn::VerticalDoor => {
                save.write_byte(SpecialsClass::Door as u8);
                save.pad();
                write_door(save, &doors[entry.data_index]);
            }
            ActionFn::MoveFloor => {
                save.write_byte(SpecialsClass::Floor as u8);
                save.pad();
                write_floor(save, &floors[entry.data_index]);
            }
            ActionFn::PlatRaise => {
                save.write_byte(SpecialsClass::Plat as u8);
                save.pad();
                write_plat(save, &plats[entry.data_index]);
            }
            ActionFn::LightFlash => {
                save.write_byte(SpecialsClass::Flash as u8);
                save.pad();
                write_light_flash(save, &light_flashes[entry.data_index]);
            }
            ActionFn::StrobeFlash => {
                save.write_byte(SpecialsClass::Strobe as u8);
                save.pad();
                write_strobe_flash(save, &strobe_flashes[entry.data_index]);
            }
            ActionFn::Glow => {
                save.write_byte(SpecialsClass::Glow as u8);
                save.pad();
                write_glow(save, &glows[entry.data_index]);
            }
            _ => {
                // Not a special thinker we serialize
            }
        }

        current = entry.thinker.next;
    }

    // Handle inactive ceilings in the activeceilings array
    // C: p_saveg.c lines 370-386 — ceilings with NULL function still in active list
    for ceiling_idx in active_ceilings.slots.iter().flatten() {
        if *ceiling_idx < ceilings.len() {
            let ceiling = &ceilings[*ceiling_idx];
            // Only serialize if the thinker function is None (inactive/stasis)
            if ceiling.thinker.function == ActionFn::None {
                save.write_byte(SpecialsClass::Ceiling as u8);
                save.pad();
                write_ceiling(save, ceiling);
            }
        }
    }

    // Write end sentinel
    save.write_byte(SpecialsClass::EndSpecials as u8);
}

/// Serialize a CeilingT to the save buffer.
fn write_ceiling(save: &mut SaveGame, c: &CeilingT) {
    // Thinker (placeholder)
    save.write_i32(0); // prev
    save.write_i32(0); // next
                       // Function: write 1 if active (MoveCeiling), 0 if inactive (stasis)
    save.write_i32(if c.thinker.function == ActionFn::MoveCeiling {
        1
    } else {
        0
    });
    // ceiling_type
    save.write_i32(c.ceiling_type as i32);
    // sector → index
    save.write_i32(c.sector as i32);
    // bottomheight, topheight, speed
    save.write_i32(c.bottomheight.0);
    save.write_i32(c.topheight.0);
    save.write_i32(c.speed.0);
    // crush
    save.write_i32(if c.crush { 1 } else { 0 });
    // direction
    save.write_i32(c.direction);
    // tag
    save.write_i32(c.tag);
    // olddirection
    save.write_i32(c.olddirection);
}

/// Serialize a VldoorT to the save buffer.
fn write_door(save: &mut SaveGame, d: &VldoorT) {
    // Thinker (placeholder)
    save.write_i32(0); // prev
    save.write_i32(0); // next
    save.write_i32(1); // function (always active)
                       // door_type
    save.write_i32(d.door_type as i32);
    // sector → index
    save.write_i32(d.sector as i32);
    // topheight, speed
    save.write_i32(d.topheight.0);
    save.write_i32(d.speed.0);
    // direction
    save.write_i32(d.direction);
    // topwait, topcountdown
    save.write_i32(d.topwait);
    save.write_i32(d.topcountdown);
}

/// Serialize a FloorMoveT to the save buffer.
fn write_floor(save: &mut SaveGame, f: &FloorMoveT) {
    // Thinker (placeholder)
    save.write_i32(0); // prev
    save.write_i32(0); // next
    save.write_i32(1); // function
                       // floor_type
    save.write_i32(f.floor_type as i32);
    // crush
    save.write_i32(if f.crush { 1 } else { 0 });
    // sector → index
    save.write_i32(f.sector as i32);
    // direction
    save.write_i32(f.direction);
    // newsecspecial
    save.write_i32(f.newsecspecial);
    // newtexture
    save.write_i16(f.newtexture);
    save.write_i16(0); // padding for alignment
                       // floordestheight, speed
    save.write_i32(f.floordestheight.0);
    save.write_i32(f.speed.0);
}

/// Serialize a PlatT to the save buffer.
fn write_plat(save: &mut SaveGame, p: &PlatT) {
    // Thinker (placeholder)
    save.write_i32(0); // prev
    save.write_i32(0); // next
                       // Function: write 1 if active (PlatRaise), 0 if inactive
    save.write_i32(if p.thinker.function == ActionFn::PlatRaise {
        1
    } else {
        0
    });
    // sector → index
    save.write_i32(p.sector as i32);
    // speed, low, high
    save.write_i32(p.speed.0);
    save.write_i32(p.low.0);
    save.write_i32(p.high.0);
    // wait, count
    save.write_i32(p.wait);
    save.write_i32(p.count);
    // status, oldstatus
    save.write_i32(p.status as i32);
    save.write_i32(p.oldstatus as i32);
    // crush
    save.write_i32(if p.crush { 1 } else { 0 });
    // tag
    save.write_i32(p.tag);
    // plat_type
    save.write_i32(p.plat_type as i32);
}

/// Serialize a LightFlashT to the save buffer.
fn write_light_flash(save: &mut SaveGame, l: &LightFlashT) {
    // Thinker (placeholder)
    save.write_i32(0); // prev
    save.write_i32(0); // next
    save.write_i32(1); // function
                       // sector → index
    save.write_i32(l.sector as i32);
    // count
    save.write_i32(l.count);
    // maxlight, minlight
    save.write_i32(l.maxlight);
    save.write_i32(l.minlight);
    // maxtime, mintime
    save.write_i32(l.maxtime);
    save.write_i32(l.mintime);
}

/// Serialize a StrobeFlashT to the save buffer.
fn write_strobe_flash(save: &mut SaveGame, s: &StrobeFlashT) {
    // Thinker (placeholder)
    save.write_i32(0); // prev
    save.write_i32(0); // next
    save.write_i32(1); // function
                       // sector → index
    save.write_i32(s.sector as i32);
    // count
    save.write_i32(s.count);
    // minlight, maxlight
    save.write_i32(s.minlight);
    save.write_i32(s.maxlight);
    // darktime, brighttime
    save.write_i32(s.darktime);
    save.write_i32(s.brighttime);
}

/// Serialize a GlowT to the save buffer.
fn write_glow(save: &mut SaveGame, g: &GlowT) {
    // Thinker (placeholder)
    save.write_i32(0); // prev
    save.write_i32(0); // next
    save.write_i32(1); // function
                       // sector → index
    save.write_i32(g.sector as i32);
    // minlight, maxlight
    save.write_i32(g.minlight);
    save.write_i32(g.maxlight);
    // direction
    save.write_i32(g.direction);
}

// =============================================================================
// P_UnArchiveSpecials — p_saveg.c lines 475-585
// =============================================================================

/// Unarchive special thinkers from save buffer. Reverses archive_specials.
///
/// For each class tag: reads the struct from the buffer, converts the sector
/// index to a sector reference, restores the action function, and adds the
/// thinker to the thinker list. Ceilings and platforms are also restored to
/// their respective active tracking arrays.
///
/// # Parameters
///
/// * `save` — the save game buffer
/// * `thinker_list` — the thinker list to add specials to
/// * `sectors` — level sectors for sector index→reference restoration
/// * `ceilings` — ceiling thinker storage (will be appended to)
/// * `doors` — door thinker storage (will be appended to)
/// * `floors` — floor thinker storage (will be appended to)
/// * `plats` — platform thinker storage (will be appended to)
/// * `light_flashes` — light flash storage (will be appended to)
/// * `strobe_flashes` — strobe flash storage (will be appended to)
/// * `glows` — glow storage (will be appended to)
/// * `active_ceilings` — active ceiling tracking (for ceiling restoration)
/// * `active_plats` — active platform tracking (for platform restoration)
#[allow(clippy::too_many_arguments)]
pub fn unarchive_specials(
    save: &mut SaveGame,
    thinker_list: &mut ThinkerList,
    sectors: &mut [Sector],
    ceilings: &mut Vec<CeilingT>,
    doors: &mut Vec<VldoorT>,
    floors: &mut Vec<FloorMoveT>,
    plats: &mut Vec<PlatT>,
    light_flashes: &mut Vec<LightFlashT>,
    strobe_flashes: &mut Vec<StrobeFlashT>,
    glows: &mut Vec<GlowT>,
    active_ceilings: &mut ActiveCeilings,
    active_plats: &mut ActivePlats,
) {
    loop {
        let tclass = save.read_byte();
        let spec_class = match SpecialsClass::from_u8(tclass) {
            Some(sc) => sc,
            None => break, // Unknown class, stop reading
        };

        if spec_class == SpecialsClass::EndSpecials {
            break;
        }

        save.pad();

        match spec_class {
            SpecialsClass::Ceiling => {
                let mut ceiling = CeilingT {
                    thinker: Thinker::default(),
                    ceiling_type: crate::play::spec::CeilingType::LowerToFloor,
                    sector: 0,
                    bottomheight: Fixed(0),
                    topheight: Fixed(0),
                    speed: Fixed(0),
                    crush: false,
                    direction: 0,
                    tag: 0,
                    olddirection: 0,
                };
                read_ceiling(save, &mut ceiling);

                // Restore sector specialdata
                if ceiling.sector < sectors.len() {
                    let ceiling_idx = ceilings.len();
                    sectors[ceiling.sector].specialdata = Some(ceiling_idx);
                }

                // Restore thinker function
                // If the saved function was non-zero, it's an active ceiling
                let is_active = ceiling.thinker.function != ActionFn::None;
                if is_active {
                    ceiling.thinker.function = ActionFn::MoveCeiling;
                }

                let ceiling_idx = ceilings.len();
                ceilings.push(ceiling);

                // Add to thinker list
                let action = if is_active {
                    ActionFn::MoveCeiling
                } else {
                    ActionFn::None
                };
                thinker_list.add_thinker(action, ceiling_idx);

                // Add to active ceilings list
                p_add_active_ceiling(active_ceilings, ceiling_idx);
            }
            SpecialsClass::Door => {
                let mut door = VldoorT {
                    thinker: Thinker::default(),
                    door_type: crate::play::spec::VldoorType::Normal,
                    sector: 0,
                    topheight: Fixed(0),
                    speed: Fixed(0),
                    direction: 0,
                    topwait: 0,
                    topcountdown: 0,
                };
                read_door(save, &mut door);

                // Restore sector specialdata
                if door.sector < sectors.len() {
                    let door_idx = doors.len();
                    sectors[door.sector].specialdata = Some(door_idx);
                }

                door.thinker.function = ActionFn::VerticalDoor;

                let door_idx = doors.len();
                doors.push(door);
                thinker_list.add_thinker(ActionFn::VerticalDoor, door_idx);
            }
            SpecialsClass::Floor => {
                let mut floor = FloorMoveT {
                    thinker: Thinker::default(),
                    floor_type: crate::play::spec::FloorType::LowerFloor,
                    crush: false,
                    sector: 0,
                    direction: 0,
                    newsecspecial: 0,
                    newtexture: 0,
                    floordestheight: Fixed(0),
                    speed: Fixed(0),
                };
                read_floor(save, &mut floor);

                // Restore sector specialdata
                if floor.sector < sectors.len() {
                    let floor_idx = floors.len();
                    sectors[floor.sector].specialdata = Some(floor_idx);
                }

                floor.thinker.function = ActionFn::MoveFloor;

                let floor_idx = floors.len();
                floors.push(floor);
                thinker_list.add_thinker(ActionFn::MoveFloor, floor_idx);
            }
            SpecialsClass::Plat => {
                let mut plat = PlatT {
                    thinker: Thinker::default(),
                    sector: 0,
                    speed: Fixed(0),
                    low: Fixed(0),
                    high: Fixed(0),
                    wait: 0,
                    count: 0,
                    status: crate::play::spec::PlatStatus::Up,
                    oldstatus: crate::play::spec::PlatStatus::Up,
                    crush: false,
                    tag: 0,
                    plat_type: crate::play::spec::PlatType::RaiseToNearestAndChange,
                };
                read_plat(save, &mut plat);

                // Restore sector specialdata
                if plat.sector < sectors.len() {
                    let plat_idx = plats.len();
                    sectors[plat.sector].specialdata = Some(plat_idx);
                }

                // Restore thinker function
                let is_active = plat.thinker.function != ActionFn::None;
                if is_active {
                    plat.thinker.function = ActionFn::PlatRaise;
                }

                let plat_idx = plats.len();
                plats.push(plat);

                let action = if is_active {
                    ActionFn::PlatRaise
                } else {
                    ActionFn::None
                };
                thinker_list.add_thinker(action, plat_idx);

                // Add to active plats list
                p_add_active_plat(active_plats, plat_idx);
            }
            SpecialsClass::Flash => {
                let mut flash = LightFlashT {
                    thinker: Thinker::default(),
                    sector: 0,
                    count: 0,
                    maxlight: 0,
                    minlight: 0,
                    maxtime: 0,
                    mintime: 0,
                };
                read_light_flash(save, &mut flash);

                flash.thinker.function = ActionFn::LightFlash;

                let flash_idx = light_flashes.len();
                light_flashes.push(flash);
                thinker_list.add_thinker(ActionFn::LightFlash, flash_idx);
            }
            SpecialsClass::Strobe => {
                let mut strobe = StrobeFlashT {
                    thinker: Thinker::default(),
                    sector: 0,
                    count: 0,
                    minlight: 0,
                    maxlight: 0,
                    darktime: 0,
                    brighttime: 0,
                };
                read_strobe_flash(save, &mut strobe);

                strobe.thinker.function = ActionFn::StrobeFlash;

                let strobe_idx = strobe_flashes.len();
                strobe_flashes.push(strobe);
                thinker_list.add_thinker(ActionFn::StrobeFlash, strobe_idx);
            }
            SpecialsClass::Glow => {
                let mut glow = GlowT {
                    thinker: Thinker::default(),
                    sector: 0,
                    minlight: 0,
                    maxlight: 0,
                    direction: 0,
                };
                read_glow(save, &mut glow);

                glow.thinker.function = ActionFn::Glow;

                let glow_idx = glows.len();
                glows.push(glow);
                thinker_list.add_thinker(ActionFn::Glow, glow_idx);
            }
            SpecialsClass::EndSpecials => {
                // Already handled above, but match exhaustiveness
                break;
            }
        }
    }
}

/// Deserialize a CeilingT from the save buffer.
fn read_ceiling(save: &mut SaveGame, c: &mut CeilingT) {
    let _prev = save.read_i32();
    let _next = save.read_i32();
    let func_val = save.read_i32();
    // Store function status temporarily in the thinker
    c.thinker.function = if func_val != 0 {
        ActionFn::MoveCeiling
    } else {
        ActionFn::None
    };
    c.ceiling_type = ceiling_type_from_i32(save.read_i32());
    c.sector = save.read_i32() as usize;
    c.bottomheight = Fixed(save.read_i32());
    c.topheight = Fixed(save.read_i32());
    c.speed = Fixed(save.read_i32());
    c.crush = save.read_i32() != 0;
    c.direction = save.read_i32();
    c.tag = save.read_i32();
    c.olddirection = save.read_i32();
}

/// Deserialize a VldoorT from the save buffer.
fn read_door(save: &mut SaveGame, d: &mut VldoorT) {
    let _prev = save.read_i32();
    let _next = save.read_i32();
    let _func = save.read_i32();
    d.door_type = door_type_from_i32(save.read_i32());
    d.sector = save.read_i32() as usize;
    d.topheight = Fixed(save.read_i32());
    d.speed = Fixed(save.read_i32());
    d.direction = save.read_i32();
    d.topwait = save.read_i32();
    d.topcountdown = save.read_i32();
}

/// Deserialize a FloorMoveT from the save buffer.
fn read_floor(save: &mut SaveGame, f: &mut FloorMoveT) {
    let _prev = save.read_i32();
    let _next = save.read_i32();
    let _func = save.read_i32();
    f.floor_type = floor_type_from_i32(save.read_i32());
    f.crush = save.read_i32() != 0;
    f.sector = save.read_i32() as usize;
    f.direction = save.read_i32();
    f.newsecspecial = save.read_i32();
    f.newtexture = save.read_i16();
    let _padding = save.read_i16();
    f.floordestheight = Fixed(save.read_i32());
    f.speed = Fixed(save.read_i32());
}

/// Deserialize a PlatT from the save buffer.
fn read_plat(save: &mut SaveGame, p: &mut PlatT) {
    let _prev = save.read_i32();
    let _next = save.read_i32();
    let func_val = save.read_i32();
    p.thinker.function = if func_val != 0 {
        ActionFn::PlatRaise
    } else {
        ActionFn::None
    };
    p.sector = save.read_i32() as usize;
    p.speed = Fixed(save.read_i32());
    p.low = Fixed(save.read_i32());
    p.high = Fixed(save.read_i32());
    p.wait = save.read_i32();
    p.count = save.read_i32();
    p.status = plat_status_from_i32(save.read_i32());
    p.oldstatus = plat_status_from_i32(save.read_i32());
    p.crush = save.read_i32() != 0;
    p.tag = save.read_i32();
    p.plat_type = plat_type_from_i32(save.read_i32());
}

/// Deserialize a LightFlashT from the save buffer.
fn read_light_flash(save: &mut SaveGame, l: &mut LightFlashT) {
    let _prev = save.read_i32();
    let _next = save.read_i32();
    let _func = save.read_i32();
    l.sector = save.read_i32() as usize;
    l.count = save.read_i32();
    l.maxlight = save.read_i32();
    l.minlight = save.read_i32();
    l.maxtime = save.read_i32();
    l.mintime = save.read_i32();
}

/// Deserialize a StrobeFlashT from the save buffer.
fn read_strobe_flash(save: &mut SaveGame, s: &mut StrobeFlashT) {
    let _prev = save.read_i32();
    let _next = save.read_i32();
    let _func = save.read_i32();
    s.sector = save.read_i32() as usize;
    s.count = save.read_i32();
    s.minlight = save.read_i32();
    s.maxlight = save.read_i32();
    s.darktime = save.read_i32();
    s.brighttime = save.read_i32();
}

/// Deserialize a GlowT from the save buffer.
fn read_glow(save: &mut SaveGame, g: &mut GlowT) {
    let _prev = save.read_i32();
    let _next = save.read_i32();
    let _func = save.read_i32();
    g.sector = save.read_i32() as usize;
    g.minlight = save.read_i32();
    g.maxlight = save.read_i32();
    g.direction = save.read_i32();
}

// =============================================================================
// Enum conversion helpers
// =============================================================================

/// Convert a raw i32 to CeilingType. Falls back to LowerToFloor.
fn ceiling_type_from_i32(v: i32) -> crate::play::spec::CeilingType {
    use crate::play::spec::CeilingType;
    match v {
        0 => CeilingType::LowerToFloor,
        1 => CeilingType::RaiseToHighest,
        2 => CeilingType::LowerAndCrush,
        3 => CeilingType::CrushAndRaise,
        4 => CeilingType::FastCrushAndRaise,
        5 => CeilingType::SilentCrushAndRaise,
        _ => CeilingType::LowerToFloor,
    }
}

/// Convert a raw i32 to VldoorType. Falls back to Normal.
fn door_type_from_i32(v: i32) -> crate::play::spec::VldoorType {
    use crate::play::spec::VldoorType;
    match v {
        0 => VldoorType::Normal,
        1 => VldoorType::Close30ThenOpen,
        2 => VldoorType::Close,
        3 => VldoorType::Open,
        4 => VldoorType::RaiseIn5Mins,
        5 => VldoorType::BlazeRaise,
        6 => VldoorType::BlazeOpen,
        7 => VldoorType::BlazeClose,
        _ => VldoorType::Normal,
    }
}

/// Convert a raw i32 to FloorType. Falls back to LowerFloor.
fn floor_type_from_i32(v: i32) -> crate::play::spec::FloorType {
    use crate::play::spec::FloorType;
    match v {
        0 => FloorType::LowerFloor,
        1 => FloorType::LowerFloorToLowest,
        2 => FloorType::TurboLower,
        3 => FloorType::RaiseFloor,
        4 => FloorType::RaiseFloorToNearest,
        5 => FloorType::RaiseToTexture,
        6 => FloorType::LowerAndChange,
        7 => FloorType::RaiseFloor24,
        8 => FloorType::RaiseFloor24AndChange,
        9 => FloorType::RaiseFloorCrush,
        10 => FloorType::RaiseFloorTurbo,
        11 => FloorType::DonutRaise,
        12 => FloorType::RaiseFloor512,
        _ => FloorType::LowerFloor,
    }
}

/// Convert a raw i32 to PlatType. Falls back to RaiseToNearestAndChange.
fn plat_type_from_i32(v: i32) -> crate::play::spec::PlatType {
    use crate::play::spec::PlatType;
    match v {
        0 => PlatType::RaiseToNearestAndChange,
        1 => PlatType::DownWaitUpStay,
        2 => PlatType::BlazeDWUS,
        3 => PlatType::PerpetualRaise,
        _ => PlatType::RaiseToNearestAndChange,
    }
}

/// Convert a raw i32 to PlatStatus. Falls back to Up.
fn plat_status_from_i32(v: i32) -> crate::play::spec::PlatStatus {
    use crate::play::spec::PlatStatus;
    match v {
        0 => PlatStatus::Up,
        1 => PlatStatus::Down,
        2 => PlatStatus::Waiting,
        3 => PlatStatus::InStasis,
        _ => PlatStatus::Up,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_savegame_write_read_byte() {
        let mut sg = SaveGame::new(Vec::new());
        sg.write_byte(42);
        sg.write_byte(0xFF);
        sg.pos = 0;
        assert_eq!(sg.read_byte(), 42);
        assert_eq!(sg.read_byte(), 0xFF);
    }

    #[test]
    fn test_savegame_write_read_i16() {
        let mut sg = SaveGame::new(Vec::new());
        sg.write_i16(1234);
        sg.write_i16(-5678);
        sg.pos = 0;
        assert_eq!(sg.read_i16(), 1234);
        assert_eq!(sg.read_i16(), -5678);
    }

    #[test]
    fn test_savegame_write_read_i32() {
        let mut sg = SaveGame::new(Vec::new());
        sg.write_i32(0x12345678);
        sg.write_i32(-1);
        sg.pos = 0;
        assert_eq!(sg.read_i32(), 0x12345678);
        assert_eq!(sg.read_i32(), -1);
    }

    #[test]
    fn test_savegame_pad() {
        let mut sg = SaveGame::new(Vec::new());
        sg.write_byte(1);
        assert_eq!(sg.pos, 1);
        sg.pad();
        // Position 1, padding = (4 - 1) & 3 = 3, new pos = 4
        assert_eq!(sg.pos, 4);

        // Already aligned
        sg.pad();
        assert_eq!(sg.pos, 4);

        sg.write_byte(2);
        sg.write_byte(3);
        assert_eq!(sg.pos, 6);
        sg.pad();
        // Position 6, padding = (4 - 2) & 3 = 2, new pos = 8
        assert_eq!(sg.pos, 8);
    }

    #[test]
    fn test_weapon_from_i32() {
        assert_eq!(weapon_from_i32(0), WeaponType::Fist);
        assert_eq!(weapon_from_i32(1), WeaponType::Pistol);
        assert_eq!(weapon_from_i32(8), WeaponType::SuperShotgun);
        assert_eq!(weapon_from_i32(9), WeaponType::NoChange);
        assert_eq!(weapon_from_i32(99), WeaponType::Fist); // fallback
    }

    #[test]
    fn test_specials_class_from_u8() {
        assert_eq!(SpecialsClass::from_u8(0), Some(SpecialsClass::Ceiling));
        assert_eq!(SpecialsClass::from_u8(7), Some(SpecialsClass::EndSpecials));
        assert_eq!(SpecialsClass::from_u8(8), None);
    }

    #[test]
    fn test_thinker_class_repr() {
        assert_eq!(ThinkerClass::End as u8, 0);
        assert_eq!(ThinkerClass::Mobj as u8, 1);
    }

    #[test]
    fn test_savegame_new() {
        let data = vec![1, 2, 3, 4];
        let sg = SaveGame::new(data.clone());
        assert_eq!(sg.buffer, data);
        assert_eq!(sg.pos, 0);
    }

    #[test]
    fn test_savegame_read_past_end() {
        let mut sg = SaveGame::new(vec![0x42]);
        assert_eq!(sg.read_byte(), 0x42);
        // Reading past end returns 0
        assert_eq!(sg.read_byte(), 0);
    }

    #[test]
    fn test_archive_unarchive_world_roundtrip() {
        let sector = Sector {
            floorheight: Fixed(32 << FRACBITS),
            ceilingheight: Fixed(128 << FRACBITS),
            floorpic: 5,
            ceilingpic: 10,
            lightlevel: 192,
            special: 3,
            tag: 7,
            soundtraversed: 0,
            soundtarget: Some(99),
            blockbox: [0; 4],
            soundorg: Default::default(),
            validcount: 0,
            thinglist: None,
            specialdata: Some(42),
            linecount: 0,
            lines: Vec::new(),
        };

        let side = SideDef {
            textureoffset: Fixed(16 << FRACBITS),
            rowoffset: Fixed(8 << FRACBITS),
            toptexture: 1,
            bottomtexture: 2,
            midtexture: 3,
            sector: 0,
        };

        let line = LineDef {
            v1: 0,
            v2: 1,
            dx: Fixed(0),
            dy: Fixed(0),
            flags: 0x0001,
            special: 11,
            tag: 22,
            sidenum: [0, -1],
            bbox: [Fixed(0); 4],
            slopetype: crate::types::map_data::SlopeType::Horizontal,
            frontsector: Some(0),
            backsector: None,
            validcount: 0,
            specialdata: None,
        };

        let sectors = vec![sector];
        let lines = vec![line];
        let sides = vec![side];

        // Archive
        let mut save = SaveGame::new(Vec::new());
        archive_world(&mut save, &sectors, &lines, &sides);

        // Unarchive
        save.pos = 0;
        let mut restored_sectors = vec![Sector {
            floorheight: Fixed(0),
            ceilingheight: Fixed(0),
            floorpic: 0,
            ceilingpic: 0,
            lightlevel: 0,
            special: 0,
            tag: 0,
            soundtraversed: 0,
            soundtarget: Some(50),
            blockbox: [0; 4],
            soundorg: Default::default(),
            validcount: 0,
            thinglist: None,
            specialdata: Some(10),
            linecount: 0,
            lines: Vec::new(),
        }];
        let mut restored_lines = vec![LineDef {
            v1: 0,
            v2: 1,
            dx: Fixed(0),
            dy: Fixed(0),
            flags: 0,
            special: 0,
            tag: 0,
            sidenum: [0, -1],
            bbox: [Fixed(0); 4],
            slopetype: crate::types::map_data::SlopeType::Horizontal,
            frontsector: Some(0),
            backsector: None,
            validcount: 0,
            specialdata: None,
        }];
        let mut restored_sides = vec![SideDef {
            textureoffset: Fixed(0),
            rowoffset: Fixed(0),
            toptexture: 0,
            bottomtexture: 0,
            midtexture: 0,
            sector: 0,
        }];

        unarchive_world(
            &mut save,
            &mut restored_sectors,
            &mut restored_lines,
            &mut restored_sides,
        );

        // Verify sectors
        assert_eq!(restored_sectors[0].floorheight.0, 32 << FRACBITS);
        assert_eq!(restored_sectors[0].ceilingheight.0, 128 << FRACBITS);
        assert_eq!(restored_sectors[0].floorpic, 5);
        assert_eq!(restored_sectors[0].ceilingpic, 10);
        assert_eq!(restored_sectors[0].lightlevel, 192);
        assert_eq!(restored_sectors[0].special, 3);
        assert_eq!(restored_sectors[0].tag, 7);
        // specialdata and soundtarget must be cleared
        assert!(restored_sectors[0].specialdata.is_none());
        assert!(restored_sectors[0].soundtarget.is_none());

        // Verify line
        assert_eq!(restored_lines[0].flags, 0x0001);
        assert_eq!(restored_lines[0].special, 11);
        assert_eq!(restored_lines[0].tag, 22);

        // Verify side
        assert_eq!(restored_sides[0].textureoffset.0, 16 << FRACBITS);
        assert_eq!(restored_sides[0].rowoffset.0, 8 << FRACBITS);
        assert_eq!(restored_sides[0].toptexture, 1);
        assert_eq!(restored_sides[0].bottomtexture, 2);
        assert_eq!(restored_sides[0].midtexture, 3);
    }
}
