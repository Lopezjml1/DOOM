//! Game module — high-level game flow, loop control, and string constants.
//!
//! Translated from linuxdoom-1.10/d_main.c, d_net.c, g_game.c, dstrings.c/h,
//! d_englsh.h, d_french.h

// Submodules — only declare those whose source files exist on disk.
// Other game submodules (game_main, game_loop, game_net) will be
// declared here once their source files are created by their respective agents.
pub mod game_ctrl;
pub mod strings;
