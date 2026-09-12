//! Board sharing, the daemon's half (T-215 v1): the daemon is the Teams
//! client. It holds the device keys and the board keys, seals what leaves
//! and opens what arrives, and applies every remote change through the same
//! single writer everything else goes through.
//!
//! - [`device`]: the per-user identity file.
//! - [`state`]: the per-board sharing state file.
//! - [`project`]: the board as shared objects, and the digests that say
//!   which of them changed.
//! - [`sync`]: the one thread that talks to the relay. It decides nothing.
//!
//! The decisions live in `server/teamglue.rs`, on the writer.
pub mod device;
pub mod project;
pub mod state;
pub mod sync;
