//! The `mods` e2e binary: the Claude mod road.
//!
//! Each file beside this one is a module of it, so the area links once (T-1197).
//! A file without its `mod` line here is never compiled:
//! `guard::every_test_file_is_a_module` fails on one.

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/guard.rs"]
mod guard;

mod mod_auto_e2e;
mod mod_bridge_e2e;
mod mod_cost_e2e;
mod mod_plugin;
mod mod_turns_e2e;
