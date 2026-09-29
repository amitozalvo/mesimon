# Mesimon development guide

Mesimon is a Rust terminal kanban that coordinates coding-agent sessions through a
per-repository daemon and a private tmux server. This file is the compact operating
contract for Codex. `CLAUDE.md` is the same contract for Claude Code, plus the change
recipes and the measured traps; `docs/ARCHITECTURE.md` holds the long-form
implementation narrative. Consult either when useful, but do not assume its milestone
narrative is current.

## Ticket context

If `MESIMON_TICKET` is set and a Mesimon MCP `get_ticket` tool is available, call it before
planning or editing. The ticket description and notes may contain context absent from the
prompt. If the tool is unavailable, continue normally rather than treating that as a blocker.

## Source of truth

Use this precedence when sources disagree:

1. Code and tests are the specification.
2. `docs/STALE-MAP.md` is the durable design record of what shipped, what was refuted, and why.
3. The promises in `README.md` (full text in `docs/PROMISES.md`) bind product behavior.
4. The pre-code research corpus the source cites (`07 §4.2`-style) is not in this repository;
   those citations are provenance, not authority.

For a behavioral change, update tests and append the resulting decision or deviation to
`docs/STALE-MAP.md`. Avoid adding historical implementation detail to this file.

## Architecture and invariants

The workspace has five crates:

- `mesimon`: the single binary and its TUI/daemon/hook/gate/MCP subcommand dispatch.
- `mesimon-core`: pure models, commands, authorization, keymaps, and MCP definitions.
- `mesimon-daemon`: persistence, session/worktree lifecycle, hooks, and the single writer.
- `mesimon-backend-tmux`: all private-tmux interaction.
- `mesimon-tui`: application state, terminal handling, and rendering.

Preserve these boundaries:

- The daemon's main thread is the only board-state mutator. Feed new asynchronous input to it
  as messages; do not mutate state from listener or worker threads.
- Every mutation crosses the core authorization chokepoint with a real principal and action.
- The wire protocol is newline-delimited JSON over the per-repository Unix socket. The TUI
  responds to a change notification by fetching a complete snapshot.
- Git subprocesses in the daemon go through its scrubbed Git command helper; do not introduce
  raw `Command::new("git")` calls there.
- Server and client tmux commands must resolve the same Mesimon tmux binary. Mesimon's private
  config deliberately has no prefix and detaches with `Ctrl+]` or `Ctrl+5`.
- Keep `mesimon-core` free of I/O policy and keep authorization/persistence decisions out of
  the MCP shim, which is an untrusted transport process.
- Rendering changes must preserve the tested color and geometry laws. Regenerate goldens only
  for deliberate visual changes and inspect their diffs.
- The mascot's pixels live in `assets/mascot/shin.txt` and its engine in
  `crates/mesimon-tui/src/creature.rs`; the notification PNGs and the installer's welcome are
  that engine's goldens (`MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui creature`).
- The paid Teams relay is a separate, private repository (`mesimon-relay`) built against this
  one as a sibling checkout; `crates/mesimon-team`, `crates/mesimon-web` and `web/mesophon` are
  its Apache clients. Do not bring relay code into this repository. `mt/` is gitignored research
  scratch, not project content.
- `web/mesophon` has no build step: the relay serves it as it is under a CSP with no inline
  style or script, so styling stays in `style.css` and Preact/htm stay vendored plain modules
  with no bare imports (`web/mesophon/vendor/README.md`).

The README promises are hard constraints: Mesimon writes only to its documented allowlist,
`doctor` diagnoses without applying configuration changes, and Mesimon never rewrites or adds
to the user's conversation. MCP tool definitions are the explicit model-input surface; keep
their text descriptive, bounded, and non-instructional.

## Working agreement

- Start by checking `git status`. Preserve user changes and concurrent work; do not overwrite,
  revert, or reformat unrelated files.
- If the branch starts with `msmn/`, it is an isolated ticket worktree. Commit finished work on
  that branch because uncommitted work blocks Mesimon's merge and cleanup flow.
- Rebase an `msmn/` branch only when asked. Never check out, merge into, or push `main` from a
  ticket worktree; the user performs the fast-forward merge through Mesimon.
- Do not use destructive Git commands unless the user explicitly requests them.
- A daemon process keeps running after rebuilds. After daemon-side changes, use the TUI's `U`
  handover or stop the old daemon cleanly before manual runtime verification.
- Do not launch an interactive Mesimon board from a Mesimon-managed agent pane. Prefer tests;
  perform explicitly requested manual TUI checks from an ordinary external terminal.

## Build and verification

Process-owning tests (the e2es and the tmux backend's) need Python 3 and tmux: each runs
its daemon as a subprocess under `ci/test_guard.py`, which reaps it, its private tmux server
and its dirs even when the test panics or is killed. `python3 -B ci/test-run.py [-- cargo test
...]` is the bounded entry point (20-minute deadline, overlap lock, fixture audit; `--jobs N`
caps concurrency, unbounded by default). Seams reach the daemon only through
`Harness::boot_with_env` / `TestFixture::set_env`, never `std::env::set_var`. Report a
timeout, a cleanup failure or a skipped test as what it is, never as a pass. Never sweep every
Mesimon socket or kill by a broad process-name match: this agent may be inside the user's
live board. Inspect the retained run registry if cleanup fails.

Do not point concurrent ticket builds at one shared Cargo target directory. Development and test
profiles disable incremental compilation by default to limit per-worktree disk use;
`CARGO_INCREMENTAL=1` is an explicit local opt-in. The bounded runner requires 5 GiB
free on build and fixture/state volumes and checks again during the run. Low space
stops only that check through its cleanup supervisor and reports failure. Free unused
build output before retrying; `--min-free-gib N` changes the reserve (`0` disables it).
This is a periodic guard, not a disk quota, and direct Cargo commands bypass it.

Use the smallest relevant check while iterating, then the repository gates appropriate to the
change:

```sh
cargo ut
cargo test -p mesimon-core <test-filter>
cargo test -p mesimon --test hook_e2e
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

`cargo ut` is the fast unit-test loop. `cargo nextest run --workspace` is the complete suite and
runs the tmux-backed integration binaries in parallel. `cargo test --workspace` is valid but
substantially slower because those binaries run sequentially.

Tmux-backed tests create Unix sockets and detached processes. In a Codex sandbox they can fail
with missing-socket, server, or pane errors even when the product is correct. When the symptom
specifically indicates sandbox denial, rerun the exact necessary test command with
command-scoped elevated permission. Do not weaken a test, disable the sandbox globally, or
commit personal Codex configuration to make it pass.

Additional targeted gates:

- Deliberate TUI rendering change: `MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui`, followed
  by visual inspection of every golden diff.
- Linux suite: `ci/test-linux.sh` (requires Docker).
- Linux release artifacts: `ci/build-linux.sh`.
- Release rehearsal: `ci/release.sh --dry-run`; follow the script's current Docker policy and
  never publish as part of an ordinary development task.
- Homebrew formula: `ci/homebrew/mesimon.rb` is the template `release.sh` fills and pushes to
  `amitozalvo/homebrew-tap`; its header gives the local-tap check for a change.
- README GIFs: `assets/demo/record.sh [tape]` re-records the clip `assets/demo/<tape>.tape`
  names, `demo` (`assets/demo.gif`) by default (needs `vhs`), after a change the recording shows.
  `agent.gif` is the real claude: `MESIMON_DEMO_KEY_FILE=<key file>` (see `assets/demo/README.md`).

Before handing off, run `git diff --check` and report which checks ran, which were skipped, and
why. Never turn a skipped tmux integration into an implied pass.
