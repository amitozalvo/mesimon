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
attention — kill and respawn them too. Softener (2026-08-30): a RUNNING TUI watches its own
binary's mtime and offers `update ready (U reloads)` in the header — `U` shuts the daemon down
cleanly and execs the new binary in place (`tui/src/update.rs`, `lib.rs::reexec`; STALE-MAP
"Opt-in binary update reload"), so interactive dogfooding rarely needs the manual kill. The TUI
also survives daemon death now (reconnect cadence in `tui/src/client.rs`).

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
(`mesimon-tui/src/detect.rs`, via terminal-colorsaurus, queried exactly once before raw mode),
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
31-entry generated hook set (`daemon/src/hook_settings.rs`; its unit tests encode Claude Code's
silent-failure traps: no `if` off tool events, matchers only where supported). Each hook execs
`mesimon hook`, a pure observer (`mesimon/src/hook.rs` — reads stdin to EOF first, never writes
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

`Key::ShiftEnter` is the ONE atom off the legacy floor, and it is only admissible because every
binding on it is gated on `Ctx::rich_keys` — the cached kitty-protocol probe, set on `App` by
`lib.rs` after `init_terminal`. A terminal that reports Shift+Enter as a bare Enter therefore
gets the key unbound AND unhinted, never half-working; `shift_enter_is_inert_without_rich_keys`
is what keeps that exception honest. Do not add a second off-floor atom without the same gate.

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

**Tags are ticket metadata on an axis, and `^t` opens a picker.** `Board.tags` is the registry
(`Tag {name, group, color}`), persisted in `columns.toml` (schema 2 — the bump exists so an older
build bars its writes instead of dropping the registry); `Ticket.tags` is `Vec<TagRef {name,
group}>`, a pointer into it. Colour lives on the REGISTRY, never on the ticket, so recolouring
repaints every card at once instead of leaving 40 tickets holding a stale copy. Nothing is seeded:
"create on the fly" means no setup step, not a derived list — a name enters by being typed in the
picker and stays until `ForgetTag`. Max `MAX_TAGS_PER_GROUP` (5) per axis; groups are 1-10 (`0`
addresses 10).

`^t` opens `Scope::TagChord` from the board, the ticket screen AND the composer (a Ctrl-letter is
the only legacy-floor atom a text field cannot swallow — `ctrl+<digit>` is a banned atom, see
STALE-MAP "Ticket tags"). The picker is a grid: `hjkl` walks it, a digit jumps to that group's
row and steps along it on a repeat, `enter` wears/unwears (or opens the name field on `+ new`),
`tab` cycles the tint, `r` renames, `d` deletes board-wide in two presses, `esc` leaves the field
then the picker. **While naming, resolve against `Scope::TagChord`, never `Scope::Input`** — that
borrow leaked the composer's own hints ("shift+enter save + ask claude") under a tag-name field.
An atom may not appear twice in a scope even with different `avail`, so `enter`/`esc` are one
binding each whose hint switches on `Ctx::tag_naming`.

A tag renders as a **painted band across the bottom of the card block**, one row per tag, in the
tag's tint; `p` writes the names onto the bands. Painted, never drawn — every rule glyph
(`─ ▁ █`) sits in the `0x2500-0x259F` range the L1 law bans. Below TrueColor the tint collapses to
one grey, so the band carries `#name` as text instead (`Theme::paints_bands`). Untagged tickets
cost zero rows, which is what keeps the resting board identical to the pre-tags one.
`board::sanitize_tag` runs at the daemon boundary: a tag name is user text on a card row.

Board-wide actions (external drawer, archived list, sleep-all, archive-all) deliberately have
NO key — they live in the Esc menu (`ui/menu.rs`, rows from `keymap::menu_items`), because
they are rare, are not about the selection, and a menu row has room to say what it will do.

**Suggestions are pointers at menu rows, never their own surface.** `keymap::SUGGESTIONS` is a
priority-ordered list (update ready > sleep N agents > archive N tickets); each entry's
availability IS its menu row's `avail`, so the header cannot offer what the menu will not do,
and `menu_items` floats the suggested rows to the top in that order. The header shows exactly
ONE — right-aligned, `(esc)` or `(U ∙ esc)` for the route, no count of the rest — and `◦`
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
`MESIMON_TICKET`/`MESIMON_WORKTREE_BRANCH`, and spawns pass the user's own
`permissions.defaultMode` as `--permission-mode` (fresh worktree paths lost it otherwise).
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
(shrink the 8 s interrupt-probe quiet threshold), `MESIMON_SERVER_GUARD_TICKS` (shrink the 15 s
server-alive guard cadence). E2e pattern: in-process
daemon thread + real tmux + the real built binary via `env!("CARGO_BIN_EXE_mesimon")` (only
available in `crates/mesimon/tests/`).

## Boundaries

- Apache-2.0 core; `team/` is reserved for a future source-available tier — never mix code across
  that boundary (CONTRIBUTING.md records the CLA rule).
- `mt/` is leftover research scratch, not project content.
- Product promises (README): strict write allowlist, no config mutation (`doctor` prints fixes,
  never applies), zero token injection by default. Don't write code that violates them.
