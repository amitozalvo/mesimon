---
name: release
description: Cutting a mesimon release — the order (bump, CHANGELOG, test run, tag, ship.sh), the unstable and stable channels, mesimon.dev and its install line, the Homebrew tap, the build-cache prune and the Linux musl cross-build. Load before bumping a version or running ci/release.sh, ci/site.sh, ci/build-linux.sh, ci/prune-deps.py or ci/homebrew-formula.sh.
---

# Releasing mesimon

See `AGENTS.md` for the shared development contract. When the person asks for a release,
the release agent runs it through production.

**Release order**: bump + CHANGELOG (`cargo update --workspace --offline` for the lock) → commit →
`ci/release-prepare.sh` → merge any relay changes → commit the relay lock update →
dry run → ship. Never verify before the bump — the bump relinks every crate and the run is
thrown away. `ci/release-prepare.sh` (T-1101) is the one command after the commit: it refuses a
dirty tree or a CHANGELOG whose top heading is not the version's, runs the stamped gate
(`ci/test-run.py --both-roads --exclusive`, with hooks and mod running side by side), and once the tests are running
warms beside them, niced, what `release.sh` and the ship will run again on the same commit: the
gate's clippy (a red one stops the gate: the commit will change), the drift check, the site, and
in the main checkout the macOS and two Linux release builds with `MESIMON_RELEASE=1`, so the
ship script finds them built. In a worktree the release builds are skipped (cargo keys
artifacts by source path) and the stamp is the deliverable.

The gate waits for other bounded runs and holds the exclusive lock through both roads and any
reruns. Both nextest passes use `--no-fail-fast`. After both finish, the runner reruns each
named failure once, alone on its original road at the same clean SHA. Each result names the
road, binary and test as `passed-alone` or `failed-alone`. A rerun that could not execute is
`inconclusive`. Any red in the full runs leaves the gate red and writes no stamp. Bring the
named results to the person for any exception to the two-green-roads requirement.

Use the default of six jobs per road. T-1101 measured 1,071 s on hooks and 1,038 s on mod at
six. The observed four-job and six-job gates used different candidates. Record the job count,
road summaries and elapsed gate time at the next real release before changing the default.
Logs land in `target/release-prepare/`.

Resolve `../mesimon-relay` relative to the main mesimon checkout. Use the resolved absolute
path wherever `<relay>` appears below. If the release changes relay code or tests, confirm
the relay checkout is clean and on `main`. The release agent fast-forwards it with
`git -C "<relay>" merge --ff-only <relay-branch>`, using the actual `msmn/…` branch from
the relay worktree.

Invoke the ship script by absolute path. Run `"<relay>/deploy/ship.sh" all --dry-run`.
When it passes, run `"<relay>/deploy/ship.sh" all` directly.
It deploys the relay, tags and pushes the release, then runs `ci/release.sh` to
publish the site. Raise a hand only for an unexpected result or a permission denial.
On denial, name the exact missing allow rule for the blocked command.
`ci/release.sh` refuses while the live relay
lacks a change to what it serves (`ci/check-relay.sh` reads its `/version`). The runner stamps
`target/suite-passed.json` (HEAD sha + the tmux that drove it, by path and hash) and `release.sh`
skips its own suite when the stamp matches; `MESIMON_RELEASE_RETEST=1` overrides. Releases are
cut locally, not on a runner. **Both Docker gate steps are paused by the author** until they say Windows/WSL2 is
operational — the script prints `SKIPPED` and `MESIMON_RELEASE_DOCKER=1` runs them. Docker is
still needed for the release itself since T-1097: the two Linux tmux binaries build in an Alpine
container and are verified in a Debian one (`ci/build-tmux.sh <target>`, cached under
`vendor/tmux-<target>/` across releases, a verify of seconds when cached). Say so before opening
Docker Desktop.
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

`ci/prune-deps.py` deletes the test executables newer links have superseded and the
split-debuginfo objects no remaining binary references (the runner calls it after every run, on
the build dir that run used, and it skips a `deps/` unchanged since its last prune). A version
bump gives every test binary a new hash and leaves the old one behind, so a release is when the
stale executables are minted. It is what keeps the first-exec Gatekeeper hold down: syspolicyd's
scan is slow in proportion to the *directory*, and one edit to a core crate mints ~6,500 `.o`
files that cargo never collects. Past 50k entries it advises `cargo clean`.

**The Linux cross-build needs a musl C toolchain** — `x86_64-linux-musl-gcc` and
`aarch64-linux-musl-gcc` on PATH (`brew trust messense/macos-cross-toolchains`, then `brew
install messense/macos-cross-toolchains/<target>`), because `ring` came in with Teams (T-332)
and the daemon links it unconditionally. `ci/build-linux.sh` dies with the brew line when one is
missing. Apple clang cannot stand in (no musl sysroot).
