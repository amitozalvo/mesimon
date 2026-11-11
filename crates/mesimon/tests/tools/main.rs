//! The `tools` e2e binary: what an agent calls or reads: MCP, notes, tags, attachments, the brief and raise_hand.
//!
//! Each file beside this one is a module of it, so the area links once (T-1197).
//! A file without its `mod` line here is never compiled:
//! `guard::every_test_file_is_a_module` fails on one.

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/guard.rs"]
mod guard;

mod agent_tools_e2e;
mod attachments_e2e;
mod boardlink_e2e;
mod brief_e2e;
mod brief_offer_e2e;
mod channel_cli;
mod duplicate_e2e;
mod mcp_e2e;
mod notes_e2e;
mod raise_hand_e2e;
mod tags_e2e;
mod ticket_create_e2e;
mod told_hand_e2e;
