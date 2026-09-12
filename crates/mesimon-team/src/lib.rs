//! Board sharing (T-215, v1 plan of 2026-09-12).
//!
//! The Apache half of Teams. The daemon is the Teams client: it holds the
//! device keys and every board key, seals records before they leave the
//! machine and opens the ones that arrive. The relay under `team/relay` stores
//! ciphertext and membership and never sees plaintext.
//!
//! - [`crypto`]: device keys, board keys, key wrapping and record sealing.
//! - [`invite`]: the one-time invite code a person types to join a board.
//! - [`wire`]: the relay protocol, one newline-delimited JSON frame per
//!   request, plus the single framer both ends use.
//! - [`relay`]: the TLS client for that protocol.
pub mod crypto;
pub mod hex;
pub mod invite;
pub mod relay;
pub mod wire;
