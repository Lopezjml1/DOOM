# Building DOOM Rust on Windows 11

This guide covers everything needed to build and run the DOOM Rust port on a
fresh Windows 11 installation. It walks through every prerequisite, the build
process itself, how to launch the game, and how to troubleshoot common issues.
By the end of this guide you will have a playable DOOM session running natively
on Windows 11.

> **Important:** You must own a legal copy of DOOM or DOOM II to play. The
> game's IWAD data files are **not** included in this repository. They are
> available on [Steam](https://store.steampowered.com/app/2280/Ultimate_Doom/)
> and [GOG](https://www.gog.com/game/doom_ii_final_doom).

---

## Table of Contents

1. [System Requirements](#system-requirements)
2. [Prerequisites Installation](#prerequisites-installation)
   - [Step 1 — Install the Rust Toolchain](#step-1--install-the-rust-toolchain)
   - [Step 2 — Install Visual Studio Build Tools 2022](#step-2--install-visual-studio-build-tools-2022)
   - [Step 3 — Obtain an IWAD File](#step-3--obtain-an-iwad-file)
3. [Building the Project](#building-the-project)
   - [Clone the Repository](#clone-the-repository)
   - [Build in Release Mode](#build-in-release-mode)
   - [Verify the Build](#verify-the-build)
4. [Running DOOM](#running-doom)
   - [Basic Launch](#basic-launch)
   - [Steam DOOM II Example](#steam-doom-ii-example)
   - [Advanced Options](#advanced-options)
   - [Expected Behavior](#expected-behavior)
5. [Controls Reference](#controls-reference)
6. [Development Workflow](#development-workflow)
7. [Troubleshooting](#troubleshooting)
8. [Project Structure Overview](#project-structure-overview)
9. [Original Build System Comparison](#original-build-system-comparison)

---

## System Requirements

| Requirement | Details |
|-------------|---------|
| **Operating System** | Windows 11 (22H2 or later) |
| **Architecture** | x86\_64 (64-bit) |
| **Hardware** | Any machine capable of running Windows 11. DOOM's software renderer is trivially performant on modern hardware. |
| **Disk Space** | ~2 GB for the Rust toolchain + ~500 MB for Visual Studio Build Tools + ~100 MB for the project and compiled artifacts |
| **IWAD File** | A legally-owned DOOM IWAD file: `DOOM.WAD`, `DOOM2.WAD`, `TNT.WAD`, or `PLUTONIA.WAD`. These are **not** included in the repository — you must own a copy of DOOM, available on Steam, GOG, or other retailers. |

---

## Prerequisites Installation

Three things must be installed before you can build the project. Follow each
step in order.

### Step 1 — Install the Rust Toolchain

1. Download `rustup-init.exe` from <https://rustup.rs>.
2. Run the installer and choose the **default** option when prompted. This
   installs the `stable-x86_64-pc-windows-msvc` toolchain.
3. After installation completes, open a **new** terminal (Command Prompt or
   PowerShell) and verify:

   ```
   rustc --version
   cargo --version
   ```

   Both commands should print a version number (e.g. `rustc 1.XX.0`).

> **Note:** The repository includes a `rust-toolchain.toml` file that pins the
> stable channel and ensures the `rustfmt` and `clippy` components are
> available. Rustup reads this file automatically, so the correct toolchain
> configuration is applied when you build the project.

### Step 2 — Install Visual Studio Build Tools 2022

The SDL2 library is compiled from C source during the Cargo build (via the
`bundled` feature). This requires a C/C++ compiler on your system.

1. Download the installer from
   <https://visualstudio.microsoft.com/visual-cpp-build-tools/>.
2. Run the installer and select the **"Desktop development with C++"**
   workload.
3. Ensure the following components are checked (they should be selected by
   default with the workload):
   - **MSVC v143 — VS 2022 C++ x64/x86 build tools**
   - **Windows 11 SDK**
4. Click **Install** and wait for the download and installation to finish.
5. Restart your terminal after installation.

> **Note:** You do **not** need the full Visual Studio IDE — just the Build
> Tools. The installer is a separate, smaller download.

### Step 3 — Obtain an IWAD File

DOOM requires an IWAD (Internal WAD) file that contains the game's levels,
textures, sounds, and music. You must provide your own legally-owned copy.

**Common Steam installation paths:**

| Game | Typical IWAD Location |
|------|-----------------------|
| Ultimate DOOM | `C:\Program Files (x86)\Steam\steamapps\common\Ultimate Doom\base\DOOM.WAD` |
| DOOM II | `C:\Program Files (x86)\Steam\steamapps\common\Doom 2\base\DOOM2.WAD` |
| TNT: Evilution | `C:\Program Files (x86)\Steam\steamapps\common\Final Doom\base\TNT.WAD` |
| The Plutonia Experiment | `C:\Program Files (x86)\Steam\steamapps\common\Final Doom\base\PLUTONIA.WAD` |

If you purchased DOOM from GOG, check your GOG installation directory for the
WAD file.

**Note the full path to your IWAD file** — you will need it when running the
game.

---

## Building the Project

### Clone the Repository

```
git clone <repository-url>
cd doom-rust
```

### Build in Release Mode

```
cargo build --release
```

This command performs the following steps automatically:

1. Downloads and compiles all Rust crate dependencies from crates.io.
2. Compiles SDL2 from C source via the `bundled` feature — this may take
   **several minutes** on the first build.
3. Compiles all five workspace crates in dependency order:
   - `doom-wad` — WAD/IWAD file parsing library
   - `doom-core` — Deterministic game logic
   - `doom-render-soft` — BSP-based software renderer
   - `doom-platform-win` — Windows 11 SDL2 platform backend
   - `doom-bin` — Executable entry point
4. Links the final Windows executable.

The compiled binary is located at:

```
target\release\doom-rust.exe
```

### Verify the Build

Run the executable with the `--help` flag to confirm it was built correctly:

```
target\release\doom-rust.exe --help
```

You should see CLI usage information listing the `--iwad`, `--pwad`, `--warp`,
`--skill`, and `--verbose` options.

---

## Running DOOM

### Basic Launch

Using `cargo run`:

```
cargo run --release -- --iwad "C:\path\to\your\DOOM2.WAD"
```

Or run the executable directly:

```
target\release\doom-rust.exe --iwad "C:\path\to\your\DOOM2.WAD"
```

### Steam DOOM II Example

```
cargo run --release -- --iwad "C:\Program Files (x86)\Steam\steamapps\common\Doom 2\base\DOOM2.WAD"
```

### Advanced Options

| Option | Description | Example |
|--------|-------------|---------|
| `--iwad <PATH>` | **(Required)** Path to the IWAD file | `--iwad DOOM2.WAD` |
| `--pwad <PATH>` | Load a PWAD patch file (can be specified multiple times) | `--pwad mypwad.wad` |
| `--warp <LEVEL>` | Warp directly to a level. DOOM 1: `<episode> <map>`. DOOM 2: `<map>` | `--warp 1 1` (E1M1) |
| `--skill <1-5>` | Set the difficulty level (default: 3) | `--skill 4` |
| `--verbose` / `-v` | Enable verbose debug logging | `--verbose` |

**Difficulty levels:**

| Value | Name |
|-------|------|
| 1 | I'm Too Young to Die |
| 2 | Hey, Not Too Rough |
| 3 | Hurt Me Plenty *(default)* |
| 4 | Ultra-Violence |
| 5 | Nightmare! |

You can also control logging granularity with the `RUST_LOG` environment
variable (e.g., `set RUST_LOG=debug` in Command Prompt, or
`$env:RUST_LOG="debug"` in PowerShell).

### Expected Behavior

When launched with a valid IWAD:

- A window opens displaying the **DOOM title screen**.
- **Keyboard input** is responsive: arrow keys for movement, Ctrl to fire,
  Space to open doors, Enter to select menu items, Escape for the menu.
- **Sound effects** play during gameplay (weapon fire, door open, enemy alert).
- **Music** plays on level start.
- The game maintains **stable frame pacing at 35 tics/second**.

---

## Controls Reference

| Key | Action |
|-----|--------|
| Arrow Up | Move forward |
| Arrow Down | Move backward |
| Arrow Left | Turn left |
| Arrow Right | Turn right |
| Ctrl | Fire weapon |
| Space | Use / Open doors |
| Shift | Run (hold) |
| Tab | Toggle automap |
| Enter | Select menu item |
| Escape | Open / close menu |
| F2 | Save game |
| F3 | Load game |
| F5 | Change detail level |
| F6 | Quick save |
| F9 | Quick load |
| 1–7 | Select weapon |

---

## Development Workflow

If you are contributing to or modifying the project, the following commands are
useful for day-to-day development.

### Running Tests

```
cargo test --workspace
```

Runs all unit and integration tests across every crate in the workspace.

### Running Lints

```
cargo clippy --all-targets -- -D warnings
```

Runs Clippy with warnings treated as errors, matching the CI configuration.

### Checking Formatting

```
cargo fmt --check
```

Verifies that all source files conform to the project's `rustfmt.toml` rules
without modifying them. To auto-format, omit `--check`:

```
cargo fmt
```

### Debug Build

Debug builds compile faster but run slower. They also enable additional runtime
checks such as integer overflow detection and debug assertions.

```
cargo build
cargo run -- --iwad "C:\path\to\your\DOOM2.WAD"
```

> **Tip:** Always use `cargo build --release` for actual gameplay. Debug builds
> may not maintain the required 35 tics/second frame rate during complex scenes.

### CI Validation Checklist

The continuous integration pipeline runs the following checks on every commit.
Make sure they all pass locally before pushing:

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo build --release
```

---

## Troubleshooting

### "IWAD not found" error

- Verify that your WAD file exists at the specified path.
- If the path contains spaces, make sure it is wrapped in double quotes.
- Double-check common Steam installation locations:
  - `C:\Program Files (x86)\Steam\steamapps\common\Ultimate Doom\base\DOOM.WAD`
  - `C:\Program Files (x86)\Steam\steamapps\common\Doom 2\base\DOOM2.WAD`

### Build fails with "link.exe not found"

- Install **Visual Studio Build Tools 2022** with the **"Desktop development
  with C++"** workload (see [Step 2](#step-2--install-visual-studio-build-tools-2022)).
- **Restart** your terminal or Command Prompt after installation so the new
  PATH entries take effect.
- Verify that `cl.exe` is available by opening a **Developer Command Prompt for
  VS 2022** and running `cl`.

### Build fails during SDL2 compilation

- Ensure the Visual Studio Build Tools **C++ workload** is installed.
- Ensure the **Windows 11 SDK** is included (it is selected by default with the
  C++ workload).
- Try a clean rebuild:

  ```
  cargo clean
  cargo build --release
  ```

- If the error mentions CMake, ensure CMake is installed (it is included with
  the C++ workload, but can also be installed separately from
  <https://cmake.org/download/>).

### No audio output

- Verify that your default audio device is working in **Windows Sound
  Settings** (right-click the speaker icon in the taskbar → Sound settings).
- Check the console output for SDL2 audio initialization messages.
- Try forcing the WASAPI audio driver by setting the environment variable
  before launching:

  ```
  set SDL_AUDIODRIVER=wasapi
  target\release\doom-rust.exe --iwad "path\to\IWAD"
  ```

### Window does not appear

- Ensure your **graphics drivers** are up to date.
- SDL2 uses DirectX by default on Windows. If you encounter issues, try
  overriding the video driver:

  ```
  set SDL_VIDEO_DRIVER=windows
  target\release\doom-rust.exe --iwad "path\to\IWAD"
  ```

### Game is sluggish or laggy

- Make sure you built in **release mode**:

  ```
  cargo build --release
  ```

  Debug builds (`cargo build` without `--release`) are significantly slower
  because compiler optimizations are minimal and extra runtime checks are
  enabled.

### Antivirus false positive

- Some antivirus programs may flag newly compiled executables. If
  `doom-rust.exe` is quarantined, add the `target\release\` directory to your
  antivirus exclusion list.

---

## Project Structure Overview

The Rust workspace is organized into five crates with explicit dependency
boundaries. For full architectural details, see
[docs/ARCHITECTURE.md](ARCHITECTURE.md).

| Crate | Purpose |
|-------|---------|
| `doom-wad/` | WAD and IWAD file parsing library. Reads the binary WAD format, builds the lump directory, and provides cached lump access. |
| `doom-core/` | Deterministic game logic — the heart of the engine. Contains type definitions, the game loop, all gameplay systems (physics, AI, map specials), UI subsystems, and platform-abstraction trait definitions. |
| `doom-render-soft/` | BSP-based software renderer. Traverses the BSP tree, renders walls, floors, ceilings, and sprites into a 320×200 palettized framebuffer. |
| `doom-platform-win/` | Windows 11 platform backend built on SDL2. Provides window management, input handling, audio mixing, high-resolution timing, and filesystem utilities. |
| `doom-bin/` | Thin executable entry point. Parses CLI arguments via `clap`, initializes all subsystems, and enters the game loop. |

```
doom-rust/
├── Cargo.toml               (workspace manifest)
├── rust-toolchain.toml       (toolchain pinning)
├── doom-wad/                 (WAD parsing)
├── doom-core/                (game logic)
├── doom-render-soft/         (software renderer)
├── doom-platform-win/        (SDL2 platform backend)
├── doom-bin/                 (executable)
├── docs/
│   ├── BUILDING.md           (this file)
│   ├── ARCHITECTURE.md       (architecture decisions)
│   └── ISSUES.md             (issue resolution matrix)
├── README.md                 (project overview)
├── README.TXT                (original Carmack release notes)
└── LICENSE.TXT               (GPL v2)
```

---

## Original Build System Comparison

The Rust Cargo workspace replaces the original GNU Make build system entirely.

| Aspect | Original (Linux) | Rust Port (Windows 11) |
|--------|-------------------|------------------------|
| **Compiler** | `gcc` | `rustc` (via `cargo`) |
| **Build command** | `make` | `cargo build --release` |
| **Compiler flags** | `-g -Wall -DNORMALUNIX -DLINUX` | Configured in `Cargo.toml` profiles and `clippy.toml` |
| **Libraries** | `-lXext -lX11 -lnsl -lm` (manual flags) | Declared in `Cargo.toml`; resolved automatically by Cargo |
| **Platform deps** | X11, Xext, OSS `/dev/dsp` | SDL2 (compiled from source via `bundled` feature) |
| **Output** | `linux/linuxxdoom` (ELF binary) | `target/release/doom-rust.exe` (Windows PE binary) |
| **Source files** | ~55 `.c` files + ~55 `.h` files in a flat directory | Modular Rust crates with enforced dependency boundaries |
| **Build time** | Seconds (small C project) | Minutes on first build (SDL2 C compilation); seconds for incremental builds |

The original `linuxdoom-1.10/Makefile` is preserved in the repository as a
historical reference and is not used by the Rust build.
