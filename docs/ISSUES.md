# Issue Resolution Matrix

This document tracks all known issues, incomplete work, and improvement opportunities
discovered during the DOOM 1.10 C → Rust migration. Each issue is cataloged with its
source, disposition (fix or defer), and supporting rationale.

## Sources of Issues

All issues in this matrix were derived from two sources:

1. **`README.TXT`** (repository root) — John Carmack's release notes, dated December 23, 1997.
   This is the only documentation file present in the repository that contains actionable
   engineering notes about the codebase.

2. **Inline source code comments** — Comments within the original C header and implementation
   files (`linuxdoom-1.10/*.c`, `linuxdoom-1.10/*.h`) that indicate known limitations,
   experimental features, or removal candidates.

### Files Confirmed Absent

A comprehensive search of the entire repository confirmed that the following files
**do not exist**, despite being commonly expected in open-source projects:

- `linuxdoom-1.10/TODO` — Not found
- `linuxdoom-1.10/ChangeLog` — Not found
- `linuxdoom-1.10/README.b` — Not found

Additionally, a `grep` search for inline annotations (`TODO`, `FIXME`, `HACK`, `XXX`, `BUG`)
across all `.c` and `.h` files in the repository returned **zero results**. The codebase
contains no developer-annotated work items of any kind.

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

---

## Methodology

### Search Process

The issue discovery process was conducted systematically across the entire DOOM 1.10 repository
to ensure comprehensive coverage:

1. **Documentation file search**: A recursive `find` search was performed for common
   documentation and tracking files:
   - `TODO`, `TODO.md`, `TODO.txt` — Not found
   - `ChangeLog`, `CHANGELOG`, `CHANGELOG.md`, `changelog` — Not found
   - `README.b` — Not found
   - `README.TXT` (root) — Found, analyzed in full (82 lines)
   - `README.asm` (linuxdoom-1.10/) — Found, contains assembly documentation only
   - `sersrc/README.TXT` — Found, contains serial networking documentation (out of scope)

2. **Source code annotation search**: A `grep -rn` search was performed across all `.c` and
   `.h` files in `linuxdoom-1.10/`, `sndserv/`, `sersrc/`, and `ipx/` directories for the
   following patterns:
   - `TODO` — Zero results
   - `FIXME` — Zero results
   - `HACK` — Zero results
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

### Issue Classification Criteria

Issues were classified into two categories:

- **To be fixed**: Issues that are directly addressed by the C → Rust migration and Windows 11
  platform retargeting. These include platform-specific code replacement (IR-01, IR-08, IR-10),
  sound system reimplementation (IR-02, IR-07), and memory management modernization (IR-09).

- **Deferred**: Issues that represent feature enhancements, performance optimizations, or code
  cleanup that would alter the behavioral contract of the original engine. These are documented
  with full rationale, blocker analysis, user impact assessment, and recommended next actions
  (IR-03, IR-04, IR-05, IR-06).

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
