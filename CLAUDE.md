# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**mesimon** — a Rust terminal kanban that
orchestrates many coding-agent sessions behind a per-repo daemon and a private tmux server.
Pre-v0.1. Milestones M0 (spikes), M1 (walking skeleton), M2 (attention), M3 (adoption +
resources), M3.5 (design foundation), M4a (per-ticket worktrees + the staged merge flow),
and M4b (read-only diff viewer: ticket `v` → `Screen::Diff`; DiffList/DiffFile served
read-only off the writer thread on the connection threads; `!` shell-in-worktree; STALE-MAP
"M4b read-only diff viewer shipped" records the deviations) are built, plus the M6 keymap
pass (below). M4 spec:
`~/.claude/plans/smooth-puzzling-sphinx.md`.
The roadmap and execution state live in the auto-memory (`mesimon-project-state`) and
`~/.claude/plans/reactive-painting-umbrella.md` — note the auto-memory does NOT follow into
worktree sessions; the plan files (absolute paths) and this file do.

If the current git branch starts with `msmn/`, you are in a mesimon ticket worktree: an
isolated checkout dedicated to one ticket (env carries `MESIMON_TICKET` and
`MESIMON_WORKTREE_BRANCH`). Work and commit freely on this branch — **commit your work when
done** (uncommitted changes can't merge and block cleanup), and when the user asks for a
rebase, rebase onto the default branch, resolve conflicts, then run the tests and fix
failures before reporting done. Never `git checkout main` here (it will fail — main belongs
to another worktree) and never merge or push to main yourself: the user merges through
mesimon (fast-forward only, so a green rebased branch is the deliverable).

## Commands

Cargo is on PATH (`~/.cargo/bin`, via the shell profile). A session whose shell was started
before that was set up may still need `source ~/.cargo/env`.

```sh
cargo ut                             # the inner loop: every unit test (core, daemon, tui goldens, the binary's own); ~1 s of test time
cargo nextest run --workspace        # everything, the e2e binaries in PARALLEL (~30 s). `cargo test --workspace` runs them one after another (~2.5 min)
cargo test -p mesimon-core attention # one module's tests
cargo test -p mesimon --test hook_e2e # one e2e (real hook binary + in-process daemon + real tmux; skips without tmux)
cargo clippy --workspace --all-targets -- -D warnings # the release gate's exact clippy; tests are exempt from unwrap_used via clippy.toml
cargo run                            # TUI for cwd; `cargo run -- daemon --repo <path>` runs the daemon foreground
ci/release.sh --dry-run              # the full release gate, minus the upload
```

**mesimon ships its own tmux** (alpha-2): `ci/build-tmux.sh` builds a static tmux 3.6a
(libevent/ncursesw/utf8proc static, system terminfo, pinned+checksummed sources) into
`vendor/tmux/`, and the release packages it beside the binary as `mesimon-tmux` — NOT `tmux`,
which would shadow the user's own on PATH and would also mean an install into a directory that
already has tmux silently "bundles" that one. `mesimon_backend_tmux::tmux_bin()` is the ladder
(`MESIMON_TMUX_BIN` → sibling `mesimon-tmux` → PATH) and BOTH the server commands and
`attach_argv` must use it: a client from a different tmux build refuses the server over protocol
version. `ci/release.sh` runs the e2e suite against the bundled binary, so what ships is what was
tested.

**Releases are cut locally**, not on a runner (`ci/release.sh`): GitHub's macOS
runners bill at 10x on a private repo and the only shipped target is this machine.
The script is the gate — clean tree, tag == HEAD == workspace version, tag pushed,
clippy, dup-dep drift, the full suite with `MESIMON_REQUIRE_TMUX=1` — then build,
`codesign -v` (the binary is deliberately NOT stripped: strip invalidates the
linker's ad-hoc arm64 signature and the symptom elsewhere is SIGKILL), run the
packaged artifact, upload. `.github/workflows/ci.yml` is manual-dispatch only.

**Rebuild trap:** the daemon is a singleton (flock) started detached; a rebuild swaps the binary on
disk but the RUNNING daemon keeps old code and old `current_exe()` paths. After changing daemon
code, kill it (`pgrep -f "mesimon daemon"`) so the client respawns the new one — SIGTERM is a
CLEAN exit since 2026-09-01: `begin_shutdown` flushes every pending attention settle through
`apply_change` (automove included) before the socket goes, the same road `Command::Shutdown`
takes, so a `Stop` one second before the kill still lands its ticket in REVIEW (STALE-MAP "A
pending settle survives a shutdown"; e2e `shutdown_flush_e2e`). Same for spawned
Claude sessions: hooks are injected at launch, so sessions spawned by an old daemon never emit
attention — kill and respawn them too. Softener (2026-08-30): a RUNNING TUI watches its own
binary's mtime and offers `update ready (U reloads)` in the header — `U` shuts the daemon down
cleanly and execs the new binary in place (`tui/src/update.rs`, `lib.rs::reexec`; STALE-MAP
"Opt-in binary update reload"), so interactive dogfooding rarely needs the manual kill. The TUI
also survives daemon death now (reconnect cadence in `tui/src/client.rs`).

**A RELEASED board also asks whether a newer one exists, and a dev board never does.**
`update.rs` only ever fires for someone who already updated — on a released machine nothing moves
that mtime but a hand-run `install.sh` — so `tui/src/release.rs` supplies the missing half and
only that half: it asks the dist repo for the newest tag (at most every 6 h, `curl` on a worker,
the LIST endpoint because `/releases/latest` skips the prereleases every alpha is), raises
`◦ v0.1.0-alpha.5 available (esc)`, and on the menu row being taken downloads it, verifies the
published `.sha256` (**absent means refuse** — `install.sh` only warns because a person is
watching it), runs `--version` on it, and lands it at our own path. **It restarts nothing**: the
swap moves the mtime `update.rs` is already watching, so the existing chip and `U` finish the
job, and `Ctx::release_available` is ANDed with `!update_ready` so a binary already on disk is
reloaded rather than fetched twice. **The dev gate is a stamp, never a heuristic** — a wrong
answer would point a download at somebody's build tree: `mesimon-tui/build.rs` writes
`MESIMON_CHANNEL`, `release` only when `ci/release.sh` set `MESIMON_RELEASE`, and under it sits a
guard nothing lifts (an exe with a `target` component is refused whatever the channel says). The
release script greps `doctor install` on the UNPACKED artifact and dies if the line reads `off`,
because an unstamped release fails silently and forever. `~/.local/state/mesimon/update-check.json`
is a CACHE at the state ROOT (one binary per machine, not per repo) and is treated as one —
unreadable or newer-schema means ignored and rewritten, the inverse of the four state files' rule
— and it is written only on an answer, so a week offline never reads back as a week of checks.
`Ctx::release_tag` is Ctx's first `String` (a version is not a word from a fixed set), which is
why `..ctx` now needs a clone. (STALE-MAP "A released board asks whether a newer one exists".)

E2e tests use per-test dirs `/tmp/msmn-e2e-*`; a test that panics before its cleanup leaks a
private tmux server (plus an idle zsh). `tmux -S /tmp/mesimon-501/<proj16>/tmux.sock kill-server`
cleans one up.

**You may be running INSIDE mesimon** (dogfooding: a session spawned by the very daemon this repo
builds). Everything still applies, plus:

- `pkill -f "mesimon daemon"` does NOT kill you. Your pane belongs to the private tmux server,
  which the daemon does not own; daemon death loses no state (store persisted, TUI reconnects,
  records re-derive from `Unknown{DaemonRestarted}`). Run it after daemon-side rebuilds as usual.
- Prefer the opt-in reload over pkill when a TUI is likely attached: after `cargo build`, the
  user's TUI shows `update ready (U reloads)` and the U press restarts daemon + TUI on the new
  binary. So: build, mention the chip, only pkill if asked or clearly headless.
- Never `cargo run` (the TUI) inside your pane — it wedges a nested fullscreen client there.
  Verify with `cargo test` / goldens; the user runs the real TUI.
- E2e tests are safe here: they use their own `/tmp/msmn-e2e-*` sockets, never your tmux server.
- In a git worktree you get a different `proj16` (own daemon, own sockets) and the repo-root
  auto-memory does not follow you — this file is your only standing context there.

## Adding things: the recipes

Each list is the complete set of places a change touches. The compiler finds most of them
(the exhaustive matches are the mechanism); this is the rest. If a step is missing here,
add it here.

**A wire command** (something the TUI or an agent asks the daemon to do):
1. `core/src/command.rs`: the `Command` variant — its serde `snake_case` name IS its wire name
   and its activity-feed name — then `Command::meta()`: read or mutate, logged or not, which
   ticket. Exhaustive; a new variant does not compile until classified.
2. `core/src/mcp.rs::agent_allows`: may an agent send it? Exhaustive, no `_` arm. Almost always no.
3. `daemon/src/server.rs::handle`: the dispatch arm, calling a `Daemon` method. Only
   `place_ticket` moves a ticket.
4. The sender: a `Verb` in the TUI (next recipe) or a tool in `mesimon/src/mcp.rs`.
5. An e2e if it touches a pane or the disk (below).

**A key binding:**
1. `core/src/keymap.rs`: a `Binding` in that scope's list (keys, `avail` over `Ctx`, `hint`,
   `mutates`) and a `Verb`. A fact `Ctx` lacks is a field on it — `Ctx` derives `Default`, so
   that is the struct plus `App::ctx()` in `tui/src/app.rs`, nothing else. A new `Scope` also
   goes in `Scope::ALL`, beside the enum; `scope_list_is_complete` catches one that does not.
2. `App::dispatch` in `tui/src/app.rs`: the arm. The match is exhaustive, so the compiler
   points at it.
3. `cargo ut`: the keymap validators run. A hint that shows on the board changes the board
   goldens: `MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui`, then review the diff by eye.

**A card or ticket-page visual:** `tui/src/ui/card.rs` / `ticket.rs` / `tags.rs`; a golden in
`tui/src/ui/tests.rs` (`MESIMON_UPDATE_GOLDEN=1` mints it). The L1 law tests
(`test_no_banned_sgr`, `test_no_drawn_structure`) and, for a colour, the law tests in
`theme.rs` run in `cargo ut` and say what is wrong.

**An e2e test** (`crates/mesimon/tests/<name>_e2e.rs`): `mod common; use common::*;`, then
`let Some(h) = Harness::boot("name", Some(STUB)) else { return };` — the daemon, the private
tmux, the seams and the teardown are the harness's; `h.client("name")` speaks the wire,
`hook_send` runs the real hook binary, `wait_until` polls. Any other `MESIMON_*` seam is set
BEFORE `boot` (the daemon reads them once). Each test owns a `/tmp/msmn-e2e-*` dir and its own
tmux socket, which is what lets nextest run them in parallel. `prompt_e2e.rs` is the exemplar.

**Text from a user or an agent** crosses one of two functions in `core/src/text.rs` at the
boundary it crosses — `scrub_cells` before it is drawn, `scrub_text` before it leaves for
another process. The hazard lists live there and nowhere else; do not write a sanitizer.

**Git, in the daemon,** is `crate::git::git(repo)`, never `Command::new("git")`: it scrubs the
`GIT_*` targeting variables a dogfooding daemon inherits.

**A daemon-side change** is not running until the daemon restarts: press `U` in the TUI, or
kill it (the rebuild trap, below).

## Docs are research, not authority (demoted 2026-08-31)

`docs/` is a ~346k-word corpus written BEFORE any code existed, and the code has overtaken it —
every milestone since M1 ended by appending a block to `docs/STALE-MAP.md` recording where the
corpus was wrong. The old ladder (00-DECISIONS binding → STALE-MAP → topic-owner doc → verify) is
retired. What holds now:

1. **The code and its tests are the spec** — the keymap validators, the colour-law tests in
   `theme.rs`, the goldens, the back-compat fixtures. Executable truth is the only kind that
   cannot drift.
2. **`docs/STALE-MAP.md` is the design record**: what was built, what was refuted, and why (e.g.
   the hook exec form is `"command": <exe>` + `"args": [...]` — docs/11's bare-`args` example
   fails the live validator; the hook transport is SOCK_STREAM one-shot because macOS caps unix
   datagrams at 2 KB). Read it before changing something you did not build, and append a block
   when you ship something — that is how a decision survives.
3. **The README's three promises bind** — write allowlist, no config mutation, zero token
   injection. Those are commitments to users, not research.
4. **`docs/spikes/` is trustworthy** — it measured a live system rather than reasoning about one.
5. **Everything else, `00-DECISIONS` included, is idea stock.** Read it for its measurements, its
   reasoning and its failure catalogue; never cite it as the reason something must be a certain
   way; re-verify every version number and API claim at implementation time. Where it disagrees
   with the code, the code is right and the doc is history — the 261 `07 §4.2`-style citations in
   the source are provenance, not obligation.

**M3.5 (built 2026-08-29) is the design foundation**: OSC-11 light/dark detection
(`mesimon-tui/src/detect.rs`, via terminal-colorsaurus, first asked before raw mode and then
re-asked every 3 s from inside `App::tick`, so an OS appearance flip repaints the board live —
`detect::FlavorWatch`; the terminal is the authority, never the OS, and `MESIMON_THEME`, a
terminal that cannot answer, a waiting keypress and an open text field each disarm or defer the
query; STALE-MAP "Light/dark follows the terminal, live"),
the graphite/chalk token themes for all five colour profiles (`theme.rs` — the colour-law tests
in it are the palette's spec), pure board geometry (`layout.rs`, post-D33k arithmetic), card
anatomy per 07 §4 (`ui/card.rs`), spines + the minted cursor-column treatment (`ui/board.rs`),
and the ticket screen skeleton (`ui/ticket.rs` — replaced `Mode::Pick`; zero daemon changes).
Deviations recorded in STALE-MAP's "M3.5 implementation deviations". Rendering goldens live in
`mesimon-tui/testdata/golden/` — `MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui` regenerates
after a deliberate visual change; review the diff by eye. `MESIMON_THEME`/`MESIMON_COLOR` force
flavor/profile. Remaining polish (decay, animation, banners, density ladder, keymap validator)
stays in M6. One hard visual rule: exactly ONE saturated colour on the board, reserved for
needs-you (`Theme::attn`; `test_attn_provenance*` enforces it), nothing else ever.

## Architecture

Five crates: `mesimon` (the single binary; subcommand dispatch is a hand-rolled match in
`main.rs` — `hook`, `gate` and `mcp` first because they run inside an agent's turn),
`mesimon-core` (pure logic, no I/O), `mesimon-daemon`, `mesimon-backend-tmux`, `mesimon-tui`.

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

**A pane gets the user's own shell environment, delivered by a launcher, never by argv.** A
Claude pane is exec'd DIRECTLY by tmux (multi-element argv), so no shell runs and no rc file is
ever read on that path; a shell pane is `[$SHELL]`, one element, which tmux execs into an
interactive zsh that sources `~/.zshrc` normally. D29's nine-name allowlist therefore meant an
`export` the user added could not reach an agent at all — and it was frozen besides, since the
tmux server captures its global env at first launch and nothing ever restarts it. So the daemon
asks the user's login shell instead: `daemon/src/shellenv.rs` runs `$SHELL -l -i -c 'env -0 >
<dump>'` from a CLEAN base env (a prepending rc would otherwise preserve the staleness forever),
off the writer thread, back as `Msg::ShellEnvCaptured`; `core/src/shellenv.rs` filters it with a
DENYlist (tmux plumbing, `TERM*`/`LINES`/`COLUMNS`, `PWD`/`SHLVL`/`_`, `MESIMON_*`) because what a
user may export is not enumerable but what mesimon must withhold is. **The delivery is
`mesimon exec`** (`mesimon/src/exec.rs`): the daemon writes the filtered set plus `PATH` to
`<rt_dir>/shellenv.env` (0600, `K=V\0`, temp+rename) and every pane's real command line is
`mesimon exec --env <file> --set MESIMON_TICKET=… -- <argv>`, which applies the file, then
mesimon's own variables last, and `exec`s — the pid stays the agent's, no shell runs. It
replaced `new-session -e K=V` per variable (2026-09-01), which spelled the user's whole
environment, API keys included, on a command line every user on the machine can read with
`ps`; a `--set` ticket key stays on argv on purpose. `SessionRecord::argv` holds the RAW argv
and `Daemon::launch` wraps it at every spawn. **`PATH` also rides `TmuxBackend::set_path`**,
because tmux takes a pane's PATH from the spawning CLIENT and resolves its own lookups against
it; the launcher is the authority inside the pane, the client road keeps tmux agreeing with it,
and that is why `tmux_bin()` resolves a bare `tmux` to an absolute path. A live pane keeps the
env it was born with (nothing can change a running process's environment); sleep/wake is how a
session picks up a new one, and the Esc-menu row says so. An rc file moving raises
`Ctx::shell_env_stale` → `◦ shell env changed (esc)`; reloading is offered, never automatic (an
editor save must not fork the user's shell). E2e: `crates/mesimon/tests/shell_env_e2e.rs`
(asserts the value is NOT on the pane's command line) and `exec_e2e.rs`; STALE-MAP "A pane
gets the user's own shell environment" + "The environment travels inside the pane".

**Attention flow (M2).** Claude sessions spawn with `--settings <state>/hooks/<uuid>.json` — a
32-entry generated hook set (`daemon/src/hook_settings.rs`; its unit tests encode Claude Code's
silent-failure traps: no `if` off tool events, matchers only where supported). Thirty-one of the
entries exec `mesimon hook`, a pure observer (`mesimon/src/hook.rs` — reads stdin to EOF first, never writes
stdout because stdout is injected into the agent's context, exits 0 always, 500 ms self-abort)
which forwards one frame to `hook.sock`. `daemon/src/ingest.rs` distills frames into `Signal`s;
`core/src/attention.rs` holds the pure, time-injected state machine (fixed precedence ranks 0–16;
ranks 0–8 are the attention set; debounce: enters 0 ms, leaves 1500 ms settle, 15-min stale
demote). **No polling for exits**: tmux's `pane-died` hook is the only exit signal, and a 15 s
server-alive guard catches wholesale tmux death. One deliberate poll exists for the Esc
interrupt, which emits NOTHING (no hook, no transcript record — spike S-E refuted the corpus's
OSC Tier A−): a `Running` Claude pane whose `#{window_activity}` goes quiet 8 s demotes to
`Idle{Interrupted}` at medium confidence, demotion-only (`probe_activity` in server.rs). After a
daemon restart, our own Claude sessions sit at `Unknown{DaemonRestarted}` (reconcile never trusts
stale claims) and borrow the observe tier while `Unknown`: the transcript tail re-derives state at
Low confidence until a hook re-asserts. Sessions mesimon didn't spawn get no hooks and can never
emit attention (adoption tier is M3).

**The keymap is data (`core/src/keymap.rs`), and this is load-bearing.** Nothing in `ui/`
contains a hint literal and nothing in `app.rs` matches on a raw key. A keypress becomes a
`Verb` only through `keymap::resolve(scope, key, &ctx)`, and `App::dispatch` matches `Verb`
exhaustively — so a binding with no handler will not compile. Every `Binding` carries an
availability predicate over `Ctx`, and the SAME predicate gates the key and its hint: a key
that is hinted works, a key that is not available is inert. That is why the footer changes
with the selection (an empty column offers no `rename`) and why `?` lists a different set on
every screen. `Ctx` is built once per keypress and per frame by `App::ctx()`.

To add a binding: add the `Binding` (with its `avail` and `hint`), add the `Verb`, handle it
in `dispatch` — the compiler finds the third step for you. `prio: 0` means overlay-only.
Chord tails (`d`, `a`, `z`, `^t`) are scopes with no parent, so a stray key inside one resolves to
`None` and cancels rather than acting. A prefix's `show` is the single key (`d`, `a`) — never
`d d`: pressing it swaps the footer to the tail's scope, which names the key still to press.
Adding a `Scope` also means adding it to the test module's length-annotated `ALL_SCOPES`, or it
is validated by nothing. Twenty-four tests in `keymap.rs` enforce 04 §2.0:
no key bound twice in a scope chain, legacy-floor atoms only, no banned atoms, and the
product rules (one verb per key across screens, Shift stays on one axis, `q` pops, `?`
everywhere). The `mutates` field is what the D22 `--observer` client will be generated from.

**Two atom families sit off the legacy floor, on two different clauses**, and `OFF_FLOOR` in the
test module is the whole list. `Key::ShiftEnter` is *ambiguous* — a terminal that cannot report
it sends a plain `Enter`, which is another verb — so every binding on it is gated on
`Ctx::rich_keys`, the cached kitty-protocol probe set on `App` by `lib.rs` after `init_terminal`,
and the key is unbound AND unhinted where the terminal cannot spell it
(`shift_enter_is_inert_without_rich_keys`). The four Alt directions (`AltLeft`/`Right`/`Up`/`Down`
— `alt+h` and `alt+←` are one atom, the way the two spellings of `ctrl+]` are) fail the other
way: the modifier is eaten and NOTHING arrives, so the key is inert rather than wrong. That is
affordable exactly while no CAPABILITY stands behind the atom, and that — not the count — is what
`alt_is_admitted_only_for_a_nudge` holds. Every Alt binding must be a NUDGE (it moves the thing
under the cursor one step) and must carry a legacy-floor spelling of the same move on the same
screen, hinted. Two qualify: `Verb::Nudge` on the board, with `> <` beside it, and
`Verb::TagCarryLeft` in the tag picker, where `HJKL` and the four Alt atoms share ONE binding —
the strongest form of the clause, since the accelerator cannot reach a move the floor does not.
A text field never sees an Alt atom at all: `keys::to_key_text`
strips the modifier, because there Alt is `word_wise`'s "by word" and nothing else — which is
why `key_tag` reads `to_key` while steering and `to_key_text` only while naming, and why an
Alt atom (or the `˙` a terminal composes instead) never dismisses the picker as a stray key
would. A third off-floor atom needs one of these two clauses, argued — not a third one.
(STALE-MAP "Alt is admitted, for one verb" + "A tag moves".)

**Composing a ticket: Enter saves, Shift+Enter saves and asks.** A fresh Claude spawn always
types the ticket title into the agent's box and stops (zero token injection, a README promise).
The composer's Shift+Enter is the one gesture that also presses Enter — it mints the ticket,
spawns claude, submits the title as the first prompt, and stays on the board (no handover; the
card is how you watch it). It travels as `Command::SpawnSession { submit_prompt }` →
`SessionRecord.pending_submit` → `send-keys Enter`, started on the
`SessionStart{source: Startup}` frame and **repeated every 500 ms until the `UserPromptSubmit`
ack** (`deliver_pending_submit` / `retry_pending_submits` / `ack_pending_submit`). The retry is
not belt-and-braces: Claude fires SessionStart *during* startup, so a single press on that edge
loses a race it lost in the first real use. Two other traps are measured, not assumed: an Enter
sent *with* the text is swallowed by Claude's paste detection, and the prompt must never ride
argv — commander dispatches a title that names a subcommand ("doctor", "update") to that
subcommand, and `--` does not shield it. Retries stop outside `Spawning`/`Idle`/`Running` so a
startup modal is never answered on the user's behalf. See docs/spikes/T-5's 2026-08-31 addendum
+ correction and STALE-MAP "Shift+Enter composes and asks".

**And on a ticket that already has an agent, the same key ASKS it.** Shift+Enter says one sentence
— *ask claude, and stay on the board* — at three stages: before the ticket exists it mints, spawns
and submits the title (`Verb::SaveStart`); on a ticket with a live claude PANE it opens a one-line
field on the card (`Verb::Prompt` → `InputPurpose::Prompt`); inside that field Enter sends and so
does a second Shift+Enter (the finger is still holding shift). `shift_enter_asks_claude_at_every_stage`
is what keeps that one idea; a fourth home makes it two, and the atom is off the legacy floor
precisely because it buys ONE. The gate is `Ctx::ticket_promptable` — `has_pane()`, NOT
`ticket_has_claude`'s `is_live()`, which counts a `Sleeping` session that has no process to type
at — and it mirrors the daemon's `prompt_target`, which picks the same session `board_enter`
focuses. `Command::PromptSession { ticket, text }` is `MergeToAgent`'s twin (same `paste_text`
delivery: bracketed paste, then a SEPARATE `send-keys Enter`), and the difference is whose words
travel: the merge flow pastes mesimon's, this pastes only the user's, which is why this one hangs
off an ordinary key. `command::sanitize_prompt` only ever REMOVES (control chars — a bare CR would
split one prompt into two turns — and anything past 4 KB) and nothing is appended, so what Claude
reads is a SUBSEQUENCE of what the user typed; `agent_allows` denies the command outright. The
field hangs UNDER the card rather than taking the title line: the ticket is not what is being
edited, it is who the text is going to, so the card renders whole and the prompted card stays the
cursor card (the only text field that does). Status line says `asked`, never "sent" — whether the
agent took it is the hooks' to say. E2e: `crates/mesimon/tests/prompt_e2e.rs` (a `read`-loop stub
writing to a file, so one line proves delivery AND submission). Board only, deliberately — the
ticket page's rail has its own selected session and "which claude" answers differently there.
(STALE-MAP "Shift+Enter asks the agent from the board".)

**Tags are ticket metadata on an axis, and `^t` opens a picker.** `Board.tags` is the registry
(`Tag {name, group, color}`), persisted in `columns.toml` (schema 2 — the bump exists so an older
build bars its writes instead of dropping the registry); `Ticket.tags` is `Vec<TagRef {name,
group}>`, a pointer into it. Colour lives on the REGISTRY, never on the ticket, so recolouring
repaints every card at once instead of leaving 40 tickets holding a stale copy. Nothing is seeded:
"create on the fly" means no setup step, not a derived list — a name enters by being typed in the
picker and stays until `ForgetTag`. Max `MAX_TAGS_PER_GROUP` (10) per axis; groups are 1-10 (`0`
addresses 10). Ten names is more than a picker row fits, so the row is WINDOWED, not
clipped (`tagpicker::window`): the cell under the cursor is always drawn, the window grows
left from it first and then right, and a `~` marks the side still holding cells.

`^t` opens `Scope::TagChord` from the board, the ticket screen AND the composer (a Ctrl-letter is
the only legacy-floor atom a text field cannot swallow — `ctrl+<digit>` is a banned atom, see
STALE-MAP "Ticket tags"). The picker is a grid: `hjkl` walks it, a digit jumps to that group's
row and cycles along it on a repeat, `enter` wears/unwears (or opens the name field on `+ new`),
`tab` cycles the tint, `r` renames, `d` deletes board-wide in two presses, `esc` leaves the field
then the picker. (`w` cycled where the second tag went; it and its two losing homes are gone —
see the stripe paragraph below.)

**`HJKL` (or `alt+hjkl`) carries the tag under the cursor** — the board's nudge, in the picker's
grid, and the cursor rides with it. Along the row it is ORDER, which is what the row draws and
what a repeated digit walks; across rows it is the AXIS, and before this a tag created on the
wrong one had no repair at all (`d` is the only other way off an axis and it strips the tag from
every ticket on the way out). One wire command, `Command::MoveTag { group, name, to_group,
to_index }` → `board::move_tag`, and the wearers travel with it. A cross-axis move is **refused,
never resolved**, when a ticket wearing the tag already wears one on the destination: one tag per
group is what lets a digit address an axis, and the alternative is dropping somebody else's tag
off a card nobody is looking at. The other two refusals are `register_tag`'s — a full axis, a
name that axis already holds. A tag arriving on an axis JOINS it at the end (`to_index` past the
row is the row's end), and the composer mirrors the wearer check client-side because its picks
are on no ticket for the daemon to see. E2e: `crates/mesimon/tests/tags_e2e.rs`. Naming edits the cell **in place**, in its own slot in the grid, with the real
hardware cursor — there is no edit mode, and no block glyph standing in for a cursor. **While naming, resolve against `Scope::TagChord`, never `Scope::Input`** — that
borrow leaked the composer's own hints ("shift+enter save + ask claude") under a tag-name field.
An atom may not appear twice in a scope even with different `avail`, so `enter`/`esc` are one
binding each whose hint switches on `Ctx::tag_naming`.

A tag renders as **the card's own accent bar, painted**: neutral while nothing is tagged, the
tag's tint once something is. The card spends NO cell on tags — the frame is still
`[bar 1][pad 1][content T][pad 1]`, `T = width - 3`. **The bar is the tag channel and nothing
else** (author 2026-09-01): state is the glyph's job, and needs-you also has the inverted title
row, so a second colour ladder on the bar was saying it twice. The ASCII tiers keep their `: | #`
ladder (a shape, not a colour) and a move trail still goes ghost.

**The SECOND tag stacks, and where the stripe is tall it stops being a half-block.** A resting
card has ONE bar cell, so `tags::bar_cell` draws `▀` in the FIRST tag's tint over the second tag's
paint — the split runs across the bar, and since a cell is taller than it is wide those are the
fatter halves. An OPEN card's stripe is three to six cells, and there `tags::stack_full` repaints
it as two runs of full painted blocks: the first tag takes the top ~70%, the second the ~30% under
it, same order, no glyph at all. `second_rows` is the arithmetic (rounded, never fewer than one
row, never more than half) and it returns `None` below three rows, which is what keeps a short
card on the half-block instead of calling a 1/1 split "70/30". The repaint touches span 0 of each
line and nothing else, so no text moves. Two rival homes were built and CUT (author 2026-09-01):
`▌` side-by-side, and the card's right-edge pad — with them went `w`, `MESIMON_TAG_SECOND` and
`tags::Second`, and the picker footer cell `w` held is what `HJKL move tag` now sits in.
**`▀` U+2580 is the ONE admitted exception** — inside the `0x2500-0x259F` range the L1 law bans
AND East Asian Width *Ambiguous* — granted because the channel it replaced (an SGR-58 underline
across the bar) was built, shipped, and could not be seen: one pixel at the bottom of a fully
painted cell. `▌` went back to being banned with the home that spent it: an exception nothing
uses is a ban. `test_no_drawn_structure` names the one admitted codepoint and still bans `▔`/`█`
and the rest of the range; a width test pins it at one cell. The second tag costs no width either
way — `test_the_second_tag_costs_no_width` renders one tag against two, peek off and on. Below
TrueColor there is no tint, no half-block and no split: a plain underline says "tagged" without
saying which. Tags past the second
are named in the peek row and on the ticket page, never on the card. `board::sanitize_tag` runs at
the daemon boundary: a tag name is user text on a card row.

**The palette is TEN hues on one ring, at a lightness each flavor picks for itself, and it has
three loudnesses.** Ten because that is `MAX_TAGS_PER_GROUP`, so one axis can be entirely
colour-distinct (`theme::PIPS` == `board::TAG_TINTS` == 10, pinned by `tag_tints_agree`); six ran
out in use. The hues are even around the wheel EXCEPT the 50-100° band, which is skipped because
that is where `attn` lives — at ten, even spacing is what maximises the worst pair, and the
hand-picked six-hue set could no longer be reproduced. **One ring, not two rings of five**: a
second lightness separates same-hue pairs by ΔL* alone (~ΔE 12) and loses to ten hues on one ring
(ΔE 15.6 graphite / 13.2 chalk), and it would make some tags louder than others, which a tag axis
may never do. Graphite is L* 62 / C* 30; **chalk is L* 38 / C* 26, and the lightness gap is
deliberate** — chalk's ground is paper, so a tint is INK on it and has to be as far below the paper
as graphite's is above its ground. At L* 45 it cleared the text floor and still read as a smudge
(author 2026-09-01: "barely visible on light theme"). Chalk's chroma stays a step lower because
its own accent has less to be a register above: `attn` C* 53.8 puts the ceiling at 26.9. The ramp
stays a register below the accent: `attn` keeps a 2x chroma margin and is still the only token
above C* 30, which `test_pip_ramp_is_low_chroma_and_legible` enforces (ceiling 30.5, 2x under attn,
≥ 4.5 on the page ground — the ticket page writes the ground onto a chip of the tint, so that
number IS the chip's text contrast — ≥ 4.0 on the selected surface, and now every PAIR ≥ ΔE76 12,
which is the number that says whether ten will still go).

A terminal has no alpha, so **`Theme::faded(colour, TagLevel)` blends toward the page ground** (down
into graphite, up into chalk — receding on either flavor) and the card's own state picks the level:
`Selected` is the full tint (the cursor card carries the loudest tags on the board), `Rest` is
where almost every tag is read, `Sleeping` is a parked ticket that still has to answer "which tag",
so its floor is hue survival, not contrast. The first cut used 0.82/0.50 and **neither boundary was
visible on a real board** — an 18% blend is nothing on a one-cell block — so the law test also
asserts each step is ≥ 12% of the ground-to-tint distance. **The two flavors need different
constants to mean the same thing**: the blend is a ratio in sRGB bytes and the same ratio costs far
more toward WHITE than toward black, so graphite is 0.70/0.38 and chalk is 0.76/0.46. Under one
pair, chalk's resting tint landed at C* 16.8 / contrast 2.82 where graphite's landed at 21.7 / 3.61,
and its sleeping tint sat on the C* 8 floor — the other half of "barely visible on light theme".
With the darker ramp and the gentler steps every chalk level now meets or beats the graphite one it
mirrors (rest C* 17.9 / k 3.77, sleep C* 9.8 / k 2.08) while the step stays a step. "Parked" is read off the
SESSIONS (a Sleeping session and no pane), never off the aggregate glyph, which missed any ticket
whose parked session sat behind another glyph. **The ladder is the CARD's, not the palette's**: an
untagged card's neutral block dims and brightens exactly the same way (`tags::bar_cell` fades the
neutral bg too), because a board where only tagged tickets answer "is this asleep?" answers it for
some cards and not others — that is the bug that shipped twice. The law test holds `Rest` above
the dim2 body floor and `Sleeping` above the dim3 de-emphasis floor, and asserts every level keeps
C* ≥ 9 and stays distinct per tag. Below TrueColor the levels collapse: one grey, no ground to fade
into.

**The peek names them** (`tags::chips`): with `p` on, the cursor card carries one row of painted
name-chips under the title, above the reply. Colour says how many and which hues; only words say
which tag, and the stripe only has room for two. Several tags share the row longest-gives-first,
never an equal split (an equal split cut "BUG" to make room for a "STAGING" that then got cut
anyway), nothing shrinks below three cells, and the tail drops rather than every name going
illegible. The row is the TICKET's metadata, so an open card earns it with no session at all —
gating it on `peek.is_some()` meant the commonest tagged card on the board could never show it.

**And a quick-tag digit opens the card it tagged, for 1500 ms.** The stripe is one cell at rest
and carries no words, so `cycle_tag` arms `App::tag_flash` and the card draws itself open —
chip row, and the tall stripe's 70/30 split. `App::peek_showing(ticket)` is the seam (`p` OR a
live flash) and `ui/board.rs` is its only caller. Keyed to the ticket, so `j` ends it; every
repeat re-arms it, so walking an axis keeps the card open for the whole walk; a refusal arms
nothing. `Ctx::peek_on` stays the PREFERENCE — `p` keeps hinting `show replies` under a flash,
because the footer describes the toggle and a flash is not a state anyone toggled. (STALE-MAP
"A quick tag opens the card it tagged".)

**The ticket page's PREVIEW zone reads markdown (`tui/src/rich.rs`).** An agent reply is
markdown, so the zone draws it instead of showing its source — but 06 §5.1 bans SGR 2/3/5/9 and
reserves SGR 4, so the whole vocabulary is value, weight, paint and space: body `dim1`, emphasis
one step up to `base`, strong adds bold, struck text falls to `dim3`, a quote takes the `›`
prefix, a heading buys a breathing row (it cannot have a rule), `---` IS a blank row, and a code
span/fence is `Theme::code_bg()` — the one elevated surface, `selected_bg`, whose second tenant
this is — painted, shrink-wrapped, never bordered and never reflowed. No markdown role touches a
chromatic token. Below the paint (chalk-256, mono) code keeps its backticks rather than faking the
treatment. `peek::sanitize` therefore KEEPS newlines (block structure is nothing else); the card
is unaffected because `peek::wrap` splits on whitespace, and single-row consumers flatten with
`text::one_line`. `test_no_banned_sgr` / `test_no_drawn_structure` now attach a markdown
transcript to the ticket screen and assert it is on screen before sweeping. (STALE-MAP "The
transcript zone reads markdown".)

**The same zone previews a SHELL's pane, and it is the one thing the TUI polls.** A shell keeps no
transcript — tmux is its only record — so a live-shell selection on the ticket page draws the
pane's last lines there instead (`Command::PaneTail` → `TmuxBackend::capture_tail`, oldest first,
bounded 200 lines x 1000 cols). A tty echoes what is typed into it, so the capture carries the
command AND its output with no parsing. **One heading, PREVIEW, covers both** (author
2026-09-01): neither side is the record, both are the last of it, and the rail row beside the zone
already says which session the cursor is on — the zone was briefly TRANSCRIPT/TERMINAL and the
split name earned nothing. It rides the writer thread (the diff service's off-thread
treatment exists for git, not for one small fork); `App::poll_shell_tail` asks on a 1 s clock and
ONLY while a ticket page has a live shell selected, keyed to that session so the cursor never
shows another pane; a refusal still stamps the attempt. Pane bytes go through `peek::sanitize`
before a cell, and both L1 law tests render a dirty tail to prove it. `mcp::agent_allows` denies
`PaneTail`, and `authorize` gets a real `Resource::Session` on that path. E2e:
`crates/mesimon/tests/pane_tail_e2e.rs`. (STALE-MAP "The ticket page reads a shell's pane".)

**A shell never wears the working spinner.** The daemon pins a `Bash` session at `Running` for the
whole life of its pane (D15 — pane death is the only shell event), so `Running` there is liveness,
not activity: `glyphs::is_working` is the single test, a live shell wears the unmoving idle mark,
its age stops ticking seconds, and a card whose only live session is a shell carries no aggregate
glyph. The spinner is the one place D19's motion ban bends and it may only bend for something
moving. (STALE-MAP "A shell does not spin".)

**A card's age is time in COLUMN.** `Ticket.entered_at` is stamped by `mint_ticket` and by
`place_ticket` on a column change (and by `unarchive_ticket` only when its fallback changes the
column) — never by a reorder, a rename, a tag or a session — and `card.rs` renders `age_slot` off
`Ticket::column_since` (falls back to `created_at` on a pre-field ticket) for EVERY card, session
or not; the seconds band still ticks only while an agent works. It was the newest session state
change, which reset on every hook. `reorder_e2e` pins it. (STALE-MAP "A card's age is time in
column".)

**And a launching card wears that arc slowed, not nothing.** `glyphs::launching` is
`spinner(tier, frame / SLOW_STEP_TICKS)` — the working shape at 400 ms a frame, because spawning
is not a different thing from working, it is working that has not started, and the slowness IS
the message. It exists because Shift+Enter STAYS on the board (the card is how you watch the work
land) and the card said nothing until the first hook. It sits between `running` and `sleeping` in
`card_glyph`; `session_glyph` gives it to the rail too. **The window is `glyphs::is_launching`,
NOT `Spawning`** — the `SessionStart` frame both moves the record to `Idle{Unknown}` and is when
the deferred Enter is first pressed, so keying on `Spawning` went dark for the ~500 ms before the
`UserPromptSubmit` ack. `pending_submit` carries it across that seam (an `Idle` agent otherwise
rightly has no glyph: idle means it waits for YOU, and the owed Enter says the wait is ours); it
mirrors the daemon's `pressable`, excludes `Idle{EndTurn}`, and is consulted in no other state, so
a flag reloaded from disk can never strand the mark. Since a worktree ticket's first spawn is
parked with no record at all, `card.rs` falls back to the same arc on a `queued`/`provisioning`
binding — sound because `queue_provision` is reachable only from a spawn. Deliberately NOT
disjoint from the spinner, where `waiting` must be: `Unknown` means we lost track, launching means
we are seconds early. One fast cadence and one slow one on the board, no third. (STALE-MAP "The
launch window is visible".)

**A turn parked on a backgrounded task is `Idle{Background}`, not `Running` and not done.** When
`Stop` arrives carrying a live entry in `background_tasks[]` (`attention::task_blocks_end_turn` —
a dormant `monitor` does NOT count), the turn is PAUSED: the agent said its piece and is waiting
on work it started. Re-asserting `Running` there was two lies in a row — the pane stops painting
the moment the agent parks, so `probe_activity` refuted it 8 s later by demoting to
`Idle{Interrupted}`, which nothing had interrupted, which `automove` rightly refuses to promote,
and which `card_glyph` had no arm for at all: the card went BLANK for the life of the task
(dogfood 2026-09-01, T-128, a backgrounded build-poll, two minutes). `Idle` is invisible to
`probe_activity` (it only scans `Running`), so the misread stops being possible instead of
needing a corrective — which is the difference from `SubagentStop`, the one misread that has one.
Rank is untouched (`Idle{..}` is 13, so D28's table does not move) and only `EndTurn` promotes, so
the ticket correctly stays in IN PROGRESS. The wake arrives as a `UserPromptSubmit` only when a
TASK NOTIFICATION delivers it (a background shell, an unnamed subagent); a named agent in an
interactive session is an in-process TEAMMATE whose report wakes the lead as a teammate message
and fires no prompt hook at all, so the lead's own `PostToolUse` frames (`ToolCompleted { nested:
false }` — `nested` is the payload's `agent_id`, which only a subagent's tool carries) are what
promote a parked turn back to `Running`. **And a teammate is listed `running` in every later Stop
payload for its whole life, idle or not**, so it is COUNTED (`Signal::Stop::teammates`), never
classed: the machine parks iff `blocking_tasks || teammates > idle_teammates.len()`, where the set
is fed by `TeammateIdle{name}` / drained by `SendMessage`'s addressee (`TeammateMessaged`) and
persisted on the record (`SessionRecord.idle_teammates`) so a restart cannot re-park a finished
session (dogfood 2026-09-01, T-135: four idle `/simplify` reviewers held a finished lead at
`Idle{Background}` through three Stops; STALE-MAP "Idle teammates do not hold a turn open").
Its glyph rides the SLOW cadence, because a build
genuinely IS running, just not in this pane — `⠒ ⠌ ⠡`, a two-dot bar turning through the CENTRE,
against `waiting`'s two-dot pair on the rim and the spinner's three-dot arc. Two dots is forced:
one is invisible at dim2 (which is why `waiting` has two) and three would claim work in flight
HERE. ASCII cannot borrow it — `| / - \` ARE the spinner — so it breathes, `o O`. Still no third
speed. (STALE-MAP "A turn parked on background work is its own state".)

**Leaving a Claude session is the same thing as sleeping it.** Ctrl+C-out, `/exit` and Ctrl+D end
the process, never the conversation, so `Daemon::park_on_exit` converts a clean exit to `Sleeping`
and `x` wakes it — one gesture, not two, and the ticket keeps its worktree lock. The gate is the
SAME predicate `resume_session` judges it by afterwards (park exactly when wake would succeed):
`ExitReason::UserQuit` only — where BOTH clean-exit roads land, `SessionEnd{prompt_input_exit}` and
`pane-died` status 0, so whichever wins the race parks — plus non-empty argv and a transcript that
exists. A crash keeps its error mark, a shell is never parked (its pane IS its record), and a
session Ctrl+C-ed before its first prompt stays `Exited` because parking it would mint a sleeper
`x` refuses forever. Re-minting the machine as `Sleeping` is load-bearing: the latch swallows the
pane-died that follows the hook, so a park cannot be flipped back into a corpse by its own echo.
The startup reconcile gets the same say, over records IT just moved and never over already-dead
corpses. E2e: `crates/mesimon/tests/exit_parks_e2e.rs`. (STALE-MAP "Leaving a Claude session parks
it".)

Three neighbours went with it. **`/clear` is not an exit** — like `/resume` before it, it ends the
CONVERSATION inside a living pane, and honoring `SessionEnd{clear}` as a death left the record a
corpse for the rest of the session, since the `SessionStart{clear}` that follows hits the terminal
latch. Both kinds now return `None` from `target`, and neither may relabel a real death, which
makes `ExitReason::Cleared`/`Resumed` unmintable (they read old state files and nothing else).
**`x` on a corpse dismisses it** — `sleep_verb` routes on `Ctx::sel_dead`, the same flag `Enter`
reads to hint "resume", and `x` hints a third word. **And "no transcript to resume" is no longer a
dead end**: where there is no conversation, resuming and starting fresh have the same outcome, so
`resume_session` spawns a fresh one in the same record under a NEWLY MINTED uuid (never `rec.id`
again — collision is not this code's to reason about), sets `claude_session_id`, clears
`transcript_path`, and reports `Response::Spawned { fresh: true }` so the TUI can say the history
is gone. D24 is what makes that free: mesimon's identity is `rec.id` in the `--settings`/
`--mcp-config` blobs, so hooks and the MCP principal never notice the conversation's id move.

Board-wide actions (external drawer, archived list, sleep-all, archive-all) deliberately have
NO key, bar the two the header itself teaches (`U` reloads, `Z` sleeps the done agents — both
overlay-only, so the footer stays the selection's) — they live in the Esc menu (`ui/menu.rs`,
rows from `keymap::menu_items`), because they are rare, are not about the selection, and a menu
row has room to say what it will do.

**Suggestions are pointers at menu rows, never their own surface.** `keymap::SUGGESTIONS` is a
priority-ordered list (update ready > sleep N agents > archive N tickets); each entry's
availability IS its menu row's `avail`, so the header cannot offer what the menu will not do,
and `menu_items` floats the suggested rows to the top in that order. The header shows exactly
ONE — right-aligned, `(esc)` or `(U ∙ esc)`/`(Z ∙ esc)` for the route, no count of the rest — `◦`
marks both the chip and the rows it stands in front of. To add one: add the menu row, add the
`Suggestion`, done. (STALE-MAP "Suggestions are one right-hand chip and a marked menu".)

`?` (`ui/help.rs`) renders `keymap::overlay` and is the complete answer for the current
screen and state.

**Schema evolution.** Every new `SessionRecord` field must still be `#[serde(default)]` — the
defaults ARE the migration (back-compat fixture test in `core/src/board.rs`) — but the stakes
changed in alpha-1: `store.rs` no longer hard-fails on a parse error, so a missing default now
QUARANTINES the file (bytes preserved as `<name>.quarantine-<ms>`, board up on a default, notice
in the advisory row) instead of killing the daemon. Each of the four state files carries its own
`schema_version`; a file from a NEWER build is left untouched and its writes are barred rather
than downgraded. `store::load` returns `Loaded { board, notices, columns_write_barred,
sessions_write_barred }`, and every save goes through `persist_columns`/`persist_sessions`/
`persist_worktrees`, which honour the bars — never call `store::save_*` or
`worktree::save_bindings` directly from the daemon. The `SessionState` enum is matched non-exhaustively in several places; use
`state.is_live()` for working-set membership and `state.has_pane()` for pane existence —
`Sleeping` is live-but-parked (no pane, no process; the attention machine latches until wake).

**The agent tier (T-84): three board tools, and three named movers.** Every Claude session
mesimon spawns also carries `--mcp-config '<inline JSON>'` naming `mesimon mcp` — a stdio shim
that forwards each `tools/call` to `orch.sock` as `Envelope { principal: Agent { session } }`.
The config is written to NO file (no `.mcp.json`, no `~/.claude.json`, no `settings.local.json`,
no plugin), so a session mesimon did not spawn can never reach the tools and revoking is "stop
passing the flag". There is no bearer token, deliberately: `orch.sock` already accepts
`Principal::Local` from any same-uid process and a token in the tmux session env is readable
from every other pane, so the boundary is the 0700 runtime dir — the same one `hook.sock`
already relies on. The shim is untrusted (it runs in the agent's process tree) and holds no
policy.

Tools: `get_ticket`, `list_board`, `move_ticket`. **No tool takes a ticket id** — the ticket
comes from the session binding, so there is no ownership check to get wrong. `to_column` is a
plain string validated server-side, never an `enum`, because column names are the user's words
and an enum would inject them into every request forever. `core/src/mcp.rs` holds the tool
definitions, the description lint (no second person, no imperatives) and the ≤820-byte cap.

`mcp::agent_allows` is an **exhaustive match over `Command` with no `_` arm**: adding a wire
command will not compile until someone decides whether an agent may send it. That is the
enforcement for D10's never-tier — no spawn, no kill, no delete/archive/rename, no workspace, no
merge, no diff, no tags (all six tag commands mutate board-wide registry state), no session
read at any tier. `authorize()` is now real for `Agent`: `Session` is denied outright and so is
`Mutate` on `Resource::Board`.

**`Principal` has three inhabitants, and keeping them apart is load-bearing.** `automove` and
hook ingestion used to run as `Principal::Agent` because it was the only value meaning "not the
human"; once an agent could ask for a move itself that became unworkable. `Automation { rule }`
is now the daemon's own rules, `Agent { session }` means an agent asked, `Local` is a person.
**`Daemon::place_ticket` is the only function that moves a ticket** (it absorbed `auto_move` and
`move_ticket`; the M4 DONE gate lives in it, so every mover obeys it). `daemon/src/movegate.rs`
restrains everything that is not a person, and never a person: *no undo* (an automatic mover may
not reverse a different principal's move inside 60 s), *depth zero* (no automation inside an
automation's move — nothing recurses today, which is why it was cheap before M5's column
on-enter actions), and a *fuse* (6 automatic moves of one ticket in 120 s suspends automation for
it, notice in the advisory row, cleared by any move by hand). State is in memory on purpose: a
debounce, not a security control, so no schema field to get wrong. **M5's column automations
become another `Principal::Automation { rule }` calling `place_ticket` and inherit all of it.**

**The one deciding hook is `mesimon gate`**, a separate subcommand precisely so `mesimon hook`'s
never-writes-stdout invariant stays literally true. It is the 32nd entry — `PreToolUse` matcher
`"Edit,Write,NotebookEdit"`, synchronous, disjoint from the narrow observer entry — and it
refuses structured writes under `<repo>/.mesimon` and the state dir. `core/src/verdict.rs`'s
`Verdict` has `Deny` and `NoOpinion` and **no `Allow`, ever**; a repo-wide test asserts no source
line puts `"allow"` or `"ask"` in a `permissionDecision` position (`ask` collapses to a deny in
headless). The decision is local and static from argv, so a dead daemon cannot make it fail
open; the denial is reported to `hook.sock` afterwards for the feed. **Bash is NOT hooked** and
the README says so: command-shape matching is an evasion hole and hooking every shell call taxes
the thing agents do constantly. `mesimon doctor --mcp` prints the whole surface. E2e:
`crates/mesimon/tests/mcp_e2e.rs`. **Trap for the next e2e: `automove` is edge-triggered AND the
attention machine pins at `Confidence::Low` after >4 committed changes in 20 s (`FLAP_MAX`),
where `automove` refuses to move — a second `UserPromptSubmit` to an already-`Running` session
produces no edge, so an assertion resting on it passes for the wrong reason.**

**Adoption (M3).** Foreign sessions are discovered lazily (drawer open → `RescanExternal`; never
at startup, never polled) by cwd-field census of `~/.claude/projects/` (`daemon/src/census.rs`;
pure parsers in `core/src/adopt.rs`). Attached records have `provenance: Adopted` + empty argv =
observe-only (transcript tail poller, Low confidence, cannot focus); takeover spawns
`claude --settings <hooks> --resume <claude_session_id>` — argv replay is THE resume mechanism
(resume restores neither --settings nor --mcp-config), never `--bare`. Sleep = SIGTERM pgid →
5 s reaper → kill-pane, record parked FIRST; badge word is `external` (see STALE-MAP M3 section
+ D35 for all deviations).

**Worktrees + merge (M4a).** Workspace is a per-ticket field (`Ticket.workspace`, layered:
column policy will only default NEW tickets in M5; board default = shared_checkout — the
composer's Shift+Tab sets it, LOCKED once a session or binding exists). Worktrees live at
`~/.local/state/mesimon/<proj16>/worktrees/<KEY>-<slug>/`, branch `msmn/<KEY>-<slug>`
(doc-12 slugger in `core/src/workspace.rs`; argv arrays always — a ticket title is an
injection vector). Provisioning is lazy (first spawn; `daemon/src/worktree.rs` stages
precheck/add/mark/include/ready, OFF the writer thread via `Msg::Provisioned`, concurrency 2
— trap: `tmutil addexclusion` stalls 11 s on TCC, keep it detached); the parked spawn replays
on ready. Bindings persist in `worktrees.json` (state dir); ownership marker in the git admin
dir; pid-bearing locks + crash-safe sweep. Workspace is chosen ONCE, in the composer
(Shift+Tab) — it locks the moment a session or binding exists, so the ticket screen has no
`w`. **Merges are ff-only** — TUI `m` is a staged flow
(stage derived from git state): ahead+ff → confirm→merge; main moved → inject
"rebase+test" to the agent (conflicts resolve in the worktree, tests run pre-main); merged →
inject the notice. Delete gates on unmerged bindings (`d` then `D` discards, branch `-D`); DONE move
blocked while unmerged; teardown waits for the reaper (never remove a live cwd), single
`--force` only. Card mark `⎇ ⎇… ⎇↑ ⎇↓ ⎇✓ ⎇! ⎇x ⎇-`; sessions in worktrees carry
`MESIMON_WORKTREE_BRANCH` (every spawn carries `MESIMON_TICKET`, worktree or not — T-84), and
spawns pass the user's own `permissions.defaultMode` as `--permission-mode` (fresh worktree
paths lost it otherwise).
E2e: `crates/mesimon/tests/worktree_e2e.rs` (the one e2e with a real git repo).

**Test seams.** `MESIMON_CLAUDE_BIN` (stub agent binary), `MESIMON_HOOK_BIN` (hook binary path for
the pane-died notify — required in e2e because the in-process daemon's `current_exe()` is the test
binary), `MESIMON_REQUIRE_TMUX` (turns the e2e tmux skip into a hard failure — set in CI, because a
runner without tmux otherwise reports a green suite that ran almost nothing), `MESIMON_CI` (relaxes
the two wall-clock budgets in `hook_e2e`), `MESIMON_NO_DAEMON_RESTART` (disables the build-skew
daemon restart), `MESIMON_FAKE_BUILD` (makes the daemon report that build in `Hello`, to
manufacture a stale — or a newer — one), `MESIMON_DAEMON_BIN` (what `spawn_detached` respawns;
required by `restart_skew_e2e`, the only test that drives the real `Client::connect`, because a
test binary has no `daemon` subcommand), `MESIMON_CLAUDE_HOME` (census root override for fabricated `~/.claude`
trees),
`MESIMON_SLEEP_MIN_AGE_MS` (e2e cannot wait out the 60 s sleep floor), `MESIMON_PANE_QUIET_MS`
(shrink the 8 s interrupt-probe quiet threshold), `MESIMON_NO_UPDATE_CHECK` (the user-facing opt-out for the release
check), `MESIMON_UPDATE_CHECK` (force a dev build past the CHANNEL gate — the build-tree guard
still refuses, so copy the binary out of `target/` first),
`MESIMON_SERVER_GUARD_TICKS` (shrink the 15 s
server-alive guard cadence). E2e pattern: in-process
daemon thread + real tmux + the real built binary via `env!("CARGO_BIN_EXE_mesimon")` (only
available in `crates/mesimon/tests/`).

## Boundaries

- Apache-2.0 core; `team/` is reserved for a future source-available tier — never mix code across
  that boundary (CONTRIBUTING.md records the CLA rule).
- `mt/` is leftover research scratch, not project content.
- Product promises (README): strict write allowlist, no config mutation (`doctor` prints fixes,
  never applies), zero PROMPT injection — mesimon adds, removes and reorders no token of the
  conversation, and the three MCP tool definitions are the one named exception (T-84 narrowed
  promise 3 from "token" to "prompt"; `mesimon doctor --mcp` prints the whole surface). Don't
  write code that violates them.
