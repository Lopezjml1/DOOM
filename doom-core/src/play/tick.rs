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

//! Thinker management and per-tic simulation driver.
//!
//! Translated from linuxdoom-1.10/p_tick.c and p_tick.h
//!
//! This module implements the core thinker linked-list management that drives
//! all gameplay entity simulation in DOOM. Every active entity — monsters,
//! doors, platforms, lights, etc. — has a thinker that gets dispatched once
//! per game tick (35 Hz).
//!
//! # Original C functions translated
//!
//! | Rust function | C function | Description |
//! |---------------|------------|-------------|
//! | `p_init_thinkers` | `P_InitThinkers` | Reset thinker list to empty sentinel |
//! | `p_add_thinker` | `P_AddThinker` | Append thinker to end of list |
//! | `p_remove_thinker` | `P_RemoveThinker` | Mark thinker for lazy removal |
//! | `p_run_thinkers` | `P_RunThinkers` | Iterate list, dispatch or remove |
//! | `p_ticker` | `P_Ticker` | Per-tic simulation entry point |
//!
//! # Design decisions
//!
//! The C code uses a circular doubly-linked list with a sentinel `thinkercap`
//! node. Thinkers are allocated via `Z_Malloc` and freed via `Z_Free`. Removal
//! is lazy: `P_RemoveThinker` sets the function pointer to a sentinel value
//! (-1 cast), and the next `P_RunThinkers` pass unlinks and frees it.
//!
//! In this Rust port, thinkers are stored in a `Vec<ThinkerEntry>` arena.
//! Each entry has an `ActionFn` enum for dispatch and `Option<usize>` indices
//! for the doubly-linked list. The sentinel approach is preserved via
//! `ActionFn::PendingRemoval` to maintain behavioral parity with the original.

use crate::types::doomdef::MAXPLAYERS;
use crate::types::thinker::ActionFn;

// =============================================================================
// Thinker arena entry (replaces Z_Malloc'd thinker_t nodes)
// =============================================================================

/// A single entry in the thinker arena, participating in a doubly-linked list.
///
/// Replaces the C `thinker_t` struct which used raw `prev`/`next` pointers
/// and a `Z_Malloc`'d allocation. The arena-index approach enables safe Rust
/// traversal without raw pointer manipulation.
#[derive(Debug, Clone)]
pub struct ThinkerEntry {
    /// The action function to dispatch for this thinker, or `PendingRemoval`
    /// if marked for lazy deletion, or `None` for the sentinel node.
    pub action: ActionFn,

    /// Next entry index in the thinker list (circular).
    pub next: Option<usize>,

    /// Previous entry index in the thinker list (circular).
    pub prev: Option<usize>,

    /// Whether this slot in the arena is actively in use.
    pub active: bool,

    /// Opaque data index — references the concrete thinker data (door, ceiling,
    /// platform, mobj, etc.) in its respective storage. The dispatcher uses
    /// `action` to determine which storage to look up.
    pub data_index: usize,
}

impl Default for ThinkerEntry {
    fn default() -> Self {
        Self {
            action: ActionFn::None,
            next: None,
            prev: None,
            active: false,
            data_index: 0,
        }
    }
}

// =============================================================================
// Thinker list state
// =============================================================================

/// Manages the doubly-linked list of active thinkers.
///
/// Replaces the C global `thinker_t thinkercap` sentinel and the implicit
/// linked-list operations scattered across `p_tick.c`.
///
/// The list is circular: `head` is the sentinel node whose `next` points to
/// the first real thinker and whose `prev` points to the last. An empty list
/// has `head.next == head_index` and `head.prev == head_index`.
#[derive(Debug, Clone)]
pub struct ThinkerList {
    /// Arena of thinker entries. Index 0 is reserved for the sentinel (head).
    pub entries: Vec<ThinkerEntry>,

    /// Index of the sentinel/head node (always 0 after init).
    pub head: usize,

    /// Free-list of reusable arena slots (indices of inactive entries).
    free_slots: Vec<usize>,
}

impl Default for ThinkerList {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkerList {
    /// Create a new thinker list with only the sentinel node.
    pub fn new() -> Self {
        // Entry 0 is the sentinel (thinkercap equivalent).
        let sentinel = ThinkerEntry {
            action: ActionFn::None,
            next: Some(0),
            prev: Some(0),
            active: true,
            data_index: 0,
        };

        Self {
            entries: vec![sentinel],
            head: 0,
            free_slots: Vec::new(),
        }
    }

    /// Reset the thinker list to empty (sentinel only).
    ///
    /// Equivalent to `P_InitThinkers` in p_tick.c:
    /// ```c
    /// void P_InitThinkers(void) {
    ///     thinkercap.prev = thinkercap.next = &thinkercap;
    /// }
    /// ```
    pub fn init_thinkers(&mut self) {
        self.entries.clear();
        self.free_slots.clear();

        let sentinel = ThinkerEntry {
            action: ActionFn::None,
            next: Some(0),
            prev: Some(0),
            active: true,
            data_index: 0,
        };

        self.entries.push(sentinel);
        self.head = 0;
    }

    /// Add a new thinker at the end of the list.
    ///
    /// Equivalent to `P_AddThinker` in p_tick.c:
    /// ```c
    /// void P_AddThinker(thinker_t* thinker) {
    ///     thinkercap.prev->next = thinker;
    ///     thinker->next = &thinkercap;
    ///     thinker->prev = thinkercap.prev;
    ///     thinkercap.prev = thinker;
    /// }
    /// ```
    ///
    /// Returns the arena index of the newly added thinker entry.
    pub fn add_thinker(&mut self, action: ActionFn, data_index: usize) -> usize {
        let new_entry = ThinkerEntry {
            action,
            next: Some(self.head),
            prev: self.entries[self.head].prev,
            active: true,
            data_index,
        };

        // Allocate slot: reuse from free list or push new.
        let new_idx = if let Some(idx) = self.free_slots.pop() {
            self.entries[idx] = new_entry;
            idx
        } else {
            let idx = self.entries.len();
            self.entries.push(new_entry);
            idx
        };

        // Link: old_tail.next = new, head.prev = new
        let old_tail = self.entries[self.head].prev.unwrap_or(self.head);
        self.entries[old_tail].next = Some(new_idx);
        self.entries[self.head].prev = Some(new_idx);

        new_idx
    }

    /// Mark a thinker for lazy removal.
    ///
    /// Equivalent to `P_RemoveThinker` in p_tick.c:
    /// ```c
    /// void P_RemoveThinker(thinker_t* thinker) {
    ///     thinker->function.acv = (actionf_v)(-1);
    /// }
    /// ```
    ///
    /// The thinker is not actually unlinked until `run_thinkers` encounters it.
    /// This two-phase approach prevents iterator invalidation during traversal.
    pub fn remove_thinker(&mut self, idx: usize) {
        if idx < self.entries.len() && idx != self.head {
            self.entries[idx].action = ActionFn::PendingRemoval;
        }
    }

    /// Iterate the thinker list, dispatching active thinkers and removing
    /// those marked `PendingRemoval`.
    ///
    /// Equivalent to `P_RunThinkers` in p_tick.c:
    /// ```c
    /// void P_RunThinkers(void) {
    ///     thinker_t* currentthinker = thinkercap.next;
    ///     while (currentthinker != &thinkercap) {
    ///         if (currentthinker->function.acv == (actionf_v)(-1)) {
    ///             currentthinker->next->prev = currentthinker->prev;
    ///             currentthinker->prev->next = currentthinker->next;
    ///             Z_Free(currentthinker);
    ///         } else {
    ///             if (currentthinker->function.acp1)
    ///                 currentthinker->function.acp1(currentthinker);
    ///         }
    ///         currentthinker = currentthinker->next;
    ///     }
    /// }
    /// ```
    ///
    /// Returns a `Vec` of `(ActionFn, data_index)` pairs for thinkers that
    /// need to be dispatched. The caller is responsible for actually calling
    /// the appropriate handler for each action type, since the thinker list
    /// does not own the concrete thinker data (doors, ceilings, etc.).
    pub fn run_thinkers(&mut self) -> Vec<(ActionFn, usize)> {
        let mut dispatch_list = Vec::new();
        let mut current = self.entries[self.head].next.unwrap_or(self.head);

        while current != self.head {
            let next = self.entries[current].next.unwrap_or(self.head);

            if self.entries[current].action == ActionFn::PendingRemoval {
                // Unlink and free
                let prev = self.entries[current].prev.unwrap_or(self.head);
                let next_idx = self.entries[current].next.unwrap_or(self.head);

                self.entries[prev].next = Some(next_idx);
                self.entries[next_idx].prev = Some(prev);

                self.entries[current].active = false;
                self.entries[current].next = None;
                self.entries[current].prev = None;
                self.free_slots.push(current);
            } else if self.entries[current].action != ActionFn::None {
                // Dispatch: collect for caller to process
                dispatch_list.push((
                    self.entries[current].action,
                    self.entries[current].data_index,
                ));
            }

            current = next;
        }

        dispatch_list
    }

    /// Return the number of active (non-sentinel, non-free) thinkers.
    pub fn count(&self) -> usize {
        self.entries
            .iter()
            .enumerate()
            .filter(|(i, e)| *i != self.head && e.active && e.action != ActionFn::PendingRemoval)
            .count()
    }

    /// Iterate over all active thinker entries (excluding sentinel and pending removal).
    ///
    /// Returns `(arena_index, action, data_index)` tuples.
    pub fn iter_active(&self) -> Vec<(usize, ActionFn, usize)> {
        let mut result = Vec::new();
        let mut current = self.entries[self.head].next.unwrap_or(self.head);

        while current != self.head {
            let entry = &self.entries[current];
            if entry.active && entry.action != ActionFn::PendingRemoval {
                result.push((current, entry.action, entry.data_index));
            }
            current = entry.next.unwrap_or(self.head);
        }

        result
    }
}

// =============================================================================
// Tick state — per-level timing
// =============================================================================

/// Per-level timing state, replacing the C global `int leveltime` from p_tick.c.
#[derive(Debug, Clone, Default)]
pub struct TickState {
    /// Level time in tics (incremented once per P_Ticker call when unpaused).
    ///
    /// Original C: `int leveltime;` (p_tick.c line 36)
    /// Used for par time comparison on intermission screen and periodic
    /// effects (e.g., ceiling sound every 8 tics).
    pub leveltime: i32,
}

// =============================================================================
// P_Ticker — top-level per-tic simulation driver
// =============================================================================

/// Context trait providing all state needed by `p_ticker`.
///
/// The concrete implementation wires together the game state subsystems.
pub trait TickContext {
    /// Whether the game is paused.
    fn paused(&self) -> bool;

    /// Whether a network game is in progress.
    fn netgame(&self) -> bool;

    /// Whether the menu is active.
    fn menu_active(&self) -> bool;

    /// Whether demo playback is in progress.
    fn demo_playback(&self) -> bool;

    /// Console player index.
    fn console_player(&self) -> usize;

    /// Access player data by index.
    fn player_viewz(&self, idx: usize) -> i32;

    /// Whether player `idx` is in the game.
    fn player_in_game(&self, idx: usize) -> bool;

    /// Run `P_PlayerThink` for the given player.
    fn p_player_think(&mut self, player_idx: usize);

    /// Access the thinker list.
    fn thinker_list(&self) -> &ThinkerList;

    /// Mutably access the thinker list.
    fn thinker_list_mut(&mut self) -> &mut ThinkerList;

    /// Dispatch a single thinker action (called for each active thinker).
    ///
    /// The implementation should match on `action` and call the appropriate
    /// handler (T_MoveCeiling, T_VerticalDoor, T_MoveFloor, T_PlatRaise,
    /// T_FireFlicker, T_LightFlash, T_StrobeFlash, T_Glow, P_MobjThinker).
    fn dispatch_thinker(&mut self, action: ActionFn, data_index: usize);

    /// Run P_UpdateSpecials (animation, button timers, etc.).
    fn p_update_specials(&mut self);

    /// Run P_RespawnSpecials (deathmatch item respawn).
    fn p_respawn_specials(&mut self);

    /// Access tick state.
    fn tick_state(&self) -> &TickState;

    /// Mutably access tick state.
    fn tick_state_mut(&mut self) -> &mut TickState;
}

/// Per-tic simulation driver. Called once per game tic (35 Hz).
///
/// Carries out all thinking of monsters, players, and specials.
///
/// Equivalent to `P_Ticker` in p_tick.c:
/// ```c
/// void P_Ticker(void) {
///     int i;
///     if (paused) return;
///     if (!netgame && menuactive && !demoplayback
///         && players[consoleplayer].viewz != 1) return;
///     for (i=0; i<MAXPLAYERS; i++)
///         if (playeringame[i])
///             P_PlayerThink(&players[i]);
///     P_RunThinkers();
///     P_UpdateSpecials();
///     P_RespawnSpecials();
///     leveltime++;
/// }
/// ```
pub fn p_ticker(ctx: &mut dyn TickContext) {
    // Run the tic — bail if paused.
    if ctx.paused() {
        return;
    }

    // Pause if in menu and at least one tic has been run (single-player only).
    // Original check: `players[consoleplayer].viewz != 1` ensures at least one
    // tic has executed (viewz is initialized to 1 before the first tic).
    if !ctx.netgame() && ctx.menu_active() && !ctx.demo_playback() {
        let cp = ctx.console_player();
        if ctx.player_viewz(cp) != 1 {
            return;
        }
    }

    // Run player thinking for all active players.
    for i in 0..MAXPLAYERS {
        if ctx.player_in_game(i) {
            ctx.p_player_think(i);
        }
    }

    // Run thinkers: collect dispatch list, then dispatch each.
    let dispatch_list = ctx.thinker_list_mut().run_thinkers();
    for (action, data_index) in dispatch_list {
        ctx.dispatch_thinker(action, data_index);
    }

    // Update animations, button timers, scrolling specials.
    ctx.p_update_specials();

    // Respawn items in deathmatch.
    ctx.p_respawn_specials();

    // Increment level time (for par times).
    ctx.tick_state_mut().leveltime += 1;
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thinker_list_init() {
        let list = ThinkerList::new();
        assert_eq!(list.entries.len(), 1, "Should have only sentinel");
        assert_eq!(list.head, 0);
        assert_eq!(list.entries[0].next, Some(0));
        assert_eq!(list.entries[0].prev, Some(0));
        assert_eq!(list.count(), 0);
    }

    #[test]
    fn test_add_thinker() {
        let mut list = ThinkerList::new();

        let idx1 = list.add_thinker(ActionFn::MobjThinker, 42);
        assert_eq!(idx1, 1);
        assert_eq!(list.count(), 1);
        assert_eq!(list.entries[idx1].action, ActionFn::MobjThinker);
        assert_eq!(list.entries[idx1].data_index, 42);

        // Verify linking: sentinel -> thinker1 -> sentinel
        assert_eq!(list.entries[0].next, Some(1));
        assert_eq!(list.entries[0].prev, Some(1));
        assert_eq!(list.entries[1].next, Some(0));
        assert_eq!(list.entries[1].prev, Some(0));
    }

    #[test]
    fn test_add_multiple_thinkers() {
        let mut list = ThinkerList::new();

        let idx1 = list.add_thinker(ActionFn::MobjThinker, 10);
        let idx2 = list.add_thinker(ActionFn::VerticalDoor, 20);
        let idx3 = list.add_thinker(ActionFn::MoveFloor, 30);

        assert_eq!(list.count(), 3);

        // Verify order: sentinel -> idx1 -> idx2 -> idx3 -> sentinel
        assert_eq!(list.entries[0].next, Some(idx1));
        assert_eq!(list.entries[idx1].next, Some(idx2));
        assert_eq!(list.entries[idx2].next, Some(idx3));
        assert_eq!(list.entries[idx3].next, Some(0));

        // Reverse: sentinel -> idx3 -> idx2 -> idx1 -> sentinel
        assert_eq!(list.entries[0].prev, Some(idx3));
        assert_eq!(list.entries[idx3].prev, Some(idx2));
        assert_eq!(list.entries[idx2].prev, Some(idx1));
        assert_eq!(list.entries[idx1].prev, Some(0));
    }

    #[test]
    fn test_remove_thinker_lazy() {
        let mut list = ThinkerList::new();

        let idx1 = list.add_thinker(ActionFn::MobjThinker, 10);
        let _idx2 = list.add_thinker(ActionFn::VerticalDoor, 20);

        // Mark for removal — count should still include it until run_thinkers
        list.remove_thinker(idx1);
        assert_eq!(list.entries[idx1].action, ActionFn::PendingRemoval);
    }

    #[test]
    fn test_run_thinkers_removes_and_dispatches() {
        let mut list = ThinkerList::new();

        let idx1 = list.add_thinker(ActionFn::MobjThinker, 10);
        let _idx2 = list.add_thinker(ActionFn::VerticalDoor, 20);
        let _idx3 = list.add_thinker(ActionFn::MoveFloor, 30);

        // Mark idx1 for removal
        list.remove_thinker(idx1);

        // Run thinkers
        let dispatch = list.run_thinkers();

        // idx1 should be removed, idx2 and idx3 dispatched
        assert_eq!(dispatch.len(), 2);
        assert_eq!(dispatch[0], (ActionFn::VerticalDoor, 20));
        assert_eq!(dispatch[1], (ActionFn::MoveFloor, 30));

        // After run, count should be 2
        assert_eq!(list.count(), 2);

        // idx1 slot should be inactive
        assert!(!list.entries[idx1].active);
    }

    #[test]
    fn test_init_thinkers_resets() {
        let mut list = ThinkerList::new();
        list.add_thinker(ActionFn::MobjThinker, 1);
        list.add_thinker(ActionFn::VerticalDoor, 2);

        list.init_thinkers();

        assert_eq!(
            list.entries.len(),
            1,
            "Should have only sentinel after init"
        );
        assert_eq!(list.count(), 0);
        assert_eq!(list.entries[0].next, Some(0));
        assert_eq!(list.entries[0].prev, Some(0));
    }

    #[test]
    fn test_slot_reuse() {
        let mut list = ThinkerList::new();

        let idx1 = list.add_thinker(ActionFn::MobjThinker, 10);
        let _idx2 = list.add_thinker(ActionFn::VerticalDoor, 20);

        // Remove idx1 and process
        list.remove_thinker(idx1);
        list.run_thinkers();

        // Add new thinker — should reuse idx1's slot
        let idx3 = list.add_thinker(ActionFn::MoveFloor, 30);
        assert_eq!(idx3, idx1, "Should reuse freed slot");
        assert_eq!(list.entries[idx3].action, ActionFn::MoveFloor);
        assert_eq!(list.entries[idx3].data_index, 30);
        assert!(list.entries[idx3].active);
    }

    #[test]
    fn test_cannot_remove_sentinel() {
        let mut list = ThinkerList::new();
        list.remove_thinker(0);
        // Sentinel should remain unchanged
        assert_eq!(list.entries[0].action, ActionFn::None);
    }

    #[test]
    fn test_iter_active() {
        let mut list = ThinkerList::new();

        list.add_thinker(ActionFn::MobjThinker, 10);
        list.add_thinker(ActionFn::FireFlicker, 20);
        list.add_thinker(ActionFn::PlatRaise, 30);

        let active: Vec<_> = list.iter_active();
        assert_eq!(active.len(), 3);
        assert_eq!(active[0].1, ActionFn::MobjThinker);
        assert_eq!(active[1].1, ActionFn::FireFlicker);
        assert_eq!(active[2].1, ActionFn::PlatRaise);
    }

    #[test]
    fn test_empty_run_thinkers() {
        let mut list = ThinkerList::new();
        let dispatch = list.run_thinkers();
        assert!(dispatch.is_empty());
    }
}
