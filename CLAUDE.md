# CLAUDE.md

## mesimon

When `MESIMON_TICKET` is set, this session is working a
mesimon ticket. Call `get_ticket` before you start — the
ticket's description and notes may carry context the
prompt does not.
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
ci/test-linux.sh                     # the whole suite on Linux (Docker, Debian 12, the DISTRO's tmux 3.3a); ~90 s warm. Docker Desktop must be up — SAY SO before `open -a Docker`
ci/build-linux.sh                    # the two Linux release binaries, cross-linked from this Mac (static musl, rust-lld, no Docker)
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
suite with `MESIMON_REQUIRE_TMUX=1`, the same suite on Linux in Docker (dies without
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
the daemon it asked to stop").

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
`hook_send` runs the real hook binary, `wait_until` polls. Any other `MESIMON_*` seam is set
BEFORE `boot` (the daemon reads them once). Each test owns a `/tmp/msmn-e2e-*` dir and its own
tmux socket, which is what lets nextest run them in parallel. `prompt_e2e.rs` is the exemplar.

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
or worktree has locked it (`Ctx::workspace_open` mirrors the daemon's `set_workspace` lock). The
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
get `read_note`/`write_note` (seven tools now, with `create_ticket` and `tag_ticket`) and
`get_ticket` carries the description. Adding a
field to `NoteMeta` is `#[serde(default)]` like everything else. (STALE-MAP "Notes: files under
the ticket".) **An approved plan is the agent's note** (2026-09-03): the `PostToolUse` frame
`ExitPlanMode` fires carries the plan (`tool_response.plan` since Claude Code 2.1.259,
`tool_input.plan` on 2.1.251–2.1.258 — `plan_of` reads both) and only fires once the user approved
it, so `ingest::plan_of` → `Daemon::record_plan` writes it through `write_note` as `agent:<uuid>`
— one note per session (`SessionRecord.plan_note`), revised on a re-plan, minted afresh after a
delete, `notes[0]` (so the description) on a ticket that had none. (STALE-MAP "An approved plan
is the agent's note".)

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
transcript record at all. After a
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
does a second Shift+Enter (the finger is still holding shift). **On a ticket whose claude seat is
EMPTY the board's press is the composer's second half a press late** (2026-09-03): the same
`Verb::Prompt` binding, and `dispatch` routes on `Ctx::ticket_has_claude` to `start_composed` —
claude spawns with the title submitted, no field, no attach, hint `ask claude the title`. A
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
`Ctx::ask_queueable`: a shared-checkout ticket with an awake pane, never a worktree, never a
Sleeping claude). A queued ask rides `PromptSession { queued: true }` into the daemon's in-memory
`queued` list and is pasted by `drain_queue` when `checkout_holders(cwd)` is empty —
`core/src/quiet.rs::working_tickets`: no claude with the same `cwd` Spawning / Running /
RequiresAction / Idle{Background} / `pending_submit` / a paste of ours still owed its ack
(`Daemon::inflight`); a shell never counts — hooked beside `auto_move` in `apply_change`, on the
1 s bucket, and at enqueue (a quiet checkout sends at once); one per checkout per pass, FIFO, one
per ticket. It is DROPPED by any `UserPromptSubmit` on the ticket while it waits (the daemon
cannot tell its own paste's ack from the user's keystroke, so the next prompt closes it either
way), by sleep / kill / delete, by the sweep (target gone, replaced, parked, ticket archived) —
never by a hand move. The snapshot's `pending: Vec<Pending>` (kept general: the train's rows ride
it) prefills the field on the next Shift+Enter (Esc keeps, a blank Enter drops via
`DropQueuedAsk`), feeds the card's owed mark (`glyphs::queued`, slow cadence, over still marks
only — `queued_over`) and the cursor card's `queued ∙ after T-12` row (`App::pending_row`, the
snooze row's slot; the ticket page's state row reads the same). A restart drops the queue like
`pending_prompt`. E2e `ask_queue_e2e`. (STALE-MAP "The board's ask can wait for a quiet
checkout".)

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
gating it on `peek.is_some()` meant the commonest tagged card on the board could never show it.

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

**A ticket holds ONE claude, and the second seat is a shell** (2026-09-02). `spawn_session`
refuses a `Claude` spawn when `Board::live_claude(ticket)` finds one (`is_live`, so a parked one
holds the seat); resume and wake re-enter an existing record and are not gated, so older boards
keep what they have. `C`/`Verb::ClaudeNew` is gone, `S`/`ShellNew` stays, and `c` on a parked
claude hints `wake claude` and wakes it (`focus_session` resumes a paneless record before it
attaches). Everything that picks "the" agent of a ticket — `pane_target`, `board_enter`,
`auto_move`, `card_glyph`, the worktree lock — assumes one; with two they picked the first in
spawn order and automove ping-ponged the column between their turns. (STALE-MAP "One claude per
ticket".)

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
`TICKET_SCHEMA` is 3 (a v2 build would drop `until` and the ticket would sleep forever). The
calendar rungs are pure arithmetic over `LocalTime` in `struct tm`'s conventions with the libc
(`localtime_r`/`mktime`, `tm_isdst = -1`) in `tui/src/localtime.rs`. **The wake is the tick
wheel's** (`Daemon::wake_snoozed`, the 1 s bucket): back at the TOP of its column
(`Position::Top`), `entered_at` restamped, the move gate forgetting it, feed line `snooze_woke`,
no broadcast of its own. **A woken ticket with `needs_you` sets `Ticket.woke_at` — the one
TICKET-level producer of the saturated colour**: `card::render` wraps `card_glyph` with `!` in
`Register::Attn`, `card::needs_you(ticket, sessions)` feeds the badge and the spine, and
`Board::needs_you_count()` is the header chip's AND the tmux status line's number. The mark
comes off on a KEYPRESS that leaves the cursor on it (`App::ack_woke` at the end of `on_key` →
`Command::SeenTicket`), never on the draw clock — a ticket wakes while the user is away and a
parked cursor must not clear it. The preference (`prefs.json::snooze_needs_you`, default on) is
the Settings submenu's `Snooze returns with needs-you / quietly` row; `App::save_prefs` is every
preference's write. E2e: `crates/mesimon/tests/snooze_e2e.rs`. (STALE-MAP "A ticket can be
snoozed".)

Board-wide actions (external drawer, archived list, sleep-all, archive-all) deliberately have
NO key, bar the two the header itself teaches (`U` reloads, `X` sleeps the done agents, `x`'s
own shift widened to the column, since 2026-09-04 — both
overlay-only, so the footer stays the selection's) — they live in the Esc menu (`ui/menu.rs`,
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
(theme, agent replies, how a snooze returns, the week's first day — `MenuItem`s, so
`ui/menu.rs::draw_list` draws both lists) is behind `Verb::Settings` → `Mode::Settings` / `Scope::Settings` (word
`SETTINGS`, the menu's three shapes, `esc back`). Choosing a settings row KEEPS the list open
— the row relabels itself — and the theme picker pops back onto its row on Enter and Esc
alike; Esc from the list lands on the menu's `Settings` row (`App::menu_row` /
`settings_row`). No settings row is ever a suggestion (`every_suggestion_is_a_menu_row` holds
the two lists apart), and the row's detail names the current theme so the door says what is
behind it. Golden `settings_120x30`. (STALE-MAP "The preferences move into a Settings submenu".)

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

**The agent tier (T-84): seven tools, and three named movers.** Every Claude session
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
`tag_ticket`. **No tool takes a ticket id** — the ticket comes from the session binding, so there is no
ownership check to get wrong. `to_column` is a plain string validated server-side, never an
`enum`, because column names are the user's words and an enum would inject them into every
request forever. `core/src/mcp.rs` holds the tool definitions, the description lint (no second
person, no imperatives) and the ≤820-byte cap. **`create_ticket` (2026-09-03) is the one tool
that touches a ticket other than the caller's, by minting it**: `Command::AgentCreateTicket {
title, column?, description?, idempotency_key? }` → `Daemon::agent_create_ticket`, the same
`mint_ticket` + `sanitize_title` a human's composer gets, authorized as `Mutate` on
`Resource::Column` (one card appended, the board itself untouched — `authorize` still denies
`Mutate` on `Board`), refused under the columns bar, `column` absent = the board's first column,
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

**The tools can be switched OFF, and the board offers to tell CLAUDE.md they exist**
(T-217, 2026-09-04). `Board.mcp_tools` is a per-REPO scalar in `columns.toml` beside
`tags_seeded` (default ON; `COLUMNS_SCHEMA` 3 — a bump, not a serde default, because an older
build dropping `mcp_tools = false` would hand every agent its tools back after the user took
them away; `Board::Default` is hand-written for the same field). `claude_argv` omits
`--mcp-config` entirely when it is off — not an empty config — and `resume_argv` both DROPS the
pair when off and INSERTS it when on and the persisted argv lacks it, so a wake is the road a
session takes to pick the switch up either way; a live pane keeps what it was born with.
`Command::SetMcpTools` (denied to agents: a tier that could switch itself off is not one), the
Settings row `Agent tools: on|off`, and a `doctor` line. And because the layers are `MESIMON_TICKET`
for the shell and `get_ticket` for the model, with nothing telling the model to USE the second
one, `core/src/claudemd.rs::SNIPPET` is four lines mesimon offers to append to the repo's own
`CLAUDE.md` — hard-wrapped to `WRAP` (56) because `Mode::ClaudeMd` shows it VERBATIM and
`dialog::MAX_W` is 64, with `MESIMON_TICKET` itself as the marker (so applying twice is
impossible and this repo is never offered anything). The dialog is mesimon's ONE modal
confirmation — every other confirm is a chord tail or `m`'s arm, which draw nothing — and its
four answers are `enter` add / `c` copy (OSC 52, `osc.rs::copy_to_clipboard`, write-only so it
never claims success, and the one key that leaves the dialog up) / `i` never (stamps
`Board::claude_md_ignored`) / `esc` not now. `doctor` prints the snippet whatever the stamp
says — that is the door "never" does not close, and why `doctor::wrap` now wraps one PARAGRAPH
at a time. The write canonicalizes first (a symlinked CLAUDE.md must not become a regular file)
and is still atomic. **README promise 1 names this write** — reworded 2026-09-04 at the author's
request, in the pass that also closed the two standing gaps (the `/tmp` runtime dir and the
self-update's binary); the promise now separates what mesimon writes on its own from the three
it writes only when asked, and the "What mesimon writes" table has a row for each.
(STALE-MAP "mesimon offers the CLAUDE.md line" + "The write allowlist says what it does".)

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

**The merge train (opt-in, 2026-09-04).** Settings rows `Merge train` (`prefs.json::merge_train`,
off) and `Train tells the agent after a merge` (`merge_train_notice`, on). The daemon reads no
preference: the TUI pushes `Command::SetAutomation` on every toggle and from
`App::reconcile_train` whenever a snapshot reads it unarmed while the pref is on (30 s back-off;
a pref that is off pushes nothing), and the daemon holds it in memory tied to the CONNECTION
(`daemon/src/train.rs`, a `Weak` on the client's writer `Arc`; `Msg::ClientGone` from
`client_loop` disarms it and prunes the subscription) — a closed board is a stopped train.
`train_pass` runs after `refresh_worktree_flags` on its own bucket (`MESIMON_WT_REFRESH_TICKS`,
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
the cards say what is coming (`merge ∙ after T-3 +1`). **The header says NOTHING** — a ` ∙ train`
clause beside the checkout's own branch shipped for a day and was cut (author 2026-09-04, with
` ∙ auto-merge`, its hour-old rename): the header speaks for the whole board and the train only
ever reaches ATTACHED worktree tickets, so a board-wide word claims more than it does. Armed-ness
is the Settings row's to say and what it will do is the card's. `mesimon doctor` prints a
`merge train` line. E2e `merge_train_e2e`. (STALE-MAP "The
merge train".)

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
server-alive guard cadence), `MESIMON_WT_REFRESH_TICKS` (the worktree flags' and the merge
train's cadence, default 40 ticks). E2e pattern: in-process
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
