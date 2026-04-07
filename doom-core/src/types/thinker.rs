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

//! Translated from linuxdoom-1.10/d_think.h
//!
//! Thinker linked list management. In DOOM, "thinkers" are the actor update
//! mechanism — every active entity (monsters, doors, platforms, lights, etc.)
//! has a thinker that gets called once per game tick.
//!
//! The original C uses a doubly-linked list with function pointer unions.
//! This Rust version uses arena indices for linking and an enum for the
//! action function dispatch.
//!
//! # Original C types replaced
//!
//! - `actionf_t` union (function pointer variants) → [`ActionFn`] enum
//! - `think_t` typedef → [`ThinkT`] type alias
//! - `thinker_t` struct (raw `*prev`/`*next` pointers) → [`Thinker`] struct
//!   with `Option<usize>` arena indices
//!
//! # Design decisions
//!
//! The C code uses a union of three function pointer types:
//! ```c
//! typedef void (*actionf_v)();
//! typedef void (*actionf_p1)(void*);
//! typedef void (*actionf_p2)(void*, void*);
//! typedef union { actionf_p1 acp1; actionf_v acv; actionf_p2 acp2; } actionf_t;
//! ```
//!
//! In practice, the engine only ever assigns a finite set of known functions
//! to thinkers. Rather than storing raw function pointers (which would require
//! `unsafe`), this Rust translation uses an enum whose variants name each
//! concrete thinker action. The game's tick dispatcher matches on this enum
//! to call the appropriate handler.
//!
//! The doubly-linked list pointers (`struct thinker_s* prev/next`) are replaced
//! with `Option<usize>` arena indices, enabling safe traversal without raw
//! pointer manipulation.

/// Action function dispatch enum, replacing the C `actionf_t` union.
///
/// Each variant corresponds to a specific thinker callback function in the
/// original engine. The game's per-tick update loop matches on this enum to
/// dispatch the correct behavior for each active thinker.
///
/// # Variants
///
/// | Variant | Original C function | Source file |
/// |---------|-------------------|-------------|
/// | `None` | NULL / sentinel | (head node) |
/// | `PendingRemoval` | -1 / marked for unlink | p_tick.c |
/// | `MobjThinker` | `P_MobjThinker` | p_mobj.c |
/// | `MoveCeiling` | `T_MoveCeiling` | p_ceilng.c |
/// | `VerticalDoor` | `T_VerticalDoor` | p_doors.c |
/// | `MoveFloor` | `T_MoveFloor` | p_floor.c |
/// | `PlatRaise` | `T_PlatRaise` | p_plats.c |
/// | `FireFlicker` | `T_FireFlicker` | p_lights.c |
/// | `LightFlash` | `T_LightFlash` | p_lights.c |
/// | `StrobeFlash` | `T_StrobeFlash` | p_lights.c |
/// | `Glow` | `T_Glow` | p_lights.c |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionFn {
    /// No action — used for the sentinel/head node of the thinker list.
    ///
    /// In the original C code, this corresponds to a NULL function pointer
    /// in the `actionf_t` union, indicating that this thinker node serves
    /// only as the list anchor and should not be dispatched.
    None,

    /// Thinker is marked for removal from the linked list.
    ///
    /// In the original C code, when `P_RemoveThinker` is called, the
    /// thinker's function pointer is set to a sentinel value (cast of -1)
    /// rather than immediately unlinking it. The next traversal of the
    /// thinker list in `P_RunThinkers` detects this marker and performs
    /// the actual unlink and deallocation. This two-phase removal prevents
    /// iterator invalidation during thinker list traversal.
    PendingRemoval,

    /// Map object thinker — handles monsters, items, projectiles, and all
    /// other mobile map objects.
    ///
    /// Corresponds to `P_MobjThinker` in `p_mobj.c`. This is by far the
    /// most common thinker type, responsible for state machine transitions,
    /// physics (gravity, friction, momentum), and action function dispatch
    /// for every `mobj_t` in the game world.
    MobjThinker,

    /// Ceiling movement thinker — handles raising and lowering ceilings.
    ///
    /// Corresponds to `T_MoveCeiling` in `p_ceilng.c`. Manages crusher
    /// ceilings, ceiling-to-highest-ceiling movements, and other ceiling
    /// specials triggered by line switches or walk-over triggers.
    MoveCeiling,

    /// Vertical door thinker — handles opening and closing doors.
    ///
    /// Corresponds to `T_VerticalDoor` in `p_doors.c`. Manages all door
    /// types: normal doors, blazing doors, locked doors, and doors that
    /// open and stay open. Handles the wait-then-close timer for standard
    /// doors.
    VerticalDoor,

    /// Floor movement thinker — handles raising and lowering floors.
    ///
    /// Corresponds to `T_MoveFloor` in `p_floor.c`. Manages floor
    /// specials including raise-to-nearest, lower-to-lowest, donut
    /// effects, and stair building sequences.
    MoveFloor,

    /// Platform raise thinker — handles lift/platform movement.
    ///
    /// Corresponds to `T_PlatRaise` in `p_plats.c`. Manages lifts that
    /// move between two heights, including the perpetual platforms and
    /// down-wait-up-stay platforms commonly used in DOOM levels.
    PlatRaise,

    /// Fire flicker effect thinker — simulates flickering fire light.
    ///
    /// Corresponds to `T_FireFlicker` in `p_lights.c`. Produces a
    /// rapid, irregular flickering effect by alternating the sector's
    /// light level between its base value and a randomly reduced value.
    FireFlicker,

    /// Light flash thinker — produces bright flashing light effects.
    ///
    /// Corresponds to `T_LightFlash` in `p_lights.c`. Creates a
    /// periodic flash between the sector's maximum and minimum
    /// neighboring light levels with randomized timing.
    LightFlash,

    /// Strobe flash thinker — produces regular strobe light effects.
    ///
    /// Corresponds to `T_StrobeFlash` in `p_lights.c`. Creates a
    /// regular on/off strobe pattern with configurable bright and
    /// dark durations. Used extensively in DOOM's techbase levels.
    StrobeFlash,

    /// Glow effect thinker — produces smooth oscillating light.
    ///
    /// Corresponds to `T_Glow` in `p_lights.c`. Creates a smooth,
    /// sinusoidal-like oscillation between the sector's minimum and
    /// maximum neighboring light levels, producing a gentle pulsing
    /// glow effect.
    Glow,
}

impl Default for ActionFn {
    /// Returns [`ActionFn::None`], representing a sentinel/inactive thinker.
    #[inline]
    fn default() -> Self {
        ActionFn::None
    }
}

impl ActionFn {
    /// Returns `true` if this action represents an active thinker that should
    /// be dispatched during the game tick.
    ///
    /// `None` and `PendingRemoval` are not dispatched — `None` is the
    /// sentinel head node, and `PendingRemoval` indicates the thinker is
    /// queued for unlinking.
    #[inline]
    pub fn is_active(&self) -> bool {
        !matches!(self, ActionFn::None | ActionFn::PendingRemoval)
    }

    /// Returns `true` if this thinker has been marked for removal.
    ///
    /// Used by the thinker list traversal code to detect thinkers that
    /// should be unlinked and deallocated.
    #[inline]
    pub fn is_pending_removal(&self) -> bool {
        matches!(self, ActionFn::PendingRemoval)
    }
}

/// Thinker list node, replacing the C `thinker_t` struct.
///
/// In the original C code (d_think.h lines 64-70):
/// ```c
/// typedef struct thinker_s {
///     struct thinker_s* prev;
///     struct thinker_s* next;
///     think_t           function;
/// } thinker_t;
/// ```
///
/// The raw `struct thinker_s*` pointers are replaced with `Option<usize>`
/// arena indices. `None` indicates the absence of a link (equivalent to a
/// NULL pointer in C). The arena that owns the thinker nodes is managed
/// externally by the game's thinker list controller (in `doom-core/src/play/tick.rs`).
///
/// # Arena-based design
///
/// Rather than using Rust references or raw pointers for the doubly-linked
/// list, thinkers are stored in a `Vec<Thinker>` arena. The `prev` and `next`
/// fields store indices into this arena. This approach:
///
/// - Eliminates all `unsafe` pointer manipulation
/// - Avoids lifetime complexity of self-referential structures
/// - Preserves the O(1) insert/remove semantics of the original linked list
/// - Enables straightforward serialization for save/load game support
#[derive(Debug, Clone, Copy)]
pub struct Thinker {
    /// Index of the previous thinker in the doubly-linked list.
    ///
    /// `None` indicates this is the first node (or the node is unlinked).
    /// In the original C: `struct thinker_s* prev`.
    pub prev: Option<usize>,

    /// Index of the next thinker in the doubly-linked list.
    ///
    /// `None` indicates this is the last node (or the node is unlinked).
    /// In the original C: `struct thinker_s* next`.
    pub next: Option<usize>,

    /// The action function to call each game tick for this thinker.
    ///
    /// Determines which thinker handler is dispatched. In the original C,
    /// this was `think_t function` — a `typedef` for `actionf_t`, the union
    /// of function pointers.
    pub function: ActionFn,
}

impl Default for Thinker {
    /// Creates a default thinker with no links and no action.
    ///
    /// This is suitable for initializing the sentinel/head node of the
    /// thinker list, which serves as the anchor for the circular
    /// doubly-linked list but is never dispatched.
    #[inline]
    fn default() -> Self {
        Thinker {
            prev: None,
            next: None,
            function: ActionFn::None,
        }
    }
}

impl Thinker {
    /// Creates a new thinker with the specified action function and no links.
    ///
    /// The `prev` and `next` fields are initialized to `None`. The caller
    /// is responsible for linking this thinker into the list by setting
    /// the appropriate arena indices.
    #[inline]
    pub fn new(function: ActionFn) -> Self {
        Thinker {
            prev: None,
            next: None,
            function,
        }
    }

    /// Returns `true` if this thinker is currently linked into a list.
    ///
    /// A thinker is considered linked if either `prev` or `next` is `Some`.
    /// An unlinked thinker has both set to `None`.
    #[inline]
    pub fn is_linked(&self) -> bool {
        self.prev.is_some() || self.next.is_some()
    }

    /// Clears the link fields, setting both `prev` and `next` to `None`.
    ///
    /// This does NOT remove the thinker from any list — the caller must
    /// update the adjacent nodes' links before calling this method.
    #[inline]
    pub fn unlink(&mut self) {
        self.prev = None;
        self.next = None;
    }
}

impl PartialEq for Thinker {
    /// Two thinkers are equal if they have the same links and function.
    fn eq(&self, other: &Self) -> bool {
        self.prev == other.prev && self.next == other.next && self.function == other.function
    }
}

impl Eq for Thinker {}

/// Type alias preserving the C naming convention.
///
/// In the original C code (d_think.h line 60):
/// ```c
/// typedef actionf_t think_t;
/// ```
///
/// `think_t` was simply a typedef for `actionf_t`. In the Rust translation,
/// [`ThinkT`] is a type alias for [`ActionFn`], maintaining compatibility
/// with code that references the original C naming convention while using
/// the idiomatic Rust enum type.
pub type ThinkT = ActionFn;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_fn_default_is_none() {
        assert_eq!(ActionFn::default(), ActionFn::None);
    }

    #[test]
    fn action_fn_is_active() {
        assert!(!ActionFn::None.is_active());
        assert!(!ActionFn::PendingRemoval.is_active());
        assert!(ActionFn::MobjThinker.is_active());
        assert!(ActionFn::MoveCeiling.is_active());
        assert!(ActionFn::VerticalDoor.is_active());
        assert!(ActionFn::MoveFloor.is_active());
        assert!(ActionFn::PlatRaise.is_active());
        assert!(ActionFn::FireFlicker.is_active());
        assert!(ActionFn::LightFlash.is_active());
        assert!(ActionFn::StrobeFlash.is_active());
        assert!(ActionFn::Glow.is_active());
    }

    #[test]
    fn action_fn_is_pending_removal() {
        assert!(!ActionFn::None.is_pending_removal());
        assert!(ActionFn::PendingRemoval.is_pending_removal());
        assert!(!ActionFn::MobjThinker.is_pending_removal());
    }

    #[test]
    fn action_fn_equality() {
        assert_eq!(ActionFn::MobjThinker, ActionFn::MobjThinker);
        assert_ne!(ActionFn::MobjThinker, ActionFn::MoveCeiling);
        assert_ne!(ActionFn::None, ActionFn::PendingRemoval);
    }

    #[test]
    fn action_fn_clone_copy() {
        let a = ActionFn::Glow;
        let b = a;
        let c = a;
        assert_eq!(b, c);
        assert_eq!(a, ActionFn::Glow);
    }

    #[test]
    fn thinker_default() {
        let t = Thinker::default();
        assert_eq!(t.prev, None);
        assert_eq!(t.next, None);
        assert_eq!(t.function, ActionFn::None);
    }

    #[test]
    fn thinker_new() {
        let t = Thinker::new(ActionFn::MobjThinker);
        assert_eq!(t.prev, None);
        assert_eq!(t.next, None);
        assert_eq!(t.function, ActionFn::MobjThinker);
    }

    #[test]
    fn thinker_is_linked() {
        let mut t = Thinker::default();
        assert!(!t.is_linked());

        t.next = Some(1);
        assert!(t.is_linked());

        t.next = None;
        t.prev = Some(0);
        assert!(t.is_linked());

        t.prev = Some(0);
        t.next = Some(1);
        assert!(t.is_linked());
    }

    #[test]
    fn thinker_unlink() {
        let mut t = Thinker {
            prev: Some(0),
            next: Some(2),
            function: ActionFn::VerticalDoor,
        };
        assert!(t.is_linked());
        t.unlink();
        assert!(!t.is_linked());
        assert_eq!(t.prev, None);
        assert_eq!(t.next, None);
        // function is preserved after unlink
        assert_eq!(t.function, ActionFn::VerticalDoor);
    }

    #[test]
    fn thinker_equality() {
        let a = Thinker {
            prev: Some(0),
            next: Some(2),
            function: ActionFn::MobjThinker,
        };
        let b = Thinker {
            prev: Some(0),
            next: Some(2),
            function: ActionFn::MobjThinker,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn thinker_inequality() {
        let a = Thinker::new(ActionFn::MobjThinker);
        let b = Thinker::new(ActionFn::Glow);
        assert_ne!(a, b);

        let c = Thinker {
            prev: Some(1),
            next: None,
            function: ActionFn::MobjThinker,
        };
        assert_ne!(a, c);
    }

    #[test]
    fn thinker_clone_copy() {
        let a = Thinker {
            prev: Some(5),
            next: Some(10),
            function: ActionFn::PlatRaise,
        };
        let b = a;
        let c = a;
        assert_eq!(b, c);
        assert_eq!(a.function, ActionFn::PlatRaise);
    }

    #[test]
    fn think_t_alias() {
        // ThinkT is an alias for ActionFn
        let t: ThinkT = ActionFn::StrobeFlash;
        assert_eq!(t, ActionFn::StrobeFlash);

        let default_think: ThinkT = ThinkT::None;
        assert_eq!(default_think, ActionFn::None);
    }

    #[test]
    fn all_action_fn_variants_are_distinct() {
        let variants: [ActionFn; 11] = [
            ActionFn::None,
            ActionFn::PendingRemoval,
            ActionFn::MobjThinker,
            ActionFn::MoveCeiling,
            ActionFn::VerticalDoor,
            ActionFn::MoveFloor,
            ActionFn::PlatRaise,
            ActionFn::FireFlicker,
            ActionFn::LightFlash,
            ActionFn::StrobeFlash,
            ActionFn::Glow,
        ];
        // Verify all pairs are distinct
        for i in 0..variants.len() {
            for j in (i + 1)..variants.len() {
                assert_ne!(
                    variants[i], variants[j],
                    "Variants at index {} and {} should be different",
                    i, j
                );
            }
        }
    }

    #[test]
    fn action_fn_debug_format() {
        // Verify Debug trait works (no panic)
        let formatted = format!("{:?}", ActionFn::MobjThinker);
        assert!(formatted.contains("MobjThinker"));
    }

    #[test]
    fn thinker_debug_format() {
        let t = Thinker::new(ActionFn::Glow);
        let formatted = format!("{:?}", t);
        assert!(formatted.contains("Glow"));
        assert!(formatted.contains("None"));
    }
}
