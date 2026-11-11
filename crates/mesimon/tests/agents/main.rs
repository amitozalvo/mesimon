//! The `agents` e2e binary: launching and driving an agent session (providers, plans,
//! prompts, the ask queue, interrupts, hooks, trust, spawn, compact and tiers).
//!
//! Each file beside this one is a module of it, so the area links once (T-1197).
//! A file without its `mod` line here is never compiled:
//! `guard::every_test_file_is_a_module` fails on one.

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/guard.rs"]
mod guard;

mod approve_bridge;
mod ask_after_crash_e2e;
mod ask_after_park_e2e;
mod ask_queue_e2e;
mod async_hook_e2e;
mod bash_mode_e2e;
mod claude_mode_e2e;
mod codex_startup_recovery_e2e;
mod compact_e2e;
mod exec_e2e;
mod execution_policy_e2e;
mod external_provider_e2e;
mod hook_e2e;
mod interrupt_e2e;
mod interrupt_status_e2e;
mod interrupt_tail_e2e;
mod plan_accept_e2e;
mod plan_mode_e2e;
mod prompt_e2e;
mod provider_e2e;
mod spawn_ready_e2e;
mod state_lab_e2e;
mod status_line_e2e;
mod tier_e2e;
mod trust_accept_e2e;
mod usage_e2e;
