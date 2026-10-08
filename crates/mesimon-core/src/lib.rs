//! Board model, `reconcile()`, and `authorize()` — pure logic, no I/O.
//!
//! D24: `reconcile(tickets, discovery_snapshot) -> links` is written and unit-tested
//! before any daemon code. D32c: every mutation carries a `principal` and passes
//! through `authorize()`, which returns `Allow` unconditionally in v0.1 but is called
//! on every mutation path anyway.

pub mod adopt;
pub mod attachment;
pub mod attention;
pub mod authorize;
pub mod automove;
pub mod background;
pub mod board;
pub mod board_source;
pub mod brief;
pub mod claudemd;
pub mod clock;
pub mod command;
pub mod content;
pub mod cost;
pub mod crown;
pub mod diff;
pub mod exe;
pub mod fracindex;
pub mod keymap;
pub mod links;
pub mod mcp;
pub mod mesophon;
pub mod notify;
pub mod prefs;
pub mod principal;
pub mod prompts;
pub mod quiet;
pub mod reconcile;
pub mod relnotes;
pub mod road;
pub mod search;
pub mod shellenv;
pub mod snooze;
pub mod summary;
pub mod team;
pub mod text;
pub mod tier;
pub mod train;
pub mod usage;
pub mod verdict;
pub mod workspace;

pub use authorize::{authorize, Action, Decision, Resource};
pub use principal::Principal;
pub use verdict::{RuleId, Verdict};
