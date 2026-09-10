# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**mesimon** — a Rust terminal kanban that
orchestrates many coding-agent sessions behind a per-repo daemon and a private tmux server.
Pre-v0.1. Milestones M0 (spikes), M1 (walking skeleton), M2 (attention), M3 (adoption +
resources), M3.5 (design foundation), M4a (per-ticket worktrees + the staged merge flow),
and M4b (read-only diff viewer: `v` → `Screen::Diff`, from a ticket page or the board;
DiffList/DiffFile served read-only off the writer thread on the connection threads; `!`
shell-in-worktree — since T-273 the project's TERMINAL, below; STALE-MAP "M4b read-only diff
viewer shipped" records the deviations) are
built, plus the M6 keymap pass (below). M4 spec:
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

## Release notes

When writing or revising release notes, use the `release-notes` skill
(`/release-notes`), available at
[.claude/skills/release-notes/SKILL.md](.claude/skills/release-notes/SKILL.md).
Its source is [docs/release-notes/SKILL.md](docs/release-notes/SKILL.md).
Lead with the concrete user-visible change, then the controls and limitations
needed to use it. Keep release notes free of stories, jokes, metaphors, and
debugging history. Edit `CHANGELOG.md` and preserve release headings and dates.

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
ci/test-linux.sh                     # the whole suite on Linux (Docker, Debian 12, the DISTRO's tmux 3.3a); ~90 s warm. Docker Desktop must be up — SAY SO before `open -a Docker`
ci/build-linux.sh                    # the two Linux release binaries, cross-linked from this Mac (static musl, rust-lld, no Docker)
MESIMON_TMUX_BIN=$PWD/vendor/tmux/tmux python3 -B ci/test-run.py  # the RELEASE's suite, once, on a clean committed tree: stamps target/suite-passed.json, then prunes deps/
python3 -B ci/prune-deps.py             # delete the split-debuginfo objects no current binary references (test-run.py does it after every run); `--dry-run` counts
ci/release.sh --dry-run              # the full release gate, minus the upload; honours the stamp and skips its own suite
```

**mesimon ships its own tmux** (alpha-2): `ci/build-tmux.sh` builds a static tmux 3.6a
(libevent/ncursesw/utf8proc static, system terminfo, pinned+checksummed sources) into
`vendor/tmux/`, and the release packages it beside the binary as `mesimon-tmux` — NOT `tmux`,
which would shadow the user's own on PATH and would also mean an install into a directory that
already has tmux silently "bundles" that one. `mesimon_backend_tmux::tmux_bin()` is the ladder
(`MESIMON_TMUX_BIN` → sibling `mesimon-tmux` → PATH) and BOTH the server commands and
`attach_argv` must use it: a client from a different tmux build refuses the server over protocol
version. `ci/release.sh` runs the e2e suite against the bundled binary, so what ships is what was
tested. **That is macOS only. Linux ships no tmux** and runs the distro's (3.2a on Ubuntu 22.04
up to 3.5a on Debian 13) — and **every tmux before 3.6 rewrites a control character in `-F`
format OUTPUT as `_`**, which is why `SEP` in `backend-tmux/src/lib.rs` is a printable `|` and
why the suite has to run on a distro tmux, not the bundled one (`ci/test-linux.sh`). Doctor's
floor is 3.3 (`allow-passthrough`; 3.1–3.2 runs but WARNs that T-10's containment is off).
(STALE-MAP "Linux ships, and every tmux before 3.6 rewrites a tab".)

**Releases are cut locally**, not on a runner (`ci/release.sh`): GitHub's macOS
runners bill at 10x on a private repo and the macOS target is this machine; the two
Linux targets (`x86_64`/`aarch64-unknown-linux-musl`, static, WSL2 is the Windows
road) are cross-linked here by the toolchain's own `rust-lld` (`ci/build-linux.sh`,
no C in the dependency graph so no cross toolchain). The script is the gate — clean
tree, tag == HEAD == workspace version, tag pushed, clippy, dup-dep drift, the full
suite with `MESIMON_REQUIRE_TMUX=1` (linked first, then run under a 40-minute deadline: macOS holds every freshly linked executable ~30 s on its first exec, and the serial suite ran past the wrapper's default 20 minutes on alpha.15 with every test green — NEVER warm the binaries in parallel, it stalls every exec on the machine and hung both live boards; STALE-MAP "The release gate leaves room for a cold run"). **Since alpha.16 the gate honours a STAMPED pass instead of re-running it** (2026-09-06): `ci/test-run.py` writes `target/suite-passed.json` after a clean FULL-workspace run on a CLEAN tree — HEAD sha, the tmux that drove it by path and sha256 — and `release.sh` skips its link+test steps only when the stamp names HEAD and the BUNDLED tmux by hash (`MESIMON_RELEASE_RETEST=1` runs them anyway). So the release order is: bump + CHANGELOG → commit → `MESIMON_TMUX_BIN=$PWD/vendor/tmux/tmux python3 -B ci/test-run.py` (nextest, once, on the commit that ships — 53 s end to end on a clean `deps/`) → tag → push → `ci/release.sh`. Never verify before the bump too: the bump relinks every crate and the run is thrown away. The hold was MEASURED that day and it is NOT the binary: syspolicyd's first-exec check is slow in proportion to the executable's DIRECTORY — an identical byte-patched, re-signed file execs in 0.4 s from `target/debug/` (14 entries) or an empty dir and in 37 s from `target/debug/deps/` (879,000 entries, 50 GB after ONE WEEK of relinks: split-debuginfo `.o` files and stale e2e binaries that cargo never collects), with syspolicyd at 60% CPU walking it and then a 12 s XProtect pass. One edit to a core crate mints ~6,500 new `.o` files (content-hashed names) and cargo deletes none, so `ci/prune-deps.py` removes every loose object no current binary's `OSO` stabs name — under cargo's build lock — and `ci/test-run.py` runs it after each run; past 50k entries it advises `cargo clean` (a full rebuild is ~25 s). That is what removes the hold; Privacy & Security → Developer Tools does nothing for it (24 s from iTerm2 with the grant), and neither does binary size (`mesimon` at 23.6 MB scans in 0.5 s from `target/debug/`); STALE-MAP "The release gate honours a stamped pass"), the same suite on Linux in Docker (dies without
Docker, never skips — **but both Docker steps are PAUSED by the author since 2026-09-02
until they say Windows/WSL2 is operational**: the script prints `SKIPPED` for each and
`MESIMON_RELEASE_DOCKER=1` runs them; do not open Docker Desktop for a release) — then build, `codesign -v` (the macOS binary is deliberately NOT
stripped: strip invalidates the linker's ad-hoc arm64 signature and the symptom
elsewhere is SIGKILL), run every packaged artifact (the Linux ones inside a Debian
container of their own architecture, checking `--version` and the `update checks`
stamp), upload. The asset names live in `release.rs::PUBLISHED`, `ci/build-linux.sh`
and `install.sh`, pinned to each other by a unit test that reads the scripts.
`.github/workflows/ci.yml` is manual-dispatch only, a macOS + ubuntu matrix.

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
also survives daemon death now (reconnect cadence in `tui/src/client.rs`). **The handover is
lock-aware (2026-09-04):** `reexec` waits for the socket AND the daemon's flock to go
(`client::await_daemon_gone`, up to `HANDOVER_MAX` 30 s, a sentence on the terminal after 1 s),
`connect_or_spawn` spawns nothing and runs no stopwatch while `daemon.lock` is held (probed
`LOCK_SH`, never taken), and no daemon at launch opens an EMPTY board on the reconnect cadence
with the reason in the advisory row instead of exiting — a `U` during a parallel e2e run outran
the old 2 s + 5 s and left twelve live sessions with no board (STALE-MAP "The reload waits for
the daemon it asked to stop"). **And the reload SPEAKS (2026-09-05):** `reexec` blanks the primary
screen and prints `mesimon: reloading…`, and `run` holds a `client::LateWord` (`mesimon:
connecting to the daemon…`, said only past 1 s) across the connect and the first snapshot — before
this, tmux's stale `[detached …]` line was the only thing on screen for the whole connect, and a
slow one under load read as a hang (STALE-MAP "The reload says what it is doing"). **And the
binary's own path is asked ONCE, through `mesimon_core::exe::current_exe`** (T-280, 2026-09-06):
on Linux `/proc/self/exe` reads `…/mesimon (deleted)` the moment an install renamed a new file
over the running one, and the reload exec'd that name on WSL; the helper caches the first answer
and strips the suffix, every caller (reload, update watch, hook set, daemon respawn, bundled tmux,
doctor) goes through it, and a core test scans `src/` for a raw `std::env::current_exe` (STALE-MAP
"The reload execs the path, not the inode").

**A RELEASED board also asks whether a newer one exists, and a dev board never does.**
`update.rs` only ever fires for someone who already updated — on a released machine nothing moves
that mtime but a hand-run `install.sh` — so `tui/src/release.rs` supplies the missing half and
only that half: it asks the dist repo for the newest tag (at most every 30 min, `curl` on a worker,
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

E2e tests each own a `/tmp/msmn-e2e-<name>-*` dir and run their daemon as a SUBPROCESS under a
Python supervisor (`ci/test_guard.py`, spoken to from Rust by `ci/test_support.rs`; Python 3 is a
test dependency only, nothing shipped uses it) that outlives a panicking or killed test and reaps
the daemon, its private tmux server and the registered dirs — so nothing is left to sweep by hand.
If one ever is, `tmux -S /tmp/mesimon-501/<proj16>/tmux.sock kill-server` still cleans it and its
`owner.json` says what it owned; never sweep every mesimon socket or `pkill` by name, since a
dogfooding session's own board is among them. `python3 -B ci/test-run.py [-- cargo test ...]` is
the bounded entry point CI and the release gate use: a 20-minute deadline, an overlap lock, and an
audit that every fixture reported `cleaned` (a failed audit keeps its `/tmp/msmn-test-run-*`
registry; a clean one removes it). `--jobs N` caps build jobs and test threads; unbounded by
default, so the plain cargo commands above stay the inner loop at their usual speed. The daemon's
env is built PER CHILD (`TestFixture::set_env` / `Harness::boot_with_env`) and every `MESIMON_*`
in the test process is dropped on the way, so a seam set with `std::env::set_var` never reaches
it — `TestFixture::new` panics on one (`common::DAEMON_SEAMS`). A failing test echoes its
children's `child-N.log` into the captured output before the fixture is torn down. A timeout, a
cleanup failure or a skipped test is reported as what it is, never as a pass.

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

**A column setting** (T-117, 2026-09-06):
1. The field on `core/src/board.rs::ColumnSettings` — `#[serde(default)]`, `skip_serializing_if`
   its default. It reaches the disk inside the column's `[[columns]]` table and the TUI on the
   snapshot for free. If its default is today's behaviour, no schema bump; if an older build
   DROPPING it would widen what a spawn or an agent gets (`agent_tools`, `claude_mode`), bump
   `COLUMNS_SCHEMA` (`daemon/src/store.rs`, the doctrine is on the constant).
2. `board::template_settings` if the four template columns should carry it — the ONE place a
   column name is read as a literal (a fresh board and the v3→v4 migration both seed from it).
3. The reader — `automove`, `train::plan`, `place_ticket`'s gate, `permission_mode_for`,
   `agent_tier`, `reclaim_columns` — reads the ticket's column through `Board::column`, never a
   name. `Board::set_column_settings` is where a cross-column reference is validated.
4. `ColumnSettings::summary` (doctor's `columns` line and the dialog's details) and a `MenuItem`
   in `keymap::COLUMN_ITEMS` with its `Ctx::col_*` word, filled in `App::ctx()`; a toggle's
   dispatch arm is one `self.set_column(|s| …)`.

**A theme:** a `static` `Palette` in `tui/src/theme.rs` (every profile, hand-authored — never
nearest-matched), a `Flavor` variant and its arms in `palette`/`name`/`blurb` (the compiler
finds them), a `Kind` for the law or a fourth clause argued in `test_chroma_law`. `cargo ut`
runs the laws over `Flavor::ALL`; the ring, if there is one, must skip the accent's hue band AND
the ground's. The picker, the prefs file and the goldens need nothing: rows come from
`Flavor::ALL` and goldens are colourless.

**A card or ticket-page visual:** `tui/src/ui/card.rs` / `ticket.rs` / `tags.rs`; a golden in
`tui/src/ui/tests.rs` (`MESIMON_UPDATE_GOLDEN=1` mints it). The L1 law tests
(`test_no_banned_sgr`, `test_no_drawn_structure`) and, for a colour, the law tests in
`theme.rs` run in `cargo ut` and say what is wrong.

**An e2e test** (`crates/mesimon/tests/<name>_e2e.rs`): `mod common; use common::*;`, then
`let Some(h) = Harness::boot("name", Some(STUB)) else { return };` — the daemon, the private
tmux, the seams and the teardown are the harness's; `h.client("name")` speaks the wire,
`hook_send` runs the real hook binary, `wait_until` polls. Any other `MESIMON_*` seam rides
`Harness::boot_with_env(name, stub, &[(key, value)])` — the daemon is a child process, so a
`std::env::set_var` in the test would never reach it, and the fixture panics on one. Each test
owns a `/tmp/msmn-e2e-*` dir and its own tmux socket, which is what lets nextest run them in
parallel. `prompt_e2e.rs` is the exemplar.

**Text from a user or an agent** crosses one of two functions in `core/src/text.rs` at the
boundary it crosses — `scrub_cells` before it is drawn, `scrub_text` before it leaves for
another process. The hazard lists live there and nowhere else; do not write a sanitizer.

**Notes are markdown files under the ticket, and `notes[0]` is the description** (2026-09-02).
`Ticket.notes: Vec<NoteMeta>` (`[[notes]]` after `[[tags]]`, before `[archived]`; `TICKET_SCHEMA`
2) is the metadata — id, `name` (the body's first line, recomputed on every write), `rev`, who and
when as `local` / `agent:<uuid>` — and the body is `notes/<ULID>.md`, read by `Command::ReadNote`
(never in the snapshot) and written whole by `WriteNote { note: None | Some }` (blank on an
existing note deletes). `sanitize_note` keeps newlines and caps at 32 KiB. The TUI's `Mode::Editor`
is the one multi-line field (`text.rs::TextArea`): `Tab` grows the composer into it — composing,
it is a DIALOG over the board (the cursor card grown: same surface, same tag-painted bar down its
left edge, snapped to a run of WHOLE columns so the cards beside it never show as slivers —
`editor::dialog_rect`/`snap_to_columns`) that grows out of the phantom card for 180 ms
(`Editor::grow`, `App::cursor_card`, `editor::draw_dialog`) and names the column on its context
row; **`Tab` on a board card opens the ticket's description in the SAME dialog, grown out of that
card** (T-163, 2026-09-03 — it took the key from the `needs you` attention walk, which is gone),
and there `Shift+Tab` still sets the ticket's workspace (`SetWorkspace`, at once) while no session
or worktree has locked it (`Ctx::workspace_open` mirrors the daemon's `set_workspace` lock — since
T-309 that is a SUBJECT fact, and the board and the ticket page carry the same key; below). The
editor's surface is the SCREEN's: over the board it is the dialog whatever it holds (`n`/`N` too),
from the ticket page it takes the whole screen. `n`/`N` open a note. **`^s` saves and leaves the
dialog, either way** (2026-09-04, user request): on a note the body is written and the editor
closes (a clean one just closes); composing, the description is kept and the dialog folds back
into the one-line composer (`InputPurpose::Create { description }`, `App::fold_composer`; `Tab`
reopens the editor on it, Enter mints it as `notes[0]`). **`^S` (ctrl+shift+s) saves, leaves and
puts the words in front of claude**: composing, it is the one-line composer's Shift+Enter in the
bigger room (mint, write the description, spawn claude with the title submitted, stay on the
board); on a note it follows who is on the ticket — a paned claude is told (`NoteToAgent`,
mesimon's own sentence, human gesture only; `Ctx::editor_claude_paned`), a ticket with NO claude
gets one started on the title like a new ticket (`Ctx::editor_seat_empty`), a Sleeping claude
leaves it inert (`c` wakes it). The editor's own Shift+Enter is a newline. `Key::Ctrl('S')` is off
the legacy floor on Shift+Enter's clause — a legacy terminal sends the bare `^s` — so it is gated
on `rich_keys`. `^g`'s return is the one road that writes and STAYS (`App::write_note`).
(STALE-MAP "The grown composer: `^s` keeps the draft, `^S` mints and asks".) **`^g`
hands the body to the user's own editor** (T-181, 2026-09-03): `$VISUAL`, else `$EDITOR`, else
`vi`, run through the shell on the terminal the TUI gives back for the duration — the focus
handover's road (`tui/src/external.rs`, `lib.rs::event_loop`) — on a 0600 file under
`<state>/edit/`, gone after. What comes back is SAVED at once on a note (the editor's write is the
commit, as for a commit message; through `editor_save`, so an emptied note still asks twice) and
dropped into the draft composing (`^s` still mints). The hint names the program (`^g nvim`);
`App::editor_word` is set in `lib.rs`, never `App::new`, so no test or golden sees a developer's
`$EDITOR` and the key is inert there. `mesimon doctor` prints an `editor` line. (STALE-MAP "A note
opens in the user's own editor".) The ticket page draws the description under the identity line and
lists notes in the rail (`RailRow`); `App::poll_notes` fetches bodies once per `(id, rev)`. Agents
get `read_note`/`write_note` (eight tools now, with `create_ticket`, `tag_ticket` and
`raise_hand`) and
`get_ticket` carries the description. Adding a
field to `NoteMeta` is `#[serde(default)]` like everything else. (STALE-MAP "Notes: files under
the ticket".) **An approved plan is the agent's note** (2026-09-03): the `PostToolUse` frame
`ExitPlanMode` fires carries the plan (`tool_response.plan` since Claude Code 2.1.259,
`tool_input.plan` on 2.1.251–2.1.258 — `plan_of` reads both) and only fires once the user approved
it, so `ingest::plan_of` → `Daemon::record_plan` writes it through `write_note` as `agent:<uuid>`
— one note per session (`SessionRecord.plan_note`), revised on a re-plan, minted afresh after a
delete, `notes[0]` (so the description) on a ticket that had none. (STALE-MAP "An approved plan
is the agent's note".)

**A ticket's notes carry its LINKS, and `^k` opens one (T-256, 2026-09-05).** `core/src/links.rs::extract`
reads a note body for URLs, ticket keys (`T-12`, `board::KEY_PREFIX`) and path candidates —
pure, never persisted, never on the snapshot (derived data on disk drifts from its deriver) —
and `App::ticket_links` resolves them against the live board (`Board::ticket_by_key`, the
ticket's own key excluded) and the disk (a path must be a FILE under the ticket's dir: worktree
when attached, else the repo root). `^k` on the board or the ticket page (`Verb::Links`, both
scopes, `prio: 0`; nothing on screen names it since T-312 — the ticket page's state row hinted
` ∙ ^k links` while a fetched body held one, `?` is the one home now, `!`'s shape after T-277)
fetches the bodies the cache lacks through `Command::ReadNote` (`App::fetch_links`) and opens
`Mode::Links { ticket, links, idx }` / `Scope::Links` (`dialog::draw_links`, the archived list's
shapes plus `c copy` and `^k` as `Back`); nothing to list is `no links in T-12`, never an empty
dialog. `^K` (`Key::Ctrl('K')`, in `OFF_FLOOR` on `^S`'s clause, `rich_keys`-gated) opens the
first with no dialog. Opening: a URL → `App::pending_open` → `lib.rs` → `opener::launch`,
DETACHED (null stdio, reaped on a thread; ladder `MESIMON_OPEN` → `open` → `wslview` → `xdg-open`,
`App::opener` set in `lib.rs` like `editor_word` so no test finds a browser; status `opening …`,
never `opened`); a text file (git's NUL rule at open time, `links::looks_text`) → the `^g` road's
editor via `external::open_argv` on `pending_attach` (`+LINE` for vi's family/nano/emacs/micro,
cwd = the file's dir so the exit status is not judged); any other file → the opener; a ticket →
the board cursor, or its page from a page or when archived. `doctor` prints `opener`. **And
the ticket's LATEST AGENT WORDS are the second body (T-307, 2026-09-07):** `App::latest_words`
is `peek_cache.peek(path)?.text` — exactly what the card's peek row and the page's PREVIEW show,
so what can be read can be opened and nothing is listed from a part of the transcript nobody can
see — appended AFTER the notes (a reply rewrites itself every turn; `^K` must keep meaning "this
ticket's link", and dedup by target keeps the note's row and its label). The transcript is
`App::latest_transcript`: `Board::pane_target` while a pane lives, else the ticket's newest
claude record by `state_changed_at`, because what an agent said last outlives its pane; the
rail's selection is deliberately not consulted. The key's gate is `Ctx::ticket_linkable`
(`App::ticket_linkable` — a description OR a transcript, board-only, no disk read), which is
what `ticket_described` became. (STALE-MAP "A ticket's notes carry its links" + "`^k` also lists
what the agent just said".)

**A paste is ONE event, and only a text field takes it.** `init_terminal` arms bracketed paste,
`App::tick` routes `Event::Paste` to `App::on_paste`, and `EditBuffer::paste` flattens it to one
line (newlines are spaces, never Enters) under the field's byte `limit` — the same number the
daemon caps the text at (`board::TITLE_MAX_BYTES` 2 KB via `sanitize_title`, `TAG_MAX_BYTES`,
`command::PROMPT_MAX_BYTES`); a cut says `paste trimmed ∙ … holds at most …` in the status. A
new text field passes its limit to `EditBuffer::new`. `TextArea::paste` is the multi-line
counterpart (the note editor's body): newlines kept, CRLF normalised, same byte limit. (STALE-MAP
"A paste is one event".)

**Git, in the daemon,** is `crate::git::git(repo)`, never `Command::new("git")`: it scrubs the
`GIT_*` targeting variables a dogfooding daemon inherits.

**A daemon-side change** is not running until the daemon restarts: press `U` in the TUI, or
kill it (the rebuild trap, below). **The daemon keeps a journal** (2026-09-05): `<state>/daemon.log`
(`daemon/src/journal.rs`) — `started`, `stopping: <why>` (SIGTERM, or `shutdown asked by <Hello
client>`), `stopped`, and `slow turn: <message> took N ms ∙ slowest stage <on_tick step>` for any
writer turn past 1 s. It is the first file to read when a board went quiet; `doctor` prints its
last stop. The feed is what the board did, the journal is what the process did (STALE-MAP "The
daemon keeps a journal").

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
— optionally — re-asked every 3 s from inside `App::tick`, so an OS appearance flip repaints the board live —
`detect::GroundWatch`, **OFF by default since 2026-09-02** (`MESIMON_GROUND_WATCH=1` arms it): a late
reply to the periodic query kept typing into the board and opening rename, STALE-MAP "The ground
watch is opt-in"; the terminal is the authority, never the OS, and `MESIMON_THEME`, a
terminal that cannot answer, a waiting keypress and an open text field each disarm or defer the
query; STALE-MAP "Light/dark follows the terminal, live"; and a reply that comes back AFTER the
150 ms budget lands on stdin as keystrokes — `tui/src/osc.rs::ReplySwallow` recognises it on the
raw crossterm event ahead of the keymap and the text-field barrier and discards it, STALE-MAP "A
late reply to the colour query is caught before it can type"),
the token themes for all five colour profiles (`theme.rs` — graphite and chalk, then blue, amber
and green since 2026-09-02; the colour-law tests in it are the palette's spec), pure board geometry (`layout.rs`, post-D33k arithmetic), card
anatomy per 07 §4 (`ui/card.rs`), spines + the minted cursor-column treatment (`ui/board.rs`),
and the ticket screen skeleton (`ui/ticket.rs` — replaced `Mode::Pick`; zero daemon changes).
Deviations recorded in STALE-MAP's "M3.5 implementation deviations". Rendering goldens live in
`mesimon-tui/testdata/golden/` — `MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui` regenerates
after a deliberate visual change; review the diff by eye. `MESIMON_THEME`/`MESIMON_COLOR` force
flavor/profile. Remaining polish (decay, animation, banners, density ladder, keymap validator)
stays in M6. One hard visual rule: exactly ONE saturated colour on the board, reserved for
needs-you (`Theme::attn`; `test_attn_provenance*` enforces it), nothing else ever.

**The chrome after T-158 (2026-09-03): framed dialogs, a screen chip, a painted footer, one
place per hint.** Every floating surface (menu, theme picker, archived, external drawer, `?`,
tag picker, the composer dialog) draws through `tui/src/ui/dialog.rs::frame` — `╭─ TITLE ─╮ …
╰─ its keys ─╯` in `dim3`, ascii `+-|` — which records the rectangle on `App::frames`;
`test_no_drawn_structure` admits a box glyph ONLY on a recorded perimeter (plus `▀`), so L1's
"no drawn structure" has exactly one allowlisted role and the board, cards and pages stay
painted. A dialog's keys are its scope's LEFT footer cluster (`keymap::footer_split`) set into
its bottom edge; the footer under it shows only the mode chip and the right cluster. The header
(`chrome::draw_header`, one function for every screen) opens with a chip naming the screen —
`BOARD` / `TICKET` / `DIFF` / `NOTE` — and the footer's mode word appears only when a mode has
taken the keys (`DELETE`, `TAG`, `NEW`, `ASK`, `MENU`). The footer is a band on `selected_bg`
with `key word` pairs (`chrome::hint_spans`, key bold — 06 §5.1 clause 3); `Group::App` items
(`esc menu` prio 254, `? keys` prio 255) form a right-aligned cluster, and `? keys` appears only
where `?` resolves (never in a text field or chord tail; `footer_always_keeps_the_help_tail`).
Hints sit where they operate: the ticket rail's trailer rows carry `c s x` and `N`, the PREVIEW
heading carries `{ } page` — those bindings are prio 0 on `Scope::Ticket` and reached by
`keymap::binding_for`. The ticket page has its own title row (bold; `r` edits it there) and a
state row (`IN PROGRESS ∙ 3d here ∙ created 2w ago ∙ tags ∙ branch`); the note editor's header
names its ticket. (STALE-MAP "The UI overhaul".)

**Six themes, picked from the Esc menu, saved in two slots (2026-09-02, user request; solarized 2026-09-03).** A
theme is a `Palette` TABLE in `theme.rs` (truecolor hexes plus hand-authored 256/16/8 forms, the
diff tints, the tag ring, and `shadow`, the colour `faded()` blends toward) and
`Flavor::palette()` is the exhaustive gate — a sixth flavor does not compile until `palette`,
`name`, `blurb` and `from_name` classify it; the law tests read the table, never a
transcription. The law has five `Kind`s, matched exhaustively in `test_chroma_law`: `Paper`
(graphite, chalk: greys C* ≤ 8.2 plus three registers), `ChromaticGround` (blue: the navy IS a
colour, both ramps stay grey, every register ≥ 60° of hue from the ground, and the fade target
is a NEUTRAL at the ground's lightness — a tint blended into navy takes the navy's hue and ten
tags become one), `Phosphor` (green — restated 2026-09-03, "white text": the ground,
cursor surface, `calm` and cursor bar sit within 15° of `attn`, which IS the phosphor at full
beam, C* ≥ 60; both ramps are neutral and the base is L* ≥ 90; `err` is a red ≥ 45° off the
phosphor so the armed-delete flash is red there; the diff tints exist and the del tint is red;
the tag ring keeps ≥ 35° of hue from `attn` under the ordinary 2x chroma clause — it shipped
ringless for an hour and the author wanted the colours back), and `Ladder` (amber, the same
evening: the glow's clauses with the three dim steps, the bars and both surfaces on the hue too,
each ≥ 8 L* under the beam and less chromatic; only the base step of each ramp is white — the
original monitor toned down, with white titles, which the author asked for after seeing the glow
on amber and kept on green), and `TintedPaper` (solarized, the sixth flavor, same day: canonical
Solarized-light greys and ground, accents darkened to clear 4.5 — paper C* 5–16, ink ≤ 16 and
≥ 90° off the paper's hue, `shadow == bg`; STALE-MAP "A sixth theme, Solarized light"). Wherever the
ground has a hue the fade target `shadow` is a neutral at its lightness, never the ground.
Nvim's `#005faf` cursor line was refused as blue's cursor surface (2.7:1 under a mid-ramp grey;
it is `#2C3590`). `attn_is_its_own_colour` is what keeps `test_attn_provenance*` meaningful on
a phosphor, and both provenance laws now sweep `Flavor::ALL`. The picker is a row of the Settings submenu
(`Verb::ThemePick` → `Mode::Theme`, `ui/themes.rs`, `Scope::Theme` with the menu's three
shapes) whose cursor IS the preview — `App::preview` is the one road every retheme takes, the
watch's included — Enter keeps, Esc puts `App::resting_flavor()` back. The preference is
`tui/src/prefs.rs`: `~/.local/state/mesimon/prefs.json`, one theme per GROUND (`dark`/`light`,
what OSC 11 can say; the watch now reports a `Ground` and `App::watch_flavor` maps it to the
slot; a pick sets the slot the terminal is on). It is a PREFERENCE, the inverse of
`update-check.json`'s rule: a newer schema is read and never written back, garbage falls to the
defaults with a status line, saves merge into the loaded document so a foreign name in the other
slot survives, and `lib.rs` loads it — never `App::new`, so no test reads the developer's file
(`prefs_path` None = never write). `MESIMON_THEME` accepts every name, still pins and disarms the
watch (the ground is still asked ONCE so the picker sets the right slot), and a menu pick
outranks it for the session while the status says it pins the next launch. `mesimon doctor`
prints a `theme` line. (STALE-MAP "Five themes, and the law learns three kinds" + "Themes are a
menu row with two slots".)

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

**The board says where its own checkout stands (T-124, 2026-09-04).** The header hangs
`⎇ main ↑2 ↓1 ∙ 3 changed` off the breadcrumb: `daemon/src/gitstatus.rs::sample` is ONE
`git status --porcelain=v2 --branch -z` fork on a worker thread (the shell-env road:
`queue_git_sample` → `Msg::GitSampled` → `on_git_sampled`, broadcast on a real delta only, an
ask mid-flight queues on `git_wanted`), landing as `Response::Board.git: RepoGit`
(`#[serde(default)]`; unsampled draws nothing, so goldens never see it unless seeded).
`chrome::git_clause` fits it AFTER the suggestion chip is sized — the offer has first claim, the
count drops first, the name truncates to `GIT_BRANCH_FLOOR`, the arrows are never cut — in
`dim3`/`dim2` for the name and **calm** for the arrows (never attn). `glyphs::branch_mark`/
`ahead_mark`/`behind_mark` are the one home for `⎇ ↑ ↓`; the card reads them too. The fetch is
opt-in — `MESIMON_GIT_FETCH=<minutes>` or the Esc menu's `Fetch origin` row (`Command::GitFetch`,
a person's gesture, denied to agents) — and runs on the same worker before the sample, fenced
(`gitstatus::fetch`: `gc.auto=0`, `--no-write-fetch-head`, every prompt door closed, `setsid` +
group kill at 30 s, `GIT_SSH_COMMAND` deliberately untouched). README promise 1 names the
remote-tracking refs it writes. The menu row's detail spells the arrows in words
(`App::git_fetch_note`). The same change made every snapshot field ride one `Snapshot` struct
through `App::absorb` — `shell_env` had been dropped on the refresh road. E2e:
`crates/mesimon/tests/gitstatus_e2e.rs`. (STALE-MAP "The board says where its checkout stands".)

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

**Columns own their automations (T-117, 2026-09-06).** A column is `Column { name, order,
settings: ColumnSettings }` and its NAME is its identity (`Ticket.column`'s foreign key, no id):
`RenameColumn` is a daemon transaction over every ticket file, archived ones included, every
other column's rule naming it, the move gate's memory and the grace band. No code path compares a
column name to a literal after `board::template_settings`, the one seeding table (a fresh board,
and the store's v3→v4 migration of an existing one — `COLUMNS_SCHEMA` 4). `on_working`/`on_done`
ARE automove (`core/src/automove.rs` reads the ticket's column's settings), `train` is the merge
train's reach (`Merge` = candidates + rebase asks, `Rebase` = asks only), `requires_merge` is the
DONE gate, `reclaim` is the sleep/archive offer and `X`/`Z`, `workspace` defaults a ticket CREATED
there by stamping the ticket field at mint (never retroactive), `collapsed` pins a spine, `claude_mode`
rides `--permission-mode` (`inherit` = the user's own `defaultMode`, else `auto`/`plan`/`manual`;
the enum cannot spell `bypassPermissions`; `--permission-mode` is in `resume_argv`'s `owned` list so
a wake re-applies the column), `agent_tools` is a four-rung tier — `off < read < annotate < full`,
`mcp::tier_needed_by` — advertised at spawn as `--tools <word>` on the shim's argv (it lists
`tools_for(tier)`) and enforced in `handle_agent` at EVERY call against the ticket's column as it
stands then, ANDed with `Board.mcp_tools`; `agent_allowed_columns` is empty below `full`. `auto_run`
("start claude on creation") fires from `Daemon::create_ticket` ONLY — a person at the composer —
as `spawn_session(.., Claude, submit_prompt: true)`, feed `auto_run_started` /
`auto_run_refused:<why>` with actor `automation`, `Response::Created { started }` so the composer
starts no second; never on a move, an agent's `create_ticket`, a wake, an unarchive. For that the
brief is read at PASTE time (`retry_pending_submits` reads `description_body` when `Parked.brief`),
so a description written after the spawn still travels, and the composer's workspace rides
`CreateTicket { workspace }`. `DeleteColumn` refuses live tickets (`move its N tickets first`) and
the last column; `SortColumn` is one-shot and `SortBy` is its five orders — newest arrival,
oldest, key, needs-you first, and **`Tag` (T-283), which is the PICKER's row order**: a group's
row is its registry entries as the flat `Board.tags` holds them, `MoveTag` is the only thing that
arranges it, so carrying a tag left in `^t` raises its cards. Axis 1 decides and axis 2 breaks its
ties (a `[u8; 10]` of row indices, compared lexicographically); an axis a ticket wears nothing on
ranks `u8::MAX`, so the untagged sink; `Tag` is LAST in `SortBy::ALL` because the dialog's row
opens on `ALL[0]` and the goldens read `Sort now: newest first`. The six commands are local-only (`agent_allows`),
`Mutate` on the board, barred under `columns_barred`. `mesimon doctor` prints a `columns` line.
**In the TUI** the column HEADER is a cursor position (`App::cursor_row: Option<usize>`, `None`;
an empty column IS its header — `App::on_column_header`, `Ctx::col_header`, both renamed off
`on_header` by T-305, which gave the word a second meaning): Enter opens
`Mode::ColumnSettings` (`Scope::ColumnSettings`, `keymap::COLUMN_ITEMS` drawn by
`menu::draw_dense`, one line a row; the Name row is a text field in place and then the scope is
`Input`), `r` renames in the header row (`InputPurpose::RenameColumn`), `HJKL` moves the column
(`ReorderColumn`), `d d` deletes (`Doomed::Column`), `O` adds one after the cursor's and names it
first (`ColumnSubject::New`); the menu has `Column settings` and `Add a column` rows. A header
under the cursor wears the cursor bar; a column that does something wears ` →` after its count
(`glyphs::auto_mark`, dropped first when tight); a pinned column is a spine unless the cursor is in
it (`layout::board_geometry`'s `pinned`). Goldens `board_header_*`, `board_pinned_120x30`,
`column_settings_*`, `column_add_120x30`, `help_header_120x30`. E2e `column_e2e`, `auto_run_e2e`,
`claude_mode_e2e`, `agent_tools_e2e`. (STALE-MAP "Columns own their automations".)

**And one step above a column header is the BOARD's own top row (T-305, 2026-09-07).** `k` there
sets `App::header_focus` and `App::scope()` answers `Scope::Header` — a cursor position on the
board, not a screen: nothing is drawn over it, the cursor column keeps its painted band (which is
what says where `j` returns to) and gives its bar cell up, and `App::on_column_header` is FALSE
while the row holds the cursor, so the four column verbs stand down through the one predicate
they already read. `App::at_column_header` is the wider question the draw asks (the column shows
its top either way). ONE section is focusable — the checkout's git clause — so `h`/`l` are
unbound, `k` is unbound (nothing is above the top row), and the press is refused where no sample
has landed for the clause to be drawn at all; Enter is `Verb::Act` → `open_checkout_diff`, the
board's own `v` on the section that draws the count it opens, and `j`/Esc walk back into the
column. `chrome::git_clause` paints the focused clause on the elevated surface with a pad cell
each side (the header chip's shape), its greys stepping onto the `sel` ramp while the arrows keep
the calm register, and bold stands in where a profile can neither paint nor reverse. **It stopped
spelling ` v diff` beside the count** (T-221's hint, the ticket's second half): a section the
cursor can stand on says what Enter does in the footer, which is one home for the hint instead of
two — the key still works on the board and `?` still lists it, and the clause's give-way ladder
lost its first rung (the name truncates to its floor, then the count drops). Goldens
`board_header_bar_120x30`, `help_header_bar_120x30`, `board_git_120x30`. (STALE-MAP "The board's
top row is a place the cursor can stand".)

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
OSC Tier A−): a `Running` Claude pane whose `#{window_activity}` goes quiet 60 s demotes to
`Idle{Interrupted}` at medium confidence, demotion-only (`probe_activity` in server.rs) — and
since 2026-09-04 the same row lands in ~2 s off Claude Code's own `~/.claude/sessions/<pid>.json`
(`status: idle` stamped after the Running spell began; `probe_status_files`, STALE-MAP "The
recordless Esc is caught by Claude's own session file"), because a quick Esc writes no
transcript record at all. **But that file flips `idle` at the end of EVERY turn, milliseconds
before the Stop hook fires, so the probe races the hook on every turn** (T-242, 2026-09-05: a
`cargo nextest run` in another session relinked the hook binary, macOS held the fresh unsigned
binary's first execs in syspolicyd's malware scan for 41 s, and a finished simbly turn wore
"interrupted" until the Stop landed). Now the probe reads the transcript tail first
(`tail::turn_done_since` over `adopt::turn_edge`): a closing record — an `assistant` with
`stop_reason: end_turn`, or the `stop_hook_summary` after it; current Claude Code writes no
`turn_duration` — stamped at or after the Running spell means `Signal::StatusFileIdle { turn_done:
true }` → `Idle{EndTurn}` at Medium (automove takes it to REVIEW; a late Stop then commits High over
it); the previous turn's close is older than the spell, so the recordless Esc still reads
`Interrupted`. E2e `interrupt_status_e2e` runs both (STALE-MAP "A late Stop is not an Esc"). **And a
reload mid-tool seeds `Running`** (T-265, 2026-09-06): a trailing assistant record with only a
`tool_use` block is `TailEvent::ToolInFlight`, never `Other`, because a tool in flight keeps the
transcript still for its whole duration and the mtime-quiet rule in `resting_hint` read a 3.5-minute
`cargo` call as a dead turn (STALE-MAP "A tool in flight survives a reload"). After a
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
screen, hinted. Two qualify: `Verb::Nudge` on the board and
`Verb::TagCarryLeft` in the tag picker, and in BOTH `HJKL` and the four Alt atoms share ONE binding
(the board's since 2026-09-04, user request; `> <` is the aiming move beside it, overlay-only) —
the strongest form of the clause, since the accelerator cannot reach a move the floor does not.
A text field never sees an Alt atom at all: `keys::to_key_text`
strips the modifier, because there Alt is `word_wise`'s "by word" and nothing else — which is
why `key_tag` reads `to_key` while steering and `to_key_text` only while naming, and why an
Alt atom (or the `˙` a terminal composes instead) never dismisses the picker as a stray key
would. A third off-floor atom needs one of these two clauses, argued — not a third one: the third,
`Key::Ctrl('S')` (2026-09-04, the grown composer's mint-and-ask), took Shift+Enter's — the bare
control byte a legacy terminal sends is `^s`, another verb, so it is `rich_keys`-gated and
`ambiguous_atoms_are_inert_without_rich_keys` is the one law both run.
(STALE-MAP "Alt is admitted, for one verb" + "A tag moves".)

**Composing a ticket: Enter saves, Shift+Enter saves and asks.** A fresh Claude spawn always
types the ticket title into the agent's box and stops (zero token injection, a README promise).
The composer's Shift+Enter is the one gesture that also presses Enter — it mints the ticket,
spawns claude, submits the title as the first prompt, and stays on the board (no handover; the
card is how you watch it). **And since T-224 (2026-09-05) the description goes with it**: on every
road where mesimon presses the Enter (`submit_prompt`), `spawn_session` parks `notes[0]`'s body
in `pending_prompt` as a `Parked { brief: true }` and the first tick after `SessionStart` pastes
it under the typed title (`paste_text`, the wake-and-ask shape — never typed ahead), so the first
prompt is the whole brief; the plain-Enter road stays title-only because the user is about to
edit the box. Agents skipped `get_ticket` however CLAUDE.md asked (the snippet is now imperative
and first, and `get_ticket`'s own description says the prompt is often only the title); a prompt
cannot be skipped. The paste, or a `get_ticket`, stamps `SessionRecord.ticket_read`, and the
ticket page's state row says `description unread` for a claude that has taken a turn on a
described ticket without either (`SessionState::has_prompted`). README promise 3 names the paste.
E2e `brief_e2e`. (STALE-MAP "The brief travels with the title".) It travels as `Command::SpawnSession { submit_prompt }` →
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
does a second Shift+Enter (the finger is still holding shift). **On a ticket whose claude seat is
EMPTY the board's press is the composer's second half a press late** (2026-09-03): the same
`Verb::Prompt` binding, and `dispatch` routes on `Ctx::ticket_has_claude` to `start_composed` —
claude spawns with the title submitted, no field, no attach, hint `ask claude the title` — **unless
another claude is working in the same checkout, where the field opens at `queued` instead** (T-294,
below). A
`Sleeping` claude is not an empty seat and the key WAKES it and asks (2026-09-04, user: "ask claude
on sleeping agent auto wakes it for the user"): the field opens as on a paned claude (hint `wake +
ask claude`), and `Daemon::prompt_sleeping` — the road `prompt_session` takes when `prompt_target`
finds no pane — runs `resume_session` (its guards intact, `Spawned { fresh }` back so the status can
say `woke claude ∙ asked` or that a fresh conversation started), parks the words in the in-memory
`pending_prompt` map and sets `pending_submit` (the launching arc); the `SessionStart` edge —
`Startup` OR `Resume` now — starts the retry clock without pressing, and the FIRST tick pastes the
words through `paste_text` (bracketed paste + Enter, the live-pane shape; never typed ahead — a pty
in canonical mode keeps 1 KiB), with the later ticks the ordinary Enter retries until the
`UserPromptSubmit` ack. A restart drops the parked words like it drops `submit_retry`. `prompt_e2e`
drives it with a 2.5 KB prompt (its stub runs `stty -icanon`: a canonical tty keeps 1 KiB of a
line). The same test found the wake racing the sleep's own `pane-died` — the notify names only the
sid16 the new pane reuses — so `Daemon::pane_reborn` drops a death frame for a `Spawning` record
whose pane tmux lists alive (STALE-MAP "The ask at a sleeping claude wakes it"). A shell on the
ticket does not fill the seat. `shift_enter_asks_claude_at_every_stage`
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

**And the ask can WAIT for a quiet checkout (2026-09-04, after five claudes in one checkout
committed at once).** Shift+Tab inside the ask field flips `now` / `queued` on the row under it
(`card::render_ask_mode`, composer-style; the one `BackTab` binding in `Scope::Input`, widened —
`Ctx::ask_queueable`: a shared-checkout ticket, never a worktree, whose checkout is its own). A
queued ask rides `PromptSession { queued: true }` into the daemon's in-memory
`queued` list and is delivered by `drain_queue` when `checkout_holders(cwd)` is empty —
`core/src/quiet.rs::working_tickets`: no claude with the same `cwd` Spawning / Running /
RequiresAction / Idle{Background} / `pending_submit` / a paste of ours still owed its ack
(`Daemon::inflight`); a shell never counts — hooked beside `auto_move` in `apply_change`, on the
1 s bucket, and at enqueue (a quiet checkout sends at once); one per checkout per pass, **in
BOARD order — column order, then top to bottom, the merge train's walk** (`Daemon::queue_order`,
T-263, 2026-09-06: the user sorts the queue by moving the cards; `waits_on` names the holders and
then the asks ahead, so the row's `+N` falls as a card rises), one per ticket. It is DROPPED by any `UserPromptSubmit` on the ticket while it waits (the daemon
cannot tell its own paste's ack from the user's keystroke, so the next prompt closes it either
way), by sleep / kill / delete, by the sweep (target gone, replaced, parked, ticket archived) —
never by a hand move. The snapshot's `pending: Vec<Pending>` (kept general: the train's rows ride
it) prefills the field on the next Shift+Enter (Esc keeps, a blank Enter drops via
`DropQueuedAsk`), feeds the card's owed mark (`glyphs::queued`, slow cadence, over still marks
only — `queued_over`) and the cursor card's `queued ∙ after T-12` row (`App::pending_row`, the
snooze row's slot; the ticket page's state row reads the same). A restart drops the queue like
`pending_prompt`. E2e `ask_queue_e2e`. (STALE-MAP "The board's ask can wait for a quiet
checkout".)

**And what waits may be the SESSION (T-294, 2026-09-06, user: "shift+enter on non started sessions
should ask if now / queued when there is a running session").** The two roads that skipped the gate
were the two that add a WRITER to the checkout rather than asking the one already in it: an empty
seat spawned at once, and a `Sleeping` claude was woken at once (`enqueue_ask` refused both — "a
queued ask needs an awake claude"). Now `Daemon::seat_of` answers `Pane | Wake | Start` and
`Daemon::deliver` is the ONE road every ask takes — a send-now `PromptSession` and `drain_queue`
alike, so a queued ask and a sent one can never disagree about what "the ticket's claude" means:
paste, or `prompt_sleeping`, or `spawn_session(.., submit_prompt: true, prompt)`, whose words ride
UNDER the brief in `Parked` (`retry_pending_submits` composes description then user, and
`spawn_session` takes the prompt so `pending_spawns` can replay it after provisioning). A `Start`
entry's `text` may be EMPTY — there the prompt is the ticket's own title, so `sanitize_prompt`'s
blank refusal is lifted for that seat alone (the Enter lands on the title the spawn types, which is
a turn the user did write). The entry remembers its seat and `seat_stands` drops it when the seat
changed — a different pane, a killed record, or a claude somebody started by hand where a `Start`
was waiting — never redirecting; `Pending.action` carries the seat's word (`ask` | `wake` |
`start`, `Pending::is_queued_ask`) so the card says `starts ∙ after T-3` rather than that words
wait. **In the TUI the field opens only where waiting means something**: `Ctx::checkout_busy`
(`App::checkout_busy`, the TUI's own read of `quiet::is_working` over shared-checkout sessions plus
the snapshot's `in_flight` rows) is a HINT — it decides whether the press stops to ask, never how
the words are delivered — so a quiet checkout keeps the one-key start and the hint `ask claude the
title`, and a busy one opens the field at `queued` with the hint `start claude`. A live PANE keeps
`now` either way: it is one turn in a conversation already there, and the press may well mean
interrupt. A blank Enter drops a waiting entry as it always has (T-241) EXCEPT on an empty seat
with the toggle moved to `now`, which is how a queued start — whose field is empty by nature —
jumps its own queue; the placeholder says which (`start on the title` / `enter drops`). E2e
`ask_queue_e2e::a_queued_start_waits_for_the_checkout_and_then_spawns_a_claude`; `prompt_e2e`'s two
"no live claude" refusals are now starts. (STALE-MAP "What waits for the checkout may be the
session".)

**Tags are ticket metadata on an axis, and `^t` opens a picker.** `Board.tags` is the registry
(`Tag {name, group, color}`), persisted in `columns.toml` (schema 2 — the bump exists so an older
build bars its writes instead of dropping the registry); `Ticket.tags` is `Vec<TagRef {name,
group}>`, a pointer into it. Colour lives on the REGISTRY, never on the ticket, so recolouring
repaints every card at once instead of leaving 40 tickets holding a stale copy. **A board with no tags at
all is offered three starters ONCE** (2026-09-04, user: "creating first tag gets people
overwhelmed"): `board::STARTER_TAGS` — `BUG` `FEATURE` `CHANGE` on group 1, rose/green/blue as
chosen colours — written by `Board::seed_starter_tags` from `store::load` when
`columns.toml` says the offer is still owed (`tags_seeded`, a scalar before the tables; absent
on every older file, so an existing board with no vocabulary gets them on its first load and one
with its own is only stamped). Forgetting all three is respected: the stamp is what keeps them
from coming back. `MESIMON_NO_TAG_SEED=1` declines it (the seam `tags_e2e`/`mcp_e2e` set,
since they build a registry from nothing). Beyond that nothing is seeded:
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
**`▀` U+2580 is the FIRST admitted exception** (the second, `▎` U+258E, is the ticket page's
description bar — `Theme::desc_bar`, 2026-09-03, its only producer) — inside the `0x2500-0x259F` range the L1 law bans
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
into graphite, up into chalk — receding on either flavor) and the cursor picks the level: `Selected`
is the full tint (the cursor card carries the loudest tags on the board), `Rest` is every other
card, one step down. **Two levels, not three** (author 2026-09-02): a quieter `Sleeping` level for
a parked ticket (0.38 graphite / 0.46 chalk, read off the sessions) shipped 2026-09-01 and was cut
the next day as too muted — the glyph already says asleep, and the block's one job is "which tag".
The first cut used 0.82 and **the boundary was not visible on a real board** — an 18% blend is
nothing on a one-cell block — so the law test asserts the step is ≥ 12% of the ground-to-tint
distance. **The two flavors need different constants to mean the same thing**: the blend is a
ratio in sRGB bytes and the same ratio costs far more toward WHITE than toward black, so graphite
is 0.70 and chalk 0.76 (`Tints::fade`). Under one number, chalk's resting tint landed at C* 16.8 /
contrast 2.82 where graphite's landed at 21.7 / 3.61 — the other half of "barely visible on light
theme"; with the darker ramp and the gentler step chalk's rest is C* 17.9 / k 3.77. **The ladder
is the CARD's, not the palette's**: an untagged card's neutral block dims and brightens exactly
the same way (`tags::bar_cell` fades the neutral bg too), because a board where only tagged
tickets answer "is this the cursor card?" answers it for some cards and not others — that is the
bug that shipped twice. The law test holds `Rest` above the dim2 body floor, C* ≥ 8 and distinct
per tag. Below TrueColor the levels collapse: one grey, no ground to fade into.

**The peek names them** (`tags::chips`): with `p` on, the cursor card carries one row of painted
name-chips under the title, above the reply. Colour says how many and which hues; only words say
which tag, and the stripe only has room for two. Several tags share the row longest-gives-first,
never an equal split (an equal split cut "BUG" to make room for a "STAGING" that then got cut
anyway), nothing shrinks below three cells, and the tail drops rather than every name going
illegible. The row is the TICKET's metadata, so an open card earns it with no session at all —
gating it on `peek.is_some()` meant the commonest tagged card on the board could never show it. **`P` opens every card** (T-237, 2026-09-05): `Verb::PeekAll`, board only,
overlay-only like `p` (user: "no need to hint this") — `App::peek_all` implies `peek`, `P` off
narrows back to the cursor card, `p` off takes both; a resting open card draws only its chips and
its reply on the resting ramp with no surface (`card.rs`), the session list stays the cursor
card's. The ticket page's `P` (the session pin, `PinAwake`/`pinned_awake`) was REMOVED the same
hour at the user's ask (STALE-MAP "`P` opens every card, and the pin is gone").

**And a quick-tag digit opens the card it tagged, for 1500 ms.** The stripe is one cell at rest
and carries no words, so `cycle_tag` arms `App::tag_flash` and the card draws itself open —
chip row, and the tall stripe's 70/30 split. `App::peek_showing(ticket)` is the seam (`p` OR a
live flash) and `ui/board.rs` is its only caller. Keyed to the ticket, so `j` ends it; every
repeat re-arms it, so walking an axis keeps the card open for the whole walk; a refusal arms
nothing. `Ctx::peek_on` stays the PREFERENCE — `p` keeps hinting `show replies` under a flash,
because the footer describes the toggle and a flash is not a state anyone toggled. (STALE-MAP
"A quick tag opens the card it tagged".)

**And the done mark decays once seen (T-173, 2026-09-04).** A finished agent's mark is the
heavy `✔` (`glyphs::done_unread`) in the calm register while the reply it stands for is one the
cursor has not been on the card for, and the thin `✓` on the grey ramp the moment the cursor
lands or the ticket page opens — `card.rs` swaps the glyph and demotes `Register::Calm` to
`Grey` on `App::spoke_unseen`; the cursor card and the move ghost are always drawn seen, the
rail stays calm, mono reads `+` either way. `✔` carries the Emoji property (narrow text on
iTerm2; `done_unread` is the one place to fall back to a bold `✓`). No cell spent: a `◊` beside the title
shipped first and was cut the same day (author: "too big, and with the worktree mark it takes
too much space"); line 1 has no room for a second right-hand mark. The trade is that only an
end-of-turn reply shows — a sentence under a spinner has no channel. The source is the
transcript's assistant RECORD (`peek::Peek::reply_key`, a hash of its `uuid`) — not pane bytes,
not the words, and never the user's own prompt, which leaves the key `None` and reads as
"nothing new". State is TUI-local: `App::spoke` holds a `Spoke { session, path, key, seen }` per
ticket; `scan_ticket` reads `Board::pane_target` (the one paned claude), starts a fresh entry
(reply unread) when the session or path differs, and drops the entry when there is no such
session; `poll_spoke` in `App::tick` scans the departing card the tick the cursor leaves it, the
board every `SPOKE_EVERY` (1 s), and acks the subject every tick — a redraw, never a snapshot. A
fresh board finds every reply unread, which is how `✓` always looked. `PeekCache` is one entry
per path for this (pruned against every session's transcript; the cost is in peek.rs's module
doc). No key, no `Ctx` field. (STALE-MAP "The done mark decays once seen".)

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

**And `{ }` (or `pgup`/`pgdn`) page it**, the diff's keys on the diff's verb, routed by
`App::dispatch` on the screen. Hinted only while the zone overflows: `Ctx::preview_scrolls` reads
`App::preview_view`, which the DRAW writes (the zone's height is a fact of the frame), and the
request `App::preview_scroll` is keyed to the document (session + reply text, `ticket::doc_key`),
so a rail move or a new reply starts at the top. A shell tail defaults to its bottom and is
released back to following when `}` reaches it. **A press GLIDES** (2026-09-04): the record moves
at once, the rows follow over `GLIDE` (= `GROW`, one speed of motion on a screen) through
`App::preview_glide` / `Glide::offset`, keyed to the document and retired by the draw; a press
mid-turn starts where the eye is. Tests page through `page()`, which presses and settles.
(STALE-MAP "The preview zone pages" + "A page turn glides".)

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

**A turn that starts without a prompt shows on its first tool frame (T-228, 2026-09-05).** A
`!` bash command in Claude Code puts its output into the conversation and the model takes a turn
on it, and NO `UserPromptSubmit` fires — the feed showed a High `Idle{EndTurn}`, then three
minutes of `PostToolUse` frames the machine held inert by the "a background task's completion
must not flip a real end_turn" rule, and the ticket sat in REVIEW with no working mark until a
`PermissionDenied` happened to promote it. T-135 had already measured that the guarded frame
does not exist, so now the session's own non-nested `ToolCompleted` promotes ANY `Idle` to
`Running` at High (automove brings the card back to IN PROGRESS); a nested one still says
nothing about the lead. The mark lags the first tool's own duration, since the observer hooks
no generic `PreToolUse`. (STALE-MAP "A turn that starts without a prompt".)

**A ticket holds ONE claude, and the second seat is a shell** (2026-09-02). `spawn_session`
refuses a `Claude` spawn when `Board::live_claude(ticket)` finds one (`is_live`, so a parked one
holds the seat); resume and wake re-enter an existing record and are not gated, so older boards
keep what they have. `C`/`Verb::ClaudeNew` is gone, `S`/`ShellNew` stays, and `c` on a parked
claude hints `wake claude` and wakes it (`focus_session` resumes a paneless record before it
attaches). Everything that picks "the" agent of a ticket — `pane_target`, `board_enter`,
`auto_move`, `card_glyph`, the worktree lock — assumes one; with two they picked the first in
spawn order and automove ping-ponged the column between their turns. (STALE-MAP "One claude per
ticket".)

**And the rail OFFERS that one seat, while the ticket's own shell is gated (T-300, 2026-09-07).**
`RailRow::NewClaude` is a phantom rail row — `+ claude session`, no record behind it — drawn after
the sessions and before the notes, so a ticket with none opens with the cursor ON it (the user's
ask: focus this even where there is a description) and `Enter` spawns through `spawn_and_focus`,
the road `c` takes. Sessions stay first because a position in `rail_sessions` IS a `rail_idx`
(`board_enter`, the focus return). It stands exactly when the press would work — `App::new_claude_row`
mirrors the daemon's two refusals: `live_claude` holds the seat (parked counts) and an archived
ticket may not grow a pane — so a resumable corpse shows both rows (`enter` resumes that
conversation, the row starts a new one) and the archived page finally offers nothing. `c` on the
ticket page keeps ONE word, `wake claude`, and is otherwise silent-but-bound (`binding_for` drops
an empty hint): a listed row and this row are both a second spelling of the press. `jk` gates on
`Ctx::ticket_rail_rows > 1` now, not on sessions — a rail of notes was unwalkable before, and the
hint says `select row` where there is no session — and one row is not a list, so the key is inert
there. The SHELL is gated, not removed (user: "keep the feature but gate it for now"): the daemon
still spawns `Bash`, the rail lists one, `PaneTail` previews it, `x` sleeps it, and every e2e is
untouched — what is shut is `s`/`S` on the ticket page and the board's overlay-only `s`, on
`Ctx::ticket_shells`, opened by `MESIMON_TICKET_SHELLS=1` (read in `lib.rs::run`, never
`App::new`, the `editor_word`/`opener` rule) with a `doctor` line to find it by. `!` is unaffected:
the project's terminal is a place to stand, not a session of the ticket. Golden
`ticket_new_claude_120x30`. (STALE-MAP "The rail offers the session, and the ticket's shell is
gated".)

**And the PREVIEW zone beside that row previews the SESSION (T-308, 2026-09-07).** It drew nothing
there — `draw_preview`'s chain is shell, then note, then reply-or-working, and the offer is none of
the three — so the one row whose whole purpose is an unmade press sat next to the emptiest half of
the screen. `ticket.rs::empty_seat` draws a mark, the press through
`binding_for(Scope::Ticket, Verb::Act)` + `chrome::hint_spans` (the footer's own binding, so the two
cannot disagree), and `seat_rows`: `starts in a worktree of its own` / `in the checkout` — never the
branch, which the state row four lines up already carries — plus the column's `claude_mode` and
`agent_tools` ONLY where they differ from a spawn by hand (`Board.mcp_tools` off reads as
`AgentTools::Off`); then `types the ticket title into its box, and sends nothing`, which is this
road's contract (`submit_prompt: false`, so no brief travels — the one place the difference from the
composer's Shift+Enter is visible); then, under `App::checkout_busy` only, `another claude is already
writing in this checkout`, since this road does not queue and the press adds a second writer
(T-294's hazard, at the press that causes it). Every fact is already on the page: no `Command`, no
`Snapshot` field, no `Ctx` field, no key. **The art is redrawn, never borrowed** — Claude Code's own
welcome is Clawd and a starfield in `█ ░ ▒ ▓` and the quadrants, sixteen rows tall: that range is
exactly what `test_no_drawn_structure` bans, the height does not fit, and it is somebody else's
brand art. `SPARK` is five rows of hand-authored ASCII (one drawing for all four glyph tiers, and
both L1 sweeps render it), greyscale — star on `dim1`, spokes `dim2`, field `dim3`, the three
brightnesses `░ ▒ ▓` gave them — with no hue (the one saturated colour is needs-you's) and no
motion. Under `SPARK_MIN_H` (12) rows of zone the picture goes and the words stay; the mark is
centred over the TEXT block, not the zone, which is 87 cells wide against ~55 of sentence. Goldens
`ticket_new_claude_120x30`, `ticket_new_claude_worktree_120x30`. (STALE-MAP "The empty seat previews
the session it would start".)

**And the same zone reports a SESSION with nothing to read (T-308's second half).** The chain was
shell / note / `reply.is_some() || working`, so a selected session with no readable reply drew
nothing — a claude coming up, one waiting for its first prompt, a corpse or sleeper whose
transcript is gone, an `Unknown` after a restart, a live shell before its first `PaneTail`. It is
`reply` / `record` / `seat` now (`draw_preview` takes the `&SessionRecord`, not its uuid, and
`working_row` was lifted out to serve both the reply's closing indicator and this arm's headline).
`ticket.rs::quiet_words` is PURE over the record — testable frameless — and reuses
`glyphs::state_word` wherever nothing better is true: this zone invents no second name for a state
the rail already names. Before the first turn the fact is the BOX — `waiting for you ∙ the ticket
title is in its box, unsent`, or `starting up ∙ … ∙ mesimon presses enter when it is ready` when
`pending_submit` — gated on `Idle { stop_reason: Unknown }`, the one `Idle`
`SessionState::has_prompted` refuses (an `EndTurn` with no words is a finished turn, not a fresh
box, and keeps `done`). A turn in flight gives the row to the PULSE plus `nothing said yet ∙ the
first words land when the turn does`, and the mark is not drawn there — it stands only while the
conversation has not STARTED (`Spawning` or that `Idle`, no transcript, claude), so it is the empty
seat's face one press later and the zone does not blink between the press and the first prompt.
A needs-you session's headline is its own QUESTION in `attn_text`, wrapped to three rows: the rail
cuts it to 26 cells, this has 87 — the card-versus-page split, index truncates and reading surface
reads. A sleeper with no transcript says `waking it starts a fresh one` BEFORE the press
(`resume_session` mints a new uuid there); a shell says `reading its pane`, since a shell keeps no
transcript by design. **No press row** — unlike the seat's, a session row is already spelled by its
own `enter resumes` badge and the footer. Golden `ticket_starting_120x30`, plus twelve ticket
goldens that stopped being blank. (STALE-MAP "And the session with nothing to read yet".)

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

**A ticket can be snoozed, and a snooze IS an archive with a deadline (T-74, 2026-09-04).**
`z` on the board or the ticket page arms `Scope::SnoozeChord` (word `SNOOZE`, a barrier like
`d`/`a`): `z` again walks the fixed ring `1h · 4h · tomorrow 9:00 · next Monday 9:00`
(`core/src/snooze.rs::Preset`; the last rung's day is the **week start** preference,
`snooze::Weekday` Monday/Sunday/Saturday, `prefs.json::week_start`, the Settings row `Week
starts on …` cycles it — `Preset::label(week_start)` stays a literal per day), Enter snoozes, Esc or any stray key cancels; the armed card
draws open with the preset on its own row and the status names the ring, never the clock (the
golden is deterministic; the confirm status says `snoozed T-9 until 15:42 ∙ u undoes it`).
`Command::SnoozeTicket { id, until, needs_you }` writes `Archived { until: Some, needs_you }`
through `archive_ticket`'s gates plus "already past" — so the ticket leaves through
`Board::column_tickets`, the ARCHIVED row reads `wakes in 3h`, `a`/`u` restore it (= cancel).
**Where the archive refuses over an awake session, the snooze sleeps it first** (2026-09-04):
what `x` would take (`sleep_eligible`, no age floor) goes through `sleep_one`, all-or-nothing
and judged before any is signalled; a claude still working holds the ticket on the board
(`claude still awake — only idle sessions sleep`; `App::snooze_blocked` says the same at the
first `z`, so the chord never arms for a refused Enter). The sessions stay parked when the
ticket returns; `c` wakes them. **The armed card blinks** — the title on `Theme::move_blink`,
the move ghost's clock, board card and ticket-page title row alike; the delete's red flash is
a deletion's.
`TICKET_SCHEMA` was bumped to 3 for it (a v2 build would drop `until` and the ticket would sleep
forever; it is 4 since T-227's `manual_merge`). The
calendar rungs are pure arithmetic over `LocalTime` in `struct tm`'s conventions with the libc
(`localtime_r`/`mktime`, `tm_isdst = -1`) in `tui/src/localtime.rs`. **The wake is the tick
wheel's** (`Daemon::wake_snoozed`, the 1 s bucket): back at the TOP of its column
(`Position::Top`), `entered_at` restamped, the move gate forgetting it, feed line `snooze_woke`,
no broadcast of its own. **A woken ticket with `needs_you` sets `Ticket.woke_at` — one of the two
TICKET-level producers of the saturated colour** (the other is T-107's raised hand): `card::render` wraps `card_glyph` with `!` in
`Register::Attn`, `card::needs_you(ticket, sessions)` feeds the badge and the spine, and
`Board::needs_you_count()` is the header chip's AND the tmux status line's number. The mark
comes off on a KEYPRESS that leaves the cursor on it (`App::ack_woke` at the end of `on_key` →
`Command::SeenTicket`), never on the draw clock — a ticket wakes while the user is away and a
parked cursor must not clear it. The preference (`prefs.json::snooze_needs_you`, default on) is
the Settings submenu's `Snooze returns with needs-you / quietly` row; `App::save_prefs` is every
preference's write. E2e: `crates/mesimon/tests/snooze_e2e.rs`. (STALE-MAP "A ticket can be
snoozed".)

Board-wide actions (external drawer, archived list, sleep-all, archive-all) deliberately have
NO key, bar the three the header itself teaches (`U` reloads, `X` sleeps the done agents, `x`'s
own shift widened to the column, since 2026-09-04, and `!` the project's terminal since T-273 —
all overlay-only, so the footer stays the selection's) — they live in the Esc menu (`ui/menu.rs`,
rows from `keymap::menu_items`), because they are rare, are not about the selection, and a menu
row has room to say what it will do. The board's footer names the door — `esc menu`, in the
right cluster beside `? keys` (T-158).

**Suggestions are pointers at menu rows, never their own surface.** `keymap::SUGGESTIONS` is a
priority-ordered list (update ready > sleep N agents > archive N tickets); each entry's
availability IS its menu row's `avail`, so the header cannot offer what the menu will not do,
and `menu_items` floats the suggested rows to the top in that order. The header shows exactly
ONE — right-aligned, `(esc)` or `(U ∙ esc)`/`(X ∙ esc)` for the route, no count of the rest — `◦`
marks both the chip and the rows it stands in front of. To add one: add the menu row, add the
`Suggestion`, done. (STALE-MAP "Suggestions are one right-hand chip and a marked menu".)

**The preferences are one level down, behind the menu's `Settings` row (2026-09-04, user
request).** A menu row is an action or a door, never a toggle: `keymap::SETTINGS_ITEMS`
(theme, agent replies, where the tmux status line sits, how a snooze returns, the week's first
day — `MenuItem`s, so `ui/menu.rs::draw_list` draws both lists) is behind `Verb::Settings` → `Mode::Settings` / `Scope::Settings` (word
`SETTINGS`, the menu's three shapes, `esc back`). Choosing a settings row KEEPS the list open
— the row relabels itself — and the theme picker pops back onto its row on Enter and Esc
alike; Esc from the list lands on the menu's `Settings` row (`App::menu_row` /
`settings_row`). No settings row is ever a suggestion (`every_suggestion_is_a_menu_row` holds
the two lists apart), and the row's detail names the current theme so the door says what is
behind it. Golden `settings_120x30`. (STALE-MAP "The preferences move into a Settings submenu".) **The
status line's side is one of those rows (T-264, 2026-09-06)**: `Status line at the bottom / top`
→ `prefs.json::status_line_top` → `Command::SetStatusLine { top }` (denied to agents) →
`TmuxBackend::set_status_position`, which re-renders the conf for the NEXT server AND
`set-option`s the live one; the snapshot's `status_top` is the daemon's word and
`App::reconcile_status_line` pushes the preference whenever they differ, either way, on the
train's back-off. E2e `status_line_e2e`. (STALE-MAP "The tmux status line can sit at the top".)

**The board says it out loud, and only while it is open (T-282, 2026-09-06, user: "notifications
(OS + sound effects)").** D15 said never build a notification channel and ship `watch --json`
instead; that primitive never shipped, so the quiet was total. What is kept of D15 is its
reasoning — default OFF, coalesced, said by the CLIENT, quiet while you are looking — and what
is not is its conclusion. Two rising edges, read off each snapshot against the last by
`notify::Differ` (in `App::absorb` until T-291 moved it to the thread below):
`Board::needs_you_tickets`' three roads are *needs you* — an attention-set session, a snooze
that woke (T-74), a raised hand (T-107) — so the banner and the `!N` chip are one set; the words
beside it are `attention::reason_word` for a session and the AGENT'S OWN SENTENCE for a raised
hand, which is why `notify::Event::why` is a `String` and not the `&'static str` a hint is.
`Idle{EndTurn}` is *a turn finished*, the state automove reads. Two guards: **`EndTurn` only at High|Medium**
(after a daemon restart the tail re-derives every finished turn at LOW — a burst of stale
chimes) and **the first scan only SEEDS** (`Differ::primed`; `U` restarts the process and an
opening board must not announce its backlog). `core/src/notify.rs` is the pure half —
`Differ` (the rising edges, T-291), `Coalescer` (one post per `WINDOW_MS` 5 s carrying the
aggregate, the window rolling from the last thing SAID, so `20 agents finished ∙ T-1 T-2 T-3 T-4
+16`), the wording, the `Sound` ring,
and `Presence`, whose fallback DIRECTION is the design: a terminal that reports focus (DECSET
1004, `EnableFocusChange`) is believed, one that does not falls back to keystroke presence
(`KEY_PRESENCE_MS` 30 s), and no evidence at all reads as AWAY — silence is the failure it
cannot fall into. **`Presence::looking` is the board being ON SCREEN and focused** (T-291,
2026-09-07): the terminal is focused for the whole of a handover too, and the original rule read
that as "you are looking at it" and swallowed every banner in the one case the feature exists
for. Looking suppresses the BANNER only; the sound plays either way, and a Settings
row opts out. **`Presence::watching` is that rule one level finer (T-292)**: inside a ticket's own
claude PANE, that ticket says nothing — banner AND sound, the one suppression that takes both,
because the permission prompt IS the pane and the finished turn IS its last line, so there is
nowhere to go look; the other nineteen agents still speak. `Coalescer::forget(ticket)` is how
(every beat, not on the edge, so a line held from before the attach goes too); the differ still
MARKS it, so detaching announces nothing you already watched. Only an attach to that ticket's
claude counts — a shell beside it, the `!` terminal in its worktree and a `^g` editor all show
the user's own words (`App::watched_ticket`) — and the Settings row `Inside the agent's own pane:
silent | said anyway` (`prefs.json::notify_in_pane`, default silent) opts out. **And it asks
whether anybody is still IN there, by asking tmux (T-299, 2026-09-07, dogfooding)**: `watching`
was the attach and nothing else, so a pane left open behind a browser silenced its own ticket
completely — no banner and no chime, the one suppression that takes both — for as long as the
user stayed away. Two halves. **A focus report is evidence only while it can be REFUTED**:
`restore_terminal` sends `\e[?1004l`, so nothing is reported for the whole of a handover and a
`true` from the moment before the attach would stand forever; `Presence::focused` consults
`self.focus` only while `on_screen`, and off screen presence is keystrokes alone. **And the
keystrokes are tmux's**, since ours never arrive: `Command::FocusQuiet` (no argument — the
subject is whatever the daemon holds the focus token on, so a stale id cannot answer for a pane
nobody is in; a Read, denied to agents like every session read) → `Daemon::focus_quiet` →
`TmuxBackend::client_quiet_secs`, one `list-clients -t <sid16> -F '#{client_activity}'` fork
giving the freshest client's silence in SECONDS (tmux's own resolution; empty listing = nobody
attached). `Presence::saw_pane_quiet(now, Option<u64>)` is `saw_key` for those keystrokes —
`None` CLEARS (no evidence reads as away, here as everywhere), an answer past `KEY_PRESENCE_MS`
is stored as absence rather than as an old moment (subtracting an hour from a young monotonic
clock saturates to zero, and zero is a keypress at start-up), and `saw_board(true, ..)` drops it
with `watching`. The notifier asks only when the answer can change something —
`Presence::attached()` is Some AND `Coalescer::holds(ticket)` — so a board nobody is attached to
never forks at all. The trade the design accepts: reading a long turn without touching the
keyboard for 30 s reads as away, and the ticket chimes. `tui/src/notify.rs` is the I/O half, `opener.rs`'s ladder shape twice over
(`MESIMON_NOTIFY` `off`|`osc`|a program → `terminal-notifier` → `osascript` → `notify-send` →
**OSC 9** to our own stdout, the rung that always resolves; `MESIMON_SOUND` → `afplay` →
`paplay`/`pw-play`/`canberra-gtk-play` → the bell), resolved in `lib.rs::run` and NEVER
`App::new`, so no test app makes a noise. **And the banner can be CLICKED, which raises the
terminal it came from (T-293)** — `terminal-notifier -activate <bundle-id>`, and that rung only:
`osascript` can carry no action at all and `notify-send --action` needs a process that stays
alive to read the click, which a detached null-stdio launch is not, so `click_words` promises one
in `doctor` only on the rung that answered (on Linux a `TERM_PROGRAM` of `vscode` resolves a
macOS id and the line would otherwise lie). `-sender` is refused — nicer icon, but its own README
says it cannot be combined with `-activate`, which needs the sender to BE terminal-notifier.
WHICH terminal is a third ladder in `find_activate`: `MESIMON_TERM_BUNDLE` (`off`, or an id) → an
outer tmux VETOES the question → `__CFBundleIdentifier` (macOS stamps it on the app it launches
and every child inherits it, so it names the id exactly — no table — for kitty, Alacritty, Warp
and whatever ships next) → `TERM_BUNDLES`, a two-row `TERM_PROGRAM` table whose every entry must
be VERIFIED (`osascript -e 'id of app "…"'`) before it is added. The veto is the load-bearing
rung: inside the user's own tmux the inherited `__CFBundleIdentifier` names whatever started the
SERVER, not the client attached now, and `-activate` LAUNCHES an app that is not running — so a
stale id opens a window of the wrong terminal, which is worse than doing nothing; `TERM_PROGRAM`
is rewritten to `tmux` in every pane and is the one reliable negative. `LC_TERMINAL` was refused:
it can only answer where both others are absent, which on macOS is ssh, and there it would raise
iTerm2 on the machine nobody is looking at. `bundle_id` REJECTS where `field` scrubs (a dropped
character yields a different, possibly real id; a leading `-` is another flag to terminal-notifier),
and `Banner::Custom` is not handed the id — a fourth argv word would change what `$3` means to a
program written against T-282, and `opener::launch` clears no environment, so one that wants it
reads the variable itself. No rung may grow a FLAG in `Osascript`'s argv: its words are found by
position (`item 1`/`item 2`/`item 3`). **And the click lands in the board's own TAB, not just its
application (T-301, 2026-09-07, dogfooding T-293)** — `-activate` names an application, so on a
terminal with two boards open in it the click brought the right app forward showing the wrong tab.
The tab is a SECOND flag on the same rung, `-execute`, a `/bin/sh -c` line terminal-notifier runs
AFTER the activation (its source runs BOTH actions, `bundleID` then `command`, which is why the
two are sent together rather than one instead of the other; a script refused permission leaves
exactly T-293's click). `Channels.activate` became `Channels.click: Option<Click>` (`Click { app,
reveal }`), and `Reveal` is a ladder of its own: `MESIMON_TERM_REVEAL` (`off`, or a program of the
user's own — `kitty @ focus-window`, `wezterm cli activate-pane`; a PROGRAM like `MESIMON_NOTIFY`,
so whitespace is refused rather than split) → iTerm2 by the session uuid its
dictionary calls `id of session`, the tail of `ITERM_SESSION_ID` → Apple Terminal by a tab's
`tty`, which is the one on our own stdin (`own_tty`, `libc::ttyname(0)`, asked once from `find`).
The tmux veto needs no rung here: an outer tmux rewrites `TERM_PROGRAM` in every pane, so the
two-name table never answers — right, since in there `ITERM_SESSION_ID` is inherited from whatever
started the server. Three rules, each a test. **A script talks to the app the click raises**:
`Reveal::app()` names its own bundle id and `find_click` DROPS a reveal that is not the one
`-activate` was given, so a `MESIMON_TERM_BUNDLE` naming something else cannot leave a script
aimed here; the user's own program names no app and is exempt. **It raises a tab and never an
application**: both scripts are wrapped in `if application id … is running`, because `tell
application` STARTS what is not running and a click that opens an empty terminal is the veto's
failure by another road — and that guard is what makes the `activate` INSIDE it safe, which is
there because Apple Terminal reorders its windows only while it is the ACTIVE app (`set frontmost`
in a background one returns success and does nothing, measured). **And the command carries no word
from a payload**: a constant script plus one id validated by `session_uuid` / `tty_path` (REJECT,
never repair — a dropped character names a different tab), single-quoted by `sh_line`, the one
place mesimon builds a shell command and argv's rule kept where argv is not on offer. iTerm2 needs
`select` on the window, the tab AND the session (a split pane is a session); Apple Terminal has no
session and no `select` — a tab is `selected`, a window is `frontmost`. An id nothing matches is a
silent no-op. `doctor` says which half it has (`click raises com.googlecode.iterm2 and this
board's own tab`, else `…, not this tab ∙ MESIMON_TERM_REVEAL names a program that can`). Still
owed, and unchanged: T-293's other half, the CURSOR on the ticket (`mesimon show <KEY>`).
**`tui/src/notifier.rs` is the thread that drives them**
(T-291): dispatch hung off `App::tick` until then, and `handover::run` blocks on `cmd.status()`
for the whole life of an attached pane / `!` / `^g`, so nothing fired at all while you were in
one. It owns a SECOND daemon connection (`Client::connect_observer` — never restarts the daemon
on a skew, and reopens through `open_existing`, which spawns nothing: `U` waits for the daemon it
asked to stop, and a thread respawning one behind that wait made every reload thirty seconds of
nothing), holds the differ and the coalescer, and posts. `App` keeps one field — `notifier`, set
by `lib.rs`, never `App::new` — and four forwarders: `saw_focus`, `saw_key`, `saw_board` (both
halves of `Presence`, plus the terminal handle, flipped around every handover in `lib.rs`),
`push_notify_prefs` (on every `set_pref`) and `preview` (the Settings row's sound, said at once
because the control message wakes the thread). The two ESCAPE rungs (OSC 9, the bell) write to
the same stdout ratatui draws on, so both go through `notify::Console`, one lock the draw takes
to draw, a rung takes to write, and a handover takes to change hands — and while the board is off
screen they say NOTHING, which is the honest limit (a helper program is unaffected, and `doctor`
says so on the OSC line). E2e `restart_skew_e2e::an_observer_subscribes_and_never_spawns_a_daemon`. No crate: `notify-rust` reaches ObjC and dbus and
`ci/build-linux.sh` cross-links with `rust-lld` only because nothing in the graph is C. The words
ride **argv, never a program's source** (`osascript -e 'on run argv' …`, `workspace.rs`'s rule)
and every field crosses `text::scrub_text`, which is also what makes the OSC rung safe. It is a
SUBMENU (`Scope::Notifications`, `keymap::NOTIFY_ITEMS`, the same `draw_list`) because that
function does not scroll and Settings already outruns a 20-row terminal — the row sits THIRD, not
last, because last is inside the clipped region; `the_settings_subtitle_marquees` moved to idx 6
with it. Seven rows now, 2–7 gated on the first (14 lines of dialog, so it still clears
`MIN_H` at `draw_list`'s eight); the two sound rows PLAY what they name as you cycle
(the theme picker's rule that the cursor is the preview). Seven `prefs.json` keys, the two sound
names taking the week-start shape. Nothing daemon-side moved — no `Command`, no `Snapshot` field,
no schema.
`doctor` prints a `notifications` line naming the rungs even while it is off. Goldens
`notifications_120x30`, `settings_120x30`.

**The board keeps the machine awake while an agent is mid-turn (T-288).** Opt-in under
Settings › Behaviour. While enabled, a fixed-width label beside the board's ticket count reads `☕ on`
when held and dimmed `☕ off` when idle (`@ on` / `@ off` in ASCII mode); disabling hides it.
The header cursor moves left/right between the label and git, and Enter on the label opens
its selected settings row. Idle stays dimmed even when focused.
`caffeine_watch::Monitor`
observes snapshots on its own read-only daemon connection, including pending submissions,
so activity continues to update during attached panes and external editors. It works with
notifications disabled. `quiet::is_mid_turn` excludes waits for user action and retains
Codex's conservative observation hold. The observer never starts a daemon. A disconnect
retains the last activity level; reconnect fetches a complete snapshot. Disabling or dropping
the monitor releases synchronously even while its observer is blocked on a request.

`caffeine.rs` implements the power backends: process-owned IOKit on macOS,
`systemd-inhibit --what=idle:sleep … cat` on Linux, and the opt-in, unverified WSL bridge.
Linux needs the sleep lock because desktop power managers request Suspend independently of
logind's idle handling; this can also block explicit suspend. The `caffeinate` override wraps
`cat`, so EOF ends it even after a board crash or exec. Arbitrary custom programs must implement
the documented stdin-EOF release contract themselves. `doctor` names the backend and its limits.
The display and closed-lid promises apply to macOS. See STALE-MAP's T-288 review corrections.

**A banner says WHICH BOARD, WHICH TICKET and WHAT HAPPENED (T-292, 2026-09-07, dogfooding: "OS
notification doesn't show ticket title. and no transcript").** `Post` has three fields — `title`
`mesimon - simbly` (the product name because the banner is posted under the HELPER's identity, not
mesimon's, plus the board's own directory; a hyphen, so the source does not read as a peer of the
`∙`-joined news a folded rung gets), `subtitle` `T-12 ∙ Add auth to the API`, `body` `needs you ∙ PERMISSION` /
`finished ∙ <the agent's last line>`. **The verb always leads and content is APPENDED**, so the
sentence with nothing to append is the one that shipped before (`needs you`, `finished a turn`)
and the two moments are told apart without the chime. **Only a batch of ONE gets it**: three
titles do not fit a banner, so several keep T-282's count shape with no subtitle, `body()` is the
aggregate's alone and `names`/`said` are the single's. **The words are resolved at POST time** —
`Coalescer::due(now, &Voice, &dyn Fn(Ulid) -> Option<Detail>)`, asked about ONE ticket and only
past the window check, because a turn's closing record lands around the moment the state flips and
reading at the edge races the writer; `Worker` keeps its last `Board` and `detail_for` is a FREE
function so the closure borrows that field alone. The reply is taken only when
`peek::Peek::reply_key` is set (the peek falls back to the USER's own words prefixed `>`, which a
banner must never quote back), flattened with `text::one_line`. `core::notify` clips a title at 72
chars and a reply at 120 on a word boundary; `MAX_FIELD` (240 bytes) is the backstop, and the
folded line two already-capped fields. **The rungs that have one field fold** (`Post::folded`, so
the separator is decided once) — `notify-send`, a `MESIMON_NOTIFY` program and OSC 9 keep the
arity they had; `terminal-notifier` gains `-subtitle` and `osascript` a three-item script, ONLY
when the subtitle is non-empty, so an aggregate posts the old argv byte for byte and the words
still ride argv, never the script. **`-group mesimon-<proj16>`** (one per BOARD, the canonical-path
hash) makes a new banner REPLACE the last in Notification Centre — coalescing extended into the
OS; T-293 must revisit it, since only the newest banner survives to be clicked. The seventh
Settings row `The agent's words: quoted | withheld` (`prefs.json::notify_words`, default quoted,
sitting THIRD) withholds the agent's last line and a raised hand's sentence — never the ticket,
never mesimon's own reason word, which is why `Event` grew `quoted: bool` (`why` is
`reason_word` on one road and `Raised::reason` on another, indistinguishable as strings). Off is
also cheaper: no transcript is opened. (STALE-MAP "The board says it out loud" +
"Notifications speak from a thread of their own" + "A ticket is quiet inside its own pane"
(which is T-291's, whatever its heading says) + "A notification says the ticket's title and what
the agent said" + "A banner you can click raises the terminal" + "A click lands in the board's own
tab".)

`?` (`ui/help.rs`) renders `keymap::overlay` and is the complete answer for the current
screen and state.

**Release notes are `CHANGELOG.md`, compiled in, on a screen of their own (2026-09-04).** The
root changelog is the one store: `core/src/relnotes.rs` holds `SOURCE` (`include_str!`, so an
edit is in the next binary) and `parse` — a release is a `## <tag> — <date>` heading (em dash or
hyphen, ISO date) and the markdown under it; a `## ` line that is not one is body text, so prose
never mints a version, and a fenced line never splits one. `every_release_is_dated_and_in_order`
runs over the REAL file in `cargo ut`: every heading dated, tags unique and newest first, and the
top entry IS `v{CARGO_PKG_VERSION}` — bump the version, write its notes, or the suite says so.
`ci/release.sh` dies early without a dated heading for the tag and lifts that section for the
GitHub release body (a prefix match now). The Esc menu's `Release notes` row (`Verb::ReleaseNotes`,
beside the theme row, never a suggestion) opens `Screen::Releases` — state in `App::releases:
ReleasesState` (parsed notes, the build tag, scroll, and the band rows the draw records) — drawn by
`ui/releases.rs`: chip `RELEASES`, an identity row, then one document in a reading measure of at
most 100 cells centred on the screen: a band per release on the elevated surface (tag bold, date
in words, `this build` on the running one) and `rich.rs` under it. The band of the release the
window starts inside stays pinned to the first row. `Scope::Releases` inherits `Global` and wears
the diff's reading keys on the diff's verbs (`jk`, `{ }`, `n N` = next / previous release, `q`),
routed on the screen in `App::dispatch`; the menu is the board's, so `q` always returns there.
Goldens are seeded with a three-release fixture (`install_releases`), never the real file, which
would drift every release; `test_real_changelog_reads_lawfully` pages through the real one at
three widths under L1 instead. (STALE-MAP "Release notes are the changelog, on a screen".)

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

**The agent tier (T-84): eight tools, and three named movers.** Every Claude session
mesimon spawns also carries `--mcp-config '<inline JSON>'` naming `mesimon mcp` — a stdio shim
that forwards each `tools/call` to `orch.sock` as `Envelope { principal: Agent { session } }`.
The config is written to NO file (no `.mcp.json`, no `~/.claude.json`, no `settings.local.json`,
no plugin), so a session mesimon did not spawn can never reach the tools and revoking is "stop
passing the flag". There is no bearer token, deliberately: `orch.sock` already accepts
`Principal::Local` from any same-uid process and a token in the tmux session env is readable
from every other pane, so the boundary is the 0700 runtime dir — the same one `hook.sock`
already relies on. The shim is untrusted (it runs in the agent's process tree) and holds no
policy.

Tools: `get_ticket`, `list_board`, `move_ticket`, `read_note`, `write_note`, `create_ticket`,
`tag_ticket`, `raise_hand`. **No tool takes a ticket id** — the ticket comes from the session binding, so there is no
ownership check to get wrong. `to_column` is a plain string validated server-side, never an
`enum`, because column names are the user's words and an enum would inject them into every
request forever. `core/src/mcp.rs` holds the tool definitions, the description lint (no second
person, no imperatives) and the ≤820-byte cap. **`create_ticket` (2026-09-03) is the one tool
that touches a ticket other than the caller's, by minting it**: `Command::AgentCreateTicket {
title, column?, description?, idempotency_key? }` → `Daemon::agent_create_ticket`, the same
`mint_ticket` + `sanitize_title` a human's composer gets, authorized as `Mutate` on
`Resource::Column` (one card appended, the board itself untouched — `authorize` still denies
`Mutate` on `Board`), refused under the columns bar, `column` absent = the board's DEFAULT column
(T-279, 2026-09-06: `Board::landing_column` — `Board.default_column`, a name in `columns.toml`
chosen by the Settings row `Default column: …` / `Command::SetDefaultColumn`, denied to agents;
the first column until one is chosen or once that column is deleted; a rename carries it),
and the description written through `write_note` so the note carries `agent:<uuid>` as author.
The receipt is a KEY (`T-9`), never an id, and nothing takes a key back; the new card has no
session and no tool starts one. The replay map is now `AgentReplay::{Moved, Created}`, keyed by
tool as well as by idempotency key. Feed line `create_ticket` with the agent as actor.
(STALE-MAP "An agent can file a ticket".) **Tags cross the tier read-mostly**: `get_ticket`
carries `tags` (worn) and `allowed_tags` (the registry, `allowed_columns`' twin) as
`AgentTagView {name, group}`, and `create_ticket` takes `tags`, an array of NAMES resolved by
`Daemon::resolve_agent_tags` before the mint — exact then unique case-insensitive, unknown /
ambiguous / two-on-one-group refused, the registry never written (STALE-MAP "An agent sees
tags"). **`tag_ticket` (T-164, 2026-09-03) wears one of the
board's EXISTING tags on the caller's ticket, or takes it off** — `Command::AgentTagTicket { name,
group?, remove }` → `Daemon::agent_tag_ticket`. The registry is READ and never written on this
path: a name the picker never saw is refused with a pointer at `get_ticket`'s `allowed_tags`
(the registry as `AgentTagView`s, no colour), a name on two axes is refused until `group` says which, a
different case that resolves to exactly one entry lands as the registry spells it, and the
groupmate that came off travels back as `replaced`. Both roads share `Daemon::lookup_agent_tag`. Idempotent (wear what is worn, remove what is
absent: both succeed), so no replay key. Authorized as `Mutate` on `Resource::Ticket`; feed line
`tag_ticket`. The human's six tag commands stay in the never-tier because `SetTag` registers on
the fly and the other five rewrite the registry. (STALE-MAP "An agent wears the user's tags".)

**`raise_hand` (T-107) is the one tool that reaches the LOUD register, and it reaches one card:
its own.** Before it, an agent ending an ordinary turn produced no attention at all — `Stop` →
`Idle{EndTurn}` → automove to REVIEW — so "I finished the refactor" and "I cannot proceed until
somebody chooses an auth provider" looked identical on a board of twenty tickets. (Claude Code's
own `AskUserQuestion` does light a card, at rank 2, but it FREEZES the turn on a modal in the
pane; this is the same message with the turn over.) `Command::AgentRaiseHand { reason }` →
`Daemon::agent_raise_hand` writes `Ticket.raised: Option<Raised { at, by, reason }>` — a
`[raised]` table with the tables, after `workspace`, before `[[tags]]` — `AgentTools::Annotate`
(it writes on the caller's own ticket), Mutate on `Resource::Ticket`, refused on an archived
ticket, feed line `raise_hand`, `reason` REQUIRED and capped at 160 bytes by
`board::sanitize_reason` (the mark is a pointer, the transcript is the record; the receipt says
what was kept). The description cannot spell the product's own phrase — `lint_tool_text` bans
`"you "`, which "needs-you mark" contains — so it says "waiting on a person".

**It is the ticket's, not the session's, and that is the whole design**: the `Stop` that lands
moments after the call would wipe a `RequiresAction` reason, the 15-minute stale demote would
drop it, D28 pins the ranks, and a restart re-derives every session as `Unknown{DaemonRestarted}`
— the turn ENDING is exactly what must not clear it. **The next turn BEGINNING does**, and there
are three roads: leaving the ticket's page (`App::ack_hand` → `Command::LowerHand`, never-tier),
any `UserPromptSubmit` on that claude (`lower_hand_on`, beside `asked_by_hand`/`ack_owed`), and —
since T-311, 2026-09-07 — `apply_change`'s `Idle{EndTurn}` → `Running` at High, which is T-228's
promotion read a second time: a `!` bash command in Claude Code puts its output into the
conversation and the model takes a turn on it with NO prompt hook, and that is exactly the shape
of an answer to a hand ("run `gcloud auth login`, then tell me"), so the `!` stood on a card that
was visibly working again. The three conditions each exclude a road that is not a person: `Background`
is a park a teammate's report resumes (T-135), sub-High into `Running` is `SubagentStop`
correcting a misread tail rather than a new turn, and `RequiresAction` → `Running` is a permission
dialog resolving mid-turn. Unlike `woke_at` the board
CURSOR lowers nothing (a glance is not an answer) and the page lowers it on the way OUT (clearing
on arrival would blank the row before it could be read). `train::plan` skips a ticket with a hand
up on both lists — a person looks before the branch goes anywhere — which also keeps the train's
own merged-notice from clearing the hand it should respect. The `!` outranks the session glyph as
the woke mark does; the reason draws on the CURSOR card only, in the snooze/owed row's slot, and
on the ticket page's state row (`∙ claude asked 4m ago ∙ …`). No new key, no `Ctx` field, no
`TICKET_SCHEMA` bump (a dropped hand loses an alert, not recoverable state). `needs_you_count`
became a count of TICKETS on the way past (`Board::needs_you_tickets`): the three roads overlap,
and `!2` for one ticket matched nothing on screen. E2e `raise_hand_e2e`, goldens `board_raised_*`
/ `ticket_raised_*`. (STALE-MAP "An agent can ask for the user" + "A turn that starts without a
prompt answers the hand".)

**The tools can be switched OFF, and the board offers the AGENT BRIEF — one line in the system
prompt of the claudes it starts** (T-217, 2026-09-04; re-aimed from CLAUDE.md to
`--append-system-prompt` by T-224, 2026-09-05, user: "a better approach than modifying
claude.md ∙ opt in ∙ tell the user verbatim what will be added, and that it's only for mesimon
created sessions"). `Board.mcp_tools` is a per-REPO scalar in `columns.toml` beside
`tags_seeded` (default ON; `COLUMNS_SCHEMA` 3 — a bump, not a serde default, because an older
build dropping `mcp_tools = false` would hand every agent its tools back after the user took
them away; `Board::Default` is hand-written for the same field). `claude_argv` omits
`--mcp-config` entirely when it is off — not an empty config — and `resume_argv` both DROPS the
pair when off and INSERTS it when on and the persisted argv lacks it, so a wake is the road a
session takes to pick the switch up either way; a live pane keeps what it was born with.
`Command::SetMcpTools` (denied to agents: a tier that could switch itself off is not one), the
Settings row `Agent tools: on|off`, and a `doctor` line. And because the layers are `MESIMON_TICKET`
for the shell and `get_ticket` for the model, with nothing telling the model to USE the second
one, `core/src/brief.rs::TEXT` is the sentence mesimon offers to put in the SYSTEM PROMPT of every
claude it starts here — `Board.system_prompt`, a per-repo scalar beside `mcp_tools` (plain serde
default, no schema bump: dropping it sends LESS), OFF by default, `Command::SetSystemPrompt`
(denied to agents), honoured by `claude_argv`/`resume_argv` only while `mcp_tools` is on
(`Daemon::brief_on` — the sentence names `get_ticket`), regenerated on a wake like the MCP blob.
Hard-wrapped to `claudemd::WRAP` (56) because `Mode::Brief` shows it VERBATIM and `dialog::MAX_W`
is 64. The offer (`Verb::BriefOffer`, chip + menu row) stands while the brief is off, the repo's
CLAUDE.md lacks the `MESIMON_TICKET` marker (`claudemd::Sampler`, `ClaudeMdStatus` — a user who
wrote it themselves is never nagged), the tools are on, and
"never" was not said. The dialog is mesimon's ONE modal confirmation — every other confirm is a
chord tail or `m`'s arm, which draw nothing — its first two lines say the REACH (claude sessions
mesimon starts here, only those, nothing written to disk), and its four answers are `enter` turn
on / `c` copy (the text itself, OSC 52, write-only so it never claims success, the one key that
leaves the dialog up; `claudemd::SNIPPET` survives as the CLAUDE.md form `doctor` prints) / `i` never (stamps
`Board::claude_md_ignored`, the key keeping T-217's name on disk) / `esc` not now. Settings row
`Agent brief: on|off` is the other road: OFF is one press (and stamps the offer answered — a
person who turned it off is not re-asked), ON opens the SAME dialog (`Mode::Brief {
from_settings: true }`, every answer returning to the row via `App::leave_brief`) so the words
are never switched on blind; `doctor` prints the text whatever the stamp says. **The CLAUDE.md WRITE is gone** — `daemon/src/claudemd.rs` only samples now — and
README promise 1 no longer names it; promise 3 names the brief as the one consented exception
beside the tool registry. (STALE-MAP "mesimon offers the CLAUDE.md line" + "The write allowlist
says what it does" + "The brief moves into the system prompt".)

`mcp::agent_allows` is an **exhaustive match over `Command` with no `_` arm**: adding a wire
command will not compile until someone decides whether an agent may send it. That is the
enforcement for D10's never-tier — no spawn, no kill, no delete/archive/rename, no workspace, no
merge, no diff, no tag REGISTRY writes (the six human tag commands: `SetTag` registers on the fly,
the other five rewrite the vocabulary board-wide), no session read at any tier. `authorize()` is now real for `Agent`: `Session` is denied outright and so is
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
it, notice in the advisory row, cleared by any move by hand). **A person's prompt reaching the agent
(`Signal::UserPromptSubmit`) supersedes the person's OWN last move** — `MoveGate::asked_by_hand`,
T-186 — so `<<` to TODO then Shift+Enter lands the card in IN PROGRESS; an agent's move and the
fuse keep their protection. State is in memory on purpose: a
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
attention machine arms a flap pin after >4 committed changes in 20 s (`FLAP_MAX`) — since
2026-09-04 the pin drops only INFERRED signals (probe, status file, transcript tail) and a stated
High one (`Stop`, `UserPromptSubmit`, a hook) commits through it at High; before, a `Stop` was
dropped and a finished turn sat `Running` until the probe called it interrupted (STALE-MAP "A
stated Stop commits through the flap pin"). A second `UserPromptSubmit` to an already-`Running`
session still produces no edge, so an assertion resting on it passes for the wrong reason.**

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
dir; pid-bearing locks + crash-safe sweep. **Workspace is chosen with `Shift+Tab`, for as long as
the choice is open (T-309, 2026-09-07)**: the composer's pick, and then the same atom on the board,
on the ticket page and in the description editor, all four through `App::set_ticket_workspace` →
`Command::SetWorkspace` so they cannot disagree about which way the toggle goes. `Ctx::workspace_open`
is the gate and it MIRRORS the daemon. **The lock is what the choice would RELOCATE** — a worktree
binding, or a session with a PANE (`SessionState::has_pane`, so `Sleeping` is a conversation and not
a checkout); it was ANY session record until T-309, and on a real board that is a lock nothing can
open (17 of the author's 44 live tickets, none provisioned). A parked record relocates nothing:
`resume_session` replays the record's own cwd and re-resolves only when that directory is gone
(T-278), so the field governs the next SPAWN, which is what it is for. `Ctx::workspace_worktree` is
what names the
DESTINATION in `keymap::workspace_hint` (`t`'s idiom: `own worktree` / `shared checkout`), since the
card and the state row already say where the ticket stands. HINTED on the ticket page, overlay-only
on the board (that footer is at its width at 120 columns). **The binding is `m`'s shape and the
second one with it — LIVE while unhinted**, because a locked ticket has something worth saying and
the first cut's silence read as a broken key (user, 2026-09-07): `App::set_ticket_workspace` names
the ticket and the reason (a running agent, a worktree, or a workspace board — that last one the TUI's own
refusal, since the daemon would take the field and only `resolve_spawn_cwd` would refuse later).
**The composer's ring is TWO stops, always explicit** (`App::cycled_workspace`, the one-line
composer and the editor both): it walked three (`None` / `Worktree` / `SharedCheckout`, T-117) and
two of them drew the same word wherever a column has no default, so coming back from `worktree` took
two presses; the row's `(column default)` tail is the readout the third stop stood in for. The
status says where the ticket landed, and the
card's mark says the rest: **a worktree asked for but not cut is `⎇·`**, one dot in the DORMANT
register against `queued`'s three (`card::worktree_mark`'s no-binding arm, `WtTone::Dormant`),
because provisioning is lazy and between the pick and the first spawn the card said nothing at all.
Archived tickets and a workspace board (`multi_repo`) offer nothing, and the ticket screen still has
no `w`. Goldens `board_worktree_planned_120x30`, `ticket_new_claude_120x30`; e2e `exit_parks_e2e`
holds the loosened lock from both sides. **Merges are ff-only** — TUI `m` is a staged flow
(stage derived from git state): ahead+ff → confirm→merge; main moved → inject
"rebase+test" to the agent (conflicts resolve in the worktree, tests run pre-main); merged →
inject the notice. Delete gates on unmerged bindings (`d` then `D` discards, branch `-D`); DONE move
blocked while unmerged; teardown waits for the reaper (never remove a live cwd), single
`--force` only. **The archive reclaims a LANDED worktree (T-278, 2026-09-06)**: a ticket
archived with its branch merged by `ticket_merged`'s oracle (ancestor or T-267's patch-id
verdict) and nothing holding a pane goes down the same teardown road (`Teardown { why:
Archived }`) — directory removed, `branch -d` tried, the binding dropped with the branch or
kept `Evicted` where git refused (a squash), so a restore's spawn provisions fresh or replays;
unmerged work keeps its tree, a snooze touches nothing, and a wake on a `Worktree` ticket
whose cwd is gone re-provisions through `resolve_spawn_cwd` (`pending_resumes`,
`Response::Provisioning`) instead of refusing. E2e `archive_reclaim_e2e` (STALE-MAP "The
archive reclaims a landed worktree"). Card mark `⎇ ⎇… ⎇↑ ⎇↓ ⎇✓ ⎇! ⎇x ⎇-`; sessions in worktrees carry
`MESIMON_WORKTREE_BRANCH` (every spawn carries `MESIMON_TICKET`, worktree or not — T-84), and
spawns pass the user's own `permissions.defaultMode` as `--permission-mode` (fresh worktree
paths lost it otherwise).
E2e: `crates/mesimon/tests/worktree_e2e.rs` (the one e2e with a real git repo).

**A branch merged UPSTREAM reads merged, squash and all (T-267, 2026-09-06).** M4's merged test
was ancestry alone (`merge-base --is-ancestor`, and `compute_flags`'s "nothing ahead"), so a PR
squashed on a forge — which leaves not one of the branch's commits behind — read `⎇↓ main moved`
forever, asked its agent to rebase finished work, and was refused DONE. The rule now has a second
clause: the branch's PATCH is already on the target, compared by `patch-id` the way `git cherry`
does it (`worktree::content_merged`: ours = the branch as one `diff-tree` patch, theirs = `log -p`
over the target since the merge base filtered to the branch's own files, each = the branch's
commits one at a time for a rebase-merge). **It writes nothing** — the usual `commit-tree` +
`git cherry` trick would put a loose object in the repo, which README promise 1 does not allow.
The target is ONE ref per pass: `origin/main` where there is one holding everything the local base
holds (`worktree::upstream_base`, `Daemon::upstream_ref`, forgotten with `base_branch` after a
fetch), else the local base — so a squash merged here and not pushed still lands in the right
place. Three measured traps live in the code: `--no-renames` on BOTH sides (the path filter would
flip rename detection on one only), `GIT_PINS` on the command line (`diff.orderFile` gone is fatal,
`log.follow` reaches `log` and not `diff-tree`, `log.abbrevCommit` zeroes the commit column), and
the WINDOW — `--max-count` takes the newest N, so the walk is anchored a week before the branch's
own last commit (`--since=@…` off `%(committerdate:unix)`, added to the `for-each-ref` that was
already being made) and a verdict that NAMED a commit is re-affirmed forever by one
`--is-ancestor` on it (`ContentSeen`, memoised on `FlagInput::seen`/`Flags::seen`;
`CONTENT_SCANS_PER_PASS` caps a fetch's stampede at two bindings a pass). Everything downstream
reads the one `merged` flag, so the card's `⎇✓`, `train::plan`'s skip and `App::merge_stage`
followed with no code; the DONE and delete gates go through `ticket_merged`, which asks ancestry
fresh and then the sample's verdict with the tip re-read — **the gates must answer as the card
does**. `merge_ticket` takes the same oracle ("already in origin/main", never "main moved").
Teardown is unchanged and conservative: `branch -d` on a squashed branch is refused by git, so the
branch survives unless the user discards it. `absorb_worktree_flags` now returns whether anything
the board DRAWS changed and `on_worktree_flags` broadcasts on it (it broadcast only when the train
acted): a merge made elsewhere moves no session and fires no hook, so the sample's own delta is the
only thing that can tell the board. The card says only `⎇✓`; the ticket page's state row
says `∙ merged` for an ordinary ancestor merge and `∙ merged into origin/main as 1a2b3c4`
otherwise (`ui/ticket.rs::merged_word`, `WorktreeItem.merged_in`/`merged_oid`, both serde-default,
`merged_in` empty for the ordinary case). Timeliness is the user's: an upstream merge shows after
a fetch, which stays opt-in. `doctor` prints a `merge base` line. E2e `pr_merge_e2e`.
(STALE-MAP "A branch merged upstream reads merged".)

**The diff viewer has two targets, and the SCREEN says which (T-221, 2026-09-04).**
`Command::DiffTarget` is `Ticket { id }` (the worktree branch, `BASE...BRANCH` through a
`worktree::Binding` — what this ticket changed) or `Checkout` (the board's own repo, `git diff
HEAD` → the working tree — what is uncommitted here). Ticket `v` opens the first, board `v` the
second, and a worktree ticket under the board cursor changes nothing: the board is the
repository's screen, the ticket page is the ticket's, so a branch diff is still `space` then
`v`. `Screen::Diff` is a UNIT variant — the target lives on `DiffState`, and `App::diff_ticket()`
is the one place a screen asks which it is on. The board's binding is `Group::View` (the board
has no other Worktree binding; one would mint a `BRANCH` section in `?` over a working-tree
diff), `avail: |c| c.git_repo` (= `RepoGit::sampled`, so it is inert with no repository), and
**`prio: 0` — the hint sits where it operates**: `chrome::git_clause` draws ` v diff` beside the
header's own `∙ 3 changed`, the T-158 idiom the ticket rail's `c s x` and the PREVIEW heading's
`{ } page` already use, so the footer stays the selection's. It rides the COUNT, so a clean
checkout says nothing and `?` is where the key stays; and it is the FIRST rung the clause gives
up when the row is tight (then the count, then the name truncates). `!` on the diff is the
project's terminal on the diff's own target (T-273, below): the worktree's on a branch diff with
its worktree present, the checkout's otherwise. **On the checkout an untracked file OPENS**: the daemon
stamps the row `status = "A"` / `old_mode = "000000"` / `new_mode` from `symlink_metadata` and
serves it from `git diff --no-index -- /dev/null <path>` (which exits 1 on differences, hence
`git_bytes_diff`), so `diff_fetch`'s `status.is_empty()` skip and 08 §2's "not reviewable" copy
both needed no target condition at all. The list runs `-uall`, not `-unormal`, because
`-unormal` collapses an untracked directory to one unopenable `? dir/` row. Trap: `git diff
--raw HEAD` writes the destination blob as FORTY ZEROS, so `build_file_diff`'s `ModeOnly` clause
can no longer be blob equality — it is "modes differ, both real file modes, no hunks, not
binary", and the `000000` half is what keeps an empty add or delete from reading as a chmod.
E2e: `crates/mesimon/tests/diff_e2e.rs`. (STALE-MAP "The board diffs its own checkout".)

**`!` is the project's TERMINAL (T-273, 2026-09-06): a persistent shell on the private tmux
server, in the checkout — `git fetch` / `pull` / `push` without leaving the board or opening a
tab.** `Verb::Terminal` on the board, the ticket page and the diff, and the SCREEN says which
directory (T-221's rule): the board is the repository's screen, so the checkout root; a ticket's
page its ATTACHED worktree, else the checkout; the diff its own target (`App::terminal_ticket`).
It is NOT a `SessionRecord` — a session belongs to a ticket (`SessionRecord.ticket` is a plain
`Ulid`) and every card, rail, quiet gate and reaper reads it as one, and the terminal is a place
to stand, not work on a ticket — but a named tmux session the way the first-run GATE's
`msmn-gate` is: `msmn-term` for the root, `msmn-term-<ticket ulid>` for a worktree
(`server.rs::terminal_name`; the ULID because teardown runs after the ticket left the board and
the binding carries no key). `Command::OpenTerminal { ticket }` → `Daemon::open_terminal`: alive
(listed, not `pane_dead`) is REUSED — a `git pull` in flight is never lost, and the pane outlives
the TUI and the daemon like every session does; a dead pane (`exit` typed; `remain-on-exit`) is
killed and respawned through `Daemon::launch` (the user's exports and PATH; a worktree's also
gets `MESIMON_TICKET` / `MESIMON_WORKTREE_BRANCH` via `session_vars`, the root's no ticket
variable). It answers `Response::Attach` and holds the focus token — `Daemon::focus` is
`Option<Focus>` now, `Session(uuid)` | `Terminal { ticket }`, so a focused session keeps the
terminal out and the terminal keeps `FocusStart` out — released by `Command::TerminalEnd`; the
tmux status line's leaf reads `terminal`. Both commands are denied to agents. In the TUI the
attach rides the focus road (`App::focus_target`: GATE first, then the grant, parked as
`FocusTarget::Terminal` in the slots a session's `FocusTarget::Session(uuid, origin)` uses) and
the return sends `TerminalEnd` and STAYS on the screen the key was pressed on — no origin, since
nothing was selected. `process_teardowns` kills the worktree's terminal before `worktree::remove`
(never remove a live cwd; the reaper never saw it because it is no session). Hints: NONE on screen
since T-277 (2026-09-06, user: "keep only on ? help menu") — all three bindings are `prio: 0` and
`?` lists `! terminal` / `! terminal in worktree`; for a day `chrome::git_clause` drew it after
` v diff` and the ticket and diff footers carried it. Before T-273 `!` was the
diff's `WorktreeShell`: `$SHELL` in the foreground through the handover, gone on return, inert
on the checkout diff. `pending_attach_cwd` survives for the `^k` editor road only. E2e
`terminal_e2e`, and the worktree case in `worktree_e2e`. (STALE-MAP "`!` is the project's
terminal".)

**A board on a WORKSPACE — repositories nested one level under the root — says so and diffs
them together (T-225, 2026-09-05).** The author's simbly is nineteen independent repos under a
three-file meta repo (`.gitignore` = `*/`); every git answer there was about the meta.
`gitstatus::census(root)` (one `readdir` + `<child>/.git` probe, depth one, classified by
`workspace::nested_repos`: a `.git` DIR is a repo of its own, a gitfile belongs to someone else,
a declared submodule never counts; capped at `MAX_WORKSPACE_REPOS`) fills `RepoGit.repos`, and
`gitstatus::sample(root)` sums `changed` across the root and every child (`sample_one` is the
single-repo sample). The header keeps the root's own branch and arrows and adds the count as a clause — `⎇ master ∙
19 repos ∙ 214 changed  v diff`, `workspace::repos_word`; a FOLDER (no repo at the root) of one
takes that one's branch, of several is named by the count; `1 repo` is never said (the mesimon
checkout's `mt/` scratch repo on `orphan` must not become the board's branch) — and board `v` is
ONE list:
`checkout_diff_list` runs per repo, the root's rows bare, each child's prefixed `<repo>/`, and
`checkout_diff_file` routes on the first path component against the CHILD's list. A folder of
several repos with no repo at the root still samples (branch empty); `gitstatus::branch_dir` is
where the sampled branch lives and where the fetch runs. **A worktree ticket is refused in
words** at `resolve_spawn_cwd` (census asked there, not the cached sample) until workspace
worktrees exist, and `Ctx::multi_repo` hides the composer's/editor's Shift+Tab workspace
choice. `doctor` prints a `workspace` line. On the way past, `worktree::default_branch` stopped
reading `origin/HEAD` literally: it asks the remote the checked-out branch tracks, then
`origin`, then the sole remote (simbly's are named `gitlab`). The design and the phases still
owed — learned repos from the hook stream, workspace worktrees as a meta worktree holding one
child worktree each, per-child merge — are `docs/spikes/T-225-multirepo-workspace.md`. E2e:
`crates/mesimon/tests/workspace_e2e.rs`. (STALE-MAP "A board on a workspace of repositories".)

**The merge train (opt-in, 2026-09-04).** Settings rows `Merge train` (`prefs.json::merge_train`,
off) and `Train tells the agent after a merge` (`merge_train_notice`, on). The daemon reads no
preference: the TUI pushes `Command::SetAutomation` on every toggle and from
`App::reconcile_train` whenever a snapshot reads it unarmed while the pref is on (30 s back-off;
a pref that is off pushes nothing), and the daemon holds it in memory tied to the CONNECTION
(`daemon/src/train.rs`, a `Weak` on the client's writer `Arc`; `Msg::ClientGone` from
`client_loop` disarms it and prunes the subscription) — a closed board is a stopped train.
`train_pass` runs when the worktree flags LAND — `on_worktree_flags`, since T-216 (2026-09-05) the flags are sampled on a worker (`queue_worktree_flags` → `worktree::compute_flags`, one `for-each-ref` + one `rev-list --left-right --count` per binding → `Msg::WorktreeFlags`, dropped if `refresh_worktree_flags`'s synchronous road ran meanwhile); the tick only queues the sample, on its own bucket (`MESIMON_WT_REFRESH_TICKS`,
default `RSS_TICKS`) only while `board_busy()` is empty, and does ONE thing: ff-merge the first
candidate of `core/src/train.rs::plan` (a REVIEW ticket, attached, ahead, ff-able, claude
`Idle{EndTurn}` High|Medium or absent; board order) through `merge_ticket` under
`Principal::Automation { rule: "merge_train" }` — both merge roads take a principal now, and the
quiet gate counts CLAUDE sessions only, so a `!` shell no longer blocks `m` — then the merged
notice into its agent if that is on; else ONE rebase ask (IN PROGRESS or REVIEW, base moved past
it, claude idle, pane silent ≥ 5 s by `#{window_activity}`) once per base tip (`Train::asked`, a
hand `m` records too), fused at 6 asks / 2 h (`Notice merge_train_suspended`, a hand `m` or move
clears it). Merged tickets do not move. `AutomationStatus` on the snapshot says armed / asked /
suspended (`merge_outstanding` reads `train_asked`), `pending` carries `merge` / `rebase` rows so
the cards say what is coming (`merge ∙ after T-3 +1`). **A merge the CHECKOUT refused says so
(T-289, 2026-09-07):** the refusal detail rides `Pending.text`, the card's owed row reads
`auto-merge ∙ blocked` (it outranks `after …` — a blocked merge is not waiting for quiet), and
the sentence is a standing `merge_train_blocked` notice in the advisory row beside the fuse's,
built from `pending_items()` so the row and the notice cannot disagree. `Train::refuse`
remembers per `(branch tip, base tip)` and neither moves on a `git stash`, so `on_git_sampled`
calls `Train::forget_refusals()` on a sample delta — which is why the git sample rides
`wt_refresh_ticks()` (= `RSS_TICKS`) now. **The header says NOTHING** — a ` ∙ train`
clause beside the checkout's own branch shipped for a day and was cut (author 2026-09-04, with
` ∙ auto-merge`, its hour-old rename): the header speaks for the whole board and the train only
ever reaches ATTACHED worktree tickets, so a board-wide word claims more than it does. Armed-ness
is the Settings row's to say and what it will do is the card's. `mesimon doctor` prints a
`merge train` line. E2e `merge_train_e2e`. (STALE-MAP "The
merge train".) **And `t` takes one ticket off it (T-227, 2026-09-05)**: `Verb::ManualMerge` on
the board and the ticket page flips `Ticket.manual_merge` (`Command::SetManualMerge`, denied to
agents, `TICKET_SCHEMA` 4 so an older build cannot drop it and re-arm the merge) and
`train::plan` skips a marked ticket on both lists — no auto-merge, no rebase ask, `m` by hand
still does both. Hint `t merge by hand` while `Ctx::train_reaches` (attached binding, train
pref OR armed), `t auto-merge` while marked; the owed row reads `auto-merge ∙ next` / `auto-merge
∙ after T-3 +1` for a candidate and `auto-merge ∙ off` for a marked one (no owed mark: nothing is
owed). Not Esc: that is the menu / `back`. (STALE-MAP "A ticket can be taken off the merge train".)

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
(shrink the 60 s interrupt-probe quiet threshold — 8 s until 2026-09-02, when a working pane was measured silent for up to ~50 s while a large tool input streamed), `MESIMON_NO_UPDATE_CHECK` (the user-facing opt-out for the release
check), `MESIMON_UPDATE_CHECK` (force a dev build past the CHANNEL gate — the build-tree guard
still refuses, so copy the binary out of `target/` first),
`MESIMON_SERVER_GUARD_TICKS` (shrink the 15 s
server-alive guard cadence), `MESIMON_WT_REFRESH_TICKS` (the slow bucket — the worktree
flags, the merge train, the CLAUDE.md sample and the checkout's git sample; default 40 ticks). E2e pattern: in-process
daemon thread + real tmux + the real built binary via `env!("CARGO_BIN_EXE_mesimon")` (only
available in `crates/mesimon/tests/`).

## Boundaries

- Apache-2.0 core; `team/` is reserved for a future source-available tier — never mix code across
  that boundary (CONTRIBUTING.md records the CLA rule).
- Product promises (README): strict write allowlist, no config mutation (`doctor` prints fixes,
  never applies), zero PROMPT injection — mesimon adds, removes and reorders no token of the
  conversation, and the three MCP tool definitions are the one named exception (T-84 narrowed
  promise 3 from "token" to "prompt"; `mesimon doctor --mcp` prints the whole surface). Don't
  write code that violates them.
