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

//! Translated from linuxdoom-1.10/r_plane.c and r_plane.h
//!
//! Here now what is needed to handle the visplanes.
//! Floor and ceiling rendering, span generation, and sky column rendering.
//!
//! # Architecture
//!
//! Visplanes are the primary data structure for floor and ceiling rendering
//! in the DOOM engine. Each visplane represents a contiguous screen area of
//! a single flat texture at a single height and light level. The rendering
//! proceeds in two phases:
//!
//! 1. **Allocation phase** (during BSP traversal): [`find_plane`](PlaneState::find_plane)
//!    and [`check_plane`](PlaneState::check_plane) are called from the BSP/segs
//!    code to find or allocate visplanes for floor and ceiling surfaces.
//!
//! 2. **Rendering phase** (end of frame): [`draw_planes`](PlaneState::draw_planes)
//!    iterates all allocated visplanes and either:
//!    - Renders sky via column drawing primitives (for `F_SKY1` sectors)
//!    - Renders floor/ceiling flats via horizontal span drawing primitives
//!
//! The column-to-span conversion is performed by [`make_spans`](PlaneState::make_spans),
//! which converts the per-column top/bottom data stored in each visplane into
//! horizontal span calls to [`map_plane`](PlaneState::map_plane).
//!
//! # Critical Implementation Details
//!
//! - Visplane `top[]` uses `0xff` as a sentinel value meaning "no data for this column"
//! - The openings array is shared between plane clipping and masked texture columns
//! - `floorclip`/`ceilingclip` define per-column Y boundaries that shrink as walls are drawn
//! - Sky is always rendered full bright: `colormaps[0]` regardless of sector light or INVUL
//! - Fixed-point 16.16 arithmetic is used throughout for distance and texture mapping

use crate::data::DataState;
use crate::defs::{Fixed, Visplane, ANG90, ANGLETOFINESHIFT, SCREENHEIGHT, SCREENWIDTH};
use crate::draw::DrawState;
use crate::main::{
    ColFunc, RenderMain, SpanFunc, LIGHTLEVELS, LIGHTSEGSHIFT, LIGHTZSHIFT, MAXLIGHTZ,
};
use crate::sky::{SkyState, ANGLETOSKYSHIFT};
use crate::things::ThingsState;

use doom_core::types::tables::{finecosine, FINESINE};
use doom_wad::types::PurgeTag;
use doom_wad::wad_provider::WadProvider;

// =============================================================================
// Screen dimension as usize for array sizing
// =============================================================================

/// Screen width as usize for array indexing and sizing.
const SCREENWIDTH_USIZE: usize = SCREENWIDTH as usize;

/// Screen height as usize for array indexing and sizing.
const SCREENHEIGHT_USIZE: usize = SCREENHEIGHT as usize;

// =============================================================================
// Constants — r_plane.h / r_plane.c
// =============================================================================

/// Maximum number of visplanes per frame.
///
/// Original C: `#define MAXVISPLANES 128` (r_plane.c line 70).
/// If exceeded during `find_plane`, a fatal error is raised matching
/// the original `I_Error("R_FindPlane: no more visplanes")`.
pub const MAXVISPLANES: usize = 128;

/// Maximum number of screen openings per frame.
///
/// Sized as `SCREENWIDTH * 64` = `320 * 64` = 20480.
/// Used for sprite clipping and masked texture column storage.
///
/// Original C: `#define MAXOPENINGS SCREENWIDTH*64` (r_plane.c line 74).
pub const MAXOPENINGS: usize = SCREENWIDTH_USIZE * 64;

// =============================================================================
// PlaneState — Collected r_plane.c globals
// =============================================================================

/// Visplane rendering state, collecting all formerly-global variables from
/// `r_plane.c` into a single owned struct.
///
/// This struct manages:
/// - The visplane allocation pool and current frame pointers
/// - The openings array for sprite clipping and masked textures
/// - Per-column floor/ceiling clip arrays
/// - Span generation bookkeeping arrays
/// - Texture mapping caches and pre-calculated lookup tables
/// - Floor/ceiling drawing function dispatch selectors
///
/// # Lifecycle
///
/// 1. [`clear_planes`](PlaneState::clear_planes) resets per-frame state at the
///    start of each rendering frame.
/// 2. During BSP traversal, [`find_plane`](PlaneState::find_plane) and
///    [`check_plane`](PlaneState::check_plane) allocate and validate visplanes.
/// 3. [`draw_planes`](PlaneState::draw_planes) renders all allocated visplanes
///    at the end of the frame.
pub struct PlaneState {
    // =========================================================================
    // Visplane array (r_plane.c lines 71-73)
    // =========================================================================
    /// Pool of visplanes for the current frame.
    ///
    /// Original C: `visplane_t visplanes[MAXVISPLANES]` (r_plane.c line 71).
    pub visplanes: Vec<Visplane>,

    /// Index of the next free visplane slot. All visplanes `[0..lastvisplane)`
    /// are active for the current frame.
    ///
    /// Original C: `visplane_t* lastvisplane` — converted from pointer arithmetic
    /// to index-based tracking.
    pub lastvisplane: usize,

    /// Index of the current floor visplane being filled during BSP traversal.
    /// `None` when no floor visplane is active for the current subsector.
    ///
    /// Original C: `visplane_t* floorplane` (r_plane.c line 73).
    pub floorplane: Option<usize>,

    /// Index of the current ceiling visplane being filled during BSP traversal.
    /// `None` when no ceiling visplane is active for the current subsector.
    ///
    /// Original C: `visplane_t* ceilingplane` (r_plane.c line 73).
    pub ceilingplane: Option<usize>,

    // =========================================================================
    // Openings array (r_plane.c lines 75-76)
    // =========================================================================
    /// Screen openings buffer for sprite clipping and masked texture storage.
    ///
    /// Shared between plane clipping (floor/ceiling boundaries) and masked
    /// texture columns. Managed via `lastopening` as a bump allocator.
    ///
    /// Original C: `short openings[MAXOPENINGS]` (r_plane.c line 75).
    pub openings: Vec<i16>,

    /// Index of the next free opening slot in the openings array.
    ///
    /// Original C: `short* lastopening` (r_plane.c line 76).
    pub lastopening: usize,

    // =========================================================================
    // Clip arrays (r_plane.h lines 55-56)
    // =========================================================================
    /// Per-column floor clip value (bottom boundary for rendering).
    /// Initialized to `viewheight` at the start of each frame and shrinks
    /// as walls are drawn.
    ///
    /// Original C: `short floorclip[SCREENWIDTH]` (r_plane.h line 55).
    pub floorclip: [i16; SCREENWIDTH_USIZE],

    /// Per-column ceiling clip value (top boundary for rendering).
    /// Initialized to `-1` at the start of each frame and grows
    /// as walls are drawn.
    ///
    /// Original C: `short ceilingclip[SCREENWIDTH]` (r_plane.h line 56).
    pub ceilingclip: [i16; SCREENWIDTH_USIZE],

    // =========================================================================
    // Span start/stop arrays (r_plane.c lines 78-79)
    // =========================================================================
    /// Per-row starting X column for horizontal span generation.
    /// When column extents change between adjacent columns, span starts
    /// are recorded here and spans are emitted when the extents decrease.
    ///
    /// Original C: `int spanstart[SCREENHEIGHT]` (r_plane.c line 78).
    pub spanstart: [i32; SCREENHEIGHT_USIZE],

    /// Per-row stopping X column for span generation.
    /// Note: unused in the original DOOM code but preserved for API
    /// completeness.
    ///
    /// Original C: `int spanstop[SCREENHEIGHT]` (r_plane.c line 79).
    pub spanstop: [i32; SCREENHEIGHT_USIZE],

    // =========================================================================
    // Texture mapping state (r_plane.c lines 82-93)
    // =========================================================================
    /// Index into `RenderMain.zlight[]` for the current plane's light level.
    /// Used in `map_plane` to select the distance-based colormap.
    ///
    /// Original C: `lighttable_t** planezlight` — a pointer into the 2D zlight
    /// array. Here stored as an index into the first dimension.
    pub planezlight: usize,

    /// Absolute height difference between the current plane and the viewpoint,
    /// in 16.16 fixed-point. Used to compute the distance to each scanline.
    ///
    /// Original C: `fixed_t planeheight` (r_plane.c line 83).
    pub planeheight: i32,

    /// Pre-calculated Y slope values for each screen row.
    /// `yslope[y]` converts `planeheight` to distance for scanline `y`.
    ///
    /// Original C: `fixed_t yslope[SCREENHEIGHT]` (r_plane.h line 61).
    pub yslope: [i32; SCREENHEIGHT_USIZE],

    /// Pre-calculated distance scale for each screen column.
    /// Used to correct span texture coordinate stepping for perspective.
    ///
    /// Original C: `fixed_t distscale[SCREENWIDTH]` (r_plane.h line 62).
    pub distscale: [i32; SCREENWIDTH_USIZE],

    /// Base X-axis scale for flat texture mapping, computed per frame from
    /// the view angle. `basexscale = FixedDiv(finecosine[angle], centerxfrac)`.
    ///
    /// Original C: `fixed_t basexscale` (r_plane.c line 87).
    pub basexscale: i32,

    /// Base Y-axis scale for flat texture mapping, computed per frame from
    /// the view angle. `baseyscale = -FixedDiv(finesine[angle], centerxfrac)`.
    ///
    /// Original C: `fixed_t baseyscale` (r_plane.c line 88).
    pub baseyscale: i32,

    // =========================================================================
    // Floor/ceiling drawing function dispatch (r_plane.h)
    // =========================================================================
    /// Span drawing function for floor rendering.
    /// Set to `DrawSpan` (high detail) or `DrawSpanLow` (low detail) by
    /// `R_ExecuteSetViewSize`.
    ///
    /// Original C: `planefunction_t floorfunc` (r_plane.c global).
    pub floorfunc: SpanFunc,

    /// Span drawing function for ceiling rendering.
    /// Set to `DrawSpan` (high detail) or `DrawSpanLow` (low detail) by
    /// `R_ExecuteSetViewSize`.
    ///
    /// Original C: `planefunction_t ceilingfunc` (r_plane.c global).
    pub ceilingfunc: SpanFunc,

    // =========================================================================
    // Per-scanline caches for R_MapPlane optimization (r_plane.c lines 90-93)
    // =========================================================================
    /// Cached plane height per scanline. When `planeheight` matches the
    /// cached value, distance/stepping values are reused without recomputation.
    ///
    /// Original C: `fixed_t cachedheight[SCREENHEIGHT]` (r_plane.c line 90).
    cachedheight: [i32; SCREENHEIGHT_USIZE],

    /// Cached distance per scanline (from `FixedMul(planeheight, yslope[y])`).
    ///
    /// Original C: `fixed_t cacheddistance[SCREENHEIGHT]` (r_plane.c line 91).
    cacheddistance: [i32; SCREENHEIGHT_USIZE],

    /// Cached X texture step per scanline.
    ///
    /// Original C: `fixed_t cachedxstep[SCREENHEIGHT]` (r_plane.c line 92).
    cachedxstep: [i32; SCREENHEIGHT_USIZE],

    /// Cached Y texture step per scanline.
    ///
    /// Original C: `fixed_t cachedystep[SCREENHEIGHT]` (r_plane.c line 93).
    cachedystep: [i32; SCREENHEIGHT_USIZE],
}

impl Default for PlaneState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaneState {
    /// Creates a new `PlaneState` with all fields zero-initialized.
    ///
    /// Visplane pool is pre-allocated to `MAXVISPLANES` entries.
    /// Openings buffer is pre-allocated to `MAXOPENINGS` entries.
    /// All clip arrays, caches, and lookup tables start at zero.
    pub fn new() -> Self {
        let mut visplanes = Vec::with_capacity(MAXVISPLANES);
        for _ in 0..MAXVISPLANES {
            visplanes.push(Visplane::default());
        }

        Self {
            visplanes,
            lastvisplane: 0,
            floorplane: None,
            ceilingplane: None,

            openings: vec![0i16; MAXOPENINGS],
            lastopening: 0,

            floorclip: [0i16; SCREENWIDTH_USIZE],
            ceilingclip: [0i16; SCREENWIDTH_USIZE],

            spanstart: [0i32; SCREENHEIGHT_USIZE],
            spanstop: [0i32; SCREENHEIGHT_USIZE],

            planezlight: 0,
            planeheight: 0,

            yslope: [0i32; SCREENHEIGHT_USIZE],
            distscale: [0i32; SCREENWIDTH_USIZE],
            basexscale: 0,
            baseyscale: 0,

            floorfunc: SpanFunc::DrawSpan,
            ceilingfunc: SpanFunc::DrawSpan,

            cachedheight: [0i32; SCREENHEIGHT_USIZE],
            cacheddistance: [0i32; SCREENHEIGHT_USIZE],
            cachedxstep: [0i32; SCREENHEIGHT_USIZE],
            cachedystep: [0i32; SCREENHEIGHT_USIZE],
        }
    }

    // =========================================================================
    // R_InitPlanes — r_plane.c lines 101-104
    // =========================================================================

    /// Initializes the plane rendering subsystem.
    ///
    /// In the original DOOM source, this function was a no-op with the comment
    /// "Doh!" — it exists solely for API symmetry with the other `R_Init*`
    /// functions. Preserved here for compatibility.
    ///
    /// Original C: `void R_InitPlanes(void)` (r_plane.c lines 101-104).
    pub fn init_planes(&mut self) {
        // Doh! — no initialization needed, matching original behavior.
    }

    // =========================================================================
    // R_MapPlane — r_plane.c lines 121-178
    // =========================================================================

    /// Maps a horizontal span of a floor or ceiling flat texture at a given
    /// screen row, from column `x1` to column `x2` inclusive.
    ///
    /// This is the core texture-mapping primitive for floor/ceiling rendering.
    /// It computes the world-space distance to the scanline, derives the
    /// texture stepping values, and dispatches to the appropriate span drawing
    /// function via the `DrawState`.
    ///
    /// # Caching Optimization
    ///
    /// For each scanline row, the distance, xstep, and ystep values are
    /// cached. When `planeheight` has not changed since the last call for
    /// the same row, the cached values are reused, avoiding expensive
    /// fixed-point multiplications.
    ///
    /// # Parameters
    ///
    /// - `y` — Screen row (0 = top of screen, SCREENHEIGHT-1 = bottom)
    /// - `x1` — Starting screen column (inclusive)
    /// - `x2` — Ending screen column (inclusive)
    /// - `render_main` — View state (viewpoint, angle, lighting tables)
    /// - `draw` — Low-level drawing state (span parameters and drawing method)
    /// - `screens` — Video screen buffers
    /// - `colormaps` — Global colormap data
    ///
    /// Original C: `void R_MapPlane(int y, int x1, int x2)` (r_plane.c lines 121-178).
    pub fn map_plane(
        &mut self,
        y: i32,
        x1: i32,
        x2: i32,
        render_main: &RenderMain,
        draw: &mut DrawState,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
    ) {
        // RANGECHECK equivalent — validate inputs in debug builds
        #[cfg(debug_assertions)]
        {
            if x2 < x1
                || !(0..SCREENWIDTH).contains(&x1)
                || !(0..SCREENWIDTH).contains(&x2)
                || !(0..SCREENHEIGHT).contains(&y)
            {
                tracing::warn!("R_MapPlane: invalid range y={} x1={} x2={}", y, x1, x2);
            }
        }

        let y_idx = y as usize;

        // Distance calculation with caching optimization.
        // If planeheight hasn't changed since last call for this row,
        // reuse the cached values.
        let distance: i32;
        let ds_xstep: i32;
        let ds_ystep: i32;

        if self.planeheight != self.cachedheight[y_idx] {
            // Cache miss — recompute
            self.cachedheight[y_idx] = self.planeheight;

            // distance = FixedMul(planeheight, yslope[y])
            let planeheight_fixed = Fixed::new(self.planeheight);
            let yslope_fixed = Fixed::new(self.yslope[y_idx]);
            distance = planeheight_fixed.fixed_mul(yslope_fixed).raw();
            self.cacheddistance[y_idx] = distance;

            // ds_xstep = FixedMul(distance, basexscale)
            let dist_fixed = Fixed::new(distance);
            ds_xstep = dist_fixed.fixed_mul(Fixed::new(self.basexscale)).raw();
            self.cachedxstep[y_idx] = ds_xstep;

            // ds_ystep = FixedMul(distance, baseyscale)
            ds_ystep = dist_fixed.fixed_mul(Fixed::new(self.baseyscale)).raw();
            self.cachedystep[y_idx] = ds_ystep;
        } else {
            // Cache hit — reuse previously computed values
            distance = self.cacheddistance[y_idx];
            ds_xstep = self.cachedxstep[y_idx];
            ds_ystep = self.cachedystep[y_idx];
        }

        // Compute texture coordinate origin for this scanline.
        // The view angle is decomposed into fine sine/cosine components,
        // and the distance is projected along the view direction.
        let angle_val = (render_main
            .viewangle
            .value()
            .wrapping_add(render_main.xtoviewangle[x1 as usize].value()))
            >> ANGLETOFINESHIFT;
        let angle_idx = angle_val as usize & 8191; // FINEMASK

        let dist_fixed = Fixed::new(distance);

        // ds_xfrac = viewx + FixedMul(finecosine[angle], distance)
        let length = dist_fixed
            .fixed_mul(Fixed::new(self.distscale[x1 as usize]))
            .raw();
        let length_fixed = Fixed::new(length);
        let cos_val = finecosine(angle_idx);
        let sin_val = FINESINE[angle_idx];

        // Note: ds_yfrac uses NEGATIVE viewy, matching original C:
        //   ds_xfrac = viewx + FixedMul(finecosine[pangle], length)
        //   ds_yfrac = -viewy - FixedMul(finesine[pangle], length)
        draw.ds_xfrac = render_main
            .viewx
            .raw()
            .wrapping_add(length_fixed.fixed_mul(cos_val).raw());
        draw.ds_yfrac = render_main
            .viewy
            .raw()
            .wrapping_neg()
            .wrapping_sub(length_fixed.fixed_mul(sin_val).raw());

        // Set span stepping and parameters
        draw.ds_xstep = ds_xstep;
        draw.ds_ystep = ds_ystep;

        // Select colormap: fixedcolormap overrides distance-based lighting
        if let Some(fixed_cm) = render_main.fixedcolormap {
            draw.ds_colormap = fixed_cm;
        } else {
            // Index into zlight table by distance
            let mut index = (distance as u32 >> LIGHTZSHIFT) as usize;
            if index >= MAXLIGHTZ {
                index = MAXLIGHTZ - 1;
            }
            draw.ds_colormap = render_main.zlight[self.planezlight][index];
        }

        draw.ds_y = y;
        draw.ds_x1 = x1;
        draw.ds_x2 = x2;

        // Call the span drawing function
        draw.draw_span(screens, colormaps);
    }

    // =========================================================================
    // R_ClearPlanes — r_plane.c lines 185-209
    // =========================================================================

    /// Resets all per-frame plane rendering state at the beginning of each
    /// rendering frame.
    ///
    /// This must be called before any BSP traversal or wall rendering occurs.
    /// It:
    /// - Initializes `floorclip[i]` to `viewheight` (bottom of screen)
    /// - Initializes `ceilingclip[i]` to `-1` (above top of screen)
    /// - Resets the visplane and openings allocators
    /// - Clears the per-scanline computation cache
    /// - Computes `basexscale` and `baseyscale` from the current view angle
    ///
    /// # Parameters
    ///
    /// - `viewangle` — Current camera facing direction (BAM angle)
    /// - `viewwidth` — Current rendered viewport width in pixels
    /// - `viewheight` — Current rendered viewport height in pixels
    /// - `centerxfrac` — Center X pixel in 16.16 fixed-point
    ///
    /// Original C: `void R_ClearPlanes(int viewwidth, int viewheight)` but
    /// also accesses `viewangle` global (r_plane.c lines 185-209).
    pub fn clear_planes(
        &mut self,
        viewwidth: i32,
        viewheight: i32,
        viewangle: u32,
        centerxfrac: i32,
    ) {
        // Initialize per-column clip arrays
        // floorclip = viewheight (nothing clipped from bottom)
        // ceilingclip = -1 (nothing clipped from top)
        for i in 0..viewwidth as usize {
            self.floorclip[i] = viewheight as i16;
            self.ceilingclip[i] = -1;
        }

        // Reset visplane allocator
        self.lastvisplane = 0;
        self.lastopening = 0;

        // Clear the per-scanline cache to force recomputation on first use.
        // Setting cachedheight to 0 means any non-zero planeheight will be
        // a cache miss on the first access.
        for i in 0..SCREENHEIGHT_USIZE {
            self.cachedheight[i] = 0;
        }

        // Compute base texture mapping scale from the view angle.
        // angle = (viewangle - ANG90) >> ANGLETOFINESHIFT
        let angle_val = viewangle.wrapping_sub(ANG90.value()) >> ANGLETOFINESHIFT;
        let angle_idx = angle_val as usize & 8191; // FINEMASK

        let centerxfrac_fixed = Fixed::new(centerxfrac);

        // basexscale = FixedDiv(finecosine[angle], centerxfrac)
        self.basexscale = finecosine(angle_idx).fixed_div(centerxfrac_fixed).raw();

        // baseyscale = -FixedDiv(finesine[angle], centerxfrac)
        self.baseyscale = FINESINE[angle_idx]
            .fixed_div(centerxfrac_fixed)
            .raw()
            .wrapping_neg();
    }

    // =========================================================================
    // R_FindPlane — r_plane.c lines 217-259
    // =========================================================================

    /// Finds an existing visplane matching the given parameters, or allocates
    /// a new one.
    ///
    /// # Sky Special Case
    ///
    /// When `picnum` matches the sky flat number (`skyflatnum`), the height
    /// and light level are forced to 0, causing all sky sectors to share a
    /// single visplane regardless of their actual height or light level.
    ///
    /// # Returns
    ///
    /// The index of the matching or newly allocated visplane.
    ///
    /// # Panics
    ///
    /// Panics if the visplane pool is exhausted (more than `MAXVISPLANES`
    /// visplanes are needed in a single frame), matching the original
    /// `I_Error("R_FindPlane: no more visplanes")`.
    ///
    /// Original C: `visplane_t* R_FindPlane(fixed_t height, int picnum, int lightlevel)`
    /// (r_plane.c lines 217-259).
    pub fn find_plane(
        &mut self,
        height: i32,
        picnum: i32,
        lightlevel: i32,
        skyflatnum: i32,
    ) -> usize {
        // Sky special case: force height=0, lightlevel=0
        let (check_height, check_light) = if picnum == skyflatnum {
            (0i32, 0i32)
        } else {
            (height, lightlevel)
        };

        // Search existing visplanes for a match
        // Original C searches [0..lastvisplane), checking height/picnum/lightlevel
        for i in 0..self.lastvisplane {
            let vp = &self.visplanes[i];
            if vp.height.raw() == check_height
                && vp.picnum == picnum
                && vp.lightlevel == check_light
            {
                return i;
            }
        }

        // No match found — allocate a new visplane
        if self.lastvisplane >= MAXVISPLANES {
            tracing::error!("R_FindPlane: no more visplanes");
            panic!("R_FindPlane: no more visplanes");
        }

        let idx = self.lastvisplane;
        self.lastvisplane += 1;

        // Initialize the new visplane
        let vp = &mut self.visplanes[idx];
        vp.height = Fixed::new(check_height);
        vp.picnum = picnum;
        vp.lightlevel = check_light;
        vp.minx = SCREENWIDTH;
        vp.maxx = -1;

        // Fill top array with sentinel value 0xff — "no data for this column"
        for t in vp.top.iter_mut() {
            *t = 0xff;
        }

        idx
    }

    // =========================================================================
    // R_CheckPlane — r_plane.c lines 265-324
    // =========================================================================

    /// Validates that a visplane can extend to cover columns `[start, stop]`.
    ///
    /// If the new column range overlaps with existing data in the visplane
    /// (i.e., any column in the intersection already has `top[x] != 0xff`),
    /// a new visplane is allocated with the same properties to cover the
    /// new range. Otherwise, the existing visplane's `minx`/`maxx` are
    /// extended in place.
    ///
    /// # Returns
    ///
    /// The index of the visplane to use — either the original `pl_idx`
    /// (if extended) or a newly allocated visplane index.
    ///
    /// # Panics
    ///
    /// Panics if allocating a new visplane exceeds `MAXVISPLANES`.
    ///
    /// Original C: `visplane_t* R_CheckPlane(visplane_t* pl, int start, int stop)`
    /// (r_plane.c lines 265-324).
    pub fn check_plane(&mut self, pl_idx: usize, start: i32, stop: i32) -> usize {
        // Compute the intersection of existing [minx, maxx] with new [start, stop]
        let existing_minx = self.visplanes[pl_idx].minx;
        let existing_maxx = self.visplanes[pl_idx].maxx;

        // intrl = max(start, minx) — intersection left
        let intrl = if start < existing_minx {
            existing_minx
        } else {
            start
        };

        // intrh = min(stop, maxx) — intersection right
        let intrh = if stop > existing_maxx {
            existing_maxx
        } else {
            stop
        };

        // Check if intersection area has any filled columns
        // If intrl > intrh, there is no intersection — safe to extend
        let mut need_new = false;
        if intrl <= intrh {
            // Scan the intersection for any non-sentinel values
            let mut x = intrl;
            while x <= intrh {
                if self.visplanes[pl_idx].top[x as usize] != 0xff {
                    need_new = true;
                    break;
                }
                x += 1;
            }
        }

        if !need_new {
            // No overlap — extend the existing visplane's range
            if start < existing_minx {
                self.visplanes[pl_idx].minx = start;
            }
            if stop > existing_maxx {
                self.visplanes[pl_idx].maxx = stop;
            }
            return pl_idx;
        }

        // Overlap detected — allocate a new visplane with the same properties
        if self.lastvisplane >= MAXVISPLANES {
            tracing::error!("R_CheckPlane: no more visplanes");
            panic!("R_CheckPlane: no more visplanes");
        }

        // Copy properties from the original visplane
        let height = self.visplanes[pl_idx].height;
        let picnum = self.visplanes[pl_idx].picnum;
        let lightlevel = self.visplanes[pl_idx].lightlevel;

        let new_idx = self.lastvisplane;
        self.lastvisplane += 1;

        let new_vp = &mut self.visplanes[new_idx];
        new_vp.height = height;
        new_vp.picnum = picnum;
        new_vp.lightlevel = lightlevel;
        new_vp.minx = start;
        new_vp.maxx = stop;

        // Fill top array with sentinel value
        for t in new_vp.top.iter_mut() {
            *t = 0xff;
        }

        new_idx
    }

    // =========================================================================
    // R_MakeSpans — r_plane.c lines 330-359
    // =========================================================================

    /// Converts per-column top/bottom visplane data into horizontal spans
    /// and emits them via [`map_plane`](PlaneState::map_plane).
    ///
    /// This is the key column-to-span conversion algorithm. It is called
    /// for each column in sequence during `draw_planes`. The parameters
    /// represent the current column's top/bottom extents (`t1`/`b1`) and
    /// the next column's extents (`t2`/`b2`).
    ///
    /// - When the top extent decreases (`t1 < t2`): horizontal spans that
    ///   ended at this column are emitted via `map_plane`.
    /// - When the top extent increases (`t2 < t1`): new span starting
    ///   positions are recorded in `spanstart`.
    /// - Same logic applies for the bottom extent.
    ///
    /// # Parameters
    ///
    /// - `x` — Current column being processed
    /// - `t1` — Top of visible area at the current column
    /// - `b1` — Bottom of visible area at the current column
    /// - `t2` — Top of visible area at the next column
    /// - `b2` — Bottom of visible area at the next column
    /// - `render_main` — View state for map_plane calls
    /// - `draw` — Drawing state for span emission
    /// - `screens` — Video screen buffers
    /// - `colormaps` — Global colormap data
    ///
    /// Original C: `void R_MakeSpans(int x, int t1, int b1, int t2, int b2)`
    /// (r_plane.c lines 330-359).
    pub fn make_spans(
        &mut self,
        x: i32,
        t1: i32,
        b1: i32,
        t2: i32,
        b2: i32,
        render_main: &RenderMain,
        draw: &mut DrawState,
        screens: &mut [Vec<u8>],
        colormaps: &[u8],
    ) {
        // Handle top edge: emit spans that ended, record new span starts
        //
        // Original C (r_plane.c lines 335-346):
        //   while (t1 < t2 && t1<=b1) { spanfunc(t1,spanstart[t1],x-1); t1++; }
        //   while (t2 < t1 && t2<=b2) { spanstart[t2] = x; t2++; }
        let mut t1 = t1;
        let mut t2 = t2;
        let mut b1 = b1;
        let mut b2 = b2;

        // Emit spans where top boundary decreased (t1 < t2)
        while t1 < t2 && t1 <= b1 {
            let row = t1 as usize;
            if row < SCREENHEIGHT_USIZE {
                self.map_plane(
                    t1,
                    self.spanstart[row],
                    x - 1,
                    render_main,
                    draw,
                    screens,
                    colormaps,
                );
            }
            t1 += 1;
        }

        // Record span starts where top boundary increased (t2 < t1)
        while t2 < t1 && t2 <= b2 {
            let row = t2 as usize;
            if row < SCREENHEIGHT_USIZE {
                self.spanstart[row] = x;
            }
            t2 += 1;
        }

        // Handle bottom edge: emit spans that ended, record new span starts
        //
        // Original C (r_plane.c lines 348-358):
        //   while (b1 > b2 && b1>=t1) { spanfunc(b1,spanstart[b1],x-1); b1--; }
        //   while (b2 > b1 && b2>=t2) { spanstart[b2] = x; b2--; }

        // Emit spans where bottom boundary increased (b1 > b2)
        while b1 > b2 && b1 >= t1 {
            let row = b1 as usize;
            if row < SCREENHEIGHT_USIZE {
                self.map_plane(
                    b1,
                    self.spanstart[row],
                    x - 1,
                    render_main,
                    draw,
                    screens,
                    colormaps,
                );
            }
            b1 -= 1;
        }

        // Record span starts where bottom boundary decreased (b2 > b1)
        while b2 > b1 && b2 >= t2 {
            let row = b2 as usize;
            if row < SCREENHEIGHT_USIZE {
                self.spanstart[row] = x;
            }
            b2 -= 1;
        }
    }

    // =========================================================================
    // R_DrawPlanes — r_plane.c lines 367-453
    // =========================================================================

    /// Renders all allocated visplanes for the current frame.
    ///
    /// Called at the end of each rendering frame, after all BSP traversal and
    /// wall rendering is complete. Iterates through all active visplanes and
    /// renders them as either:
    ///
    /// - **Sky columns** — When `picnum == skyflatnum`: renders vertical
    ///   sky texture columns using the column drawing primitive. Sky is always
    ///   drawn at full brightness (`colormaps[0]`).
    ///
    /// - **Flat spans** — For all other visplanes: loads the flat texture
    ///   data from the WAD, computes lighting, and renders horizontal spans
    ///   using the span drawing primitive via `make_spans` → `map_plane`.
    ///
    /// Original C: `void R_DrawPlanes(void)` (r_plane.c lines 367-453).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_planes(
        &mut self,
        render_main: &RenderMain,
        draw: &mut DrawState,
        data: &mut DataState,
        sky: &SkyState,
        things: &ThingsState,
        wad: &mut dyn WadProvider,
        screens: &mut [Vec<u8>],
        colormaps_data: &[u8],
    ) {
        // Iterate all active visplanes
        for pl_idx in 0..self.lastvisplane {
            // Skip empty visplanes
            let minx = self.visplanes[pl_idx].minx;
            let maxx = self.visplanes[pl_idx].maxx;
            if minx > maxx {
                continue;
            }

            let picnum = self.visplanes[pl_idx].picnum;

            // =================================================================
            // Sky rendering path
            // =================================================================
            if picnum == sky.skyflatnum {
                // Sky column drawing parameters:
                // dc_iscale = pspriteiscale >> detailshift
                draw.dc_iscale = things.pspriteiscale.raw() >> render_main.detailshift;

                // Sky is ALWAYS full bright — use colormaps base (index 0)
                // This is an explicit design choice in the original engine:
                // "dc_colormap = colormaps;" (r_plane.c line 395)
                draw.dc_colormap = 0;

                // Sky texture vertical offset
                draw.dc_texturemid = sky.skytexturemid;

                for x in minx..=maxx {
                    // Check that this column has valid data
                    let top_val = self.visplanes[pl_idx].top[x as usize];
                    let bottom_val = self.visplanes[pl_idx].bottom[x as usize];
                    if top_val == 0xff {
                        continue;
                    }

                    draw.dc_yl = top_val as i32;
                    draw.dc_yh = bottom_val as i32;

                    // Compute sky texture column from viewangle + xtoviewangle[x]
                    let angle = (render_main
                        .viewangle
                        .value()
                        .wrapping_add(render_main.xtoviewangle[x as usize].value()))
                        >> ANGLETOSKYSHIFT;

                    // Get column data from the sky texture
                    let col_data = data.get_column(sky.skytexture as usize, angle as i32);

                    // Set the source column data for the column drawing function.
                    // We copy the column data into dc_source since the drawer
                    // requires owned data for its rendering loop.
                    draw.dc_source = col_data.to_vec();
                    draw.dc_x = x;

                    // Dispatch to the appropriate column drawing function
                    match render_main.colfunc {
                        ColFunc::DrawColumn => {
                            draw.draw_column(screens, colormaps_data, render_main.centery);
                        }
                        ColFunc::DrawColumnLow => {
                            draw.draw_column_low(screens, colormaps_data, render_main.centery);
                        }
                        _ => {
                            // Sky always uses basic column draw (no fuzz/translation)
                            draw.draw_column(screens, colormaps_data, render_main.centery);
                        }
                    }
                }

                continue; // Done with this sky visplane
            }

            // =================================================================
            // Flat (floor/ceiling) rendering path
            // =================================================================

            // Load the flat texture data from WAD.
            // lumpnum = firstflat + flattranslation[picnum]
            let translated = if (picnum as usize) < data.flattranslation.len() {
                data.flattranslation[picnum as usize]
            } else {
                picnum
            };
            let lumpnum = (data.firstflat + translated) as usize;

            // Cache the flat data as PurgeTag::Static during rendering
            let flat_data = wad.cache_lump_num(lumpnum, PurgeTag::Static).to_vec();
            draw.ds_source = flat_data;

            // planeheight = abs(pl->height - viewz)
            let height_raw = self.visplanes[pl_idx].height.raw();
            let viewz_raw = render_main.viewz.raw();
            self.planeheight = (height_raw.wrapping_sub(viewz_raw)).abs();

            // Select light level for this plane.
            // light = (pl->lightlevel >> LIGHTSEGSHIFT) + extralight
            let lightlevel = self.visplanes[pl_idx].lightlevel;
            let mut light = (lightlevel >> LIGHTSEGSHIFT) + render_main.extralight;

            // Clamp light to valid range [0, LIGHTLEVELS-1]
            if light >= LIGHTLEVELS as i32 {
                light = LIGHTLEVELS as i32 - 1;
            }
            if light < 0 {
                light = 0;
            }
            self.planezlight = light as usize;

            // Set sentinels for the make_spans boundary check.
            // In the original C, padding bytes around the top/bottom arrays
            // provided sentinel access at top[minx-1] and top[maxx+1].
            // In Rust, we handle this in the make_spans loop below by
            // treating out-of-bounds access as 0xff sentinels.

            // Copy top/bottom arrays to local buffers to avoid borrow checker
            // conflicts when calling make_spans (which borrows self mutably).
            //
            // Buffer layout: local_top/bottom use a +1 offset so that
            // local_top[x + 1] = visplane.top[x]. This allows accessing
            // top[x-1] as local_top[x] and top[x] as local_top[x + 1],
            // providing sentinel values at both boundaries:
            //   - local_top[minx] = 0xff (sentinel for top[minx-1])
            //   - local_top[maxx+2] = 0xff (sentinel for top[maxx+1])
            //   - local_bottom[minx] = 0 (sentinel for bottom[minx-1])
            //   - local_bottom[maxx+2] = 0 (sentinel for bottom[maxx+1])
            let mut local_top = [0xffu8; SCREENWIDTH_USIZE + 2];
            let mut local_bottom = [0u8; SCREENWIDTH_USIZE + 2];

            for x in minx..=maxx {
                let xu = x as usize;
                local_top[xu + 1] = self.visplanes[pl_idx].top[xu];
                local_bottom[xu + 1] = self.visplanes[pl_idx].bottom[xu];
            }
            // Sentinel values are automatically correct:
            // local_top[minx] and local_top[maxx+2] remain 0xff from init,
            // local_bottom[minx] and local_bottom[maxx+2] remain 0 from init.
            // This matches the original C padding bytes (pad1/pad2 for top,
            // pad3/pad4 for bottom).

            // Iterate columns [minx, maxx+1] calling make_spans.
            //
            // Original C (r_plane.c lines 441-449):
            //   stop = pl->maxx + 1;
            //   for (x = pl->minx; x <= stop; x++)
            //       R_MakeSpans(x, pl->top[x-1], pl->bottom[x-1],
            //                      pl->top[x],   pl->bottom[x]);
            //
            // With +1 offset: top[x-1] = local_top[x], top[x] = local_top[x+1]
            let stop_col = maxx + 1;
            for x in minx..=stop_col {
                let xu = x as usize;

                // t1 = top[x-1] (previous column)
                let t1 = local_top[xu] as i32;
                // b1 = bottom[x-1] (previous column)
                let b1 = local_bottom[xu] as i32;
                // t2 = top[x] (current column)
                let t2 = local_top[xu + 1] as i32;
                // b2 = bottom[x] (current column)
                let b2 = local_bottom[xu + 1] as i32;

                self.make_spans(
                    x,
                    t1,
                    b1,
                    t2,
                    b2,
                    render_main,
                    draw,
                    screens,
                    colormaps_data,
                );
            }

            // Release the flat data back to cache after rendering.
            // This matches the original Z_ChangeTag(source, PU_CACHE).
            // The actual tag change is managed by the WAD provider's internal
            // cache; calling cache_lump_num with PurgeTag::Cache marks the
            // lump as evictable.
            let _ = wad.cache_lump_num(lumpnum, PurgeTag::Cache);
        }
    }
}
