---
name: release
description: Cutting a mesimon release — the order (bump, CHANGELOG, test run, tag, ship.sh), the unstable and stable channels, mesimon.dev and its install line, the Homebrew tap, the build-cache prune and the Linux musl cross-build. Load before bumping a version or running ci/release.sh, ci/site.sh, ci/build-linux.sh, ci/prune-deps.py or ci/homebrew-formula.sh.
---

# Releasing mesimon

See `AGENTS.md` for the shared development contract. When the person asks for a release,
the release agent runs it through production.

**Release order.** Run `ci/release-prepare.sh --lint` on the warm tree before the
version bump. Fix any lint there. Bump the version, update CHANGELOG and the lock
(`cargo update --workspace --offline`), then commit. Run `ci/release-prepare.sh` on
the candidate. It checks clippy and compiles the relay tests against this exact core
before starting the exclusive two-road gate. For paired relay changes, set
`MESIMON_RELAY_DIR` to the clean relay worktree that will merge.

Once the gate's tests start, prep runs the dependency drift check, site build and
the macOS and two Linux release builds beside them. In a worktree it saves the
three finished binaries and the runner's stamp under the main checkout's
`target/release-prepared/<commit>.zip`. After the core fast-forward, run
`ci/release-prepare.sh --handoff` from either checkout at that commit. This
verifies the binaries and installs the stamp on main for the relay's dry run.
The relay's ship script uses its sibling core main checkout by default. If
`MESIMON_CORE_DIR` is set, point it at that main checkout. The packaging step
reads the prepared binaries.
Merge any relay changes and commit the relay lock update before the dry run.

The gate waits for other bounded runs and holds the exclusive lock through both roads and any
reruns. Both nextest passes use `--no-fail-fast`. After both finish, the runner reruns each
named failure once, alone on its original road at the same clean SHA. Each result names the
road, binary and test as `passed-alone` or `failed-alone`. A rerun that could not execute is
`inconclusive`. Any red in the full runs leaves the gate red and writes no stamp. Bring the
named results to the person for any exception to the two-green-roads requirement.

Run `ci/release-prepare.sh` with Cargo and nextest's default parallelism. Pass `--jobs N`
to cap Cargo build jobs and nextest threads per road. At the next release, record the
parallelism used, both road summaries and elapsed gate time.
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
