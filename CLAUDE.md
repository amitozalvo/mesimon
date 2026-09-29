# CLAUDE.md

The operating contract for Claude Code in this repository.

**Do not add historical implementation detail to this file.** A new decision goes in
`docs/STALE-MAP.md`; a new rule, recipe or trap goes here, in a line or two. This file was
169k characters on 2026-09-11 because every shipped ticket appended a block to it *and* to
STALE-MAP; the long-form narrative now lives in `docs/ARCHITECTURE.md` and nothing was lost.
`AGENTS.md` is the same contract for Codex — when you change a rule here, check it there.

## What this is

**mesimon** — a Rust terminal kanban that orchestrates many coding-agent sessions behind a
per-repo daemon and a private tmux server. Pre-v0.1. Built: M0 (spikes), M1 (walking
skeleton), M2 (attention), M3 (adoption + resources), M3.5 (design foundation), M4a (per-ticket
worktrees + the staged merge flow), M4b (read-only diff viewer), and the M6 keymap pass.

Roadmap and execution state live in the auto-memory (`mesimon-project-state`) and
`~/.claude/plans/reactive-painting-umbrella.md`. The auto-memory does **not** follow into
worktree sessions; the plan files (absolute paths) and this file do.

**If the current git branch starts with `msmn/`, you are in a ticket worktree** — an isolated
checkout dedicated to one ticket, with `MESIMON_TICKET` and `MESIMON_WORKTREE_BRANCH` in the
environment. Work and commit freely on that branch, and **commit your work when done**:
uncommitted changes cannot merge and block cleanup. When asked to rebase, rebase onto the
default branch, resolve conflicts, then run the tests and fix failures before reporting done.
Never `git checkout main` there (it will fail — main belongs to another worktree) and never
merge or push to main yourself: the user merges through mesimon, fast-forward only, so a green
rebased branch is the deliverable. **Never call Claude Code's `EnterWorktree` tool** — a
worktree is the ticket's workspace setting, cut by mesimon. `EnterWorktree` changes the process
cwd, and Claude Code re-homes the transcript under a project dir for the new cwd; the daemon
follows the move (T-433), but a wake still launches from the ticket's cwd.

**You may be running inside mesimon** (a session spawned by the very daemon this repo builds).
Then also: `pkill -f "mesimon daemon"` does not kill you — your pane belongs to the private
tmux server, which the daemon does not own, and daemon death loses no state. Never `cargo run`
in your pane; it wedges a nested fullscreen client. Verify with tests and goldens and let the
user drive the real TUI. E2e tests are safe: they use their own `/tmp/msmn-e2e-*` sockets.

## Source of truth

1. **The code and its tests are the spec** — the keymap validators, the colour laws in
   `theme.rs`, the goldens, the back-compat fixtures. Executable truth cannot drift.
2. **`docs/STALE-MAP.md` is the design record**: what shipped, what was refuted, and why. Read
   it before changing something you did not build, and **append a block when you ship
   something** — that is how a decision survives.
3. **The README's three promises bind** (see Boundaries). They are commitments to users; the
   README gives each one line, and `docs/PROMISES.md` holds their full text.
4. **The pre-code research corpus is not in this repository.** The ~346k-word planning corpus
   (`00-DECISIONS` through `19-tmux-backend-v01`, the proposals, the spikes and the agent-state
   research) was written before any code existed and the code has overtaken it; the author
   keeps it privately. Where a doc disagrees with the code, the code is right and the doc is
   history.

The 261 `07 §4.2`-style citations in the source point into that corpus. They are provenance,
not obligation.

## Commands

Cargo is on PATH (`~/.cargo/bin`, via the shell profile). A session whose shell started before
that was set up may need `source ~/.cargo/env`.

```sh
cargo ut                                              # the inner loop: every unit test, ~1 s
cargo test -p mesimon-core attention                  # one module's tests
cargo test -p mesimon --test hook_e2e                 # one e2e; skips without tmux
cargo nextest run --workspace                         # everything, e2es in PARALLEL (~30 s)
cargo clippy --workspace --all-targets -- -D warnings # the release gate's exact clippy
cargo run                                             # TUI for cwd; `-- daemon --repo <p>` runs the daemon
MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui     # remint goldens after a deliberate visual change
python3 -B ci/test-run.py                             # bounded runner: 20-min deadline, lock, fixture audit
ci/test-linux.sh                                      # whole suite on Debian 12 in Docker (~90 s warm)
ci/build-linux.sh                                     # the two Linux release binaries, cross-linked here
ci/release.sh --dry-run                               # the full release gate, minus the upload
assets/demo/record.sh [tape]                          # re-record a README GIF from <tape>.tape, default demo (vhs)
MESIMON_DEMO_KEY_FILE=<key file> assets/demo/record.sh agent  # the agent clip with the real claude (Sonnet, cents)
```

`cargo test --workspace` is valid but runs the e2e binaries serially (~2.5 min). Tests are
exempt from `unwrap_used` via `clippy.toml`. Say so **before** opening Docker Desktop.

**Release order**: bump + CHANGELOG → commit → `MESIMON_TMUX_BIN=$PWD/vendor/tmux/tmux python3
-B ci/test-run.py` → tag → push → `ci/release.sh`. Never verify before the bump — the bump
relinks every crate and the run is thrown away. The runner stamps `target/suite-passed.json`
(HEAD sha + the tmux that drove it, by path and hash) and `release.sh` skips its own suite when
the stamp matches; `MESIMON_RELEASE_RETEST=1` overrides. Releases are cut locally, not on a
runner. **Both Docker steps are paused by the author** until they say Windows/WSL2 is
operational — the script prints `SKIPPED`, `MESIMON_RELEASE_DOCKER=1` runs them, and you should
not open Docker Desktop for a release.

**The Homebrew tap** (`brew install amitozalvo/tap/mesimon`) is one formula, generated from
`ci/homebrew/mesimon.rb` by `ci/homebrew-formula.sh` and pushed to `amitozalvo/homebrew-tap` by
`release.sh` after `gh release create`. The template's header says how to check a change. The
updater never replaces a binary brew installed (`release.rs::by_homebrew`).

`ci/prune-deps.py` deletes split-debuginfo objects no current binary references (the runner
calls it after every run). It is what keeps the first-exec Gatekeeper hold down: syspolicyd's
scan is slow in proportion to the *directory*, and one edit to a core crate mints ~6,500 `.o`
files that cargo never collects. Past 50k entries it advises `cargo clean`.

**The Linux cross-build needs a musl C toolchain** — `x86_64-linux-musl-gcc` and
`aarch64-linux-musl-gcc` on PATH (`brew trust messense/macos-cross-toolchains`, then `brew
install messense/macos-cross-toolchains/<target>`), because `ring` came in with Teams (T-332)
and the daemon links it unconditionally. `ci/build-linux.sh` dies with the brew line when one is
missing. Apple clang cannot stand in (no musl sysroot).

**mesimon ships its own tmux on macOS** — `ci/build-tmux.sh` builds a static tmux 3.6a into
`vendor/tmux/` and the release packages it as `mesimon-tmux`, never `tmux` (which would shadow
the user's own on PATH). `mesimon_backend_tmux::tmux_bin()` is the resolution ladder
(`MESIMON_TMUX_BIN` → sibling `mesimon-tmux` → PATH) and **both the server commands and
`attach_argv` must use it**: a client from a different tmux build refuses the server over
protocol version. **Linux ships no tmux** and runs the distro's.

**Rebuild trap.** The daemon is a singleton (flock) started detached; a rebuild swaps the
binary on disk but the *running* daemon keeps old code and old `current_exe()` paths. After
changing daemon code, either press `U` in the TUI (clean handover — it shuts the daemon down
and execs the new binary) or `pkill -f "mesimon daemon"` so the client respawns it. SIGTERM is
a clean exit: pending attention settles flush through `apply_change` before the socket goes.
Claude sessions have their hooks injected at launch, so sessions spawned by an old daemon never
emit attention — respawn those too. Prefer the `U` chip when a TUI is likely attached; mention
it after a build rather than pkilling unasked.

## Adding things: the recipes

Each list is the complete set of places a change touches. The compiler finds most of them (the
exhaustive matches are the mechanism); this is the rest. **If a step is missing here, add it
here.**

**A wire command** (something the TUI or an agent asks the daemon to do):
1. `core/src/command.rs`: the `Command` variant — its serde `snake_case` name is its wire name
   and its activity-feed name — then `Command::meta()`: read or mutate, logged or not, which
   ticket. Exhaustive; a new variant does not compile until classified.
2. `core/src/mcp.rs::agent_allows`: may an agent send it? Exhaustive, no `_` arm. Almost always no.
3. `daemon/src/server.rs::handle`: the dispatch arm, calling a `Daemon` method. Only
   `place_ticket` moves a ticket.
4. The sender: a `Verb` in the TUI (next recipe) or a tool in `mesimon/src/mcp.rs`.
5. An e2e if it touches a pane or the disk.

**A key binding:**
1. `core/src/keymap.rs`: a `Binding` in that scope's list (keys, `avail` over `Ctx`, `hint`,
   `mutates`) and a `Verb`. A fact `Ctx` lacks is a field on it — `Ctx` derives `Default`, so
   that is the struct plus `App::ctx()` in `tui/src/app.rs`, nothing else. A new `Scope` also
   goes in `Scope::ALL`, in `parent()`, `word()`, `bindings()` and in `scope_list_is_complete`'s
   `index()`, or it is validated by nothing; a BARRIER scope (parent `None`, every key its own)
   joins the exemption list in `q_pops_and_help_is_everywhere` — `?` is text in a text field.
2. `App::dispatch` in `tui/src/app.rs`: the arm. The match is exhaustive, so the compiler
   points at it.
3. `cargo ut` runs the keymap validators. A hint that shows on the board changes the board
   goldens — remint, then review the diff by eye.

**A column setting:**
1. The field on `core/src/board.rs::ColumnSettings` — `#[serde(default)]`,
   `skip_serializing_if` its default. It reaches disk inside the column's `[[columns]]` table
   and the TUI on the snapshot for free. If its default is today's behaviour, no schema bump;
   if an older build *dropping* it would widen what a spawn or an agent gets, bump
   `COLUMNS_SCHEMA` (`daemon/src/store.rs`; the doctrine is on the constant).
2. `board::template_settings` if the four template columns should carry it — the one place a
   column name is read as a literal.
3. The reader (`automove`, `train::plan`, `place_ticket`'s gate, `agents/claude.rs::flags`,
   `agent_tools_for`, `reclaim_columns`) reads the ticket's column through `Board::column`,
   never a name. `Board::set_column_settings` validates cross-column references.
4. `ColumnSettings::summary` (doctor's `columns` line, the dialog's details) and a `MenuItem` in
   `keymap::COLUMN_ITEMS` with its `Ctx::col_*` word, filled in `App::ctx()`.

**A launch flag** (a column's, a tier's — anything a spawn and a wake must both carry): a
`LaunchContext` field filled in `Daemon::launch_context`, then `agents/claude.rs::flags` AND
its `resume` owned-pair list (a wake replays the stored argv), and `agents/codex/mod.rs::prepare`.
A tier is `core/src/tier.rs`; which one a ticket launches on is `tier::Book`, never a field read.

**A theme:** a `static` `Palette` in `tui/src/theme.rs` (every profile, hand-authored — never
nearest-matched), a `Flavor` variant and its arms in `palette`/`name`/`blurb` (the compiler
finds them), a `Kind` for the law or a fourth clause argued in `test_chroma_law`. `cargo ut`
runs the laws over `Flavor::ALL`; a tag ring must skip the accent's hue band and the ground's.
The picker, the prefs file and the goldens need nothing — rows come from `Flavor::ALL` and
goldens are colourless.

**A machine pref** (a `prefs.json` key; T-361 lets a board override most of them):
1. `core/src/prefs.rs::PrefKey`: the variant, its `name()` (the JSON key), `label()`, and
   whether `board_overridable` — machine-only is for a key about the terminal or the person.
2. `tui/src/prefs.rs`: the `Prefs` field, `load`, `body` (a named value gets the foreign-value
   clause), `word`, and `overlay` if a board may set it; a round-trip test.
3. The `Ctx` field and `App::ctx()`; the `MenuItem` in `SETTINGS_ITEMS`/`NOTIFY_ITEMS` and its
   place in `settings_items`; the `Verb` and its arm through `set_pref`. `keymap::pref_key`
   names the row's key, and board scope then cycles it with no further code.
4. The push, if anything outside the TUI consumes it: a `push_*`/`reconcile_*` pair for the
   daemon, `From<&Prefs>` in `notifier.rs` for the notifier, `drive_caffeine` for power.
5. A `doctor` line via `load_home()`.
**`App::prefs` is the resolved view** (machine under this board's overrides): a test seeds it
with `seed_pref`, never by assignment, and `save_prefs` writes `machine_prefs`.

**A card or ticket-page visual:** `tui/src/ui/card.rs` / `ticket.rs` / `tags.rs`, plus a golden
in `tui/src/ui/tests.rs`. The L1 law tests (`test_no_banned_sgr`, `test_no_drawn_structure`) and
the colour laws in `theme.rs` run in `cargo ut` and say what is wrong.

**The shin (the mascot):** pixels in `assets/mascot/shin.txt`, the engine in `tui/src/creature.rs`,
inks from `Theme::creature_ink`. A new `SessionState` does not compile until `Anim::of` gives it
an animation. The notification PNGs and the installer's welcome are the engine's goldens:
`MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui creature`.

**An e2e test** (`crates/mesimon/tests/<name>_e2e.rs`): `mod common; use common::*;`, then
`let Some(h) = Harness::boot("name", Some(STUB)) else { return };` — the daemon, the private
tmux, the seams and the teardown are the harness's; `h.client("name")` speaks the wire,
`hook_send` runs the real hook binary, `wait_until` polls. Any other `MESIMON_*` seam rides
`Harness::boot_with_env`. `prompt_e2e.rs` is the exemplar.

**Text from a user or an agent** crosses one of two functions in `core/src/text.rs` at the
boundary it crosses: `scrub_cells` before it is drawn, `scrub_text` before it leaves for
another process. The hazard lists live there and nowhere else — **do not write a sanitizer.**

**A paste is one event, and only a text field takes it.** `App::tick` routes `Event::Paste` to
`App::on_paste`; `EditBuffer::paste` flattens to one line (newlines become spaces) under the
field's byte `limit` — the same number the daemon caps at. A new text field passes its limit to
`EditBuffer::new` and joins `App::text_field`, or the layout pause (T-458) reads its Hebrew as
a stray key. `TextArea::paste` is the multi-line counterpart: newlines kept, CRLF
normalised, same limit.

**Notes are markdown files under the ticket, and `notes[0]` is the description.**
`Ticket.notes: Vec<NoteMeta>` is the metadata; the body is `notes/<ULID>.md`, read by
`Command::ReadNote` (never in the snapshot) and written whole by `WriteNote` (blank on an
existing note deletes). Adding a `NoteMeta` field is `#[serde(default)]` like everything else.

**Git, in the daemon,** is `crate::git::git(repo)`, never `Command::new("git")`: it scrubs the
`GIT_*` targeting variables a dogfooding daemon inherits.

**A daemon-side change is not running until the daemon restarts** (see the rebuild trap). The
daemon keeps a journal at `<state>/daemon.log` — `started`, `stopping: <why>`, `stopped`, and
`slow turn:` for any writer turn past 1 s. It is the first file to read when a board went
quiet. The feed is what the board did; the journal is what the process did.

## Architecture

Five crates: `mesimon` (the single binary; subcommand dispatch is a hand-rolled match in
`main.rs` — `hook`, `gate` and `mcp` first because they run inside an agent's turn),
`mesimon-core` (pure logic, no I/O), `mesimon-daemon`, `mesimon-backend-tmux`, `mesimon-tui`.

**Single-writer daemon (D22).** One mpsc channel; the main thread is the only mutator of board
state. Client requests, hook frames and the 250 ms tick all arrive as a `Msg` variant in
`daemon/src/server.rs`. Never mutate state from another thread — add new inputs as new `Msg`
variants fed by listener threads. Every mutation passes `authorize()`
(`core/src/authorize.rs`); construct a real `Principal`/`Action`.

**Three principals, kept apart.** `Local` is a person, `Agent { session }` means an agent
asked, `Automation { rule }` is the daemon's own rules. **`Daemon::place_ticket` is the only
function that moves a ticket** — the DONE gate lives in it, so every mover obeys it.
`daemon/src/movegate.rs` restrains everything that is not a person and never a person: no undo
(an automatic mover may not reverse another principal's move inside 60 s), depth zero (no
automation inside an automation's move), and a fuse (6 automatic moves of one ticket in 120 s
suspends automation for it until a person moves that ticket to another column or 120 s pass
with no automatic attempt; a move on a session's turn edge that follows that same session's
last one is cadence and not counted — `place_ticket`'s `turn_of`). Its state is in memory on
purpose — a debounce, not a security control. A person's prompt reaching the agent supersedes that person's own last move
(`MoveGate::asked_by_hand`).

**Wire protocol.** Newline-delimited JSON over `orch.sock`: `Envelope { principal, command }` →
`Response`, plus one push `Event::BoardChanged` to subscribers. The TUI is
push-then-full-refresh: any event triggers a complete `Snapshot` round-trip. There is no finer
event granularity, and every snapshot field rides one `Snapshot` struct through `App::absorb`.

**Paths (D33b).** Per repo, keyed by `proj16` = sha256(canonical repo path)[..16]: sockets under
`/tmp/mesimon-<uid>/<proj16>/` (sun_path budget — a new socket must be added to the length test
in `daemon/src/paths.rs`), persisted state under `~/.local/state/mesimon/<proj16>/`, board data
under `<repo>/.mesimon/` (uncommitted, excluded via `$GIT_DIR/info/exclude` — **never
`.gitignore`**, per the README's write allowlist).

**tmux backend.** A private server (own socket, generated conf) — never the user's tmux. Session
name = `sid16`, the first 16 hex of the mesimon-minted session UUID; identity is never
discovered (D24). **A running server never re-reads the conf**, so a conf change affects only
fresh servers and must also be issued as a live command (see `install_pane_died_hook`).

**A pane gets the user's own shell environment, delivered by a launcher, never by argv.** The
daemon captures it by running the user's login shell from a clean base env
(`daemon/src/shellenv.rs`, off the writer thread), filters it with a **denylist**
(`core/src/shellenv.rs`) because what a user may export is not enumerable but what mesimon must
withhold is, writes it to `<rt_dir>/shellenv.env` (0600), and every pane's real command line is
`mesimon exec --env <file> --set MESIMON_TICKET=… -- <argv>`, which applies the file, applies
mesimon's own variables last, and `exec`s. The pid stays the agent's and no shell runs. This
replaced per-variable `new-session -e K=V`, which spelled the user's whole environment — API
keys included — on a command line every user on the machine can read with `ps`. `PATH` also
rides `TmuxBackend::set_path`, because tmux takes a pane's PATH from the spawning client. A
live pane keeps the env it was born with; sleep/wake is how a session picks up a new one.

**The keymap is data (`core/src/keymap.rs`), and this is load-bearing.** Nothing in `ui/`
contains a hint literal and nothing in `app.rs` matches on a raw key. A keypress becomes a
`Verb` only through `keymap::resolve(scope, key, &ctx)`, and `App::dispatch` matches `Verb`
exhaustively — so a binding with no handler will not compile. Every `Binding` carries an
availability predicate over `Ctx`, and **the same predicate gates the key and its hint**: a key
that is hinted works, a key that is not available is inert. `Ctx` is built once per keypress and
per frame by `App::ctx()`. Chord tails (`d`, `a`, `z`, `^t`) are scopes with no parent, so a
stray key inside one resolves to `None` and cancels rather than acting; a prefix's `show` is the
single key. `prio: 0` means overlay-only — **a hint sits where it operates**, so a key about one
section is drawn beside that section and not in the footer. Twenty-four tests enforce the
product rules: no key bound twice in a scope chain, legacy-floor atoms only, no banned atoms,
one verb per key across screens, Shift on one axis, `q` pops, `?` everywhere. **Nothing reads
`avail` directly**: `Binding::live` (and `MenuItem::live`) is the one predicate every reader
asks, and it adds the two board-wide rules a joined team board carries — no session, worktree
or git verb on a content-only board, no ticket edit for a viewer — so a new binding never has
to remember them.

**Two atom families sit off the legacy floor, on two different clauses**, and `OFF_FLOOR` in
the test module is the whole list. `Key::ShiftEnter` is *ambiguous* — a terminal that cannot
report it sends a plain `Enter`, another verb — so every binding on it is gated on
`Ctx::rich_keys` and is unbound *and* unhinted where the terminal cannot spell it;
`Key::Ctrl('S')` took that same clause. The four Alt directions fail the other way: the modifier
is eaten and nothing arrives, so the key is inert rather than wrong. That is affordable exactly
while no capability stands behind the atom — every Alt binding must be a **nudge** and must
carry a legacy-floor spelling of the same move on the same screen, hinted. A third off-floor
atom needs one of these two clauses, argued, not a third one.

**Attention flow (M2).** Claude sessions spawn with `--settings <state>/hooks/<uuid>.json`, a
32-entry generated hook set (`daemon/src/hook_settings.rs`; its unit tests encode Claude Code's
silent-failure traps). Thirty-one entries exec `mesimon hook`, a **pure observer** that reads
stdin to EOF first, **never writes stdout** (stdout is injected into the agent's context), exits
0 always, and self-aborts at 500 ms. `daemon/src/ingest.rs` distills frames into `Signal`s;
`core/src/attention.rs` is the pure, time-injected state machine (fixed precedence ranks 0–16,
ranks 0–8 the attention set; enters at 0 ms, leaves after a 1500 ms settle, 15-min stale
demote). **No polling for exits** — tmux's `pane-died` hook is the only exit signal, with a 15 s
server-alive guard for wholesale tmux death. **A death frame names its pane** (`pane` in the hook
header: `<server pid>:<pane id>`, from `--pane` or the pane's own `TMUX`/`TMUX_PANE`) and the
record keeps the key it spawned (`SessionRecord.pane_key`); a wake reuses the session name and
the session uuid, never the pane, so a death frame from another pane is dropped
(`straggler_death`) and no tmux is asked. A frame or record without a key is trusted. After a daemon restart our sessions sit at
`Unknown{DaemonRestarted}` and re-derive from the transcript tail at Low confidence until a hook
re-asserts; reconcile never trusts stale claims.

**The one deciding hook is `mesimon gate`**, a separate subcommand precisely so `mesimon hook`'s
never-writes-stdout invariant stays literally true. It is the 32nd entry — `PreToolUse` matcher
`"Edit,Write,NotebookEdit"`, synchronous — and it refuses structured writes under
`<repo>/.mesimon` and the state dir. **`core/src/verdict.rs`'s `Verdict` has `Deny` and
`NoOpinion` and no `Allow`, ever**; a repo-wide test asserts no source line puts `"allow"` or
`"ask"` in a `permissionDecision` position. The decision is local and static from argv, so a
dead daemon cannot make it fail open. **Bash is not hooked** and `docs/USING.md` says so:
command-shape matching is an evasion hole.

**The agent tier (T-84): eight tools, three named movers.** Every Claude session mesimon spawns
carries `--mcp-config '<inline JSON>'` naming `mesimon mcp`, a stdio shim forwarding each
`tools/call` to `orch.sock` as `Envelope { principal: Agent { session } }`. **The config is
written to no file**, so a session mesimon did not spawn can never reach the tools and revoking
is "stop passing the flag". There is no bearer token, deliberately: the boundary is the 0700
runtime dir. The shim is untrusted and holds no policy. Tools: `get_ticket`, `list_board`,
`move_ticket`, `read_note`, `write_note`, `create_ticket`, `tag_ticket`, `raise_hand`. **No tool
takes a ticket id** — the ticket comes from the session binding, so there is no ownership check
to get wrong. `to_column` is a plain string validated server-side, never an enum, because column
names are the user's words. `core/src/mcp.rs` holds the definitions, the description lint (no
second person, no imperatives) and the ≤820-byte cap.

`mcp::agent_allows` is an **exhaustive match over `Command` with no `_` arm**: a new wire command
will not compile until someone decides whether an agent may send it. That is the enforcement for
the never-tier — no spawn, no kill, no delete/archive/rename, no workspace, no merge, no diff,
no tag registry writes, no session read at any tier. `authorize()` is real for `Agent`:
`Resource::Session` is denied outright and so is `Mutate` on `Resource::Board`.

**Schema evolution.** Every new `SessionRecord` field must be `#[serde(default)]` — the defaults
*are* the migration (back-compat fixture test in `core/src/board.rs`). A parse error
**quarantines** the file (bytes preserved as `<name>.quarantine-<ms>`, board up on a default,
notice in the advisory row) rather than killing the daemon. Each of the six state files carries
its own `schema_version`; a file from a newer build is left untouched and its writes are
**barred** rather than downgraded. Every save goes through
`persist_columns`/`persist_sessions`/`persist_worktrees`/`persist_queue`/`persist_started`, which
honour the bars — never call `store::save_*`, `worktree::save_bindings`, `askqueue::save` or
`started::save` directly. The ask queue's starts and wakes ride `queue.json` (T-418) and come
back after a restart; a queued PANE ask is memory-only on purpose and dies with the daemon.
`started.json` (T-441) keeps every conversation a spawned session held, out of the External
drawer after the record is gone. `SessionState` is matched
non-exhaustively in places: use `state.is_live()` for working-set membership and
`state.has_pane()` for pane existence. `Sleeping` is live-but-parked.

**Worktrees and merge (M4a).** Workspace is a per-ticket field (`Ticket.workspace`); worktrees
live under the state dir at `worktrees/<KEY>-<slug>/` on branch `msmn/<KEY>-<slug>` (slugger in
`core/src/workspace.rs`; **argv arrays always** — a ticket title is an injection vector).
Provisioning is lazy, off the writer thread, and the parked spawn replays on ready. Bindings
persist in `worktrees.json`, with an ownership marker in the git admin dir and pid-bearing locks
with a crash-safe sweep. **A binding is a list of legs** (`worktree::Binding::legs`, schema 2):
one git repo each, the root's unnamed; on a workspace root every census repo gets a leg on the
same `msmn/` branch inside one container, and flags, merge, locks, diff and teardown iterate
legs — never `paths.repo_root` alone. **Merges are ff-only.** A branch reads merged by ancestry *or* by
patch-id against one target ref — and that comparison **writes nothing**, because README promise
1 does not allow a loose object in the repo. The DONE gate and the delete gate go through
`ticket_merged`, and **they must answer as the card does**. Teardown waits for the reaper: never
remove a live cwd.

**One claude per ticket; the second seat is a shell.** `spawn_session` refuses a `Claude` spawn
when `Board::live_claude(ticket)` finds one (a parked one holds the seat); resume and wake
re-enter an existing record and are not gated. Everything that picks "the" agent of a ticket —
`pane_target`, `board_enter`, `auto_move`, `card_glyph`, the worktree lock — assumes one.

**The `!` terminal is per ticket and adoptable (T-366).** A ticket page's `!` opens
`msmn-term-<ticket ulid>` (worktree or checkout), no record; the daemon lists live terminals in
`Response::Board.terminals` and `AdoptTerminal` **renames** that pane to a new Bash record's
`sid16`, so every sid16-keyed road works unchanged. A shell's `foreground` (tmux's
`pane_current_command`) lives in a daemon-side map and rides the snapshot only — never
`sessions.json` — and `glyphs::is_working` is what makes a busy shell spin.

**Leaving a Claude session is the same as sleeping it.** Ctrl+C, `/exit` and Ctrl+D end the
process, never the conversation, so a clean exit parks the record as `Sleeping` and the ticket
keeps its worktree lock. The gate is the same predicate `resume_session` judges it by afterwards
— park exactly when wake would succeed. `/clear` and `/resume` are **not** exits: they end the
conversation inside a living pane, and neither may relabel a real death.

**One hard visual rule: exactly one saturated colour on the board**, reserved for needs-you
(`Theme::attn`; `test_attn_provenance*` enforces it), nothing else ever. Rendering goldens live
in `mesimon-tui/testdata/golden/`. The L1 laws ban SGR 2/3/5/9, reserve SGR 4, and admit a box
glyph only on a recorded frame perimeter plus two named codepoints (`▀`, `▎`). `MESIMON_THEME`
and `MESIMON_COLOR` force flavor and profile.

**Anything resolved from the user's environment — the editor, the opener, the notifier, the
sound, the ticket-shell seam — is set in `lib.rs::run`, never `App::new`**, so no test or golden
sees a developer's `$EDITOR`, opens a browser, or makes a noise.

**Mesophon (`web/mesophon`) has no build step.** The relay serves it as it is, under a CSP with
no inline style or script: styling is classes in `style.css`, and Preact/htm are vendored plain
modules with no bare imports (`vendor/README.md`). Its view subscribes to `store.js` in a layout
effect, never after paint, or a fast boot's emit is lost and the page freezes at "Starting…".
A paired phone's one board write is `Action::FileTicket` into a column: never widen `Paired` to
`Mutate`, which also reaches `PromptColumn`, and a paired device starts nothing.

## Traps that were measured

Each of these cost a debugging session. They are facts about other people's software, so they
will not show up in our tests until they break something.

- **An Enter sent *with* the text is swallowed by Claude's paste detection.** A prompt is a
  bracketed paste and then a *separate* `send-keys Enter`.
- **A prompt must never ride argv.** Commander dispatches a title that names a subcommand
  ("doctor", "update") to that subcommand, and `--` does not shield it.
- **`SessionStart` fires *during* startup**, so a single Enter on that edge loses a race it lost
  in the first real use. The press repeats every 500 ms until the `UserPromptSubmit` ack, and
  stops outside `Spawning`/`Idle`/`Running` so a startup modal is never answered for the user.
- **A pty in canonical mode keeps only 1 KiB of a line**, which is why long prompts are pasted
  rather than typed.
- **macOS caps unix datagrams at 2 KB**, which is why the hook transport is SOCK_STREAM one-shot.
- **A `connect()` on a unix socket path can still succeed for a few hundred microseconds after
  its listener's `close()` returned** — XNU routes it into the dying backlog. A liveness probe
  that reads "connected" as live is right to; a test that closes and probes at once must wait
  the kernel out (`native_quit_requires_live_listener_not_a_stale_wrapper_socket`).
- **The hook exec form is `"command": <exe>` plus `"args": [...]`** — docs/11's bare-`args`
  example fails the live validator. No `if` off tool events; matchers only where supported.
- **`git diff --raw HEAD` writes the destination blob as forty zeros**, so a mode-only test
  cannot be blob equality.
- **`git diff --no-index` exits 1 on differences**, which is not an error.
- **`-uall`, never `-unormal`**: `-unormal` collapses an untracked directory to one unopenable
  `? dir/` row.
- **patch-id comparison needs `--no-renames` on *both* sides** (a path filter would flip rename
  detection on one only) and the git pins on the command line (`diff.orderFile` gone is fatal,
  `log.follow` reaches `log` but not `diff-tree`, `log.abbrevCommit` zeroes the commit column).
- **`--max-count` takes the newest N**, so a walk must be anchored by date, not by count.
- **`tmutil addexclusion` stalls 11 s on TCC** — keep it detached.
- **strip invalidates the linker's ad-hoc arm64 signature**; the macOS binary is deliberately
  not stripped, and the symptom elsewhere is SIGKILL.
- **`/proc/self/exe` reads `…/mesimon (deleted)`** the moment an install renames a new file over
  the running one. Ask for the binary's path through `mesimon_core::exe::current_exe`, which
  caches the first answer and strips the suffix; a core test scans `src/` for a raw
  `std::env::current_exe`. Under Homebrew on Linux it names the versioned keg
  (`Cellar/mesimon/<v>/bin`), which `brew upgrade` deletes; `current_exe` takes it through
  `opt/mesimon`. macOS names brew's `bin/` link, which survives.
- **A human refusing a dialog fires no hook.** "No, keep planning", an Esc out of a question or a
  denied permission emits no `PostToolUse`, no `PostToolUseFailure` and no `PermissionDenied`
  (that one is auto mode's classifier). The agent's next `PreToolUse` is the first frame that
  says the dialog is gone, and it is the refusal road (`Signal::ToolStarted`, T-447).
- **A tool in flight keeps the transcript still for its whole duration**, so an mtime-quiet rule
  reads a 3.5-minute `cargo` call as a dead turn.
- **Claude Code's session file flips `idle` at the end of every turn**, milliseconds before the
  `Stop` hook fires, so a status probe races the hook on every turn.
- **A teammate is listed `running` in every later `Stop` payload for its whole life**, idle or
  not, so teammates are counted and never classed.
- **`automove` is edge-triggered**, and a second `UserPromptSubmit` to an already-`Running`
  session produces no edge — an e2e assertion resting on one passes for the wrong reason. The
  attention machine also arms a flap pin after >4 committed changes in 20 s; it drops inferred
  signals only, and a stated High signal commits through it.
- **`probe_activity` only scans `Running`**, which is what makes `Idle{Background}` immune to it.
- **`log` is a zsh builtin** — use `/usr/bin/log`.
- **A tmux pane id is unique per server, and the server exits with its last session.** A wake
  that kills the only session restarts the server, and the new pane is `%0` like the old one;
  a pane identity must carry the server pid (`#{pid}`) beside `#{pane_id}`.
- **tmux keeps one XTVERSION reply per client and types the next one.** Its own attach-time
  query takes iTerm2's `ESC P >|iTerm2 …`; a pane that asks again through a passthrough gets
  the reply back as M-P plus text (T-488). `extended-keys` stays `on`, never `always`: snacks.nvim
  routes its probe around tmux only under `on`, and the root `S-Enter` bind carries Shift+Enter
  to the panes that never asked.
- **A kitty key-release report outlives the keypress, and tmux types it.** Under the pushed
  flags a release is `CSI code;mods:3 u`; tmux 3.6a's CSI-u parser stops at the `:` and passes
  it to the pane as text. `restore_terminal` pops the flags and then fences on a
  cursor-position reply before the attach (`settle_key_reports`), so nothing reported under the
  flags reaches the tmux client.
- **A `SubagentStop`'s `background_tasks` is the whole session's list**, not the subagent's
  (2.1.283 fills both Stop hooks from one task registry), and its type labels spell the Bash
  tool's background commands and the Monitor tool's watches both `shell`. Only the tool result
  that started a task (`backgroundTaskId`, `taskId`) says which it is (T-483).
- **Plan mode admits an MCP tool only on `annotations.readOnlyHint: true` and ignores allow
  rules; default and auto mode admit only on an allow rule and ignore the hint** (2.1.270). A
  read tool needs both, and `read_rung_is_hinted_read_only` keeps them one list.

## Test infrastructure

E2e tests each own a `/tmp/msmn-e2e-<name>-*` dir and run their daemon as a **subprocess** under
a Python supervisor (`ci/test_guard.py`, spoken to from Rust by `ci/test_support.rs`; Python 3 is
a test dependency only, nothing shipped uses it) that outlives a panicking or killed test and
reaps the daemon, its private tmux server and the registered dirs. The daemon's env is built
**per child** (`TestFixture::set_env` / `Harness::boot_with_env`) and every `MESIMON_*` in the
test process is dropped on the way, so **a seam set with `std::env::set_var` never reaches it** —
`TestFixture::new` panics on one. A failing test echoes its children's `child-N.log` before
teardown. A timeout, a cleanup failure or a skipped test is reported as what it is, **never as a
pass**.

`python3 -B ci/test-run.py [-- cargo test ...]` is the bounded entry point CI and the release
gate use: a 20-minute deadline, an overlap lock, and an audit that every fixture reported
`cleaned`. `--jobs N` caps build jobs and test threads; unbounded by default, so the plain cargo
commands stay the inner loop at their usual speed.

If a fixture is ever left behind, `tmux -S /tmp/mesimon-501/<proj16>/tmux.sock kill-server`
cleans it and its `owner.json` says what it owned. **Never sweep every mesimon socket or `pkill`
by name** — a dogfooding session's own board is among them.

**Test seams.** `MESIMON_CLAUDE_BIN` (stub agent binary), `MESIMON_HOOK_BIN` (hook binary for the
pane-died notify — required in e2e, because the in-process daemon's `current_exe()` is the test
binary), `MESIMON_REQUIRE_TMUX` (turns the e2e tmux skip into a hard failure — set in CI, because
a runner without tmux otherwise reports a green suite that ran almost nothing), `MESIMON_CI`
(relaxes two wall-clock budgets in `hook_e2e`), `MESIMON_NO_DAEMON_RESTART`, `MESIMON_FAKE_BUILD`
(manufacture a stale or newer build in `Hello`), `MESIMON_DAEMON_BIN` (what `spawn_detached`
respawns; required by `restart_skew_e2e`, the only test driving the real `Client::connect`),
`MESIMON_CLAUDE_HOME` (census root for fabricated `~/.claude` trees), `MESIMON_SLEEP_MIN_AGE_MS`,
`MESIMON_CODEX_CLEANUP_STALE_MS` (how long an unconfirmed Codex cleanup may own a live ticket's
checkout before the sweep goes looking for its runtime),
`MESIMON_PANE_QUIET_MS`, `MESIMON_NO_UPDATE_CHECK`, `MESIMON_UPDATE_CHECK`,
`MESIMON_SERVER_GUARD_TICKS`, `MESIMON_WT_REFRESH_TICKS` (the slow bucket: worktree flags, merge
train, CLAUDE.md sample, checkout git sample), `MESIMON_NO_TAG_SEED`,
`MESIMON_TICKET_SHELLS`, `MESIMON_UPDATE_GOLDEN`.

The e2e pattern is an in-process daemon thread + real tmux + the real built binary via
`env!("CARGO_BIN_EXE_mesimon")`, which is only available in `crates/mesimon/tests/`.

## Release notes

Use the `release-notes` skill (`/release-notes`). Lead with the concrete user-visible change,
then the controls and limitations needed to use it. Keep release notes free of stories, jokes,
metaphors and debugging history. Edit `CHANGELOG.md` and preserve release headings and dates —
`core/src/relnotes.rs` compiles that file in, and `cargo ut` asserts every heading is dated, the
tags are unique and newest-first, and the top entry is `v{CARGO_PKG_VERSION}`.

## Boundaries

- Apache-2.0 core. **The paid Teams relay is a separate, private repository (`mesimon-relay`)**
  that builds against this one as a sibling checkout; its clients — `crates/mesimon-team`,
  `crates/mesimon-web`, `web/mesophon` — are Apache and live here. Never bring relay code into
  this repo (CONTRIBUTING.md records the CLA rule). `mt/` is gitignored research scratch.
- **The three README promises.** A strict write allowlist: mesimon writes to `<repo>/.mesimon`,
  the state dir, `$GIT_DIR/info/exclude`, the worktrees it cuts and the remote-tracking refs an
  opt-in fetch updates, and nowhere else. No config mutation: `doctor` prints fixes and never
  applies them. **Zero prompt injection**: mesimon adds, removes and reorders no token of the
  conversation — the MCP tool definitions and the opt-in agent brief are the two named, consented
  exceptions. Don't write code that violates them, and don't reword promise 1 unasked — its full text in
  `docs/PROMISES.md` or its line in the README.
