# CLAUDE.md

The operating contract for Claude Code in this repository.

**Do not add historical implementation detail to this file.** A new decision goes in
`docs/STALE-MAP.md`; a new rule or trap goes here, in a line or two, or in the nested
`CLAUDE.md` of the one area it is about; a recipe goes in the `adding-things` skill. This file was
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
**"The tests" are sized to the change, not the whole suite every time** (T-661: 230 full runs in
ticket worktrees, 11 red, 10 of them load flakes and 0 caught by a rebase):
- **A rebase** is `cargo ut` + `cargo build --workspace`. Add the e2e binaries of an area only
  when a conflict you resolved was in that area's code (a STALE-MAP or CHANGELOG conflict is not).
- **Finishing a ticket** is `cargo ut` + the `<area>_e2e` binaries the diff can reach. Run
  `cargo nextest run --workspace` once, at the end, only when the diff changes the daemon, the
  wire (`core/src/command.rs`, `core/src/mcp.rs`), a state file's schema, the tmux backend or
  the e2e harness — the one real catch was a schema bump an unrelated e2e pinned. A TUI-, docs-
  or `ci/`-only diff never needs it.
- The release gate (`ci/test-run.py`, both roads) is the full-suite backstop and is unchanged.
- **A change to what an agent or the crown is told or does** — tool text, `get_ticket`'s
  answer, the brief, a crown wake, a turn road — also gets a rig test in `ci/rig/tests.toml`
  (a bare ask, the acceptance as a check) and one `python3 -B ci/rig.py --only <id>` run before
  the ticket is done (workers on Sonnet, the crown on Opus), with the e2e beside it: a stub proves
  the wire, not that a model finds the words (T-710: the e2e was green and a worker guessed). A
  cut to crown text is judged blind, before and after, as `ci/rig/README.md` says (T-930). Never
  on every build or release; only when such behaviour changes.

In a ticket worktree, never `git checkout main` (it will fail — main belongs to another worktree) and never
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
   something** — that is how a decision survives. **This repository is public; the relay's
   record is not**: a decision about the relay's internals, its hosting, billing or a weakness
   goes in `mesimon-relay/docs/STALE-MAP.md`, and the public block keeps the wire and the
   core's behaviour and links by ticket key. A weakness found in either repository is never
   written here.
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
python3 -B ci/release-watch.py [--dry-run|--mark-read]  # on a claude/codex version change: print the upstream digest; the crown files the research ticket
cargo ut                                              # the inner loop: every unit test, ~1 s
cargo test -p mesimon-core attention                  # one module's tests
cargo test -p mesimon --test hook_e2e                 # one e2e; skips without tmux
cargo nextest run --workspace                         # everything, e2es in PARALLEL (~8 min on 11 cores)
cargo clippy --workspace --all-targets -- -D warnings # the release gate's exact clippy
cargo run                                             # TUI for cwd; `-- daemon --repo <p>` runs the daemon
MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui     # remint goldens after a deliberate visual change
python3 -B ci/test-run.py                             # bounded runner: 20-min deadline, build-output lock, fixture audit, + the mod road pass
python3 -B ci/rig.py [--lay|--only R3|--failed|--reset|--stale] # in a ticket worktree: a board there drives the real claude (Sonnet) through its crown, tests in ci/rig/tests.toml; each run clears dead rig boards
ci/test-linux.sh                                      # whole suite on Debian 12 in Docker (~90 s warm)
ci/sandbox.sh [up|tui|pair|build|relay]               # Docker sandbox: this tree's daemon + relay + Remote Control page
ci/build-linux.sh                                     # the two Linux release binaries, cross-linked here
ci/release.sh --dry-run                               # the full release gate, minus the upload
assets/demo/record.sh [tape]                          # re-record a README GIF from <tape>.tape, default demo (vhs)
ci/site.sh [--publish]                                # build mesimon.dev (site/) into dist/site; publish to Pages
MESIMON_DEMO_KEY_FILE=<key file> assets/demo/record.sh agent  # the agent clip with the real claude (Sonnet, cents)
```

`cargo test --workspace` is valid but runs the e2e binaries serially (~2.5 min). Tests are
exempt from `unwrap_used` via `clippy.toml`. Say so **before** opening Docker Desktop.

**Releasing** — the order, the channels, mesimon.dev, the Homebrew tap, the build-cache
prune and the Linux cross-build — is the `release` skill (`.claude/skills/release/SKILL.md`).

**Remote Control's hosted origin is `https://remote.mesimon.dev` (port 443), fixed**: a browser
keeps its pairing and outbox per origin, so any change re-pairs every phone. **Teams' is
`relay.mesimon.dev:443`** on the same port, split by SNI (T-519); 8443 stays published until
2027-10 for Macs that signed in before.

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

## Adding things

The recipes — a wire command, a key binding, a column setting, a launch flag, a theme, a
machine pref, something the mouse can pick, a card or ticket-page visual, the shin, an e2e
test — are the `adding-things` skill (`.claude/skills/adding-things/SKILL.md`): each lists the
complete set of places a change touches. **If a step is missing there, add it there.**

**Text from a user or an agent** crosses one of two functions in `core/src/text.rs` at the
boundary it crosses: `scrub_cells` before it is drawn, `scrub_text` before it leaves for
another process. The hazard lists live there and nowhere else — **do not write a sanitizer.**

**Text an agent or the crown is told** states what the board does and what is the person's, never
how to work (T-930): how to judge, order, phrase or choose is the person's, in their own prompts.

**Notes are markdown files under the ticket, and `notes[0]` is the description.**
`Ticket.notes: Vec<NoteMeta>` is the metadata; the body is `notes/<ULID>.md`, read by
`Command::ReadNote` (never in the snapshot) and written whole by `WriteNote` (blank on an
existing note deletes). Adding a `NoteMeta` field is `#[serde(default)]` like everything else. **A `Summary` section's
rows (T-696) are derived in the TUI** from the bodies (`core/src/summary.rs`, `App::summaries`),
never on the snapshot, and derived again for the agent on every `get_ticket` (`summary`, with
`mcp::SUMMARY_ABOUT` saying what the heading does, T-710); a `WriteNote` carrying `rev` is
refused when the note moved on.

**Git, in the daemon,** is `crate::git::git(repo)`, never `Command::new("git")`: it scrubs the
`GIT_*` targeting variables a dogfooding daemon inherits.

**A daemon-side change is not running until the daemon restarts** (see the rebuild trap). The
daemon keeps a journal at `<state>/daemon.log` — `started`, `stopping: <why>`, `stopped`, and
`slow turn:` for any writer turn past 1 s. It is the first file to read when a board went
quiet. The feed is what the board did; the journal is what the process did.

## Architecture

Five crates: `mesimon` (the single binary; subcommand dispatch is a hand-rolled match in
`main.rs` — `hook`, `gate`, `mcp` and `mod-bridge` first because they run inside an agent's turn),
`mesimon-core` (pure logic, no I/O), `mesimon-daemon`, `mesimon-backend-tmux`, `mesimon-tui`.

**Single-writer daemon (D22).** One mpsc channel; the main thread is the only mutator of board
state. Client requests, hook frames and the 250 ms tick all arrive as a `Msg` variant in
`daemon/src/server.rs`. Never mutate state from another thread — add new inputs as new `Msg`
variants fed by listener threads. Every mutation passes `authorize()`
(`core/src/authorize.rs`); construct a real `Principal`/`Action`. **The tick forks nothing on
the writer** (T-857): every tmux or `ps` read a tick stage wants is a `want_*` that adds to
`panes::Query`, rides one worker at a time (`queue_pane_sample`) and lands as `Msg::PaneSampled`,
where the stage's absorb runs under its own name with the *sample's* time and re-reads its
candidates off the board, so a record that moved on drops its answer. A new per-tick probe joins
the query; an action at an edge (a paste, an Enter, a kill) stays a fork on the writer.

**Three principals, kept apart.** `Local` is a person, `Agent { session }` means an agent
asked, `Automation { rule }` is the daemon's own rules. **A fourth is another board's daemon**
(T-733): `Peer { board }` reaches this daemon over `orch.sock` for exactly one command, words for
this board's crown (`PeerAsk`), admitted only while the machine's `~/.local/state/mesimon/links.toml`
pairs the two boards (`server/peer.rs`); `authorize` denies it everything else. **`Daemon::place_ticket` is the only
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
**That round-trip is the board's only one** (T-686): the notifier and the keep-awake monitor read
`App.board` (an `Arc<Board>`, handed by `App::hand_board` after every absorb) and dial a snapshot
of their own only while the board is handed away. A new observer thread takes the hand-off too.
**And a push never waits on the UI thread** (T-812): a push, and nearly every mutating key, asks
`App::refresh_soon`, which sends `Snapshot` on the client's second connection (the lane,
`Client::snapshot_detached`) and lands the answer in `pump` (`poll_snapshot`); one ask is out at
a time, and whatever wanted the board meanwhile becomes one follow-up. `App::refresh` is the
waited-for fetch, for a key whose next line reads the fresh board (the `select_ticket` of a card
just made); it lands the ask in flight first, so no board lands after a newer one. A new mutating
key calls `refresh_soon` (or `send`) unless the line after it reads the board. **A
read-modify-write key** — one whose write is decided by a read of the board (a toggle, a cycle,
a whole `ColumnSettings`, a move's card and place, `u`'s target, a note's revision) — calls
`App::catch_up` *before* it reads the board to compute anything, or waits for its own board;
and an intent armed before an ask (a parked focus, a cursor kept home) ignores answers asked
before the key (`App::landing`, the asks' count). **Such a key does block** while the daemon is
slow: `catch_up` waits on this thread for one `RESPONSE_WAIT` at most, then sends nothing and
says `daemon slow ∙ the board is behind ∙ nothing sent`; what that landing reconciles (the
train, the status line, a parked focus) runs in the next `pump`, outside the key's wait. Only the
keys that read nothing (navigation, the frame) never wait.

**Paths (D33b).** Per repo, keyed by `proj16` = sha256(canonical repo path)[..16]: sockets under
`/tmp/mesimon-<uid>/<proj16>/` (sun_path budget — a new socket must be added to the length test
in `daemon/src/paths.rs`), persisted state under `~/.local/state/mesimon/<proj16>/`, board data
under `<repo>/.mesimon/` (uncommitted, excluded via `$GIT_DIR/info/exclude` — **never
`.gitignore`**, per the README's write allowlist).

**tmux backend.** A private server (own socket, generated conf) — never the user's tmux. Session
name = `sid16`, the first 16 hex of the mesimon-minted session UUID; identity is never
discovered (D24). **A running server never re-reads the conf**, so a conf change affects only
fresh servers and must also be issued as a live command (see `install_pane_died_hook`).
**On macOS the server is started by `TmuxBackend::ensure_server` before the first pane** —
`posix_spawn` with the responsibility disclaimed, running `tmux -D` — so it answers to macOS
for itself (T-690); everywhere else the first `new-session` forks it as before. A `-D` server
lives on between its sessions (`exit-empty` off), so `server_alive` is `list-sessions`, never
`has-session`, which fails on an empty server, and `TmuxBackend::panes` is `None` only when no
server answers — the Codex cleanup probes the tmux socket on that alone, never on an empty
list. `folder_access` (`ls` through `run-shell`, the path in the server's environment and never
in the command, since `run-shell` runs under the person's own shell) is the one probe of what a
pane may read; `doctor`'s `private server` line and the daemon's `server_cut_off` notice both
ask it, and only `Operation not permitted` is the cut-off.

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

**Attention flow** (the hook set, the attention machine, exits, restart recovery) and
**the mod road** (T-577 onward: the probe, `auto`, native launches, the mod's half, the turn
roads, `ModUsage`) are described in `crates/mesimon-daemon/CLAUDE.md`.

**The one deciding hook is `mesimon gate`**, a separate subcommand precisely so `mesimon hook`'s
never-writes-stdout invariant stays literally true. It is the 32nd entry — `PreToolUse` matcher
`"Edit,Write,NotebookEdit"`, synchronous — and it refuses structured writes under
`<repo>/.mesimon` and the state dir; on the mod road the mod holds the same rules at `tool.call`,
in the same words, from the pane's `MESIMON_MOD_GATE_*` roots. **`core/src/verdict.rs`'s `Verdict` has `Deny` and
`NoOpinion` and no `Allow`, ever**; a repo-wide test asserts no source line puts `"allow"` or
`"ask"` in a `permissionDecision` position. The decision is local and static from argv, so a
dead daemon cannot make it fail open. **Bash is not hooked** and `docs/USING.md` says so:
command-shape matching is an evasion hole.

**The agent tier (T-84): eight tools, three named movers.** Every Claude session mesimon spawns
on the hook set's road (on the mod's, the mod registers the same tools, above)
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
no tag registry writes, no session read at any tier. The crown's keyed tools are the named
exceptions; its `archive_ticket` is further gated by a board switch a person turns on
(`Board::crown_archives`, T-590, off by default), and its `watch_ticket` by one a person may
turn off (`Board::crown_watches`, T-712, on by default since T-732), which has the board wake
the crown for a person's ticket and lets the crown's `ask_agent` reach its agent as one it
started (T-734); its `sample_agent` (T-973), the one session read, by one it may also turn off
(`Board::crown_samples`, on by default). `authorize()` is real for `Agent`: `Resource::Session`
is denied outright but for `Action::SampleAgent`, and so is `Mutate` on `Resource::Board`.

**Schema evolution.** Every new `SessionRecord` field must be `#[serde(default)]` — the defaults
*are* the migration (back-compat fixture test in `core/src/board.rs`). A parse error
**quarantines** the file (bytes preserved as `<name>.quarantine-<ms>`, board up on a default,
notice in the advisory row) rather than killing the daemon. Each of the ten state files carries
its own `schema_version`; a file from a newer build is left untouched and its writes are
**barred** rather than downgraded. Every save goes through
`persist_columns`/`persist_sessions`/`persist_worktrees`/`persist_queue`/`persist_started`/
`persist_costs`/`persist_crown`/`persist_train`/`persist_forge`, which honour the bars — never call `store::save_*`, `worktree::save_bindings`,
`askqueue::save`, `started::save` or `cost::save` directly. `board.sessions` and the crown
ledger's fields are `Tracked` (T-839): any `&mut` marks them and an unmarked one is not
serialised, so a tick stage reads before it writes. `crown.json` (T-602) is the crown's
wake ledger, so a restart neither loses a wake nor says one twice; `train.json` (T-635) keeps
the merge train's rebase asks whose turn is still open, so a restart mid-rebase keeps the hold; `forge.json` (T-752) keeps each ticket's PRs, so a
restart's first snapshot carries them. The ask queue's starts and wakes ride `queue.json` (T-418) and come
back after a restart; a queued PANE ask is memory-only on purpose and dies with the daemon.
`started.json` (T-441) keeps every conversation a spawned session held, out of the External
drawer after the record is gone; `costs.json` (T-327) keeps every transcript a ticket's sessions
held, with how far it is read and its tokens by hour. The machine's quota reading is not a state
file: `~/.local/state/mesimon/usage.json` is a shared cache, merged per provider. `SessionState` is matched
non-exhaustively in places: use `state.is_live()` for working-set membership and
`state.has_pane()` for pane existence. `Sleeping` is live-but-parked. **`Idle{Background}` is
working, except to words** (T-599): a queued ask, the crown's ask and the crown's wake land on it
(`quiet::holds_against_words`); the train, the parks and the working set never decide it by time.

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
when `Board::live_agent(ticket)` finds one (a parked one holds the seat); resume and wake
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
glyph only on a recorded frame perimeter, inside a recorded picture (`App::mascot`, the pairing
QR's `App::qr`), plus two named codepoints (`▉`, `▎`). `MESIMON_THEME` and `MESIMON_COLOR` force
flavor and profile.

**Anything resolved from the user's environment — the editor, the opener, the notifier, the
sound, the ticket-shell seam — is set in `lib.rs::run`, never `App::new`**, so no test or golden
sees a developer's `$EDITOR`, opens a browser, or makes a noise.

Mesophon's rules (no build step, the CSP, the paired phone's writes) are `web/mesophon/CLAUDE.md`.

## Traps that were measured

Each of these cost a debugging session. They are facts about other people's software, so they
will not show up in our tests until they break something.
The traps about Claude Code's hooks, its mod engine and git are in `crates/mesimon-daemon/CLAUDE.md`;
the terminal's (iTerm2, kitty, tmux's XTVERSION) in `crates/mesimon-tui/CLAUDE.md`; a test stub's shell
(bash 3.2's `read`, dash) in `crates/mesimon/tests/CLAUDE.md`.

- **An Enter sent *with* the text is swallowed by Claude's paste detection.** A prompt is a
  bracketed paste and then a *separate* `send-keys Enter`.
- **A prompt must never ride argv.** Commander dispatches a title that names a subcommand
  ("doctor", "update") to that subcommand, and `--` does not shield it.
- **`SessionStart` fires *during* startup**, so a single Enter on that edge loses a race it lost
  in the first real use. The press repeats every 500 ms until the `UserPromptSubmit` ack, and
  stops outside `Spawning`/`Idle`/`Running` so a startup modal is never answered for the user.
- **…and before Claude asks for raw mode or bracketed paste.** tmux's `paste-buffer -p` brackets
  only for an application that asked, and has no format that says whether it did, so a paste on
  that edge is echoed and cut by the cooked tty (T-570). Launch words go only into a composer
  `agents::claude::composer` reads off the screen. **Exactly one Ctrl+C clears a composer that
  holds text; a second exits Claude** — the resend sends one, only into a box seen holding text.
- **A pty in canonical mode keeps only 1 KiB of a line**, which is why long prompts are pasted
  rather than typed.
- **macOS caps unix datagrams at 2 KB**, which is why the hook transport is SOCK_STREAM one-shot.
- **A `connect()` on a unix socket path can still succeed for a few hundred microseconds after
  its listener's `close()` returned** — XNU routes it into the dying backlog. A liveness probe
  that reads "connected" as live is right to; a test that closes and probes at once must wait
  the kernel out (`native_quit_requires_live_listener_not_a_stale_wrapper_socket`).
- **`tmutil addexclusion` stalls 11 s on TCC** — keep it detached.
- **strip invalidates the linker's ad-hoc arm64 signature**; the macOS binary is deliberately
  not stripped, and the symptom elsewhere is SIGKILL.
- **`/proc/self/exe` reads `…/mesimon (deleted)`** the moment an install renames a new file over
  the running one. Ask for the binary's path through `mesimon_core::exe::current_exe`, which
  caches the first answer and strips the suffix; a core test scans `src/` for a raw
  `std::env::current_exe`. Under Homebrew on Linux it names the versioned keg
  (`Cellar/mesimon/<v>/bin`), which `brew upgrade` deletes; `current_exe` takes it through
  `opt/mesimon`. macOS names brew's `bin/` link, which survives.
- **`pgrep -f <pattern>` matches the shell that runs it**, whose own argv holds the pattern: a
  `while pgrep -f …; do sleep 5; done` never ends (T-599's ten hours). Wait on a pid, a pidfile
  or `pgrep -x`, never on the command's own words.
- **`log` is a zsh builtin** — use `/usr/bin/log`.
- **The libc crate deprecates its `time_t` alias on musl**, so spelling `libc::time_t` warns in
  the Linux builds only. Let a call's signature infer it; `ci/build-linux.sh` clippies each musl
  target with `-D warnings`.
- **A tmux pane id is unique per server, and the server exits with its last session.** A wake
  that kills the only session restarts the server, and the new pane is `%0` like the old one;
  a pane identity must carry the server pid (`#{pid}`) beside `#{pane_id}`.
- **A distro tmux before 3.6 can lose a pane's exit** (T-917): with libutempter (Debian, Ubuntu),
  a fast-exiting pane can read `pane_dead=1` with no status, stay a zombie and fire no
  `pane-died`. `nudge_unreaped` raises SIGCHLD at the server; a GitHub runner also has `gh` in
  `/usr/bin`, ahead of a test stub that only the daemon's PATH carries.
- **macOS keys a process's access to Documents, Desktop and Downloads to its *responsible
  process*, inherited at fork** (T-690): a tmux server forked by the first `new-session`
  answered to the terminal app that opened the board, and once that app quit — and the
  kernel's cached allow lapsed, a week later — every pane read `Operation not permitted` on
  the checkout, agents died at launch in 40 ms and worktree workers could not commit, with
  nothing in mesimon changed. `responsibility_spawnattrs_setdisclaim` + `tmux -D` makes the
  server its own responsible process; macOS then prompts once per tmux binary and keeps the
  answer by path **and code hash**, so a rebuilt `vendor/tmux/tmux` asks again, a platform
  binary (`/bin/ls`) is refused with no prompt, and a binary that sits under `~/Documents`
  (the dev tree's vendored tmux, under `MESIMON_TMUX_BIN`) asks on its first start because it
  page-faults its own text. `tmux -S <sock> run-shell 'ls <repo>'` is the diagnosis.
- **A file written from a Claude Code session carries `com.apple.provenance`, and Gatekeeper
  scans such an executable at its first exec** (T-738): the stub an e2e writes is held 0.2–0.7 s
  before its first line on an idle machine and seconds under the suite's load, `ps` already
  naming `/bin/sh claude-stub.sh` at 0 CPU — nothing of mesimon's. `xattr -d` returns 0 and
  removes nothing. A test that signals a stub waits for a fact the stub writes, never a timer.
  The installed `claude` and a released `mesimon` are assessed once and cached: a real launch
  pays nothing (measured, 60–70 ms pane to answer).

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
gate use: a 20-minute deadline, a lock per build output, and an audit that every fixture reported
`cleaned`. `--jobs N` caps build jobs and test threads; unbounded by default, so the plain cargo
commands stay the inner loop at their usual speed.
**Bounded runs in different worktrees overlap** (T-830): a run locks each Cargo target/build
directory it writes (as cargo resolves them, config included), so only runs sharing build output
take turns, and the waiting one prints `== waiting (not testing yet)` with the holder's pid,
checkout and phase (`running tests`, `pruning the build cache`, …). `--exclusive` (the release
gate) waits for every run and holds new ones off. The locks live in `/tmp/msmn-test-locks-<uid>/`.

If a fixture is ever left behind, `tmux -S /tmp/mesimon-501/<proj16>/tmux.sock kill-server`
cleans it and its `owner.json` says what it owned. **Never sweep every mesimon socket or `pkill`
by name** — a dogfooding session's own board is among them.

The test seams (every `MESIMON_*` a test may set) and the e2e pattern are `crates/mesimon/tests/CLAUDE.md`.

## Release notes

Use the `release-notes` skill (`/release-notes`). Lead with the concrete user-visible change,
then the controls and limitations needed to use it. Keep release notes free of stories, jokes,
metaphors and debugging history. Edit `CHANGELOG.md` and preserve release headings and dates —
`core/src/relnotes.rs` compiles that file in, and `cargo ut` asserts every heading is dated, the
tags are unique and newest-first, and the top entry is `v{CARGO_PKG_VERSION}`.

**Public text is about mesimon** — the site, the README, `docs/`, release notes: no author
biography, location, origin story or self-praise. A line stays only if a reader needs it to
decide, install or use the tool (T-648 took "writes it in Israel" off the site).

## Boundaries

- Apache-2.0 core. **The paid Teams relay is a separate, private repository (`mesimon-relay`)**
  that builds against this one as a sibling checkout; its clients — `crates/mesimon-team`,
  `crates/mesimon-web`, `web/mesophon` — are Apache and live here. Never bring relay code into
  this repo (CONTRIBUTING.md records the CLA rule). `mt/` is gitignored research scratch.
  Relay work happens in a worktree of its own, never in the author's relay checkout:
  `mesimon-relay/CLAUDE.md` has the recipe.
- **A ticket that touches `crates/mesimon-team` carries its relay half** on a relay branch
  named after the same ticket (`msmn/<KEY>-…` in `mesimon-relay`), built and tested against
  this branch, so nothing is owed at release time.
- **The three README promises.** A strict write allowlist: mesimon writes to `<repo>/.mesimon`,
  the state dir, `$GIT_DIR/info/exclude`, the worktrees it cuts and the remote-tracking refs an
  opt-in fetch updates, and nowhere else. No config mutation: `doctor` prints fixes and never
  applies them. **Zero prompt injection**: mesimon adds, removes and reorders no token of the
  conversation — the MCP tool definitions and the agent brief (on by default; the README shows it
  word for word, T-850) are the two named, consented exceptions. Don't write code that violates them, and don't reword promise 1 unasked — its full text in
  `docs/PROMISES.md` or its line in the README.
