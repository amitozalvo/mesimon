//! The `crown` e2e binary: the crown: its tools, wakes, holds, reads and rules.
//!
//! Each file beside this one is a module of it, so the area links once (T-1197).
//! A file without its `mod` line here is never compiled:
//! `guard::every_test_file_is_a_module` fails on one.

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/guard.rs"]
mod guard;

mod crown_checkout_e2e;
mod crown_e2e;
mod crown_hold_e2e;
mod crown_reads_e2e;
mod crown_rules_e2e;
