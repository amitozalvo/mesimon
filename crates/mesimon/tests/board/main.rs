//! The `board` e2e binary: the board itself: columns, restart, terminals, shells, setup and snooze.
//!
//! Each file beside this one is a module of it, so the area links once (T-1197).
//! A file without its `mod` line here is never compiled:
//! `guard::every_test_file_is_a_module` fails on one.

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/guard.rs"]
mod guard;

mod column_ask_e2e;
mod column_description_e2e;
mod column_e2e;
mod compose_e2e;
mod exit_parks_e2e;
mod focus_quiet_e2e;
mod inactivity_park_e2e;
mod m1_e2e;
mod m3_e2e;
mod pane_tail_e2e;
mod pickup_e2e;
mod reorder_e2e;
mod restart_e2e;
mod restart_skew_e2e;
mod setup_e2e;
mod shell_env_e2e;
mod shell_line_e2e;
mod shutdown_flush_e2e;
mod sleep_offer_e2e;
mod slow_tmux_e2e;
mod snooze_e2e;
mod terminal_adopt_e2e;
mod terminal_e2e;
mod wake_straggler_e2e;
