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
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
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
//! # Architecture
//!
//! The C code uses a circular doubly-linked list with a sentinel `thinkercap`
//! node and raw pointer manipulation. This Rust port uses an arena-based
//! approach: thinkers are stored in a `Vec<ThinkerEntry>` where each entry
//! wraps a [`Thinker`] with arena management metadata. The sentinel concept
//! is preserved (entry 0 = thinkercap), and lazy removal via
//! [`ActionFn::PendingRemoval`] maintains behavioral parity with the original.
//!
//! # Original C functions translated
//!
//! | Rust function | C function | Description |
//! |---|---|---|
//! | [`p_init_thinkers`] | `P_InitThinkers` | Reset thinker list to empty sentinel |
//! | [`p_add_thinker`] | `P_AddThinker` | Append thinker to end of list |
//! | [`p_remove_thinker`] | `P_RemoveThinker` | Mark thinker for lazy removal |
//! | [`p_allocate_thinker`] | `P_AllocateThinker` | Empty stub (preserved for API parity) |
//! | [`p_run_thinkers`] | `P_RunThinkers` | Iterate list, dispatch or remove |
//! | [`p_ticker`] | `P_Ticker` | Per-tic simulation entry point |
//!
//! # Cross-module dependencies
//!
//! `P_Ticker` orchestrates per-tic calls to:
//! - [`user::p_player_think`] — player input processing, movement, powerups
//! - [`spec::p_update_specials`] — animated textures, scrolling, button timers
//! - [`mobj::p_respawn_specials`] — item respawning in deathmatch/nightmare

use crate::types::doomdef::MAXPLAYERS;
use crate::types::fixed::Fixed;
use crate::types::player::Player;
use crate::types::thinker::{ActionFn, Thinker};

// Sibling play module dependencies. These functions are called indirectly
// through the TickContext trait. The trait implementation wires its methods
// to the concrete functions in these modules:
//   - user::p_player_think  -> TickContext::p_player_think
//   - spec::p_update_specials -> TickContext::p_update_specials
//   - mobj::p_respawn_specials -> TickContext::p_respawn_specials
#[allow(unused_imports)]
use crate::play::mobj;
#[allow(unused_imports)]
use crate::play::spec;
#[allow(unused_imports)]
use crate::play::user;

// =============================================================================
// ThinkerEntry — Arena entry wrapping a Thinker with management metadata
// =============================================================================

/// A single entry in the thinker arena, composing a [`Thinker`] node with
/// arena-specific management fields.
///
/// The [`Thinker`] provides the doubly-linked list structure (`prev`, `next`,
/// `function`), while this wrapper adds the `active` flag for arena slot
/// management and `data_index` for referencing the concrete thinker data
/// (door state, ceiling state, mobj index, etc.).
#[derive(Debug, Clone, Default)]
pub struct ThinkerEntry {
    /// The base thinker node with linked-list pointers and action dispatch enum.
    ///
    /// The `thinker.function` field determines which handler is called during
    /// [`p_run_thinkers`]. [`ActionFn::PendingRemoval`] marks the entry for
    /// lazy deletion. [`ActionFn::None`] is used for the sentinel node.
    pub thinker: Thinker,

    /// Whether this slot in the arena is actively in use.
    ///
    /// When a thinker is removed during [`p_run_thinkers`], this is set to
    /// `false` and the slot index is pushed onto the free list for reuse.
    pub active: bool,

    /// Opaque data index — references the concrete thinker data (door, ceiling,
    /// platform, mobj, etc.) in its respective storage. The dispatcher uses
    /// `thinker.function` to determine which storage to look up.
    pub data_index: usize,
}

// =============================================================================
// ThinkerList — Arena-based thinker management (thinkercap equivalent)
// =============================================================================

/// Manages the doubly-linked list of active thinkers.
///
/// This is the Rust equivalent of the C `thinkercap` sentinel variable combined
/// with the implicit linked-list formed by `thinker_t::prev`/`next` pointers.
///
/// The list is circular: entry 0 is the sentinel (thinkercap). The sentinel's
/// `thinker.next` points to the first real thinker and `thinker.prev` points to
/// the last. An empty list has both pointing to index 0 (self-referential).
///
/// # Arena design
///
/// Thinker entries are stored in a `Vec<ThinkerEntry>`. Removed entries are
/// deactivated and their indices pushed onto a free-list for O(1) reuse.
/// This avoids repeated heap allocation/deallocation while preserving the
/// O(1) insert/remove semantics of the original doubly-linked list.
#[derive(Debug, Clone)]
pub struct ThinkerList {
    /// Arena of thinker entries. Index 0 is reserved for the sentinel (thinkercap).
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
    ///
    /// The sentinel's `thinker.prev` and `thinker.next` both point to itself,
    /// creating an empty circular list — exactly matching `P_InitThinkers`.
    pub fn new() -> Self {
        let sentinel = ThinkerEntry {
            thinker: Thinker {
                prev: Some(0),
                next: Some(0),
                function: ActionFn::None,
            },
            active: true,
            data_index: 0,
        };

        Self {
            entries: vec![sentinel],
            head: 0,
            free_slots: Vec::new(),
        }
    }

    /// Access the sentinel node (thinkercap equivalent).
    ///
    /// The sentinel's `prev` points to the last thinker and `next` points to
    /// the first thinker in the circular list. These fields, along with
    /// `function`, are the members exposed by the `thinkercap` export.
    #[inline]
    pub fn thinkercap(&self) -> &Thinker {
        &self.entries[self.head].thinker
    }

    /// Mutable access to the sentinel node (thinkercap equivalent).
    #[inline]
    pub fn thinkercap_mut(&mut self) -> &mut Thinker {
        &mut self.entries[self.head].thinker
    }

    /// Reset the thinker list to empty (sentinel only).
    ///
    /// Equivalent to `P_InitThinkers` in p_tick.c lines 53-56:
    /// ```c
    /// void P_InitThinkers(void) {
    ///     thinkercap.prev = thinkercap.next = &thinkercap;
    /// }
    /// ```
    ///
    /// All existing thinker entries are dropped, and the sentinel is
    /// re-initialized as a self-referential circular list of one node.
    pub fn init_thinkers(&mut self) {
        self.entries.clear();
        self.free_slots.clear();

        let sentinel = ThinkerEntry {
            thinker: Thinker {
                prev: Some(0),
                next: Some(0),
                function: ActionFn::None,
            },
            active: true,
            data_index: 0,
        };

        self.entries.push(sentinel);
        self.head = 0;
    }

    /// Add a new thinker at the end of the list (before thinkercap).
    ///
    /// Equivalent to `P_AddThinker` in p_tick.c lines 65-71:
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
        // Get old tail (thinkercap.prev)
        let old_tail = self.entries[self.head].thinker.prev.unwrap_or(self.head);

        // Build the new entry with Thinker properly linked
        let new_entry = ThinkerEntry {
            thinker: Thinker {
                next: Some(self.head), // thinker->next = &thinkercap
                prev: Some(old_tail),  // thinker->prev = thinkercap.prev
                function: action,
            },
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

        // thinkercap.prev->next = thinker  (old tail points forward to new)
        self.entries[old_tail].thinker.next = Some(new_idx);
        // thinkercap.prev = thinker  (sentinel points backward to new tail)
        self.entries[self.head].thinker.prev = Some(new_idx);

        new_idx
    }

    /// Mark a thinker for lazy removal.
    ///
    /// Equivalent to `P_RemoveThinker` in p_tick.c lines 80-84:
    /// ```c
    /// void P_RemoveThinker(thinker_t* thinker) {
    ///     thinker->function.acv = (actionf_v)(-1);
    /// }
    /// ```
    ///
    /// The thinker is NOT actually unlinked here — it will be unlinked and
    /// freed during the next [`run_thinkers`](Self::run_thinkers) pass when
    /// `PendingRemoval` is detected. This two-phase approach prevents iterator
    /// invalidation during traversal.
    ///
    /// The sentinel (index 0) cannot be removed.
    pub fn remove_thinker(&mut self, idx: usize) {
        if idx < self.entries.len() && idx != self.head {
            self.entries[idx].thinker.function = ActionFn::PendingRemoval;
        }
    }

    /// Walk the thinker list, dispatching active thinkers and removing those
    /// marked [`ActionFn::PendingRemoval`].
    ///
    /// Equivalent to `P_RunThinkers` in p_tick.c lines 101-122.
    ///
    /// Returns a `Vec<(ActionFn, data_index)>` of thinkers to dispatch. The
    /// caller is responsible for calling the appropriate handler for each
    /// action type. This deferred-dispatch pattern avoids borrow checker
    /// conflicts from modifying the arena while iterating.
    ///
    /// # Removal mechanics
    ///
    /// When a thinker's function is `PendingRemoval`:
    /// 1. Save `next` before any modification (critical for safe traversal).
    /// 2. Unlink: `next_node.prev = current.prev`, `prev_node.next = current.next`.
    /// 3. Deactivate the slot and push its index onto the free list.
    pub fn run_thinkers(&mut self) -> Vec<(ActionFn, usize)> {
        let mut dispatch_list = Vec::new();
        let mut current = self.entries[self.head].thinker.next.unwrap_or(self.head);

        while current != self.head {
            // CRITICAL: Save next BEFORE processing. The C code reads
            // currentthinker->next AFTER the if/else block, which works because
            // removal doesn't zero the next pointer. We save it upfront for safety.
            let next = self.entries[current].thinker.next.unwrap_or(self.head);

            if self.entries[current].thinker.function == ActionFn::PendingRemoval {
                // Unlink from doubly-linked list
                let prev_idx = self.entries[current].thinker.prev.unwrap_or(self.head);
                let next_idx = self.entries[current].thinker.next.unwrap_or(self.head);

                self.entries[prev_idx].thinker.next = Some(next_idx);
                self.entries[next_idx].thinker.prev = Some(prev_idx);

                // Free the slot: deactivate and add to free list
                self.entries[current].active = false;
                self.entries[current].thinker.unlink();
                self.free_slots.push(current);
            } else if self.entries[current].thinker.function != ActionFn::None {
                // Active thinker — collect for dispatch
                dispatch_list.push((
                    self.entries[current].thinker.function,
                    self.entries[current].data_index,
                ));
            }

            current = next;
        }

        dispatch_list
    }

    /// Return the number of active (non-sentinel, non-free) thinkers.
    ///
    /// Does not count the sentinel node or entries pending removal.
    pub fn count(&self) -> usize {
        self.entries
            .iter()
            .enumerate()
            .filter(|(i, e)| {
                *i != self.head && e.active && e.thinker.function != ActionFn::PendingRemoval
            })
            .count()
    }

    /// Iterate over all active thinker entries in list order.
    ///
    /// Returns `(arena_index, action, data_index)` tuples in insertion order.
    /// Entries marked for removal are excluded.
    pub fn iter_active(&self) -> Vec<(usize, ActionFn, usize)> {
        let mut result = Vec::new();
        let mut current = self.entries[self.head].thinker.next.unwrap_or(self.head);

        while current != self.head {
            let entry = &self.entries[current];
            if entry.active && entry.thinker.function != ActionFn::PendingRemoval {
                result.push((current, entry.thinker.function, entry.data_index));
            }
            current = entry.thinker.next.unwrap_or(self.head);
        }

        result
    }
}

// =============================================================================
// Standalone functions — C API name equivalents (schema exports)
// =============================================================================

/// Reset the thinker list to empty (sentinel only).
///
/// Equivalent to `P_InitThinkers` in p_tick.c lines 53-56.
///
/// After this call, the list contains only the self-referential sentinel
/// node (thinkercap). All previous thinker entries are dropped.
#[inline]
pub fn p_init_thinkers(list: &mut ThinkerList) {
    list.init_thinkers();
}

/// Add a new thinker at the end of the list (before thinkercap).
///
/// Equivalent to `P_AddThinker` in p_tick.c lines 65-71.
///
/// Returns the arena index of the newly added thinker entry.
#[inline]
pub fn p_add_thinker(list: &mut ThinkerList, action: ActionFn, data_index: usize) -> usize {
    list.add_thinker(action, data_index)
}

/// Mark a thinker for lazy removal.
///
/// Equivalent to `P_RemoveThinker` in p_tick.c lines 80-84.
///
/// The thinker is not unlinked immediately — it will be removed during the
/// next [`p_run_thinkers`] pass.
#[inline]
pub fn p_remove_thinker(list: &mut ThinkerList, idx: usize) {
    list.remove_thinker(idx);
}

/// Allocate a thinker — empty stub.
///
/// Equivalent to `P_AllocateThinker` in p_tick.c lines 92-94.
/// The original C function body was empty and is preserved here for
/// completeness and API parity.
///
/// ```c
/// void P_AllocateThinker(thinker_t* thinker) {
/// }
/// ```
#[inline]
pub fn p_allocate_thinker(_list: &mut ThinkerList) {
    // Empty stub — original C function had no implementation.
}

/// Walk the thinker list, dispatching active thinkers and removing pending ones.
///
/// Equivalent to `P_RunThinkers` in p_tick.c lines 101-122.
///
/// Returns a dispatch list of `(ActionFn, data_index)` tuples for the caller
/// to process. This deferred-dispatch pattern is used instead of inline calls
/// to avoid borrow checker conflicts with the arena during iteration.
#[inline]
pub fn p_run_thinkers(list: &mut ThinkerList) -> Vec<(ActionFn, usize)> {
    list.run_thinkers()
}

// =============================================================================
// TickState — per-level timing (leveltime)
// =============================================================================

/// Per-level timing state, replacing the C global `int leveltime` from p_tick.c
/// line 36.
///
/// # Exported field
///
/// * `leveltime` — current level time in tics, incremented once per [`p_ticker`]
///   call when the game is unpaused. Used for par time comparison on the
///   intermission screen and for periodic effects (e.g., ceiling sound every
///   8 tics, button revert countdown, level timer specials).
#[derive(Debug, Clone, Default)]
pub struct TickState {
    /// Level time in tics.
    ///
    /// Original C: `int leveltime;` (p_tick.c line 36)
    pub leveltime: i32,
}

// =============================================================================
// TickContext trait — game state interface for P_Ticker
// =============================================================================

/// Context trait providing all game state needed by [`p_ticker`].
///
/// The concrete implementation wires together the game state subsystems:
/// - `p_player_think` should delegate to [`user::p_player_think`]
/// - `p_update_specials` should delegate to [`spec::p_update_specials`]
/// - `p_respawn_specials` should delegate to [`mobj::p_respawn_specials`]
///
/// This trait-based approach decouples the tick driver from concrete game state
/// types, enabling unit testing with mock contexts and preserving the module
/// boundary between the tick driver, player processing, and specials subsystems.
pub trait TickContext {
    /// Whether the game is paused (from `doomstat.paused`).
    fn paused(&self) -> bool;

    /// Whether a network game is in progress (from `doomstat.netgame`).
    fn netgame(&self) -> bool;

    /// Whether the menu is active (from `menuactive`).
    fn menu_active(&self) -> bool;

    /// Whether demo playback is in progress (from `demoplayback`).
    fn demo_playback(&self) -> bool;

    /// Console player index (from `consoleplayer`).
    fn console_player(&self) -> usize;

    /// Access a player's state by index.
    ///
    /// Returns a reference to the [`Player`] struct, enabling direct access to
    /// fields like [`Player::viewz`] for the menu-pause sentinel check in
    /// [`p_ticker`].
    fn get_player(&self, idx: usize) -> &Player;

    /// Whether player `idx` is in the game (from `playeringame[idx]`).
    fn player_in_game(&self, idx: usize) -> bool;

    /// Run `P_PlayerThink` for the given player.
    ///
    /// The implementation should delegate to [`user::p_player_think`], passing
    /// the player index and a `UserContext` derived from the game state.
    fn p_player_think(&mut self, player_idx: usize);

    /// Access the thinker list (immutable).
    fn thinker_list(&self) -> &ThinkerList;

    /// Mutably access the thinker list for iteration and removal.
    fn thinker_list_mut(&mut self) -> &mut ThinkerList;

    /// Dispatch a single thinker action.
    ///
    /// The implementation should match on `action` and call the appropriate
    /// handler: `P_MobjThinker`, `T_MoveCeiling`, `T_VerticalDoor`,
    /// `T_MoveFloor`, `T_PlatRaise`, `T_FireFlicker`, `T_LightFlash`,
    /// `T_StrobeFlash`, `T_Glow`.
    fn dispatch_thinker(&mut self, action: ActionFn, data_index: usize);

    /// Run `P_UpdateSpecials` — animated textures, scrolling walls, button timers.
    ///
    /// The implementation should delegate to [`spec::p_update_specials`].
    fn p_update_specials(&mut self);

    /// Run `P_RespawnSpecials` — item respawning in deathmatch/nightmare.
    ///
    /// The implementation should delegate to [`mobj::p_respawn_specials`].
    fn p_respawn_specials(&mut self);

    /// Access tick state (immutable).
    fn tick_state(&self) -> &TickState;

    /// Mutably access tick state (for incrementing leveltime).
    fn tick_state_mut(&mut self) -> &mut TickState;
}

// =============================================================================
// P_Ticker — per-tic simulation entry point
// =============================================================================

/// Per-tic simulation driver. Called once per game tic (35 Hz).
///
/// Carries out all thinking of monsters, players, and specials. This is the
/// heartbeat of the game simulation — every gameplay entity (thinker) is
/// dispatched, and the level timer advances.
///
/// Equivalent to `P_Ticker` in p_tick.c lines 130-158:
///
/// 1. If `paused`, return immediately.
/// 2. Menu pause check: if single-player, menu active, not demo playback,
///    and `players[consoleplayer].viewz != Fixed(1)`, return.
///    The `viewz != 1` sentinel ensures at least one tic has been run
///    before allowing menu pause (viewz is initialized to 1 before the
///    first tic).
/// 3. For each active player (up to [`MAXPLAYERS`]), call `P_PlayerThink`.
/// 4. Run thinkers ([`p_run_thinkers`]) — remove pending, dispatch active.
/// 5. Update specials (`P_UpdateSpecials`).
/// 6. Respawn specials (`P_RespawnSpecials`).
/// 7. Increment `leveltime`.
pub fn p_ticker(ctx: &mut dyn TickContext) {
    // Step 1: Bail if paused.
    if ctx.paused() {
        return;
    }

    // Step 2: Menu pause check (single-player only).
    // The viewz != Fixed(1) sentinel ensures at least one tic has been run
    // before allowing menu pause. viewz is initialized to Fixed(1) before
    // the first tic.
    //
    // Original C:
    //   if (!netgame && menuactive && !demoplayback
    //       && players[consoleplayer].viewz != 1)
    //       return;
    if !ctx.netgame() && ctx.menu_active() && !ctx.demo_playback() {
        let cp = ctx.console_player();
        let player: &Player = ctx.get_player(cp);
        if player.viewz != Fixed(1) {
            return;
        }
    }

    // Step 3: Run player thinking for all active players.
    // Original C: for (i=0; i<MAXPLAYERS; i++)
    //                 if (playeringame[i]) P_PlayerThink(&players[i]);
    for i in 0..MAXPLAYERS {
        if ctx.player_in_game(i) {
            ctx.p_player_think(i);
        }
    }

    // Step 4: Run thinkers — collect dispatch list, then dispatch each.
    // The two-phase (collect then dispatch) approach avoids borrowing the
    // thinker list mutably while also needing mutable game state access
    // for thinker dispatch.
    let dispatch_list = ctx.thinker_list_mut().run_thinkers();
    for (action, data_index) in dispatch_list {
        ctx.dispatch_thinker(action, data_index);
    }

    // Step 5: Update animated textures, scrolling walls, button timers.
    // Delegates to spec::p_update_specials through the context.
    ctx.p_update_specials();

    // Step 6: Respawn items in deathmatch/nightmare mode.
    // Delegates to mobj::p_respawn_specials through the context.
    ctx.p_respawn_specials();

    // Step 7: Increment level time (for par times and periodic effects).
    ctx.tick_state_mut().leveltime += 1;
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // ThinkerList unit tests
    // =========================================================================

    #[test]
    fn test_thinker_list_init() {
        let list = ThinkerList::new();
        assert_eq!(list.entries.len(), 1, "Should have only sentinel");
        assert_eq!(list.head, 0);
        assert_eq!(
            list.entries[0].thinker.next,
            Some(0),
            "Sentinel next is self"
        );
        assert_eq!(
            list.entries[0].thinker.prev,
            Some(0),
            "Sentinel prev is self"
        );
        assert_eq!(list.entries[0].thinker.function, ActionFn::None);
        assert_eq!(list.count(), 0);
    }

    #[test]
    fn test_thinkercap_access() {
        let list = ThinkerList::new();
        let cap = list.thinkercap();
        assert_eq!(cap.prev, Some(0));
        assert_eq!(cap.next, Some(0));
        assert_eq!(cap.function, ActionFn::None);
    }

    #[test]
    fn test_add_thinker() {
        let mut list = ThinkerList::new();

        let idx1 = list.add_thinker(ActionFn::MobjThinker, 42);
        assert_eq!(idx1, 1);
        assert_eq!(list.count(), 1);
        assert_eq!(list.entries[idx1].thinker.function, ActionFn::MobjThinker);
        assert_eq!(list.entries[idx1].data_index, 42);
        assert!(list.entries[idx1].active);

        // Verify linking: sentinel -> thinker1 -> sentinel (circular)
        assert_eq!(list.entries[0].thinker.next, Some(1));
        assert_eq!(list.entries[0].thinker.prev, Some(1));
        assert_eq!(list.entries[1].thinker.next, Some(0));
        assert_eq!(list.entries[1].thinker.prev, Some(0));
    }

    #[test]
    fn test_add_multiple_thinkers() {
        let mut list = ThinkerList::new();

        let idx1 = list.add_thinker(ActionFn::MobjThinker, 10);
        let idx2 = list.add_thinker(ActionFn::VerticalDoor, 20);
        let idx3 = list.add_thinker(ActionFn::MoveFloor, 30);

        assert_eq!(list.count(), 3);

        // Verify forward order: sentinel -> idx1 -> idx2 -> idx3 -> sentinel
        assert_eq!(list.entries[0].thinker.next, Some(idx1));
        assert_eq!(list.entries[idx1].thinker.next, Some(idx2));
        assert_eq!(list.entries[idx2].thinker.next, Some(idx3));
        assert_eq!(list.entries[idx3].thinker.next, Some(0));

        // Verify reverse order: sentinel -> idx3 -> idx2 -> idx1 -> sentinel
        assert_eq!(list.entries[0].thinker.prev, Some(idx3));
        assert_eq!(list.entries[idx3].thinker.prev, Some(idx2));
        assert_eq!(list.entries[idx2].thinker.prev, Some(idx1));
        assert_eq!(list.entries[idx1].thinker.prev, Some(0));
    }

    #[test]
    fn test_remove_thinker_lazy() {
        let mut list = ThinkerList::new();

        let idx1 = list.add_thinker(ActionFn::MobjThinker, 10);
        let _idx2 = list.add_thinker(ActionFn::VerticalDoor, 20);

        // Mark for removal — only sets PendingRemoval, doesn't unlink
        list.remove_thinker(idx1);
        assert_eq!(
            list.entries[idx1].thinker.function,
            ActionFn::PendingRemoval,
            "Should be marked PendingRemoval"
        );
        // Still linked (lazy removal)
        assert!(list.entries[idx1].thinker.next.is_some());
        assert!(list.entries[idx1].thinker.prev.is_some());
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

        // idx1 slot should be inactive and unlinked
        assert!(!list.entries[idx1].active);
        assert_eq!(list.entries[idx1].thinker.prev, None);
        assert_eq!(list.entries[idx1].thinker.next, None);
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
        assert_eq!(list.entries[0].thinker.next, Some(0));
        assert_eq!(list.entries[0].thinker.prev, Some(0));
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
        assert_eq!(list.entries[idx3].thinker.function, ActionFn::MoveFloor);
        assert_eq!(list.entries[idx3].data_index, 30);
        assert!(list.entries[idx3].active);
    }

    #[test]
    fn test_cannot_remove_sentinel() {
        let mut list = ThinkerList::new();
        list.remove_thinker(0);
        // Sentinel should remain unchanged
        assert_eq!(list.entries[0].thinker.function, ActionFn::None);
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

    #[test]
    fn test_remove_out_of_bounds() {
        let mut list = ThinkerList::new();
        // Should not panic when removing index that doesn't exist
        list.remove_thinker(999);
        assert_eq!(list.count(), 0);
    }

    #[test]
    fn test_remove_all_thinkers() {
        let mut list = ThinkerList::new();
        let idx1 = list.add_thinker(ActionFn::MobjThinker, 1);
        let idx2 = list.add_thinker(ActionFn::VerticalDoor, 2);
        let idx3 = list.add_thinker(ActionFn::MoveFloor, 3);

        list.remove_thinker(idx1);
        list.remove_thinker(idx2);
        list.remove_thinker(idx3);

        let dispatch = list.run_thinkers();
        assert!(dispatch.is_empty(), "All removed, nothing to dispatch");
        assert_eq!(list.count(), 0);

        // Sentinel should still be intact
        assert_eq!(list.entries[0].thinker.next, Some(0));
        assert_eq!(list.entries[0].thinker.prev, Some(0));
    }

    // =========================================================================
    // Standalone function tests
    // =========================================================================

    #[test]
    fn test_standalone_p_init_thinkers() {
        let mut list = ThinkerList::new();
        list.add_thinker(ActionFn::MobjThinker, 1);
        p_init_thinkers(&mut list);
        assert_eq!(list.count(), 0);
    }

    #[test]
    fn test_standalone_p_add_thinker() {
        let mut list = ThinkerList::new();
        let idx = p_add_thinker(&mut list, ActionFn::VerticalDoor, 55);
        assert_eq!(list.entries[idx].thinker.function, ActionFn::VerticalDoor);
        assert_eq!(list.entries[idx].data_index, 55);
    }

    #[test]
    fn test_standalone_p_remove_thinker() {
        let mut list = ThinkerList::new();
        let idx = p_add_thinker(&mut list, ActionFn::MobjThinker, 1);
        p_remove_thinker(&mut list, idx);
        assert_eq!(list.entries[idx].thinker.function, ActionFn::PendingRemoval);
    }

    #[test]
    fn test_standalone_p_allocate_thinker() {
        let mut list = ThinkerList::new();
        // Should do nothing (empty stub)
        p_allocate_thinker(&mut list);
        assert_eq!(list.count(), 0);
    }

    #[test]
    fn test_standalone_p_run_thinkers() {
        let mut list = ThinkerList::new();
        p_add_thinker(&mut list, ActionFn::MobjThinker, 10);
        p_add_thinker(&mut list, ActionFn::Glow, 20);

        let dispatch = p_run_thinkers(&mut list);
        assert_eq!(dispatch.len(), 2);
        assert_eq!(dispatch[0], (ActionFn::MobjThinker, 10));
        assert_eq!(dispatch[1], (ActionFn::Glow, 20));
    }

    // =========================================================================
    // P_Ticker tests (requires mock TickContext)
    // =========================================================================

    /// Minimal mock implementing TickContext for testing p_ticker.
    struct MockTickContext {
        paused: bool,
        netgame: bool,
        menu_active: bool,
        demo_playback: bool,
        console_player: usize,
        players: [Player; MAXPLAYERS],
        player_in_game: [bool; MAXPLAYERS],
        thinker_list: ThinkerList,
        tick_state: TickState,
        // Tracking counters for verification
        player_think_calls: Vec<usize>,
        dispatched: Vec<(ActionFn, usize)>,
        update_specials_called: bool,
        respawn_specials_called: bool,
    }

    impl MockTickContext {
        fn new() -> Self {
            Self {
                paused: false,
                netgame: false,
                menu_active: false,
                demo_playback: false,
                console_player: 0,
                players: Default::default(),
                player_in_game: [false; MAXPLAYERS],
                thinker_list: ThinkerList::new(),
                tick_state: TickState::default(),
                player_think_calls: Vec::new(),
                dispatched: Vec::new(),
                update_specials_called: false,
                respawn_specials_called: false,
            }
        }
    }

    impl TickContext for MockTickContext {
        fn paused(&self) -> bool {
            self.paused
        }
        fn netgame(&self) -> bool {
            self.netgame
        }
        fn menu_active(&self) -> bool {
            self.menu_active
        }
        fn demo_playback(&self) -> bool {
            self.demo_playback
        }
        fn console_player(&self) -> usize {
            self.console_player
        }
        fn get_player(&self, idx: usize) -> &Player {
            &self.players[idx]
        }
        fn player_in_game(&self, idx: usize) -> bool {
            self.player_in_game[idx]
        }
        fn p_player_think(&mut self, player_idx: usize) {
            self.player_think_calls.push(player_idx);
        }
        fn thinker_list(&self) -> &ThinkerList {
            &self.thinker_list
        }
        fn thinker_list_mut(&mut self) -> &mut ThinkerList {
            &mut self.thinker_list
        }
        fn dispatch_thinker(&mut self, action: ActionFn, data_index: usize) {
            self.dispatched.push((action, data_index));
        }
        fn p_update_specials(&mut self) {
            self.update_specials_called = true;
        }
        fn p_respawn_specials(&mut self) {
            self.respawn_specials_called = true;
        }
        fn tick_state(&self) -> &TickState {
            &self.tick_state
        }
        fn tick_state_mut(&mut self) -> &mut TickState {
            &mut self.tick_state
        }
    }

    #[test]
    fn test_p_ticker_paused() {
        let mut ctx = MockTickContext::new();
        ctx.paused = true;
        ctx.player_in_game[0] = true;

        p_ticker(&mut ctx);

        // Nothing should happen when paused
        assert!(ctx.player_think_calls.is_empty());
        assert!(!ctx.update_specials_called);
        assert!(!ctx.respawn_specials_called);
        assert_eq!(ctx.tick_state.leveltime, 0);
    }

    #[test]
    fn test_p_ticker_menu_pause_viewz_not_one() {
        let mut ctx = MockTickContext::new();
        ctx.menu_active = true;
        ctx.player_in_game[0] = true;
        // Set viewz to something other than Fixed(1) — triggers menu pause
        ctx.players[0].viewz = Fixed(100);

        p_ticker(&mut ctx);

        // Should return early due to menu pause
        assert!(ctx.player_think_calls.is_empty());
        assert!(!ctx.update_specials_called);
        assert_eq!(ctx.tick_state.leveltime, 0);
    }

    #[test]
    fn test_p_ticker_menu_pause_viewz_is_one() {
        let mut ctx = MockTickContext::new();
        ctx.menu_active = true;
        ctx.player_in_game[0] = true;
        // viewz == Fixed(1) means first tic hasn't run yet — don't pause
        ctx.players[0].viewz = Fixed(1);

        p_ticker(&mut ctx);

        // Should NOT pause — runs normally
        assert!(!ctx.player_think_calls.is_empty());
        assert!(ctx.update_specials_called);
        assert!(ctx.respawn_specials_called);
        assert_eq!(ctx.tick_state.leveltime, 1);
    }

    #[test]
    fn test_p_ticker_menu_pause_netgame_skips() {
        let mut ctx = MockTickContext::new();
        ctx.menu_active = true;
        ctx.netgame = true; // Menu pause doesn't apply in netgame
        ctx.player_in_game[0] = true;
        ctx.players[0].viewz = Fixed(100);

        p_ticker(&mut ctx);

        // Should NOT pause — netgame overrides menu pause
        assert!(!ctx.player_think_calls.is_empty());
        assert!(ctx.update_specials_called);
        assert_eq!(ctx.tick_state.leveltime, 1);
    }

    #[test]
    fn test_p_ticker_menu_pause_demo_playback_skips() {
        let mut ctx = MockTickContext::new();
        ctx.menu_active = true;
        ctx.demo_playback = true; // Menu pause doesn't apply during demo
        ctx.player_in_game[0] = true;
        ctx.players[0].viewz = Fixed(100);

        p_ticker(&mut ctx);

        // Should NOT pause — demo playback overrides menu pause
        assert!(!ctx.player_think_calls.is_empty());
        assert!(ctx.update_specials_called);
        assert_eq!(ctx.tick_state.leveltime, 1);
    }

    #[test]
    fn test_p_ticker_player_think() {
        let mut ctx = MockTickContext::new();
        ctx.player_in_game[0] = true;
        ctx.player_in_game[2] = true;

        p_ticker(&mut ctx);

        // Only players 0 and 2 should get P_PlayerThink called
        assert_eq!(ctx.player_think_calls, vec![0, 2]);
    }

    #[test]
    fn test_p_ticker_thinker_dispatch() {
        let mut ctx = MockTickContext::new();
        ctx.player_in_game[0] = true;

        ctx.thinker_list.add_thinker(ActionFn::MobjThinker, 10);
        ctx.thinker_list.add_thinker(ActionFn::VerticalDoor, 20);

        p_ticker(&mut ctx);

        assert_eq!(ctx.dispatched.len(), 2);
        assert_eq!(ctx.dispatched[0], (ActionFn::MobjThinker, 10));
        assert_eq!(ctx.dispatched[1], (ActionFn::VerticalDoor, 20));
    }

    #[test]
    fn test_p_ticker_full_sequence() {
        let mut ctx = MockTickContext::new();
        ctx.player_in_game[0] = true;
        ctx.thinker_list.add_thinker(ActionFn::MobjThinker, 1);

        // First tic
        p_ticker(&mut ctx);

        assert_eq!(ctx.player_think_calls, vec![0]);
        assert_eq!(ctx.dispatched, vec![(ActionFn::MobjThinker, 1)]);
        assert!(ctx.update_specials_called);
        assert!(ctx.respawn_specials_called);
        assert_eq!(ctx.tick_state.leveltime, 1);

        // Second tic
        ctx.player_think_calls.clear();
        ctx.dispatched.clear();
        ctx.update_specials_called = false;
        ctx.respawn_specials_called = false;

        p_ticker(&mut ctx);

        assert_eq!(ctx.tick_state.leveltime, 2);
    }

    #[test]
    fn test_p_ticker_leveltime_increments() {
        let mut ctx = MockTickContext::new();

        for expected in 1..=10 {
            p_ticker(&mut ctx);
            assert_eq!(ctx.tick_state.leveltime, expected);
        }
    }

    #[test]
    fn test_tick_state_default() {
        let state = TickState::default();
        assert_eq!(state.leveltime, 0);
    }
}
