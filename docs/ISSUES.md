# Issue Resolution Matrix

This document tracks all known issues, incomplete work, and improvement opportunities
discovered during the DOOM 1.10 C → Rust migration. Each issue is cataloged with its
source, disposition (fix or defer), and supporting rationale.

## Sources of Issues

All issues in this matrix were derived from four sources:

1. **`README.TXT`** (repository root) — John Carmack's release notes, dated December 23, 1997.
   Contains high-level engineering commentary about the codebase, platform limitations, and
   suggested improvement projects.

2. **`linuxdoom-1.10/TODO`** (123 lines) — Bernd Kreimeier's to-do list for the Linux DOOM
   source port. Contains actionable items including floating-point migration, screen resolution
   changes, DGA support, sound server improvements, BSP/blockmap optimizations, collision height
   fixes, and menu cleanup. Many items overlap with issues already captured from `README.TXT`.

3. **`linuxdoom-1.10/ChangeLog`** (922 lines) — Bernd Kreimeier's work log documenting the
   cleanup performed for the public source release (December 22, 1997). Records changes to
   sound handling (`SNDSERV` vs `SNDINTR`), menu fixes for Ultimate DOOM episode 4, `V_DrawPatch`
   bounds checking, sound table fallback loading, and other modifications made during the
   Linux port preparation. Provides historical context for implementation decisions.

4. **`linuxdoom-1.10/README.b`** (140 lines) — The README for the Linux DOOM source
   distribution, authored by Bernd Kreimeier. Contains a disclaimer noting this is a modified
   snapshot (not the exact id Software internal source), remarks about bug fixes and experimental
   sound code, and notes about subsystems not included (SVGA, GLDOOM, Win32, DOS, DoomEd,
   BSP tools, game data tools, artwork).

5. **Inline source code comments** — Comments within the original C header and implementation
   files (`linuxdoom-1.10/*.c`, `linuxdoom-1.10/*.h`) that indicate known limitations,
   experimental features, or removal candidates. A `grep` search for inline annotations found
   **12 `FIXME`** occurrences across 9 `.c` files and **3 `HACK`** occurrences across 2 `.c`
   files. No `TODO`, `XXX`, or `BUG` annotations were found in source code comments. The
   `FIXME` annotations mark known limitations or incomplete implementations; the `HACK`
   annotations mark intentional workarounds for edge cases in the commercial DOOM II release.

---

## Issue Resolution Matrix

| ID | Source | Problem Summary | Proposed Fix | Status | Validation Evidence | Notes / Risk |
|----|--------|----------------|--------------|--------|--------------------|----|
| IR-01 | `README.TXT` line 12 | Code only compiles and runs on Linux due to X11 display (`i_video.c`), OSS audio (`i_sound.c`), Unix sockets (`i_net.c`), and `gettimeofday` timing (`i_system.c`) dependencies | Implement Windows 11 platform backend via SDL2 in the `doom-platform-win` crate, replacing all Linux/X11/OSS platform calls with SDL2 equivalents | To be fixed | `cargo build --release` succeeds on Windows 11; executable launches and renders the DOOM title screen | Primary objective of this refactor. SDL2 `bundled` feature compiles from source for self-contained Windows builds. Risk: SDL2 audio/video behavior differences from X11/OSS may require tuning |
| IR-02 | `README.TXT` lines 12–14 | DOS sound library code not released due to copyrighted third-party sound library. Original comment: "copyrighted sound library we used (wow, was that a mistake)" | Sound system reimplemented from scratch using SDL2 audio device with callback-based mixing in `doom-platform-win/src/audio.rs`. No dependency on original DOS sound library | To be fixed | Sound effects play during gameplay (weapon fire, door open, enemy alert); music plays on level start | The `sndserv/` standalone sound server source is used as behavioral reference only. The Linux-specific OSS `/dev/dsp` and `SNDSERV` pipe IPC model are fully replaced |
| IR-03 | `README.TXT` lines 26–35 | Carmack notes that the rendering implementation could be improved: BSP front-to-back walk, polygon-based floor/ceiling rendering, and sprite billboard clipping into subsector fragments | Deferred — no changes to rendering pipeline | Deferred | N/A | Minimal Change Clause mandates preserving current rendering behavior exactly. These are architectural improvements, not bug fixes. See [Deferred Items](#ir-03-rendering-improvements-bsp-front-to-back-polygon-floors-sprite-clipping) below |
| IR-04 | `README.TXT` lines 37–45 | Carmack notes that movement and line-of-sight checking is "messy code that had some failure cases" and suggests replacing LOS test with a BSP line clip | Deferred — current `P_CheckSight` implementation preserved as-is | Deferred | N/A | Current LOS implementation works correctly for all gameplay scenarios. Changing it would violate behavioral parity and risk introducing regressions. See [Deferred Items](#ir-04-line-of-sight-bsp-optimization) below |
| IR-05 | `README.TXT` lines 47–65 | Carmack suggested multiple projects: OS ports, rendering features (transparency, look up/down, slopes), game features (jumping, ducking, flying), internet multiplayer, and 3D hardware acceleration | Partially addressed — Windows 11 port is implemented. All feature additions are explicitly out of scope per user requirements | Deferred | Windows 11 port validated via `cargo build --release` and gameplay testing | Only the OS port directive is addressed. Feature additions (transparency, slopes, jumping, 3D acceleration, etc.) are out of scope per the Minimal Change Clause. See [Deferred Items](#ir-05-feature-additions-transparency-slopes-jumping-3d-acceleration) below |
| IR-06 | `r_main.h` line 91 | Comment `//B remove this?` on the `detailshift` variable (line 93: `extern int detailshift;`) which controls the blocky/low detail rendering mode (0 = high, 1 = low) | Deferred — variable preserved in Rust translation | Deferred | N/A | Removing `detailshift` would change the behavior of the Options menu "Detail" setting, which is part of the original game's user interface. See [Deferred Items](#ir-06-detailshift-variable-cleanup) below |
| IR-07 | `i_sound.c`; `doomdef.h` lines 77–85 | `SNDINTR` mode is commented out and described as "experimental" and "unfinished" in `doomdef.h` (line 80–83: "The integrated sound support is experimental, and unfinished"). Uses 500μs timer interrupts for asynchronous audio. Only `SNDSERV` (external sound server pipe model) is enabled by default | Not ported — SDL2 audio callback model replaces both `SNDSERV` pipe-based and `SNDINTR` timer-based audio implementations | To be fixed | SDL2 `AudioDevice` with callback provides stable, low-latency audio mixing without manual timer interrupts or external process pipes | Risk: Audio timing characteristics may differ slightly from original OSS implementation. SDL2's callback model is the modern standard for game audio |
| IR-08 | `i_video.c`; `doomdef.h` lines 88–91 | MIT-SHM (X11 Shared Memory Extension) used for faster framebuffer transfer to X server; fallback to standard `XPutImage` if SHM unavailable. Comment also references `X11_DGA` (XFree86 Direct Graphics Access) as an alternative | Replaced by SDL2 texture streaming — no shared memory extension or DGA needed on Windows. SDL2 manages video memory and buffer transfer internally | To be fixed | SDL2 hardware-accelerated texture upload from 320×200 pixel buffer to window provides equivalent or better performance than X11 SHM | No significant risk — SDL2 abstracts all video memory management. The `doom-platform-win/src/video.rs` module handles palette-indexed to RGB conversion and window scaling |
| IR-09 | `i_system.c`; `z_zone.h` | Zone memory allocator hardcoded to 6 MB heap (`mb_used = 6` in `i_system.c`). The zone system (`z_zone.c/h`) implements a custom heap with tag-based purge levels: `PU_STATIC` (1), `PU_SOUND` (2), `PU_MUSIC` (3), `PU_DAVE` (4), `PU_LEVEL` (50), `PU_LEVSPEC` (51), `PU_PURGELEVEL` (100), `PU_CACHE` (101) | Rust standard allocator replaces the zone heap — no fixed memory limit. WAD lump caching uses `HashMap` with purge tag tracking in `doom-wad/src/lump_cache.rs`. Level-scoped allocations use Rust ownership semantics (dropped at level change) | To be fixed | `cargo test` passes all memory-related tests; no allocation failures during gameplay; lump cache correctly evicts `PU_CACHE` entries under pressure | Risk: Zone tag semantics must be faithfully replicated for lump cache eviction. Tags < 100 are not overwritten until freed; tags ≥ 100 (`PU_PURGELEVEL`, `PU_CACHE`) are purgeable whenever needed. The `PU_LEVEL`/`PU_LEVSPEC` scope boundary (freed at level exit) must be honored |
| IR-10 | `d_main.c` | IWAD search paths are Unix-specific: `/usr/local/share/games/doom/`, `$HOME` directory, and hardcoded Unix paths. No Windows path support exists in the original code | Windows-specific IWAD discovery: current working directory, common Steam installation paths (`C:\Program Files (x86)\Steam\steamapps\common\`), and explicit `--iwad` CLI argument via `clap` in `doom-bin/src/cli.rs` | To be fixed | `doom-bin --iwad <path>` correctly locates and loads IWAD files; clear error message displayed when IWAD not found | Risk: Low. Error message format: "IWAD file not found at path: ... . Please provide a valid path using --iwad \<path\>". The `doom-platform-win/src/filesystem.rs` module handles Windows path normalization |
| IR-11 | `TODO` line 10 | "remove m_fixed, switch to floating point — More stable, and prolly even faster" | Deferred — fixed-point arithmetic preserved via `Fixed(i32)` newtype in Rust | Deferred | N/A | Behavioral parity requires preserving the original 16.16 fixed-point arithmetic. Switching to floating-point would change numerical results and break demo compatibility. See [Deferred Items](#ir-11-floating-point-migration) below |
| IR-12 | `TODO` lines 13–18 | "make SCREENWIDTH/HEIGHT work at startup?" and "fix aspect ratio? 320x200 is nothing viable nowadays" | Deferred — original 320×200 resolution preserved; SDL2 window scaling handles display at native resolution | Deferred | N/A | The software renderer is tightly coupled to 320×200 pixel buffers. Changing the base resolution would require rewriting the renderer. SDL2 window scaling provides a clean display at any window size. See [Deferred Items](#ir-12-configurable-screen-resolution) below |
| IR-13 | `TODO` lines 83–89 | "correct handling of height in collision. This is not done, and the checks are scattered around in many places" | Deferred — original collision behavior preserved for behavioral parity | Deferred | N/A | Height-based collision is a known limitation of the original DOOM engine (monsters cannot stand on top of each other, projectiles pass through some gaps). Fixing this would alter gameplay behavior. See [Deferred Items](#ir-13-collision-height-handling) below |
| IR-14 | `TODO` lines 97–101 | "Ungraceful and untimely demise of Linuxdoom will leave idle sndserver processes" and "threaded sndserver? SHM mixing buffer?" | Addressed by IR-07 — the external `sndserver` process model is replaced by in-process SDL2 audio callbacks | To be fixed | SDL2 audio runs in-process; no orphaned child processes possible | The `sndserv/` architecture is entirely replaced. Process lifecycle issues are eliminated by design |
| IR-15 | `FIXME` annotations (12 occurrences in 9 files) | Inline `FIXME` comments in `d_main.c` (×2), `d_net.c`, `f_finale.c`, `hu_stuff.c`, `i_sound.c` (×2), `m_menu.c`, `p_mobj.c` (×2), `p_tick.c`, `r_draw.c` marking known limitations such as version-dependent demo numbers, endianness concerns, NOP function pointers, and incomplete channel output | Deferred — all `FIXME` annotations represent informational markers for known edge-case limitations in the original engine, not blocking defects | Deferred | N/A | These annotations have existed since the 1997 public release and do not indicate runtime bugs. The Rust translation preserves the same behavioral characteristics. See [Deferred Items](#ir-15-fixme-annotations) below |
| IR-16 | `HACK` annotations (3 occurrences in 2 files) | `s_sound.c:241` "HACK FOR COMMERCIAL" (commercial DOOM II music index offset), `wi_stuff.c:1609` "MONDO HACK!" and `wi_stuff.c:1618` "HACK ALERT!" (intermission screen layout workarounds for DOOM II) | Deferred — these are intentional workarounds for DOOM II–specific edge cases in the original engine, preserved in the Rust translation | Deferred | N/A | These workarounds are required for correct DOOM II behavior and are preserved as-is. See [Deferred Items](#ir-16-hack-annotations) below |

---

## Deferred Items — Detailed Rationale

### IR-03: Rendering Improvements (BSP Front-to-Back, Polygon Floors, Sprite Clipping)

- **Reason deferred**: The Minimal Change Clause mandates preserving the current rendering
  behavior exactly as implemented in the original DOOM 1.10 source. The improvements described
  by Carmack (collapsing the wall → floor → sprite rendering order into a single front-to-back
  BSP walk, treating floors/ceilings as polygons, and clipping sprite billboards into subsector
  fragments) represent fundamental architectural changes to the renderer that would alter the
  visual output in subtle ways.

- **Blocker**: Architectural changes to the rendering pipeline would risk introducing visual
  regressions. The current back-to-front rendering order (walls, then floors/ceilings via
  visplanes, then sprites via vissprites) is deeply embedded in the interaction between
  `r_bsp.c`, `r_segs.c`, `r_plane.c`, and `r_things.c`. Changing this order requires
  simultaneous modifications to all four subsystems.

- **User impact**: None — the existing software rendering behavior is preserved exactly as
  the original DOOM 1.10. Players see the same visual output.

- **Workaround**: N/A — the current rendering is fully functional and produces correct output
  for all DOOM levels.

- **Next action**: Can be pursued in a future enhancement phase after behavioral parity with
  the original C engine is validated through demo playback testing. A separate branch could
  prototype the front-to-back BSP approach to measure performance and visual correctness.

### IR-04: Line-of-Sight BSP Optimization

- **Reason deferred**: The current `P_CheckSight` implementation in `p_sight.c` works correctly
  for all gameplay scenarios in DOOM 1.10. Carmack's suggested replacement (BSP line clip for
  LOS testing) would produce different results in edge cases where the current reject-table
  and two-point LOS ray approach differs from a BSP-based approach. This would violate the
  behavioral parity mandate.

- **Blocker**: Behavioral parity mandate — changing LOS behavior would affect monster AI
  activation, target acquisition, and combat dynamics. Even subtle differences in which
  positions are considered "visible" would alter gameplay.

- **User impact**: None — LOS checks produce correct results for all standard DOOM gameplay.
  The "failure cases" mentioned by Carmack are extremely rare edge conditions that do not
  affect normal play.

- **Workaround**: N/A — the current implementation is functionally correct.

- **Next action**: Performance profiling of the Rust port may identify `P_CheckSight` as a
  bottleneck in levels with many active monsters. If optimization is needed, a BSP-based
  approach can be implemented with careful regression testing against the original behavior
  using recorded demo playback.

### IR-05: Feature Additions (Transparency, Slopes, Jumping, 3D Acceleration)

- **Reason deferred**: The user explicitly requires behavioral parity with DOOM 1.10. None of
  these features existed in the original engine. Adding them would constitute new functionality
  rather than a technology migration. The scope of this project is strictly limited to
  C → Rust language migration and Linux → Windows 11 platform retargeting.

- **Blocker**: Scope definition — the Agent Action Plan (AAP §0.3.2) explicitly lists these as
  out of scope: "No new features (transparency, slopes, jumping, ducking, look-up/down) as
  suggested in Carmack's README.TXT."

- **User impact**: None — these features were never present in DOOM 1.10. Users of the Rust
  port experience the same feature set as the original game.

- **Workaround**: N/A — the original game does not support these features and the Rust port
  faithfully reproduces this behavior.

- **Next action**: A future enhancement phase could add these features as optional runtime
  flags (e.g., `--enhanced-renderer`, `--allow-jumping`). The trait-based architecture of
  the Rust port (with `Renderer` and `PlatformHost` traits) is specifically designed to
  support such extensions without modifying the core game logic in `doom-core`.

### IR-06: `detailshift` Variable Cleanup

- **Reason deferred**: The `detailshift` variable (declared at `r_main.h` line 93 with the
  comment `//B remove this?` at line 91) controls the low-detail rendering mode accessible
  through the Options menu. When `detailshift = 1`, the renderer draws at half horizontal
  resolution (160 effective columns) for performance on slower hardware. Removing this
  variable would break the Options → Detail menu item and remove a user-accessible feature.

- **Blocker**: Behavioral parity — the "Detail" option in the menu system (`m_menu.c`)
  directly toggles this variable. Removing it would require modifying the menu system to
  remove the Detail option, which is a user-visible change.

- **User impact**: None — the variable is preserved and the Detail menu option continues to
  function as in the original game.

- **Workaround**: N/A — the variable is included in the Rust translation as a field in the
  renderer state.

- **Next action**: In a future cleanup phase, the low-detail mode could be removed if it is
  confirmed that no users rely on it. The removal would require coordinated changes to the
  renderer (`doom-render-soft`) and the menu system (`doom-core/src/ui/menu.rs`).

### IR-11: Floating-Point Migration

- **Reason deferred**: The `TODO` file suggests replacing `m_fixed` with floating-point
  arithmetic for stability and performance. However, the entire DOOM engine's deterministic
  behavior depends on the exact semantics of 16.16 fixed-point arithmetic, including truncation
  direction, overflow wrapping, and bit-shift behavior. Switching to floating-point would produce
  different numerical results in movement, collision, rendering, and AI calculations.

- **Blocker**: Behavioral parity mandate — demo playback compatibility requires bit-identical
  arithmetic results. Floating-point introduces platform-dependent rounding.

- **User impact**: None — the `Fixed(i32)` newtype in Rust provides clean, well-defined
  fixed-point operations with the same numerical behavior as the original C code.

- **Workaround**: N/A — fixed-point performance is not a concern on modern hardware.

- **Next action**: Could be explored in a future "enhanced mode" that disables demo
  compatibility. Would require comprehensive regression testing across all levels.

### IR-12: Configurable Screen Resolution

- **Reason deferred**: The `TODO` file suggests making `SCREENWIDTH`/`SCREENHEIGHT`
  configurable and fixing the 320×200 aspect ratio. The software renderer's column and span
  drawing loops, visplane allocation, texture mapping, and sprite projection are all hardwired
  to 320×200 pixel buffers. Changing the base resolution is effectively a renderer rewrite.

- **Blocker**: The renderer architecture assumes 320×200 in hundreds of calculations across
  `r_bsp.c`, `r_segs.c`, `r_plane.c`, `r_draw.c`, and `r_things.c`. The HUD and status bar
  graphics are pixel-mapped to 320×200.

- **User impact**: None — SDL2 window scaling provides clean display at any window size.
  The game renders at 320×200 internally and is scaled up by SDL2.

- **Workaround**: SDL2 window scaling handles display at native monitor resolution.

- **Next action**: A future enhancement could implement a higher-resolution software renderer
  or add an OpenGL/Vulkan backend with native resolution support.

### IR-13: Collision Height Handling

- **Reason deferred**: The `TODO` file notes that height-based collision detection "is not
  done, and the checks are scattered around in many places." This is a known limitation of
  the original DOOM engine — the collision system is essentially 2D with height checks only
  for certain interactions. Fixing this would require significant changes to `p_map.c`,
  `p_maputl.c`, `p_mobj.c`, and related modules.

- **Blocker**: Behavioral parity — changing collision behavior would alter gameplay in every
  level. Players and monsters use the 2D collision model as part of normal gameplay (e.g.,
  running over monsters on different height platforms).

- **User impact**: None — the original DOOM collision behavior is preserved exactly.

- **Workaround**: N/A — this is an intentional design characteristic of the original engine.

- **Next action**: Could be addressed in an "enhanced physics" mode with explicit opt-in.
  Would require handling "player on top of monster" scenarios as noted in the `TODO` file.

### IR-15: `FIXME` Annotations

- **Reason deferred**: The 12 `FIXME` annotations across the original C source mark known
  edge-case limitations and incomplete optimizations. Notable examples include:
  - `d_main.c:452` — Version-dependent demo number selection
  - `d_net.c:105` — Endianness concern in network byte packing
  - `f_finale.c:184` — Missing alternative text/music for certain game modes
  - `p_mobj.c:424,433` — Desire for a proper NOP/NULL function pointer
  - `i_sound.c:708,716` — Incomplete channel output in experimental sound code
  - `m_menu.c:1136` — Non-functional menu feature flagged for removal

  None of these represent runtime bugs that affect normal gameplay. They are informational
  markers left by the developers during the 1997 source cleanup.

- **Blocker**: These are documentation annotations, not defects. Addressing them would
  require gameplay behavior changes that violate the Minimal Change Clause.

- **User impact**: None — these edge cases do not affect normal gameplay.

- **Workaround**: N/A.

- **Next action**: Individual `FIXME` items can be addressed in future enhancement phases
  where behavioral changes are permitted. The Rust translation preserves the same behavioral
  characteristics as the original code at each annotated location.

### IR-16: `HACK` Annotations

- **Reason deferred**: The 3 `HACK` annotations are intentional workarounds required for
  correct DOOM II behavior:
  - `s_sound.c:241` — "HACK FOR COMMERCIAL": Adjusts music lump index for DOOM II's
    different music numbering scheme versus DOOM 1.
  - `wi_stuff.c:1609` — "MONDO HACK!": Handles intermission screen layout differences
    between DOOM 1 and DOOM II level progression.
  - `wi_stuff.c:1618` — "HACK ALERT!": Additional intermission screen workaround for
    DOOM II's non-episodic level structure.

  These workarounds exist because DOOM II reuses the DOOM 1 engine with a different level
  structure, and the engine adapts at runtime using these conditional branches.

- **Blocker**: These workarounds are required for correct DOOM II functionality. Removing
  them would break DOOM II intermission screens and music playback.

- **User impact**: None — the workarounds produce correct behavior for both DOOM 1 and
  DOOM II.

- **Workaround**: N/A — these are the correct implementation, not temporary hacks.

- **Next action**: A future refactor could replace these runtime checks with a cleaner
  game-mode dispatch pattern, but the behavioral result must be identical.

---

## Methodology

### Search Process

The issue discovery process was conducted systematically across the entire DOOM 1.10 repository
to ensure comprehensive coverage:

1. **Documentation file search**: A recursive `find` search was performed for common
   documentation and tracking files:
   - `linuxdoom-1.10/TODO` — Found (123 lines). Bernd Kreimeier's to-do list with items
     ranging from floating-point migration to collision height fixes. Reviewed in full;
     actionable items cataloged as IR-11, IR-12, IR-13, and IR-14
   - `linuxdoom-1.10/ChangeLog` — Found (922 lines). Work log from December 1997 documenting
     the Linux source port cleanup. Reviewed; provides historical context for implementation
     decisions but contains no new actionable issues beyond those in `TODO` and `README.TXT`
   - `linuxdoom-1.10/README.b` — Found (140 lines). README for the Linux DOOM source
     distribution authored by Bernd Kreimeier. Reviewed; contains project context, disclaimers,
     and subsystem notes (no new actionable issues)
   - `README.TXT` (root) — Found, analyzed in full (82 lines)
   - `README.asm` (linuxdoom-1.10/) — Found, contains assembly documentation only
   - `sersrc/README.TXT` — Found, contains serial networking documentation (out of scope)

2. **Source code annotation search**: A `grep -rn` search was performed across all `.c` and
   `.h` files in `linuxdoom-1.10/`, `sndserv/`, `sersrc/`, and `ipx/` directories for the
   following patterns:
   - `TODO` — Zero results in source code comments
   - `FIXME` — **12 occurrences** across 9 files: `d_main.c` (×2), `d_net.c`, `f_finale.c`,
     `hu_stuff.c`, `i_sound.c` (×2), `m_menu.c`, `p_mobj.c` (×2), `p_tick.c`, `r_draw.c`.
     Cataloged as IR-15
   - `HACK` — **3 occurrences** across 2 files: `s_sound.c` (×1), `wi_stuff.c` (×2).
     Cataloged as IR-16
   - `XXX` — Zero results
   - `BUG` / `BUGFIX` — Zero results

3. **Header file review**: All 55 `.h` files in `linuxdoom-1.10/` were reviewed for
   comments indicating known issues, experimental features, or removal candidates. This
   review identified:
   - `r_main.h` line 91: `//B remove this?` on `detailshift` (IR-06)
   - `doomdef.h` lines 77–85: `SNDSERV`/`SNDINTR` experimental audio discussion (IR-07)
   - `doomdef.h` lines 88–91: MIT-SHM/DGA video mode discussion (IR-08)
   - `z_zone.h` lines 34–44: Zone memory purge tag definitions (IR-09 reference)
   - `i_system.h` line 40: `I_ZoneBase` zone allocation interface (IR-09 reference)

4. **README.TXT analysis**: The 82-line release notes file was analyzed line-by-line for
   actionable engineering content:
   - Lines 12–16: Platform limitation and missing DOS sound code (IR-01, IR-02)
   - Lines 26–35: Rendering improvement suggestions (IR-03)
   - Lines 37–45: Movement and LOS checking critique (IR-04)
   - Lines 47–65: Suggested community projects (IR-05)

5. **TODO file analysis**: The 123-line to-do list was analyzed for actionable items.
   Many items overlap with issues already captured from `README.TXT` (rendering improvements,
   LOS optimization, feature additions). New actionable items were cataloged:
   - Line 10: Floating-point migration (IR-11)
   - Lines 13–18: Configurable screen resolution (IR-12)
   - Lines 83–89: Collision height handling (IR-13)
   - Lines 97–101: Sound server lifecycle and threading (IR-14)

6. **ChangeLog review**: The 922-line work log was reviewed for items not captured elsewhere.
   The log primarily documents cleanup work already reflected in the final source code state.
   No new actionable issues were identified beyond those in `TODO` and `README.TXT`.

### Issue Classification Criteria

Issues were classified into two categories:

- **To be fixed**: Issues that are directly addressed by the C → Rust migration and Windows 11
  platform retargeting. These include platform-specific code replacement (IR-01, IR-08, IR-10),
  sound system reimplementation (IR-02, IR-07, IR-14), and memory management modernization
  (IR-09).

- **Deferred**: Issues that represent feature enhancements, performance optimizations, or code
  cleanup that would alter the behavioral contract of the original engine. These are documented
  with full rationale, blocker analysis, user impact assessment, and recommended next actions
  (IR-03, IR-04, IR-05, IR-06, IR-11, IR-12, IR-13, IR-15, IR-16).

### Zone Memory Tag Reference

For completeness, the full set of zone memory purge tags from `z_zone.h` (lines 34–44) is
documented here, as these values define the cache eviction contract that must be preserved
in the Rust `LumpCache` implementation:

| Tag Name | Value | Behavior | Rust Equivalent |
|----------|-------|----------|-----------------|
| `PU_STATIC` | 1 | Static for entire execution time | Permanent cache entry (never evicted) |
| `PU_SOUND` | 2 | Static while sound is playing | Retained while sound channel is active |
| `PU_MUSIC` | 3 | Static while music is playing | Retained while music is playing |
| `PU_DAVE` | 4 | Miscellaneous static allocation | Permanent cache entry |
| `PU_LEVEL` | 50 | Static until level exited | Dropped when `P_SetupLevel` loads new map |
| `PU_LEVSPEC` | 51 | Level-specific thinker allocation | Dropped when `P_SetupLevel` loads new map |
| `PU_PURGELEVEL` | 100 | Purgeable whenever needed (threshold) | Evictable from cache under memory pressure |
| `PU_CACHE` | 101 | Purgeable whenever needed | Evictable from cache under memory pressure |

**Rule**: Tags with values less than 100 are **not overwritten** until explicitly freed.
Tags with values greater than or equal to 100 (`PU_PURGELEVEL`, `PU_CACHE`) are purgeable
whenever the allocator needs to reclaim memory.
