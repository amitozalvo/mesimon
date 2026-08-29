//! Board model, `reconcile()`, and `authorize()` — pure logic, no I/O.
//!
//! D24: `reconcile(tickets, discovery_snapshot) -> links` is written and unit-tested
//! before any daemon code. D32c: every mutation carries a `principal` and passes
//! through `authorize()`, which returns `Allow` unconditionally in v0.1 but is called
//! on every mutation path anyway.

pub mod authorize;
pub mod principal;

pub use authorize::{authorize, Action, Decision, Resource};
pub use principal::Principal;
