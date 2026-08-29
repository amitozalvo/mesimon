# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**mesimon** (repo dir is `kanban-tui`; the product name is mesimon) — a Rust terminal kanban that
orchestrates many coding-agent sessions behind a per-repo daemon and a private tmux server.
Pre-v0.1. Milestones M0 (spikes), M1 (walking skeleton), and M2 (attention) are built; M3
(adoption + resources) is next. The milestone plan and current execution state live in the
auto-memory (`mesimon-project-state`) and `~/.claude/plans/reactive-painting-umbrella.md`.

## Commands

Cargo is not on the default PATH — `source ~/.cargo/env` (or prefix `PATH="$HOME/.cargo/bin:$PATH"`).

```sh
cargo test --workspace              # all tests; e2e tests need tmux installed (skip gracefully)
cargo test -p mesimon-core attention # one module's tests
cargo test -p mesimon --test hook_e2e # the M2 attention e2e (real hook binary + in-process daemon)
cargo clippy --workspace --all-targets # keep clean; workspace warns on unwrap_used (tests exempt by convention)
cargo run                            # TUI for cwd; `cargo run -- daemon --repo <path>` runs the daemon foreground
```

**Rebuild trap:** the daemon is a singleton (flock) started detached; a rebuild swaps the binary on
disk but the RUNNING daemon keeps old code and old `current_exe()` paths. After changing daemon
code, kill it (`pgrep -f "mesimon daemon"`) so the client respawns the new one. Same for spawned
Claude sessions: hooks are injected at launch, so sessions spawned by an old daemon never emit
attention — kill and respawn them too.

E2e tests use per-test dirs `/tmp/msmn-e2e-*`; a test that panics before its cleanup leaks a
private tmux server (plus an idle zsh). `tmux -S /tmp/mesimon-501/<proj16>/tmux.sock kill-server`
cleans one up.

## Doc authority (read this before trusting any doc)

`docs/` is a ~282k-word research corpus written BEFORE any code existed. The rules:

1. `docs/00-DECISIONS.md` is binding; its amendment blocks (D33, D34) override everything above
   them AND all 18 section docs, which do not know about the amendments.
2. `docs/STALE-MAP.md` maps every amendment to the section text it supersedes — check it first.
   It also records **implementation-verified refutations**: e.g. hook exec form is
   `"command": <exe>` + `"args": [...]` (docs/11's bare-`args` example fails the live validator),
   and the hook transport is SOCK_STREAM one-shot (macOS caps unix datagrams at 2 KB).
3. Where 00 is silent, the topic-owner doc wins (table in `docs/README.md`): 02 daemon/wire,
   04 keybindings, 06 design, 07 board UX, 11 attention/state enum, 13 data model, 15 security.
4. Every version number and API claim in the corpus needs re-verification at implementation time —
   the docs have been wrong twice already. Spike verdicts in `docs/spikes/` are empirical and
   trustworthy.

Visual design was deliberately interim through M3; **M3.5 (next) is the design-foundation pass**
(reordered before M4, 2026-08-29): OSC-11 light/dark palette, cursor-column indication, card
anatomy per doc 07, ticket screen skeleton. Remaining polish (decay, animation, banners,
keymap validator) stays in M6. One hard visual rule from day one: exactly ONE saturated colour
on the board, reserved for needs-you (`ACCENT_ATTN` in `mesimon-tui/src/ui.rs`), nothing else
ever.

## Architecture

Five crates: `mesimon` (the single binary; subcommand dispatch is a hand-rolled match in
`main.rs` — `hook` first because it runs inside an agent's turn), `mesimon-core` (pure logic, no
I/O), `mesimon-daemon`, `mesimon-backend-tmux`, `mesimon-tui`.

**Single-writer daemon (D22).** One mpsc channel; the main thread is the only mutator of board
state. Everything — client requests, hook frames, the 250 ms tick — arrives as a `Msg` variant in
`daemon/src/server.rs`. Never mutate state from another thread; add new inputs as new `Msg`
variants fed by listener threads. Every mutation passes `authorize()` (`core/src/authorize.rs`),
which always allows in v0.1 but is the D32c chokepoint — construct a real `Principal`/`Action`.

**Wire protocol.** Newline-delimited JSON over `orch.sock`: `Envelope { principal, command }` →
`Response`, plus one push `Event::BoardChanged` to subscribers. The TUI is push-then-full-refresh:
any event triggers a complete `Snapshot` round-trip (`tui/src/app.rs::tick`). There is no finer
event granularity yet.

**Paths (D33b).** Per repo, keyed by `proj16` = sha256(canonical repo path)[..16]:
sockets under `/tmp/mesimon-<uid>/<proj16>/` (sun_path budget — new sockets must be added to the
length test in `daemon/src/paths.rs`), persisted state under `~/.local/state/mesimon/<proj16>/`
(sessions.json, hooks/, activity.jsonl, tmux.conf), board data under `<repo>/.mesimon/`
(uncommitted, excluded via `$GIT_DIR/info/exclude` — never `.gitignore`, per the README's write
allowlist, which is a product promise: mesimon writes nowhere else).

**tmux backend.** A private server (own socket, generated conf) — never the user's tmux. tmux
session name = `sid16` (first 16 hex of the mesimon-minted session UUID; identity is never
discovered, D24). A running server never re-reads the conf: conf changes only affect fresh
servers, so live-server changes must also be issued as commands (see `install_pane_died_hook`).
Child env is allowlisted (`env_clear`), never inherited.

**Attention flow (M2).** Claude sessions spawn with `--settings <state>/hooks/<uuid>.json` — a
30-entry generated hook set (`daemon/src/hook_settings.rs`; its unit tests encode Claude Code's
silent-failure traps: no `if` off tool events, matchers only where supported). Each hook execs
`mesimon hook`, a pure observer (`mesimon/src/hook.rs` — reads stdin to EOF first, never writes
stdout because stdout is injected into the agent's context, exits 0 always, 500 ms self-abort)
which forwards one frame to `hook.sock`. `daemon/src/ingest.rs` distills frames into `Signal`s;
`core/src/attention.rs` holds the pure, time-injected state machine (fixed precedence ranks 0–16;
ranks 0–8 are the attention set; debounce: enters 0 ms, leaves 1500 ms settle, 15-min stale
demote). **There is no polling**: tmux's `pane-died` hook is the only exit signal, and a 15 s
server-alive guard catches wholesale tmux death. Sessions mesimon didn't spawn get no hooks and
can never emit attention (adoption tier is M3).

**Schema evolution.** `store.rs` hard-fails on parse errors, so every new `SessionRecord` field
must be `#[serde(default)]` — the defaults ARE the migration (back-compat fixture test in
`core/src/board.rs`). The `SessionState` enum is matched non-exhaustively in several places; use
`state.is_live()` for working-set membership and `state.has_pane()` for pane existence —
`Sleeping` is live-but-parked (no pane, no process; the attention machine latches until wake).

**Adoption (M3).** Foreign sessions are discovered lazily (drawer open → `RescanExternal`; never
at startup, never polled) by cwd-field census of `~/.claude/projects/` (`daemon/src/census.rs`;
pure parsers in `core/src/adopt.rs`). Attached records have `provenance: Adopted` + empty argv =
observe-only (transcript tail poller, Low confidence, cannot focus); takeover spawns
`claude --settings <hooks> --resume <claude_session_id>` — argv replay is THE resume mechanism
(resume restores neither --settings nor --mcp-config), never `--bare`. Sleep = SIGTERM pgid →
5 s reaper → kill-pane, record parked FIRST; badge word is `external` (see STALE-MAP M3 section
+ D35 for all deviations).

**Test seams.** `MESIMON_CLAUDE_BIN` (stub agent binary), `MESIMON_HOOK_BIN` (hook binary path for
the pane-died notify — required in e2e because the in-process daemon's `current_exe()` is the test
binary), `MESIMON_CLAUDE_HOME` (census root override for fabricated `~/.claude` trees),
`MESIMON_SLEEP_MIN_AGE_MS` (e2e cannot wait out the 60 s sleep floor). E2e pattern: in-process
daemon thread + real tmux + the real built binary via `env!("CARGO_BIN_EXE_mesimon")` (only
available in `crates/mesimon/tests/`).

## Boundaries

- Apache-2.0 core; `team/` is reserved for a future source-available tier — never mix code across
  that boundary (CONTRIBUTING.md records the CLA rule).
- `mt/` is leftover research scratch, not project content.
- Product promises (README): strict write allowlist, no config mutation (`doctor` prints fixes,
  never applies), zero token injection by default. Don't write code that violates them.
