---
name: release
description: Cutting a mesimon release — the order (bump, CHANGELOG, test run, tag, ship.sh), the unstable and stable channels, mesimon.dev and its install line, the Homebrew tap, the build-cache prune and the Linux musl cross-build. Load before bumping a version or running ci/release.sh, ci/site.sh, ci/build-linux.sh, ci/prune-deps.py or ci/homebrew-formula.sh.
---

# Releasing mesimon

From the root `CLAUDE.md` (Commands). Production deploys — `deploy/ship.sh`, the tag, the push, `ci/release.sh` — are the author's.

**Release order**: bump + CHANGELOG → commit → `MESIMON_TMUX_BIN=$PWD/vendor/tmux/tmux python3
-B ci/test-run.py` → tag → push → `ci/release.sh`. Never verify before the bump — the bump
relinks every crate and the run is thrown away. From the tag on, the author runs **one
command, the relay repository's `deploy/ship.sh all`** (`--dry-run` runs every check): the
relay, then the tag, push and `ci/release.sh`, which publishes the site. `ci/release.sh` refuses
while the live relay lacks a change to what it serves (`ci/check-relay.sh` reads its `/version`). The runner stamps `target/suite-passed.json`
(HEAD sha + the tmux that drove it, by path and hash) and `release.sh` skips its own suite when
the stamp matches; `MESIMON_RELEASE_RETEST=1` overrides. Releases are cut locally, not on a
runner. **Both Docker steps are paused by the author** until they say Windows/WSL2 is
operational — the script prints `SKIPPED`, `MESIMON_RELEASE_DOCKER=1` runs them, and you should
not open Docker Desktop for a release.
**Release channels (T-946)**: a version `<last stable>.unstable.N` (from 0.1.0 final,
`0.2.0-unstable.N`) is an unstable release, tagged `unstable-<version>` in the same repository,
which no picker since T-877 takes (beta.12's updater on, and the served `install.sh`). The order is the same, the channel is the version's, and
a promotion is one bump on the soaked commit. The first stable release carrying T-946 ships before
the first unstable tag. A stable version that may overtake the train keeps the train's schema
readers, or does not ship. The relay and Remote Control are single-channel: the live relay serves
both channels' cores, and the relay repository keeps a stable-client and a candidate-client
fixture passing before a shared deployment that changes their contract.

**mesimon.dev is GitHub Pages on `amitozalvo/mesimon-releases`' main branch** — the page is
`site/`, and the `install.sh` it serves is the one `release.sh` syncs there after each release,
never main's. The install line is in five files and `ci/site.sh` fails when one disagrees.

**The Homebrew tap** (`brew install amitozalvo/tap/mesimon`) is one formula, generated from
`ci/homebrew/mesimon.rb` by `ci/homebrew-formula.sh` and pushed to `amitozalvo/homebrew-tap` by
`release.sh` after `gh release create`. The template's header says how to check a change. The
updater never replaces a binary brew installed (`release.rs::by_homebrew`).

`ci/prune-deps.py` deletes split-debuginfo objects no current binary references (the runner
calls it after every run, on the build dir that run used, and it skips a `deps/` unchanged since
its last prune). It is what keeps the first-exec Gatekeeper hold down: syspolicyd's
scan is slow in proportion to the *directory*, and one edit to a core crate mints ~6,500 `.o`
files that cargo never collects. Past 50k entries it advises `cargo clean`.

**The Linux cross-build needs a musl C toolchain** — `x86_64-linux-musl-gcc` and
`aarch64-linux-musl-gcc` on PATH (`brew trust messense/macos-cross-toolchains`, then `brew
install messense/macos-cross-toolchains/<target>`), because `ring` came in with Teams (T-332)
and the daemon links it unconditionally. `ci/build-linux.sh` dies with the brew line when one is
missing. Apple clang cannot stand in (no musl sysroot).
