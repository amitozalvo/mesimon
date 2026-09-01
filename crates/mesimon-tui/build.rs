//! One stamp: is this binary a RELEASE, or somebody's build?
//!
//! The release checker (`src/release.rs`) reaches the network and can replace
//! the binary at our own path, so "is this a dev build" cannot be a heuristic
//! — a wrong answer would point a download at somebody's `target/` directory.
//! It is therefore not inferred at all. `ci/release.sh` sets `MESIMON_RELEASE`
//! for the one build it publishes, and nothing else does: a `cargo run`, a
//! plain `cargo build --release`, and every test binary all come out `dev`,
//! and the checker is inert in all of them.

fn main() {
    println!("cargo:rerun-if-env-changed=MESIMON_RELEASE");
    let channel = if std::env::var_os("MESIMON_RELEASE").is_some() { "release" } else { "dev" };
    println!("cargo:rustc-env=MESIMON_CHANNEL={channel}");
}
