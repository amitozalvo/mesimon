# STALE-MAP — which sections the amendments supersede

> The 18 section documents were written **before** the `00-DECISIONS.md` amendment blocks
> (synthesis corrections, D33a–n, second-pass corrections) and before **D34** (round 3,
> 2026-08-29). This file maps each amendment to the sections it invalidates, so nobody has to
> re-litigate a stale section against a binding decision. Compiled from the four-agent critical
> review of 2026-08-29. A section listed here is **superseded** — read it for its measurements and
> reasoning, never for its conclusion.

## D33b — runtime state moved to `~/.local/state/mesimon/<repo-id>/`

Every `.mesimon/state/` path is stale. Sockets stay under `$RT` (`/tmp/mesimon-<uid>/…`, `sun_path`
budget); persisted state moves.

| doc | stale sections |
|---|---|
| 02 | §2 runtime-layout table entries placing sessions/scrollback/undo/index in repo-local `state/` |
| 05 | §8 logging paths, R-S0-13 daemon log path, D9 references |
| 09 | §1.1 `state/claude/<column>.settings.json`, §11.1 `state/claude-probe.json` |
| 10 | §4 `state/agents/`, `agents.lock` |
| 11 | §11.2.1 `state/hooks/<uuid>.json`, §11.8.2 `state/watch.json` |
| 13 | §13.1.x tree (whole `state/` subtree), §13.9.3 WAL-relocation machinery (now the default world) |
| 15 | §3.3 `state/consent.toml`, `consented/` blobs |
| 16 | §10 first-run grant G1 (state-dir creation no longer needs a repo-write grant) |
| 17 | §4.4 example putting `tmux.conf` in `.mesimon/state/` |

## D33c — write allowlist narrowed

- 14 §9.2 fsmonitor-as-consented-grant path: **deleted** — `doctor` prints the commands, mesimon never writes repo config.
- 13 §13.1.5 / 18 §8.1 sparse-checkout board exclusion: cut (no no-write implementation exists). Under **D34.9** the problem disappears entirely (nothing committed → nothing checked out into worktrees).

## D33e — no hard concurrency cap

- 02 §7.5.5 "cap live sessions at 64, refuse at the cap": superseded — indicate, warn, refuse only at the OS resource boundary (511-PTY ceiling stays as the OS boundary).
- 16 §1.4 `[limits] max_agents = 4` as a hard ceiling: becomes a warning threshold, not a refusal.
- 14 §5.1 PTY spawn budget survives (it *is* the OS boundary).

## D33g / D33h — stages are tags; no regroup-by-stage

| doc | stale sections |
|---|---|
| 12 | §12.7.8 ancestry-cost work (kept as documented rejected option), `stage_reach` cache row in §12.7.4, stage rows in §12.9/§12.10.3; commit-graph offer loses one of two consumers |
| 13 | the `board/stages/` / `policy/stages/` / `state/stages.json` tri-split |
| 16 | §4.10 stage schema and the `KT-S0xx` validator range |
| 18 | §5.5 rows 80–84; much of §8.5's gutter argument |
| 07 | §3.3 STAGE mode and `Space b` regroup (D33h); `requires_stage`/M12/`P` badge (§13.6); §7.2 stage-refusal previews |
| open residue | `◔` "was reached, no longer" needs a memory mechanism for a *removed tag* or the glyph dies — undecided |

## D33i — 4 default columns: TODO / IN PROGRESS / REVIEW / DONE

- 16 `[init]` column list (Backlog/Todo/In Progress/Done): stale.
- 07 degradation ladder re-derives with N=4 as the default case; collapsed spines matter only for user-extended boards.
- Three competing geometry formulas exist (07 §2, 06 §6.3, 01 §8A). **07 §2 owns**; the others are superseded.

## D33k — attention rail cut *(and see D34.3, which re-closes the digit hole)*

| doc | stale sections |
|---|---|
| 01 | §4C "never ships without the rail", §7A row 9, §8A rail arithmetic |
| 04 | §2.3 NAV `1–9` rail jump, §2.13 digit table, `Space 1–9` row, `g<N>` disambiguation, open items 7–8. **Note:** with D34.3 (no counts either), digits are simply unbound in v0.1 — 16 §3.4's deleted count machinery stays deleted and V18 stands |
| 06 | §2.8 mono clause ("the rail is load-bearing in mono") — needs rewrite: mono needs-you = SGR-7 title row + case rule + header count; §10.8 rail mockup; L3 allowed-owner list |
| 07 | §2.x layout tables embedding `RAIL = 3`, §5 one-shot digit hint |
| 08 | §10.1 layout (gains 3 cells) |

## D33m / D34.4 — the daemon may `Allow`; Inbox permission rows use the RPC

- 02 §5.9 "no Allow variant at all, and must never grow one": **wrong as stated**; `Allow` exists behind the machine-local-provenance gate.
- 15 Part 2 `Verdict` enum, its no-Allow doc-comment, and CI guard #2 (grep for `"allow"`): rewrite around `Allow | Deny | NoOpinion`.
- 01 R4/§7C deny-only language.
- 07 §8.4/§21 grid-scrape digit-answer machinery: dead for permission rows (RPC answers them); survives only as fallback for free-text asks.
- Board-level one-keystroke approve and rule-based auto-approval are **two features** — docs conflate them; D33m's four rails govern the auto path only.

## Second-pass corrections (already in `00`, repeated here for the map)

- D31a glyphs: `◉ ◔ ◌ ∙` per 06 §4.1a (not `● ○ ◐ ·`).
- D25: Claude Code keys trust on the **main checkout root** — 12 §12.8.1 has the correction; older probe text elsewhere is wrong.
- D13: `git restore --source=<branch> --worktree`, never `git show <branch>:<path>`.
- D15 enum: `exited{reason}` per 11 §11.7.1; 02's invented `reboot` reason does not exist.
- D22: drop-oldest applies to encoded frames only; "single writer" means board state; one owner per PTY.
- D17: rail floor text — moot post-D33k.

## D34.1 — tmux backend for v0.1 (normative: `19-tmux-backend-v01.md`)

- 03: **entire emulator surface deferred** to the native backend (~v0.2). Survives now: §13.3 previews, child-env allowlist.
- 05: emulator-side correctness deferred; **TUI-side staged restore discipline stays** (handover exercises it).
- 02: supervisor role + SPROTO, render plane §5.4–5.6, sync-2026 pump §5.8, letterboxing §6.2 — all deferred. §2/§3/§4/§5.2/§5.3/§5.9/§7.3/§9 survive.
- 17: §4's own-emulator verdict superseded by author decision; the evaluation itself stands as the record.
- 18: B-E1–E7, B-I1/I2 (emulator family) transfer to tmux or defer.
- 14: §1.4–1.5/§4 emulator-parse budgets deferred; INV rows (0-bytes-idle, 0-timers, RSS-return, wakeups) stay binding.

## D34.2 — adoption in v0.1

- 01 §7B's deferral of "adopt sessions mesimon did not spawn": superseded. Mechanics in 19 §4.
- Sleep/reclaim promoted from P4 (01 §7A row 16) to the adoption milestone.

## D34.5 / D34.6 — undo grace-band only; no content index

- 13 §13.7 full undo journal: design retained, **not built** in v0.1 (01 §7A/§7B wins). `external.edit` REVERSIBLE-class concern noted for whenever it is built.
- 13 §13.12 FTS5 index: v0.2-labelled option (01 §10 and 14 §9.1 win). 15's index-hardening items (B-S3/S4) defer with it.

## D34.7 — REVIEW is a normal column; review screen → ticket diff view

- 08: column binding, queue-keyed-on-column reading, `on_success_column`, and its open questions 3/4 — superseded; the git layer (§1.2, §3.1, §4.1, §4.2.1, §6.5–6.6) survives as the diff-view engine. Land-order planner and conflict matrix stay cut.
- 04 `Space t r` = open review and 07 §6's review keys: rebind to the ticket's diff-view entry.

## D34.9 — `.mesimon/` not committed in v0.1

- 13 §13.1.3 exclude gymnastics: mostly dissolve (one exclude line covers the whole tree); §13.1.4's double `.gitattributes` unnecessary.
- D9 trust gate: **moot for v0.1** (nothing potent arrives via `git pull`). The design survives for the v0.2 opt-in commit flip.
- D30 shared-board rendering ("unclaimed" cards): v0.2.

## D34.10 — conflict adjudications

- Activity feed: append-only JSONL file (14 §1.7 wins; 13 §13.6's SQLite projection deferred).
- Card anatomy: 07 §4.x owns; 06 §6.1 two-row mock superseded.
- Hook observer registration: 11 §11.2.3 wins; 09 §5.3 superseded (exec-form `"$MESIMON_SESSION_SOCK"` never expands).
- `PermissionRequest` wire shape: all three specs (09 §5.2, 11 §11.2.4, 15 §2.2) unverified — spike S-B settles it empirically.

## Cross-document drift (no amendment needed — 04 owns the keymap; regenerate 07 §6 / 06 §10.15)

`x` (04 sleep/wake vs 07 explain) · `y` (single yank vs prefix) · `Space t r` (rename vs review) ·
`Space Q` in 07 §6 violates 04 §2.9's no-kill-binding rule · `g d`/`g a` drift · 08 §10.4's
inherited digit row. Validator rules 3a/12–15 have never been run — run them in M1.

## Known-unverified claims — sprint-0 spike verdicts (2026-08-29, `docs/spikes/`)

| claim | verdict |
|---|---|
| hook event set (31 names, 09 §5.1) | **CONFIRMED** — all 31 accepted; **`StopFailure` exists and fired** (`authentication_failed`). Field drift: `UserPromptSubmit.prompt`, `StopFailure.error` (confirms 11 over 09). Unknown names silently ignored |
| `PermissionRequest` wire shape | **11 §11.2.4 wins** (see D34.10). No `tool_use_id` in payload — 11 §11.7.3's PreToolUse bridge needs another key. Headless no-decision ⇒ auto-deny |
| `~/.claude.json` trust key | **PARTIAL** — `projects["<abs cwd>"].hasTrustDialogAccepted` confirmed; but **worktrees observed with their own per-directory entries (8)**, so D25's second-pass "keyed on main checkout root" is **back in doubt** — live worktree A/B is a required hand-check before the spawn gate is coded |
| `--bare` vs `--settings` hooks | **CONFIRMED dangerous** — `--bare` suppresses hooks even when passed via `--settings` (A/B'd). `--bare` is off the table for observed sessions |
| `CLAUDE_CODE_CHILD_SESSION` flips the renderer | **REFUTED on 2.1.251** — renderer governed by the `tui` setting alone (default `inline`). Env allowlist survives on hygiene/other suppressions; the D0 §2 renderer narrative is dead |
| prompt delivery (D11 mechanics) | **CONFIRMED** — `load-buffer` → `paste-buffer -p` → separate `send-keys Enter`: 3,696 bytes byte-identical, `UserPromptSubmit` fired **94 ms** after Enter (= the delivery ack). Negative confirmed: one-shot body+Enter loses the Enter AND truncates (630/3696); `;` splits tmux commands |
| Claude fullscreen under tmux | **CONFIRMED** — alt screen enters under `TERM=tmux-256color`; **Shift+Enter inserts newline** via CSI-u and via tmux `S-Enter` with `extended-keys always` (3.6a). Wants `focus-events on`. `run-shell -t <pane>` leaves `pane_in_mode=1` swallowing send-keys — guard needed |
| flag survey on 2.1.251 | **CONFIRMED** — all probed flags exist; `--permission-mode` spells normal mode `manual` |

Still unverified (no spike yet): Codex `[hooks.state]` claim (gates the codex-adapter tier
argument) · `CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS` and MCP timeout-floor behaviors · third-party
issue numbers cited in 03/05/17 · "starship/p10k do blocking CPR reads" · Claude fullscreen
mouse-mode ordering.

**NEEDS-AUTH:** the isolated spike `CLAUDE_CONFIG_DIR` has no credentials (Keychain OAuth is
per-config-dir). The live-turn portions (remaining 23 hook events firing, S-B live wire capture)
unlock after the author runs `claude /login` once under that config dir — exact commands in the
spike files.

**NEEDS-MANUAL (author):** Ctrl+] detach across GUI terminals (T-3) · real-ratatui byte-clean
handover (T-4, an M1 item) · physical Shift+Enter through a real outer terminal + nested chain
(T-6) · the worktree-trust live A/B (S-C).

## M2 implementation deviations (2026-08-29)

- **Hook transport is `SOCK_STREAM` one-shot, not the DGRAM/SEQPACKET of `11` §11.2.2.** macOS
  caps unix datagrams at 2 KB (`net.local.dgram.maxdgram`) and has no `AF_UNIX` `SOCK_SEQPACKET`;
  real payloads (`last_assistant_message`, `tool_input`) exceed the cap, and truncation would force
  the hook binary to re-serialize JSON (banned by the §11.2.2 rule 6). One connect + one write +
  EOF-as-frame keeps rule 6's spirit. The binary is `mesimon hook` (14 §1.7's spelling), not the
  separate `kt-hook` of §11.2.2's argv example.
- **Startup-modal detector (`11` §11.5.3) is approximated**: without byte streams, "no OSC 0 within
  3 s of first byte" becomes a +10 s probe of `#{pane_title}` + `capture-pane` while `Spawning`
  (modal ⇒ medium confidence), with the no-bytes ⇒ `unknown` verdict at +30 s.
- **`ExitReason` carries a non-normative extra variant `Killed`** (mesimon's own kill ladder ended
  the session) beyond `11` §11.7.1's five.
- **`11` §11.2.2's exec-form example is wrong on 2.1.251**: a command hook requires
  `"command": "<executable>"` with `"args": [...]` as its arguments — a bare `"args"` array fails
  settings validation ("Expected string, but received undefined" on `hooks.*.command`, whole file
  skipped). Verified live via the Settings Error dialog; generator fixed accordingly.

## D35 — M3 scope decisions (2026-08-29)

- Manual-only sleep: `07` §15.2's offer banner/confirm/receipt strings, `16` §1.4's `[sleep]`
  block, and D23's automatic tier are **deferred** (not superseded — D23's floors still bind the
  manual path). The `after_idle` 10-min (`00`/`07`) vs 4-h (`16`) drift stays open.
- `19` §4 tier 1's "one-shot `claude agents --json` at daemon start" is **not built** in v0.1
  (D35.4); the census itself is lazy (drawer-open only, D35.3), so `02` §9 step 6 has no M3
  implementation either.
- Badge word is **`external`**, superseding `19` §4's "observed" (collision with `07` §14.2's
  observer-client vocabulary).

## M3 implementation deviations (2026-08-29)

- **`Reason::ResumeDialog` added to `11` §11.7.1 at shared rank 8** (with `startup_modal`) — the
  resume-from-summary dialog state `19` §4 tier 3 demanded; D28's fixed ranks are untouched.
- **TIOCGPGRP sleep guard (`14` §6.1a) is tmux-recast**: mesimon holds no PTY master, so the
  bash-session floor is "pane process has live children" via `pgrep -lP <pane_pid>`. The
  `never_if_foreground` name list is deferred with the config system; `pinned_awake` ships.
  A foreground REPL with no child process is NOT caught (the ioctl's coverage is not fully
  reproduced) — the manual keystroke is the mitigation.
- **`14` §6.1's 8-step sleep is a 4-step ladder under tmux**: transcript copy (+ cheap B-A22
  check) → park the record as `sleeping` FIRST (the machine latches: the kill's own
  `SessionEnd`/pane-died must not flip it) → SIGTERM the pane's process group → `kill-pane`
  after a 5 s grace. No grid snapshot, no arena, no PTY-master release — the RSS/PTY recovered
  are the child process's own. Bash sessions may also be slept manually from `Running` (they
  have no `idle` state; the children guard is the floor); wake respawns a fresh shell.
- **Sleep age floor is measured from `state_changed_at`** (time in the current state), not
  session creation time — no created-at field exists on the record. Stricter than D23's wording.
- **The 60 s age floor binds only bulk reclaim** (author, dogfood 2026-08-30): a manual
  per-session `z` is explicit intent and sleeps immediately — D23's "never sleep within 60s"
  hard floor is narrowed to `reclaim_all` and the header reclaim figures. Rationale: the floor
  guards automation churn; a keypress on one card is not churn. The other eligibility checks
  (pinned, idle-only, bash live-children) still apply everywhere.
- **The kill ladder's grace-then-kill-pane gap (M2 note) is closed**: `kill_session`, grace-band
  expiry, and sleep all SIGTERM then reap the pane via a shared 5 s reaper.
- **Observe-tier state evidence** is transcript-tail only (`09` §4.3 cursor, `09` §4.4 signals →
  Low confidence always) — `11` §11.7.5's OSC-glyph row for adopted sessions assumed byte access
  the tmux backend does not give. 45 s of transcript quiet while `running` demotes to `idle`.
  The tail cursor starts at EOF on attach: history is not activity (preview comes from the census).
- **Keymap drift vs `04` (M1/M2-minimal precedent, author hand-check pending)**: board `e` opens
  the External drawer (04 has no binding — B gap), board `Z` = reclaim-all (04: `Space s Z`),
  picker `z` = sleep/wake toggle (04: `x`), picker `p` = pin-awake, picker `x` stays kill (M2
  muscle memory), drawer `a` = attach / `R` = resume-here (04 §2.6's ticket-screen `a` = adopt is
  the spiritual parent; there is no ticket screen yet).
- **Header resource line** renders `N live · M asleep   ptys U/T · X.XGiB` in the grey ramp —
  `07` §15.1's `live N/M` warn-threshold form and quota percentages are not yet built. RSS is a
  10 s `ps` aggregate over owned pane process groups (ASK-23); the PTY `used` figure is the
  `/dev/ttys*` high-water count on Darwin (B-D16) and live `/proc` figures on Linux; the spawn
  gate recounts only at spawn time and refuses only at the OS boundary (D33e), naming the reason.
- **Test seams added**: `MESIMON_CLAUDE_HOME` (census root override) and
  `MESIMON_SLEEP_MIN_AGE_MS` (e2e cannot wait out the 60 s floor).
- **Drawer gesture is IMPORT, not attach-to-selected-ticket** (author, dogfood round): `a` and
  `R` mint a fresh ticket in the first column, titled from the session's title latch → preview →
  id — `19` §4 tier 2's "the user attaches one to a ticket" pre-selection flow is superseded.
  The wire keeps `ticket: Option<Ulid>` so a future ticket-screen `a` (04 §2.6) can still target
  an existing ticket. Drawer previews/names also read the EOF latches (`last-prompt`,
  `ai-title`/`custom-title`) — assistant text can sit MBs before EOF (measured 6.8 MB).

## M3.5 implementation deviations (2026-08-29)

The design-foundation pass (`06`/`07`/`04` are the owners; D33g/D33i/D33k/D34.9/D34.10 applied).
What shipped differs from the corpus in these ways:

- **Elevation collapsed to two painted surfaces** (`bg` + `selected`) in every colour profile —
  `06` §2.1's six-surface model (`sunken/surface/raised/overlay`) is an M6 refinement. The zone
  bands on the ticket screen paint `selected` for the same reason (no `surface` token exists).
- **Detection ladder subset** (`06` §2.9): `MESIMON_THEME` env stands in for the `[ui] theme`
  config rung (no config system yet); `MESIMON_COLOR` for `--color`. `CSI ? 996 n` and
  `CSI ? 2031` live re-theming are deferred to M6 together (2031 never armed → nothing to
  disarm → `test_dsr_2031_disarmed` defers with it). The two terminfo rungs are deferred:
  terminfo is a guaranteed false negative under tmux; env rungs carry the weight. OSC 11 goes
  through `terminal-colorsaurus` 1.0.3 (reads `/dev/tty`, sidesteps the T-4 stdin race).
- **Card anatomy** (`07` §4): the spawning/idle-other states render NO badge glyph —
  `07` §4.1's "glyph pair present only when abnormal" wins over `06` §10.3's gallery, which
  shows `▸`/`◦` on resting cards. Session liveness appears in the meta-strip dots instead.
  Amended 2026-08-30: a RUNNING card (and running session rows) carries an animated grey
  spinner (braille frames; `|/-\` on ascii) — the ticking age alone read as ambiguous. This
  pulls one slice of `06`'s M6 animation forward; frames and cadence are hardcoded
  (`glyphs.rs`), configurability stays deferred with the rest of M6.
  Tag-pip and stage zones are zero-width (no tags field; D33g), so a session-less card is one
  line with no age (created-at staleness needs a parsed timestamp — deferred).
- **Density is `normal` only**: `compact`/`detail`/`map` and the `z` ladder are M6. The
  accordion is the 1+3 normal form (short key + top-2 sessions; branch/diff line has no data).
- **Cursor-column treatment minted** (spec gap): header cell 0 painted `bar.cursor` weight
  (`#` in mono) + column name value step `dim1 → base`. Zero chroma.
- **Spine is render-only**: auto-collapse under `MIN_COL` pressure with the window sliding on
  `h`/`l`; `z o`/`z c` manual collapse and collapsed-by-default terminal columns deferred
  (needs a column `kind`). Width hysteresis (`06` §6.3) deferred — no child PTY resize exists.
- **MOVE ghost**: the held card renders with `bar.cursor` + the selected surface in the target
  column, but the origin column still shows the card in place (M1 behaviour kept) — `06`
  §10.13's dimmed origin slot is M6 polish.
- **Ticket screen skeleton** (`07` §14): metadata header + one-line board strip + `DOCUMENTS (0)`
  placeholder + the SESSIONS rail (two rows per session; a waiting session shows its `detail`
  question). The ticket-directory IA (`ticket.toml`, `spec.md`, `notes/`) is NOT created —
  zero wire/daemon/store changes; `e`/`n`/`[` scrollback/`\` rail toggle/`a` adopt/`o` observe
  defer with it. `Mode::Pick` and the centered picker are deleted; unfocus with >1 session
  returns to the ticket screen, else the board.
- **Keymap**: board `Enter` opens the ticket screen (the direct-focus fast path moved onto the
  ticket screen's `Enter`); board gains `a` as a create alias (07 §16.2's `a  add here`).
  Inside TICKET, `c`/`s` follow `04` §2.6 (claude/shell, focus-or-spawn; `C`/`S` force new) —
  so `s` means shell on TICKET but claude on BOARD until the M6 keymap pass. Picker keys
  `z`/`p`/`x` re-homed onto the rail selection. Digits stay unbound (D34.3).
- **`✓` done renders with no decay** (`06` §3.6 `seen_at` is M6); `err`/`attn` never decay.
- **Age slot vocabulary** is `now/…s/…m/…h/…d/…w/>1y` from `state_changed_at` only.
- **Goldens**: hand-rolled text goldens under `mesimon-tui/testdata/golden/`
  (`MESIMON_UPDATE_GOLDEN=1` regenerates) — not `insta`. Colour/SGR laws are asserted
  cell-wise over `TestBackend` (`test_attn_provenance*`, `test_no_banned_sgr`,
  `test_no_drawn_structure`, `test_alarm_never_dimmed`, `test_cursor_column_header`).

## M3.5 dogfood round (2026-08-30)

- **`SessionStart` → `idle`, not `running`** (`11` §11.7.3 amendment, attention.rs transition
  table): a session that just started sits at the prompt; fresh spawns were reading "working"
  forever. Exception: `SessionStart{source: compact}` fires mid-turn → stays `running`.
- **Ticket screen header is a breadcrumb**: `mesimon > repo !N > title` (needs-you glyph +
  count beside the repo name, accent colour), then one identity line
  `KEY ∙ COLUMN ∙ created by you AGE ago` — the full board strip of `07` §14.2 is dropped
  (author: no need for all columns). Creator is hardcoded "you" (single-user v0.1, D33f).
- **`h`/`l` adjacent-ticket on the ticket screen unbound** (author: a ticket screen holds one
  ticket) — `04` §2.6's binding is rejected, not deferred.
- **Session rows drop the kind letter**: `claude`/`bash` word only (the `c`/`b` letter of
  `07` §4.3/§14.2 read as noise).
- **Rename/create are in-place edits** (`r` on board card, `r` in the ticket title; `o`/`a`
  create edits a phantom card at the column tail): hardware cursor per `06` §5.7, tail kept
  visible; footer shows only `NEW`/`RENAME  enter save ∙ esc cancel`.
- **Marquee reveal** for the cursor card's truncated title (hold 1.2 s, 1 cell / 200 ms,
  hold, loop) — an M6-animation exception pulled forward by the author; motion is
  cursor-triggered only, so L5/idle-zero-frames applies to every card except the one under
  the cursor.

## M3.5 dogfood round 2 (2026-08-30)

- **Short keys hidden from the UI** (author: `T-N` confuses): dropped from the ticket identity
  line, the board accordion, the grace row, and the import status. D24's key survives in the
  data model and the wire — this is presentation only; resurface it when branches/worktrees
  give it a job.
- **Selection must not move what was already visible**: accordion session rows right-align
  their state glyph + age into the resting card's dot/age columns, and a session-less cursor
  card renders one line with NO breathing rows (nothing below it shifts).
- **Session-kind mark**: `✻` U+273B for claude (the Claude Code banner mark; Emoji=No,
  Neutral = 1 cell — font coverage across the 06 §13 seven faces unverified), `$` for bash,
  `*` ascii tier. Used in the accordion and the ticket rail; the rail also drops the sid8
  (same identifier-noise rationale as short keys).

## M3.5 dogfood round 3 (2026-08-30)

- **`created_at` is `@<epoch-secs>`** (server.rs `now_iso` — the name lies): the TUI's
  `created_at_epoch_ms` parses that form first, RFC3339 as compatibility. The "created by you
  X ago" age was silently absent because the parser only spoke RFC3339.
- **Ticket rail is one line per session** — the state word line under every row read as a
  selectable item of its own. The glyph carries the state; a second line exists only for a
  waiting session's question or for badges, and it wears the row's selected surface so it
  reads as part of its session.
- **PTY headroom hidden below 80% of the OS cap** (round 3 addendum): `ptys U/T` was
  machine-wide noise (high-water `/dev/ttys*` count on Darwin, B-D16); it now appears only at
  ≥80% of `kern.tty.ptmx_max`, as `ptys U/T ∙ close to the limit` in full-value grey — never
  the accent (L3). The "close N idle sessions (reclaim X MB)" suggestion shape stays with the
  M6 offer-banner work.
- **Header is the breadcrumb component on every screen** (round 3 addendum): ` mesimon > project`
  (project bold, 1-cell page pad), needs-you as `!N` beside the project on both screens —
  `07` §2.2's separate `needs you N` word form is superseded; the board header count is
  tickets-on-board, not live sessions.
- **Restart demotes activity claims** (round 3 addendum): `state_for` gained a
  `hook_instrumented` flag — after a daemon restart, a Claude record's persisted
  `Running`/`Spawning` (and pane-proven Exited/Unknown/Sleeping) becomes
  `Unknown{DaemonRestarted}` ("unavailable ?") instead of being trusted: the `Stop` that ended
  the turn may have fired while the daemon was down, and nothing polls to ever correct the lie.
  Sticky claims (RequiresAction/Idle/Throttled/Failed) still survive. Bash keeps the old
  mapping — no hook stream, a live shell pane is trivially Running.
- **Focused tmux status line renders the breadcrumb** (round 3 addendum): ` mesimon >
  project !N > ticket title ` replaces `FOCUSED` + the sid16
  (`window-status-current-format` now empty). Set live via `set-option` at FocusStart and
  refreshed from `broadcast()` while focus is held (dedup against the last pushed string);
  the conf default is a bare ` mesimon ` for panes attached outside the focus flow (gate).
  `!N` renders terminal yellow (both flavors' 16-colour attn, 06 §2.7) popped
  `#[noreverse]` out of the reversed bar. Titles pass `tmux_text` (## escape, quotes/controls
  stripped, 48-char cap). tmux chrome is backend-owned display — the daemon still never
  styles a wire string (D22 §2.9 intact).
- **Breadcrumb leaf + wording** (round 3 addendum): the focused status line appends
  ` > session` in bold — the pane's OSC-0 title when the agent named itself (tmux reports
  the hostname when it never did — filtered), else the kind word. Cached at FocusStart
  (`focus_label`); broadcast refreshes never query tmux. `Ctrl+] board` → `Ctrl+] back`
  (conf + pushed live for pre-existing servers).
- **Resting meta strip CUT; blank rows reinstated** (round 4, 2026-08-30): the session-dot
  line duplicated the aggregate glyph (a lone `?` under a `?` card) — a resting card is now
  always ONE line, per-session detail lives in the accordion + ticket rail (`07` §4.5's strip
  has no job until tags exist). And `07` §3.1's zero-gap stacking lost to `06` §5.5 in
  dogfood: one blank row between cards is back ("hard to separate them with the eye").
  `a  add here` renders only when the cursor is on the empty column.
- **Cursor-column treatment reminted** (round 4 addendum): the header's 1-cell `bar.cursor`
  read as another card (same vocabulary as accent bars). Now the cursor column's header row
  is a full-width painted band on the `selected` surface, name at `sel.base` bold — a shape
  cards never take. Mono/Ansi8 fall back to the sanctioned reverse; light-256 (no painted
  selected) relies on the name's value step alone.

## D25 worktree-trust A/B resolved (2026-08-30, docs/spikes/D25-worktree-trust-ab.md)

Live A/B: a fresh worktree of a trusted repo shows **no trust dialog** and mints **no**
`~/.claude.json` entry on a bare visit. D25 (trust keyed to the main repo) stands; the M0
doubt ("worktrees have own ~/.claude.json entries") is retired — those entries were session
artifacts. **M4's worktree spawn needs no trust gate and no config writes.**

## Tier A− refuted; Esc-interrupt catch rebuilt on pane activity (2026-08-30, docs/spikes/S-E-esc-interrupt-tier-a-minus.md)

- **`11` §11.1 rows #9/#10 and §11.6.2's triad are dead on current Claude Code (2.x, under
  tmux)**: no OSC `9;4` is ever emitted, and the OSC 0 title glyph is a static `✳` in every
  state (label changes only) — it no longer distinguishes computing from waiting. §11.7.3's
  interrupt row (`Running` + `9;4;0`, no `Stop` ≤3 s → `Idle{Interrupted}`) therefore has no
  trigger as written.
- **The interrupt emits nothing at all** (dogfood forensics): no hook frame (corpus [D] claim
  confirmed live), no `Notification` in 15+ min, and — new — an Esc landing before the first
  assistant output appends **no transcript record** (no `isAbortedMidStream`), so the tail
  `Aborted` classifier cannot cover it either.
- **Rebuilt as the pane-activity quiet probe**: `Running` Claude pane of ours with
  `#{window_activity}` age ≥ 8 s → `Idle{Interrupted}`, conf `medium`, after the standard
  1.5 s settle; demotion-only (promotion stays hooks-only). Empirical basis: working turns
  hold the age at 0–1 s (spinner repaints sub-second); idle panes emit only sparse statusline
  bursts. Seam `MESIMON_PANE_QUIET_MS`; e2e `interrupt_e2e`.
- **Amended same day (T-50 forensics): the transcript IS the primary catch; the quiet probe
  is the fallback.** Two of the bullets above overstate the spike: (1) a mid-turn Esc (the
  common case — the pre-first-output Esc is the exception the spike happened to hit) DOES
  append a transcript record — a `user` record carrying `interruptedMessageId` and the text
  `[Request interrupted by user]`; (2) "idle panes emit only sparse statusline bursts" is
  false right after a turn — post-turn/post-interrupt painting held `#{window_activity}` at
  age 0 for 60–80 s live, so the 8 s probe left an interrupted card on "working" until the
  user killed it. Fix: `classify_tail_record` treats `interruptedMessageId` as `Aborted`,
  and `poll_tails` gains an abort-only candidacy class for our own `Running` sessions
  (only the `Aborted` hint is forwarded; everything else, `StaleQuiet` included, stays
  hooks-owned) → `Idle{Interrupted}` at Low conf within one 2 s poll + settle. The quiet
  probe stays for the recordless case. E2e `interrupt_tail_e2e`.

## Exited conversations stay resumable (2026-08-30, dogfood)

- **The ticket rail carries one resumable corpse** — `07` §14.2's "one row per process" is
  amended: the latest exited claude conversation on a ticket stays on the rail (dim row,
  `enter resumes` hint) unless `x` dismissed it (`Exited{Killed}` stays hidden, and `x` on
  the corpse row itself dismisses it). Enter sends `ResumeSession`; the daemon already
  accepted resume on paneless records — the affordance was the missing half. Motivating
  case: takeover of an adopted session, then quitting claude in-pane (double ctrl+c) made
  the conversation unreachable from its ticket. `c` still means "focus a LIVE claude or
  spawn fresh" — corpse resume is Enter's, deliberately.
- **Drawer re-import reuses the corpse record**: `attach_external` now returns the existing
  non-live record (and its ticket) for a re-imported `claude_session_id` instead of minting
  a duplicate record + ticket — the completion of `rescan_external`'s "re-importable, not
  shadow-banned" rule. An explicit target ticket still retargets the record.

## Daemon-restart recovery borrows the observe tier (2026-08-30)

- `11` §11.7.3's "daemon restart → `Unknown(DaemonRestarted)` → reconcile" chain had no
  re-derivation step for OUR sessions: reconcile is honest (never trusts stale activity
  claims), but a restart mid-turn then showed "?" until the next hook boundary — minutes,
  while Claude visibly streams (dogfood 2026-08-30; frequent, because dogfooding on the repo
  itself means killing the daemon at every rebuild). Spawned/takeover Claude sessions now
  become transcript-tail candidates **while `Unknown{*}` only**: the tail re-derives state at
  Low confidence (`AssistantText` → `running`, `turn_duration` → `idle`, …); leaving
  `Unknown` drops the cursor and hooks own the state again. E2e: `restart_e2e`.
- **Mint-time backfill amends "history is not activity" (`09` §4.3) for exactly this case**: a
  turn that ENDED before the restart never grows its transcript again, so an at-EOF cursor
  would leave the card at "?" until the next prompt (dogfood 2026-08-30: 3 of 4 sessions).
  For an `Unknown` session only, cursor mint classifies the LAST uuid-bearing record —
  `turn_duration` → `idle (done)`, aborted → `idle (interrupted)`, pending
  `AskUserQuestion`/`ExitPlanMode` → the reason at Low (never announced, never queued),
  trailing assistant/user record quiet-gated by file mtime (45 s). One bounded 64 KiB read;
  the walk stops at the first uuid record so an older `turn_duration` can never claim a
  freshly-started turn is done. The §4.3 rule's INTENT (no replayed announcements) is
  preserved — Low confidence structurally cannot announce.

## Header sleep suggestion (2026-08-30) — a first sliver of D23's offer tier

- The header resource line grows `∙ free ~X.XGiB (sleep N in done)` when sessions on
  sleep-safe tickets pass the D23 floors (same `sleep_eligible` predicate the keys use —
  the suggestion never offers what a keystroke would refuse). Payoff first, the how in
  parens, grey ramp; hidden under 0.1 GiB. Suggestions over shortcuts (author 2026-08-29);
  D35.1 still holds — nothing sleeps without a keystroke, no timer, no `[sleep]` config.
- **Sleep-safe is hardcoded to the `DONE` column** until M5's column policies land, where it
  becomes per-column `sleep = never|offer|auto`.
- RSS measurement now keeps its per-session split (`parse_ps_rss_by`) to price the offer.

## Board transcript peek (2026-08-30) — `p` on BOARD

- **`07` §4.3's "identifiers and glyphs, never sentences" is amended for one opt-in row
  group**: with the peek toggle on, the cursor card's accordion appends the latest assistant
  reply (≤4 wrapped rows, `~` cut marker, `sel.dim1`, indent 2). Off by default; the resting
  board is unchanged. The full sentence still lives one focus away — peek answers "what did
  it just say" without leaving the board.
- **`04` §2.4's BOARD `p` (duplicate the yanked ticket) is displaced**: duplicate is
  unimplemented, and `p` = peek is the product's own mnemonic (INBOX `p`, `04` §2.10). The
  M6 keymap pass re-homes duplicate.
- **Mechanism**: zero daemon/wire changes. The snapshot already carries `transcript_path`
  per session record; the TUI reads the file's last 64 KiB at draw time (census-style
  reverse scan for the last assistant text block, `core::adopt::classify_tail_record`)
  behind a (len, mtime) cache (`tui/src/peek.rs`; one entry then, one per path since the done-mark decay
  mark below) — steady cost is one `stat` per frame for the single peeking card. Text is
  sanitized before render: control chars and
  the drawn-structure range 0x2500–0x259F stripped (the L1 no-drawn-structure law holds for
  transcript content too). Highest-precedence session with a transcript wins (bash rows
  never have one).

## Opt-in binary update reload + daemon-restart resilience (2026-08-30)

New decision — the corpus is silent on updates. Dev rebuilds and production upgrades are
one mechanism: a newer mtime on the executable at our own path, held for one 2 s check
interval (`tui/src/update.rs`; the hold debounces a binary still being linked).

- **Never automatic** (nodeterm lesson: the human owns lifecycle). The header shows a grey
  `update ready (U reloads)` offer — dim2, like the sleep suggestion; attn stays
  needs-you-only. `U` (global, both screens, not in text input) sends `Command::Shutdown`
  (already daemon-supported: persists, breaks the msg loop, removes sockets), waits for
  `orch.sock` to vanish (flock race), then `exec`s the new binary in place
  (`tui/src/lib.rs::reexec`). The fresh TUI's connect-spawn brings up the NEW daemon.
- **Daemon restart is the designed story, not damage**: tmux + agent sessions survive;
  records reconcile to `Unknown{DaemonRestarted}` and re-derive via the observe tier. Hook
  settings keep working across the swap because they exec `mesimon hook` by path and the
  path is stable — an in-place swap means old sessions run the new hook code (softens the
  CLAUDE.md rebuild trap for same-path rebuilds).
- **The TUI no longer dies with the daemon**: `client.rs` holds a reconnectable `Conn`; a
  dead connection keeps the last board on screen, shows `daemon unreachable ∙ reconnecting`,
  and re-dials every 2 s (reopening spawns the daemon when it is truly gone; a protocol-
  version refusal from a newer daemon surfaces in the status line). Transport failures
  reach key handlers as ordinary `Response::Err` via `App::req` — no mutation is ever
  blindly replayed after a lost reply.

## Answered question clears mid-turn — narrow PostToolUse pair (2026-08-30, dogfood)

Dogfood find: an `AskUserQuestion` ticket stayed needs-you AFTER the user answered, until the
whole turn ended (Stop). Nothing in the registered set fires when the answer lands — 11 §11.2.5
banned PostToolUse wholesale as verbose tier.

**Supersedes 11 §11.2.5's blanket PostToolUse ban**: the ban is now read as broad-matcher only.
The registered set gains ONE `PostToolUse` entry with the same narrow
`AskUserQuestion,ExitPlanMode` matcher as PreToolUse (31 entries total,
`daemon/src/hook_settings.rs`). Completion of either tool = the user answered; ingest maps it to
`Signal::PostToolUse` and the machine drops `RequiresAction` → `Running` (a leave, so the
1500 ms settle applies). Interrupting the question instead of answering still fires nothing —
that path clears via the next `UserPromptSubmit`, or the 15-min stale demote.

Existing sessions spawned before this change carry the 30-entry settings file and keep the old
behaviour until respawned (hooks are injected at launch).

## Accepted permission clears mid-turn — PostToolUse goes broad (2026-08-30, dogfood)

Same shape one tool wider: a GENERIC permission (`RequiresAction{Permission}`, e.g. a Bash
approval) also stayed needs-you after the user accepted, until Stop. 11 §11.7.3 already said it
plainly — "There is no permission answered event"; the allow path is the tool's own
`PostToolUse` — but the narrow matcher above meant no frame ever fired for it.

**Supersedes the previous entry's narrow-matcher reading**: the single `PostToolUse` entry now
carries matcher `*` (still 31 entries). Ingest keeps the sharp `AskUserQuestion`/`ExitPlanMode`
signals and maps every other completion to `Signal::ToolCompleted`, which the machine honours
ONLY from `RequiresAction{Permission}` → `Running` (1500 ms settle) and ignores everywhere else
— so a parallel sibling's completion cannot clear a held Question/Plan. The broad matcher is one
short-lived `mesimon hook` exec per tool completion; the verbose-tier ban stays in force for
broad `PreToolUse`, `PostToolBatch`, `MessageDisplay`.

Known miss, accepted: with no join key (`PermissionRequest` carries no `tool_use_id`, 11
§11.2.4), a parallel sibling finishing while a DIFFERENT generic dialog is held clears the badge
early; the idle `Notification{permission_prompt}` re-asserts it at Medium, so the miss
self-heals. Human DENY still fires nothing — that path clears via the next
`UserPromptSubmit`/`Stop` as before.

Existing sessions keep the narrow matcher until respawned (hooks are injected at launch).

## Board grab key is `>`/`<`, not `m` (2026-08-30, dogfood; final shape same day)

04 §BOARD's `m grab` is superseded. Final gesture ("move the item from the first press,
blinking, pending"): the first `>`/`<` grabs the cursor card AND shifts its ghost one column
that way immediately (wrapping at the ends) — a pending, blinking move, nothing sent to the
daemon yet. The same key again (or `enter`) commits where the ghost stands, so `>>` / `<<` is
one column in one gesture; the OPPOSITE key cancels outright; `esc` cancels; hjkl fine-place
meanwhile (the grab direction rides in `Mode::Move::grab`). Every commit goes through the same
ghost drop, so the card keeps its row, `idx.min(ghost_len)`. Board `m` is unbound (free for a
future board-level merge mnemonic; ticket-screen `m` merge is untouched). This retires two
earlier same-day shapes: single-press immediate move (appended to column end), and
grab-in-place-then-double-press. Doc 04's keymap table gets the sweep in M6.

## In-app /resume is a conversation handoff, not an exit (2026-08-30, dogfood)

Implementation-verified refutation of the 11 §11.2.3 reading that every `SessionEnd` marks a
dead session: Claude Code's in-app `/resume` fires `SessionEnd{reason:resume}` +
`SessionStart{source:resume}` **in the same live pane** — the process survives, hosting a
different conversation. Honoring the End as an exit stranded a live session as a corpse, and
the next rail Enter killed the user's real pane and crash-looped
`claude --resume <record-uuid>` on a conversation that never existed ("No conversation
found", exit 1, forever — the argv was replayed verbatim). Dogfood case: T-36.

The fixes, all landed together:

- **Attention machine**: `SessionEnd{Resume}` is inert — no transition from any state, and it
  never refines an already-exited reason (`ExitReason::Resumed` is now unreachable from hooks;
  kept for serde back-compat). Real death still arrives as `PaneDied` (T-7 authority).
- **Identity relearn**: the `SessionStart` frame's `transcript_path` filename IS the
  conversation id — `on_hook` derives `claude_session_id` from the stem when it differs from
  the record's minted uuid (and clears it on a handoff back). The one deliberate exception to
  D24's "identity is never discovered": the handoff surfaces nowhere else.
- **`resume_argv` never replays a stale target**: the identity flag (`--session-id` OR a
  previous `--resume`) is rewritten to `claude_session_id.unwrap_or(rec.id)` on every resume.
- **`resume_transcript_missing`** trusts the record's `transcript_path` only when its filename
  matches the target id; otherwise it scans the projects dirs for `<target>.jsonl`.

## Kill ≠ dismiss: `Killed` corpses stay on the rail (2026-08-30, dogfood)

Amends "Exited conversations stay resumable" above: `x` on a LIVE session kills the process
but the conversation survives — its corpse now stays on the rail (state word `killed`, grey,
not `FAILED`/Err; a deliberate kill is not a failure). The rail hides only the new
`ExitReason::Dismissed`, which `kill_session` records when the target is already non-live —
i.e. `x` on a corpse row is the dismissal gesture, exactly as before, but it no longer
shadows the kill semantics. Dismissed corpses remain drawer-re-importable (only live records
shadow-ban drawer rows).

## MOVE ghost blinks in place (2026-08-30, dogfood)

"I press `<`, I expect the ticket to blink in place; h/l move the blinking ticket." The grabbed
card (MOVE mode's held ghost) blinks its title until dropped or cancelled: an 800 ms square wave
down the sel ramp — `sel.base` 400 ms / `sel.dim3` 400 ms (`Theme::move_blink`; BOLD rides both
phases, it is the cursor-title treatment, not the blink). This supersedes D19/06 §8's blanket
blink ban for exactly this one element — grab feedback is the point — via luminance repainting
on the redraw clock, never real SGR blink. Grey ramp only, so the one-saturated-colour law holds
(`move_blink_rides_the_sel_ramp_only` + `test_move_ghost_blinks` are the spec). Mono degrades to
the steady cursor treatment (its ramp is all Reset; the reversed surface marks the grab).
While the move is pending the card at its ORIGINAL spot stays visible semi-transparent — dim3
title, ghost-weight bar, even an attn card demotes there (the ghost carries the weight;
`test_move_trail_is_semi_transparent`).

Two abandoned detours the same day (misread of "this session is currently moving" as the
WORKING state): a breath, then a hard blink, on the running spinner glyph — both reverted; the
working glyph is back to the rotating spinner at steady `dim2`, and D19's "working is
motion-free" stands except for the rotation itself.

## Ctrl+5 unfocus needs its own bind — extended-keys refutes rung 1b's "same byte" (2026-08-30, dogfood)

docs/04 §2.14 rung 1b claims `Ctrl+5` is a free alias of `Ctrl+]` because both are legacy byte
`0x1D`. True at the terminal (verified: `cat -v` outside tmux shows `^]` for both, iTerm-class
terminal, English AND Hebrew layouts) — but our own conf sets `extended-keys always` (spike T-6),
so tmux negotiates CSI-u with the outer terminal and `Ctrl+5` arrives as the DISTINCT key
`CSI 53;5u`, never folding into `C-]`. The lone `bind-key -n C-]` therefore misses it and the
byte falls through into the focused pane. Fix: an explicit `bind-key -n C-5 detach-client` in the
conf plus the live-server replay in `set_status_left` (running servers never re-read conf).
Status-right and the first-run gate message now name both keys.

The motivating failure is D20's scariest-failure prophecy come true on Hebrew layout: brackets
mirror under RTL, so Ctrl+physical-`]` emits `0x1B` = Esc — straight into the Claude pane as an
interrupt. No bind can intercept that (it IS Esc); `Ctrl+5` is the layout-safe unfocus. Rung 1b's
mechanism survives, its "no code change" conclusion does not.

## M4b read-only diff viewer shipped (2026-08-30)

Plan §H's adjudications, recorded at ship:

- **Diff view is read-only** — `08`'s accept (`a`/`A`), selective checkout (`X`), send-back
  (`s`), land (`L`), both caches (§3.1), and the PostToolUse invalidation trigger are all
  **deferred, not dead**: the plumbing (`--raw -z --abbrev=40 --find-renames`, per-file
  `-U{n}` on demand, the `FileDiff`/`Hunk`/`Render` model, S/D/U display flags) shipped as
  specified and actions can land on it later. Entry is ticket `v` (attached | evicted,
  column-agnostic per D34.7); `!` opens `$SHELL` in the worktree via the focus-handover
  path — `08` §1.4's configurable `[review] external` + consent grant deferred with the
  actions.
- **`--raw` beats `--name-only`** (08 §0.1 over 12 §12.7.4) — implemented as written.
- **Per-file rename diffs need `--find-renames` + both pathspecs** — `08` §1.2's per-file
  command run verbatim on a renamed file emits an add-only patch (or nothing); the daemon
  adds `--find-renames -- <old> <new>` for `R` entries. Flag table otherwise verbatim.
- **Binary detection is the patch marker alone** — `08` §1.3's "numstat `-` `-`" detector
  misfires on pure renames (also no counts, empty patch); `Binary files … differ` /
  `GIT binary patch` is the classifier, numstat `None` only feeds the badge.
- **`WorktreeItem.path` narrows 12 §12.10.1** — oids stay daemon-side, but the worktree
  path now travels in the snapshot (attached bindings only) solely so the TUI can hand
  `!`'s shell its cwd.
- **Wire**: `DiffList`/`DiffFile` are served on the daemon's connection threads (never the
  writer), bounded by a 2-permit pool, authorize()d explicitly (`Action::Read`,
  `Resource::Ticket`) — D22's single-writer rule now has a documented read-only bypass
  lane, D32c's chokepoint held on it. Responses serialize before taking the socket writer
  lock (the same handle broadcast() blocks on).
- **`08` §10.1's drawn rules translate to painted bands** — the ASCII mock's `│`/`────`
  are L1-illegal in the shipped design system; hunk headers are `selected`-surface band
  rows, the pane divider is a 3-cell gap. Breakpoints per §10.2 minus the dead rail
  (D33k): two panes ≥100 cols, single pane + `z p` swap below. `z z` density and `R`
  recompute (no caches, §3.1 deferred); ahead/behind stays `—` all of M4.
- Stop-of-a-bound-session recompute deferred: the TUI has no per-session Stop signal
  distinct from BoardChanged; `R` covers it.

## M4b dogfood round 1 (2026-08-30)

- **`!` shell handover killed the TUI silently** — system(3) semantics were missing: with raw
  mode off, a Ctrl+C before the child takes the terminal hits our process group too; default
  SIGINT killed mesimon while the interactive shell survived, stranding the user inside it
  ("crashed, moved my pwd to the worktree"). handover now ignores SIGINT/SIGQUIT for exactly
  the child wait (latent since M1's attach — tmux grabbed the terminal fast enough to hide it).
  An interactive shell's exit status (= its last command's) is also no longer reported as
  "focus failed"; only real attach argvs surface non-zero.
- **Adds/deletes get colour** (author, amending the ship block's glyph+weight-only rule and
  06's grey-ramp diff line): add lines ride the **calm** register (muted green, bold), delete
  lines the **err** register (muted red) — theme/profile-aware registers, never raw RGB, so
  the one-saturated-colour law and mono legibility stand (glyph + weight kept; cell test
  `test_diff_add_del_registers`). Intraline ("greener/redder") emphasis deferred — needs the
  `similar`-based word diff from 08 §1.2.
- **Density words**: `-U1/-U3/-U8` read as noise — identity line + `z z` status now say
  `tight` / `normal` / `wide` ("N context lines around each change").
- **`{` / `}` (+ PgUp/PgDn) page the hunk pane** (~20 rows, draw-clamped). `v diff` joined
  the ticket footer hint (the viewer had no visible affordance).

## M4b dogfood round 2 (2026-08-30)

- **Full-line add/del grounds** ("like normal diff viewers"): page bg blended one step toward
  the calm/err registers (`Theme::diff_add_bg/diff_del_bg`; graphite 0x1E2C28/0x2E2127, chalk
  0xDFEBE4/0xF2E0E4) — TrueColor both flavors only; the indexed cube has no tint this quiet, so
  256/16/8/mono keep the fg-register + glyph encoding. Ratatui fact worth keeping: a Line's
  style paints only its TEXT cells — full-row grounds require padding the row to the pane
  width by hand (the M3.5 "painted band" empty-Line idiom paints nothing; the diff hunk band
  now pads too).
- **FILES pane widens to 36 at ≥140 cols** (08 §10.2's outline width; 28 below), and the
  selected row's overflowing path reveals marquee-style — same clock behaviour as the board
  card title and the ticket rail (`DiffState.marquee`, reset on landing).

## Archive is A/V (2026-08-30, T-46)

Archive shipped as docs/13 §data-model specifies — `[archived] {at, by}` is a FIELD on
`ticket.toml`, never a directory move, undo class REVERSIBLE — with one deviation: `by` is
always `"local"` (v0.1 has no user@host/clone actor plumbing). Archived tickets stay in
`board.tickets` and every snapshot; `Board::column_tickets` is the one chokepoint that hides
them from the board, so the ticket page keeps rendering an archived ticket (badge
`∙ archived ∙ A restores` on the identity line). Restore is exact: `column` and `order`
survive archival untouched.

Keymap (04 unaware; its `Space t a` leader at 04:1185 and 07 §13.4's `z A` toggle are both
unbuilt — the M6 sweep stands): BOARD/TICKET `A` archives (on an archived ticket's page `A`
restores), BOARD `V` opens the archived-list dialog (`jk`/`enter` opens the ticket page/`A`
restores/`esc`). Gate: refused while any session of the ticket `has_pane()` — sleep everything
first; Sleeping/Exited pass, zero-session tickets pass. Unmerged worktrees do NOT block
archive (reversible, binding + branch persist; 12's `on_ticket_archive = "evict"` deferred).
Spawn/wake/resume/move on an archived ticket refuse with "ticket archived — restore it first".

Header suggestion mirrors the sleep offer (same predicate as the keystroke): DONE tickets
holding no pane and untouched past `ARCHIVE_SUGGEST_MS` (1 h; seam
`MESIMON_ARCHIVE_SUGGEST_MS`) price ` ∙ N to archive (A on the card)` — sleeping sessions all
asleep that long, or (dogfood 2026-08-30) NO live sessions at all: a session-less or
corpse-only DONE ticket counts once the newest of `created_at` / corpse `state_changed_at`
ages past the threshold. Computed on the 1 s
tick bucket, deliberately NOT in `refresh_rss` — its no-pane early-return fires exactly when
archive candidates exist. BOARD `X` takes the offer (`ArchiveAll` — the Z/ReclaimAll rule:
archives exactly the priced candidate set, per-ticket gate re-checked, honest
archived/skipped split). chrome.rs's old "archived doesn't exist until v0.2" header comment
is refuted; the header count now excludes archived tickets.

## Alpha-1 survivability + lifecycle (2026-08-31)

Three deviations, recorded because each one contradicts text that is still
sitting in the corpus.

**1. A newer client restarts a stale daemon, with no keypress.** This reverses
02 §5.2's *"The daemon never self-restarts on its own — that would fail D6/P1
(a behaviour the user cannot see before it runs)"* and this map's own "Opt-in
binary update reload" entry, *"Never automatic (nodeterm lesson: the human owns
lifecycle)"*, which is 24 hours old. The letter of 02 survives — the daemon does
not restart *itself*, the client does it — but the spirit does not, and pretending
otherwise would be dishonest. The reasoning: the version string is frozen at
`0.1.0-dev` across every dev rebuild, so build skew is invisible, and a client
silently driving a daemon that runs last week's code is *also* a behaviour the
user cannot see — with no chip to notice and no way to find out except odd
behaviour later. Shipping to people who are not the author made "invisible until
something breaks" the worse of the two. Guard rails, all structural:
`daemon_pid == our pid` is never restarted (that is the in-process e2e daemon —
no test can trip it); only a daemon carrying `MESIMON_DETACHED=1` is restarted,
so a human's foreground `mesimon daemon --repo` is left alone; at most one
attempt per connect, then the mechanism disables itself for the process; and the
comparison is ORDERED, not merely "different" — only a strictly newer client
acts, which is what stops two TUIs of different builds from taking turns
restarting the daemon to their own version forever (semver when both name a
version, exe mtime when they agree, and nothing at all when neither can be
ordered). `restart_skew_e2e` covers both directions with the real client. Seam:
`MESIMON_NO_DAEMON_RESTART=1`. 02's `Welcome`/`Refused` frames and
`mesimon daemon upgrade` remain unbuilt; `Response::Hello` grew `build`,
`exe_stamp` and `detached` instead, where `build` is 02's `server_build`.

**2. `schema_version` applies to four STATE files, not 16 §6.2's three config
files.** `config`/`keymap`/`policy` do not exist in v0.1. The rules are adopted
verbatim (== load; absent → treat as 1; newer → refuse THAT file and do not
guess), applied to `columns.toml`, `ticket.toml`, `sessions.json` and
`worktrees.json` as four independent counters. The `older → migrate in memory`
arm is unreachable while every counter is 1 and is where the migration chain
goes when a second version lands. Two mechanical consequences worth recording:
the stamp on a ticket lives on a `TicketFile` wrapper in `store.rs`, not on
`Ticket`, so it stays off the wire and out of every fixture; and nothing uses
`#[serde(untagged)]` to accept the legacy shapes, because untagged collapses
every failure into "did not match any variant" and 13 §13.10.3 requires
file:line:column. A shape probe plus a two-pass parse keeps the span.

**3. Quarantine is 13 §13.10.3's disposition without its mechanism.** The doc
specifies quarantine for the *watcher*: a three-way merge against the last known
daemon write, per-field resolution, a resolution screen. v0.1 has no watcher and
no undo journal. What ships is the same disposition at load time only — bytes
preserved by rename to `<name>.quarantine-<epoch_ms>`, the entity excluded from
the board, a non-blocking notice carrying the parser's own file:line:column, and
`KT-C002`'s "unresolved merge conflict" rather than a syntax error pointing at
line 43. No merge, no resolution screen. Also fixed while here: `write_atomic`
had been shipping without the fsync 13 §13.9.1 requires ("No exceptions, no fast
path") since M1 — recorded as a closed deviation, not a new decision.

Notices reach the board on `Response::Board.notices` (serde-additive, like
`worktrees` before it) and render in the grace row, now an advisory row: the
header already carries five optional `∙` clauses and a sixth pushes the update
chip off a 100-column terminal. `Notice.kind` is a String, not an enum, because
an unknown variant from a newer daemon would fail the whole `Response::Board`
deserialize and the client drops lines it cannot parse — one new notice kind
would blank the board on an older client.

## The keymap became data, and the collisions are gone (2026-08-31, M6 keymap pass)

`04` Part II's tables are superseded wholesale by `core/src/keymap.rs`, which is now the
only keymap: the doc describes an intent the code no longer needs to be read against.
What the pass actually changed, and why each one:

**The mechanism (04 §2.16 built, finally).** A keypress becomes a `Verb` only through
`keymap::resolve(scope, key, &ctx)`; `App::dispatch` matches `Verb` exhaustively, so a
binding with no handler does not compile. Every binding carries an availability predicate
over `Ctx`, and that same predicate gates the key AND its hint — which is what retires the
nine drifting hint literals the audit found (`chrome.rs`, `ticket.rs`, `diff.rs`,
`board.rs` now contain none). Ten validators in the module enforce 04 §2.0's rules
mechanically. `mutates` is present on every binding, unused until the D22 observer client.

**Context-aware hints (author 2026-08-31).** The footer and `?` show only what applies right
now: an empty column offers no `rename`/`move`/`delete`, a ticket with no sessions offers no
`sleep`, `enter` reads "go to the agent" or "ticket page" depending on what it will do, `x`
reads sleep or wake. Unavailable means INERT, not merely unhinted — pressing it does nothing.
The single exception is ticket `m`, which stays bound while unhinted so it can answer "the
agent is still working"; the invariant that survives is one-directional: **a key that is
hinted works.**

**Collisions retired.** `s` was claude on BOARD and shell on TICKET — now `c` = claude and
`s` = shell on both, and BOARD has no `C`/`S` (author: want a *new* one, go to the ticket
page). `m` was grab on BOARD and merge on TICKET — now merge only, everywhere; the board
grab is `>`/`<` alone, which already did the whole gesture (this finishes the 2026-08-30
grab-key entry above, which said board `m` should be unbound and was not). `p` was
peek/pin/pane-swap — now peek only; pin is `P`, pane swap is `z s`. `z` was sleep on TICKET
and the view prefix in DIFF — now the view prefix everywhere; sleep is `x`, per 04 §2.3.
`R` was refresh in DIFF and resume in the drawer — the drawer's folded into `enter`.

**Kill is gone.** Ticket `x` used to kill a session: irreversible, no confirm, named in no
hint. Sleep already reclaims the memory and is reversible, and a session you want gone goes
with its ticket. 04 §2.9's refusal to bind a kill is upheld rather than worked around.

**`d` and `a` are chords.** `d`+`d` deletes, `d`+`D` deletes and discards the branch, `a`+`a`
archives (author 2026-08-31). A stray `d` or `a` on a card now costs nothing, and the
top-level `D` — one shift from `d`, discarding a branch — no longer exists. **Restoring is
NOT a chord**: `a` on an archived ticket acts at once, because undoing a mistake must never
be harder than making it. A chord prefix advertises only itself (`d`, `a`) — pressing it
swaps the footer to the tail's scope, which names the key still to press, so nothing ever
renders `d d` (author 2026-08-31); `chord_prefixes_advertise_a_single_key` holds that.

**Undo covers archive, and says which.** Undo moved to the GLOBAL scope so a delete made from
the ticket page is undoable wherever the user lands, and `u` now also undoes an archive
(author 2026-08-31). Archive needs no daemon-side grace band — it destroys nothing and the
ticket stays in the snapshot — so the TUI remembers the last archived id (`App::last_undo`)
and `undo_target()` re-derives it from the board every time rather than trusting it: an
expired grace band or a ticket restored from another client stops being `u`'s target. A
delete supersedes a remembered archive, so `u` always means the last thing. The hint word
follows (`u undo archive` / `u undo delete`), and goes silent when there is nothing to undo.

**Board-wide actions have no keys.** `e` (external), `V` (archived), `X` (archive-all) and
`Z` (sleep-all) are retired as bindings and live in the **Esc menu** (author 2026-08-31:
"some things should move to menu"). Rationale: they are rare, they are not about the
selection, and a menu row has room to say what it will do in words before you commit. This
also gives Esc a meaning on the board, where it did nothing. The header's offers now point
at the menu instead of naming retired keys. Shift is therefore free to mean one thing:
harden or force the same verb on the same target (`d`→`d D`, `c`→`C`).

**Escape ladder (04 §2.14, simplified to one rule).** `q` pops one level; at the board that
is quit. `Esc` pops one level; at the board it opens the menu. MOVE takes `q` now. `Ctrl+]`
/ `Ctrl+5` still pop the ticket screen (the hand is on them after an unfocus) but are
overlay-only. `Ctrl+C` still quits from the board — a deliberate deviation from 04 §2.2,
which forbids it: a TUI that cannot be Ctrl+C'd is user-hostile, and quitting the client
costs nothing (daemon and sessions survive).

**Diff navigation (the one place consistency lost).** `j`/`k` scroll the hunk pane rather
than moving the selection, because this screen is read, not picked; file navigation moved to
`n`/`N` (04 §2.3's next/previous-match keys, previously unbound). `h`/`l` and the `J`/`K`
aliases are unbound there. The `?` overlay states the exception.

**Workspace has one spelling.** The composer's Shift+Tab, and nothing else — ticket `w` is
retired (author 2026-08-31). The choice locks the moment a session or binding exists, so
creation time is the only moment it is genuinely open.

**New in 04's terms, and previously missing entirely:** `?` (per-screen which-key overlay,
`ui/help.rs`), `^L` redraw, `^Z` suspend-this-client, and MOVE's `1`–`9` column addressing
(04 §2.5). `04` §2.12's leader and command palette remain unbuilt; the Esc menu covers what
the leader was going to be needed for in v0.1.

**Not done:** doc `04`'s own tables are not rewritten — `keymap.rs` is the source and the
doc is now historical. `07` §6's footer mock-ups are stale for the same reason.

## An armed artifact monitor is not in-flight work (2026-08-31, dogfood)

**Supersedes 11 §11.7.4's `background_tasks` row** (the `Running | Stop with `background_tasks`
non-empty | Running` transition) and narrows 11 §11.2.3's `Stop` row.

The doc gates `Idle{EndTurn}` on the array being *empty*. That is too coarse, and it fails
closed **permanently**. Publishing an Artifact arms an `artifact-comment-monitor` for the rest
of the session, so from the first publish onward every `Stop` carried a non-empty
`background_tasks` and was swallowed: the rule targets `Running`, the state the machine is
already in, so `apply` returns `None` and no transition is even recorded. The pane-quiet probe
then demoted the finished turn to `Idle{Interrupted}` 8 s later at Medium, and `automove`
refuses to promote an interrupt — so the ticket sat in IN PROGRESS with the card reading
"running" and then, falsely, "interrupted".

Observed on T-72 "shortcuts UX": activity seq 114 `Stop`, no transition, seq 115
`running → idle{interrupted}` +9934 ms. Across the same log 76 of 87 `Stop` frames settled to
`idle{end_turn}` at High in ~1.5 s; the one session holding an armed monitor at Stop time is
the one that failed.

**The rule is now a classification, not a count** (`attention::task_blocks_end_turn`, applied
in `ingest::signal_of`; the `Signal::Stop` field is `blocking_tasks`, not `background_tasks`).
A `monitor` is a dormant watch on an EXTERNAL human, not work that will produce more output,
so it does not hold the turn open; `shell`, `subagent` and friends still do.

**The `.type` spellings remain unverified** — 11 §11.2.3 lists
`shell|subagent|monitor|workflow|teammate|cloud session|MCP task`, but spike S-A never captured
a live `Stop` payload (only 5/31 events fired in its unauthenticated run), so doc rule 4
applies. Hence two guards: matching is on a normalised token (`"MCP task"`, `"mcp_task"`,
`"mcpTask"` agree, and anything containing "monitor" reads as a watch), and an **unrecognised
type blocks** — keeping the old conservative behaviour for anything new rather than ending a
turn that is still running. An entry with no readable `.type` blocks for the same reason.

**Not done:** 11 §11.7.4 also asks for a card badge naming `background_tasks[0].type`. Deliberately
skipped (author 2026-08-31) — the card has no room for it, and `running` already says the
useful thing.

## Suggestions are one right-hand chip and a marked menu (2026-08-31, author direction)

Supersedes the "Header sleep suggestion (2026-08-30)" entry's placement, not its rule. The
offer tier stays exactly what D23/D35.1 allows — an offer, never an action — but it stopped
being a growing list of `∙` clauses on the left of the header.

- **One suggestion at a time, right-aligned**, in a fixed priority order: `update ready`,
  `sleep N agents`, `archive N tickets`. The chip names its route — `(esc)`, or `(U ∙ esc)`
  where a key also takes it — and says nothing about what is queued behind it (a `+N` was
  tried and cut, author 2026-08-31: it sat awkwardly between the offer and its key, and a
  header that reports queue depth is a dashboard). A chip with no room says nothing rather
  than shearing the line; scarcity (the PTY warning) outranks it for the space.
- **A suggestion is not an independent surface. It is a pointer at an Esc-menu row**
  (`keymap::SUGGESTIONS`), and its availability IS that row's `avail` — so the header can
  never offer something the menu will not do, and `menu_items` floats the suggested rows to
  the top in the same priority order. `every_suggestion_is_a_menu_row` enforces all of it.
  Esc then Enter takes the offer the header named.
- **The visual language is `◦`**, on the chip and on the menu rows it stands in front of, and
  nowhere else. Two candidates were cut by the author on 2026-08-31: `›` (reads as "you are
  here" — every terminal prompt has trained that, and a suggestion is the opposite of where
  you are) and `◊` (a full-height diamond outline, "a bit big" — louder than the offer it
  introduces). `◦` U+25E6 is a small mid-height ring, directionless, EAW=N, 6/7 present per
  §4.1. **It is §4.2's `idle`/`spawning` mark reused**, deliberately: that glyph lives on
  cards, this one lives in the chrome, and no row ever shows both. ASCII tier falls back to
  `*`, not §4.1's `.`, which is too faint to read as a mark.
- **The 0.1 GiB floor moved off the suggestion and onto the payoff.** The chip appears whenever
  there is anything to sleep; the menu row's detail spends the GiB when it rounds to something
  and says "frees their memory" when it does not. The header and the menu can no longer
  disagree about whether sleeping is worth offering — **they did, and it shipped**: the header
  gated on `reclaim_bytes >= 0.1GiB` while the menu row gated on `reclaim_sessions > 0`, so a
  small sleep offer was a menu row with no chip, and archive took the header alone (author
  2026-08-31, dogfood: "the suggestion hint showed archive even though sleep was in the menu
  as well"). Regression: `test_a_sub_floor_sleep_offer_still_outranks_archive`.
- Menu labels and details became `fn(&Ctx) -> String` so a row can carry its own count
  (`Sleep 3 agents in done`), matching the chip's words. `Ctx` gained `bulk_sleep_bytes`.
- Goldens: `board_suggestions_120x30`, `menu_suggestions_120x30`.

## rustfmt is pinned (2026-08-31)

`rustfmt.toml` (`max_width = 100`, `use_small_heuristics = "Max"`) now records the style the
workspace was always written in. Without it `cargo fmt` ran on rustfmt defaults and rewrote
every file it touched, which four separate sessions then tried to un-do by hand.

## Shift+Enter composes and asks (2026-08-31, T-5 re-run)

The composer's Enter mints a ticket and arms the fresh-ticket fast path (a second Enter starts
claude and takes the terminal). **Shift+Enter does the whole thing in one press and gives the
terminal back**: ticket created, claude spawned, the ticket title delivered as a prompt that has
already been *submitted*, board still on screen. `Command::SpawnSession` grew a `submit_prompt`
flag (serde-additive, `false` = the prefill-only behaviour that remains the default) and
`SessionRecord` grew `pending_submit`.

Three measurements decided the delivery mechanism (private tmux 3.6a, claude 2.1.251):

- **The Enter cannot ride with the text.** Prefill via `send-keys -l` followed immediately by
  `send-keys Enter` leaves the title sitting in the box forever — Claude's paste detection
  absorbs a CR that arrives in the same byte burst. This is T-5's original negative test,
  reconfirmed against a *fresh* pane rather than a warm one.
- **`SessionStart` is a START signal, not a readiness signal.** An Enter sent from inside the
  `SessionStart` hook (t+1.10 s from spawn) submits cleanly — but that was measured with a
  one-hook settings file. In the real 31-hook build the frame reaches the daemon *during* Claude's
  startup, and the first dogfood press landed **5 ms** after it and was lost (feed: `SessionStart`
  at 648165, press at 648170, no `UserPromptSubmit` ever). A single press on that edge is a race.
  So the frame only *starts* the delivery — `Startup` only, since a Resume/Clear/Compact
  SessionStart lands in a conversation that already has its prompt.
- **`UserPromptSubmit` is the ack, and the delivery presses until it arrives.** T-5 already named
  it the "prompt accepted" signal (~94 ms); the first cut simply did not wait for it.
  `retry_pending_submits` re-presses every 500 ms, up to 10 attempts, and stops on the ack.
  Verified: an Enter fired 100 ms after spawn is eaten exactly as before, and a later press
  submits — once, with the surplus presses landing in an empty box as no-ops. Retries are skipped
  while the session is not `Spawning`/`Idle`/`Running`, because a startup modal's Enter is an
  ANSWER and mesimon does not answer dialogs for the user. Giving up leaves the title typed,
  which is the ordinary spawn's behaviour.
- **The prompt must NOT ride argv.** `claude "<title>"` works, but commander dispatches a title
  that happens to name a subcommand to that subcommand instead — `claude -- doctor` runs
  `doctor` and exits, so `--` does not shield it. A ticket called "doctor" or "update" is not
  hypothetical in this repo. Keystrokes have no such vocabulary, and keeping the prompt out of
  argv also keeps `resume_argv`'s replay from re-asking the question.

`Key::ShiftEnter` is the ONE atom off 04 §2.0's legacy floor. It is admissible only because
every binding on it is gated on the new `Ctx::rich_keys` (the cached kitty-protocol probe
`init_terminal` already runs), so on a terminal that reports Shift+Enter as a bare Enter the key
is unbound AND unhinted — the user gets exactly the plain-Enter behaviour, never a half-working
one. `keymap::shift_enter_is_inert_without_rich_keys` is what makes the exception load-bearing
rather than a hole.

This does not soften the README's zero-token-injection promise: the same title is typed either
way, and Shift+Enter only decides whether mesimon also presses Enter on the user's behalf —
per session, on an explicit keystroke, the way the `m` flow's paste already works.

## Uncertain waits, it does not ask (2026-08-31, author direction)

`06` §4.2's row for `unknown` reads `?` / `?` / `dim3`. Refuted in use: a question mark is a
prompt — it reads as *mesimon asking the user something*, on a card whose whole point is that
nobody has to do anything yet. The state is "we have lost the thread and are waiting to pick it
back up" (typically `Unknown{DaemonRestarted}` for the seconds before the transcript tail or a
hook re-asserts), which is a LOADING state, so it now animates: `glyphs::waiting`, a two-dot
braille pair walking the ring (`⠉ ⠘ ⠰ ⠤ ⠆ ⠃`; `" : ,` falling on ascii, none of which is `.`,
the idle mark), off the same redraw clock as the spinner but divided by `WAIT_STEP_TICKS` = 4 —
400 ms a frame. Grey register, unchanged; the word is still `unavailable`.

Two rules hold it in place, both tested (`unknown_waits_instead_of_asking`,
`waiting_is_four_times_slower_than_working`): the waiting frames and the spinner frames are
DISJOINT, so no still frame of one can be mistaken for the other, and waiting is four times
slower — D19's motion ban already bent once for the working spinner and must not bend twice at
the same cadence (the peek's `pulse` is the third and slowest beat, 1 s).

First cut used a SINGLE braille dot. Reverted the same hour: at `dim2` on a real board it
disappeared — "2/3 running agents show no glyph at all" (author, mid-restart, when every card
was `Unknown`). Subtle is a ceiling on ink, not a licence to render nothing.

## The corpus is demoted to research (2026-08-31, author)

`docs/` stops being authority. The code and its tests are the spec, this file is the design
record, the README's three promises bind, `docs/spikes/` stays empirical, and everything else
— `00-DECISIONS.md` included — is idea stock: read for measurements and reasoning, never cited
as the reason something must be a certain way, every version and API claim re-verified at
implementation time.

The old ladder (00 binding → this map → topic-owner doc → verify) is retired, and with it the
propagation pass the 2026-08-29 review asked for: every unpropagated amendment it named has
since been settled by shipped code. What this file records has inverted accordingly — it began
as a map of which sections an amendment invalidated, and the tables above are now archaeology;
what earns a block from here on is what was built and why, and what the corpus got wrong when
someone went and measured. CLAUDE.md carries the rule.

## Ticket tags (2026-08-31, T-83)

D18's tag pips and D31b are built. Six things the corpus did not settle, or settled wrongly:

- **The key is `^t`, not `ctrl+<digit>`.** The author asked for `Ctrl+1..9`; it cannot ship.
  `no_banned_atoms` exists specifically to reject `ctrl+<digit>` (04 §1.1's table: `Ctrl+3` IS
  Esc, `Ctrl+4` IS VQUIT, `Ctrl+2/8` collide with Ctrl+Space/Backspace), and the one whitelisted
  spelling `Ctrl+5` is already `Back` — the layout-safe unfocus the Hebrew-bracket entry above
  bought. A `rich_keys` gate like `ShiftEnter`'s would work on kitty terminals only, and the
  requirement was that tags reach the **composer**, where the real constraint bites: `INPUT` is a
  barrier that types any atom it does not bind, so a bare `t` is text. `Ctrl+<letter>` is the only
  legacy-floor atom a text field cannot swallow (`keys.rs:16` matches Ctrl before the printable
  arm). Hence `^t` + a digit, and hence a fourth chord tail (`Scope::TagChord`), bound on BOARD,
  TICKET **and** INPUT. 04 §1.1's ban stands; only the author's preferred spelling is refuted.
- **The registry is board-level and persisted, in `columns.toml` at schema 2.** `Board.tags` is
  the vocabulary; `Ticket.tags` is what each ticket wears. Nothing is seeded, and "create on the
  fly" means only that: no setup step before the board is usable. Typing a name once registers it
  (`Board::register_tag`, from the daemon's `set_tag`) and it stays in the cycle after the last
  ticket drops it — 16 §config's `board.toml` guessed the right shape, wrong file. The schema
  bump is deliberate: left at 1, an older build would parse the file, ignore `tags`, and drop the
  whole registry on its next write; at 2 it refuses the file and bars its writes instead
  (16 §6.2). `ForgetTag` is the only removal, and it strips the name from every wearer in the
  same pass — a ticket left wearing a retired tag shows a pip the cycle can neither reach nor
  clear. D33g's open residue about "a memory mechanism for a *removed* tag" is answered: the
  registry is that memory, and retiring is explicit.
- **A first cut derived the vocabulary from the tickets, and it was wrong twice.** Recorded
  because the second failure is not obvious: a tag vanished when its last wearer dropped it
  (which is what the author's correction was actually about), and — subtler — the ticket being
  cycled was itself part of the derivation, so taking a tag moved it in the list and the next
  press walked back to where it started. The cycle oscillated between two values and `none` was
  unreachable. Any future "just compute it from the board" idea has to answer both. D31b's
  "stable order, config order, never by recency" is satisfied by registry insertion order.
- **`06` §2.4's chalk pip ramp is refuted on contrast.** Its `L*` 62 hexes measure **2.81:1** on
  the chalk page and **2.33:1** on the selected surface — below even the `dim3` de-emphasis floor
  the code already enforces. Re-derived at `L*` 54, same six hues, `C*` still under the 8.2
  ceiling `test_chroma_law` was pre-sized for. Graphite's hexes measured fine (4.40 / 3.40) and
  are kept verbatim. Pips are now held to **>= 3.0 on both surfaces** — above `dim3`, below the
  `dim2` body floor, because a pip is read but ambient (`test_pip_ramp_is_low_chroma_and_legible`).
- **Pips ride card line 1, not a reinstated meta strip.** D18 puts them on the meta row; that row
  was cut in dogfood for being noise, and a second line on every tagged card is a heavy price on a
  kanban. The zone is a fourth subtrahend in the title budget and collapses to **zero width** when
  untagged — which is why thirteen board goldens are byte-identical after this change; only
  footers moved. The run caps at 3 then `+N` (D31b) and the tint is dropped entirely on a trail
  ghost or an inverted needs-you row.
- **Peek is where the colour-only exception is paid for.** D31b grants tags the system's ONE
  colour-only encoding, and the grant is explicitly conditional on the full names being one
  keystroke away. `p` now reveals `#BUG #STAGING` in the accordion — which forced the accordion's
  `selected && !sessions.is_empty()` gate open, since a tagged ticket with no sessions used to
  early-return one line. The `p` hint says "replies + tags" for the same reason.

Below TrueColor the tint is abandoned wholesale (`Theme::pip` returns `dim2`), per `06` §2.4: the
indexed cube has no low-chroma hue wheel, and a *visible* tag colour is precisely the
accent-spending failure D19 forbids. Nothing is lost because the pip is the tag's first letter,
lowercase — uppercase stays reserved for "a human is required" — and the letter was always the
identity. Sanitization is at the daemon boundary (`board::sanitize_tag`, 24 bytes, control chars,
`0x2500-0x259F`, VS15/16, ZWJ), because a tag name is user text on a card row and a width
disagreement there strands a `selected_bg` cell the diff never repaints.

`15`'s "`set_tags`: NO TIER, NO TOOL, EVER" stands — tagging is human curation and there is no MCP
surface yet.

## Tags become a picker, and the pips become bands (2026-08-31, T-83 round 2)

Author dogfood found the first cut "really messy and not intuitive", plus a concrete bug. What
replaced it, and what each change refutes:

- **`^t` opens a PICKER, not a cycle.** The first cut made a digit cycle its axis blind: nothing
  on screen said what the digits held, so a vocabulary the user had to build themselves was also
  one they had to remember. Now `^t` opens a grid panel — one row per group, its tags across it,
  a `+ new` cell at the end — and `hjkl`/digits steer a cursor over it. Digits survive as the
  fast path (jump to a row, step along it on a repeat). `enter` wears or unwears, `tab` cycles
  the tint, `r` renames, `d` deletes in two presses.
- **Naming must resolve against `Scope::TagChord`, not `Scope::Input`.** The bug the author hit:
  the tag-name field borrowed the composer's scope for its editing keys, so the footer offered
  "shift+enter save + ask claude" and "shift+tab workspace" under a field that does neither. The
  editing keys are now handled directly off the `KeyCode` and only `enter`/`esc` resolve. A scope
  barrier must own its hints, not sublet them.
- **One atom, one binding per scope — even with disjoint `avail`.** `no_key_bound_twice_in_a_chain`
  rejects two `Enter` bindings gated on `tag_naming` and `!tag_naming`. So `enter` and `esc` are
  single bindings whose HINT switches on `Ctx::tag_naming` and whose dispatch branches. Worth
  knowing before designing another modal tail.
- **Pips became bands.** `D18`/`06` §2.4's "pip is the tag's first letter" is refuted by use: at
  three tags a card read `rpa+1`, which is a bar code, not a set. A tag is now a painted row
  across the bottom of the card block, and `p` writes the name onto it. Painted, never drawn —
  `─` U+2500, `▁` U+2581 and `█` U+2588 are all inside the `0x2500-0x259F` range the L1 law bans,
  so a band is spaces with a background, the trick the accent bar already uses. Below TrueColor
  the tint collapses to one grey and the band carries `#name` as text instead: six identical grey
  bands say less than six names do. Goldens capture `.symbol()` only, so bands read as blank rows
  there — `test_tag_bands_are_painted_in_their_own_tint` is what actually checks the paint, and
  `test_tag_bands_never_spend_the_accent` holds D19's line where tags now spend real ink.
- **Colour is on the registry and user-chosen.** `Tag.color: Option<u8>` indexes the tag ramp;
  `None` falls back to a hash of the name, so a tag is coloured from the moment it exists and Tab
  only overrides. Storing it on the registry rather than the ticket is what makes a recolour
  repaint every card instead of leaving old copies behind. The ramp is still the six low-chroma
  tints and there is no free-colour path — D31b's law binds harder here than it did for pips,
  because a band is far more ink.
- **`MAX_TAGS_PER_GROUP = 5`**, and groups run 1-10 with `0` addressing 10. Past five a row stops
  fitting and an axis stops being an axis.

The composer shows what it is about to tag: the phantom card carries the same bands, and the
picker marks the picks worn. The first cut showed neither, which is why the author could not tell
whether tagging while composing had worked at all.

## The transcript zone reads markdown (2026-08-31)

The ticket page's TRANSCRIPT zone showed the agent's last reply as source: asterisks and
backticks on screen, and — because `peek::sanitize` flattened `\n` to a space for the card row —
every bullet, fence and paragraph run together into one grey block. An agent reply IS markdown,
and the ticket page is the surface with room to read it, so `mesimon-tui/src/rich.rs` now parses
it and draws it. What that cost, and what it refutes:

- **`peek::sanitize` keeps newlines now.** The flattening was written for a 4-row card and taken
  as universal; block structure lives entirely in those newlines. The card is unaffected —
  `peek::wrap` splits on whitespace, so a newline was only ever a word break there — and anything
  that must stay on one row (the `Doing::Tool` step title) flattens with `text::one_line` at its
  own boundary. That is where the call belonged.
- **06 §5.1 decides the whole treatment, and it is the interesting part.** SGR 2, 3, 5 and 9 are
  banned and SGR 4 is reserved for the scope chip, so italic is not slant, strike is not a line
  and a link is not underlined. What is left is value, weight, paint and space — which turns out
  to be enough: body sits at `dim1`, emphasis steps up to `base`, strong adds bold, struck text
  drops to `dim3` (the de-emphasis token, which is what struck text means), a quote takes §5.1's
  own `›` prefix plus a value step, and a heading buys a breathing row above it because it cannot
  have a rule. No markdown role reaches for a chromatic token, so the accent stays needs-you's.
- **A code span is the elevated surface, painted.** `Theme::code_bg()` returns `selected_bg`: the
  design collapses to ONE elevated surface (theme.rs's header) and the cursor card was only its
  first tenant. A fenced block is that surface shrink-wrapped to its widest row — no border, since
  every box glyph is inside the `0x2500-0x259F` range L1 bans, and no reflow, because code that
  rewraps is code that lies. Below the paint (chalk-256, mono) the backticks survive instead of
  the treatment being faked — the same call `pip()` makes for tag tints.
- **A thematic break (`---`) is a BLANK ROW.** There is no legal rule glyph, and a blank row is
  what the ticket header already uses as its zone divider.
- **A pipe table is kept verbatim and its delimiter row is dropped.** Reflowing a table destroys
  the only thing it encodes; the header the delimiter marked is said with value instead.
- **The parse is deliberately markdown-LITE and dependency-free.** This is one message in a ~15
  row zone, not a document viewer. The flanking rules are what earn their keep: `foo_bar_baz`,
  `5 * 3` and `*.rs` are what agent replies are actually full of, and an opener with no valid
  closer is literal text.
- **The law tests now render this zone.** `test_no_banned_sgr` and `test_no_drawn_structure`
  attach a markdown transcript to the ticket screen and assert the reply is on screen before they
  sweep — a law test over a surface that is not drawn proves nothing. `rich.rs`'s own
  `markdown_never_spends_a_banned_attribute_or_the_accent` sweeps a kitchen-sink document across
  all five profiles, both flavors and three widths.

`ticket_peek_120x30` is unchanged byte for byte: prose with no markdown in it renders exactly as
it did. `ticket_richtext_120x30` is the new golden.

## The tag mark is an underline, not a glyph (2026-08-31, T-83 round 3)

Author: use the space row below the ticket, with coloured top-only glyphs; and stop entering an
edit mode to name a tag. Both landed, but the first one could not land as asked:

- **`▀` U+2580 and `▔` U+2594 are unavailable, twice over.** They sit inside the
  `0x2500-0x259F` range `test_no_drawn_structure` bans, and they are East Asian Width
  **Ambiguous** — the class `glyphs.rs` documents as having cost this project a real render bug
  (terminal spends two cells, `unicode-width` counts one, every later cell shifts and strands
  paint the diff never repaints). Lower risk than the `☰` case, which broke because Unicode 16
  *reclassified* it mid-flight, but the same class.
- **So the mark is an SGR attribute instead**: `UNDERLINED` + `underline_color` (SGR 58, ratatui's
  `underline-color` feature) on the card block's bottom row, segmented left to right when a ticket
  wears several. It has no width at all, so the hazard cannot arise, and it costs the card no row
  — which is what the author actually wanted when the per-tag band rows were rejected. Degradation
  is visible rather than silent: a terminal without SGR 58 still draws the underline in the row's
  own foreground, so "tagged" survives even where "which tag" does not.
- **Peek loses its tag role.** An underline has no room for text, so `p` goes back to meaning
  replies only; the names live in the picker and on the ticket page. D31b's colour-only grant is
  still paid for — the names are one keystroke away, just through `^t` rather than `p`.
- **`test_no_drawn_structure` covered four screens and not the picker, and a `█` cursor shipped
  through the hole.** It was in `board_tag_naming_120x30.txt`. The test now renders the picker
  open and mid-rename too, and the cursor is the real hardware cursor rather than a glyph.
- **Naming edits the cell in place.** The first cut replaced the whole row with a `new: …` field,
  which is the "edit mode" the author rejected: it hid the rest of the vocabulary at exactly the
  moment you are choosing a name that has to sit beside it.

`06` §2.4's pip letter and the painted bands that replaced it are both superseded; the tint ramp
and its contrast floors are unchanged, and `test_tag_underlines_never_spend_the_accent` moves
D19's check onto the underline channel.

## MCP for mesimon-spawned sessions, and the collision design (2026-08-31, T-84)

Three board tools, a deny-only write gate, and — the half that will outlive both — a single
move path with three named principals. `docs/15-mcp-security.md` is a 1370-line design for
this feature written before any code existed; most of its measurements held, and five of its
decisions did not survive contact with the codebase. Recorded here in the form "what it said,
what shipped, why".

**The transport is stdio over `orch.sock`, not a loopback HTTP listener.** 15 §1.3 chose one
TCP listener on the daemon with a path routing-id and an env-expanded bearer, `Origin`
validation and a 405 on the GET stream. That is a lot of surface for a problem mesimon had
already solved twice: `orch.sock` and `hook.sock` live in a 0700 runtime directory and are how
everything else talks to the daemon. A TCP port is reachable by every process on the machine
and by any browser page; it needs DNS-rebinding defence, a port file, and it carries a 60 s
per-request timer that stdio does not have. `mesimon mcp --sock <orch.sock> --session <uuid>`
is spawned by Claude, speaks JSON-RPC on stdin/stdout, and forwards one `Envelope` per call.
No new socket, no new protocol, no entry in the `sun_path` budget test.

**There is no bearer token, and that is the honest choice rather than the lazy one.** 15 §1.3's
credential table is right that env survives `cd` and argv does not survive `ps`. It is wrong
about what the credential would buy here. `orch.sock` already accepts `Principal::Local` with
the full command set from any process running as this user, so authenticating only the agent
path is theatre; and a token placed in the tmux session environment is readable from every other
pane on the private server (`tmux show-environment -t <sid16>`), so it would not even separate
agent A from agent B. The boundary mesimon actually has is the 0700 directory — the same one
`server.rs`'s hook listener already named in a comment ("the 0600 socket is the authentication —
no token in any agent env"). The thing the user asked for is nonetheless delivered, by a
different mechanism: the config rides on argv and is written to no file, so a session mesimon
did not spawn cannot reach the tools at all. If the threat model ever widens to distrusting one
of the user's own agents, the hardening path is `LOCAL_PEERPID` plus tmux pane ancestry, which
needs no protocol change.

**`Principal` grew a third variant, and this is the load-bearing change.** Before T-84,
`automove` (server.rs) and hook ingestion (`on_hook`) both ran as `Principal::Agent`, because
`Agent` was the only inhabitant that meant "not the human". The moment an agent could ask for a
move itself, that conflation became unworkable: `authorize()` could not restrict the asking
without breaking the automating, the feed could not say who moved a card, and nothing could
notice that two movers were undoing each other. `Principal::Automation { rule }` now covers the
daemon's own rules; `Agent { session }` means an agent asked. D32c invariant 1 said "two
inhabitants"; it is three, and the third was always implicit.

**One move function.** `Daemon::place_ticket` replaces three separate implementations
(`auto_move`, `move_ticket`, and what the agent path would have been). The M4 DONE gate lived
inside the human's path only, which means an automation could have routed around the one rule
that stops the board claiming something shipped when git says it did not. Everything a move
must obey is now in one place, so M5's column on-enter actions obey it by construction.

**Three guards, in `daemon/src/movegate.rs`.** A human is never refused by any of them.
*No undo*: an automatic mover may not perform the exact reverse of a move a different principal
made in the last 60 s — this is the flap the user watches, a card dragged back to TODO and
snapped forward again on the next `Running`. *Depth zero*: an automation may not fire inside
another automation's move; nothing recurses today, which is exactly why it was cheap to add
before M5. *A fuse*: six automatic moves of one ticket in 120 s suspends automation for it, says
so in the advisory row, and is cleared by any move by hand. The state is in memory deliberately
— a debounce, not a security control, so losing it on restart is correct and it needs no
schema field.

**A trap for whoever writes the next e2e: `automove` is edge-triggered, and the attention
machine has a flap guard of its own.** More than `FLAP_MAX` (4) committed state changes inside
20 s pins the machine at `Confidence::Low` for 20 s, and `automove` refuses to move on Low. A
test that sends a second `UserPromptSubmit` to an already-`Running` session produces no edge and
no move, and an assertion resting on that passes for the wrong reason. `mcp_e2e` gets both
directions of the collision out of three transitions for this reason.

**The deciding hook is a separate binary, `mesimon gate`.** 15 §2.3 wanted `type: "http"` to the
daemon, on the grounds that a command hook forks an interpreter per tool call. That argument
does not apply here — mesimon already registers a broad `PostToolUse` command hook, so the fork
cost is measured and paid — and it has a worse property: the decision would depend on the daemon
being alive. `gate` decides locally from argv (`--deny-board`, `--deny-state`), which makes it
sub-millisecond and means a dead daemon cannot make it fail open; the denial is reported to
`hook.sock` afterwards, best-effort, so it still reaches the activity feed. It is a separate
subcommand from `mesimon hook` so that the observer's "never writes stdout" invariant stays
literally true and stays testable. Hook count 31 → 32; the two `PreToolUse` entries have
disjoint comma matchers, and a test asserts they cannot overlap.

**Scope of the gate, stated rather than fudged.** `Edit`/`Write`/`NotebookEdit` carry
`file_path` as a real argument, so the check is exact; both sides are resolved (macOS `/tmp` is
a symlink, and a file about to be created has no canonical path of its own). `Bash` is not
hooked. 15 §4.8 already catalogued why a command-shape pre-filter is an evasion hole, and
hooking it unconditionally would put a blocking round trip on the one tool agents use
constantly. The README now says so in the user's own terms.

**Deviations from 15 that were the author's call, 2026-08-31:** the tools are ON for every
mesimon-spawned session rather than behind D27's consent ledger (the ledger is not built, and
`mesimon doctor --mcp` is the printable half of the promise instead); README promise 3 is
narrowed from "zero token injection" to "zero prompt injection" with the tool definitions named
as the exception; `ask_user` is NOT an MCP tool, because Claude's own `AskUserQuestion` is
already special-cased in the `PreToolUse` matcher and mesimon already routes it to the attention
rail; `add_note` (T1) is deferred until notes have any storage or surface at all;
`--strict-mcp-config` is not passed, so the user's own MCP servers still load (D7: dropping them
is subtractive magic).

**Two things 15 got exactly right and the code kept.** `initialize.result.instructions` is never
set, and `skills/list` returns `-32601` — the two injection surfaces that are invisible in
`tools/list`. And `move_ticket.to_column` is a plain string validated server-side, never an
`enum`: the valid set travels as data in `get_ticket`, because a board with a column called
`DO_NOT_SELF_APPROVE` would otherwise inject that string into every request of every session.

**Verified live on Claude Code 2.1.251**, not assumed: `--mcp-config` accepts inline JSON
strings and not only files (so nothing is written to disk); `--strict-mcp-config` exists;
`hookSpecificOutput` / `hookEventName` / `permissionDecision` / `permissionDecisionReason` are
all present in the binary; `CLAUDE_CODE_MCP_ALLOWLIST_ENV` gates MCP child-env scrubbing and
defaults to `CLAUDE_CODE_ENTRYPOINT === "local-agent"`, which mesimon never sets.

## The tag mark: seven attempts, and what it cost (2026-08-31/09-01, dogfood)

Author, looking at a real board: the underline is too thin — "can be thicker on demand?" — and
"colors are too muted. I wasn't noticable." Both were true, and the second had a law behind it.
What followed is the most-revised surface in the project so far, so the whole ladder is recorded:
each rung was cheap to build and only the board could tell us it was wrong.

**The chroma fix, which every version keeps.** The ramp moves from C\* 7 to C\* ~24 (graphite
L\* 58, chalk L\* 47; six hues 60° apart). D31b's low-chroma grant was written before anything
was on screen; at C\* 7 a tint is a grey with a rumour of hue in it. The one-saturated-colour law
(D19) is NOT repealed: `attn` keeps a 2x chroma margin and remains the only token above C\* 30,
`test_pip_ramp_is_low_chroma_and_legible` now enforces a 26.5 ceiling plus that margin, and the
contrast floor rose to 4.5 on the page ground (the ticket page paints a chip in the tint and
writes the ground onto it, so that number IS the chip's text contrast). A colour nobody sees
encodes nothing.

**Rung 1 — a painted row under the card.** Cheap (it took the blank rhythm row the board already
puts between cards, so it cost no line) and wrong on sight: "there's an extra block line below the
ticket. I only wanted colored *thick* underline." A full cell of paint is a block, and the reader
files it as another element rather than as the edge of the card above.

**Rung 2 — a double underline stroke.** SGR 4 is one or two pixels of the font's choosing and
there is no half-cell (`▀` U+2580 and friends are banned twice over: the L1 range, and East Asian
Width Ambiguous). What is available is the kitty/VTE extension `CSI 4:2 m`, two strokes in one
cell. It cannot travel in a `Style` — ratatui's `Modifier` has no bit for it and the buffer is
everything `CrosstermBackend` sees — so `tui/src/sgr.rs` wraps the terminal's *writer* and appends
the upgrade after the four bytes crossterm emits for `Modifier::UNDERLINED`. **Additive on
purpose**: `\x1b[4m` first, `\x1b[4:2m` after, so a terminal that ignores subparameter SGR keeps
the single stroke — the mark degrades to thin and never vanishes. The matcher is a four-byte state
machine because crossterm writes through a buffer and `\x1b[48;…` / `\x1b[49m` share the prefix; a
lost escape would repaint the screen wrong, which is what `no_other_sgr_is_touched` guards.
**This makes 06 §5.1's reservation of SGR 4 load-bearing** — it used to be a style rule, it is now
the precondition for a byte rewrite. Verified live: **iTerm2 3.6.11 draws `4:2`** even though its
escape-code page documents only `4:3`; `MESIMON_TAG_STROKE` reaches the other styles for terminals
where it does not.

**Rung 3 — the mark gets shorter.** A full-width rule read as a border rather than as something
the ticket wears, so it became two cells per tag at the card's content start. Better, but the
stroke still sat tight under the text — "increase space of underline? it's too high" — and a
terminal draws an underline at the bottom of the cell with no sub-cell offset to spend.

**Rung 4 — a stripe cell of its own, left of the bar.** Two tags in one cell (paint for the first,
underline stroke for the second), full cell height, no extra row. It lasted one look: a painted
cell sitting against the painted accent bar reads as **one two-tone bar**, not as two things, and
it cost every title a cell (`GUT = 1`, so the cell had to come out of the card — the single cell
left of a card is the only thing separating columns).

**Rung 5 — the bar IS the mark.** The author's call: neutral while nothing is tagged, the tag's
tint once something is, and — first attempt — the second tag as an SGR-58 underline across that
same cell. Tags cost the card nothing; the frame went back to `[bar][pad][content][pad]` and all
twenty-one board goldens went back to byte-identical. Two corrections came straight back from the
board:

- **"no tag = neutral. we already have glyph for state, no need colored block."** The bar was
  still carrying the state ladder for untagged cards, which is the same thing the glyph says. It
  is now the tag channel and nothing else. Needs-you keeps the inverted title row (the loudest
  surface the board has) and `err` moved to the glyph — `test_alarm_never_dimmed` asserts the
  register on the glyph now, and that the bar is NOT painted with it.
- **"this one has 2 tags, I see only one."** True, and the design was wrong, not the data: the
  ticket wore two tints. An underline is one or two pixels at the *bottom of a cell*, and against
  a fully painted cell it is invisible at any font size. The "two colours in one cell" idea only
  ever worked under text. `sgr.rs` and the whole double-stroke apparatus came back out with it.

**Rung 6, on trial — three homes for the second tag, cycled with `w`.** `Second::Stack` (the
default, and the author's follow-up ask: "possible to do vertical instead?") draws `▀` with the
FIRST tag in the foreground over the second tag's paint, so the split runs across the bar — a cell
is taller than it is wide, so stacking gives the fatter pair of halves. `Second::Half` draws `▌`
for the side-by-side split. `Second::Edge` paints the card's right-edge cell, which was trailing
pad. None of the three costs the card a cell (`test_the_second_tag_costs_no_width` renders all
three and compares), and `MESIMON_TAG_SECOND` picks the startup home.

**`▀` and `▌` are admitted exceptions to the L1 law, granted by the author 2026-09-01.** They are
inside `0x2500–0x259F` and East Asian Width *Ambiguous* — the class that has already cost this
project a render bug — and they are here because every attribute-only channel was tried first and
could not be seen. The risk is scoped and stated rather than hidden: it only misfires where a
terminal renders Ambiguous-width as double (iTerm2 has that setting, off by default),
`unicode-width` counts both as one cell (`the_half_blocks_are_one_cell`), and
`test_no_drawn_structure` admits exactly these two codepoints by name while still banning `▔`, `█`
and the rest of the range.

**Two things that were decided rather than defaulted, and still hold:**

- **The alarm never rides the tag channel.** `attn` keeps the inverted row and `err` the glyph, so
  a tagged card that needs you still shouts (`test_a_waiting_card_still_shouts`).
- **No cell is spent on tags.** Every home so far that asked for one — the band row, the stripe
  cell — was rejected on sight, and the two survivors both reuse a cell the card already had.

**Rung 7 — the palette gets rethought, and the block gets three loudnesses** (author, same
session). Six hues at one lightness per flavor: graphite L* 62 / C* 30, chalk L* 45 / C* 26. The
hues are chosen for separation rather than even spacing, and the 60-90° band is deliberately empty
— that is `attn`'s hue, and a tag the colour of the alert is the D19 failure however low its
chroma. Both ramps clear 4.5 on their own ground and 4.0 on the selected surface, and `attn` keeps
a 2x chroma margin (`test_pip_ramp_is_low_chroma_and_legible`, ceiling now 30.5).

A terminal has no alpha, so "opacity" is a blend toward the page ground — which is what the eye
reads as a colour receding on either flavor: a tint fades DOWN into graphite and UP into chalk.
`Theme::pip_at(n, TagLevel)` does it, and the card's own state picks the level: `Selected` full
strength, `Rest` 0.70, `Sleeping` 0.38.

**The first cut used 0.82/0.50 and both boundaries were invisible** ("sleeping vs not sleeping
looks the same"; "selecting … doesn't change the intensity"). An 18% blend is nothing on a
one-cell block — a level nobody can tell from its neighbour is not a level — so the steps widened
and the law test now asserts each one moves ≥ 12% of the ground-to-tint distance, on top of its
contrast floor. Those floors are floors and not targets: a quiet level is allowed under the
body-text floor because it is paint, not text; what it may never do is stop being a colour (C* ≥ 8
at every level, and distinct per tag).

**The same report caught two real bugs.** "Parked" was read off the aggregate card glyph (`z`), so
a ticket whose sleeping session sat behind any other glyph never reached the quiet level; it is
read off the sessions now — a `Sleeping` session and no pane. And the ladder only ever reached tag
TINTS, so on a board whose sleeping tickets happen to be untagged (which was the author's board,
in the screenshot) nothing moved at all: an untagged card's neutral block now fades and brightens
with the same three levels. The loudness says what the CARD is doing, so every card has to be able
to answer it — `test_an_untagged_block_ladders_too` is the standing check. Below TrueColor the levels collapse,
because there is one grey and no ground to fade into.

**Peek carries the tags the card cannot** (`tags::chips`): with `p` on, the cursor card shows a row
of painted name-chips under the title, above the reply. Names share the row
**longest-gives-first**, never an equal split — an equal split cut "BUG" down to make room for a
"STAGING" that then got cut anyway, and three tags all arrived as two letters and a tilde. Nothing
shrinks below three cells, and the tail drops rather than every name going illegible.

## Light/dark follows the terminal, live (2026-09-01)

06 §2.9's query hygiene said the light/dark question is asked **exactly once**, at startup, and
M3.5 shipped it that way. That is right about *where* it may be asked and wrong about *how often*:
the OS flips appearance at sunset, the terminal follows it, and a board started in the morning
keeps painting graphite onto a now-white terminal until someone restarts it. The rule that
actually holds is narrower — **never ask while anything else owns the input stream** — and the TUI
event loop is precisely the place where nothing else does.

So `detect::FlavorWatch` re-asks OSC 11 every 3 s from inside `App::tick`, and a changed answer
rebuilds `App::theme` in the other flavor. No refresh goes with it: the daemon owns none of this,
the next frame is drawn unconditionally, and ratatui's diff repaints every cell whose style moved.

**The terminal is asked, never the OS.** `defaults read -g AppleInterfaceStyle` is the obvious
signal and it is the wrong one: a terminal pinned to a dark profile stays dark through an OS flip,
and a board that followed the OS there would paint chalk ink onto a black background. The
terminal's own background is the surface the palette has to sit on, so it is the only authority —
and when the terminal does follow the OS (iTerm2's light/dark profile pair), asking it gives the
OS answer for free. It also costs nothing on Linux, where there is no OS appearance to read.

Guards, each of them load-bearing:

- **`MESIMON_THEME` disarms the watch entirely.** An explicit choice is not a starting point to be
  corrected three seconds later.
- **A terminal that cannot answer is never asked twice.** The watch is armed only if the startup
  query answered; three consecutive silences after that end it for good.
- **Never with a keypress waiting, never under a text field.** `terminal-colorsaurus` reads
  `/dev/tty` and discards everything ahead of the reply, so a query racing a keystroke eats it.
  `due()` is cheap and stays true, so both cases simply wait for the next frame — the cost of a
  late flip is nothing, and the cost of a lost character in a ticket title is not.
- **The query lives in `tick` only**, so it cannot run during a handover, a suspend, or a
  provisioning stall — the same fence the once-only rule was really protecting.

`CSI ? 2031` live re-theming stays deferred, and now probably forever: it would trade a 3 s poll
the terminal never notices for unsolicited DSRs that must be disarmed before every PTY attach, on
suspend and on every exit path — the failure mode being escape-sequence garbage typed into a live
agent. Polling has no armed state to leak. `test_dsr_2031_disarmed` defers with it.

## A shell does not spin (2026-09-01, dogfood)

**Symptom:** an open shell on the ticket rail wore the working spinner forever, with nothing
running in it. Reported as "bash showing loading state even though it's not running any command".

**Cause:** two true things meeting. The daemon pins a `Bash` session at `SessionState::Running`
from spawn to pane death — D15's "a live pane is all running means", and there is no second shell
state to move to because tmux's `pane-died` is the only shell event we get. The TUI then painted
`Running` with `spinner()` regardless of kind. So the spinner was reporting that a pane existed,
which is not what a spinner says.

**Fix:** `glyphs::is_working(rec)` — only an agent's `Running` is work. A live shell wears the
idle mark (`◦` / `.`), unmoving, and its age counts in minutes rather than ticking seconds; a
card whose only live session is a shell carries no aggregate glyph at all, which is the same
answer 07 §4.1 gives for anything not abnormal. `session_glyph` now takes the whole
`SessionRecord` rather than a bare `&SessionState` (all three call sites already had one).
`a_shell_never_spins` pins it across every frame at both tiers. The board fixture's shell was
`Idle{Interrupted}` — a state the daemon never produces for a shell — and is now `Running`, so
the goldens actually cover the real record; they did not move, because both render `◦`.

The spinner is the one place D19's motion ban bends. It may only bend for something moving.

**Not done, and deliberately:** tmux can say what a pane is actually running
(`#{pane_current_command}` — the login shell's name at the prompt, the command's otherwise), which
would make a shell running `cargo build` spin truthfully. That is an enhancement, not this bug,
and it is not free: `SessionState::Running` is load-bearing for shells in `sleep_session` and its
neighbours (`(SessionKind::Bash, SessionState::Running) => {}`, else "no live shell to sleep"), so
a real busy/idle split has to widen those gates or ride a display-only field (`detail` is unused
for shells today). Worth doing on its own terms, with those gates in the diff.

## The ticket page previews a shell's pane (2026-09-01)

A shell keeps no transcript. An agent writes JSONL the TUI reads straight off disk (`peek.rs`),
and the ticket page's preview zone renders it as markdown; a shell writes to a pty and tmux is
the only record it keeps. So the zone had nothing to show for a shell and drew nothing at all —
the one session kind where "what did it just do" is a question with an answer sitting one
`capture-pane` away.

**`Command::PaneTail { session, lines }` → `Response::PaneTail { lines }`.** Oldest line first,
non-empty lines only (`TmuxBackend::capture_tail`, which the spawn probe already used), bounded
daemon-side at 200 lines × 1000 columns. Because a tty echoes what is typed into it, the capture
holds the command AND its output with no parsing at all — the e2e asserts both, in that order,
from a real `send-keys`.

**The zone is called PREVIEW, for both kinds.** It shipped for an hour as TRANSCRIPT for an agent
and TERMINAL for a shell, on the reasoning that a transcript is what an agent said and a terminal
is what a command printed. That distinction is real and it was not worth a name: neither side is
the record — one is the last reply out of a JSONL file, the other the last screenful out of a
scrollback — and the rail row two columns away already says whether the cursor is on `✻ claude` or
`$ bash`. A heading that changes under a moving cursor costs a re-read every time and settles
nothing (author 2026-09-01, "it captures both and actually is a preview"). The earlier block
"The transcript zone reads markdown" predates the rename; that zone is this one.

Decisions worth keeping:

- **It rides the writer thread**, unlike DiffList/DiffFile. That exception exists for git, which
  can spend seconds in a packfile. `capture-pane` is one small fork the tick already makes twice a
  second, and a second `DiffCtx` to move it off would buy nothing.
- **The client pulls, on a 1 s clock** (`App::poll_shell_tail`), and only while a ticket page has
  a live shell selected. Everything else on the board is pushed; a pane is the one thing the
  daemon has no event for, so it is the one thing polled — narrowly, and never from the board.
  A refusal still stamps the attempt, or a dead pane becomes a fork every 100 ms.
- **The capture is keyed to its session.** The zone draws `shell_tail` only when it matches the
  selected row, so moving the cursor never shows another session's screen for a frame.
- **The client sanitizes**, same as transcript text: `peek::sanitize` (now `pub(crate)`) sweeps a
  pane's bytes before they reach a cell — a `tree` is a boxful of the banned 0x2500–0x259F range.
  Both L1 law tests now render a shell tail full of exactly that and assert it is on screen first.
  Lines are truncated, never wrapped: output is column-aligned and wrapping mangles it.
- **`mcp::agent_allows` denies it**, which is what the exhaustive match is for. `authorize` is also
  handed a real `Resource::Session { id }` on this path rather than the blanket `Resource::Board`,
  so D10's "no session read at any tier" is reachable rather than merely true.

E2e: `crates/mesimon/tests/pane_tail_e2e.rs` — real tmux, `SHELL=/bin/sh` pinned for a predictable
prompt, `send-keys` types a command, and a killed session must refuse rather than answer empty.

## A pane gets the user's own shell environment (2026-09-01)

Reported from dogfooding: an `export` added to `~/.zshrc` never reached an agent's MCP servers,
"no matter how many times I tried opening mesimon again or sleep / wake the session". Both of the
user's instincts were sound and both were defeated by the same design.

**Two separate bugs, and measuring them changed the fix.**

1. *The environment was frozen.* `TmuxBackend::tmux()` built the tmux client's env from D29's
   nine-name allowlist taken from the daemon's own environment; the tmux SERVER captures its
   global environment at first launch, `set -g update-environment ""` stops it refreshing, and
   nothing in mesimon ever calls `kill_server` (it existed only in tests). The author's live
   server was handing fresh panes a PATH from two days earlier — missing `~/.cargo/bin`, which is
   exactly the "may still need `source ~/.cargo/env`" note in CLAUDE.md, now explained.
2. *The environment was too narrow.* A **Claude pane is exec'd directly by tmux** — argv has more
   than one element, so no shell runs and no rc file is read, ever. A **shell pane** is `[$SHELL]`,
   a single element, which tmux execs into an interactive zsh that sources `~/.zshrc` normally
   (verified: the pane's PPID is tmux itself and `$-` contains `i`). So the two pane kinds
   disagreed about the same machine, and nine names was the agent's entire world.

Sleep/wake could not help either kind: both tear the pane down and respawn on the SAME server.

**The measurement that shaped the fix: tmux takes a pane's `PATH` from the spawning CLIENT and
ignores `new-session -e PATH=…`.** Measured against tmux 3.6a — a pane spawned with
`-e PATH=/EPATH/bin` from a client holding `PATH=/CLIENTPATH/bin` came up with the client's, while
`show-environment` on that same session reported `/EPATH/bin`; a bare command present only on the
`-e` PATH exited 127. tmux does this deliberately so a command given by name can be found. Every
other variable does travel through `-e`. Pinned by
`a_panes_path_is_the_clients_and_e_path_is_ignored`, because a tmux bump could change it silently.

That is why **no server restart is needed and none was added**. `TmuxBackend::set_path` puts the
captured PATH in the client env of every tmux invocation; `publish_path` also writes it into the
live server's global env, which is cosmetic for panes but stops `show-environment -g` telling the
next person debugging this a two-day-old story.

**The allowlist became a denylist**, and that is the load-bearing reversal. The set mesimon must
WITHHOLD is small, closed and knowable — tmux plumbing (`TMUX`, `TMUX_PANE`), a description of
someone else's terminal (`TERM*`, `LINES`, `COLUMNS`), a shell's process-local bookkeeping (`PWD`,
`OLDPWD`, `SHLVL`, `_`), `MESIMON_*` (minted per spawn; a capture taken inside a pane inherits the
previous ticket), and `PATH` (different road). The set a user may legitimately export is not
enumerable in advance — which is precisely why the allowlist failed. `core/src/shellenv.rs` holds
that filter and its tests; names are syntax-checked because bash exports functions as
`BASH_FUNC_foo%%` and tmux's `-e` parses `NAME=value` positionally.

**How the capture is run** (`daemon/src/shellenv.rs`), each choice measured or reasoned:

- `$SHELL -l -i -c 'env -0 > <dump>'`. Login AND interactive, because that is what a terminal
  starts on macOS; capturing with one would produce an environment no terminal here ever has.
- **To a file, not stdout.** An interactive rc PRINTS — prompt frameworks, version managers,
  greetings — so stdout is not a channel the answer can come back on.
- **`env -0`.** A value may contain newlines; a newline-separated dump cannot be parsed correctly.
- **A clean base env** (launchd's `PATH=/usr/bin:/bin:/usr/sbin:/sbin`), NOT the daemon's. A user's
  rc almost always prepends (`export PATH=…:$PATH`), so inheriting would quietly preserve the exact
  staleness this exists to fix. `LANG`/`LC_*` ARE carried in, because a locale comes from the
  terminal emulator rather than any rc file and a clean base would hand every pane a C locale.
- **Refused if it returns < 4 variables**, and the previous environment then stands: a broken rc
  must not be able to REPLACE a working pane environment with an empty one.
- 15 s timeout, off the writer thread, one at a time, dump deleted either way (it is a copy of the
  user's whole environment, secrets included).

**`tmux_bin()` now resolves a bare `tmux` to an absolute path.** Fallout of the above, caught by
the new backend test: once the client's PATH is the user's, a bare name is looked up there, and a
PATH change having nothing to do with tmux could lose the backend entirely.

**The offer is a suggestion, not an automatic reload.** The daemon stamps the newest mtime across
the usual rc files at capture time and compares on the tick; `Ctx::shell_env_stale` gates a new
`Verb::ReloadShellEnv` menu row, and `Suggestion` #2 (after "update ready") shows
`◦ shell env changed (esc)`. Automatic would mean forking the user's shell on every editor save.
`reloading` takes the offer down the moment the press lands, so a slow rc does not leave the chip
standing as though the press had missed. The row's detail is the honest half — **"new sessions and
wakes get it ∙ live panes keep theirs"** — because a running process's environment cannot be
changed, and a user who expects otherwise concludes the feature did nothing.

A capture that FAILS is offered on the same row, with different words — `◦ shell env unreadable
(esc)` / "your shell did not answer ∙ panes are on a fallback". Without that, one timed-out capture
left the user on the fallback environment with a notice explaining the problem and no way to ask
again, since the row is otherwise gated on an rc file moving. Same act, different news, and the
chip is the only place the difference gets said.

Known blind spot: the watch list is the common rc files, not an rc's `source`d dependencies.
`touch ~/.zshrc` is the manual trigger, and the row cannot be made always-available without
breaking the law that a suggestion's availability IS its menu row's.

E2e: `crates/mesimon/tests/shell_env_e2e.rs` — a fake `$SHELL` exports a variable and a hostile
`MESIMON_TICKET`, and the test reads the environment of the process tmux really exec'd: the
variable arrives, the captured PATH arrives, the real ticket survives, and the dump is gone.

## Leaving a Claude session parks it (2026-09-01, dogfood)

Ctrl+C-out, `/exit` and Ctrl+D end the PROCESS. They do not end the conversation — `claude
--resume` brings it back by exactly the road `wake_session` already drives. Recording that as
`Exited` was therefore filing a live thing under "dead", and it cost three separate things:

- **Two gestures for one state.** `x` slept a session and `x` woke it; a session the user had
  left instead needed `Enter` on the ticket rail, an affordance that says "focus", on a record
  the board drew with the same `✓` an idle turn wears. Nobody found it. The first report of this
  was a user asking what to do when Claude Code says *"update available, restart to apply"* —
  the honest answer was "sleep it and wake it", which is a strange thing to have to say about a
  session the user could just as well have exited.
- **`is_live()` gates real machinery.** An exited record leaves the working set, so the ticket's
  worktree lock was released under a session that was coming back.
- **No transcript snapshot.** Sleep takes the B-A22 copy; the exit road took none.

So `Daemon::park_on_exit` converts a clean exit to `Sleeping`, and the gate is deliberately the
SAME predicate `resume_session` will judge it by afterwards — park exactly when wake would
succeed, never a sleeper that can never wake:

- **`ExitReason::UserQuit` only.** That is where BOTH clean-exit roads land —
  `SessionEnd{prompt_input_exit}` and `pane-died` status 0 — so whichever wins the race parks,
  which matters because the race is real and undecided. `Crashed` keeps its error mark (a
  nonzero exit is worth seeing, and Enter still resumes it), `LoggedOut` would wake into an auth
  wall, and `Cleared`/`Resumed`/`Killed`/`Dismissed` are not deaths of this shape.
- **Non-empty argv.** An observe-only adopted record has no pane of ours and nothing to replay.
- **`!resume_transcript_missing()`.** A session Ctrl+C-ed before its first prompt wrote no
  conversation. Parking it would mint a record `x` refuses forever with "no transcript to
  resume"; that one really did just end.
- **Claude only.** A shell's pane IS its record. There is no conversation, and a woken one would
  be a *different* shell wearing the same row.

Re-minting the machine as `Sleeping` is what makes the aftermath harmless: the Sleeping latch
swallows the pane-died that follows the hook (or the SessionEnd that follows pane-died), so a
park cannot be flipped back into a corpse by its own echo. No SIGTERM — the process already
left — just the same reaper `sleep_one` hands the pane to.

The startup reconcile gets the same say, because this repo restarts a daemon after every
daemon-side rebuild and "I exited claude while the daemon was down" is the ordinary case, not
the exotic one. Scope there is narrow on purpose: only records THIS reconcile moved from live to
`Exited{UserQuit}`, never corpses already persisted as dead — otherwise every old corpse on the
board would quietly rejoin the working set on the next restart.

E2e: `crates/mesimon/tests/exit_parks_e2e.rs` drives the real `pane-died` road (a stub that
exits 0 under real tmux, no hook frame sent) and asserts all three gates plus the wake.

### Three more, found while reading, fixed in the same pass

**`/clear` recorded a live pane as dead.** `SessionEnd{clear}` mapped to `Exited{Cleared}`, which
is terminal, so the `SessionStart{clear}` that follows *in the same living pane* was swallowed by
the latch and the record read dead for as long as the session went on. This is precisely the shape
the 2026-08-30 dogfood fixed for `EndKind::Resume`, and `clear` is the harsher twin: a `/resume`
handoff at least ends up somewhere, while a `/clear` leaves a working agent filed as a corpse with
no event left that can correct it. `target` now returns `None` for both kinds, and the
terminal-latch refinement refuses to relabel a real death as either. `ExitReason::Cleared` and
`Resumed` are consequently unmintable — they survive only to read back a state file an older build
wrote. Test: `in_app_clear_is_not_an_exit`, which drives the whole round trip (end, start, settle)
and checks a late clear cannot relabel a crash.

**`x` on a corpse had no handler.** `rail_sessions`' doc claimed `x` marked a dead record
`Dismissed` and `Daemon::kill_session` implemented it, but nothing in the TUI ever sent
`Command::KillSession` — the press reached `sleep_verb`, which answered "only idle sessions
sleep": true, and useless. `sleep_verb` now routes on `Ctx::sel_dead` (the flag `Enter` next door
already reads to hint "resume"), and the `x` hint carries a third word, `dismiss`. The status line
does NOT say "still in the drawer", which was the first thing written and is only sometimes true —
the drawer is a transcript census, so a dismissed record only reappears there if it has a
transcript, which the commonest corpse reaching this key does not. It says the conversation is
untouched, which is unconditional. Tests: `x_says_what_it_will_do_to_the_row_under_it`, and
`ticket_corpse_selected_120x30` — the existing corpse golden had the cursor on a LIVE row,
so the footer it captured was never the corpse's own.

**"no transcript to resume" was a dead end.** A session killed before its first prompt wrote no
conversation, and `resume_session` refused it forever with that message — while the row went on
offering `enter resume` and `x` (before the fix above) offered nothing at all. The refusal was
written against a real failure — spawning `claude --resume <id>` on a conversation that does not
exist makes claude exit 1 inside a second, which the pane-died path then files as a crash — but
refusing is not the only way to avoid that. Where there is no conversation, "resume" and "start
fresh" have the SAME outcome, because there is nothing to lose. `resume_session` now spawns a
fresh conversation in the same record.

The new conversation gets a **newly minted uuid**, not the record's own. Claude has already been
handed `rec.id` once and whether it will take that id a second time is not something this code
knows; a fresh uuid cannot collide by construction. Nothing downstream notices, and that is D24
paying off: mesimon's identity is `rec.id` and rides the `--settings` and `--mcp-config` blobs, so
hook routing and the MCP principal are untouched by the conversation's id moving.
`claude_session_id` is the field that already exists for exactly this state, and the in-app
`/resume` relearn writes it the same way. The record's `transcript_path` is cleared with it —
leaving the stale one would send the NEXT resume back to the id that had no transcript, which is
the same dead end one wake later.

`Response::Spawned` grew `#[serde(default)] fresh: bool` so the TUI can say *"nothing to resume ∙
started a fresh conversation"* — a defaulted FIELD rather than a new variant, because a client
that cannot parse a `Response` line drops it and then waits forever for a reply that already came.
`m3_e2e`'s "ghost resume must refuse" assertion is now "ghost resume must start fresh", and it
asserts the argv carries `--session-id` and not `--resume` — the original concern, kept as the
thing actually checked.

## Reordering a card inside its own column (2026-09-01)

**The MOVE ghost could be placed anywhere in its home column and the drop did nothing.** T-84
folded three movers into one `Daemon::place_ticket`, and it opens with a no-op guard — `if from ==
dest { return Ok(from) }` — written for the automatic movers, where a move to the column the
ticket is already in is genuinely not an event and must not reach the feed, the ping-pong guard or
the flap fuse. But the human's drop arrives on the same command, and dropped in its own column it
carries the one thing the guard threw away: a new slot. The daemon answered `Response::Ok`, so the
TUI showed no error and simply refreshed the board back to the order it already had. The gesture
looked implemented and was not.

`Position` is what separates the two cases and it always did: every automatic mover says `Top`,
and `Before` exists precisely because it is "a human drag, which carries its own ordering". So the
guard now routes `Before` in a same-column move to `reorder_within` and keeps its early return for
everything else.

**A reorder is not a move, and it deliberately does less.** `reorder_within` authorizes the write,
recomputes the fractional index, saves the ticket file and broadcasts — no feed line, no
`moves.record`, no fuse tick. The fuse exists to stop an automation ping-ponging a card between
columns; charging it for a card sliding two rows by hand would let a few honest drags suspend
automation for that ticket. Nothing that watches for a move can see a reorder, which is correct:
no column changed.

It also short-circuits when the ghost is dropped where it was picked up (`want == at`, computed
against the column MINUS the moving card, which is the same index space the TUI's `drop_ghost`
builds `before` from). `move_back_home_restores_the_original_height` already called that a perfect
no-op; minting a fresh index for it would lengthen the fractional key on every cancel-by-drop and
broadcast a board that did not change.

**The seam that hid it:** the TUI's in-process fake daemon implements `MoveTicket` correctly,
including `before` inside the same column, so every client-side move test passed against a
reorder the real daemon refused to perform. Tests: `reorder_e2e` (real daemon, real wire — top,
bottom, and the disk round-trip) and `move_home_and_up_a_row_reorders_the_column`.

**Still true and not fixed here:** MOVE is entered only by `> <`, which shifts the ghost a column
immediately, so an in-column reorder reads `>` `h` `j/k` `enter` — out and back. The board binding
is also gated on `Ctx::multi_column`, so a one-column board has no reorder gesture at all. Both
follow from the author's rule that `> <` is "move card" between columns; a grab-in-place would be
a new binding, not a repair.

## The ticket page's preview zone, and a chord that drew nothing (2026-09-01)

Three things on the ticket screen, all found by using it.

**The DOCUMENTS zone was a placeholder holding the page's best real estate.** It owned the top of
the left column, printed two lines about ticket directories landing with the adoption IA and
reported `(0)` — while PREVIEW, which now carries an agent's markdown reply or a live shell's pane
tail, took whatever was left underneath it. The previous block already settled that ONE heading
covers both kinds of preview; this is the removal that decision implied and did not make.
`draw_documents` is `draw_preview`, the heading and its count are gone, and the zone starts at the
top of the column. Six goldens moved with it.

**Tags read before the branch on the ident row.** The worktree clause is built into its own span
vector and appended after the tag chips, so the row says what the ticket IS before it says where
its code lives. The width budget sums both vectors, so nothing about the truncation changed.

**`^t` was bound on the ticket screen and drew nothing.** `ui::draw` returns early for
`Screen::Ticket`, and the tag-picker panel is drawn past that return — so the footer dutifully
flipped to `Scope::TagChord`'s hints over a grid that was never rendered. The keys were live and
invisible, which is the exact failure the keymap's hint-and-avail pairing exists to prevent: here
the binding and its hint agreed with each other and both lied about the screen. The ticket arm now
draws the panel and re-draws the footer over it — the same two calls the board arm makes. Golden:
`ticket_tag_chord_120x30`.

**A digit repeat walked off the end of its row.** `arm.col += 1` with no wrap, so the fourth press
of `1` on a three-tag row moved the cursor past the last cell and the key went dead — which asks
for a second key to get back, precisely what "one finger reaches every tag on an axis" was meant to
avoid. It is modulo the row length now (`ui::tag_row_len`, floored at 1 so the empty spare row
cycles onto itself), and the jump also clears `forget_armed`, because a `d` armed on one cell must
not still be armed under the next one.

## `.` repeats the last action, starting with move (2026-09-01)

Triage is a run of the same gesture — *these four go to done* — and `> < ` spends every keystroke
on the aiming rather than on the act. `.` on the board does the last move again on the card under
the cursor: same target column, no ghost, no aiming.

**The cursor deliberately does not follow the card.** A drop lands the cursor on what it moved
(`drop_ghost`), which is right for one deliberate placement and fatal for a repeat: the second `.`
would be filing a card in the destination column. `repeat_last` restores the cursor and clamps, so
the next card slides up under it and `. . .` files three without a keystroke spent travelling back.

**It lands at the top of the target column**, which is exactly where a fresh grab's ghost enters a
foreign column (`ghost_entry_idx` → 0, `double_grab_lands_on_top_of_the_next_column`). `.` is
therefore `>`-aim-`enter` with the aim already made, and nothing else.

**The column is remembered by NAME and re-resolved every frame.** `LastAction::Move { column }` is
re-derived through `repeat_target()` the way `undo_target()` re-derives `u`: an index would follow
a renamed or removed column into meaning something else, and a card already sitting in the target
has nothing to repeat — moving it would be a shuffle, so `can_repeat` goes false there and the key
goes inert AND unhinted with it. Only a move the USER made here arms it (recorded in `drop_ghost`,
the TUI's single commit point); an automove or another client's move is not something this hand
did.

**The verb is `Repeat`, not `RepeatMove`.** `repeat_target()` matches `Option<&LastAction>` with no
`_` arm, so the second repeatable action is a variant plus a match arm the compiler asks for.
`Ctx::repeat_word` is the `undo_word` shape for the same reason and the same constraint: `Hint` is
`fn(&Ctx) -> &'static str`, so the word comes from a fixed set ("move again") and the column name
lives in the status line ("moved to done").

Tests: `repeat_is_unbound_until_there_is_something_to_repeat` (keymap),
`dot_repeats_the_last_move_and_leaves_the_cursor_home` and
`dot_is_inert_before_a_move_and_on_a_card_already_there` (app).

## An axis holds ten, and the picker row windows (2026-09-01)

`MAX_TAGS_PER_GROUP` was 5, on the reasoning recorded above: past five "a row stops fitting and an
axis stops being an axis". Dogfooding refuted the first half of that and left the second intact —
five is not a vocabulary, it is a sample of one, and a user who wants six components on group 3 is
not building a list. The cap is now **10**, the same ten the groups themselves run to, so both
numbers in the tag system are the same number.

What that costs is exactly what the old comment predicted, and it is a rendering problem rather
than a modelling one:

- **A full row does not fit.** A cell is `[` + a two-cell swatch + the mark + the name + `]` + a
  space, so ten five-letter tags want 110 columns plus the five-column digit gutter — fine at 120,
  over at 80. Nothing before this needed to care: at five tags the row always fit, so the panel's
  silent clip had never been reachable.
- **So the row is WINDOWED, not clipped** (`ui/tagpicker.rs::window`). The cells of a row are
  built first and placed second; the window is the run of them around the cursor that fits, and a
  `~` — `text::truncate`'s marker, spent here for the same reason — sits on whichever side still
  holds cells. The cursor's own cell is never the one dropped.
- **It grows left first, then right.** That is a one-line field's scroll: the row stays anchored
  at its start until the cursor walks past the edge, and then follows it a cell at a time. A row
  the cursor is not on is anchored at its start instead — a digit jump lands at the head.
- **The window is a function of the cursor and nothing else.** No remembered scroll offset, so
  there is no second piece of state to fall out of step with a board that just lost a tag.

The naming cursor moved with it: a cell now carries the hardware cursor's offset *within itself*
and the assembly adds the window's origin, because a cell no longer knows its own column until the
window has been chosen.

`tags_e2e` counted the cap out by hand (`for i in 0..4` after one tag) and broke on the bump; it
reads `MAX_TAGS_PER_GROUP` now. The card is untouched — it still names at most two tags and the
peek row still drops the tail rather than shrinking every name.

## Alt is admitted, for one verb (2026-09-01, dogfood)

04 §2.0 rule 2 banned Alt/Meta outright, and `no_banned_atoms` enforced it the strongest way
available: there was no `Key::Alt` to construct. The ban was right about terminals and wrong
about the conclusion. Option+direction is what a person's hands already do to move an item —
every editor and every list in the OS binds it — and the board's only mover was `> <`, a grab
you aim and an Enter you commit, three keys deep for "this one goes right".

So `alt+hjkl` / `alt+<arrow>` now moves the card one step and takes the cursor with it
(`Verb::Nudge`), and the ban became a whitelist:

- **Four atoms, one verb, one scope**, held by `alt_is_admitted_only_for_the_nudge`. `alt+h` and
  `alt+←` are the SAME atom, the way the two spellings of `ctrl+]` are — the keymap wants a
  direction and the terminal may spell it either way.
- **The escape clause is NOT `rich_keys`, and the difference is the whole argument.**
  `ShiftEnter` had to be gated because a terminal that cannot report it reports something else —
  a plain `Enter`, which is another verb. Alt fails the other way: the modifier is eaten and
  NOTHING arrives, so the key is inert, never wrong. Inert is affordable exactly while the atom
  is an accelerator, which is why the whitelist pins it to one verb and holds `> <` beside it as
  the spelling every terminal can reach.
- **Measured on the author's own terminal, not assumed.** iTerm2 3.6.11 with `Option Key Sends:
  Esc+` delivers `⌥h` and `⌥↑`/`⌥↓` as the ALT modifier; the RIGHT option key is set to `Normal`
  and composes instead, and both profiles map `⌥←`/`⌥→` to send `esc b`/`esc f` (a word jump).
  That last one is why the map is a whitelist and not "alt+anything": `alt+b` has no atom, so it
  resolves to `None` rather than to `b`'s verb. On that machine the gesture is left-option plus
  `hjkl` or `↑↓` until those two profile mappings are deleted.
- **A text field never sees an Alt atom** (`keys::to_key_text`). Alt there is `word_wise`'s "by
  word" modifier, and a composer whose `alt+←` became `Key::AltLeft` would stop jumping by word.

The move itself is `drop_ghost`, so it inherits the laws the grab already had: sideways enters a
foreign column at the top, exactly where a ghost enters one. Two things it deliberately does not
inherit — an edge press **stays put rather than wrapping** (`>` wraps, because a ghost can still
be cancelled and a card that has already moved cannot), and a same-column reorder no longer arms
`.`: `repeat_target` refuses to repeat a move onto a card already in that column, so arming would
have left the repeat aimed at whatever column the cursor was standing in.

`prio: 0` — the `?` overlay carries it, the footer does not. The footer already teaches `> <` two
entries up, and its cells are better spent on verbs with no other spelling. The board overlay is
now one row longer, which on a 30-row terminal costs the dangling `APP` heading whose rows were
already being clipped (`help_board_120x30`); the overlay has never scrolled.

## The launch window is visible (2026-09-01, dogfood)

`SessionState::Spawning` carried no card glyph, on purpose: 07 §4.1 says a normal card starts its
title at T[0], and until Shift+Enter every spawn handed the focus straight to the pane, so nobody
was looking at the card during those seconds anyway. **The composer's Shift+Enter stays on the
board by design — the card IS how the user watches the work land** — and there it showed a title
and no sign of life until the first `SessionStart` hook flipped the record to `Running`. Nothing
distinguished "claude is booting" from "nothing happened".

**`glyphs::launching` is the working arc at the slow cadence** — `spinner(tier, frame /
SLOW_STEP_TICKS)`, 400 ms a frame against the spinner's 100. Spawning is not a different thing
from working, it is working that has not started, so a different SHAPE would have overstated the
difference; the slowness is the whole message. It rides `Register::Grey` and sits between
`running` and `sleeping` in `card_glyph`'s precedence: anything abnormal still outranks it, and it
still outranks `z`, which means nothing is happening. `session_glyph` gives it to `Spawning` too,
so the ticket rail and the accordion dots move with the card (the rail row already reads
`spawning`; now it does not sit still while it says so).

- **It is deliberately NOT disjoint from the spinner, where `waiting` must be and is.** A still
  frame of `waiting` must never read as progress, because `Unknown` means the daemon has lost
  track. Launching resolves into working within seconds and both frames mean the same thing to
  the reader — the agent is going, leave it alone. `spawning_launches_slowly` asserts every
  launching frame IS a spinner frame and is never a waiting one.
- **`WAIT_STEP_TICKS` became `SLOW_STEP_TICKS`, shared.** D19's motion ban bends once for the
  spinner; the board now has one fast cadence and one slow one, and a third speed would have been
  a third thing moving. (The peek's `pulse` is the standing exception — it changes weight, not
  shape.)
- **The worktree provisioning window is covered by the binding, not by a record.** A worktree
  ticket's first spawn is PARKED (`Response::Provisioning`) while the worktree is cut — ~2 s of
  git, and the longest wait on the board — and there is no `SessionRecord` yet to hang a mark on.
  `card.rs` falls back to the same arc when the ticket's `WorktreeItem.status` is `queued` or
  `provisioning`: provisioning is lazy, `queue_provision` is reachable only from
  `resolve_spawn_cwd`, so **a queued binding IS a parked spawn** and no new wire field was needed.
  `a_provisioning_ticket_launches_too` pins it, including that `attached` drops the mark.
- **Answered on the frame after the press, both ways.** `start_composed` already calls `refresh()`
  after the spawn request, so the `Spawning` record (shared checkout) or the queued binding
  (worktree) is on screen at the next draw.

**Correction, same day: the window is not `Spawning`, and the first cut flickered.** `Spawning` is
only its first half. Shift+Enter's Enter is DEFERRED to the `SessionStart` frame (paste detection
swallows one sent with the text) — and that same frame is what moves the record off `Spawning`, to
`Idle { stop_reason: Unknown }` (`attention.rs`, `Signal::SessionStart{..} => t(S::Idle{Unknown})`).
The turn only begins at the `UserPromptSubmit` ack, measured at ~94 ms in T-5 but a retry cadence
of 500 ms behind it. So the mark lit, went dark for half a second, and came back as the working
spinner — the card said "nothing is happening" in the middle of its own launch (dogfood
2026-09-01, "for half a second it removed the animated glyph").

`glyphs::is_launching` is now the predicate, and `SessionRecord::pending_submit` is what carries it
across the seam: an `Idle` agent normally rightly has no glyph, because idle means it is waiting
for YOU — the owed Enter is what says this wait is OURS.

- **It mirrors the daemon's `pressable`** (`retry_pending_submits`), minus that predicate's
  `Running` arm, which the spinner outranks here: once work is in flight the fast arc is truthful.
  Showing the launch mark exactly while the daemon still expects the prompt to land is the same
  discipline park-on-exit used (park exactly when wake would succeed).
- **`Idle{EndTurn}` is excluded.** A finished turn is only reachable through the ack that clears
  the flag, so it never rides this path live; a record reloaded holding a stale flag keeps `done`,
  which is both truer and what `card_glyph`'s precedence already preferred. `session_glyph` needed
  saying explicitly — its early return fires before the `EndTurn` arm.
- **A stale flag cannot strand the mark.** The daemon clears `pending_submit` on the ack, on
  giving up after `SUBMIT_ATTEMPTS`, and on any state where the pane stopped being pressable; no
  state outside `Spawning` and non-`EndTurn` `Idle` consults it. `a_stale_owed_enter_never_relabels_a_state`
  pins that for `Unknown`/`Sleeping`/`Exited`/`Failed`/`Idle{EndTurn}`.

No golden moved: no golden board has a spawning session or an unattached worktree.

## The bulk sleep gets a key, and it is `Z` (2026-09-01, author direction)

Amends "Suggestions are one right-hand chip and a marked menu", not its shape: the offer is still
one chip pointing at one menu row. What changed is that the chip's route now has a key on it, so
`◦ sleep 3 agents (Z ∙ esc)` reads exactly like `◦ update ready (U ∙ esc)` and takes one press
instead of esc + enter.

Board-wide actions have no key as a rule — they are not about the selection, and a menu row has
room to say what it will do. `U` was the first exception; this is the second, on the same terms.

- **`Z` is zzz, not shift-of-`x`.** The retired `X` *was* shift-of-`x`, and it stays retired:
  shift hardens or forces the same verb on the same target, and `x` sleeps the selection's
  sessions while this sleeps a column's. Nothing binds `z` on the board (it is the ticket
  screen's view prefix and the diff view's density), so the atom is free and unambiguous.
- **Bound on `Scope::Board` alone**, because the header — and therefore the chip that teaches it
  — is drawn only there; the ticket and diff screens compose their own chrome. A key that is
  named works, a key that is not named is inert, and this one is named in exactly one place.
- **`prio: 0`, overlay-only.** The footer keeps saying what the card under the cursor can do; the
  offer lives in the header and the `?` overlay, which is the same treatment `U` gets and for the
  same reason.
- **`avail` is the menu row's own predicate** (`bulk_sleep > 0`), so key, chip and row appear and
  disappear together — the invariant `every_suggestion_is_a_menu_row` already guards, extended by
  hand to the key in `shift_stays_on_one_axis`.
- The menu row now names `Z` at its right edge; `menu_holds_the_board_wide_actions` already
  asserts every key a row names is a key the keymap really has, so the row and the binding cannot
  drift apart.
- Golden: `menu_suggestions_120x30` (the Sleep row gained its key). `help_board_120x30` did not
  move — its fixture has nothing to sleep, which is the gate working.

## Quick tag is a bare digit, not shift+digit (2026-09-01, author direction)

Asked for as "quick tag by pressing shift+digit (cycle)". The capability shipped as asked; the
atom did not, because **there is no shift+digit atom to bind**. `keys::to_key` maps
`KeyCode::Char(c)` to `Key::Char(c)`, so Shift+1 arrives as `!` — the binding would really have
been the ten symbols `!@#$%^&*()`. That fails three ways at once:

- **Layout.** The shifted digit row is US-only. Hebrew swaps `(`/`)`, so groups 9 and 10 invert —
  the same class of defect as ctrl+] landing on Esc there. On AZERTY/German the row is shifted
  *for digits*, so Shift+1 sends `1` and the gesture misfires entirely. `Key::Char('1')` means
  "whichever key types a 1 on this layout", which is correct everywhere by construction; the
  shifted spellings name a different physical key per layout with no relation to "group 1".
- **`!` was taken** — the diff screen's `WorktreeShell` (M4b).
- **The Shift axis.** `shift_stays_on_one_axis`: shift hardens or forces the same verb on the
  same target, never introduces one. There is no unshifted digit verb on the board for it to
  harden.

What shipped instead is the picker's own digit, reached without the picker: `1`-`0` on the board
and the ticket screen step the selected ticket along that group's vocabulary.

- **The same key already means this.** Inside `^t` a digit picks an axis and advances along it;
  outside, it does the axis pick's useful half. `^t 3 3` and `3 3` land in the same place, which
  is the accelerator relationship shift was reaching for, bought with consistency instead of a
  modifier. `DIGITS` is one shared list, so the two cannot drift.
- **`prio: 0`, overlay-only**, on the Nudge precedent: it accelerates `^t`, which the footer
  teaches one entry up, and the footer's cells go to verbs with no second spelling.
- **The ladder is `none → first → … → last → none`, not a wrap** (`board::cycle_tag`, pure and
  tested). Off the end is untagged on purpose: a wrapping cycle can put a tag on a card but never
  take the last one off, which would leave `^t` the only way to undo a keystroke. A `current` the
  registry no longer holds restarts the cycle rather than sticking on a value no press can leave.
- **It reads the registry, never the tickets** — `Board::group_tags`, whose doc already recorded
  why: a ticket-derived vocabulary loses a tag when its last wearer drops it, and the ticket being
  cycled reorders its own list mid-cycle. That function existed for this feature and had no
  caller until now.
- **`Ctx::tags_exist` is board-wide, not per-group**, because ten digits share one `Binding` and
  `avail` never sees which key arrived. An empty registry is the one state where every digit is
  inert and unhinted; a digit whose own group is empty says so in the status line and points at
  `^t`, which is still where names are made.
- Bound on `Scope::Board` and `Scope::Ticket` separately. `Scope::Diff` chains to Global, not to
  Ticket, so the diff screen keeps its digits — and `Scope::Input` owns its own, so the composer
  types them.
- Tests: `keymap::digits_cycle_tags_without_the_picker` (both screens, the picker's digit intact,
  inert with no registry/no selection, absent from Diff and Input, and the ten shifted symbols
  reaching none of it), `board::the_cycle_runs_off_the_end_into_untagged`.

## Ten tints, and chalk gets its own lightness (2026-09-01)

Two complaints, one paragraph of palette: *"tag colors barely visible on light theme and not
enough colors"*.

- **Six tints was not enough and the number was arbitrary.** `theme::PIPS` and
  `board::TAG_TINTS` are now **10 == `MAX_TAGS_PER_GROUP`**, so a single axis can be entirely
  colour-distinct — the only count at which the tint means anything *within* a group. Below that,
  a board with two axes was collapsing four names onto one tint. The two constants finally have
  the test their doc comments have been promising: `theme::tests::tag_tints_agree`, on the
  `mesimon-tui` side because it is the one that can see both.
- **The hues are now even around the wheel**, minus the 50–100° band where `attn` lives. The
  six-hue set could be picked by hand; ten cannot, and once the chroma ceiling is fixed, even
  spacing is exactly what maximises the worst pair (ΔE76 15.6 graphite / 13.2 chalk). **One ring,
  not two rings of five**: a second lightness separates same-hue pairs by ΔL* alone (~ΔE 12), so
  it loses to a single ring of ten *and* makes some tags louder than others, which a tag axis may
  never do. The law test now asserts every PAIR ≥ ΔE76 12 — that is the number that says whether
  ten still go, and it is what will refuse an eleventh.
- **Chalk's tints are L\* 38, not 45, and that is the light-mode fix.** Chalk's ground is paper, so
  a tint is INK on it and has to sit as far below the paper as graphite's sits above its ground.
  At L\* 45 it cleared the text floor (5.04 on bg) and still read as a smudge, because a tint is
  faded toward the ground on almost every card and toward WHITE that costs far more than toward
  black. L\* 38 measures 6.52 on bg and 5.40 on the selected surface. Chroma stays at 26 rather
  than graphite's 30 for the reason it always did — chalk's `attn` is C\* 53.8, which puts the 2x
  ceiling at 26.9.
- **`Theme::faded` is per-flavor now: 0.70/0.38 graphite, 0.76/0.46 chalk.** The blend is a ratio
  in sRGB bytes, and one pair of numbers does not mean the same thing on both grounds. Under the
  shared pair, chalk's `Rest` landed at C\* 16.8 / contrast 2.82 where graphite's landed at
  21.7 / 3.61, and `Sleeping` sat on the C\* 8 floor. It now measures C\* 17.9 / 3.77 and
  C\* 9.8 / 2.08 — at or above the graphite level it mirrors — with the boundaries still visible
  (ΔE 14 and 20 between levels, against graphite's 17 and 19). The 12%-of-span step assertion is
  untouched and both flavors still clear it. This also fades the untagged card's neutral block, so
  the *card's* three loudnesses got the same repair on light.
- **Cost, paid once and on purpose:** `default_tint` is a hash modulo `TAG_TINTS`, so every tag
  that never had a colour picked lands on a new hue, and a stored index points at a different
  place on a re-cut ramp. Alpha, one board, and Tab re-picks.
- Goldens are colourless text, so none moved. `test_pip_ramp_is_low_chroma_and_legible` grew the
  pair-distance assertion and a `delta_e` helper (CIE76 — coarse in general, honest here, where
  one lightness and one chroma leave the hue angle as the only difference).

## The board's footer is the card's, and the ticket page took the rest (2026-09-01, author direction)

Ten board bindings dropped to `prio: 0` in one pass — `c` claude, `s` shell, `x` sleep sessions,
`a` archive, `d` delete, `esc` menu, `p` show replies, `.` repeat, `^t` tags, `q` quit — and the
ticket page's `d` came up to `prio: 90` to meet them. What is left is five entries:

    BOARD  enter ticket page ∙ o open ticket ∙ option+hjkl move card ∙ r rename ∙ ? keys Nothing was unbound: every one of those keys still resolves from the board and still
appears in `?`, which is the whole point of the two-tier footer. What changed is what the board
spends its one row of cells on.

- **The line was full, and it was full of the ticket page's subject.** A 120-cell footer held
  eight entries; three of them (`c`, `s`, `x`) were the session group, which the ticket screen
  already hints in the same words beside the sessions they act on. The board was teaching a verb
  whose result it cannot show.
- **Destroying a card is done from the page that shows the card.** `a` and `d` went the same way
  and for the sharper version of the reason: the ticket screen names the sessions and the
  worktree that a delete takes with it. `d` is now hinted there and nowhere else — the board's
  copy is the accelerator, not the teacher.
- **`esc menu` left too, and it is the one that cost something.** The Esc menu is still the only
  route to the board-wide actions, so the invariant `empty_board_hints_nothing_that_needs_a_ticket`
  used to assert ("the menu is always reachable") now reads against `resolve` and `overlay`
  instead of the footer: the key works, `?` names it, and the header's suggestion chip spells
  `(esc)` / `(U ∙ esc)` / `(Z ∙ esc)` on exactly the occasions the menu has something waiting.
  With no suggestion pending, nothing on the board says `esc` — that is the deliberate cost.
- **`p` went last, and it is a different argument.** The peek is a view preference, not a verb:
  set once, lived with, and the board it changes is the evidence it worked. A permanent cell
  teaching a toggle nobody presses twice was the cell the footer could least afford.
- **What the freed cells bought**, on the calm board: `q quit`, which never fit before, and room.
  The board footer now reads as one thought — go, open, move a card, rename, tag, leave.
- **One binding, two prios, is the mechanism** — the same `Verb` appears in `BOARD` and in
  `TICKET` with its own `prio`, so "hint it there, not here" needs no new concept and no
  `Ctx` flag. `hint_for` and `overlay` ignore `prio` by construction, so nothing else moved.
- Cost recorded: adding `d delete` to the ticket footer pushes `q board` (prio 250, the tail)
  off the line at 120 columns. `q` still pops, `esc` still backs out, and `?` still lists it.

Three that went last, each on its own argument:

- **`.` repeat** — the one binding whose availability was already its own advertisement.
  `can_repeat` is false until you have moved something, so the footer entry could only ever
  appear *after* the gesture it accelerates, to the hand that had just performed it.
- **`^t` tags, on the board AND the ticket screen, but NOT in the composer** — tagging is done
  once, while the ticket is being made and the words are already in your head; after that it is
  maintenance, and maintenance can be looked up. The composer's copy went the other way, `prio:
  35` → `25`: at 35 it sat behind `shift+tab`'s 39-cell hint and 120 cells ran out inside it, so
  the one place the footer was supposed to name `^t` had never once shown it. `shift+tab` drops
  in its place and loses nothing — the composer draws `⎇ worktree  shift+tab` on the card being
  composed, which is where that choice is made and where it says it locks.
- **`q` quit** — `q` and `^c` are the two spellings of leaving that every terminal program has
  taught for decades. The footer is for what this program does that another one would not.
  (Falling off the board freed the slot that put `q board` back on the *ticket* footer, which
  `d delete` had truncated away earlier in the same pass.)

Tests: the golden set (every board and ticket golden moved by exactly one line, the footer),
`empty_board_hints_nothing_that_needs_a_ticket` rewritten to assert `esc` and `q` resolve and are
named by the overlay rather than by the footer.

## A tag moves, and Alt's clause was the count, not the point (2026-09-01, user request)

`^t` could make a tag, wear it, recolour it, rename it and delete it board-wide. It could not
move one. Along the row that cost only convenience — registry order is what the picker draws and
what a repeated digit walks, so the first name on an axis was whichever you happened to create
first. Across rows it was a dead end: a tag created on the wrong axis had exactly one remedy,
`d`, which strips it from every ticket wearing it on the way out. So `HJKL` / `alt+hjkl` now
carries the tag under the cursor, one cell along its axis or one axis over, cursor riding with
it — the board's nudge, in the picker's grid.

- **`Command::MoveTag { group, name, to_group, to_index }` → `Board::move_tag`.** One command for
  both axes, because they are the same gesture; `to_index` is a slot in the DESTINATION row and
  is clamped there, so a client whose row moved under it lands the tag at the end rather than
  being refused. A cross-axis move carries the wearers, `rename_tag`-style, and returns their ids
  so only those files are rewritten.
- **A cross-axis move is REFUSED, never resolved, when a wearer already has a tag there.** One
  tag per group is what lets a digit address an axis; the alternative is dropping somebody else's
  tag off a card nobody is looking at, and that is not something one keypress may do quietly. The
  message names the count and the axis. The other two refusals are `register_tag`'s, unchanged: a
  full axis, and a name that axis already holds. Every refusal leaves the registry byte-identical
  — a half-applied move is worse than none.
- **A tag arriving on an axis JOINS it, at the end.** Landing at the cursor's old column would
  reorder a row the eye already knows, to make room for a tag nobody aimed at that slot.
- **The composer mirrors the wearer check client-side.** Its picks live on
  `InputPurpose::Create` and are on no ticket, so the daemon cannot see them; without the mirror
  a composing user could move a tag onto the axis their own half-made ticket was already using.

**Alt's clause was rewritten, and the rewrite is the load-bearing part.**
`alt_is_admitted_only_for_the_nudge` asserted one verb on one screen. That was the count, not the
reason. The reason is that an eaten modifier delivers NOTHING, so an Alt key is inert rather than
wrong — affordable exactly while no capability stands behind it. `alt_is_admitted_only_for_a_nudge`
now says that instead: every Alt binding is a nudge, and every one carries a legacy-floor spelling
of the same move on the same screen, hinted.

- **In the picker the two spellings share ONE binding** — `H J K L` and the four Alt atoms in one
  key list. That is the strongest form of the clause available: the accelerator cannot reach a
  move the floor does not make, because it is the same entry. It also keeps Shift on its axis
  (`hjkl` steps, `HJKL` steps carrying), the same bargain `c`/`C` and `s`/`S` make.
- **It cost a footer cell, and the cell was `w`'s.** At 120 columns the picker's footer already
  ran the full row, so a ninth entry does not join it, it evicts the last one. That was
  `w 2nd tag beside` — and the second-tag experiment was ended in the same session (below), which
  is what paid for the slot. `prio: 15`, beside the motion it hardens.
- **`key_tag` reads `to_key` while steering and `to_key_text` only while naming.** The picker is
  a text field exactly while `tag_naming`, and CLAUDE.md's rule is that a text field never sees
  an Alt atom. Reading `to_key_text` unconditionally, as it used to, meant `alt+h` arrived as a
  bare `h` and moved the CURSOR — the accelerator was silently the wrong verb, which is the one
  thing the Alt clause exists to prevent.
- **An Alt atom never dismisses the picker.** A stray key closes the panel rather than acting on
  the board behind it, and that swallowed the nudge twice over: `alt+←` on a cell with nothing to
  carry closed it, and on a terminal that eats the modifier the composed `˙` closed it too. Both
  are now inert — the accelerator must not cost a user their place on one machine and nothing on
  the next.

Tests: `board::move_tag`'s three (`moving_a_tag_along_its_axis_reorders_the_cycle`,
`moving_a_tag_to_another_axis_carries_its_wearers`,
`a_tag_never_moves_onto_an_axis_a_wearer_already_uses`), `alt_is_admitted_only_for_a_nudge`
rewritten, `shift_stays_on_one_axis` grew the picker's pair, three in `app::tests`
(`a_tag_is_carried_along_its_axis_and_onto_another`, `an_eaten_option_key_leaves_the_picker_standing`,
`a_shifted_letter_is_text_inside_a_name_field`), and a block in `tags_e2e` through the real
daemon and back off the disk. No golden moved: the binding is overlay-only.

## The board's move hint is `option+hjkl` now (2026-09-01, author direction)

`Verb::Grab` (`> <`) and `Verb::Nudge` (the four Alt directions) swapped footer billing on the
board: Grab went to `prio: 0`, Nudge took its slot at `prio: 60`, its `show` became
`option+hjkl` and its hint lost the word "now".

- **`option`, not `alt`.** The `show` string is what a hand goes looking for on a keyboard, and
  the key on the machine this ships to says `option`. The atom is unchanged — `Key::AltLeft` and
  friends, one atom per direction with `alt+←` and `alt+h` the same key, exactly as before.
- **"move card now" became "move card".** The "now" was drawing a contrast with the aiming
  gesture two entries up; with `> <` off the line there is nothing to contrast with, and the
  thing the key does is move the card.
- **`can_nudge` is the wider predicate**, so the footer now offers the move on a one-column board,
  where `> <` (which needs `multi_column`) had nothing to say and went dark.
- **The Alt admission clause is spent down to its bound-beside-it half, and this is the cost.**
  `alt_is_admitted_only_for_a_nudge` admitted the atom because no capability stands behind it AND
  a legacy-floor spelling of the same move sat beside it, on the same screen, *hinted*. `> <` is
  still bound and `?` still names it — the test now asserts exactly that, plus that the footer
  names the accelerator — but a terminal that eats the modifier reads a footer whose move key
  does nothing, and finds the working one only in the overlay. Recorded in the test's own doc
  comment, because that test is what would otherwise have stopped guarding it silently.
- `.` (repeat) sits beside the move for the same reason it always did; its comment no longer
  claims the neighbour is `> <`.
- **`> <` is hinted "move card, aiming" now.** Dropping "now" from the accelerator left two
  overlay rows reading "move card", which tells a reader which keys exist and nothing about
  which to press. The floor spelling is named for what it adds: a ghost you aim and can cancel.
- **An eleven-wide spelling found a latent bug in `?`.** `ui/help.rs` padded to a hardcoded
  `KEY_W = 10` and rendered `option+hjklmove card` — no gap — and would have done the same to
  `shift+enter` (12) in the composer's overlay, which no golden covers. `KEY_W` is a floor now
  and the rail is `max(KEY_W, widest + 1)`, so a spelling that fills the column still cannot
  touch its hint. Every existing overlay was ≤ 9 wide, so nothing had tested the arithmetic.
- Width cost: `option+hjkl` is eight cells wider than `> <`. At 100 columns `r rename` falls off
  the board footer, and at 120 with `tab needs you` up, `q quit` does.

## The second tag's trial ended, and an open card stopped needing the glyph (2026-09-01, author direction)

`Second` had three homes and `w` cycled them on a live board: `Stack` (`▀` across the bar cell),
`Half` (`▌` down it) and `Edge` (the card's right-edge pad). The trial is over. **Stack wins and
the other two are deleted** — the enum, `MESIMON_TAG_SECOND`, `edge_cell`, `CardCtx::second`,
`App::tag_second`, `Ctx::tag_second_next`, `Verb::TagWeight` and the `w` binding all went with
them. What is left is one behaviour with no switch in front of it.

- **`▌` U+258C went back to being banned.** It was admitted alongside `▀` on an explicit
  exception to the L1 no-drawn-structure law; nothing spends it now, and an exception nothing
  spends is a ban. `test_no_drawn_structure` names ONE codepoint again.
- **The freed footer cell went to `HJKL move tag`** (above), which is why that binding could come
  off `prio: 0`. The picker's row is the same length it was.

**And where the stripe is tall, the half-block is not reached for at all.** `▀` puts two tags in
one cell because a RESTING card has exactly one cell to spend. An open card's stripe is three to
six, and there `tags::stack_full` repaints it as two runs of full painted blocks: the first tag
the top ~70%, the second the ~30% under it, same order.

- **`second_rows(rows)` is the whole arithmetic** — `((rows * 3 + 5) / 10).clamp(1, rows / 2)`,
  and `None` below three rows. The clamp is not defensive: the floor of one keeps the second tag
  from vanishing on a 4-row card, and the ceiling of half keeps the FIRST tag the run the eye
  lands on, which is the entire reason the two are ordered.
- **Three rows, not two, is the threshold.** A 2-row stripe split 1/1 is halves wearing a 70/30
  name. Under the threshold the card keeps `▀`, which is the honest mark for one cell.
- **It repaints span 0 and nothing else.** The bar is span 0 on every card row by construction, so
  the split is a post-pass over the built lines rather than a second layout path — no text moves,
  no width changes, and `test_the_second_tag_costs_no_width` renders one tag against two, peek off
  and on, to hold that.
- **Height is the gate, not the peek toggle.** The direction named the peek because that is when
  the card is tall, but `p` is about replies and has nothing to say about tags; keying the stripe
  to it would make the mark change for a reason that is not about the mark. `second_rows`
  refusing under three rows is the same floor, stated where it belongs. A selected card with two
  session rows therefore splits too.
- Below TrueColor there is no tint to run and nothing changes: the underline still says "tagged"
  without saying which.

Tests: `an_open_card_runs_the_two_tags_as_full_blocks`,
`a_stripe_with_one_tag_or_no_paint_is_left_alone`, `the_second_tag_is_the_lower_half_of_the_cell`,
`the_mark_reaches_for_one_codepoint_only` (now also asserting the split draws no glyph),
`test_an_open_card_runs_the_tags_down_its_stripe` (a real render: walks the stripe down the cell
buffer and checks one changeover, not stripes), `test_two_tags_ride_one_cell` rewritten for the
one home, and `test_the_second_tag_costs_no_width` rewritten to compare one tag against two.
Goldens: every tagged card whose accordion is open lost its `▀` column; the resting two-tag cards
kept theirs.

## A turn parked on background work is its own state (2026-09-01, dogfood)

T-128 sat in IN PROGRESS after its agent had visibly finished and said so, and the card carried
**no glyph at all** for two minutes. Neither symptom was automove's: automove was never consulted.

What happened, off `activity.jsonl` and the transcript. The agent backgrounded a Bash poll loop
(`for i in $(seq 1 14); do cargo test --no-run && break; sleep 25; done` — waiting on a
neighbouring session's build), then wrapped up its turn. The `Stop` hook fired with that task
live in `background_tasks[]`, so `task_blocks_end_turn` classed it blocking and the machine
re-asserted `Running` — the turn is paused, not done, which is right. But **the pane stops
painting the instant the agent parks**, so 8 s later `probe_activity` fired `Signal::PaneQuiet`
and demoted to `Idle{Interrupted}` at Medium. Nothing had interrupted it.

Two inferences, both wrong, stacked:

- `Running` claimed the pane was painting. It was not, and the quiet probe exists precisely to
  catch that claim — so the first lie summoned its own refutation.
- `Idle{Interrupted}` claimed a person had pressed Esc. `automove` refuses to promote an
  interrupt (correctly), and `card_glyph` had no arm for it at all, so the card fell through
  every branch to `None` — indistinguishable from a ticket nobody had ever opened.

**`StopReason::Background` is the fix, and it is a stated fact rather than a third inference** —
the blocking task came straight off the Stop payload. `Stop { blocking_tasks: true }` now lands
at `Idle { stop_reason: Background }`:

- **The misread stops being possible rather than being cleaned up.** `probe_activity` only scans
  `state == Running`, so a parked session is invisible to it. Compare `SubagentStop`, which
  carries a corrective for the same misread (`subagent_stop_corrects_pane_quiet_interrupt_misread`)
  — that corrective exists because there was no better answer for subagents. A background task has
  no such signal, which is why this one stuck for the whole life of the task.
- **The ticket stays in IN PROGRESS, and now for the true reason.** Only `EndTurn` promotes, so
  the refusal is automove's existing rule, not a new special case (`a_parked_turn_is_not_done`).
- **Rank is untouched.** `SessionState::Idle { .. } => 13` already covers it, so D28's frozen
  table does not move and no attention item is minted — a parked turn asks nothing of the user.
- **The wake path already worked and is unchanged**: the task's completion arrives as a
  `UserPromptSubmit` (observed on the wire at 20:57:00) and the turn resumes. `SubagentStop`
  cannot clobber it either — the park is High confidence, and that arm only promotes below High.

**The glyph rides the slow cadence** (author direction: it should move — a build genuinely IS
running, just not in this pane; the same argument that earned `launching` its slow arc). No third
speed appears: the board still has one fast register and one slow.

- **Unicode is a two-dot bar turning through the CENTRE** — `⠒ ⠌ ⠡`, i.e. `—` `/` `\` — against
  `waiting`'s two-dot pair hugging the rim and the spinner's three-dot arc.
- **Two dots is forced, not chosen.** A single dot was measured invisible at dim2
  (2026-08-31, "no glyph at all"), which is why `waiting` carries two; three would claim the
  spinner's "work in flight *here*". So the shape had to differ at the same ink, and the
  centre/rim split is what carries it.
- **Ascii cannot borrow the idea**: `| / - \` ARE the spinner's frames, so a turning bar spelled
  in them *is* the spinner. `~` is throttled, `. : , "` are idle and waiting. The ascii tier
  breathes instead — `o O` on the same clock.
- Under the spinner in `card_glyph`: if any session on the ticket is really working, that is the
  louder and truer thing to say. Rail word is `background`, lowercase — nothing is required.

One-way door, small: `StopReason` has no `#[serde(other)]`, so a `"background"` in `sessions.json`
quarantines on a build older than this one (board still comes up, notice in the advisory row).

Tests: `stop_with_blocking_tasks_parks_the_turn` (replaces `..._stays_running`),
`a_parked_turn_is_not_demoted_to_interrupted` (the regression),
`a_parked_turn_resumes_on_the_task_notification`, `a_parked_turn_is_not_done` (automove),
`a_parked_turn_has_its_own_slow_mark`, `working_outranks_a_parked_turn_on_one_card`.
No goldens moved — no fixture holds a parked session.

## A released board asks whether a newer one exists (2026-09-01, user request)

Before this, `update ready (U reloads)` could only fire for someone who had ALREADY updated:
`update.rs` watches our own exe's mtime, and on a released machine nothing ever moves that file
except a hand-run `install.sh`. The offer was real and the news that would justify it never
arrived. `mesimon-tui/src/release.rs` supplies the missing half — and deliberately only that
half.

**The two halves stay separate, and the join is a file move.** The checker asks the dist repo for
the newest tag, and on the offer being taken it downloads, verifies and lands the new binary at
our own path. It restarts nothing. The swap changes the mtime `update.rs` is already watching, so
the existing watch raises the existing chip and `U` is still the only thing that restarts a board.
No new key, no second reload path, and the two states are ordered rather than concurrent:
`Ctx::release_available` is ANDed with `!update_ready`, so a binary already on disk is reloaded
instead of fetched again — which is exactly the state an `install.sh` run in another terminal
leaves behind (`test_a_landed_binary_outranks_a_download`).

**The dev gate is a stamp, not a heuristic, because a wrong answer overwrites a build tree.**
`crates/mesimon-tui/build.rs` stamps `MESIMON_CHANNEL`, and it reads `release` only when
`ci/release.sh` set `MESIMON_RELEASE` for that one build. A `cargo run`, a plain `cargo build
--release` and every test binary come out `dev` and are inert — `a_dev_build_is_never_eligible`
asserts the test binary itself is one, so the gate is checked by the suite that runs inside it.
Under that stamp sits one guard nothing lifts: an exe with a `target` component in its path is
refused whatever the channel says (`a_binary_in_a_build_tree_is_refused`). `MESIMON_UPDATE_CHECK=1`
forces a dev build past the CHANNEL gate only, so exercising the real path means copying the
binary out of the build tree first — which is what an install is.

The stamp being invisible is its own failure mode: an unstamped release installs, runs, and then
never tells anyone a newer version exists. So `ci/release.sh` now greps `doctor install` on the
UNPACKED artifact (never on `$bin`, which is inside `target/` and would be refused by the guard
above) and dies if the line says `off`.

**Four decisions worth the ink:**

- **`curl`, not an HTTP crate.** mesimon makes exactly two GETs in its whole life and `install.sh`
  already makes both. The alternative is a TLS stack linked into a binary that otherwise touches
  no network. It goes off the UI thread on a worker, one `Outcome` per worker, drained by
  `App::tick`.
- **The list endpoint, not `/releases/latest`** — that one skips prereleases and every alpha is
  one, so it answers 404 until the first stable build. Same trap, same note, as `install.sh`.
- **An absent checksum is a refusal here, where `install.sh` only warns.** A person is watching
  the installer and can decide; nothing is watching this, and what it is about to overwrite is the
  binary you are running.
- **`~/.local/state/mesimon/update-check.json` is a CACHE and is treated as one** — unreadable, or
  from a newer build, means ignored and rewritten, not quarantined. That inverts the four state
  files' rule on purpose: an ignored stamp costs one extra HTTP request, where an ignored
  `sessions.json` costs the board. It is at the state ROOT, not under a project key, because the
  binary is one per machine and asking once per repo would ask the same question N times. It is
  written only on an answer, so a week offline never reads back as a week of successful checks.

**Ctx gained its first `String`.** `release_tag` is a version, not a word from a fixed set, so the
`&'static str` treatment `undo_word` gets could not carry it — and the chip and the row both name
the version, because "an update is available" with no version is a claim you cannot look up,
decline, or report a bug against. The cost is that `..ctx` struct-update syntax now needs a clone
(one test site moved). `avail` requires the tag as well as the flag, so a stem row is unreachable,
and `every_menu_row_is_spelled` grew a trailing-whitespace assert to catch the shape.

**The offer comes down while the download runs**, which is the shell-env reload's discipline
(`shell_env_stale && !reloading`) rather than a new one: a menu row that is standing but inert is
worse than a status line, and the status line is what says where this stops — `nothing restarts
until you say so`.

**Product-promise consequence, OPEN and deferred to the author (2026-09-01).** Taking the offer
writes mesimon's own binary at its own path — and the `mesimon-tmux` beside it, where one exists —
which is outside README promise 1's allowlist as that promise is worded. A narrow clause naming
the exception was drafted and then **withdrawn at the author's direction**: promise 1 is a
commitment to users, so its wording is the author's, not a side effect of the feature that made it
necessary. What shipped instead is disclosure without a promise change — `update-check.json` has a
row in the writes table (that path was already inside the state dir promise 1 names, so it needed
no exception), and the binary write has none. Two items are therefore open on the same paragraph:
this one, and the pre-existing gap where promise 1 never names the `/tmp/mesimon-<uid>/<proj16>/`
runtime dir, which this feature also stages a download in. Both want one authorial pass.

## A quick tag opens the card it tagged (2026-09-01)

The stripe is ONE cell at rest and carries no words — it can say "two tags, these hues" and
nothing further — so the digit that changes it left the user reading a status line to find out
what the card now wears. `cycle_tag` therefore arms `App::tag_flash`, and the card the digit
landed on draws itself OPEN for 1500 ms: the chip row names the tag in its own tint, and where the
stripe is now three or more cells `tags::stack_full` runs the two tags down it as full blocks
instead of a half-block. `App::peek_showing(ticket)` is the whole seam — the `p` preference OR a
live flash — and `ui/board.rs` is its only caller.

- **Keyed to the TICKET, not to the board.** The first `j` ends the reveal, and no second card can
  ever be open behind the cursor. It also means the ticket screen's copy of the digit costs
  nothing: nothing there reads the flash.
- **A moment, not a mode, and the preference is untouched.** `Ctx::peek_on` stays `self.peek`, so
  `p` keeps hinting `show replies` while a flash is up — the footer describes the toggle's state,
  and a flash is not a state anyone toggled. The reveal expires against the wall clock at draw
  time, and the loop repaints every ≤100 ms (`event::poll`), so nothing schedules anything.
- **Every repeat re-arms it.** Walking an axis with one finger keeps the card open for the whole
  walk instead of blinking once per press. A refusal (an axis with no vocabulary) arms nothing —
  there is nothing to show.
- **D19's motion ban is not bent.** The card changes shape on a keypress and again when the
  reveal lapses; that is the cursor-card marquee's precedent (a time-driven reveal the cursor
  landing starts), not the spinner's.

**And the tag row stopped being the agent's.** It was gated on `peek.is_some()` — a claim about
the AGENT having a transcript — so a session-less card could not show it at any setting. That is
the commonest tagged card on the board and exactly what a quick-tag digit lands on: a backlog
ticket. The gate is now `open && !tags.is_empty()`, the accordion opens for it with no sessions at
all, and `card::render` takes `open` beside `peek` to say so. No golden moved
(`test_a_quick_tag_names_the_tag_on_a_session_less_card` is the picture that was missing).

## Shift+Enter asks the agent from the board (2026-09-01, user request)

The composer's Shift+Enter minted a ticket, spawned claude and submitted the title without leaving
the board. Everything after that first sentence needed the pane: the only way to say a second thing
to an agent was `enter`, which spends the terminal on a handover and takes the board away. So the
same key got its second stage — **on a ticket whose agent is already running, Shift+Enter opens a
one-line field on the card; Enter sends it and the board never moves.**

- **One key, one sentence, three stages.** Before the ticket exists the press mints it, spawns and
  asks the title (`Verb::SaveStart`); on a ticket with a live agent it opens the field
  (`Verb::Prompt`); inside the field it sends. `shift_enter_asks_claude_at_every_stage` is the
  test that notices when a fourth home makes it two ideas. The atom is off the legacy floor, so
  what it buys has to stay one idea — that clause is what admitted it in the first place.
- **`ticket_promptable`, not `ticket_has_claude`.** `is_live()` counts `Sleeping`, and a parked
  agent has no process to type at. The new `Ctx` field is `has_pane()` and mirrors the daemon's
  `prompt_target`, which picks the same session `board_enter` focuses — the key that asks and the
  key that goes there cannot land on different panes.
- **`Command::PromptSession { ticket, text }` is `MergeToAgent`'s twin, and the difference is
  whose words travel.** Both are an explicit gesture pasting into a live pane; the merge flow
  pastes mesimon's sentences, this one pastes only the user's. That is why this one hangs off an
  ordinary key while that one is a staged confirmation. Delivery is `paste_text` — bracketed paste
  then a SEPARATE `send-keys Enter`, because a CR in the same byte burst is absorbed as pasted
  content (T-5). The e2e's stub agent is a `read` loop writing to a file, so a line in that file
  proves BOTH halves at once.
- **README promise 3 holds in its strongest form.** `command::sanitize_prompt` only ever REMOVES —
  control characters (a bare CR would split one prompt into two turns, ESC would be read as a key,
  Tab is a completion inside Claude's box) and anything past 4 KB — and nothing is appended on the
  way to the pane. What Claude reads is a subsequence of what the user typed.
  `sanitize_prompt_only_ever_removes` checks that mechanically rather than by reading the code.
  Blank in, nothing out: an empty paste would press Enter on a turn nobody wrote.
- **The never-tier grew an entry.** `agent_allows` denies `PromptSession` — one agent steering
  another's turn with no human in between is the sharpest thing D10's never-tier exists to stop —
  and the local path names `Resource::Session` for it, so `authorize`'s "no agent reads or changes
  a session" is reachable rather than merely true.
- **The field hangs UNDER the card, and the card stays whole.** A rename takes the title line; a
  prompt must not, because the ticket is not what is being edited — it is who the text is going
  to. So the card renders complete (glyph, title, session rows, peek) and the field is appended:
  `  › ` plus the text, no bar on span 0 (the composer's workspace selector set that shape, and it
  keeps the row clear of `tags::stack_full`). The prompted card also stays the CURSOR card, which
  no other text field does — collapsing it mid-prompt would take the agent's own state off screen
  while you type at it. An empty field shows `ask claude` in dim3: without it the state is a blank
  row under a card.
- **The cost is width, knowingly.** ~26 cells of the sentence are visible and the rest scrolls. A
  full-width command line at the foot of the screen would type better and could not answer the one
  question that matters — *which agent* — so the card won. The card is also where the reply comes
  back (the peek row is two lines up), so question and answer share a place.
- **Words that are true here.** The mode word is `ASK`, not `PROMPT`: every other field on this
  screen saves to the board and this one leaves mesimon entirely. `Ctx::prompting` turns the input
  scope's `enter save` into `enter send`. The second Shift+Enter is bound (the finger is still
  holding shift from the press that opened the field) and deliberately unhinted — `enter send` one
  cell to the left already teaches it, and two footer cells reading "send" teach nothing twice.
- **The status line says `asked`, not "sent to claude".** What is provably true is that the text
  went into the box and Enter was pressed. Whether the agent took it is the card's to say, seconds
  later, in the only vocabulary ever trusted for it — the hooks. The record stays `Idle` until
  `UserPromptSubmit` lands, exactly as it would for a prompt typed in the pane.

Deliberately board-only. The ticket page has a rail with its own selected session, so "which
claude" has a different answer there; that is a second decision, not a free extension of this one.

## The simplify pass: one table, one scrubber, one harness (2026-09-01, user request)

Author's brief: the last few hours of feature work were slow, and the board is now in the hands
of friends, so a quality pass over the whole tree with two lenses — what makes a feature cost
more edits than it should, and where a trust boundary was written more than once. Four review
angles (reuse, simplification, efficiency, altitude+security), ~45 findings, these applied:

- **`Command::meta()` is the one classification** (`core/src/command.rs`): read or mutate,
  logged or not, which ticket. `handle` had three parallel tables (the D32c action, the feed name
  hand-spelled for 27 commands, and a `_ =>` default), and the feed name is now `wire_name()` —
  read back from serde, so the wire spelling and the feed spelling cannot differ. Exhaustive like
  `agent_allows`; a new command is a compile error until classified. `Resource` stays in the
  daemon because `PromptSession`'s resource needs board state.
- **`core/src/text.rs` owns the hazard lists.** Seven sanitizers carried four different lists and
  only the census one stripped bidi overrides: a tag, a prompt, a peek reply, a pane title and a
  ticket title could all carry an RLO. Now `scrub_cells` (before a cell; box drawing out) and
  `scrub_text` (before another process; box drawing kept, since a pasted table is the reader's
  business), and ticket titles are scrubbed at the daemon, which they never were. The census
  keeps its "a control is a word break" rule at its own call site.
- **The gate refused every edit in a ticket worktree.** `--deny-state <state_dir>` and worktrees
  live at `<state_dir>/worktrees/` — verified by piping a worktree path through the built
  binary. `mesimon gate --allow <root>` exempts a subtree, `Paths::WORKTREES_DIR` is the one
  spelling, and `hook_settings` renders it. The worktree e2e uses a stub agent, which is why no
  test caught a real agent being refused.
- **The boundary is checked, not assumed.** `Paths::ensure_dirs` now goes through
  `own_private_dir`: not a symlink, owned by this uid, 0700 — for `/tmp/mesimon-<uid>`, the
  runtime dir, and both state dirs. Sticky `/tmp` lets anyone pre-plant the parent; before, a
  foreign-owned one failed only because chmod returned EPERM, and a symlink was followed. Both
  sockets are 0600 (orch.sock was at the umask), and `sessions.json`/`worktrees.json` are written
  0600 (`write_atomic` takes the mode; board files stay at 0644).
- **Both socket reads are bounded.** `hook.sock` read to EOF with no cap and `mesimon hook`
  forwarded its whole stdin — a `Write` payload carries the file — and `orch.sock` buffered a line
  of any length from a client that is, by design, untrusted (the MCP shim). 1 MiB each; the hook
  drains stdin past the cap so the agent's write never EPIPEs, and an overlong request line ends
  the connection.
- **One git runner** (`daemon/src/git.rs`): `diff.rs` alone scrubbed `GIT_DIR`/`GIT_WORK_TREE`/
  `GIT_INDEX_FILE`, and the unscrubbed copies were the ones running `worktree add`, `branch -D` and
  the merge. The daemon is spawned from inside a worktree session whenever the author dogfoods.
- **`Board::pane_target` and `SessionRecord::pressable`** are in core: the daemon picked and the
  TUI hinted by two copies of "the ticket's claude with a pane", and `retry_pending_submits`'
  predicate was copied into `glyphs.rs`'s doc comment.
- **Ctx derives `Default`**; the four `_word` fields fall back at their hint. `Scope::ALL` sits
  beside the enum, and `scope_list_is_complete` is an exhaustive match, so a new scope is a
  compile error and then a length failure until listed.
- **The e2e harness is shared** (`tests/common/mod.rs`): `TestClient` was pasted into 15 files
  (byte-identical struct, drifted methods), `hook_send` had three signatures, and 14 teardowns
  ran a bare `tmux kill-server` while `ci/release.sh` runs the suite under `MESIMON_TMUX_BIN` —
  a client from another build refuses the server over protocol version, so the release gate
  leaked a private server per test. Now `tmux()`/`kill_tmux()` via `tmux_bin()`, `wait_until`,
  `sweep`, and `Harness::boot` with teardown on drop. `prompt_e2e` is migrated as the exemplar;
  the other 14 still boot by hand (identical shape, ~45 lines each) and can move one at a time.
- **The loop itself.** `cargo test --workspace` was 2 min 15 s at 12% CPU: 16 e2e binaries run in
  series, each waiting on tmux and hook timers. `cargo nextest run --workspace` runs binaries in
  parallel (each test already owns its dir and socket); `cargo ut` is the unit-only inner loop;
  `doctest = false` on the four libs (4 s of rustdoc for zero doctests); `debug =
  "line-tables-only"` in the dev profile for the 23-binary relink. CLAUDE.md gained the recipes
  section — where a command, a binding, a visual, an e2e and a text channel each go — because it
  had two "to add X" recipes in 668 lines and the reviewers counted the ceremony at six files for
  a command.

Reviewed and NOT applied, recorded so the next pass does not re-derive them:
- `Principal` is a self-declared field on the envelope; any same-uid process (the agent's own
  included) can claim `Local`. The never-tier is a contract with a cooperative agent, not a
  boundary against one; the uid IS the boundary. Deriving the principal from the peer pid
  (`LOCAL_PEERPID` → pane ancestry) would make it one. A design decision, not a cleanup.
- Release verification is integrity (a `.sha256` beside the tarball), not authenticity. A signing
  key is the fix and belongs with the README allowlist rewording already on record.
- `refresh_worktree_flags` forks git four times per binding every 10 s ON THE WRITER THREAD
  (`for-each-ref --format=%(ahead-behind:)` on the provisioning worker would make it one fork,
  off-thread). Every tmux probe doubles its forks with a `has-session` prefix. The tick fsyncs
  `sessions.json` on RSS drift that lives only in memory. The TUI draws every 100 ms and rebuilds
  `Ctx` 3-4 times a frame. All real, none in the diff, none a cleanup.
- `key_tag`'s "a non-ASCII stray key means the terminal ate Alt" heuristic changes behaviour on a
  Hebrew layout; the total form is "an unresolved key in the picker is inert". Behaviour change.
- The footer row is baked into every board golden, so a hinted binding regenerates 28 files.
  Masking it and pinning the footer once per screen is a test-infrastructure decision.
- `directional()` vs key-reading in `dispatch` (12 Verb variants exist only to spell a
  direction); `dispatch` at 443 lines could split by `Group`; the 29 `MESIMON_*` seams could be
  one `Tunables::from_env()`; `TagName`/`PromptText` newtypes would delete the daemon's remaining
  sanitizer call sites; `hook_send`'s frame is still encoded in `hook.rs` and `gate.rs` and decoded
  in `ingest.rs` separately.

## Idle teammates do not hold a turn open, and a parked turn resumes on its own tool (2026-09-01, dogfood)

T-135 ("simplify") wore the slow background mark for an hour after its agent had said "Done",
with no shell on the ticket. `sessions.json` had it at `Idle{Background}`, High, unchanged since
the first Stop at 19:32 — through twenty minutes of the agent's own tool frames, two more Stops
and the idle Notification. Two defects, both measured against Claude Code 2.1.257 with a Stop
hook that dumped its stdin (four throwaway sessions, headless and interactive-in-tmux):

- **A named `Agent` in an interactive session is an in-process teammate, and a teammate is
  listed `{type: "teammate", status: "running"}` in every later Stop payload for the rest of the
  session — idle or not.** The `/simplify` skill spawns four (`reuse`, `simplify`, `efficiency`,
  `altitude`); they reported at 19:30–19:32 and sat idle from then on. Claude Code's builder
  only lists tasks whose status is running/pending (a finished shell or subagent DOES drop out,
  captured), but a teammate's status never changes while it lives. `task_blocks_end_turn`
  excused only `monitor`, so every Stop targeted `Idle{Background}` — the state it was already
  in — and `apply` returned `None`, exactly the T-72 shape one state over.
- **A teammate's report wakes the lead as a teammate message, which fires no
  `UserPromptSubmit`.** The T-128 record said "the wake path already worked — the completion
  arrives as a `UserPromptSubmit`"; that is true of a TASK NOTIFICATION (a background shell, an
  unnamed subagent — re-captured: two prompt frames, the second `<task-notification>`), and false
  of a teammate message (captured: one prompt frame in the whole session, the human's). The
  activity feed for T-135 has four `UserPromptSubmit`s, all before 19:32. What arrived instead
  were the lead's own `PostToolUse` frames, and `ToolCompleted` was inert from a High Idle by the
  "a background task's completion must not flip a real end_turn" rule — a rule guarding against
  a frame that does not exist: a backgrounded shell's completion emits NO `PostToolUse` (captured;
  the single PostToolUse is at launch, carrying `backgroundTaskId`).

**The fix keeps teammates blocking while they work and lets the idle notices say when they stop.**
The payload cannot tell an idle teammate from a busy one, but `TeammateIdle` (already in the hook
set, fires with `teammate_name`) can, and `SendMessage`'s `tool_input.to` says when one is woken
again. So:

- `task_blocks_end_turn("teammate")` is now `false`, and `Signal::Stop` carries `teammates:
  usize` (entries whose type is a teammate, `is_teammate_task`). Every other type keeps its
  class — a shell beside idle teammates still parks.
- `Machine` keeps `idle_teammates: BTreeSet<String>`: `TeammateIdle{name}` inserts,
  `TeammateMessaged{name}` removes, in every state including the latched ones. A Stop parks iff
  `blocking_tasks || teammates > idle_teammates.len()`. Fewer listed than idle (one was shut
  down) is still "all accounted for"; a repeat idle notice is not a fifth teammate.
- The set is persisted as `SessionRecord.idle_teammates` (`#[serde(default)]`, synced by the
  daemon on the two bookkeeping frames, restored via `Machine::restore_with_teammates` at
  startup and in the supervisor-dead demotion) — a restart that forgot it would park the next
  finished turn for good, which is the bug in a new coat.
- `Signal::ToolCompleted { nested }`: `nested` is the payload's `agent_id`, which a subagent's or
  teammate's tool carries and the session's own does not (captured on the same wire: the
  parent's `Bash` has no `agent_id`, the subagent's has `agent_id` + `agent_type`). From
  `Idle{Background}`, an un-nested completion promotes to `Running` at High — the lead's own tool
  ran, so its turn resumed, whatever woke it. A nested one is the teammate's work and moves
  nothing, so a lead genuinely parked on working reviewers stays parked and out of the quiet
  probe's reach. `Idle{EndTurn}` stays inert as before: nothing is owed after a real end.

**Not changed:** `SubagentStop` still promotes only below High (a teammate finishing does not
mean the lead resumed; its own frames will say so). `Idle{Background}` keeps its glyph, rank and
automove treatment. A session already parked before this build has an empty idle set and its
teammates will not report idle again unprompted — T-135 itself is unstuck by sleeping or exiting
it, not by the upgrade.

Tests: `busy_teammates_park_the_turn`, `idle_teammates_do_not_hold_the_turn`,
`a_messaged_teammate_is_working_again`, `a_parked_turn_resumes_on_its_own_tool_completion`,
`restore_carries_idle_teammates` (core); `teammates_are_counted_not_blocking`,
`teammate_frames_carry_names_and_nesting` (ingest, on the captured payload shapes). Corrects the
T-128 block's wake claim above. Captures live in the session scratchpad, not the repo.

## A card's age is time in column (2026-09-01)

**Was:** the board card's age slot counted from the newest `state_changed_at` across the
ticket's sessions, so it reset on every hook — a ticket three days into REVIEW read `now` the
moment its agent said one more word — and a session-less card carried no age at all (07 §4.4,
"created_at staleness handling is deferred").

**Is:** `Ticket.entered_at` (`@<secs>`, `#[serde(default)]`, a scalar so it sits before the
tables in `ticket.toml`) is stamped by `mint_ticket` and by `place_ticket` on a column change —
never by `reorder_within`, a rename, a tag or a session — and by `unarchive_ticket` only when its
fallback lands the ticket in a different column. `Ticket::column_since` is the read side and falls
back to `created_at` for a ticket from before the field, the only honest value left; the card
renders `age_slot` off it for every card, session or not, and the seconds band still ticks only
while an agent is working. No schema bump: an older build ignores the field on read and drops it
on its next write, which is a stale age, not a lost ticket. The single-session open card still
lists no session row (the glyph still duplicates line 1; the session's own age is the ticket
page's). Goldens: every session-less card gained a right-hand `>1y`. E2e: `reorder_e2e` asserts
reorder keeps the stamp and a cross-column move advances it.

## The environment travels inside the pane (2026-09-01, user request)

Found while sampling processes during the simplify pass: two live panes' `new-session` lines
carried the user's full shell environment as `-e KEY=VALUE`, API keys in clear. On macOS
another user can read any process's arguments, so for the life of each spawn the environment
was machine-readable. A terminal-started `claude` never has this problem — a shell passes its
environment through `execve` — so the `-e` road was the one place mesimon was weaker than the
baseline it replaced. The capture itself (clean base, login shell, denylist, explicit reload)
was right and is unchanged.

- **`mesimon exec --env <file> [--set K=V]... -- <argv>`** is the pane launcher. It reads
  `<rt_dir>/shellenv.env` (0600, `K=V\0`, written whole-or-not by `write_atomic` on every
  capture), applies it, applies `--set` last, and `exec`s: the pid is still the agent's, tmux's
  `pane_pid`/`pane-died` are untouched, no shell runs. A missing file warns on the pane and
  execs anyway — the pre-capture behaviour, never a dead pane. The record keeps the RAW argv;
  `Daemon::launch` wraps at spawn, so a moved binary wraps with its new path.
- **`TmuxBackend::spawn` lost its `env` parameter.** There is no `-e` road left to misuse.
  The pinned tmux measurement keeps its half that still matters (a pane's PATH is the
  client's); the `-e PATH` decoy is gone with the mechanism it decoyed.
- **PATH is no longer an exception, but it still takes two roads.** It is in the file like
  every other variable (the launcher is the authority in the pane) AND on the tmux client
  (`set_path`), because tmux resolves its own commands against the client PATH. One road would
  leave tmux and the pane disagreeing; that is what `core::shellenv::PATH_IS_THE_CLIENTS` now
  says.
- **`--set` stays on argv on purpose.** `MESIMON_TICKET`/`MESIMON_WORKTREE_BRANCH` are not
  secrets, and `ps` naming a pane's ticket is a feature. `mcp_e2e` reads the ticket key from
  `#{pane_start_command}` now instead of `show-environment`, which only `-e` ever filled.
- **The regression test is the exposure:** `shell_env_e2e` asserts the exported value is NOT in
  `#{pane_start_command}` and IS in the pane's environment; `exec_e2e` runs the real binary with
  a bare command resolvable only on the file's PATH.
- Deferred at the author's word: the README sentence stating the stance ("an agent pane gets
  your login shell's environment, secrets included, as a terminal-started session would").

## A pending settle survives a shutdown (2026-09-01)

**Was:** `Command::Shutdown` set a flag and let the writer loop fall out; SIGTERM had no handler
at all. A `Running → Idle{EndTurn}` still inside its 1500 ms settle died with the process, and the
restart's transcript-tail re-derivation carries `Confidence::Low`, where `automove` refuses to
move — so a `Stop` one second before a `U` reload left T-140 in IN PROGRESS with its turn over
(dogfood 2026-09-01; the feed shows `Stop` 23:38:35, the new daemon at 23:38:36, `idle/end_turn`
at low 23:38:38, no automove).

**Is:** one shutdown road, `Daemon::begin_shutdown`: `Machine::flush` commits every pending
transition at once (the settle exists to absorb a re-trigger, and at exit there is nothing left
to absorb; stale demotion is a clock, not a signal, and is not flushed), each goes through the
ordinary `apply_change` — feed line with hook `shutdown`, automove, `persist_sessions`,
broadcast — and only then does the loop exit. `Command::Shutdown` calls it. SIGTERM takes the same
road: `install_sigterm_handler` (the `daemon` subcommand only — never the in-process daemons the
e2e suite runs in the test runner's process) raises `TERM_REQUESTED` and resets the disposition
to `SIG_DFL`, so the next wheel tick (≤ 250 ms) runs `begin_shutdown` on the writer thread and a
second TERM still kills. `pkill -f "mesimon daemon"` is therefore a clean exit now, not a signal
death. E2e: `shutdown_flush_e2e` — the in-process road via `Shutdown`, and the real binary under
`kill -TERM` (asserts exit 0, socket gone, ticket in REVIEW on disk, record `idle/end_turn`).

## The prompt field remembers what was asked (2026-09-02)

**Was:** the board's Shift+Enter field (`InputPurpose::Prompt`) opened empty every time, so
"run the tests" on the fourth ticket of the day was typed for the fourth time, and an ask sent
to the wrong ticket was gone the moment Enter landed.

**Is:** a shell-style history, in the field and nowhere else. `App::prompt_history` holds every
ask sent this TUI run, oldest first, one copy of each (a repeat moves to the end), capped at 50 —
in memory only: a recall aid, not a record, and the transcript already is the record, so no
state file and no write outside the allowlist. `↑` (`Verb::HistoryPrev`) keeps the draft under
the cursor and shows the newest ask, each further `↑` goes one older and the oldest is a wall,
not a wrap; `↓` (`Verb::HistoryNext`) walks forward and the step past the newest puts the draft
back — the walk lives on `InputPurpose::Prompt { walk: Option<HistoryWalk> }`, so it dies with
the field. **Both keys are gated on `Ctx::prompting && Ctx::prompt_history`**: a composer has no
earlier titles, and a field with nothing to recall hints nothing (`↑ earlier asks` appears only
once something was asked; `↓` is bound and silent, the `> <` shape). The `board_prompt` golden
is unchanged for exactly that reason. Barrier scopes are exempt from the one-verb-per-key rule,
which is why `↑` may mean this here and cursor-up on the board. Tests:
`prompt_field_walks_its_history_and_comes_back_to_the_draft` (app.rs) and
`prompt_history_is_the_prompt_fields_and_needs_a_past` (keymap.rs).

## Linux ships, and every tmux before 3.6 rewrites a tab (2026-09-02, user request)

**Was:** the README said "Linux is buildable but untested and unshipped", `install.sh` refused
anything but Darwin/arm64, `release.rs` knew one asset name, and the suite had only ever run
against tmux 3.6a — the brew one and the bundled one, which are the same build.

**Refuted, by measurement:** the workspace cross-compiles clean for Linux with zero code changes
(`cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu`), and then 4 tests fail
on Debian 12 with every pane alive. `tmux` 3.2a (Ubuntu 22.04), 3.3a (Debian 12), 3.4 (Ubuntu
24.04) and 3.5a (Debian 13) all rewrite a control character in `-F` format OUTPUT as `_`; only
3.6a passes a tab through. `snapshot`/`activity`/`titles` split on `\t`, so on any distro tmux
the parse yielded nothing: reconcile read every session as `Exited{Crashed}` after a restart, the
Esc-interrupt probe never saw a quiet pane, and the backend's own roundtrip test failed in 10 ms.
Not in tmux's CHANGES; found by instrumenting the run (`abc123_276_0_` where `abc123\t276\t0\t`
was expected). Autopsy of the servers the failing tests left running is what separated "pane
died" from "parse failed".

**Is:**
- **`SEP = '|'`** in `backend-tmux/src/lib.rs`, one constant for the format string (`fields`)
  and the parsers (`parse_snapshot`, `pairs`), so they cannot drift; a unit test pins that it is
  printable and that a title containing it survives `split_once`. Verified: 4/4 on tmux 3.3a,
  the backend suite on 3.6a.
- **Linux is a shipped target, two of them:** `x86_64-unknown-linux-musl` and
  `aarch64-unknown-linux-musl`, cross-linked FROM THE MAC by the toolchain's own `rust-lld`
  (`ci/build-linux.sh`; the dependency graph has no C, so no cross C toolchain). Static musl
  rather than glibc so one binary runs on Ubuntu 22.04's glibc 2.35 as well as Debian 13's —
  and under WSL2, which is the Windows answer (native Windows is 26 compile errors in the daemon
  plus the SIGTERM ladder plus tmux itself; not a target). `release.rs` carries `PUBLISHED`, and
  a test reads `ci/release.sh`, `ci/build-linux.sh` and `install.sh` so the three name lists
  stay one. `install.sh` derives the asset from `uname`, verifies with `sha256sum` or `shasum`,
  and prints the rc line for the shell it is in.
- **The release gate grew a Linux leg:** `ci/test-linux.sh` runs the whole suite in Docker
  (Debian 12, arm64 native, the DISTRO's tmux 3.3a — the version a `sudo apt install tmux`
  gives, which is the point) and dies without Docker rather than skipping; `ci/release.sh`
  packages a tarball per Linux target and executes each inside a `debian:bookworm-slim` of its
  own architecture (`--version` + the `update checks` stamp), the x86_64 one under emulation.
  Bundled licenses and `mesimon-tmux` are macOS-only in the package.
- **tmux is NOT bundled for Linux** — a deviation from alpha-2's "ships its own tmux". Every
  distro packages a tmux mesimon now runs on, `ci/build-tmux.sh` is Darwin-bound (otool, a
  Mac-only static recipe), and a second static tmux build was more risk than the fix above
  left behind. `doctor`'s floor is now **3.3, on two floors**: below 3.1 the conf does not
  parse; 3.1–3.2 runs but `allow-passthrough` does not exist yet, and before the option existed
  passthrough was simply on, so T-10's containment is a line tmux ignored — a WARN naming it,
  never a FAIL, since every distro tmux today is 3.2a or newer. `tmux_verdict` is the pure
  function; a test pins both floors. The "tmux binary" note reads `(shipped with mesimon)` off
  the sibling's NAME now, not off "is absolute" — the ladder resolves PATH to an absolute path
  too, so the old test said "shipped" for a brew tmux.
- **The clipboard is decided at runtime on Linux** (`conf::linux_clipboard`, pure over three
  facts): WSL (`/proc/version` names Microsoft) is `clip.exe` via interop, Wayland is `wl-copy`,
  X is `xclip`; absolute paths, since the pipe runs under the server's frozen D29 env. Before
  this `copy_pipe_cmd` was `None` off macOS and a Linux copy silently stayed in tmux's buffer.
- **doctor learns WSL:** the `os` line says so, `git` warns when the repo is under `/mnt` (the
  9p bridge makes git an order of magnitude slower and mesimon shells out to it constantly),
  and `install` warns when `curl` is missing, since the checker is silently inert without it.
- **Verification recipe** (also the CI matrix's second leg, `ubuntu-24.04`): the tree mounted
  read-only into `rust:<local rustc>-bookworm`, build in a named volume, `cargo test --workspace
  --locked` under `MESIMON_REQUIRE_TMUX=1`. ~90 s warm. Never mount the checkout read-write: the
  container's host triple is Linux and it would overwrite `target/debug`.

Deferred: a bundled Linux tmux (would need `build-tmux.sh` ported to an alpine/musl container);
OSC-11 flavor detection through Windows Terminal is unverified; a second `~/.claude` on the
Windows side is invisible to the adoption census.

## A working pane goes quiet, so the quiet probe waits a minute (2026-09-02, dogfood)

T-71's card kept dropping its spinner while its agent was visibly mid-turn (T-144). The activity
log shows why: `probe_activity` demoted the session `Running → Idle{Interrupted}` at Medium four
times in ninety seconds, each verdict `hook: null`, and each was undone by the very next
`PostToolUse` 3–11 s later. `card_glyph` has no arm for `Idle{Interrupted}` — rightly, an
interrupted agent is nobody's to watch — so the card went blank for each gap.

- **The probe's premise is refuted.** The 8 s threshold (STALE-MAP "the interrupt emits
  nothing", 2026-08-30) rested on "a turn in flight repaints sub-second (spinner); 8 s is ~8x the
  largest gap measured while working". Sampled `#{window_activity}` once a second across every
  live pane on 2026-09-02 (Claude Code 2.1.257): a working pane holds the stamp still for 6–10 s
  routinely, and the transcript puts the long silences exactly where the model is streaming a
  large `Edit`/`Write` input — nothing paints until the tool call is whole. Across the whole
  activity log since 2026-08-30, **19 of the probe's 40 verdicts were followed by a `PostToolUse`
  or a `Stop`** — the turn it had just declared dead, still running — with silences of 10–50 s
  behind the `PostToolUse` ones (plus one 15-minute outlier and the 200 s+ `Stop` ones, which
  predate "a turn parked on background work is its own state" and are that bug, not this one).
- **Fix: `PANE_QUIET_MS` 8 s → 60 s.** Not a smarter probe, because there is no smarter signal:
  neither the transcript nor a hook moves while a tool input streams, and reading the pane's
  CONTENT for Claude Code's own spinner line is a UI-string heuristic the design rules refuse.
  The number is sized for what the probe now is — the FALLBACK for the recordless Esc (one
  landing before the first assistant output). The primary catch, the transcript's `[Request
  interrupted by user]` record through `poll_tails`' abort-only class, lands in ~2 s regardless,
  and post-interrupt painting already held a pane "active" for 60–80 s live (the T-50 amendment),
  so the recordless case was paying most of that minute already. Sixty seconds clears every
  working silence measured.
- Seam `MESIMON_PANE_QUIET_MS` unchanged; `interrupt_e2e` (1.5 s) and `interrupt_tail_e2e`
  (600 s, i.e. off) set it and are unaffected.

Deferred: the flap-pin interaction — a session that misfired often enough sat at `Confidence::Low`
(`FLAP_MAX`), where `automove` refuses to move; with the misfires gone this should stop being
reachable in normal use, but nothing asserts it.

## The preview zone pages (2026-09-02, dogfood)

A long agent reply on the ticket page showed its first ~20 rows and a `~`, and nothing read the
rest — the diff's hunk pane had `{ }` / `pgup pgdn` and the zone beside it had nothing (author:
"`{ }` keys on ticket page to scroll big transcripts, like in diff").

- **Same verb, same keys, one more scope.** `Scope::Ticket` binds `{ } pgup pgdn` to
  `Verb::PageDown` (`resolve` folds `{`/`pgup` to `PageUp` exactly as it does for the diff), and
  `App::dispatch` routes the verb on `self.screen`: the diff pane or the preview zone. `jk` stays
  the rail's — the zone is read, not picked.
- **Hinted only while there is a further page.** `Ctx::preview_scrolls` is `App::preview_view.max
  > 0`, MEASURED BY THE LAST DRAW: the zone's height is a fact of the frame, not of the board, so
  the footer's "{ } page preview" appears under a reply that overflows and the keys are inert
  under one that fits. The footer is drawn after the zone in the same frame, so the first frame
  of a page is already right; a narrow terminal (one zone) resets the measurement.
- **The scroll belongs to the document, not the page.** `App::preview_scroll` is
  `Cell<Option<(key, rows_hidden_above)>>` where `key` hashes the session AND, for an agent, the
  reply text (`ticket::doc_key`) — moving the rail or a new reply landing reads as offset 0, so a
  page into one reply never opens the next one halfway; a shell's pane is one continuous stream
  and keys on the session alone. Draw clamps the request against the rendered rows and writes the
  clamp back, the diff pane's own treatment, and the press ALSO advances the measured offset, so a
  held key that queues several presses before a frame still turns several pages.
- **A shell tail opens at its bottom and is RELEASED there.** The zone's default for a pane is its
  newest line, so "no request" means `max`; `{` pins the window and new output no longer moves it;
  a `}` that reaches `max` clears the request instead of pinning today's last row, and the tail
  follows the pane again. One page back down after the pane grew lands one row short (a page is a
  page) and the next press releases — `test_preview_pages_a_shell_tail` says so.
- `rich::render_all` + `rich::mark_cut` (the `~` `finish` used to add) let the zone own its
  window; `render` stays for a zone that only ever shows the top. Page = window − 1 row of overlap.

Tests: `test_preview_pages_a_long_reply`, `test_preview_pages_a_shell_tail` (ui/tests.rs). No
golden moved: none of the ticket fixtures overflows the zone, so no footer gained the hint.

## The sleeping mark recedes with its bar (2026-09-02, author)

Author: "better sleep glyph (`z` is low effort). `⏾` can be nice, or an emoji without color." The
shape was measured against 06 §4.1's rule before anything moved, and the rule is what kept it.

- **Every picture fails presence.** By the same seven-face check (`fc-list :charset=` over Menlo,
  Monaco, `.SF NS Mono`, Courier New, JetBrains Mono NF, MesloLGS NF): `⏾` U+23FE POWER SLEEP is
  in the two Nerd Fonts only (2/7), `☾` U+263E and `◗` U+25D7 in Menlo + Meslo (3/7), `◔` U+25D4
  4/7. The doc rejected `⚑` at 3/7. The author's own profile is MesloLGS NF, which carries all of
  them — the exact trap §4.1 was written for: on SF Mono or JetBrains Mono a terminal falls back
  to San Francisco or STIX at another weight, and DejaVu Sans Mono on Linux draws tofu. An emoji
  is out on the `▪` precedent (Emoji_Presentation, two cells, colour, VS15 honoured by few).
  Nothing moon-shaped exists in the 6/7–7/7 set (`› ‹ « » ∙ ◦`).
- **What was low effort was the colour.** 06 §4.2 specifies the sleeping mark at `dim3`; the code
  rode `Register::Grey` = `dim2`, the same weight as the idle ring and the working spinner, beside
  a bar that `tags::bar_cell` had already faded to its Sleeping level. The glyph was the one
  element on a parked card not walking the ladder.
- **`Register::Dormant`** (`glyphs.rs`) resolves to `theme.dim3()` in `card.rs::register_style`
  and the ticket rail; both `card_glyph` and `session_glyph` put `Sleeping` on it. `card.rs`
  now routes the bar weight off the register (`Some((_, Register::Dormant)) => BarWeight::Dormant`)
  instead of matching the letter `z`, and `Theme::bar` gained the exhaustive
  `Live(Dormant)` arm, mapped to the dormant paint because a parked mark never asks for a live
  bar. `x` and `z` stay the board's two lowercase letters — the two "no process here" states.

Test: `test_sleeping_mark_is_dormant` (ui/tests.rs) reads the cell on the board AND in the rail;
no golden moved, because the goldens are text and this is paint. A nerd tier (06 §4.2's
`nf-md-sleep` U+F04B2, never auto-selected) remains the home for a picture, and `Tier` still has
two inhabitants: one glyph does not buy a third.

## One claude per ticket (2026-09-02, user request)

Author: "multiple claude sessions in one ticket can be problematic ... should we just limit to
one claude, if the user wants multiple they can use shell to create another one." Agreed, and
the audit that preceded it is the reason.

- **It worked mechanically and nothing was designed for it.** Every record is keyed by its own
  uuid — own tmux session, own hooks file, own `--session-id`, own transcript — so two claudes
  on a ticket parked and resumed independently and no bug lived on that road. Everything that
  has to pick *the* agent of a ticket assumed one: `Board::pane_target` (the board's Shift+Enter
  prompt AND the merge flow's rebase notice) took the first claude in spawn order whatever the
  second was doing; `board_enter` focused the first hot one; `auto_move` fires per session, so
  agent A's `EndTurn` moved the ticket to REVIEW while B still worked and B's next prompt moved
  it back, until the movegate fuse suspended automation; `card_glyph` ranks `✓` above the
  spinner, so one finished agent plus one working agent read as done; and the worktree lock is
  taken once, so both edited one checkout with no coordination. 00-DECISIONS' "holds N sessions
  of mixed kinds and mixed vendors" was about claude + shells, or another vendor, never N claudes
  on one work item — parallelism inside a ticket is the agent's own subagents and teammates.
- **The gate is the daemon's, on NEW records only.** `spawn_session` refuses `SessionKind::Claude`
  when `Board::live_claude(ticket)` finds one — `is_live`, so a Sleeping record holds the seat
  too — with "ticket already has a claude session — wake/focus it instead". `resume_session` and
  `wake_session` re-enter an existing record and are NOT gated, so a board written before this
  keeps every session it has, and `pending_spawns` replay through the same function and meet the
  same check. A shell is never gated: `S` still adds one, and `claude` typed into a shell pane is
  the second seat for anyone who wants it — no hooks, no attention, adoptable observe-only
  through the drawer, which is the right amount of support for an escape hatch.
- **`C` / `Verb::ClaudeNew` is gone**; `ShellNew` stays. It was `prio: 0`, so only the help
  golden moved (`help_ticket_120x30`). `c` keeps its one verb and gains a third hint: on a ticket
  whose claude is parked it reads `wake claude`, because `focus_kind_or_spawn` finds the Sleeping
  record and `focus_session` already resumes a paneless record before attaching — the key woke
  and attached before this, but the footer said `claude`, and the daemon never has to refuse it.
  `Ctx::ticket_has_claude && !ticket_promptable` is exactly Sleeping (live, no pane).

Tests: `c_wakes_a_parked_claude_instead_of_starting_a_second` (app.rs) pins the hint and that the
wire sees `ResumeSession`, never `SpawnSession`; `shift_stays_on_one_axis` now asserts `C` is
inert; `hook_e2e` asserts the refusal while its stub claude is alive; `exit_parks_e2e` gives the
no-transcript case its own ticket. Not done, on purpose: refusing the wake of a second parked
record on a pre-existing board — it would strand a conversation that already exists.

## The peek shows what the transcript holds, and a task notification is not a prompt (2026-09-02)

Dogfood: "transcript peek sometimes skips the latest agent message and shows an earlier one".
Two findings, one fix.

- **The walk was right; the record was missing.** `tui/src/peek.rs::latest_preview` was run over
  1,356 local transcripts against a full-file reference walk: zero wrong picks (the only
  differences were the designed `last-prompt` fallback past 256 KiB, and a session id that lives
  in two project dirs). Live probe of this session's own `.jsonl` (Claude Code 2.1.257): a
  message's records land in ONE append ~250 ms after the message finishes streaming — never per
  block — and **3 of 6 visible text blocks that preceded a `tool_use` in the same message were
  never written at all** (their `thinking` and `tool_use` records were). Nothing arrives late:
  0 out-of-order assistant records in 8,279 across 120 files. So mid-turn narration is not a
  reliable part of the transcript, and the peek's answer is the newest text that EXISTS. Not
  fixable from the transcript; the `Stop` hook's `last_assistant_message` covers the end of a turn
  only. Filed to Claude Code. Any later "peek is stale" report: check the file before the walk.
- **`<task-notification>` is the harness, not the user** (`core/src/adopt.rs::user_prompt`). The
  wake that ends a turn parked on a background task is a plain-string `user` record with
  `isMeta` false (22 of 22 in the local corpus), so the walk took it as the newest prompt and a
  card read `> <task-notification><task-id>…` with `thinking` under it. Rejected by its opening
  tag; the walk continues to the agent's last words. `<command-name>`, `<bash-input>` and the
  `*-stdout` records are also unflagged but are things the user typed or asked for, and are left
  as prompts on purpose.

## A late reply to the colour query is caught before it can type (2026-09-02)

Dogfood: under iTerm2's key-remap sheet (and "a few times" with no obvious trigger) the board
opened rename on the cursor card with `gb:1e1e/1e1e/1e1e\` in the field. Not another tab
sending keys — mesimon's own `OSC 11` query coming back after its 150 ms budget. Nothing can
unsend a reply: it waits in the tty queue and crossterm reads it as keystrokes. crossterm drops
the `DA1` half (`CSI ? … c` is an internal event) but has no OSC parser, so the colour half
arrives as `alt+]` `1` `1` `;` `r` `g` `b` `:` … `alt+\` — `1` `1` is quick-tag (a SILENT
mutation of the cursor card's group-1 tag, twice), `r` is rename, and the rest is the "title".
Anything that delays iTerm's answer past 150 ms does it: a preferences sheet, a background tab
it deprioritises, a `cargo build` pegging the machine. The startup query is exposed too (its
reply can land during the connect or crossterm's kitty probe, which queues what it does not
recognise for the first `read`).

Fix: `tui/src/osc.rs::ReplySwallow`, a grammar over the RAW crossterm key events, permanently
armed, fed by `App::on_key` before the text-field barrier and before the keymap (a field strips
Alt, so placed later it would miss the prefix exactly where the reply does the most damage).
The prefix `alt+]` `1` `1` `;` is nothing a hand types; the prefix keys are held and replayed in
order if the fourth never comes (a real `alt+]` costs one keystroke of latency, nothing else);
once the prefix is complete the reply is proven, body chars (`hex / : # r g`) are discarded up
to `alt+\` or ctrl+g (BEL), and a key outside the grammar ends the swallow and passes through
alone. Not a longer budget: a reply can always be later than any deadline, and the read blocks
the frame. Known gap: crossterm splitting the reply at its first byte would deliver a bare Esc
then `]`, and a bare Esc is deliberately NOT an opener (holding a real Esc would delay the menu
it opens); the tty hands the reply over in one write, so this is not expected in practice. The
cleaner root fix — the watch writing the query itself with no blocking read and the swallow
parsing the colour out of the reply — is not built: it means re-implementing colorsaurus's
parsing and lightness formula. Tests: five in `osc.rs`; `a_late_colour_reply_neither_tags_nor_renames`
and `a_late_colour_reply_types_nothing_into_an_open_field` in `app.rs`.

## Five themes, and the law learns three kinds (2026-09-02, user request)

The author wanted more themes and named the first: their Neovim `blue` scheme (Neovim's own
`blue.vim`, gold `#ffd700` on navy `#000087`, cursor line `#005faf`), the Borland / Norton
Commander look — which they run as nvim's LIGHT-mode theme, `astrodark` being the dark one.
Three shipped: `blue`, `amber` (a P3 phosphor monitor) and `green` (P1), beside graphite and
chalk. Two decisions were the author's: gold is `attn` and nothing else (cream body text, navy
ink on a gold title row — the one-saturated-colour rule stays whole), and the roster is the
'90s pack rather than Solarized/Catppuccin/Gruvbox, which are multi-hue schemes that lose most
of what people like about them under a one-accent, grey-ramp law.

**A theme became a table.** Graphite and chalk were two ~90-line constructors differing only in
constants, and the law tests carried a second hand-transcribed copy of the hexes. Now every
flavor is a `static Palette` (truecolor, 256, 16, a shared 8-colour form; the diff tints; the
tag ring; `shadow`) and `Theme::new` is one builder over it; the tests read the table.
`Flavor::palette()` is the exhaustive gate.

**Two things the nvim scheme wanted did not survive the numbers.** `#005faf` as the cursor-card
surface: it is L* 40, and a mid-ramp grey (L* 70) measures 2.7:1 on it against the 4.0 floor;
even index 25 puts dim2 at 3.0. The surface is `#2C3590` — L* 27, 8° off the navy, the same
one-step lift graphite's 234→236 makes — and the gold row is what carries the look. And
`faded()` blending toward the ground: a C* 34 tint blended 62% into navy is navy-hued whatever
it started as (169° of hue drift at the sleeping level, pairwise ΔE 3.9 — ten tags one colour),
and the old `C* ≥ 8` clause passed VACUOUSLY because the navy donated the chroma. Each palette
now declares `shadow`, the fade target: the ground on paper and phosphor, a neutral at the
ground's lightness (`#242424`) on blue. Drift is now ≤ 4.4°, and the pip law has a new clause
(Selected→Sleeping hue drift ≤ 20°) that would have caught it.

**The law is three kinds, matched exhaustively.** `Paper` is the old law. `ChromaticGround`:
C*(bg) ≥ 40, bg and selected within 15° of hue, selected ≥ 8 L* up, both ramps C* ≤ 8.2
(neutral ink on coloured paper), every register ≥ 60° of hue from the ground (blue: 100° err,
109° calm, 145° attn), the Paper chroma budget, shadow neutral within 3 L* of bg. `Phosphor`:
every token within ±6° of one hue (amber spread 1.8°, green 0.4°), C*(bg) ≥ 5, `attn` the top
by ≥ 8 L* AND C* ≤ 35 — white-hot is DEFINED by low chroma at the top, which is how it parts
from `err` on both axes (ΔE 40 / 61) — `err` ≥ 4 L* over sel.base, `calm` the pale rung (C* ≤
rest.base − 25, ΔE ≥ 20 from every ramp step), ink = bg, no diff tint, no ring. A phosphor is
a lightness ladder (L* 6 / 13 / 36 / 52 / 62 / 74 / 82 / 92) and the registers are named rungs;
which is which is the glyph's job, which is what a P1/P3 monitor can honestly do. Tight margins
to know: rest.dim2 4.44 and sel.dim2 4.25 (floor 4.0), err over sel.base 4.8 L*.

**The phosphors got their ring back within the hour.** The first cut shipped them ringless:
with a white-hot `attn` at C* 20–30 the 2× rule caps a tint at C* 10–15, where ten hues cannot
reach ΔE 12, and ten foreign hues on a one-hue screen looked like the fiction broken. The author
saw it and said the tags "don't render colored blocks correctly (it's simply amber / green)" —
a tag's colour is what the tag is FOR, and a theme does not get to take it away. So the clause
is restated for the kind, not exempted: on a phosphor the accent's loudness is lightness, so a
tint sits a register below it by ≥ 15 L* (attn 92, ring 62) instead of by half its chroma, and
the ring keeps ≥ 35° of hue from the phosphor, which every other token wears. Both rings are
graphite's L* 62 / C* 30 on ten hues at 29° spacing with a 70° band cut around the phosphor
(amber from 127.5°, green from 192.5°; worst pairs ΔE 14.8 / 14.3, ≥ 6.2 on bg). And the same
fade lesson as navy, milder: the sleeping level blended into the TINTED black drifted 47° and
fell to C* 4.7 on green, so both phosphors fade toward a neutral `#151515` (L* 6.8) — the
`shadow` clause is now "neutral within 3 L* of the ground" for every kind whose ground has a
hue. Blue's ring skips the navy's band (276–336°)
as well as the gold's (60–120°): ten hues at L* 70, C* 34 (ceiling 35 — a ring UNDER a C* 83
ground may be a step louder; gold at C* 87 leaves the 2× margin whole), worst pair ΔE 15.1.
`attn_is_its_own_colour` (ALL × four colour profiles: attn ∉ ramps ∪ registers ∪ surfaces ∪
bars ∪ pips ∪ diff) is what keeps `test_attn_provenance*` meaningful on a phosphor, and forbids
the tempting `{3,3,3,3}` eight-colour ramp whose base IS the accent. The indexed forms are
hand-authored: blue 18/19 (25 rejected), bright 9/14 for err/calm at 16 colours (the dark pair
is 1.7:1 on navy), Norton's cyan cursor surface rejected because `code_bg()` IS that surface;
amber and green sit on the grey cube's 232/234 (no dark phosphor in the cube), paint no cursor
card at 16 colours, and all three dark themes share graphite's eight-colour form — eight
colours cannot hold a navy or a phosphor, and saying so is 06 §2.7. Both provenance laws,
`test_no_banned_sgr`, `test_diff_add_del_registers` and the rich/tags sweeps now iterate
`Flavor::ALL`. The daemon stays theme-blind: its tmux chip wears graphite's pair everywhere.

## Themes are a menu row with two slots (2026-09-02, user request)

**Two slots, keyed on the GROUND.** The terminal's OSC 11 answer is the one fact the board has
about where it is read, and it only ever says light or dark — so the preference is a theme for
each answer (defaults graphite / chalk), the watch keeps flipping between the two picks, and a
pick sets the slot the terminal currently reports. That is the author's own editor setup
(`astrodark` dark, `blue` light) and it means picking blue never costs the light-mode board.
A terminal that cannot answer sets the dark slot. The watch now reports a `Ground`
(`detect::GroundWatch`); `App::watch_flavor` maps it through `prefs.for_ground`, and under an
open picker it only moves the slot the popup's header names — the preview stays.

**The picker's cursor IS the preview.** `App::theme` was already a plain field the watch
rebuilt wholesale; `App::preview(flavor)` is that, factored, and every retheme goes through it
(watch, picker cursor, Esc, Enter). `Mode::Theme { idx }` stores no entry flavor: Esc restores
`App::resting_flavor()` = the pin or the current ground's slot, which is also what makes a
ground flip under the picker right for free. Enter is `mutates: false` — nothing the daemon owns
changes. Words, never a mark, for "which slot holds this": `◦` is the suggestion chip's and
`›`/`◊` were rejected; the row's detail said `your pick for a dark terminal` until 2026-09-04, when
the author had the clause removed — the row's ground tag already says which slot, and the
blurb reads cleaner alone. The menu row sits
beside `p`: the two view preferences together, never a suggestion (a theme is not something
worth doing right now), never a footer cell (the `p` argument).

**`prefs.json` is at the state root and is a preference, not a cache.** `~/.local/state/mesimon/`
is inside README promise 1; `~/.config/` is not, and would collide with promise 2. One binary
per machine, one file per machine, beside `update-check.json` — with the OPPOSITE rule: a newer
schema is read where it can be and never written back (writes barred for the session, the four
state files' discipline), garbage falls to defaults with a status line and the next pick
rewrites it, saves MERGE into the loaded document so a name this build does not know in the
other slot survives a pick in this one (picking THAT slot is what replaces it), and the write is
the store's `write_atomic` (made `pub` for this one caller), 0644 because two theme names are no
secret. `lib.rs` loads it, never `App::new`: `prefs_path` None means never write, which is every
test app. `MESIMON_THEME` accepts every `Flavor::name` plus the old `dark`/`light` aliases,
still pins and disarms the watch — but the ground is asked ONCE under a pin, so the picker sets
the right slot — and a menu pick outranks it for the session (the more recent explicit choice)
while the status appends `MESIMON_THEME=… pins the next launch`; the row stays visible under a
pin because it is the only road to the file. `mesimon doctor` prints `theme  dark: … ∙ light: …`
and never asks the terminal (pipes). The peek toggle `p` is the first candidate to move into
this file; not done here.

## The release gate's Docker steps are paused (2026-09-02, author)

`ci/release.sh` ran the suite on Debian's tmux in Docker and executed each Linux artifact in a
container of its own architecture, and died without Docker rather than skipping. Both steps are
OFF by default now, on the author's call, until they declare Windows/WSL2 operational:
`DOCKER_GATE` reads `MESIMON_RELEASE_DOCKER` (default `0`), each step prints a `SKIPPED` line
naming the variable, and nothing else moves — the Linux binaries are still cross-linked and
published, unexecuted. The `never skips` reasoning in the header still stands and is what the
pause is measured against; restoring it is flipping the default to `1`.

## The ground watch is opt-in (2026-09-02, author)

`detect::GroundWatch` re-asked the terminal for its background every 3 s so an OS appearance
flip repainted the board live. Every query is a write to the tty and a 150 ms read back, and a
reply that comes in after the budget lands on stdin as keys. `osc::ReplySwallow` (shipped the
same day) catches the common shape, but the leak recurred — the board opened rename on a ticket
with reply bytes in it — and the author chose the easy road: the watch is armed only under
`MESIMON_GROUND_WATCH=1` (`detect::watch_enabled`). The one-shot startup query stays, so the
picker still sets the right slot and the launch ground is still the terminal's; `MESIMON_THEME`
still disarms the watch; the swallow stays in front of the keymap for the startup reply. What
was lost is the live repaint on an appearance flip — a relaunch (or a pick) is now how the
board follows the terminal. The root fix recorded under "A late reply to the colour query is
caught before it can type" (the watch writes the query itself, no blocking read, and the swallow
parses the colour out of the reply) is what would earn the default back; widening the grammar
would not.

## A paste is one event, and a title has a ceiling (2026-09-02, dogfood)

A multi-line paste into the composer, a rename or the ask field saved on its first newline and
typed the rest onto the board — the terminal was sending the clipboard as keystrokes, so a `\n`
was an Enter and the lines after it were `j`, `d`, `n`… walking the keymap. `lib.rs::init_terminal`
now arms bracketed paste for the life of the alt screen (`DisableBracketedPaste` on restore, so
a handover's tmux client gets the terminal the way it was found) and `App::tick` takes
`Event::Paste` whole into `App::on_paste`, which hands it to whichever text field is open — the
composer, a rename, the ask field, a tag name — and to nothing else: a paste on the board, or into
an open picker with no name field, is inert. `EditBuffer::paste` flattens it with `one_line`
(newline and whitespace runs become one space, the ends are trimmed, since the commonest paste is a
copied line with its newline still on) and inserts at the cursor, grapheme by grapheme. Newlines
are NOT kept even for the ask: the field is one line, and the daemon's `sanitize_prompt` already
dropped them (glueing the words together); a paragraph belongs in the pane.

Every `EditBuffer` now carries a byte `limit`, the daemon's own cap for that text, so what the
field shows is what the daemon keeps: `board::TITLE_MAX_BYTES` (2048, new — a title had no bound
at all, and a pasted document would have ridden the slugger, the feed and every card row forever;
`board::sanitize_title` is the daemon boundary on both `CreateTicket` and `RenameTicket`),
`board::TAG_MAX_BYTES` (24) and `command::PROMPT_MAX_BYTES` (4096). A paste past it is cut at a
cluster boundary and the status says `paste trimmed ∙ a title holds at most 2 KB` (bytes, honestly:
a character count is wrong in Hebrew); a key past it is inert; text loaded over it (an older
board) is kept whole and just not grown. Terminals without bracketed paste (none of the supported
ones) still send keystrokes, and there nothing changed.

## The tag block has two loudnesses, not three (2026-09-02, author)

"A card's state sets the tag loudness" (2026-09-01) gave the bar three levels — the cursor card
at the full tint, a card at rest one step down, and a parked ticket (a Sleeping session, no pane)
a second step down, still hued. The author read the third one as "too muted" after a day on the
board: a sleeping ticket's block was answering a question the glyph already answers, and paying
for it in the one thing the block is for, "which tag". `TagLevel::Sleeping` is gone, `Tints::fade`
is one factor (0.70 graphite, 0.76 chalk, 0.70 elsewhere), `card.rs` no longer reads the sessions
for the level — the cursor is the only input — and the law test holds one quiet level instead of
two. The card-not-palette rule stands (an untagged block still steps with the cursor); what
changed is that "selected or not" is the whole ladder. The per-flavor constants and the 12% step
floor recorded in "The simplify pass" and "Barely visible on light theme" still hold for the one
step that remains.

## Notes: files under the ticket, a full-screen editor, and two agent tools (2026-09-02, user request)

The ticket-directory IA the corpus drew (`docs/13 §13.3`, `docs/07 §14.1`) and STALE-MAP's own
"`add_note` (T1) is deferred until notes have any storage or surface at all" both land here, with
deviations. A note is `notes/<ULID>.md` beside `ticket.toml`, an opaque blob written whole by
`store::save_note` (`write_atomic`, `SHARED`) and never parsed; the daemon mints the id and
neither a person nor an agent ever supplies a filename (docs/15 §4.7). The metadata is a new
`[[notes]]` array of tables on `Ticket` — `NoteMeta { id, name, rev, created_at, created_by,
edited_at, edited_by }` — after `[[tags]]` and before `[archived]`, and `TICKET_SCHEMA` is 2 on
the columns file's reasoning: an older build would drop the array on its next write and orphan
the files. **`notes[0]` IS the description**: there is no `spec.md`, no separate field, and no
title on a note — its `name` is the body's first non-blank line, `#`s stripped, recomputed by
the daemon on every write (`board::note_name`) so nothing that lists notes needs a body. `rev`
exists because `edited_at` is `@<secs>` and two writes in one second look identical; the TUI's
cache and the preview page key on `(id, rev)`. The author is `Principal::note_author()` —
`local`, or `agent:<session-uuid>` (docs/13's `origin` vocabulary; the session id travels
inside the word so the file outlives the session record) — and the page renders it as
`you`/`claude`. `sanitize_note` is `scrub_cells(_, true)` capped at 32 KiB: newlines kept,
tabs become spaces (fenced code with tabs is the known cost), and the body is a subsequence of
what was written. Bodies never ride the snapshot (the board is cloned on every event);
`Command::ReadNote` fetches one, on the writer thread, and `WriteNote { note: None | Some }`
creates or replaces — blank text on an existing note deletes it. Docs/15 said `add_note`
"appends only; never replaces"; the user asked for create/edit, so `write_note` replaces, and
the file is written BEFORE the meta so a crash leaves an orphan file and never a listed note
with no file.

The agent tier is five: `read_note` (the body as the text block itself, not JSON around it) and
`write_note` join, `get_ticket` grows `description` (capped at 4 KiB) and a `notes` list, and
`agent_allows` admits `AgentReadNote`/`AgentWriteNote` — D10's T1 ANNOTATE, the home tags never
had. The binding still supplies the ticket; a note id off it reads as "no such note". The feed
gets `write_note` with actor `agent`, never the text. The write gate on `.mesimon/` is what makes
the tool the agent's only road, and the README says so now.

The TUI grew its first multi-line field. `Mode::Editor(Editor)` is a full-screen editor — a
title row over a `TextArea` body (`tui/src/text.rs`, one `String` + byte cursor + sticky
column, clean by construction: `insert` refuses control/format/cell hazards and `paste`
normalises CRLF then `scrub_cells(_, true)`, so the draw never scrubs and the cursor never
desyncs; no soft-wrap in v1, the cursor row scrolls under `edit_window`) — with two purposes.
`Compose` is the one-line composer in a bigger room: `Tab` (`Verb::Describe`, composer-only)
carries the title over, `^t` and Shift+Tab keep working (`compose_tags()` reads the picks off
whichever composer is open), `^s` mints ticket + workspace + tags + description as `notes[0]`
(`App::mint_ticket`, which the one-line composer now calls with no body), and Shift+Enter
mints and starts claude on the TITLE ONLY — the agent reads the description through
`get_ticket`, so the paste path is untouched. It is not a fourth Shift+Enter home: same verb,
same moment, second surface, and `shift_enter_asks_claude_at_every_stage` now says so and
asserts the Note purpose leaves the atom unbound. `Note` edits `n`'s target — the selected
rail note, else the description, else a fresh note that becomes the description — always
re-read from the daemon, never from the cache; `N` is always fresh (same axis, harder; silent
on the board, where the `?` overlay at 30 rows had exactly one row to spare). `^s` on a note
STAYS open, says `saved ∙ ^s again tells claude` when the ticket has a claude with a pane, and
the second press with nothing changed sends `Command::NoteToAgent` — `MergeToAgent`'s twin,
mesimon's own sentence naming the note and `read_note`, on a human gesture only (`agent_allows`
denies it) — and says `asked`. Esc is two-press when dirty; emptying an existing note is
two-press and deletes. `Scope::Editor` is a barrier like `Input`; `EDITOR` in `keymap.rs` is
the table.

On the ticket page the description renders under the identity line as rich text, capped at
`min(8, body/3)` rows with `rich::render`'s own `~`, and eats rows from the zones below, never
the footer; nothing moves when there is none. Notes are rows in the rail under the sessions
(`RailRow::Session | Note`, sessions first — the invariant `board_enter` and the focus return
lean on), `≡ <name>  <you|claude> <age>`, the description included so a long one can be paged;
a note row renders whole in the PREVIEW zone through the same `window` as a reply, keyed to
`(id, rev)`. `App::poll_notes` fetches the description and the selected note once per `(id,
rev)` from `tick`, edge-triggered; a failed read retries after 2 s; the cache holds 64.
`Ctx::sel_note` is what keeps every `sel_*` session fact false on a note row. Ticket `n` sits
at prio 95 — after `d`, before `q` — so at 120 columns the pop yields, never the destructive
key. Known gaps: no soft-wrap; last write wins between a person and an agent on one note; the
description block is capped, the editor is where it is read whole. E2e:
`crates/mesimon/tests/notes_e2e.rs`.

## Shift+Enter on an empty seat starts claude on the title (2026-09-03, user request)

A ticket saved with plain Enter was one press behind one saved with Shift+Enter, and there was no
way to take that press later: on the board Shift+Enter was gated on `ticket_promptable` (a live
claude PANE), so a ticket with no claude offered nothing on the key, and the only road to "start
claude on the title, submitted" was to have chosen it in the composer.

What holds now:

- The board's `ShiftEnter` binding is still the ONE `Verb::Prompt` binding (an atom appears once
  per scope), and its `avail` is `ticket_promptable || (has_ticket && !ticket_has_claude)`, under
  `rich_keys` as before. The hint switches on `Ctx::ticket_has_claude`: `ask claude` over a pane,
  `ask claude the title` over an empty seat.
- `App::dispatch` routes `Verb::Prompt` on the same flag: a claude present opens the one-line field
  as before; none present calls `start_composed(ticket)` — the composer's own second half — which
  sends `SpawnSession { submit_prompt: true }`, stays on the board, attaches nothing, and arms no
  fresh-ticket Enter window. The worktree `Provisioning` reply replays the parked spawn, submit
  flag included, exactly as it does for the composer.
- A `Sleeping` claude is NOT an empty seat: it holds the one-claude seat and has a conversation the
  title would repeat into, so the key stays inert there and `c` remains the wake. A shell on the
  ticket leaves the seat empty (`ticket_has_claude` counts claude only), so the press still starts
  one beside it.
- `shift_enter_asks_claude_at_every_stage` is unchanged: this is not a fourth home for the atom,
  it is the first stage reached from the board instead of from the composer. Pinned by
  `keymap::shift_enter_on_an_empty_seat_starts_claude_on_the_title` and
  `app::shift_enter_on_a_ticket_without_claude_starts_it_on_the_title`.
- No golden moves: the board goldens render without `rich_keys`, where the key is unhinted.

## An agent can file a ticket (2026-09-03, user request)

The tier had no way to record work an agent found and was not asked for: it could do it (scope
creep on somebody's ticket), drop it, or mention it in a reply that scrolls away. `write_note`
put it on the agent's OWN ticket, which is the wrong card — the finding is a new unit of work.

What holds now:

- **A sixth tool, `create_ticket`** (`core/src/mcp.rs`): `title` required; `column` optional and
  a plain string validated server-side (absent means the board's FIRST column, where a human's new
  ticket lands too); `description` optional, saved as the ticket's first note; `idempotency_key`
  optional, defaulted by the shim to the client's `toolUseId` exactly as `move_ticket`'s is.
  Description text passes the lint and the 820-byte cap.
- **`Command::AgentCreateTicket`** → `Daemon::agent_create_ticket`: the same `mint_ticket` and
  `sanitize_title` the composer's `CreateTicket` uses, refused under the same columns bar (a
  `next_key` that cannot persist regresses into an existing ticket's directory on the next start),
  and the description written through `write_note`, so the note is authored `agent:<uuid>` like
  every note an agent writes. `persist_and_notify` before the note so a failed note still leaves
  a consistent, visible ticket, and the receipt then says both ("ticket T-9 created, but its
  description was not: …").
- **Authorized as `Mutate` on `Resource::Column`, not `Board`.** `authorize` keeps denying an agent
  `Mutate` on `Board`; appending one card to a column changes no registry, no column list and no
  other card, which is the line that rule draws. The doc comment on `authorize` says so now.
- **The receipt is a KEY, never an id** (`Response::AgentCreated { key, column, board_version,
  replayed }`), and no agent command accepts a key or an id back — the caller's session stays
  bound to its own ticket, the new card has no session, and no tool can give it one. Deleting,
  renaming or archiving what was filed stays in the never-tier: nothing an agent files can be
  unfiled by an agent.
- **The replay map grew a shape**: `agent_replay` holds `AgentReplay::{Moved{column},
  Created{key, column}}` so a `move_ticket` retry is never answered with a `create_ticket`
  receipt that shared a client-minted id. Same 512-entry bound, same clear-on-overflow.
- Feed line `create_ticket`, actor the agent, subject the new ticket. The local arm's refusal of
  agent commands from a `Local` principal lists it too.
- Pinned by `mcp::exactly_six_tools`, `create_ticket_parses_and_refuses`,
  `the_tier_is_exactly_six_commands`, the shim's `a_create_result_names_the_new_key`, and
  `mcp_e2e` (default column, trimmed title, note authored by the agent, no session on the new
  card, the caller's binding unmoved, a `toolUseId` retry replayed rather than minted twice, a
  named column honoured, an unknown column refused legibly). README says six tools.

## An agent sees tags, and files a ticket under them (2026-09-03, user request)

`create_ticket` shipped with a title, a column and a description, and a card an agent filed
landed untagged on a board whose whole triage vocabulary is tags — the human had to open every
agent-filed card and `^t` it. `get_ticket` said nothing about tags either, so an agent could not
even tell what its own ticket was filed under.

What holds now:

- **`get_ticket` carries `tags` and `allowed_tags`** (`AgentTicketView`, two `Vec<AgentTagView
  {name, group}>` fields): the tags the ticket wears in group order, and the board's whole
  registry. `allowed_tags` is `allowed_columns`' twin — the vocabulary travels as transient
  result data, never as a schema enum that would put the user's words into every request.
  `AgentTagView` is hand-written, not `TagRef` reused, for the same reason the view is not
  `Ticket`: a projection that is its own type cannot grow a field when the model does. No colour:
  a tint is how a card paints a tag, not what it means.
- **`create_ticket` takes `tags`, an array of NAMES.** `parse_tool_call` accepts absent/null as
  none and refuses any other shape legibly (`tags must be an array of strings`), so `"tags":
  "BUG"` is an answer rather than a silently bare card. `Command::AgentCreateTicket.tags` is
  `#[serde(default)]`.
- **Names resolve against the registry, and the registry is never touched**
  (`Daemon::resolve_agent_tags`): exact spelling first, then a unique case-insensitive match
  (models read `BUG` off `allowed_tags` and send back `bug`); a name the board does not know is
  refused and the refusal names `allowed_tags`; a name on more than one group is refused rather
  than guessed; two names on one group are refused (`one tag per group`) because picking the
  survivor would be inventing the agent's intent; a repeat is one tag. `RegisterTag` stays
  never-tier — the tool puts a card under a word somebody already chose, it does not coin one.
- **Resolution runs BEFORE the mint**, so a refusal leaves no half-filed card; the tags are set
  on the ticket and its file saved before `persist_and_notify` broadcasts, so the first frame a
  TUI draws already has the bar painted.
- `create_ticket`'s text was trimmed to stay under `MAX_TOOL_BYTES` (916 → 811 bytes with the
  new property); the sentences it lost were restatements of `get_ticket`'s.
- Pinned by `create_ticket_parses_and_refuses` (shape cases), `every_tool_fits_the_budget`,
  and `mcp_e2e` (empty `tags`/`allowed_tags` on a bare board; a `SetTag` and two `RegisterTag`s
  by a person then visible to the agent with groups; a create wearing `["bug", "P1", "BUG"]`
  landing as `BUG@1, P1@2` with the registry still three entries; unknown name, same-group
  clash and a wrongly shaped argument each refused with nothing minted).
## An agent wears the user's tags (2026-09-03, T-164, user request)

The tier denied every tag command, and `agent_allows` said why: the six human commands are
registry writes (five of them outright, and `SetTag` because using a name is what creates it), and
"tagging is a human curation act". The user asked for a tool that helps without interfering, and
the line that satisfies both is the one `authorize` already draws for columns: an agent may change
its own card, never the board's vocabulary.

What holds now:

- **A seventh tool, `tag_ticket`** (`core/src/mcp.rs`): `name` required; `group` optional (1-10,
  refused outside that range rather than widened to "any"); `remove` optional boolean. It wears
  one of the board's EXISTING tags on the caller's ticket or takes it off. No idempotency key:
  wearing what is worn and removing what is absent both succeed and change nothing, so a retry
  after a dropped connection is already safe.
- **The registry is read and never written** (`Daemon::agent_tag_ticket`). A name the picker never
  saw is refused — "no such tag: … (get_ticket lists the board's tags as allowed_tags)" — and on an empty registry
  the refusal says where tags come from. What is stored is the registry's OWN spelling: a name in
  another case that resolves to exactly one entry lands as the user spelled it, so `sanitize_tag`
  has nothing to do and `columns.toml` is never touched (no write bar applies). This is the whole
  of "not interfere": ten agents on ten tickets still speak the user's one language, no axis fills
  with words nobody chose, nothing lands on an axis whose meaning the agent cannot know, and the
  picker never grows a row the user did not type. An agent that wants a word the board lacks has
  `write_note`.
- **The names it accepts are `get_ticket`'s `allowed_tags`** — the field the block above added
  for `create_ticket`, now serving both — and the two tools share one registry lookup,
  `Daemon::lookup_agent_tag` (exact, then unique case-insensitive, optionally narrowed to an
  axis), so they cannot drift on what a name means. The receipt's `tags` are `AgentTagView`s
  too. Landed by rebase onto the block above: the branch had minted its own `available_tags`
  and reused `TagRef`; main's names won.
- **A name on two axes is refused until `group` says which** ("tag … is on more than one group (3, 4);
  pass group to say which"),
  never guessed. One per group is what lets a digit address an axis, so the groupmate comes off and
  the receipt names it: `Response::AgentTagged { tags, replaced, board_version }` is the ticket's
  whole tag list after the call, so the model sees what it did without a second round-trip.
- **Authorized as `Mutate` on `Resource::Ticket`** — one card's own metadata, like a note; the
  board itself untouched, so `authorize` needed no change. Feed line `tag_ticket`, actor the agent,
  only when something changed. The human's six tag commands stay in the never-tier
  (`the_never_tier_holds` now lists `MoveTag` too, which it had missed), and the wire test hands
  `SetTag` and `RegisterTag` to an agent principal and watches them refused.
- Pinned by `mcp::exactly_seven_tools`, `tag_ticket_parses_and_refuses`,
  `the_tier_is_exactly_seven_commands`, the shim's `a_tag_result_names_what_is_worn_and_what_came_off`,
  and `mcp_e2e` (empty registry refused legibly, wear, replace-with-receipt, case resolved to the
  registry's spelling, ambiguity refused then resolved by `group`, an invented name refused and the
  registry unchanged, remove twice, the registry identical before and after). README says seven
  tools.

## The composer's editor is a panel over the board, and it grows out of its card (2026-09-03)

The note editor shipped 2026-09-02 as one full-screen surface for both purposes, and composing
in it lost the board: Tab from the one-line composer dropped the user into a page that shared
nothing with where they had just been, and nothing on it said which column the ticket would
land in. The author asked for three things — not full screen but on top of the board, a
transition from the mini composer to the big one, and the column named.

**The compose purpose draws as a PANEL over the cards** (`editor::draw_panel`, routed by
`ui/mod.rs` when the editor is composing AND the screen under it is the board). At rest it
covers the card rows of the columns zone edge to edge: the header, the column headers and their
breathing row stay above it, the advisory row and the footer under it — and the footer already
speaks for `Scope::Editor`, so the panel draws none. Edge to edge, not a centred box, because a
narrower panel left slivers of cut cards on both sides (a bar cell here, `>1y` there) and a
sliver is drawn structure by another name. The panel's text stands three cells in, where a
card's text stands (`LPAD` + bar + pad) and where the column headers' names do, so the title
row is on the same vertical as the column name above it. **The note purpose is untouched**: it
still takes the whole screen under the breadcrumb, from whichever screen it was opened on.

**The context line names the column**: `NEW TICKET ∙ TODO column ∙ ⎇ worktree ∙ <tags>`. The
column is `App::cursor_col`, the same one `mint_ticket` sends, read at draw time — moving the
cursor while composing is impossible, so there is nothing to keep in step. "TODO column" and
not "in TODO" because "in IN PROGRESS" reads as a stutter.

**The panel grows out of the phantom card.** The board draw records the phantom card's
on-screen rectangle in `App::compose_card` (draw-side, the way `preview_view` is: where a card
landed is a fact of the frame — and only a whole card counts, a card cut by the window's edge
would put the origin off screen), and `Verb::Describe` copies it into `Editor::grow` with the
instant. For `app::GROW` (180 ms, eased out) `draw_panel` interpolates every edge between that
rectangle and the resting one, `Clear` + ground on the interim rectangle, the title and context
inside it and the body once the rows exist — so frame zero is the card itself with its title in
the same cells, and the board stays visible around the panel while it is small. `tick`'s poll
timeout is the frame rate (the spinner's 100 ms), and `App::animating` shortens it to 16 ms for
exactly those frames. The recorded origin is widened left by `LPAD` so the panel's own indent
puts the title where the card's was, without a one-cell jump. `grow` is `None` on a note, on a
test-built editor and once settled, so every golden renders the resting panel.

This is the second bend in D19's motion ban after the working spinner, on a different clause:
the spinner moves because something is moving; this moves ONCE, on a gesture, for a fifth of a
second, never loops, and exists so the eye is carried from the composer to its bigger room
instead of being dropped into it. It is not an animation vocabulary — a third motion needs its
own argument, not this one. Pinned by `the_composer_panel_grows_out_of_its_card` (origin
recorded, carried by Tab, frame zero on the card's row with the board showing through, settled
panel with the column headers above and the column named) and the two compose goldens.

## The composer's editor is a dialog over the board, snapped to whole columns (2026-09-03, user request)

The panel above lasted a day. "Not full screen but on top of the board" had been met with a
panel that took the columns zone edge to edge, and the author asked for what they had meant: a
DIALOG on top of the board, and one that is "beautiful and structured nicely". The block above
is superseded in its geometry and stands in everything else (the routing in `ui/mod.rs`, the
context row naming the column, `Editor::grow` and its argument under D19).

**The dialog IS the cursor card, grown** (`editor::draw_dialog`). Its anatomy is a card's, not a
popup's: the frame is `[bar 1][pad 1][content][pad 1]`, the surface is `selected_bg` with the
`sel` ink (the page ground with the `rest` ink where the profile paints no cursor surface — a
whole dialog in reverse video would be a fourth SGR-7 use), the accent bar runs down its whole
left edge wearing the composer's picked tags exactly as `render_edit` paints the phantom card's
(`tags::bar_cell`, and `tags::stack_full` splits a tall stripe 70/30 for two tags, so a tagged
composer is the one place the stack is ever twenty rows tall), the title is the first row in
bold and the context row sits DIRECTLY under it where a card's meta rows do — no blank between,
which is what lets frame zero of the grow be the card itself, row for row. The head is three
rows (title, context, breathing) against the full-screen note editor's four; one breathing row
closes the bottom. No heading row of its own: a card does not announce itself, and "NEW TICKET"
on the context row already says what the dialog is. The board's footer still speaks for it.

**Its edges fall on the board's gutters.** A centred 80-cell box was built first and the golden
showed the DONE column cut to `ed accent bar  >1y` — the very sliver the panel block had refused
a narrower panel over, and a painted surface does not make a cut card whole. So `dialog_rect`
re-derives the geometry the board just drew (`layout::board_geometry` on `App::col_window`, read
and not written back) and `snap_to_columns` picks a RUN of whole expanded columns: at least
`DIALOG_MIN_W` (two columns at `MIN_COL` plus the gutter, 53), the runs within `DIALOG_MAX_W`
(96) before any wider one, then the run whose centre is nearest the zone's, then the narrowest.
At 120 columns with four columns that is the two middle ones (x 31, 59 wide); the outer two show
their cards complete on both sides, which is what makes the board around it read as the board.
Five columns at `MAX_COL` would centre on three (122 wide) and the cap hands it two instead:
prose in a 120-cell line is no kindness. Every column when no run reaches the floor (a
one-column board), and a centred box only in the no-columns case that cannot compose anyway.
Vertically it keeps the panel's inset: two rows under the column headers, so its top edge is the
cards' top edge, two rows above the advisory row.

**The recorded origin is the card's own rectangle now**, bar cell included (`ui/board.rs` no
longer widens it left by `LPAD`): the dialog's lead is the card's, so at frame zero its bar and
its title stand in the card's cells and nothing jumps. Pinned by
`the_composer_dialog_grows_out_of_its_card` (origin at the column's bar cell; frame zero is the
title row with the context row under it and the other columns showing through; settled at the
second column's x with both covered columns' cards gone and both outer columns' cards whole) and
the two compose goldens, which now show whole cards on either side of the dialog.

## A note survives delete + undo (2026-09-03, dogfood)

**Refuted:** the grace band (D21) carried the ticket and its sessions in memory and nothing
else, on the assumption that a ticket was its `ticket.toml`. Since 2026-09-02 a ticket's notes
are FILES under its directory, and `delete_ticket` removes that directory eagerly (deliberately:
a crash inside the band must not resurrect a deleted ticket on the next load). `restore_ticket`
wrote `ticket.toml` back — with the `[[notes]]` list — and nothing wrote the bodies, so the
restored ticket listed notes whose files were gone. The editor then had exactly one thing to
say, "note file missing", and no road out: it opens by re-reading the daemon, so an orphan can
neither be edited nor blanked-and-deleted. Seen on T-71 within 25 s of the feature being used
(feed: `write_note`, `delete_ticket`, `restore_ticket`).

**Built:** `GraceEntry.notes: Vec<(Ulid, String)>` — `delete_ticket` reads every listed body
BEFORE `delete_ticket_dir`, and `restore_ticket` writes them back with `save_note` before it
saves the metadata, the same body-then-list order `write_note` keeps. A note whose body could
not be read at delete time is dropped from the restored list rather than restored as the orphan
this block is about. The eager directory removal stays: memory, not a trash directory, so a
daemon restart inside the band still means gone. The tui e2e (`m1_acceptance_headless`) writes
a note before the delete and reads it after the undo. Deferred: a repair road for an orphan
that already exists (the editor could open it empty so a save re-creates the body) — the one
on the author's board was healed by hand through `WriteNote`.

## Shift+Enter in the editor is a newline (2026-09-03, user request)

**Refuted:** the editor the composer grows into carried the one-line composer's Shift+Enter
("save + ask claude") while composing, argued as the same sentence in a bigger room rather than a
fourth home (the note editor left the atom unbound). In use it was the wrong reflex: the editor
is a BODY, and every chat-shaped box the user types into — claude's own included — has taught the
finger that Shift+Enter breaks a line. The press that wanted a blank line in a description minted
the ticket and started an agent; in a note it did nothing at all.

**Built:** `Scope::Editor` binds `Key::ShiftEnter` to `Verb::EditorNewline`, the same verb as
`Enter` — in the body a newline, in the composer's title the way down to the body — for the
description and a note alike, unhinted (`enter` beside it already teaches the verb), `prio: 0`.
Gated on `rich_keys` like every ShiftEnter binding, and this is the one place the legacy-floor
degradation is exact: a terminal that cannot spell the atom sends `Enter`, which is the same verb.
`App::editor_save` lost its `start` flag (the editor never asks now), so the composing road is `^s`
to mint and then the board's Shift+Enter on the minted card, which starts claude on the title (the
"empty seat" block above) — the sentence keeps its three homes and nothing is lost.
`shift_enter_asks_claude_at_every_stage` now asserts the editor resolves to the newline, and
`shift_enter_in_the_editor_is_a_newline` (app tests) types through it in both editors. Golden
`editor_compose_120x30` drops the hint from the footer.

## The UI overhaul: framed dialogs, painted footer, one place per hint (2026-09-03, T-158)

The author asked for a UI overhaul with Bagels (github.com/EnhancedJax/Bagels) as the reference,
naming five things: the dialogs, titles "not indicated correctly", hints that were not intuitive
and sat far from what they operate, the ticket page's title and subtitle, and a more modern
layout. Decided in plan-mode Q&A: **boxes for dialogs only** (board, cards and ticket page stay
painted), the whole thing in one ticket. Recorded here because each of these was a law or an
author decision before.

**L1 gains its one allowlisted role: a dialog's frame.** `ui/dialog.rs` is the primitive every
floating surface draws through — `frame()` clears, paints, draws `╭─ TITLE ───╮ … ╰─ keys ─╯`
(ascii tier `+ - |`, `glyphs::frame_set`) in `dim3`, records the rectangle on `App::frames`
(draw-side, `RefCell<Vec<Rect>>`, cleared at the top of `ui::draw`) and returns the inner rect.
`test_no_drawn_structure` no longer sweeps strings: it renders each screen to a `Buffer`, reads
the frames that draw reported, and admits a codepoint in `0x2500..=0x259F` only on a recorded
perimeter (or as `▀`, the 2026-09-01 exception). User text is already stripped of the range by
`scrub_cells`, so a frame glyph can only be chrome's, and the test asserts the sweep covered at
least five frames so it cannot pass by covering none. The doc's L1 row (06 §0) is amended in
place. Frame in `dim3`, title `dim1` + bold, one width for every centred dialog
(`dialog::MAX_W` 64 inner; the menu, picker, archived list and drawer were 54/62/64).

**Every dialog names itself in its top edge and teaches itself in its bottom edge**, and the
in-body title row and in-body hint row are gone: `MENU`, `THEME ∙ for a dark terminal`,
`ARCHIVED ∙ 3`, `EXTERNAL ∙ 2`, `KEYS ∙ board`, `TAGS ∙ <ticket>`, `NEW TICKET`. The bottom edge
is the LEFT cluster of the dialog scope's footer (`dialog::keys` → `keymap::footer_split`), so
the keys are still the keymap's. The tag picker stays a full-width bottom sheet, framed, and now
sits one row ABOVE the footer instead of under a footer redraw. The composer dialog's frame
stands one cell outside the snapped column run on every side — the gutters and the two
breathing rows `DIALOG_INSET_Y` kept clear — so the stripe still sits on the column's own bar
cell and the cards beside it stay whole; below three rows `frame()` draws nothing and hands the
area back, which is what keeps frame zero of the grow the card itself. Framed, the context row
starts at the column (`TODO column ∙ ⎇ shared ∙ tags`) because the edge says NEW TICKET;
unframed it says it itself. The `?` overlay goes to TWO columns of groups when one would not fit
the terminal's height (the APP group fell off a 30-row terminal without a word) and marks a cut
with `~`.

**The header opens with a chip naming the screen** — `BOARD`, `TICKET`, `DIFF`, `NOTE` — on the
elevated surface, bold, before the breadcrumb (`chrome::draw_header`, one function for every
screen now; ticket, diff and the note editor stopped drawing their own row 0). The breadcrumb
ends at the repo on the ticket page; the diff and the note editor carry the ticket as a leaf
(context, not subject). The board's count and offer show only under the BOARD chip. The footer's
mode word is gone AT REST — the chip already says where you are — and appears only when a mode
has taken the keys over (`DELETE`, `TAG`, `NEW`, `ASK`, `MENU`…), which is also when the hints
beside it changed.

**The footer is a painted band on `selected_bg` with `key word` pairs** (key `base` + bold — 06
§5.1 clause 3 always sanctioned bold on the footer's key names and the code never used it — word
`dim2`, `∙` `dim3`; `chrome::hint_spans` is the one builder, shared with the frame edges and the
rail trailers). `keymap::footer_split` partitions a scope's items into the screen's own keys
(left, prio order, skip-not-cut as before) and `Group::App` (right cluster, reserved first):
`esc menu` (Board, prio 254 — back in the footer, repaying the cost the 2026-09-01 footer pass
recorded, "nothing on the board says esc") and `? keys` (Global, prio 255). **`? keys` is
emitted only where `?` resolves**: the tail was a literal before and was promised inside text
fields, where `?` types a question mark, and in chord tails, where it cancels.
`footer_always_keeps_the_help_tail` now asserts the tail iff `resolve(scope, '?')` is `Help`,
over every scope. **A hint lives in one place**: under an open dialog the footer carries only the
mode chip and the right cluster (`chrome::dialog_open`).

**The ticket page has a title row and a state row.** Row 0 the header, row 2 the title (bold
`base`, the page's own headline — it lived only in the breadcrumb; `r` edits it there), row 3
the state line: `IN PROGRESS ∙ 3d here ∙ created 2w ago ∙ tags ∙ ⎇ branch ∙ merge state ∙ m
merge`. "here" is `Ticket::column_since`, the card's own age; "created by you" went (single-user
v0.1 says nothing by it). Body zones move down one row. The sessions' keys moved off the footer
and under the rail: a `dim3` trailer row `c claude ∙ s shell ∙ x sleep` under the sessions
(the empty rail's nudge, generalised; wrapped onto a second row when the rail is narrow, never
dropped) and `N new note` under the notes; `{ } page` sits on the PREVIEW heading's right while
the zone overflows (read from the last frame's measurement). Board `o` reads `new ticket`, not
`open ticket` — it mints a card. Ticket `c s x { }` carry prio 0 now.

**The note editor names its ticket**: header `NOTE  mesimon > repo > <ticket>`, and the context
row says which note this is — `the description ∙ edited by you 3d ago`, `a note ∙ …`, `new note ∙
becomes the description` — instead of `NOTE ∙ edited by you`.

Not done, deliberately: the header is NOT a painted band (the plan said header + footer; a
band on row 0 two rows above the cursor column's painted header band fought it, so the header
keeps the page ground and only its chip is painted); no accent-coloured focus frame (L2/L3 hold:
`attn` stays needs-you only); no per-panel hue. Every golden changed (header chip, footer);
regenerated once and reviewed by eye.

## A worktree name is a handful of words (2026-09-03, dogfood)

Doc 12 §12.5.2's slugger capped the slug at 60 bytes and cut on a byte, so a sentence-shaped
title (the author's are) produced `T-164-mcp-tool-to-modify-tags-think-how-to-best-design-that-
tool-t` as a directory AND a branch — 66 characters of which the last is half a word, in every
`git branch`, on the ticket page's state row, and in the merge flow's messages. The key already
makes the name unique; the slug only has to say which ticket. `workspace::SLUG_MAX_BYTES` is 32
now, and the cut lands on the last `-` at or before the cap (`SLUG_MIN_WORD_CUT` 8: a title that
opens with one long token still cuts mid-word rather than keeping two letters), so the same
title yields `T-164-mcp-tool-to-modify-tags-think`. D28's "changing the slugger produces a lying
board" does not bite: a `Binding` persists its `path` and `branch` at provision time, so every
existing worktree keeps its name and only tickets provisioned after this build get the short
one. Tests in `workspace.rs` pin the cap, the word cut, the fallback, and the no-trailing-`-`/`.`
rule at the new cut.

## The state row's tag bullet belongs to its chips (2026-09-03, dogfood, T-167)

T-163's ticket page read `REVIEW ∙ 7m here ∙ created 19m ago ∙ ∙ ⎇ msmn/T-163-…`: a bullet
with nothing behind it. The tag clause budgeted its chips against the width MINUS the whole
worktree clause (which draws after it), and a 60-byte slug plus the merge state left no room
for ` IMPROVEMENT ` — but the ` ∙` separator was pushed before the first chip was tried, so
the tag vanished and its bullet stayed. The branch was never budgeted either, so the row ran
off the right edge and the terminal clipped the name.

Now (`ui/ticket.rs`): the chips are built aside and the separator goes in only with a chip
behind it; the chips have first claim on the row, with the merge state and detail reserved in
full and the branch name reserved to `WT_BRANCH_FLOOR` (16 cells, ` ∙ ⎇ msmn/T-163~`), so a
ticket wearing ten tags still says where it lives; and the branch name is cut with
`text::truncate`'s `~` marker to the room the row leaves, never below the floor, instead of
the line being clipped. `the_state_row_never_shows_an_empty_tag_bullet` renders the case at
four widths. The short slugger (above) shrinks future branches; this is what fixes the row for
the bindings that already hold a long one.

## The ticket page's header section is a band (2026-09-03, T-158 follow-up)

The author asked for the description area to be "more distinctive — maybe the ticket title +
description should be with a different background to indicate header section". Rows 1 through
the row under the description (breathing row, title, state line, breathing row, description, a
bottom pad) are painted `selected_bg` edge to edge and read in the `sel` ramp; the chip row
above and the zones below stay on the page ground with one breathing row between. The body
zones move down one row for the pad (`body_y = 6 + extra`), and the description cap follows.
**Markdown on the band inverts its paint**: `rich::render_on(…, Surface::Elevated)` reads the
`sel` ramp and paints a code span or slab in the PAGE ground (`Paint::of`), because the only
other surface is the one paint there is and a slab in the band's own colour vanished. `render`
is the `Ground` form and every other caller is unchanged. Where the profile paints no elevation
(mono, light-256) the section keeps its rows on the ground in `rest`. Pinned by
`test_ticket_header_section_is_a_band` (edges painted, header row and zones not, a code span on
the page ground, its bullet on the band).

**One band, and the description is the card's body inside it (same day, three passes).** The
author asked for "a clear separation between title + subtitle and the description"; a second band
was refused ("not good"), the description on the ground with a tag-tinted bar was refused too
("keep the title section background also for the description; use neutral color for the line;
no extra line separation, only one"). What holds: ONE band — pad, title, state line, one blank
row, the description, pad — and the description sits inside it in the card's own frame, `[pad
1][bar 1][pad 1][text]`, with the NEUTRAL cursor-weight bar down its left edge (`theme.bar
(BarWeight::Cursor)`, no tag tints — the state line already names the tags). The description
reads in the `sel` ramp with code sunk to the page ground (`rich::render_on(…,
Surface::Elevated)`, `Paint::of`), because a slab in the band's own colour vanished. Where the
profile paints no elevation the section keeps its rows on the ground in `rest`. Pinned by
`test_ticket_header_section_is_a_band` (every band row painted, row 0 / the breathing row / the
zones on the ground, exactly one blank between the state line and the body, the bar cell on every
description row and none on the blank, text at column 3, a code span in the page ground).
## `Tab` on a card opens its description in the composer's dialog, and the attention walk is gone (2026-09-03, T-163, user request)

`Tab` on the board was `needs you` — jump to the next card waiting on you, `Shift+Tab` the
previous — one hint in the footer, and never once pressed in dogfood: the waiting card already
shouts (the inverted title row, the `!` in the spine and the column header, the header's own
count) and the cursor goes to it by `hjkl`. The author asked for the key to open the ticket's
DESCRIPTION the way the composer's `Tab` grows into the editor, and for the attention walk to go.
`Verb::NextAttention` / `PrevAttention`, `Ctx::any_attention` and `App::cycle_attention` are
deleted, not parked; `attention_queue` keeps its other callers.

**The board's `tab` is `Verb::Describe`** — the composer's own verb, so `tab` is one verb across
the two scopes that bind it — gated on `has_ticket`, hinted `describe` at prio 65 (the room the
walk freed; at 120 columns it lands between `move card` and `rename`). `App::dispatch` opens the
description (`notes[0]`, or the fresh note that becomes it) through `open_note_editor`, which is
what `n` already did on the board; `n` keeps the note axis in the overlay and its board hint is
now the one word `note`, so the overlay does not list two keys under `describe`. The ticket page
and the diff keep `tab` inert — `n` is the description's key where there is no card to grow — and
`Shift+Tab` on a screen is bound nowhere now, which resolves what was a `BackTab` split between
the walk and the composer's workspace toggle.

**The editor's surface is the screen's, not the purpose's.** `ui/mod.rs` drew the dialog only
for a COMPOSING editor and gave every note the full screen; now every editor over the board is
the dialog and every editor from the ticket page takes the screen. So `n`/`N` on the board are
dialogs too — two keys to one document on two surfaces would have been a third place to learn.
`open_note_editor` sets `Editor::grow` from the cursor card when the screen is the board, and the
board records the CURSOR card's rectangle every frame (`App::cursor_card`, renamed from
`compose_card`: the composer's phantom card IS the cursor card, since a text field drops the
selection, so one field serves both and the phantom path lost nothing). The description dialog
therefore grows out of the card that was under the cursor, exactly as the composer's grows out of
its phantom card — `the_description_dialog_grows_out_of_the_card` pins the origin, frame zero
(the title in the card's cells, the board showing through) and the settled dialog. On the dialog
the stripe wears the TICKET's own tags (`tags::painted` on `t.tags`, where the composer's wears
the picks), so frame zero is the card, stripe and all.

**The dialog's frame names the note, and the context row names the workspace.** Rebased over
T-158's framed dialogs: the frame's top edge carries the heading — `NEW TICKET` composing,
`DESCRIPTION` / `NOTE` / `NEW DESCRIPTION` / `NEW NOTE` on a ticket that exists
(`editor::heading`) — and the context row under the title carries what rides along: composing,
the column, the workspace and the tags as before; on a note, `⎇ shared|worktree|adopt` then
`edited by you >1y ago`. Where there is no edge to say it (frame zero of the grow, the full-screen
editor under its NOTE chip) the row leads with the heading itself, the same rule the composer's
row already followed. It was `the description ∙ edited by …` with no workspace, and a
two-column dialog cut `∙ ⎇ workt` the moment the workspace joined the end of it. The full-screen
`editor_note_*` goldens moved with the row and now pin the ticket-page surface explicitly with
`Screen::Ticket`; `editor_describe_120x30` is the dialog. The body's empty-state word is
`describe it` whenever the text will be the description (`body_hint`), not only when composing.

**`Shift+Tab` in the description editor is the composer's workspace pick, a press late.** The
author asked for the "big composer" on a ticket to switch between the shared checkout and its own
worktree "if no claude created yet". The EDITOR scope's `BackTab` binding is one binding (an atom
may not appear twice in a scope) whose `avail` is `editor_composing || workspace_open`, where
`Ctx::workspace_open` is the daemon's `set_workspace` lock mirrored for the editor's ticket — no
session on it and no worktree binding (`wt_item` is `None`); the lock is any session, not only a
claude, because a shell in a worktree is just as relocated. Composing, the pick rides with the
draft as before; on a ticket that exists the key sends `Command::SetWorkspace` at once — a
workspace is the ticket's, not the note's, so it is not held for `^s` and leaves the editor
clean — toggling `Some(Worktree)` ↔ `None` exactly as the composer does, and the daemon's refusal
lands in the status. The binding is `mutates: true` now, since it is. `mesimon doctor` and the
wire are untouched: `SetWorkspace` existed for the composer's mint, and the `FakeTransport` grew
an arm for it so `shift_tab_in_the_description_sets_the_workspace_until_work_starts` can see the
toggle round-trip and the lock hold.

**Footer fallout, accepted.** `tab describe` takes the room the walk freed in the board's footer
(at 120 columns it stands where `r rename` did on a card with an agent; `r` is still hinted on the
others and in the overlay). The board's help overlay lost two Navigate rows and gained one Ticket
row; `help_ticket` lost the two walk rows and nothing else.

## An approved plan is the agent's note on the ticket (2026-09-03, user request)

Plan mode writes its plan to `~/.claude/plans/<slug>.md`, off the board, and the ticket learned
nothing of it: the card said `plan ready for review`, the user approved it in the pane, and the
document the rest of the work would follow lived in a directory nothing on the board could reach.
The `PostToolUse` frame `ExitPlanMode` fires carries the whole plan in `tool_input.plan`
(captured on 2.1.251 through 2.1.258; the response is Claude's "User has approved your plan…
saved to: …" sentence and is not read), and since `PostToolUse` fires only once a tool RETURNS
— a rejected plan is a tool error and fires nothing mesimon hooks — that frame IS the approval.
The broad PostToolUse observer was already registered (the permission-accept clear path), so no
hook entry moved.

What holds now:

- `ingest::plan_of` reads the plan off a `PostToolUse` frame whose `tool_name` is
  `ExitPlanMode` and whose `agent_id` is unset (a subagent's plan is not the session's), blank
  or truncated payloads read as no plan, and `on_hook` hands it to `Daemon::record_plan`.
- `record_plan` writes it through the SAME `write_note` the agent's tool uses — sanitized,
  capped at 32 KiB, stamped `agent:<uuid>`, authorized as the agent on `Resource::Ticket` — and
  the feed line is `plan_note` with actor `agent`, never the text. The daemon does the writing on
  the agent's behalf; the agent asked for nothing and its context receives nothing (promise 3).
- **One plan note per session, replaced on every approval**, the way the plan file itself is
  replaced on a re-plan: `SessionRecord.plan_note` (`#[serde(default)]`, persisted, so a daemon
  restart cannot turn the next re-plan into a second note) names the note, a re-plan bumps its
  `rev` and renames it after the new first line, and a note the user has deleted since is not
  resurrected under its old id — the next approval mints a fresh one.
- On a ticket with no description the plan becomes `notes[0]` and so IS the description — that
  is what the first note on any ticket is (the notes block above), and most tickets are filed as a
  title alone. The ticket page then shows the plan's head under the identity line and `n` edits
  it; a re-plan overwrites that edit, the known last-write-wins gap. No separate slot was made:
  `notes[0]` is the description by position, and a plan that lands on a ticket with a description
  is an ordinary second note in the rail.
- Adopted (observe-only) sessions get nothing: the transcript tail sees the `ExitPlanMode` call
  before the approval and never the approval itself. Hooks are the only road.

E2e: `notes_e2e::an_approved_plan_is_the_agents_note_on_the_ticket` (the real hook binary, the
three frames before and at approval, a re-plan, a delete-then-re-plan, and the feed). Unit:
`ingest::an_approved_plan_is_read_off_post_tool_use_only`.

## The e2e seams are process environment, so one harness at a time (2026-09-03, found by the alpha.11 gate)

`ci/release.sh` failed `notes_e2e` on the first alpha.11 attempt, and a loop reproduced it in
~40% of runs under `cargo test` while `cargo nextest` (the inner loop) never saw it once.

- The seams (`MESIMON_CLAUDE_BIN`, `MESIMON_HOOK_BIN`, `MESIMON_CLAUDE_HOME`) are variables of the
  TEST PROCESS, and `cargo test` runs a binary's tests as threads of one process; nextest runs one
  process per test. `notes_e2e` was the first file to boot two harnesses with two DIFFERENT stubs
  (a read loop and an `exec sleep 60`), and its two daemons raced on the variable: the notes test
  spawned the plan test's stub (which never reads the paste — "timed out waiting for the note
  sentence"), or a stub path the plan test's teardown had already swept (pane died at once — "no
  live claude session"). Every other e2e file boots one harness, which is why it never bit.
- `Harness` now holds a process-wide `SEAMS` mutex for its whole life, released after the
  teardown (last field, drops last; a poisoned lock is taken over, not propagated). Harnesses in
  one binary run one after another; the suite's parallelism is nextest's, across processes, and
  is untouched.
- The daemon still reads `MESIMON_CLAUDE_BIN` at every spawn, on purpose. Reading it once at
  construction was tried first: it did not close the race on its own (both boots set the variable
  in the same instant) and it broke `hook_e2e`, which installs its stub AFTER its daemon is up and
  spawned a real `claude` instead. The CLAUDE.md line "the daemon reads them once" is true of the
  timing seams, not of this one.
- The notes test names the refusal it gets instead of `assert!(matches!(..))`, which is how the
  cause became visible.

## The approved plan moved from the input to the response (2026-09-03, found by dogfood on 2.1.259)

The block above measured 2.1.251–2.1.258. Claude Code 2.1.259 landed on the author's machine at
02:19 the same day, and the first approval on it (T-176, a throwaway plan filed to test exactly
this) wrote no note: the daemon was fresh, the frame arrived (the card had said `plan ready for
review` off the same dialog), and `plan_note` stayed `None`. Captured live — a settings file whose
`PreToolUse` / `PermissionRequest` / `PostToolUse` hooks `cat` their stdin to a file, driven in a
scratch tmux on its own socket, because `-p` sessions have no plan-mode tools at all:

- `PreToolUse` and `PermissionRequest`: `tool_input: {plan, planFilePath}` — but those fire
  BEFORE the approval.
- `PostToolUse`: `tool_input: {}` and `tool_response: {plan, isAgent, filePath, hasTaskTool}`.

In the binary: `normalizeToolInput` injects `plan` and `planFilePath` into the tool's input from
the plan file (its schema says so — "injected by normalizeToolInput from disk"), a strip step
removes exactly those two keys after the permission decision and before the call, and the
transcript's `tool_use` block keeps the injected form. That last part is why the transcript looked
right and the hook did not, and why "read the transcript" would have hidden the change again.

What holds now: `ingest::plan_of` reads `tool_response.plan` first and `tool_input.plan` second,
so both builds land, and refuses a response whose `isAgent` is true (the `agent_id` check stays).
The unit test carries both shapes. The lesson is the one the docs ladder already teaches about
version numbers: a hook payload captured on one build is a measurement of that build, and the
frame a feature rests on wants its shape pinned in a test that names the build it came from.

## The armed delete flashes the card (2026-09-03, user request)

"Before deleting (when clicking `d` the first time) indicate on the ticket — flash with a red
tinted colour until either delete or cancel." Before this the first `d` was a status line only
(`d again deletes`), and the card it was armed on looked like every other cursor card.

Now, from the first `d` until the second (`d`/`D`) or the stray key that cancels, the card is
drawn as the diff draws a deleted line — `diff_del_bg` as its ground, `err` bold on the title,
the accordion rows on the same ground — square-waving with the ordinary cursor surface on the
MOVE ghost's cadence (400 ms a phase; `Theme::delete_lit` is the phase, `Theme::delete_row` the
lit ground). Same clause as that blink: fg/bg repainting on the 100 ms redraw clock for one card
awaiting a second gesture, never SGR 5. Both colours are ones the diff view already spends, so
the one-saturated-colour law (`attn_is_its_own_colour`, `test_attn_provenance*`) is untouched.
Where the profile has no tint (256 and below) the ground falls back to the cursor surface and
the `err` title carries the flash alone; mono holds steady (its `err` is Reset). The phosphors
tinted nothing when this shipped and their `err` was the phosphor itself, so there the flash was
amber on amber — the next block is the fix.

The ticket page arms the same chord on its subject, so its title row — that page's card — takes
the same treatment (`ui/ticket.rs`; the state row and description keep the band). `App::doomed`
is the one seam (`delete_armed == Some(ticket)`); `card::render` takes it as a parameter and
`ui/board.rs` is its only board caller. `test_delete_armed_flashes_the_card` and
`test_delete_armed_flashes_the_ticket_title` are the spec: lit phase, dark phase, bystanders
still, and the cancel puts the cursor surface back.

## A delivered merge ask is not offered again for a minute (2026-09-03, user report)

"`main moved ∙ m ask the agent to rebase` hint while agent was already notified (maybe need a
minute cooldown)." The m flow's stage is derived from git state and never stored, which is right
for what the NEXT press should do — but the only memory of a delivery was `merge_note`, and any
keypress clears that, so one `j` later the identity line offered the same ask it had just sent.

Now `App::merge_sent` remembers the last delivery (ticket, stage, instant), set on the daemon's
`Ok` for a rebase request or a merged notice, and `App::merge_outstanding` is the seam: `Some`
while the ticket is STILL at that stage and either `MERGE_ASK_COOLDOWN` (60 s) has not passed or
the agent is working (`ticket_busy` — a rebase + test outlasts a minute, and the git probe is
what says it landed). While it holds, `merge_stage_word` is `None` (no `m` hint), the identity
line reads `main moved ∙ rebase requested` / `merged ∙ agent notified` in the calm style, and `m`
itself stays live as it always has — the armed note says `rebase already requested — m asks
again`, so a second delivery is the user's choice and never a hint's. Main moving again lands on
the same stage, which is why a cooldown and not a latch: after a minute idle the offer is back.
The rebase landing changes the stage, so the record stops matching at once. In memory, TUI-side,
like `merge_armed`: a debounce beside the arm state, not a fact for the daemon to persist.
`a_delivered_ask_is_not_offered_again_for_a_minute` is the spec.

## The phosphors are a glow, not a screen (2026-09-03, user request)

"Fix amber and green themes: make it more beautiful and white text. Also red flash before delete
not visible there." The first phosphors (the block above, "Five themes") painted EVERY token on
the one hue — body text included — which is what a P3 monitor does and what nobody wants to read
a board in. And since `err` was the phosphor at full beam, the armed-delete flash on amber was
amber bold on the amber cursor surface: the register was there, the colour was not.

Now the ground and the accent share the hue and the ink is white. `Kind::Phosphor` is restated,
not retired (a fourth clause would have been two ways of saying "the accent's hue is the
ground's"): the ground, its cursor surface, `calm` and the cursor bar sit within 15° of `attn`,
which IS the phosphor at full beam — C* ≥ 60 and the most chromatic token, so the one saturated
colour finally lives where the law puts it; both ramps are neutral (C* ≤ 8.2, base L* ≥ 90 —
graphite's ramp warmed or cooled onto the hue: cream on amber, a cool white on green); `err` is
a red ≥ 45° off the phosphor (`#F26D78`, hue 20°: 57° off amber, 124° off green), so it is its
own colour on both; `calm` is the hue gone pale (C* ≤ 35, dE ≥ 20 from every ramp step);
the diff tints are back (`#501519` / `#4A171A` as the del grounds, a red step up from the
ground and ≥ 45° off the phosphor — the law names the del tint because the delete flash is what
it is for); the register budget holds; the fade target is still the neutral `#161616`. The tag
rings are unchanged — the 70° band they skip is now the accent's alone — and the pip law's
phosphor arm went back to the chroma clause (attn C* 83 over a C* 30 ring) plus the 35° hue
clearance, now measured from `attn` rather than from a ramp base that has no hue to measure.
The cursor bar is gold / mint (L* 80 / 84, C* 51 / 35): the accent's hue without the beam.
Measured: amber ring worst pair dE 14.8, green 14.3; every text role ≥ 4.5 on its surfaces;
`err` 5.9 on amber's ground, 6.0 on green's. Amber was warmed once more the same evening ("a
little more amber": ground C* 12, cursor surface C* 19.5, bar C* 51) and then, an hour later,
taken back to its LADDER — "same as before (originally), less bright with white text" — while
green keeps the glow ("green leave as is, it was good"). That is the fourth `Kind`, `Ladder`:
the glow's clauses with the three dim steps, the bars and both surfaces moved onto the hue and
held UNDER the beam (within 15° of `attn`, ≥ 8 L* below it, less chromatic), only the base step
of each ramp white (C* ≤ 8.2, L* ≥ 90), `calm` held by dE ≥ 20 from every ramp step and the
register budget rather than a chroma cap. One clause could not say "the dims are grey" for green
and "the dims are amber" for amber, and a `Kind` is exactly the thing that says which. Amber's
numbers: the original ground `#1B1201`, cursor surface `#322205` (a step up by 8.6 L*), dims
`#C08B1E` / `#A0751C` / `#664A14` (L* 61 / 52 / 34 — the original's rungs a step less bright;
6.1 / 4.5 / 2.25 on the ground), base `#F3ECDE`, bar `#D59B2C` (L* 68, a rung under the beam),
`calm` back to the original `#DEAE65`, del tint `#4E1717`. Its 256 form is the original's
172 / 136 / 94 rungs under a 230 cream base on the grey ground; at 16 colours its dims sit on 8
again, so the cursor card is structural there (`ladder_16_never_paints_selected`, which
`every_flavor_paints_selected_at_16` became). Picker blurb: `amber ladder on black, white ink`.

Below truecolor: 256 takes graphite's grey ground (233/236) with a cream (230) or white (255)
ramp, the beam on 214 / 41, `err` on 203 (a bright red — the flash has to be seen), `calm` 179 /
114, the bar 222 / 157; 16 colours takes graphite's whole form (the cursor card paints on 8 now
that the ramp is white — `phosphor_16_never_paints_selected` became
`every_flavor_paints_selected_at_16`), `attn` 11 / 10, `err` 9, `calm` 3 / 2.
`test_delete_armed_flashes_the_card` sweeps `Flavor::ALL` and asserts the lit ground is not the
cursor surface and the lit title is not the cursor title, which is the sentence the user wrote.
Goldens are colourless; the picker's two blurbs changed (`warm black, cream ink, an amber glow`
/ `green-black, white ink, a green glow`).

## A sixth theme, Solarized light, and the law's fifth kind (2026-09-03, user request)

"How easy would it be to add solarized light as well?" — then "go ahead, add it". The recipe
held (one `Palette`, one `Flavor`, the picker and the prefs slot need nothing; `Flavor::ALL` is
six), and the laws said what the earlier assessment said they would: canonical Solarized is
low-contrast by design, so what ships is Solarized where the numbers allow and darkened where
they do not. The ground, the cursor surface and the four rest rungs ARE the canonical values
(base3 `#FDF6E3`, base2 `#EEE8D5`, base02 / base01 / base00 / base1 — 12.1 / 5.0 / 4.1 / 2.5 on
the cream, which is the ramp's own floor to the digit). Base01 and base00 fall to 4.39 / 3.64 on
base2, so the `sel` dims are a step darker (`#4E646B`, `#5C717A`), chalk's move. None of the
eight accents clears 4.5 on the cream (yellow 2.98, red 4.29, cyan 2.93, the best of them
orange 4.27), so the three registers keep Solarized's hues and are darkened: `attn` `#8A6600`
(yellow at L* 45, 4.9 on the paper, white ink 5.3 on it), `err` `#9A4247` (red quieted to C* 40
so the register budget holds), `calm` `#1E6F6A` (cyan at L* 42). The ring is chalk's — measured
on this cream ≥ 6.4 on bg, ≥ 5.3 on base2 — and it fades into the cream itself, since the
measured drift stays under 20° (a neutral shadow was ready and not needed). The bars are
base1 / base01 / base02: base00 as `dormant` sat at exactly 15.0 L* under base1 and
`test_bar_ladder` said so. 256 is Solarized's own cube mapping for the greys (230 / 235 / 240 /
241 / 245) with chalk's darker accent indices, because 136 / 160 / 37 fail on 230 the way the
truecolor accents do; light-256 paints no `selected` (06 §2.6) and 16 is chalk's form.

`Kind::TintedPaper` is the fifth clause: the paper and its surface carry a chroma between 5 and
16 on one hue (the surface a step DOWN — a light ground), every ink rung is under C* 16 and
≥ 90° of hue from the paper (cool ink on warm paper is the whole design; measured 104–135°),
the register budget, both diff tints a step down from the paper, and `shadow == bg`. Paper's
8.2 would have refused the cream (C* 10.0) and the blue-greys (9.2–9.3), and a chromatic
ground's floor of 40 is four times what the cream has — the shape is its own. Picker blurb:
`solarized light, cream and blue-grey`; `MESIMON_THEME=solarized`.

Same message, the amber blurb went from `amber ladder on black, white ink` to `amber on black`
and the table's doc comment lost its narrative ("tone down the description for amber, it's not
an advertisement").

## The description bar is a quarter cell (2026-09-03, user request)

"Reduce thickness of description indicator on ticket page." The description block inside the
ticket page's header band carried the card's neutral cursor-weight bar — `Theme::bar(Cursor)`, a
painted cell, one full column wide beside every row. A painted cell has exactly one width, so
"thinner" is a glyph or nothing: `Theme::desc_bar` now draws `▎` U+258E (left one-quarter block)
in `bar_cursor`'s colour as FOREGROUND over the band, and the ladder tiers (mono, ansi8) draw
`|`. A quarter, not an eighth: at one eighth the stroke is a pixel on a laptop panel, the same
"cannot be seen" that refused the SGR-58 underline on the card. The frame is unchanged —
`[pad 1][bar 1][pad 1][text]` — so the text does not move, and the goldens now show the glyph.

That makes `▎` the SECOND codepoint `test_no_drawn_structure` admits off a dialog frame, beside
`▀` (the second tag). Same clause as the first: a thing the design needs, that no attribute and
no painted cell can say, from one named producer. `test_ticket_header_section_is_a_band` pins the
glyph, its colour and the band under it.

## A note opens in the user's own editor (2026-09-03, T-181, user request)

The ticket said "vim editing in notes / description". Two readings: vim's modal keys inside the
TUI's `TextArea`, or the note handed to vim itself. The corpus had already settled the second for
notes before there was a note editor at all (D12: "a note is `$EDITOR` on a file"; 04's `e`), the
author uses Neovim and says they are not an expert vim user (00-DECISIONS on D20), and a partial
vim — the only kind a TUI ever ships — is worse than none: every missing motion is a key that types
a letter into the note. So the note editor keeps its own keys and gains one door.

- `^g` in the note editor (`Verb::EditorExternal`, `Scope::Editor`, prio 22) hands the body to
  `$VISUAL`, else `$EDITOR`, else `vi` — git's ladder, and git's shell form (`sh -c '<cmd> "$@"'
  <cmd> <file>`), so `code --wait` and a path with a space both run. `^g` because it is the key
  Claude Code teaches for "open in external editor", so the finger already has it, and because a
  ctrl-letter is the one floor atom a text field cannot swallow (`^e` was taken by end-of-line).
- The road is the focus handover's (`lib.rs::event_loop`: restore the terminal, blank the
  primary screen, run, re-init, drain stdin) — vim leaves the alt screen the way a tmux detach
  does, and the same flash wants the same blank. `handover::run` judges the exit status on this
  path (cwd `None`), and its message lost the word "attach".
- The file is `<state>/edit/<KEY>.md` for a description, `<KEY>-<note ulid>.md` for another note,
  `<KEY>-new.md` / `new-ticket.md` for one that does not exist yet: inside the README's write
  allowlist, out of `.mesimon/` (a swap file would be a stranger there), 0600, removed after,
  `.md` so the editor reads markdown. Written with a trailing newline, and `back_to_body` takes
  exactly that newline back — vim's `fixeol` would otherwise make every round trip a change and
  dirty every save after it.
- **What comes back is saved at once on a note.** Leaving the editor IS the commit, as it is for a
  commit message and in every `$EDITOR` integration the user knows; an owed `^s` afterwards would
  be the one step none of them ask for. The save goes through `editor_save`, so an emptied note
  still takes the second press to delete, the status still says `saved ∙ ^s again tells claude`,
  and a refusal leaves the text in the editor, dirty, nothing lost. "Changed against what went
  out" is not "changed against the baseline" (type, `^g`, undo the typing in vim): that case says
  `unchanged` rather than letting `^s`'s clean road tell claude. Composing, the draft takes the
  text and `^s` still mints — the ticket does not exist yet.
- The cursor keeps its LINE across the trip (`TextArea::from_text` then `page(line)`), not its
  byte: the line is what the eye had.
- The hint names the program (`^g nvim`): `Ctx::editor_word` is the command's first token as a
  file name, one line, ≤ 16 cells, read once per process and leaked once. `App::editor_word` is
  set in `lib.rs::run` and never in `App::new`, the prefs rule again: no test app reads the
  developer's `$EDITOR`, and with the word empty the binding is inert and unhinted, so no golden
  moved. `mesimon doctor` prints `editor: nvim ($VISUAL) — ^g in a note`.
- Not built: a key on the board or the ticket page that opens vim without the dialog first
  (`n` then `^g` is two keys), and any cwd for the editor (it inherits the TUI's; a worktree
  ticket's would be an argument for `%:h`-style relative opens that nobody has made yet).

Unit: `external::tests` (the ladder, the word, the newline, a real child through the shell for
changed / unchanged / failed), `app::tests::ctrl_g_edits_the_note_outside_and_the_return_saves`,
`ctrl_g_while_composing_fills_the_draft_and_mints_nothing`,
`keymap::tests::ctrl_g_hands_the_body_to_the_users_editor`.

## The done mark decays once seen (2026-09-04, T-173)

The board had no answer to "what changed while I was away": the glyph says what an agent IS,
the `p` peek shows the cursor card's reply, and an agent that printed on a card you were not on
left no trace once its turn ended in `✓`. The T-161 research shortlisted process-compose's
unfocused-output mark (`◆` on a process that printed while unfocused); this is that idea, on a
card — and it ended up being the D19 decay the corpus had already written (06 §3.6, 07 §577:
`seen_at`, `✓` fading once seen; "decay is M6" on `Register::Calm`), built on a seen-tracker
instead of a rest timer.

- **What shows**: a finished agent's done mark is the HEAVY check `✔` U+2714 in the calm
  register while the reply it stands for is one the cursor has not been on the card for, and
  the thin `✓` U+2713 on the grey ramp (`dim2`, the `Grey` register) the moment the cursor lands
  or the ticket page opens. No cell spent, and the state (`Idle{EndTurn}`) is what it was —
  shape and loudness step down together. The colour step alone shipped first (an hour); the
  author wanted "to play with the glyph itself, not colour", and of the narrow one-cell pairs
  (`✔`/`✓`, bold `✓`/`✓`, `☑`/`✓`, `✓`/blank) picked the heavy check with the colour kept under
  it. `✔` carries the Emoji property (text-default), which 07 §18 rule 2 refuses on principle;
  it is a narrow text glyph on iTerm2, and `glyphs::done_unread` is the one place to swap it for
  a bold `✓` if a terminal draws it wide. Mono has no heavier `+`, so there the mark reads `+`
  read or unread. The cursor card and the move ghost are always drawn seen. The rail
  keeps `✓` calm: the ticket page is the looking. `Register::Calm` was always documented as
  "done-UNSEEN"; this is the half that made the word true.
- **A `◊` beside the title shipped first and was cut the same day** (author, on the live board:
  "too big, and also with worktree indication it takes too much space"). Line 1 is already
  glyph + title + `⎇x` + age; any new glyph there competes with the title and a second right-hand
  mark crowds `⎇↑`. Three zero-cell channels were weighed — the `✓` decay, a dot in the pad cell
  between the bar and the glyph, bold on the title (rejected: bold is the cursor card's own
  treatment) — and the decay won for being corpus-native and adding no vocabulary. The trade,
  accepted: only an end-of-turn reply shows. A sentence mid-turn under a spinner has no channel,
  and the spinner already says work is in flight. (For the record, `◊` U+25CA had replaced
  process-compose's `◆`: `◆` and `●` are East Asian Width *Ambiguous*, banned on a width-critical
  row by 05 §7 / 07 §18, and `unicode-width` calls them one cell so a width test proves nothing.)
- **The source is the transcript's assistant RECORD, not pane bytes and not the words.** The
  ticket's open question was tail vs `#{window_activity}`; the tail is quieter (a working
  agent's tool traffic never counts, and neither does the user's own prompt) and it is exactly
  what the peek row shows. `peek::Peek::reply_key` hashes the record's `uuid`: two "Done."
  replies to two prompts are two replies, and a prompt on top (`text` falls to `> …`) leaves the
  key `None`, which the scan reads as "nothing new".
- **State is TUI-local** (`App::spoke: HashMap<Ulid, Spoke { session, path, key, seen }>`; the
  description said a restart may forget, and it does — a fresh board finds every reply unread,
  which is how `✓` always looked, and greys each as the cursor reaches it). `scan_ticket` reads
  `Board::pane_target` — the one paned claude, the same predicate the daemon's prompt delivery
  and `board_enter` use — starts a fresh entry when the session or path differs (a spawn, a
  wake, a `/resume` that relearned the path), and drops the entry when there is no such session,
  which is the description's "a parked card never carries a stale new" (a parked card wears `z`
  anyway). "No reply yet" is 0, which `seen` starts at, so an agent that has not spoken owes
  nothing.
- **The beat is `poll_spoke` in `App::tick`**, after the refresh block, beside the shell-tail
  and note polls: the departing card is scanned and acked the tick the cursor leaves it (a reply
  that landed under the cursor between two clock beats was seen, not missed), the board is
  scanned on `SPOKE_EVERY` (1 s), and the subject is acked every tick. A verdict change is a
  redraw, never a snapshot: nothing on the wire knows what an agent said. The ack is positional
  — a card that slides under the cursor on a snapshot counts as looked at, so does one under a
  dialog — accepted.
- **`PeekCache` went from one slot to one entry per path** (`retain` prunes against EVERY
  session's transcript, not only the paned claudes': the ticket page previews a corpse through
  the same cache). Cost, stated: one `stat` per paned claude per second at rest; a Running
  session's file moves on every tool result, so each busy session costs a 64 KiB reverse scan
  (256 KiB when the window holds neither reply nor prompt) a second — ten busy agents ≈ 10–20
  ms/s on the draw thread.
- No key, no `Ctx` field. A "next unread" jump beside next-needs-you would be the first thing to
  earn one, and `Spoke.seen` is what it would read.
- Tests: `peek.rs` (`reply_key` names the record, the cache holds every path and prunes);
  `app.rs` (a reply already there is unread on first sight, cleared on landing, the ticket page
  counts, a prompt alone is not the agent, a parked card holds no entry, a reply under the cursor
  is seen when the cursor leaves, a fresh session starts its own entry, a first reply is news);
  `test_done_mark_decays_once_seen` (thin grey at rest, heavy calm when unread, thin on the
  cursor card, a visible step on every flavor, `+` in mono); the unread `✔` seeded into both L1
  sweeps.

## A person's ask supersedes the person's own park (2026-09-04, T-186)

Dogfood: a ticket ran TODO → IN PROGRESS → REVIEW, the user pressed `<<` (REVIEW → IN PROGRESS
→ TODO, two hand moves), then Shift+Enter to ask claude again — and the card sat in TODO while
the agent worked. The feed said why: `move_refused:ping_pong`. `movegate`'s no-undo rule
refuses an automatic move that is the exact reverse of a move somebody else made inside 60 s,
and automove's TODO → IN PROGRESS on the `Running` edge IS the reverse of the hand's
IN PROGRESS → TODO. The rule was written for "a human dragging a running ticket back to TODO
watches automove snap it forward" — but the drag and the ask are the SAME hand, and the ask is
the newer intent.

- **The rule**: `MoveGate::asked_by_hand(ticket)` drops the ticket's last-move entry iff its
  actor is `local`. Called from the hook frame path on `Signal::UserPromptSubmit`, BEFORE the
  machine applies the signal, so the `Running` edge that follows finds no move to protect.
  One call site covers every road an ask takes — the board's Shift+Enter field
  (`PromptSession`), the composer's submit, a line typed in the pane — because they all end in
  that frame.
- **What stays protected**: an AGENT's move. `mcp_e2e` sends a prompt after the agent's
  `move_ticket` to REVIEW and asserts automove does not drag it back; the person did not make
  that move, so their ask cannot supersede it. The fuse is untouched — a move by hand is still
  what clears it, as its notice says.
- **Known imprecision**: a task-notification wake also arrives as `UserPromptSubmit`, so a
  ticket parked on background work, dragged to TODO, and woken by its task will move to
  IN PROGRESS. The agent is genuinely working then; accepted.
- E2e: `crates/mesimon/tests/ask_after_park_e2e.rs` (fails without the call — verified). Unit:
  `movegate::tests::a_prompt_by_hand_*`.

## The Esc interrupt is known by its words (2026-09-04, dogfood)

- **Symptom**: "escape to interrupt claude causes ticket status always running". The card kept
  the spinner from the Esc until the next prompt.
- **Cause**: `adopt::classify_tail_record` recognised the Esc press by the `interruptedMessageId`
  field beside the `[Request interrupted by user…]` record, and the field is optional. Census of
  the local corpus (Claude Code 2.1.220–2.1.259): the tool-use spelling (`… for tool use]`) lacks
  it about half the time — 9 of 22 on 2.1.251–2.1.258 — and an SDK-driven interrupt never has it.
  Correlated against this repo's activity log: every flagged record produced `idle interrupted`
  within ~3 s; every unflagged one produced NOTHING in the following 120 s. The pane-quiet
  fallback (60 s) did not rescue any of them — the idle prompt keeps repainting, as T-50 measured.
- **Fix**: `adopt::is_interrupt` — a `user` record whose text (a string, or the first text block
  with no `tool_result` beside it) starts with `[Request interrupted by user` IS the press, flag
  or no flag; the flag stays as a second road. `user_prompt` takes the same test, so the sentence
  no longer reaches a card as `> [Request interrupted by user for tool use]`. Same shape as the
  `<task-notification>` tag: a harness sentinel, known by its words.
- **Accepted imprecision**: a person typing those exact words as a prompt is read as an
  interrupt — a cosmetic `Idle{Interrupted}` at Low that the turn's next hook corrects.
- `interrupt_tail_e2e` now appends the unflagged tool-use form (fails without the fix — verified);
  the flagged form and the two impostors (a quoting tool result, an assistant saying the words)
  are unit fixtures in `adopt.rs`.

## The recordless Esc is caught by Claude's own session file (2026-09-04, dogfood)

- **Symptom, second half**: the same "always running" report, reproduced live as Enter then Esc
  within ~2 s — "the conv returned to the last message before I prompted". Claude Code hands
  the prompt back to the box and writes NOTHING to the transcript (spike S-E's case, still true
  on 2.1.259): no hook, no `[Request interrupted…]` record. The 60 s pane-quiet probe was the
  whole catch, and it fired at +63 s.
- **Signal**: `~/.claude/sessions/<pid>.json` — the file Claude Code keeps for its own peers
  (`peerFeatures: notify_idle`) — carries `status: busy|idle` and `statusUpdatedAt`, and the
  interrupted session's stamp was the Esc's own second (01:13:12.012 for a 01:13:12 keypress).
  Measured across the 40 live files on this machine: every `busy` was a Running record of ours,
  every `idle` an Idle one; two files from older builds carried no status at all.
- **Doc 11 §11.3 refuted**: it barred the pid file from setting any §11.7 state as "best-effort
  enrichment". The status is written at every edge, so it gets PaneQuiet's row sixty seconds
  earlier and nothing more: `Signal::StatusFileIdle` shares the arm (Running → Idle{Interrupted},
  Medium, demotion-only, settle). `Daemon::probe_status_files` runs on the tail-poll cadence
  (2 s), finds the file once per Running spell by sessionId + live pid
  (`census::status_file_for`; a resumed conversation can leave a dead process's file beside the
  live one), and requires `statusUpdatedAt ≥ state_changed_at + 250 ms` — the previous turn's
  `idle` write and this turn's `UserPromptSubmit` can land in either order (a prompt typed
  ahead is submitted the instant the turn ends). It shipped at 1 s for an hour and blocked a
  live Esc 938 ms after Enter; the hazard is milliseconds wide, a person's Esc is not. No file
  (an older Claude Code) means a re-look every 30 s and the pane probe as before.
- **And a probe never overrides a pending stated leave** (same hour): a `Stop` settles 1500 ms,
  the file goes idle in the same second, and the probe on its 2 s cadence replaced the pending
  `EndTurn` with `Interrupted` — a finished turn that automove would not promote. `PaneQuiet` and
  `StatusFileIdle` now fire only with nothing pending; a Stop arriving inside the probe's own
  settle still replaces it (`a_probe_never_overrides_a_pending_stated_leave`).
- E2e: `crates/mesimon/tests/interrupt_status_e2e.rs` — a stale idle holds Running, a fresh one
  demotes while the pane paints and the quiet threshold sits at 600 s, a re-prompt is not
  re-demoted by the old stamp. Unit: `attention::status_file_idle_is_pane_quiet_sixty_seconds_early`.

## The grown composer: `^s` keeps the draft, `^S` mints and asks (2026-09-04, user request)

**Refuted:** "Shift+Enter in the editor is a newline" (above) left the grown composer with no
"save + ask" at all — `^s` minted and closed, and the agent was the board's Shift+Enter on the
minted card, a press later. The author asked for the one-line composer's gesture inside the big
one, then corrected the first cut (which put it on `^s` itself): "^s should just save (and exit
the composer to go back to small composer). ^S to save, exit and run claude like shift+enter
would do". So the big composer is the small one's second room, not its replacement: what is
typed there comes BACK, and the small composer's two Enters stay the two ways out.

**Built:**
- `InputPurpose::Create` carries `description: Option<String>` beside `workspace` and `tags`.
  `App::fold_composer` is the road back — `^s` (`Verb::EditorSave`, composing) and a clean Esc
  both take it, title, picks and body riding along, a blank body being no description, status
  `description kept`; `Tab` reopens the editor on the kept text (`TextArea::from_text`), clean;
  `commit_input` hands the description to `mint_ticket`, which writes it through `WriteNote`
  after `CreateTicket` and before any spawn. Nothing leaves for the daemon on `^s`. The `^s`
  hint reads `save` while composing, and its `avail` gains `editor_composing`: the carried title
  is not dirty against the editor's baseline, so `Tab` then `^s` with nothing typed was pressing
  nothing.
- **`Key::Ctrl('S')` — ctrl+shift+s — is the third off-floor atom, admitted on Shift+Enter's
  clause, not a third one.** The case IS the shift (`Key::Char` already spells `S` for
  shift+s; `Display` now writes `^s` / `^S` apart). `keys::to_key` mints the uppercase atom only
  when the SHIFT modifier arrives on a ctrl+letter, which only the kitty tier reports: a legacy
  terminal sends the bare 0x13, which is `^s`, another verb — the ambiguous family, so every
  binding on it is gated on `Ctx::rich_keys` and the press degrades to exactly the unshifted key
  (`ctrl_shift_s_is_inert_without_rich_keys` runs the same law as the ShiftEnter test through a
  shared helper; `OFF_FLOOR` lists it beside its test; app test
  `ctrl_shift_s_degrades_to_ctrl_s_on_the_legacy_floor`). A Shift that arrives without the tier
  resolves to nothing: inert, not wrong.
- `Verb::EditorSaveStart` on it, `Scope::Editor`, composing only, `^S save + ask claude`, prio 12
  — `App::editor_save_start`: `mint_ticket(.., start: true)`, the ticket, then its description,
  then `SpawnSession { submit_prompt: true }`, in that order so the agent's first `get_ticket`
  already carries the description; the board stays, status `claude started on the title` (or
  the worktree's `provisioning …`), no Enter window armed. `shift_stays_on_one_axis` names the
  pair: `^s` keeps, `^S` mints and asks — Enter / Shift+Enter's bargain on the save key's own
  shift. `Scope::Editor`'s Shift+Enter, `Verb::SaveStart` and
  `shift_enter_asks_claude_at_every_stage` are untouched: the same sentence on another atom, not
  a fourth ShiftEnter home.
- Golden `editor_compose_120x30`'s bottom edge reads `^s save ∙ ^S save + ask claude ∙ esc close ∙
  ^t tags`. A blank title: `^S` refuses in place (`a ticket needs a title`); `^s` has nothing to
  refuse and folds back for the title to be typed.

**And the same pair on a ticket that exists** (the author's third message, same hour: "if no claude
session in ticket, treat like new. either way ^s saves and exits the dialog"):
- **`^s` always saves and leaves.** On a note the body is written and the editor closes; a clean
  one just closes; a blank new one says `nothing to save` and closes; an emptied existing one keeps
  its two-press delete. The binding is `avail: editing`, hint `save`, dirty or not. **Refuted:** the
  note editor's "stays open, and a second `^s` on a saved note tells claude" (the notes block and
  T-181's) — the second press had nowhere to land once the first one left. `Ctx::editor_can_tell`
  is gone with it. The one road that still writes and STAYS is `^g`'s return
  (`external_edit_done`, through the new `App::write_note`): the editor's write is the commit, and
  the dialog is what the user came back to.
- **`^S` on a note follows who is on the ticket.** After the write (a dirty non-blank body;
  an emptied existing note refuses with `empty ∙ ^s deletes the note`; a blank new note writes
  nothing and still asks — a blank description is none): a claude with a pane is told
  (`NoteToAgent`, the old second press, hint `save + tell claude`, `Ctx::editor_claude_paned`); a
  ticket with NO claude gets one started on the title through `start_composed`, exactly as a
  minted ticket does (`Ctx::editor_seat_empty`, `Board::live_claude` none, hint `save + ask
  claude`); a Sleeping claude holds the seat and has no pane to type at, so the key is inert and
  unhinted — the board's Shift+Enter's rule there (`c` wakes it). App tests
  `a_note_save_closes_the_editor`, `ctrl_shift_s_on_a_note_tells_claude_or_starts_one`; keymap
  `ctrl_s_says_save_and_always_leaves`. The note goldens' edges read `^s save ∙ esc close`.

## An interrupted turn wears `⊘` (2026-09-04, user request)

- `Idle{Interrupted}` fell through every arm of `card_glyph` and drew nothing — the look of a
  ticket nobody had opened (author: "it looks like no session exists there"), and the rail
  showed the plain `◦ idle`. Now `glyphs::interrupted`: `⊘` U+2298 CIRCLED DIVISION SLASH, the
  halt sign — EAW=Neutral, Emoji=No, Mathematical Operators; ASCII `;`. `¦` BROKEN BAR shipped
  first and was cut within the hour as too thin ("doesn't read nicely"); `⏸`/`⏹` (emoji
  presentation) and `‖`/`■` (Ambiguous width) are barred by the width law, since any of them
  can paint two cells. Still (no motion: nothing is happening), grey register (the user
  stopped it and knows), state word `interrupted`. Precedence: under anything in flight or
  finished, over `waiting` (a known fact outranks a lost one). Golden
  `board_interrupted_120x30`; `glyphs::an_interrupted_turn_has_its_own_still_mark`.

## Release notes are the changelog, on a screen (2026-09-04, user request)

**Asked:** "release notes in menu (opens full screen beautiful release notes). think where to
store them, grouped by the release name + date."

**Where they live — decided:** `CHANGELOG.md` at the repo root, and nowhere else. It already
existed with one `## <tag>` section per release and `ci/release.sh` already lifted the tag's
section out of it for the GitHub release body, so a second store (a `releases/` directory, a
fetched body from the dist repo, a state-dir cache) would have been the same words twice with a
new way to disagree. Each heading gained its date — `## v0.1.0-alpha.11 — 2026-09-03`, backfilled
from the tags' commit dates — and the file is `include_str!`ed into `mesimon-core`
(`relnotes::SOURCE`), so the notes ship inside the binary: offline, nothing written, and a build
carries exactly the versions that existed when it was made. Fetching the newer release's notes
from the dist repo was considered and refused: the header's update chip already names the newer
version, and a page that needs the network to say what THIS build does is the wrong page.

**Built:** `core/src/relnotes.rs` (`Release { tag, date, body }`, `parse`, `date_words` → `3 Sep
2026`, `position`) with a test that runs the parser over the real file and holds it to dated,
unique, newest-first headings whose top entry is the workspace version; `ci/release.sh` dies
before anything slow without a dated heading for the tag (and its awk matches the heading as a
prefix now). `Scope::Releases` / `Verb::ReleaseNotes` / a `MenuItem` after the theme row;
`Screen::Releases` + `App::releases: ReleasesState`; `ui/releases.rs`; `RELEASES` in the header
chip. The screen is a centred reading column (≤ 100 cells) of painted bands and `rich.rs`
markdown, the current release's band marked `this build`, the band of the release under the
window pinned to the first row while its notes scroll. Keys are the diff's reading set on the
diff's verbs, routed on the screen. Goldens `releases_120x30`, `releases_scrolled_120x30`,
`releases_160x30` are seeded from a fixture; the real changelog is paged through under L1 in
`test_real_changelog_reads_lawfully`; both law sweeps gained the screen. Menu goldens grew two
rows; `test_a_sub_floor_sleep_offer_still_outranks_archive` now reads its row from the dialog's
frame, since the taller menu put the sleep row beside a board card.

**Not done:** no "new in this build" suggestion on first launch after an update (a `seen` stamp in
`prefs.json` and a `Suggestion` pointing at the row would do it; the author has not asked); no
`g`/`G`; the notes are not scrubbed through `text.rs` because they are the repo's own file, not
user or agent text.

## A page turn glides (2026-09-04, author: "`{ }` in ticket page should scroll smoothly")

**What was wrong:** `{ }` on the ticket page's PREVIEW zone jumped a whole window a press. A page
with one row of overlap is the right DISTANCE (a page is a page, as in the diff), but a jump loses
the eye: the row it was reading is somewhere else, or gone, with no motion to say which way.

**Built:** the press still moves the RECORD at once — `App::preview_page` writes the next offset
into `preview_view` and `preview_scroll` exactly as before, so the footer, the clamp, the tail
release and a held key's queued presses see the same numbers they did — and arms
`App::preview_glide: Cell<Option<Glide>>` (`Glide { key, from, at }`, `app.rs`): the document,
where the window WAS, and when. `ui/ticket.rs::window` draws the rows at `Glide::offset(target)`
— eased out, the composer dialog's curve — and retires the glide once it has landed or when the
document under it changes, so a new reply opens at its top with no motion. `App::animating` now
counts a live glide on the ticket screen, which is what makes the event loop poll at 16 ms for
the duration. **`GLIDE` is `GROW`** (180 ms): a screen gets one speed of motion, the dialog's,
not a second one for scrolling. A second press mid-turn starts from where the eye is
(`Glide::offset` of the current target), never from where the first press started, so a held key
is one continuous scroll rather than a stutter of restarts.

**Tests:** `test_preview_pages_a_long_reply` now pins frame zero (the glide dated into the future,
as the grow test does), a midway frame between the two pages, the record not moving with the
frame, the landed frame, the retire, and the mid-turn restart; `page()` in the test module presses
and settles. The shell-tail test pages through the same helper. No golden moved: a golden renders
a resting board.

**Not done:** `j`/`k` on the diff and the release notes still jump — they move one row, which is
its own smoothness; the diff's and the releases' page keys still jump a page, and could take the
same `Glide` if the author asks.

## Two hints go quiet: the sleeper's `x` and the editor's Shift+Tab (2026-09-04)

**Ask:** "remove hint `x wake` in ticket page and `shift+tab shared checkout / own worktree`
in bottom (already hinted composer)"; then, on the second: "shift+tab should stay in small
composer (and actually be introduced in big composer), but removed from bottom hints".

**`x` on a sleeper is bound, not hinted** (`keymap.rs`, `Scope::Ticket`): the rail's trailer
said `x wake` under a row whose `enter` already reads "wake" and whose `c` reads "wake claude"
— a third spelling of one act. The hint closure returns `""` on `sel_sleeping` (the `c`
precedent: `binding_for` and the footer drop an empty hint, the key still resolves), so the
verb keeps its two words, `sleep` and `dismiss`. `x_says_what_it_will_do_to_the_row_under_it`
pins the silence and that `enter` still says "wake" on the same row. Golden
`ticket_archived_120x30` lost the trailer word.

**Shift+Tab is spelled beside the pick it cycles, never in the edge.** The one-line composer's
card already carried `⎇ shared  shift+tab` (`card::render_workspace_selector`); the grown
dialog's context row named the workspace but not the key, and its bottom edge (or the footer,
where the edge had no room) said `shift+tab shared checkout / own worktree` in full. Now
`editor::context_line` puts the same `  shift+tab` (dim2) after the workspace word — composing
always, and on a description while `Ctx::workspace_open` (the choice locks with the first
session or worktree, and a key spelled beside a setting it cannot change is a lie) — and the
`Scope::Editor` binding's hint is `""` at `prio: 0`: bound, silent, one place per hint (T-158's
rule). `tags_reach_the_composer` now asserts `hint_for` is `None` there. Goldens
`editor_compose_120x30` and `editor_compose_tags_120x30` moved by the one word;
`editor_describe_120x30` did not, its ticket has a session. The one-line composer's own
`Scope::Input` binding is untouched (the card's spelling is the hint).

**Cost:** `?` on the editor no longer lists Shift+Tab, and on a sleeper's row does not list
`x` — the same trade `c` on a paned claude already makes.

## `HJKL` moves the card (2026-09-04, user: "shift + hjkl to move tickets")

The board's nudge (`Verb::Nudge`) was reachable only through the four Alt atoms since
2026-09-01, and the footer taught it as `option+hjkl`. `HJKL` now sits in the SAME binding's
key list, the arrangement the tag picker already had: `hjkl` steps the cursor, `HJKL` steps it
carrying the card, and the Alt atoms are the same entry, so the accelerator cannot reach a move
the floor does not make. The footer names `HJKL`; `> <` (aiming) stays overlay-only.

**Why it matters for the Alt clause:** `alt_is_admitted_only_for_a_nudge` requires every Alt
binding to carry a legacy-floor spelling of the same move on the same screen, HINTED. For three
days the board met the bound half (`> <`) and had spent the hinted half — a terminal that eats
the modifier read a footer whose move key did nothing. Both halves hold again, and the test's
docstring records the interval.

**Cost:** the footer's `HJKL` says nothing about `option`, which still works. `?` lists both
under one row, as it does for the picker. Goldens with a board footer moved by one cell.

## The board says where its checkout stands (T-124, 2026-09-04)

**Ask:** "git status + pull/push indication on board". Decided with the author: indication
only (no push, no pull — the shell does those), the fetch opt-in and periodic, and "think how
to style it to be beautiful".

**The look.** The board header hangs the checkout's state off the breadcrumb, because the
breadcrumb is where you are and so is the branch: ` BOARD   mesimon > kanban-tui ⎇ main ↑2 ↓1
∙ 3 changed   7 tickets … ◦ update ready (U ∙ esc)`. The glyph is `dim3` and the name `dim2` —
quiet identity, the weight of `mesimon` in the crumb — the arrows ride the calm register the
card's `⎇↑`/`⎇↓` already use for "something to do here", and the change count is a fact in
words (`3 changed`), never a `*` on the name: the header speaks in words and a star is a
footnote nobody can look up. A clean branch in sync is the name and nothing more; nothing is
drawn until a sample has landed, so **no existing golden moved** (`board_git_120x30` is the
new one). ASCII tier: `& main ^2 v1 ∙ 3 changed`. `glyphs::branch_mark`/`ahead_mark`/
`behind_mark` are now the ONE home for `⎇ ↑ ↓` and `card.rs::worktree_mark` reads them, so the
card and the header cannot drift.

**The offer has first claim on the row.** The left clauses used to shrink the right chip's
budget and a long one dropped the update chip at 100 cols (the reason notices went to the
advisory row). `draw_header` now sizes the suggestion chip FIRST and fits the git clause into
what is left (`chrome::git_clause`), which gives its parts up in order: the count drops, the
name truncates through `text::truncate` to a floor (`GIT_BRANCH_FLOOR` 10, the ticket page's
`WT_BRANCH_FLOOR` idea), the arrows are never cut, and below the floor the clause stands aside
whole rather than lie. `test_git_clause_gives_way_to_the_offer` pins 160/100/80 cols with a
42-cell branch and the release chip on. Words live in the menu: the `Fetch origin` row's
detail is `2 to push ∙ 1 to pull ∙ fetched 4m ago` (`App::git_fetch_note`, `Ctx::git_fetch_note`).

**One fork, off the writer.** `daemon/src/gitstatus.rs::sample` runs `git status
--porcelain=v2 --branch -unormal -z --no-optional-locks` once (0.01–0.03 s here; `-unormal`
never walks an ignored dir and collapses an untracked one to a row; the optional-locks flag is
what stops `status` writing the refreshed index back) and `parse` reads `branch.head/upstream/
ab` and counts the entry records — under `-z` a rename's original path is its OWN field, so the
walk is a cursor, not a NUL count (`ahead_behind_and_every_kind_of_change_counts_once`). The
worker is the shell-env road: `queue_git_sample` spawns a thread, `Msg::GitSampled` lands on
the writer, `on_git_sampled` broadcasts only on a real delta. Two flags, `git_inflight` and
`git_wanted`: an ask mid-flight (boot, a `GitFetch` press, the post-`ff_merge` sample) queues
and re-runs, never drops. The tick spawns on `ticks % RSS_TICKS == 1`, one off the writer's own
worktree-flag burst. The fetch bookkeeping (`fetching`, `fetched_at_ms`, `fetch_error`,
`fetch_every_secs`) lives on `Daemon` and is STAMPED into `Response::Board.git` by `snapshot()`,
so the cache compare sees the sampled part only and an armed fetch cannot make every cycle
read as a change. `RepoGit` is `#[serde(default)]` on `Response::Board`; an older daemon reads
as unsampled.

**The fetch, fenced.** `MESIMON_GIT_FETCH=<minutes>` (read once at daemon start; doctor's
`branch` line prints it) arms the periodic one; `Command::GitFetch` (the menu row, a person's
gesture, `agent_allows` denies it) fetches now either way. Both run on the same worker BEFORE
the sample: `git -c gc.auto=0 -c maintenance.auto=false fetch --quiet --no-write-fetch-head
--no-tags --no-recurse-submodules <remote>` with `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS=""` (an
empty value shadows `core.askPass` AND `SSH_ASKPASS` in git's ladder) and
`SSH_ASKPASS_REQUIRE=never`; `setsid` in `pre_exec` so a foreground daemon's tty cannot reach
it and so the whole process group can be killed on the 30 s deadline (`Child::kill` reaches
`git`, not the `git-remote-https`/`ssh` under it, which hold the stderr pipe). `GIT_SSH_COMMAND`
is deliberately NOT set — it would override `core.sshCommand` and the user's identity setup.
`gc.auto=0` matters: a fetch may otherwise trigger `gc --auto`, which repacks objects and
packed-refs, writes far outside the README clause. The remote is `branch.<name>.remote` (never
the `origin/` prefix, which a remote with a slash would break); `.` means a local upstream and
no fetch. A failure keeps the first stderr line on the menu row's detail and the previous
refs stand; a success clears `base_branch` (a fetch on git ≥ 2.48 can mint
`refs/remotes/origin/HEAD`, `default_branch`'s first rung). README promise 1 gained the clause.
E2e: `crates/mesimon/tests/gitstatus_e2e.rs` — bare origin, two clones, ahead, then behind
only after `GitFetch`, `FETCH_HEAD` absent, a dirty tree counted.

**A bug found on the way.** `App::fetch` dropped `shell_env` on the strength of an `App::apply`
that did not exist, so the `shell env changed` chip only ever appeared once the external drawer
had been opened. Every snapshot field now rides one `Snapshot` struct through one `App::absorb`,
whoever asked for it.

**Not done:** nothing per ticket. A configurable fetch cadence per repo (rather than the env)
and a `mesimon doctor` check that the remote answers without a prompt are the obvious next
asks.
## A ticket can be snoozed (T-74, 2026-09-04)

**Ask:** "snooze ticket from board ∙ think about snooze time presets (1h, tomorrow morning,
next week…) ∙ archive then return to board with optional needs-you returned from snooze
(configurable) ∙ snooze hotkey cycles snooze times, indicated on ticket as well as hint when
applying snooze, enter approves, esc cancels". Decided with the author: the key is `z`, the
ladder is `1h · 4h · tomorrow 9:00 · next Monday 9:00`, needs-you on return is ON with an
Esc-menu opt-out, and a woken ticket lands at the top of its column.

**A snooze IS an archive with a deadline.** `Archived` gained `until: Option<String>`
(`@<secs>`) and `needs_you: bool`, both `#[serde(default)]` and skipped when unset, so a plain
archive's file is byte-for-byte what it was. Everything an archive already gets, a snooze gets
for nothing: hidden by the one chokepoint `Board::column_tickets`, refused by `place_ticket`
and the spawns, listed in the ARCHIVED dialog (its row reads `wakes in 3h` where a plain
archive's reads its age; the ticket page says `archived ∙ wakes in 3h ∙ a restores`), restored
by `a` and by `u` — and a restore by hand simply cancels the deadline. `Response::Board` did not
change. `Command::SnoozeTicket { id, until, needs_you }` (`Mutate`, logged, never-tier for
agents) takes `archive_ticket`'s three gates plus "the deadline is already past"; the TUI
resolves the preset to unix seconds at the Enter and the daemon only compares clocks.

**`TICKET_SCHEMA` is 3**, on the notes bump's reasoning: a v2 build would read the file, drop
`until` on its next save, and the ticket would sleep forever. The trade is the same — a v3
ticket file is NOT loaded by a v2 build (a notice, the ticket absent there until the newer
build is back). `pre_snooze_ticket_toml_parses` pins both directions.

**The wake is the tick wheel's** (`Daemon::wake_snoozed`, the 1 s bucket of `on_tick`,
before the archive re-price): every ticket whose `until` has passed comes back with
`archived = None`, at the TOP of its column (`order_within(.., Position::Top)` — the return is
fresh news, the way every automatic move lands), `entered_at` restamped so the card's age
restarts, the move gate forgetting it (restore-by-hand's rule), a vanished column falling back
to the first, one `save_ticket` per ticket, a feed line `snooze_woke` with `automation` as the
actor, and NO broadcast of its own — `on_tick` fires the one for the bucket. Nothing in
`begin_shutdown`: deadlines are on disk and a restarted daemon wakes the overdue on its first
tick. The daemon has no unit harness, so `snooze_e2e.rs` is the coverage: a 2 s snooze leaves
the board, returns on top, lit, with the age moved; a quiet one returns unlit; a past deadline
and an awake session are refused.

**The first ticket-level producer of the saturated colour.** Attention was a session's
property — `RequiresAction` at usable confidence, three predicates (`card_glyph`,
`card::is_waiting`, `attention_queue`), six paint sites. A snooze that asked to be lit sets
`Ticket.woke_at` (a scalar, before `[[tags]]`) on the wake, and the card wears `!` in
`Register::Attn` — the inverted title row and the bar follow from the register — with no
session behind it. `card::render` wraps `card_glyph` rather than changing its signature (one
production caller, forty-five test sites); `card::needs_you(ticket, sessions)` wraps
`is_waiting` for the off-screen `!N` badge and the collapsed spine; `Board::needs_you_count()`
(`attention_queue` + woke tickets) is the header chip's number and the daemon's tmux status
chip's, so the two cannot disagree. `test_attn_provenance_woke` sweeps `Flavor::ALL` with a
session-less woke card: attn on its own rows and the header, nowhere else, and the folded
column carries the mark. `test_attn_provenance_calm` did not move.

**The mark comes off on a KEYPRESS, never on the draw clock.** `Command::SeenTicket { id }`
(`Mutate`, unlogged, a no-op with no write on any other ticket) is sent by `App::ack_woke` at
the end of `on_key` when the key left the cursor on a woke ticket — a `j` that passes over one
does not ack it, a `k` that lands on it does, and so does opening its page. Draw-time acking
was refused on purpose: a ticket wakes at 09:00 at the top of its column while the user is
away, and a cursor that happened to be parked in that slot would have cleared a mark nobody
looked at. `a_keypress_on_a_woke_card_acks_it` pins the passed-over case.

**`z` is a chord you stay in.** `Scope::SnoozeChord` (word `SNOOZE`, parent `None`, the fifth
barrier in `q_pops_and_help_is_everywhere`'s list): `z` on the board or the ticket page arms
on `1h` (`Verb::SnoozePrefix`, overlay-only like `a`, avail `has_ticket && !ticket_archived`;
an awake session gets the archive's own refusal before any second press), `z` inside walks the
ring (`SnoozeNext`), Enter takes the pick (`SnoozeConfirm`, `Class::Grace`, hint
`snooze::hint_for_label(c.snooze_word)` — a `&'static str` per rung, because a hint is one),
Esc leaves (`SnoozeCancel`), any stray key cancels through the `None` road with `snooze
cancelled`. The armed card draws OPEN (the quick-tag flash's seam) with the preset on a row of
its own under the title, `dim3`; the status says `z next ∙ enter snooze 1h ∙ esc cancels` and
names no clock, so the golden `board_snooze_armed_120x30` is deterministic — the resolved time
(`snoozed T-9 until 15:42 ∙ u undoes it`) is the confirm's. `Ctx` grew `snooze_word` and
`snooze_needs_you`. `snooze_is_a_chord` in the keymap and the app pin the shape;
`one_verb_one_key_across_screens` and `unavailable_keys_do_nothing` learned `z`.

**`z` beside `Z`.** `Z` sleeps the done agents and stays board-only, `prio: 0`; `z` snoozes the
selection. Both are zzz — two things that sleep — not one verb on two targets, which is what
retired `X`. `shift_stays_on_one_axis` says so. The only other `z` is the diff viewer's view
prefix, a reading screen with no ticket verbs, and neither is reachable from the other.

**The calendar rungs.** `core/src/snooze.rs` is pure: `Preset` (the ring, `label`, `next`),
`LocalTime` in `struct tm`'s own conventions (years since 1900, 0-based month, 0 = Sunday, so
the glue is a field copy), `target` (09:00, `mday + 1` or `+ days to the NEXT Monday — seven on
a Monday`, `mday` left un-normalised on purpose) and `deadline(preset, now, local, to_epoch)`.
`tui/src/localtime.rs` is the libc on either side — `localtime_r` in, `mktime` with
`tm_isdst = -1` out, which does the DST arithmetic and rolls `mday = 32` into the next month, so
no month-length rule lives anywhere in mesimon. The core tests inject a civil-from-days fake
`mktime` (Sunday/Monday/Saturday, month-end, year-end); the TUI test round-trips the real clock
and checks every rung lands ahead of now at 09:00 in whatever zone the test runs.

**The preference.** `prefs.json` gained `snooze_needs_you` (default true): read with a default,
written on every save, so a theme pick by an older build keeps it (the file is a `Map`, no
schema move). The Esc menu's `Snooze returns with needs-you` / `Snooze returns quietly` row
(`Verb::SnoozeQuiet`, never a suggestion) flips it and says so; `App::save_prefs` is the one
road every preference's write now takes (the theme pick moved onto it). `mesimon doctor` prints
a `snooze` Note.

**Not done:** snoozing a ticket whose claude is awake still asks you to sleep it first, in the
archive's words — a snooze that parks the agent on the way out is a follow-up. The card says
nothing about a snoozed ticket because a snoozed ticket is not on the board; the ARCHIVED row
is where its deadline reads. `CHANGELOG.md` is the release commit's, not this one's.

## A snooze sleeps the idle agent first (2026-09-04, user: "snooze auto sleep sessions if not running")

`snooze_ticket` took `archive_ticket`'s awake gate verbatim, so `z` on a ticket whose claude had
finished its turn came back `sessions still awake — sleep them first`: an `x`, then the `z`
again. A snooze says "not now", and an agent that has stopped is exactly what `x` would have
parked, so the daemon now does the `x` on the way. **What goes is what `x` would take** —
`sleep_eligible` without the bulk sweep's 60 s floor (a `z` is as deliberate as an `x`): a
claude at `Idle{..}`, a shell with no live child, nothing pinned awake. **All-or-nothing**:
every pane on the ticket is judged BEFORE any is signalled, so a claude still `Running` (or
waiting on the user, or `Unknown` after a restart) holds the ticket on the board with nothing
touched, and the refusal names it — `claude still awake — only idle sessions sleep`, `shell
still awake — bash has live children (…)`. The sleeps ride `sleep_one`, so the transcript copy,
the `Sleeping` latch, the SIGTERM and the reaper are the same ones; `persist_sessions` runs
before the ticket write. The TUI's confirm counts what it saw awake — `snoozed T-9 until 15:42
∙ its session asleep ∙ u undoes it` — and undo is still the restore: the ticket comes back, the
agent stays parked, `c` wakes it, on the wake by hand and on the tick wheel's alike. Plain `a`
keeps refusing: an archive has no "later" in it. `snooze_e2e` drives the stub to `Running`
(refused, in those words, the session untouched), then `Idle` through a `Stop` frame (snoozed,
the record `Sleeping`, zero awake).

**The board had a gate of its own, and it was the archive's.** `Verb::SnoozePrefix` refused
to ARM over any awake session through `archive_gated`, so the daemon's new road was
unreachable from the keyboard. It now asks `App::snooze_blocked`: a paned claude that is not
`Idle{..}` refuses at the first press in the daemon's words (the chord never arms for an Enter
that would only be refused); what the board cannot judge — a shell's live children, a pin —
is the Enter's to hear. The test fake walks the same road (`Sleeping` on the idle, the refusal
on the working). `snooze_refuses_a_working_claude_before_it_arms` and
`snooze_sleeps_an_idle_claude_on_the_way` pin both.

**And the armed card blinks** (user, minutes later: "flash while in snooze not confirmed yet").
Between the `z` and the Enter or the cancel the title borrows the MOVE ghost's blink —
`Theme::move_blink`, `sel.base` ↔ `sel.dim3` on the four-frame clock, bold both phases — on
the board card and on the ticket page's title row (in the band's own ramp there). Not the
delete's red-tinted flash: that is a deletion's, and a snooze is a card in hand, about to
leave. The preset row under the title holds still. Goldens are colourless, so
`board_snooze_armed_120x30` did not move; `test_snooze_armed_blinks_the_card` sweeps
`Flavor::ALL` (bright at frame 0, dim3 at frame 4, bystanders still, the cancel puts the
cursor title back) and `test_snooze_armed_blinks_the_ticket_title` covers the page.

## The bulk sleep moves to `X` (2026-09-04, user: "shouldn't sleep all be Shift+x now that snooze is z? and x is sleep anyway?")

Reverses the letter in "The bulk sleep gets a key, and it is `Z`" and keeps everything else in
it: still board-only, still `prio: 0`, still the menu row's own predicate, still one press where
the chip says `(X ∙ esc)`. The argument that put it on `Z` — shift hardens or forces the same
verb on the same target, and `X` would widen the target — stopped holding the day `z` snoozed the
ticket (T-74): `z`/`Z` then shared neither verb nor target, a bigger break of the shift law than
`x`/`X`, which shares the verb and only widens what it sleeps from the selection's sessions to
the done column's. `N` already forces the verb onto a different note, so "widens" is the same
kind of stretch, not a new one; the doc comment on `shift_stays_on_one_axis` now reads "hardens,
forces or widens". The retired-keys list swaps `X` for `Z`, and the older claim that the
retired `X` "was shift-of-`x`" is dropped — it was `ArchiveAll` ("Board-wide actions have no
keys", 2026-08-31). README's board-keys sentence, which still named `A` and `V` beside it, now
names `x`/`X` and points the archive at the menu. Golden: `menu_suggestions_120x30` (the Sleep
row's key). No daemon or wire change.

## The ask at a sleeping claude wakes it (2026-09-04, user: "ask claude on sleeping agent auto wakes it for the user")

Shift+Enter on the board had three stages ("Shift+Enter asks the agent from the board", "…the
same key ASKS it") and one hole between them: a ticket whose claude was `Sleeping` was neither
an empty seat (the title road refuses to mint a second claude) nor promptable (no pane), so the
key was inert and the footer said nothing. The user's road was `c`, wait for the pane, `q`,
Shift+Enter — three gestures for one sentence, on the commonest shape a parked board has.

**The key now opens the field there too, and the daemon wakes the agent on the way.**
`Verb::Prompt`'s `avail` is `has_ticket && rich_keys` — every stage of the seat — and the hint
says the extra thing it does: `wake + ask claude` where `ticket_has_claude && !ticket_promptable`
(live and paneless is exactly Sleeping, the same fact `c` reads for `wake claude`). Nothing else
on the TUI moved: the same field on the same card, the same `PromptSession`; the status says
`woke claude ∙ asked`, or `nothing to resume ∙ started a fresh conversation ∙ asked` when the
wake's `fresh` says the transcript was gone — the words `c` uses, because it is `c`'s road.

**The daemon's road is `prompt_sleeping`**, taken when `prompt_target` finds no pane and
`live_claude` finds a `Sleeping` record: `resume_session(id, false)` with every guard it has
(the double-resume refusal, "running elsewhere", the missing cwd, the fresh conversation under a
minted id), then the prompt PARKED in `Daemon::pending_prompt` and `pending_submit` set on the
record so the card wears the launching arc. The words are not typed ahead into the pty the way
the composer's title is: a pty is in canonical mode until Claude sets raw mode, and canonical
input is capped at 1 KiB (`MAX_CANON`), where a prompt is 4 KiB. So the `SessionStart` edge —
which now fires the deferred road on `Resume` as well as `Startup`, since a wake is a `--resume`
and an in-app `/resume` in a pane that owes nothing is a no-op under the flag — only starts the
retry clock, and the FIRST tick (`SUBMIT_RETRY_MS` later) delivers through `paste_text`, the
bracketed paste + separate Enter a live pane is known to take (T-5); the ticks after it are the
ordinary Enter retries to the `UserPromptSubmit` ack. The 500 ms gap is T-5's correction
applied to a paste: a lost Enter is re-pressed for free, a lost paste is the user's words gone.
`pending_prompt` sits beside `submit_retry` in memory, for its reason — a restart drops the
offer rather than pasting into a pane it no longer understands — and `clear_pending_submit`
drops both.

`prompt_e2e` grew the case: the stub driven to `Idle` through the hooks, slept by `SleepSession`,
asked with a 2.5 KB prompt — `Spawned { fresh: true }`, the record paned with `pending_submit`,
NOTHING in the stub's receipt for 1.5 s, then a `SessionStart{resume}` frame, then the whole
prompt on one line, then the ack clearing the flag. The TUI fake answers `PromptSession` on a
sleeper with `Spawned` so `shift_enter_on_a_sleeping_claude_wakes_it_and_asks` sees the status;
`prompting_a_parked_claude_says_it_wakes` replaced the keymap test that pinned the hole shut.

**The e2e found two things on the way.** First, a race that `x` then `c` had all along: `sleep_one`
SIGTERMs and returns, the record is `Sleeping` before the pane has died, and a wake a moment later
spawns a new pane under the same sid16 while the old pane's `pane-died` notify — which carries
only the session name — is still climbing the hook socket; landing on `Spawning`, it read as the
NEW pane crashing. `Daemon::pane_reborn` now drops a `PaneDied` frame for a `Spawning` record
whose pane tmux lists as alive — "listed and not dead" is the one answer that refutes a death,
and only the just-born window is ambiguous, so only there is tmux forked. Second, the stub itself
measured the canonical cap: `sh`'s `read` on a tty in canonical mode got 1 KiB of the 2.5 KB
paste and never the newline, so the stub now runs `stty -icanon` first to stand in for a raw-mode
Claude — the number the "never typed ahead" decision above rests on, seen rather than cited.

**Not done:** the editor's `^S` on a note still leaves a Sleeping claude inert (CLAUDE.md, "a
Sleeping claude leaves it inert (`c` wakes it)") — the same road would serve it, with
`NoteToAgent`'s sentence parked instead of the user's. The wake is not measured against a real
`claude --resume` yet: the paste-on-first-tick shape is T-5's live-pane result plus the
correction's cadence, not a fresh arm of the spike.

## The preferences move into a Settings submenu (2026-09-04, user: "settings submenu in main menu")

The Esc menu had grown to thirteen rows on a busy board, and three of them were not actions
at all — `Show agent replies`, `Snooze returns with needs-you`, `Theme: graphite` — toggles
and a picker sitting between `Archived tickets` and `Release notes`. They are now behind one
row, `Settings`, whose detail names what it hides (`theme: graphite ∙ agent replies ∙ how a
snooze returns`) so nobody opens it to find out.

**What was built.** `Scope::Settings` (word `SETTINGS`, parent `Global`, `Scope::ALL` index 14
— `Releases`/`Input`/`Editor` shifted one) with the menu's three bindings, the last hinted
`back` rather than `close`; `Verb::Settings` and `Mode::Settings { idx }`;
`keymap::SETTINGS_ITEMS` / `settings_items(ctx)`, the three rows moved verbatim out of
`MENU_ITEMS`, theme first. They stay `MenuItem`s so `ui/menu.rs::draw_list` draws both lists
through one function — the menu's `draw` and `draw_settings` are two calls of it with a name
and a scope. `chrome::dialog_open` counts the new mode, so the footer under it carries only the
chip and the right cluster.

**Two behaviours differ from the menu proper, on purpose.** A settings row is a toggle or a
picker, so `App::act(Scope::Settings)` dispatches WITHOUT leaving: the row relabels itself
(`Hide agent replies`) and the change is on the screen. `Verb::Peek` and `Verb::SnoozeQuiet`
lost their `self.mode = Mode::Normal` — the menu's own `act` had already set it before either
ran, so nothing else read those lines. And the theme picker is now the third level, so it pops
to the SECOND on both roads: `commit_theme` and `back(Scope::Theme)` land on `Mode::Settings`
at the theme row (`App::settings_row`), and `back(Scope::Settings)` lands on the menu's
`Settings` row (`App::menu_row`) — `q`/Esc pop exactly one level, as everywhere. `App::tick`
clamps the settings cursor the way it clamps the menu's.

**Held apart.** No settings row is a suggestion: `every_suggestion_is_a_menu_row` now asserts
`SUGGESTIONS` never points into `SETTINGS_ITEMS` (a chip's `(esc)` route takes one Enter and
would land on the wrong list). `menu_holds_the_board_wide_actions` asserts the three
preferences are in `settings_items` and NOT in `menu_items`; the key-spelling and
label-spelling checks run over both lists. TUI test
`settings_is_one_level_under_the_menu_and_a_toggle_keeps_it_open`; the picker tests walk
through `open_settings`. Goldens: `settings_120x30` new; the three menu goldens are four rows
shorter and sit lower on the board.

**Not done.** Nothing conditional lives in the settings list yet, so `settings_items`'s filter
is the menu's for symmetry only. The picker's own frame still says `THEME ∙ for a dark
terminal` and nothing about the level above it; the `esc put it back` hint is the whole
promise.

## The week starts on the user's day (2026-09-04, user: "add in settings - first day of week")

The snooze ring's last rung was `next Monday 9:00`, hard-coded — and the author's week starts
on Sunday. The day is now a preference, and the rung follows it.

**What was built.** `core/src/snooze.rs::Weekday` — `Monday` (default), `Sunday`, `Saturday`,
the three first days in use anywhere, each carrying its `struct tm` `wday` so `target` is one
`rem_euclid` with no table. `Preset::NextMonday9` is `Preset::NextWeek9`; `label(week_start)`
and `target`/`deadline` take the day, and the label STAYS a `&'static str` (three literals for
the last rung, `hint_for_label` knows all three) so the keymap's hint rule holds. `prefs.json`
gains `week_start: "monday" | "sunday" | "saturday"` under the same rules as the snooze flag:
absent reads as Monday, a day this build does not know reads as Monday and is NOT written over
by a theme pick (only a pick of the day replaces it). The Settings list's fourth row, `Week
starts on Monday` (`Verb::WeekStart`), cycles the ring on Enter and relabels itself; its detail
spells the rung it changes (`z's last rung: next Sunday 9:00`). `Ctx::week_start_word` is the
day's name (a fixed set, so a `&'static str`); `App::ctx` sets it and a bare `Ctx` falls to
Monday in the label. The Settings door's detail names it (`… ∙ snooze ∙ week start` — it had
to shorten to fit the 62-cell dialog with the longest theme name). `mesimon doctor`'s `snooze`
line names the day. Nothing crosses the wire: the daemon only ever saw an absolute `until`.

**Tests.** `next_week_is_always_ahead_whichever_day_starts_it` walks all seven weekdays for
each of the three starts and asserts the landing day IS the start; the localtime round-trip
asserts the real `mktime` lands on that `tm_wday`; `the_week_start_is_a_ring_with_a_file_spelling`
and `the_week_start_defaults_to_monday_and_round_trips` pin the ring and the file; TUI test
`the_settings_row_moves_the_start_of_the_week` cycles the row, walks `z` to the last rung and
reads the archived deadline back as a Sunday 09:00. Goldens: `settings_120x30` grew a row; the
three menu goldens for the door's detail.

**Not done.** Only the three days; a fourth is a variant plus a literal. The wake HOUR (09:00)
is still a constant, and "tomorrow 9:00" would be the row beside this one if it ever moves.

## A stated Stop commits through the flap pin (2026-09-04, user: "it stayed in review and kept running indication while it actually stopped and now it's indicating interrupt")

The deferred line under "The recordless Esc is caught…" — "the flap-pin interaction … should stop
being reachable in normal use, but nothing asserts it" — was reachable from the board in twenty
seconds: four Shift+Enter asks at one agent, each a two-second turn. `UserPromptSubmit` → Running
and `Stop` → Idle{EndTurn} are two commits an ask, so the fourth ask's Running was the fifth
commit inside `FLAP_WINDOW_MS` and armed the pin at `Confidence::Low` (T-208's feed, 12:17:10).
Three things followed, each the pin's: `automove` refused the move to IN PROGRESS on Low, so the
card stayed in REVIEW; the `Stop` two seconds later was DROPPED — the pin let only terminal
transitions through — so the card kept the spinner on a finished turn; and when the pin lifted
twenty seconds on, the status-file probe found `Running` with nothing pending and demoted it to
`Idle{Interrupted}` at Medium (12:17:34), which is the "interrupt" the user saw with no Esc
pressed.

**The guard was written for evidence that argues with itself** — 11 §11.7.4's world of OSC
tiers, transcript tails and byte-silence probes — and it was treating Claude's own hooks as more
of it. A `Stop` is not an inference: it is the turn ending, whatever came before. So the pin now
keys on confidence, which is what already separates the two kinds of signal: while pinned, a
signal at `Confidence::High` (every hook-stated transition) commits and keeps High, a terminal
one commits as before, and everything Medium or Low — `PaneQuiet`, `StatusFileIdle`,
`TranscriptHint`, the `SubagentStop` promotion — is dropped for the pin's twenty seconds. The
commit that crosses `FLAP_MAX` still arms the pin, so the inferred signals stay out while the
hooks are this busy; it just no longer marks a stated commit Low, so `automove` moves the card
the prompt asked for. The movegate's fuse (6 automatic moves of one ticket in 120 s, cleared by
a move by hand, noticed in the advisory row) is the honest limiter on that road and was
untouched. `flap_guard_pins_the_inferred_signals` replaces `flap_guard_pins_low`, and
`a_stated_stop_commits_through_the_flap_pin` replays the feed: fifth commit High and pinned,
the Stop settles to EndTurn at High, the probes after the pin have no `Running` to demote.

The same feed shows the wake road's race one build earlier (12:07:04, `spawning → exited
{crashed}` off a `PaneDied` two seconds after the ask, before `pane_reborn` existed): the
record recovered on the `SessionStart{resume}` that followed, and that binary is gone.

## A board with no tags is offered three (2026-09-04, user: "creating first tag gets people overwhelmed. seed group 1 with tags BUG FEATURE CHANGE with appropriate colors if no tags exists")

**What changed.** The registry no longer starts empty on a board that never had a tag. `store::load`
calls `Board::seed_starter_tags`, which writes `board::STARTER_TAGS` — `BUG` (tint 0), `FEATURE`
(tint 2), `CHANGE` (tint 6): rose, green and blue on the shipped graphite/chalk ring — onto group
1 (`STARTER_GROUP`) when the board has no tags, and in every case stamps `Board.tags_seeded`.
The stamp is persisted in `columns.toml` as `tags_seeded = true`, a scalar declared before the
tables (the same TOML rule `schema_version` and `next_key` obey). No schema bump: an older build
drops the key and would re-offer on the next newer load only to a board that is STILL empty,
which is the offer it would have made anyway.

**What "if no tags exist" means.** Once, not whenever. The naive rule — seed at every load that
finds an empty registry — would answer a user who pressed `d` on all three with the three coming
back on the next daemon start, so the offer is a stamp (the same instinct as the dev-channel
gate: "a stamp, never a heuristic"). A board that already has a vocabulary is stamped without a
write to its tags; a board written before today with no tags gets the offer on its first load
under this build, which is the whole point — the overwhelmed user has such a board.

**What was refused.** Seeding in `Board::with_default_columns()` would have reached only a
repo whose `columns.toml` did not exist yet, missing every existing board; seeding per flavor is
impossible because the colour is a ring INDEX in the registry and the flavor is a TUI preference
— the rings differ in hue order per flavor (blue's index 0 is a green), so the three are tuned
for the default pair and are merely distinct elsewhere. Deriving the colours from the name hash
(`color: None`) was refused because `default_tint("BUG")` is whatever the hash says, and the
request was for appropriate colours.

**Tests.** `store.rs`: `a_fresh_board_gets_the_starter_tags_once` (three on group 1, chosen
colours, the stamp in the file, and a cleared registry stays cleared on reload),
`an_existing_board_is_offered_the_starters_only_when_it_has_no_tags` (pre-stamp file with a
vocabulary is only stamped; without one is seeded, `next_key` untouched),
`the_seed_can_be_declined_for_a_test`. `board.rs`:
`the_starter_tags_are_lawful_and_offered_once`. The seam `MESIMON_NO_TAG_SEED=1` exists for
`tags_e2e` and `mcp_e2e`, which assert the registry's exact contents after building it from
nothing (`allowed_tags == []`, `group_tags(1) == ["BUG", "REGR"]`); `store::load_with(paths,
seed)` is the same switch for a unit test, so no test touches the environment. The board
goldens never load a store, so none moved.

**CLAUDE.md** paragraph "Tags are ticket metadata on an axis" now says so, replacing "Nothing
is seeded".

## The reload waits for the daemon it asked to stop (2026-09-04, user: "I can't access mesimon board due to a bug after I hit U")

**What happened.** A `U` reload landed while another session's `cargo nextest run --workspace`
(which is what had relinked the binary and raised the chip) was running every e2e in parallel.
The TUI asked the daemon to shut down, waited 2 s for `orch.sock` to vanish, exec'd the new
binary; the fresh client dialled for 5 s, spawning a daemon every 400 ms, and each one lost the
old daemon's flock and exited quietly; the client bailed, the TUI printed one line and exited,
the old daemon finished its shutdown — and the repo had twelve live sessions on its private
tmux server and no board and no daemon. `daemon.lock` still named the dead pid (a flock loser
never writes it) and `daemon.log` had never had a byte (no daemon crashed). Same binary, same
inode, came up fine by hand two minutes later; the reload two rebuilds earlier, on an idle box,
had worked.

**Why.** Three stopwatches and no clock on the thing they were timing: `reexec` gave the
shutdown 2 s, `connect_or_spawn` gave the next daemon 5 s and counted the handover against it,
and `begin_shutdown` commits every pending settle through `apply_change` — automove, worktree
flag refresh (git forks per bound worktree, eleven of them here), persist — on a box that was
at that moment forking a hundred tmux servers. The client's request to the old daemon has a
10 s timeout of its own; `Verb::Reload` ignores its result on purpose.

**What holds now.**
- `client::await_daemon_gone`: socket unlinked AND lock released, up to `HANDOVER_MAX` (30 s),
  a sentence on the (already restored) terminal after 1 s; `reexec` calls it. A wedged daemon
  still gets the exec — a client that will not start improves nothing.
- `connect_or_spawn` probes `daemon.lock` with `LOCK_SH|LOCK_NB` (released at once; it never
  claims the role). While held — the old daemon finishing, or the one we just spawned between
  its flock and its bind — nothing is spawned and the budget does not run; that wait is bounded
  by `HANDOVER_MAX`. A free lock re-arms the budget for the next daemon.
- `Client::connect` no longer fails the launch on an unreachable daemon: the client comes up
  with `conn: None` and a `daemon_down` notice carrying the reason, `App::new` opens on a
  default `Snapshot` with `note_daemon_down()`, and the existing 2 s reconnect cadence (whose
  reopen respawns) takes it from there. A bad repo path is still fatal. Test:
  `no_daemon_at_launch_opens_disconnected`; `lock_probe_follows_the_holder` pins the probe.

**Not done.** The shutdown itself is still unbounded and still on the writer thread; the
handover is now patient with it rather than fast. Bounding it (skip the worktree flag refresh
on the shutdown road, say) is the next lever if 30 s ever proves short.

**Addendum 2026-09-04 — a flock release is prompt, not instantaneous, and the test said the
wrong one of those.** `lock_probe_follows_the_holder` flaked four times in a day across two
sessions, always on its last assertion (`release reads free`), and never reproducibly: 400
sequential runs clean, 300 concurrent runs of the test alone clean, 40 loaded full-suite runs
clean — then 3 failures in 120 runs at `--test-threads=64/128`. Thread interleaving was the
knob, so it was an in-process race, and instrumenting it ended the guessing: at the moment of
failure `lsof` found NO holder, and the very next probe read free. **The lock was held by a
process that no longer existed.** A `fork` elsewhere in the binary (`release.rs`'s install test
runs a `Command`) duplicates every open descriptor into the child, and a duplicate of the test's
`holder` fd carries the same open file description and therefore the same flock — so the
parent's `close` does not release it, and the lock outlives the drop until the child reaches
`exec` and `O_CLOEXEC` fires. Microseconds wide, which is why it needed 128 threads to hit.
The test now polls for release against a 5 s deadline, which asserts the thing that is true
(the release lands) rather than the thing that is not (it lands within zero microseconds of
`close`). **`lock_held` itself is unchanged and correct** — it reports `EWOULDBLOCK` and nothing
else, and both callers poll, so a stale `held` costs one more turn of a loop already turning.
The TUI does fork in production (the external editor's handover, the release checker's `curl`
and `tar`), so the window is reachable there too and is equally harmless.


## The board's ask can wait for a quiet checkout (2026-09-04, user: five claudes in one checkout committed at once)

**What happened.** Five claudes on five tickets, all in the SHARED checkout, asked to commit via
the board's Shift+Enter within a minute. One checkout has one index: git's lock serialises the
index write, not the staging, so agents swept each other's hunks, ran tests over each other's
half-edits and regenerated each other's goldens. Nobody can know which hunk is whose — not
mesimon, not the agents. The correct shapes are separate trees (worktrees, M4a) or one writer at
a time. Mesimon refuses the third shape: no Bash hook to block a commit (the README's reasons
stand) and no "wait your turn" sentence in anybody's conversation.

**What was built.** Inside the ask field Shift+Tab cycles `now` / `queued`, on a row under the
field in the composer's own shape (`card::render_ask_mode`; the one `BackTab` binding in
`Scope::Input` widened, since a key is bound once per scope). It is offered only where waiting
means something — `Ctx::ask_queueable`: a shared-checkout ticket with an awake pane, never a
worktree (its checkout is its own) and never a Sleeping claude (waking it at delivery time would
spawn into the checkout just judged quiet). `PromptSession { queued: true }` parks the words in
the daemon's in-memory `queued` list — `pending_prompt`'s argument: a restart drops them — and
`drain_queue` pastes the first waiting ask of every QUIET checkout: no claude with the same `cwd`
in Spawning / Running / RequiresAction / Idle{Background} / `pending_submit` / a paste of ours
still owed its ack (`core/src/quiet.rs::working_tickets`; a shell never counts, D15 pins it
Running for life). Hooked beside `auto_move` in `apply_change` (the EndTurn settle and the
shutdown flush both come through it, so the words go out on the way down), on the 1 s bucket as
the net, and at enqueue (a quiet checkout sends at once, `Ok` not `Queued`). One paste per
checkout per pass: the in-flight marker keeps the checkout busy until the agent's
`UserPromptSubmit`, which the daemon cannot tell from the user's own keystroke — so the NEXT
prompt on the ticket is the ack either way, and an ask still WAITING when a prompt lands is
dropped: the user talked to the agent ahead of it (pane typing, send-now, `MergeToAgent`,
`NoteToAgent`). Sleep, kill and delete drop it by name; the sweep catches the rest (target
gone, replaced, parked, ticket archived); a hand move never does. One entry per ticket; a second
replaces. `Response::Queued { behind }` names the holders.

**What the board shows.** `Response::Board.pending: Vec<Pending { ticket, action, waits_on,
text, in_flight }>` — kept general, the merge train's rows ride it too. Every owed card wears
`glyphs::queued` (`⠑ ⠢ ⠔ ⠊`, the half-diagonal pairs no other table uses, ascii `( )`) on the
SLOW cadence over a still or empty glyph slot only (`queued_over`); the cursor card's accordion
carries `queued ∙ after T-12` (`+N`, `after its turn`, `sends next`, `sending`) in the snooze
row's slot, and the ticket page's state row the same words. Shift+Enter on a queued ticket
reopens the field on the words at `queued` (hint `edit the queued ask`); Esc keeps it, a blank
Enter drops it (`DropQueuedAsk`; the emptied field's placeholder says `enter drops`, since T-241) — no Esc-menu row, the menu
is for things not about the selection. E2e `ask_queue_e2e`.

**Not done.** Persistence across a restart; per-column policy (M5). *(The wake-then-ask road
landed in T-294, below.)*

## The merge train: mesimon merges and asks to rebase while the board is quiet (2026-09-04, user: "worktrees deserve automation")

**The deviation, on purpose.** docs/12 invariant 9 says never auto-rebase, auto-merge or
auto-push, and §12.5.5 reserved `keep_up_to_date = never | offer`. The train is OPT-IN (Settings
row `Merge train`, `prefs.json::merge_train`, off by default), only ever fast-forwards, only ever
ASKS the agent to rebase (it never runs `git rebase`), and never pushes. README promise 3
narrows by one clause: the two merge-flow sentences, which travelled on a per-press human
gesture (`m`), may now travel on a STANDING instruction — that row — and `mesimon doctor`'s
`merge train` line says when it is on.

**Armed by a connection.** The daemon reads no preference file; the TUI pushes
`Command::SetAutomation { merge_train, merge_notice }` on every toggle and from
`App::reconcile_train` whenever a snapshot reads the daemon unarmed while the preference is on
(the first snapshot, a daemon restart, another board's train having gone; 30 s back-off; a
preference that is OFF pushes nothing, so one board never disarms another's). The daemon holds
it in memory (`daemon/src/train.rs`, the movegate's sibling) tied to the arming client's writer
`Arc` — held weakly, so `is_armed` reads false the moment the reader thread returns, and
`Msg::ClientGone` (sent at the end of `client_loop`, on the same `tx` as its requests) prunes
the subscription and says `merge_train_disarmed` in the feed. A closed board is a stopped train.

**The pass.** `train_pass` runs right after `refresh_worktree_flags` on its bucket
(`MESIMON_WT_REFRESH_TICKS`, default `RSS_TICKS`; the flags moved off the RSS clause so the seam
is theirs alone), only while `board_busy()` is empty — the same predicate the queued ask uses,
board-wide — and does ONE thing. First a merge: the first candidate of `core/src/train.rs::plan`
in board order (column, then row — automove parks at the top, so the last ticket to finish goes
first): a REVIEW ticket, attached, no conflict, ahead and fast-forwardable, whose claude is
`Idle{EndTurn}` at High|Medium or absent (nobody to wait for — what a hand `m` would merge);
through `merge_ticket` under `Principal::Automation { rule: "merge_train" }` (both merge roads
take a principal now, and `authorize`), then the merged notice into its agent if `merge_notice`
is on (a turn starts; the next pass waits for it). A refused merge (a dirty main) is remembered
per `(branch tip, base tip)` and not retried every bucket; the detail rides `Pending.text`.
Else ONE rebase ask: an IN PROGRESS or REVIEW ticket the base moved past, claude idle, not fused,
not already asked at THIS base tip (`Train::asked`, recorded by hand asks too — a hand `m`
stops the train re-asking), and its pane silent ≥ 5 s by `#{window_activity}` — a person
mid-sentence there must never get mesimon's appended to theirs. Fuse: 6 train asks on one
ticket in 2 h suspends it (`Notice merge_train_suspended`), cleared by a hand `m` or a hand
move, like the movegate's. Merged tickets do NOT move.

**Also.** `merge_ticket`'s quiet gate counts CLAUDE sessions only now: a `!` shell is pinned
Running for life and an ff-merge never touches the worktree, so it had refused every hand `m`
under one. `AutomationStatus.train_asked` lets the ticket page read `rebase requested` after a
train ask without a TUI-local memory (`merge_outstanding`). The cards carry
`merge ∙ after T-3 +1` / `rebase ask ∙ next` on the owed row. E2e `merge_train_e2e`.

**The header word is cut (2026-09-04, the next day).** ` ∙ train` hung off the git clause while
armed, was renamed ` ∙ auto-merge` on the author's ask ("rename train to something more
indicative"), and was removed an hour later on their second reading: *"it's applying only to
worktree tickets and not globally really."* The header is the BOARD's row — it sits beside the
checkout's own branch and change count — and the train only ever merges an attached worktree
ticket, so any word there overstates its reach; the rename made that louder rather than fixing
it, since `auto-merge` beside `⎇ main` reads as a promise about main. The two halves already had
homes: the Settings row says whether it is armed, the card's `merge ∙ after T-3 +1` row says what
it will actually do, and `mesimon doctor` prints the line. `golden_train_120` now asserts the
INVERSE — an armed train adds no word to line 0 — so the clause cannot come back by accident.

**Not done.** A merge train that moves merged tickets to DONE (M5's column policy); the merged
notice bouncing the card through IN PROGRESS and back (automove's, and today's hand notify does
the same); a per-repo preference (it is per machine, like the rest of `prefs.json`).

## mesimon offers the CLAUDE.md line, behind a confirm dialog (T-217, 2026-09-04, user: "suggest claude.md edits to the user ∙ suggest to auto apply them, keep minimal, short and only if started with mesimon")

**The bug.** A session mesimon spawns is told which ticket it is on twice: `MESIMON_TICKET`
in the pane's environment tells the SHELL, and `get_ticket` tells the MODEL
(`server.rs::session_vars` names the two layers). Nothing told the model to USE the second
one — and the prompt it is handed is often only the ticket's TITLE, since the composer's
Shift+Enter, `start_composed` on an empty seat and a wake-and-ask all submit the title while
the description lives in `notes[0]`. So agents skipped the description and users typed "read
ticket for more context" into every prompt by hand. The user's words: *"sometimes claude
doesn't read ticket for more context, it's not reliable. users write description but claude
just skips it."*

**The snippet, and the sentence it does NOT say.** `core/src/claudemd.rs::SNIPPET` is four
lines under a `## mesimon` heading, and the wording that matters is *"the ticket's description
and notes may carry context the prompt does not"* — never "the prompt is only the title".
Only the composed spawn submits the title alone; an ask field or a prompt typed into the pane
is the user's own words, so the stronger sentence would be false on the commonest road, and a
CLAUDE.md that is wrong once is disbelieved everywhere (the user caught this: *"prompt *may*
be only the title, there *might* be more context in description"*).

`MESIMON_TICKET` doubles as the MARKER. Using the snippet's own subject as its signature means
there is nothing to keep in step: applying twice is impossible, a user who wrote the
instruction in their own words is never nagged, `.claude/CLAUDE.md` counts as much as the
root's, and mesimon's own repo is correctly offered nothing.

**Authored at 56 columns, and that is a law.** `dialog::MAX_W` is 64, so the dialog's inner
width is 62. A dialog that re-wrapped the text would not be showing what it writes — which is
the whole promise of a confirm dialog over a file the user tracks in git — so `SNIPPET` is
hard-wrapped to `claudemd::WRAP` and `the_snippet_fits_the_dialog` holds the two numbers
together. Edit the text and the test says whether the dialog can still show it.

**The one modal confirmation in mesimon.** Every other confirm is a chord tail (`d`, `a`, `z`)
or the `m` key's `Class::Arm`: they put the question in the status line and draw nothing, and
none of them can show four lines of text. `Mode::ClaudeMd` / `Scope::ClaudeMd` is drawn through
`ui/dialog.rs::frame` like the five list dialogs, but it is a question rather than a list, so
it has no `idx` and its answers are its keys: `enter` add it / `create it`, `c` copy, `i` never
ask again, `esc` not now. Enter and Esc reuse the list dialogs' own `Act`/`Back` (with a
`Scope::ClaudeMd` arm in each), which is what keeps "one verb per key across screens" true.

Two things the build got wrong first and are worth recording. The three answers were written
as `Group::App` — semantically right, structurally wrong: `footer_split` partitions on
`group != Group::App`, so they landed in the FOOTER's right cluster instead of the frame's
bottom edge. They wear `Group::Sessions` now, the drawer's precedent (a dialog's keys take the
group of what they act on, and these act on what every future session is told). And
`chrome::dialog_open` had to learn the mode, or the footer repeated the edge underneath it.

**The chip says what you GET, and the row says what it touches.** They shipped the other way
round for an hour — chip `claude.md misses the ticket line`, row `Teach CLAUDE.md to read the
ticket` — and the author cut it ("doesn't indicate well"). Three things were wrong at once: the
chip spent its width naming a file the reader has no reason to care about yet, "the ticket line"
is mesimon's own jargon and means nothing until you have seen it, and "misses" reads as longing
as readily as absence. The row's label was a category error besides — a CLAUDE.md cannot read a
ticket, an agent can. Now: chip `tell agents to read the ticket`, row `Tell agents to read the
ticket` over `adds four lines to CLAUDE.md ∙ you see them first`. The file is named one line
down, and shown in full a keystroke later.

**Decline, copy and ignore are three different answers.** `esc` writes nothing and the offer
returns; `i` stamps `Board::claude_md_ignored` and it never returns; `c` writes nothing and
stamps nothing, and is the one key that leaves the dialog standing — copying is not evidence
of pasting. "Never" is affordable because `mesimon doctor` prints the snippet whatever the
stamp says, which is the door the stamp does not close (the user asked for exactly that:
*"doctor always suggests"*).

**Copying is OSC 52, and it can silently do nothing.** No clipboard support existed: no crate
in any manifest, and the only copy code in the workspace is the tmux conf's
`pbcopy`/`wl-copy`/`xclip` pipe, which serves tmux's own copy-mode inside an agent pane and is
unreachable from the board. A native call would also be wrong here specifically — the board is
what people run over ssh, and `pbcopy` on the far side copies to a clipboard nobody is looking
at. `osc.rs::copy_to_clipboard` writes `ESC ] 52 ; c ; <base64> BEL` with a hand-rolled
encoder (~20 lines; the workspace pins one major of everything and CI fails on a duplicate, so
a crate is never free). The sequence is write-only: a terminal may refuse it and an outer tmux
swallows it without `set-clipboard on`, and mesimon cannot tell success from refusal — so the
status says `snippet copied ∙ if your terminal allows it` and the text stays on screen.

**The MCP switch, because an offer to call a tool that is off is noise.** The user asked for
the opt-out in the same breath (*"if MCP is disabled, don't suggest (we need MCP enable /
disable in configuration, opt out)"*). `Board::mcp_tools` is PER REPO, in `columns.toml`
beside `tags_seeded` — "may agents on this board see their ticket" is a property of the board
— and because `Response::Board` clones the whole `Board`, it reached the TUI with no wire
change at all. `claude_argv` (the one argv builder since T-84) omits `--mcp-config` entirely
when it is off: not an empty config, not a server with no tools, because a session that was
never told about mesimon cannot be told about it later. `resume_argv` needed BOTH halves — drop
the pair when off, and INSERT it when on and the persisted argv lacks it — since a record born
while the tools were off has no flag to rewrite, and "wake it to pick the setting up" would
otherwise be true in one direction and a lie in the other. A live pane keeps what it was born
with; that is the sentence the Settings row spends its detail on.

**`COLUMNS_SCHEMA` 2 → 3, and the reason is sharper than `tags_seeded`'s.** That stamp rode a
serde default with no bump because an older build dropping it only re-offers three tags.
Dropping `mcp_tools = false` is different in kind: an alpha.13 binary would silently hand every
agent on the board its tools back after the user took them away. A consent flag may not be lost
to a downgrade, so the bump makes an older build bar its writes instead — which is what schema
2 already exists for. `claude_md_ignored` rides the same bump. `Board`'s `Default` had to be
hand-written for one field, since `#[derive(Default)]` would have started `mcp_tools` off and a
default `Board` is what an UNREADABLE `columns.toml` falls back to — a corrupt file must not
read as a user's choice.

**Writing a file the user tracks in git.** `daemon/src/claudemd.rs::apply` copies
`paths.rs::ensure_excluded`'s shape (read, look for the marker, append, write) with two
additions. It CANONICALIZES first: a CLAUDE.md symlinked into a dotfiles repo is ordinary, and
`write_atomic`'s temp+rename would swap the LINK for a regular file. And it is still atomic,
because truncating the user's own tracked file and then crashing would cost them work that is
not mesimon's to lose. Sampling is behind an mtime+len `stat` gate (`claudemd::Sampler`): a
CLAUDE.md is commonly tens of kilobytes — mesimon's own is ninety-five — and a snapshot happens
on every board change.

**`doctor` prints the snippet, and `wrap` had to learn paragraphs.** Doctor's promise is
copy-pasteable fixes, and the advice renderer reflowed everything on whitespace, which turned
the snippet into prose. `wrap` now wraps one paragraph at a time and passes through any line
already inside the measure — a fix meant to be pasted verbatim stops being one the moment its
newlines are lost. `agents()` takes `&repo` now (`git_section`'s shape) and reads the switch
through `store::read_mcp_tools`, a read-only accessor added because `load` SEEDS and WRITES
`columns.toml` for a repo that has none, and doctor may not create a board to answer a question
about one.

**What is NOT here.** Submitting the ticket description alongside the title at spawn. It is
smaller and it is not injection — the description is the user's own words — but it reaches only
the composed-spawn road, while a wake, a resume, an adopted session and a hand-typed prompt
learn nothing, and a description edited after the spawn is never seen. The CLAUDE.md line makes
the agent fetch the CURRENT description on every road. Worth doing later as a complement; it is
not a substitute.

**README promise 1 was short by a third clause, and the author closed all three the same day**
— see "The write allowlist says what it does" below. Promise 2 needed nothing: doctor still
only prints and has no `--fix`, and the write is a keystroke through a dialog showing the exact
bytes.

**Tests.** `core/src/claudemd.rs`: the snippet names its own marker and the tool, every line
fits the dialog, applying twice is impossible, the separator is one blank line however the file
ended. `daemon/src/claudemd.rs`: the offer and the write on a repo with no file, applying twice
writes once, `.claude/CLAUDE.md` withdraws the offer, an unchanged file is not resampled, a
symlink is followed not replaced. `daemon/src/store.rs`: a v2 file reads as tools-on and
never-ignored, the default board has the tools on, both scalars round-trip above the tables.
`osc.rs`: RFC 4648's vectors, and the snippet decoded back. `ui/tests.rs`: golden
`claude_md_120x30`, the snippet on screen verbatim, the four answers in the edge, all four
clauses of the offer (including that an EMPTY path is an unknown, not a no), and the chip and
its menu row wearing the same mark. `claudemd_e2e.rs` end to end plus the restart; `mcp_e2e.rs`
proves a real spawn carries no `--mcp-config` with the switch off, and that `--settings` is
untouched — the two flags are different promises. `tags_e2e.rs`'s `schema_version = 2` pin now
reads `store::COLUMNS_SCHEMA` instead of a literal.

## The write allowlist says what it does (2026-09-04, user: "fix the readme promise wording for all three gaps")

**What changed.** README promise 1 had been literally false in three places, each recorded and
each deferred because it is a public commitment in the author's voice (auto-memory
`readme-allowlist-gap`, opened 2026-09-01, "open x3" by T-217). The author asked for the pass,
so all three are now worded, plus one clause sharpened and one table row corrected.

**The shape it took.** The promise now separates what mesimon writes ON ITS OWN from what it
writes only when asked, because that turned out to be the real distinction and it was the thing
the old single sentence could not say. The always-list gained the runtime dir; the three
consent-gated writes are a list under it.

- **Gap 1, the runtime dir** (`/tmp/mesimon-<uid>/<proj16>/`): never named, and mesimon has
  always written it — `orch.sock`, `hook.sock`, `tmux.sock`, `daemon.lock`, the release
  checker's `update/` staging, and `shellenv.env`, which is a copy of the user's login-shell
  environment with their secrets in it. The wording says that out loud rather than listing file
  names, and keeps the two facts the memory asked be kept: the dir is 0700 and mesimon refuses
  it unless it owns it, and those permissions are the ONLY thing between another user on the
  machine and the agent tool socket, because there is deliberately no token.
- **Gap 2, mesimon's own binary**: taking an update offer replaces the running binary at its own
  path plus the sibling `mesimon-tmux` where one exists. Named, with the checksum gate — which
  is the reason it is safe to promise, not a detail.
- **Gap 3, `<repo>/CLAUDE.md`** (T-217): named, with the two limits that make it a promise
  rather than a permission — the dialog shows the exact lines first, and nothing already in the
  file is edited or removed (`claudemd::appended` only ever appends).

**Two things found while auditing, and fixed in the same pass.** Every non-test write site in
the workspace was walked to make sure the pass did not leave a fourth gap. It found: the
worktree ownership marker, which lives in the git admin dir of a worktree mesimon created and
was only covered by inference (promise 1 now says "and their git bookkeeping"); and a table row
claiming `~/.local/state/mesimon/<proj16>/` holds "the private tmux socket and conf", when the
socket is in the runtime dir and only the conf is in the state dir. Everything else was already
inside a named path.

**The table grew three rows** (`/tmp/mesimon-<uid>/<project key>/`, `<repo>/CLAUDE.md`,
mesimon's own binary), so "Nothing else. If you ever find mesimon writing outside that list,
that is a bug worth reporting above all others" is a sentence that now holds.

**Promise 2 was not touched and did not need to be.** "No config mutation ∙ `mesimon doctor`
diagnoses and prints copy-pasteable fixes. It has no `--fix`" is still literally true: the
CLAUDE.md write is a TUI keystroke through a dialog, and doctor still only prints. Promise 3 is
unaffected.

**No test moved.** Nothing in the workspace reads the README (checked), so this is prose only —
which is exactly why it was worth being careful: there is no validator standing behind promise
1, only the audit.

## The board diffs its own checkout (T-221, 2026-09-04)

**What the corpus assumed.** docs/08 built the review surface around a branch: every diff is
`BASE...BRANCH` through a `worktree::Binding`, and §2's layer model made an untracked file a
*sighting* — listed, never opened, because "mesimon reads committed state only, so nothing
here can race the agent". That reasoning holds for a worktree, where the thing under review is
a commit. It does not hold for the board's own checkout, which is where most tickets actually
work: `workspace` defaults to shared_checkout, so a ticket's agent edits the repo root and
leaves the work uncommitted, and nothing in mesimon could show it. The header could count it
(`⎇ main ↑2 ∙ 3 changed`, T-124) and then had no key.

**What shipped.** `v` on the board opens the same `Screen::Diff` on `git diff HEAD` — HEAD to
the working tree, staged and unstaged in one row per path. `Command::DiffTarget { Ticket { id }
| Checkout }` replaced the two commands' `ticket` field rather than adding two more commands:
the response, the permit pool, the service function and the whole TUI state are shared, and an
`Option<Ulid>` whose `None` silently meant "the checkout" is the implicit classification
`Command::meta` and `agent_allows` exist to refuse. `Screen::Diff` lost its ticket and became a
unit variant, which its own doc comment had claimed since M4b ("state lives in `App::diff`, not
here"); `App::diff_ticket()` is now the one place a screen asks its state which target it is on.

**Which diff you get is the SCREEN's to answer, never the cursor's.** The board is the
repository's screen and the ticket page is the ticket's, so a worktree ticket under the cursor
does not change what the board's `v` shows — its branch diff is still `space` then `v`. Same
verb on both (`one_verb_one_key_across_screens` covers the key now), different subject, and the
board's binding is `Group::View` rather than `Worktree`: the board has no other Worktree binding
and one here would mint a one-row `BRANCH` section in `?` over a working-tree diff. `avail` is
`Ctx::git_repo` (`RepoGit::sampled`), so the key is inert where there is no repository rather
than answering with git's error.

**The hint sits where it operates, not in the footer** (author 2026-09-04, the same day: it
shipped in the footer for an hour). `prio: 0`, and `chrome::git_clause` draws ` v diff` beside
the checkout's own `∙ 3 changed` — T-158's idiom, the one the ticket rail's `c s x` and the
PREVIEW heading's `{ } page` already use. The footer belongs to the SELECTION and this key is
not about the selection; the header is where the checkout already speaks. It rides the COUNT
rather than the branch name, because `v` opens what is uncommitted: on a clean checkout there
is nothing for it to say, and `?` still lists it. It is also the first rung the clause gives up
when the row is tight — hint, then count, then the name truncates to its floor, and the arrows
are never cut. `hint_spans` spells it like every other hint (bold key, dim word), because "a
key looks like this wherever it is hinted" is what makes one readable off the footer at all.

**An untracked row is an ADD here, and that is what made the change small.** The daemon stamps
it — `status = "A"`, `old_mode = "000000"`, `new_mode` from `symlink_metadata` — and serves its
content from `git diff --no-index -- /dev/null <path>`. Everything downstream then needed no
target condition at all: `diff_fetch`'s `status.is_empty()` skip simply stops skipping them,
08 §2's "untracked — not reviewable" copy is never reached (and stays right for a branch), and
the gutter reads `A` + `U`. The status letter still means exactly "there is no patch behind this
row". Route on `untracked && status.is_empty()`, never on `untracked` alone: `git rm --cached`
emits **both** a `D` record and a `?` record for one path.

**Four mechanical facts, each measured rather than assumed.**

- `git diff --raw HEAD` writes the destination blob as **forty zeros** whenever the worktree
  file's stat differs from the index (git's `diff-lib.c::get_stat_data`). 08 §1.3 spelled
  `ModeOnly` as `old_blob == new_blob && old_mode != new_mode`, which therefore could NEVER fire
  on this range — a worktree-only `chmod +x` fell through to `Text` with zero hunks and rendered
  as an empty diff. `build_file_diff` now says it from the patch: the modes differ, both are real
  file modes, no hunks, no binary marker. The "real file mode" clause (non-empty, not `000000`)
  is load-bearing and not cosmetic — without it an empty new file, an empty deleted file and an
  untracked row would each newly read as a mode change, which is a live regression on the
  BRANCH diff. Three unit tests hold that line.
- `git diff --no-index` **exits 1** when it finds differences. `git_bytes` bails on any non-zero
  status, so the untracked road has a sibling that accepts 0 and 1 and nothing above.
- `status --porcelain=v2 -unormal` collapses an untracked directory to one `? dir/` row, and
  `--no-index` cannot open a directory — so that row could never be read. The checkout list uses
  `-uall`. Ignored files stay out either way (this repo: 1 untracked against 816k ignored), and
  the cost is the same walk.
- The empty tree for an unborn HEAD is asked of git (`hash-object -t tree /dev/null`), never
  spelled: `4b825dc…` is the SHA-1 value and wrong in a SHA-256 repository.

**One `status` call carries three answers** — branch (via `gitstatus::parse`), the HEAD oid, and
the dirty/untracked flags (via `diff::parse_status_v2_z`), read twice over the same bytes. The
oid is pinned from it rather than resolved again, so a commit landing between the calls cannot
leave the range and `base_oid` describing different HEADs.

**Three things fixed on the way past.** `apply_status_flags` was O(untracked × files) against a
`Vec` that grows as it pushes — harmless with `-unormal`'s handful, quadratic with `-uall`, so it
indexes the stable rows first. The branch diff's `--numstat` call omitted `--find-renames` and
rode on `diff.renames` defaulting true, which gave a rename's badge the add's count under a user
who set it false. And `serve_diff` never ran `mcp::agent_allows`: `client_loop` short-circuits
both diff commands before `handle_agent`, and `authorize` allows `(Read, _)` for an agent, so
CLAUDE.md's stated enforcement for D10's never-tier was not true on the path the commands
actually take. It is now — `Principal::Agent` is denied before the permit is taken. (Not an
escalation either way: any same-uid process can claim `Principal::Local`, and the boundary is
the 0700 runtime dir, as designed.)

**Two gaps left open, deliberately, and named in the module doc.** A path in a merge conflict
gives `git diff HEAD` a COMBINED diff (`@@@`), which `parse_hunk_header` reads as zero hunks, so
it renders as "no content change" — the branch diff never meets this because it compares two
commits, and combined-diff parsing is not worth building for it. And an untracked directory git
refuses to descend into (another repository) stays a display-only row, the same shape the branch
diff gives every untracked path. The untracked row count is capped at 2000: a response line has
no length cap on the client side, and one missing `.gitignore` rule should not be able to mint a
megabyte of them.

**Goldens.** Only `board_git_120x30` drifted — the one golden that seeds `RepoGit::sampled`, and
therefore the only header that gains ` v diff`. The five branch-diff goldens did not move, which
is the check that the copy split went in the right place. `diff_checkout_120x30` is new, and both
L1 law sweeps gained a checkout arm — each with a needle assertion, which the existing
`install_diff` arms had never had.


## The brief travels with the title (T-224, 2026-09-05, user: "claude.md prompt for agent to look at ticket are not aggressive enough, most agents skip reading ticket before executing" ∙ "do everything")

**The bug, a day after T-217.** The CLAUDE.md snippet shipped as a hedged request — *"Call
`get_ticket` before you start — the ticket's description and notes may carry context the prompt
does not"* — and agents read it as optional: most still went straight from the title to the
code. Four things were wrong at once, and all four moved:

1. **The snippet is now imperative, first, and still honest.** `claudemd::SNIPPET`: *"FIRST,
   before reading code or planning, call `get_ticket` and read the ticket's description and
   notes: they are the brief, and the prompt is often only the ticket's title. Do not start work
   without them."* It opens on the ORDER of events, names what is missed, and closes the door —
   but it says *often* only the title, never *only*: an ask field or a prompt typed into the pane
   is the user's own words, and T-217's rule stands (a CLAUDE.md that is wrong once is disbelieved
   everywhere). Still five lines under `WRAP` 56; golden `claude_md_120x30` moved; the repo's own
   CLAUDE.md copy was updated by hand (mesimon offers this repo nothing — the marker is there).
2. **`get_ticket`'s description says the prompt is often only the title.** Tool text is the one
   named exception to promise 3 and reaches every spawned session whether or not the CLAUDE.md
   offer was taken, so the sentence *"The prompt that starts a session is often the ticket's title
   alone; the description and notes here are the rest of the brief, so this is the first call of
   a session"* lives there too — descriptive, inside `lint_tool_text` (no second person, no
   imperatives, no shouting) and under `MAX_TOOL_BYTES`.
3. **A prompt cannot be skipped: the composed spawn pastes the description under the title.**
   On every road where mesimon presses the Enter (`SpawnSession { submit_prompt: true }` — the
   composer's Shift+Enter, `^S` in the grown composer, the board's ask at an EMPTY seat), 
   `spawn_session` still types the title (the fallback if the delivery gives up is unchanged) and
   now also parks `notes[0]`'s body in `pending_prompt` as `Parked { text: "\n\n" + body, brief:
   true }`; `deliver_pending_submit` presses nothing on the `SessionStart` edge while a paste is
   parked, and the first tick of `retry_pending_submits` pastes it through `paste_text` — bracketed
   paste, then a separate Enter — the wake-and-ask shape, never typed ahead (canonical-mode input
   keeps 1 KiB until Claude sets raw mode). The plain-Enter road stays title-only: the user is
   about to edit the box, and a 32 KiB description is not editable there. A wake-and-ask parks the
   USER's words with `brief: false`; a description that is blank or unreadable parks nothing.
   **README promise 3 names it**: what is submitted is the title followed by the description,
   both the user's own words written for that ticket, nothing of mesimon's.
4. **The skip is visible where it matters.** `SessionRecord.ticket_read` (`#[serde(default)]`,
   persisted) is stamped by the brief's paste and by `AgentGetTicket` (a real delta, broadcast).
   The ticket page's state row says ` ∙ description unread` — the value step, a nudge not an
   alarm, beside the description it is about — for a claude that `has_prompted()` on a ticket
   with a description and no stamp. `SessionState::has_prompted` is the new predicate: Running,
   RequiresAction, Sleeping, Throttled, and Idle on EndTurn / Interrupted / Background — never
   Spawning, `Idle{Unknown}` or `Unknown`, so a session that has not had its first turn is not
   accused. A pre-field record reads false, which is the honest answer for a session nobody
   watched. Goldens `ticket_description_120x30` and `ticket_note_selected_120x30` moved.

**Refused, on the promise.** MCP `instructions` on `initialize` — Claude Code renders them into
the system prompt, so it would work, and `initialize_answers_and_carries_no_instructions` refuses
it for exactly that reason; hook stdout on `SessionStart`/`UserPromptSubmit` (`additionalContext`
— breaks `mesimon hook`'s never-writes-stdout invariant and promise 3 outright); and
`--append-system-prompt`, the same class.

**Tests.** `core/src/claudemd.rs` unchanged in shape (marker + tool named, fits the dialog, twice
is impossible). `core/src/mcp.rs`: the lint and the byte cap over the new sentence. `ui/tests.rs`:
`test_description_unread_follows_the_record` — present, gone once read, gone while launching, gone
without a description. `brief_e2e.rs`: the receipt stub reads the title line before the brief and
the whole body arrives; `ticket_read` flips on the paste; a plain spawn flips it on `get_ticket`
with the description in the answer; a ticket with no description submits the title alone and
stamps nothing.

## A ticket can be taken off the merge train (T-227, 2026-09-05, user: "we need an indication that the worktree is going to be merged automatically by mesimon, and let the user cancel it per ticket easily (maybe double escape on ticket hover and inside ticket page?)")

**What shipped.** `t` on the board and on the ticket page (`Verb::ManualMerge`, `Group::Worktree`,
hinted in the footer at prio 62) flips `Ticket.manual_merge` through `Command::SetManualMerge {
id, on }` → `Daemon::set_manual_merge` (`with_ticket`, a no-op when nothing changes, and
`Train::hand_touched` on the way — a person's gesture on the ticket clears the fuse and the
refusal memory, like a hand `m`). `core/src/train.rs::plan` skips a marked ticket on BOTH lists:
no automatic merge and no rebase ask; `m` by hand still does both and the ticket page's identity
row keeps offering it. The hint is `t merge by hand` while the train can reach the ticket
(`Ctx::train_reaches`: an attached binding, and the preference OR the daemon's armed word — the
reconcile lags the toggle by a snapshot and the key must not flicker across it) and `t
auto-merge` while the mark is on (`Ctx::manual_merge`), so the door that closed is the door that
reopens, whatever the train's own switch says in between.

**The indication.** The owed row's train word became `auto-merge ∙ next` / `auto-merge ∙ after
T-3 +1`: `merge ∙ next` read as a hand's merge coming up, and the row is the one place a card
says the merge is mesimon's to make. The owed mark (`glyphs::queued`, every card) and the ticket
page's state row are unchanged. A marked ticket owes nothing and wears no mark, but its row reads
`auto-merge ∙ off` (`App::pending_row`'s else-branch, only over an attached binding) — a closed
door with no sign is a ticket that silently never merges. The card is 22 cells at 120 columns, so
`auto-merge ∙ after T-3 ~` gives up the count first; the ticket page carries it whole. Goldens
`train_manual_120x30` and `ticket_train_manual_120x30`; `golden_train_120` now asserts the prefix.

**Not double-Esc.** The user's suggested gesture could not be it: Esc is the menu on the board and
`back` on the ticket page, and a chord on Esc would make a stray second press act — the inverse of
what every chord tail here promises. `t` is *train*, free on both screens, and a TOGGLE rather
than a two-press chord because both directions are visible on the card and reversible with the
same key; the cancel direction is the safe one and the other only re-offers what the Settings row
already opted into.

**Persisted, and `TICKET_SCHEMA` is 4.** The train's memory is in-memory by design (a restart
forgets and the board re-arms), which is exactly why the opt-out is NOT: a ticket the user took off
the train must not climb back on when the daemon restarts. It is a scalar on the ticket
(`#[serde(default, skip_serializing_if = "is_false")]`, after `woke_at`, before `workspace`),
omitted while off, and the schema bump is the `mcp_tools` argument again — a v3 build would drop
`manual_merge = true` on its next save and the re-armed train would merge a branch the user had
taken off it. `mcp::agent_allows` denies the command: whether a branch lands on its own is the
person's call, never the agent's whose branch it is. Feed line `set_manual_merge`. E2e
`merge_train_e2e::a_ticket_taken_off_the_train_is_left_alone_until_put_back` (the file's
`ready`/`init_repo` were lifted out of the first test for it).

**Not done.** A board-wide "hold everything" (the Settings row IS that); the mark surviving a
ticket's worktree being torn down (it stays set, harmless, and `t` is offered until it is cleared
because `manual_merge` alone satisfies `avail`); a word on the card's line 1 (the owed mark is the
every-card signal, and line 1 has no cell for a second right-hand mark — T-173's argument).

## A turn that starts without a prompt shows on its first tool frame (T-228, 2026-09-05, user: "bug, ticket stayed in review and no running indication after user bash command (!) that caused the agent to run again")

**Captured.** The daemon's own feed for the session, beside the transcript, at 14:02–14:07 local:

- 14:02:21 `Stop` → `Idle{EndTurn}` High, automove to REVIEW. Correct.
- 14:04:08 the user ran `! gcloud auth login` in Claude Code. The transcript holds a
  `<bash-input>` record and a `<bash-stdout>` record, and then — with no user prompt between —
  the assistant's next turn (a Bash call at 14:04:29). **No hook fired for the `!` input: no
  `UserPromptSubmit`, nothing.** The feed between the Stop and 14:04:34 has one idle
  `Notification` and nothing else.
- 14:04:34 onward: the turn's `PostToolUse` frames stream in, seven of them over three minutes.
  Every one was a `ToolCompleted { nested: false }` landing on a High `Idle{EndTurn}`, which the
  machine held inert by the "a background task's completion must not flip a real end_turn" rule
  (`target`'s `S::Idle { .. } if self.confidence != Confidence::High` arm). The ticket sat in
  REVIEW with no working mark for the whole turn.
- 14:07:29 a `PermissionDenied` (auto mode's deny path, `t(S::Running)` from anywhere) finally
  promoted it, and the card said working for the last forty seconds of a three-minute turn.

**The rule guarded a frame that does not exist.** The T-135 record (2026-09-01) had already
measured it: a backgrounded shell's completion emits NO `PostToolUse` (its single frame is at
launch, carrying `backgroundTaskId`), and its wake is a `<task-notification>` prompt. The one
thing a non-nested `PostToolUse` can mean after a hook-stated end_turn is that the lead is taking
a turn — and a turn can start without a prompt frame, as this one did. Claude Code's `!` bash
mode is one road; the corpus had none listed.

**Fix.** `Machine::target`'s `ToolCompleted` arm: `S::Idle { .. } if !nested => Running` at
High, from ANY idle — `Background` (T-135's arm, now subsumed) and `EndTurn` alike — and the
sub-High arm (inferred idles, nested completions too) stays under it. Automove then does what
it does for a prompt: REVIEW → IN PROGRESS. A NESTED completion (`agent_id` set: a subagent's or
teammate's tool) still says nothing about the lead, so a done lead whose teammates keep
working is not re-opened by their frames. The flap pin passes it (High is stated).

**Costs, named.** The working mark lags the tool's own duration, because the observer hooks no
generic `PreToolUse` (the wide matcher would fork on every tool call; `PostToolUse` already
does) — a `!`-started turn whose first tool runs a minute shows working a minute in. A
straggler `PostToolUse` after a real Stop would read as a resumed turn and drag the card back to
IN PROGRESS, where the quiet probe's `Idle{Interrupted}` would then leave it; none has been
seen (a Stop fires after a further model round trip, seconds after the last tool), and the
per-frame 500 ms hook self-abort bounds the reorder window.

**Tests.** `attention::a_turn_resumed_without_a_prompt_shows_on_its_first_tool_frame` models
the feed; `tool_completed_is_inert_outside_a_held_permission` drops `EndTurn` from its list and
pins the nested case; `tool_completed_recovers_tail_misreads` loses its `_but_not_stated_idle`
half. `hook_e2e` gained a leg: Stop → REVIEW, then a bare `PostToolUse` (`tool_name: Bash`, no
`agent_id`) → `Running` and IN PROGRESS through the real daemon.

## The brief moves into the system prompt (T-224, 2026-09-05, user: "we should introduce this as a better approach than modifying claude.md ∙ opt in ∙ suggested instead of claude.md modification, user consent is enough ∙ tell the user verbatim what will be added, and that it's only for mesimon created sessions")

**What changed.** T-217's offer wrote four lines into the repo's `CLAUDE.md`. The block above
listed `--append-system-prompt` as refused on promise 3, and the author, asked what it was,
chose it: a Claude Code flag that appends text to the model's system prompt for that session.
It is the better home on every axis the CLAUDE.md road was weak on — it reaches ONLY the
sessions mesimon starts (a CLAUDE.md speaks to every claude in the repo), it writes no file the
user tracks in git, it cannot drift from the binary that spawned it, and it is one Settings row
to turn off. So the offer now offers THAT, and the CLAUDE.md write is gone.

**Consent is the whole design.** `Board.system_prompt` (`core/src/brief.rs::TEXT` on the argv
as `brief::FLAG`) is OFF by default and turned on by exactly two gestures: Enter in the dialog
that shows the text verbatim, or the Settings row `Agent brief: on|off`. The dialog's first two
lines say the reach before the words — *adds to the system prompt of claude sessions mesimon
starts here ∙ only those ∙ nothing is written to disk ∙ Settings turns it off* — because that
sentence is what the user is consenting to. README promise 3 now names it as the one consented
exception beside the tool registry, and promise 1 no longer names a CLAUDE.md write (the
"Three more" is "Two more", the table lost its row). `mesimon doctor` prints the text whatever
the switch says: on, so the user can see what every agent of theirs is told; off, so the offer
is never a surprise.

**Mechanics.** `Command::SetSystemPrompt { on }` (denied to agents — a tier that could write
its own system prompt is not one) and `Command::IgnoreBriefOffer` replace `Command::ClaudeMd {
action }`; `ClaudeMdAction` is gone. `claude_argv` pushes the pair after `--mcp-config`, and
`resume_argv` regenerates it on a wake (a binary whose TEXT moved is what a wake says) and drops
it where the switch is off, the MCP blob's two rules. **Only beside the tools** (`Daemon::
brief_on` = `mcp_tools && system_prompt`): the sentence names `get_ticket`, and a system prompt
telling the model to call a tool it does not have is the lie the switch exists to avoid; the
Settings row's detail says so with the tools off. The scalar rides a plain serde default with
no `COLUMNS_SCHEMA` bump — the inverse of `mcp_tools`'s argument: a downgrade that drops
`system_prompt = true` sends LESS to the model, which is the safe direction. `ClaudeMdStatus`
and the `Sampler` survive with one job: a user who wrote the words into their own CLAUDE.md is
not offered the brief. `ClaudeMdStatus.exists` went with the "creates / appends to" wording.
`Board::claude_md_ignored` keeps T-217's key on disk so nobody who answered "never" is re-asked.
`claudemd::SNIPPET` survives as the CLAUDE.md FORM of the same instruction, printed by
`doctor` for a user who would rather keep it in their own file; `c` in the dialog copies the
text on the screen. The offer's five clauses: sampled, no marker, tools on, brief off, never not
said.

**Renames.** `Scope::ClaudeMd` → `Scope::Brief` (word `AGENT BRIEF`), `Mode::ClaudeMd` →
`Mode::Brief`, `Verb::ClaudeMd{Offer,Copy,Ignore}` → `Verb::Brief{Offer,Copy,Ignore}`, new
`Verb::SystemPrompt`, `Ctx::claude_md_offer` → `brief_offer` plus `Ctx::system_prompt`,
`ui/claudemd.rs` → `ui/brief.rs`, golden `claude_md_120x30` → `brief_120x30`,
`claudemd_e2e.rs` → `brief_offer_e2e.rs`. `daemon/src/claudemd.rs::apply` and its symlink and
twice-writes tests are deleted with the road.

**Tests.** `core/src/brief.rs`: the text fits the dialog and names mesimon and the tool.
`daemon/src/claudemd.rs`: sampler only. `store.rs`: absent reads off, the default is off, the
three scalars round-trip above the tables. `ui/tests.rs`: golden `brief_120x30`, the text on
screen verbatim under the reach line, the four answers in the edge (`enter turn on ∙ c copy ∙
i never ask again ∙ esc not now`), five clauses of the offer, golden `settings_120x30` grows the
row. `app.rs`: Enter turns it on and withdraws the offer, off re-offers, `i` stamps.
`brief_offer_e2e`: on/off/idempotent, persisted, no CLAUDE.md written, the stamp, all three
switches survive a restart, the agent tier denied all three. `brief_e2e`: a real spawn's argv
carries `FLAG` then `TEXT` verbatim after `--mcp-config`, never `--system-prompt`; no tools, no
brief; switched off, gone.

## A board on a workspace of repositories says so, and diffs them together (T-225, 2026-09-05, user: "i use multirepo (simbly, take a look). worktrees, diff, and other features of mesimon may break there so I avoid using them")

**What the corpus assumed.** One board, one repository, and `Paths::repo_root` is it: the
header's branch, the checkout diff, a worktree, a merge — every git question was asked of the
directory the TUI was started in. docs/13's ticket table had a `[[repos]]` list "from v1 even
though it always has one entry — monorepo/multi-repo is the most likely future schema break",
and the code never implemented the list. The author's `simbly` is the case: a directory holding
nineteen independent git repositories under a three-file *meta* repo whose `.gitignore` says
`*/`. Measured there (`docs/spikes/T-225-multirepo-workspace.md`): nothing failed, and every
git answer was confidently about the meta — `⎇ master ∙ 1 changed` for a board over twenty
repos, `v` diffing three files, a worktree ticket minting a worktree of the meta (an empty tree
with a CLAUDE.md describing directories that were not there) and merging into its `master`.
The author's avoidance was the correct response to a tool that could not say what it did not
know.

**What shipped — phase 1 of the spike, the honest floor; no schema moved.**

- **The census.** `gitstatus::census(root)` is one `readdir` of the root and a
  `symlink_metadata` of `<child>/.git` each — depth one, never recursive, 1.5 ms on the
  author's tree — classified by `workspace::nested_repos`: a `.git` DIRECTORY is a repo of its
  own, a gitfile belongs to someone else (a worktree kept under the root, the author's `.wt/`),
  a child `.gitmodules` declares is never a workspace repo whatever its `.git` looks like (the
  superproject case the corpus already refuses, 12 §12.6.9), sorted, capped at
  `MAX_WORKSPACE_REPOS` (64). Not used as signals, deliberately: the parent's tracked-file
  count, a `*/` ignore line, manifest files of `meta`/`git-repo`/`vcstool` — the nested-`.git`
  census is exact and the rest are guesses.
- **On the snapshot.** `RepoGit.repos: Vec<String>` (`#[serde(default)]`), beside the fields it
  reinterprets: `changed` is now the SUM over the root and every nested repo (the author's
  call — files, not repos, "and we need the diff viewer to support that"), `branch`/arrows/
  `upstream` stay the root's own and speak for nothing under it. `gitstatus::sample(root)` is
  the workspace sample (`sample_one` is the old single-repo one); a folder of repos with no
  repository at the root still reads `sampled` — there IS a checkout under the board, nineteen
  of them — with `branch` empty, so the header speaks and `v` has something to open.
- **The header** names a workspace by its count where a checkout is named by its branch:
  `⎇ 19 repos ∙ 214 changed  v diff`, the root's arrows left off (they are the meta's), the
  same fit ladder. `workspace::repos_word` is the one spelling; the diff screen's identity row
  uses it too, so the two agree.
- **Board `v` is one list across the workspace.** `checkout_diff_list(root)` runs
  `checkout_entries` per repo — the root's rows first and bare, then each child's prefixed
  `<repo>/`, in census order — and `checkout_diff_file` routes on the first path component: a
  census name goes to that child with the rest, anything else is the root's own. The two
  cannot collide (a directory that is a nested repo is never a tracked path of the meta), and
  the rest is checked against the CHILD's list, so a `..` through the prefix goes nowhere. A
  meta that does not ignore its children lists each as a `? child/` sighting git will not
  descend into; those rows are dropped, because the child's own rows follow under that very
  name. The census is asked per call (1.5 ms) rather than cached — the list and the file
  must agree about which names are repos, and a cache on the connection thread would be a
  second source of truth.
- **A worktree ticket is refused in words.** `resolve_spawn_cwd` asks the census (not the last
  sample, so a spawn before the boot sample lands is judged the same) and returns `this board
  sits on a workspace of 19 repos — a worktree of it would hold none of the code; workspace
  worktrees are not built yet, use the shared checkout`. Nothing is provisioned — no binding,
  no branch, no directory. A binding already attached is kept. And `Ctx::multi_repo` hides the
  composer's and the editor's Shift+Tab workspace choice (the ASK field's `now / queued` on the
  same key is untouched — `a_workspace_board_offers_no_worktree_choice`), so the refusal is the
  belt under a choice that is not offered.
- **`doctor`** prints a `workspace` line before `branch` (which then reads `root: master, …`),
  with the first four names and the two facts above.

**And a single-repo bug on the way past: `default_branch` read `origin/HEAD` literally.** The
author's v2 repos name their one remote `gitlab`, so the first rung never fired there and a
stale local `main` on the second rung beat the real deploy branch the remote's HEAD names. It
now asks the remote the checked-out branch tracks (`gitstatus::remote_of`), then `origin`, then
the only remote there is, before the `main`/`master`/`trunk` ladder. `on_git_sampled`'s
"a fetch can mint `refs/remotes/origin/HEAD`" comment stays right in spirit — it is the
remote's HEAD, whatever the remote is called.

**What is deliberately NOT here** — the spike's phases 2–4, in order: which repos a ticket
touched, LEARNED from the hook stream (the `Edit`/`Write` `file_path` `ingest` already reads)
and never asked (author: "it's manual labor, and the agent might want to change another
subrepo in the same session"); **workspace worktrees** — a worktree of the meta as the
container, one child worktree each inside it on the same `msmn/<KEY>-<slug>` branch, base =
that child's checked-out branch, merge per child (not atomic across repos; the ticket page
says which half landed), bindings schema 2 — the refusal above is the placeholder for exactly
that; `Fetch all remotes`; the quiet gate keyed on learned repos. Measured cost of a full
workspace worktree on simbly: 3 627 files / 70 MB, to be timed before it ships.

**Tests.** `workspace.rs`: the census rule, the cap, `.gitmodules`, the word. `diff.rs`: a
scratch meta over `api`/`web` plus a gitfile child and a declared submodule — census, the
summed sample (and the same root with its `.git` removed, a folder), the prefixed list with
the sighting dropped, file routing and its three refusals. `worktree.rs`: a clone with
`-o gitlab` whose origin defaults to `trunk`. `keymap.rs`: the hidden choice. TUI:
`test_git_clause_names_a_workspace_by_its_count` and golden `board_workspace_120x30` (a clean
workspace keeps its name and loses the count and the key). E2e `workspace_e2e`: the census and
sum on the wire, the list and the file through the prefix, the refused spawn and the untouched
worktree root. `CHANGELOG.md` was NOT edited: alpha.13 is tagged, and the next heading is the
next release's to write.

**The root's branch leads; the count is a clause; `1 repo` is never said** (author, twice on
2026-09-05: "if one repo no need to show '1 repo', show the branch", then — after an hour in
which ONE nested repo was made the checkout — "why does it say orphan and not main": the mesimon
checkout carries `mt/`, the leftover research scratch, a git repo of its own on a branch called
`orphan`, and the board had adopted it). The rule now: a root that is a repository keeps its own
branch, arrows and upstream whatever is nested under it, with the nested repos' changes still
summed in; a workspace of SEVERAL adds ` ∙ 19 repos` as a clause after the arrows (`⎇ master ∙
19 repos ∙ 214 changed`, the count dropping with the change count when the row is tight); a
FOLDER — no repository at the root — holding exactly one repo takes that repo's branch, and
holding several is named by the count alone. `gitstatus::branch_dir` says where the sampled
branch lives (the root, or the one child under a folder), and the fetch runs there because
`branch.<b>.remote` is that repository's config. The diff list's word follows the same rule,
`doctor` labels the line `root: main` or `api: orphan`, and `Ctx::multi_repo` stays true at one
— a worktree of the root would still hold none of the code. `repos_word` keeps its `1 repo`
form for nothing but its own test.

**Amended the same hour (user: "it doesn't say verbatim ∙ I wanted a prompt for confirmation and
`c` for copy in that dialog ∙ if it suggests while off we need an option to ignore it").** The
Settings row had turned the brief ON in one press, showing only its label. Now ON from Settings
opens the SAME dialog (`Mode::Brief { from_settings: true }`) — text verbatim, `c copy`, `i never
ask again` — and every answer returns to the row (`App::leave_brief`), which then reads `on`; OFF
stays one press and STAMPS the offer answered (`set_system_prompt(false)` sets
`claude_md_ignored`), so a person who turned it off is not offered it again — Settings is the
way back, which is what makes the stamp affordable. The row's detail says `enter shows it
first`. Tests: `the_settings_row_turns_the_brief_on_through_the_dialog`, and the off-road
assertion in `taking_the_brief_offer_turns_it_on_and_withdraws_it`; `brief_offer_e2e` asserts
the stamp on off and that `i` leaves the switch alone.

## The ticket page's chip names its ticket (T-233, 2026-09-05)

The header chip on the ticket page reads `TICKET (T-12)` — the ticket's `short_key` in the
chip's own parentheses — where it read `TICKET` alone (user: "add ticket id in title on ticket
page"). The key is how a ticket is named everywhere off the screen (a prompt, a note, a commit
message, `get_ticket`'s answer), and the page's own title row spells only the title, so a
person on the page had to go back to the board to learn the key. `chrome::screen_word` now
returns a `String` and reads the ticket on `Screen::Ticket`; every other screen's word is
unchanged (`NOTE` keeps naming its ticket through the leaf; the diff's leaf stays the title).
The fourteen ticket-page goldens moved on their header row and nowhere else.

## The worktree flags leave the writer thread (T-216, 2026-09-05)

"Sometimes a tag digit hangs for a moment until it takes effect" (user). A digit is
`App::cycle_tag`: a synchronous `SetTag`, then a synchronous snapshot, and the card repaints
after both — so a keypress waits on whatever the daemon's single writer thread is doing.
Every `wt_refresh_ticks()` (10 s) `on_tick` ran `refresh_worktree_flags` there: `branch_tip`
of the base, then per binding `branch_tip` + `is_merged` + `ahead_count` + `ff_possible`, four
git forks each. The author's board carried thirteen attached bindings — 53 forks, 675 ms
measured idle — and a read-only snapshot probe at 20 Hz against the live daemon showed one
stall of 443–481 ms every 10.1 s, the cadence exactly. Every e2e shrinks the cadence through
`MESIMON_WT_REFRESH_TICKS` and none has thirteen bindings, so nothing had seen it.

Two changes. **The sample is one function, `worktree::compute_flags`, in `2 + n` forks**: one
`for-each-ref --format='%(refname) %(objectname)' refs/heads/` for every tip (the full refname,
never `refname:short`, which git abbreviates differently beside a remote-tracking ref of the
same name), then `rev-list --left-right --count <base>...<branch>` per binding — the left count
is what base has that the branch lacks (zero ⇔ `ff_possible`), the right is what the branch
has over base (`ahead_count`; zero ⇔ `is_merged`) — and the `worktree list` for conflicts.
"Merged" still requires the tip to have moved off `base_oid` (the 2026-08-30 fresh-branch
rule), and a branch git no longer has reads as the old helpers read it: not merged, nothing
ahead, no fast-forward. `compute_flags_agrees_with_the_single_question_helpers` holds the four
helpers and the one sample to the same answers on a scratch repo. **And the tick no longer
takes the synchronous road**: `queue_worktree_flags` runs the sample on a worker (resolving
`default_branch` there too when the cache is empty — a fetch empties it) and it lands as
`Msg::WorktreeFlags(gen, flags)` → `on_worktree_flags`, which absorbs it and runs `train_pass`
on it, as the tick did in one turn. `wt_gen` is bumped by every synchronous refresh (startup,
a merge just made, a binding attached or torn down — the roads that must read fresh flags in
the same turn keep `refresh_worktree_flags`, now `2 + n` forks itself) and a sample carrying an
older generation is dropped: the flags on hand are newer than it. One sample in flight at a
time (`wt_inflight`); the lazy unlock stays on the writer inside `absorb_worktree_flags`, the
one git fork left there, and a rare one. Live after the change, same probe, same board: no
stall over 100 ms in 35 s, max 57 ms (was 481).

Found on the way and left alone: `store::write_atomic` said "NOT F_FULLFSYNC, ~170 µs", but
Rust's `sync_all` IS `fcntl(F_FULLFSYNC)` on macOS — ~3 ms a call, ~8 ms for the temp + fsync
+ rename + dir-fsync shape, paid by every `save_ticket` and `save_sessions` (192 KB, 148
records on the author's board) on the writer thread. Not the hang; the comment now says what it
costs and why the barrier stays.


## `P` opens every card, and the pin is gone (T-237, 2026-09-05)

`P` on the board is `p` widened (user: "shift+P to peak all ∙ no need to hint this"): every
card carries its chips row and its latest reply, not only the cursor card's. `Verb::PeekAll`,
board only — the ticket page draws no cards (user: "peak belongs to the board, not ticket
page") — overlay-only like `p` (`?` names it, the footer never does), `Ctx::peek_all` for the
hint. It IMPLIES the cursor card's own peek: `P` on sets `App::peek` too, `P` off narrows back
to the cursor card rather than to nothing, and `p` off takes `peek_all` with it — shift never
switches verbs, and the ladder is off / cursor / all. `ui/board.rs` opens a card on `peek_all ||
(selected && peek_showing)`; in `card.rs` a resting card that is open draws ONLY the tag row and
the reply, on the resting ramp with no surface — the session list, the armed snooze and the owed
row stay the cursor card's accordion, since they are about the selection. The transcript is read
through the same per-path `PeekCache`, so opening thirty cards is thirty cached tails. Golden
`board_peek_all_120x30`; `shift_p_widens_the_peek_to_every_card` holds the ladder.

**The session pin went with it** (user, the same hour: "remove pin, I don't think I ever used it
and I don't know what it does"). `P` on the ticket page had been `Verb::Pin` → `Command::PinAwake`
→ `SessionRecord.pinned_awake`, the D23 "third part" manual override that refused `x` and the
bulk sweep on a pinned session (docs/14 §6.3, docs/16 — idea stock now). All of it is gone: the
verb, the binding, `Ctx::sel_pinned`, the command and its two allowlist arms, the daemon handler,
the `sleep_eligible` refusal, the record field (an old `sessions.json` carrying `pinned_awake`
still loads — serde ignores what it does not know — and the next save drops it) and the rail's
`pinned` badge. Nothing else read the flag. That is also what freed `P` for the peek without
putting two verbs on one key across the screens.
## Test resource ownership before a second runtime provider (2026-09-05)

End-only cleanup and the in-process Harness were not sufficient: startup panic
could precede construction of its guard; teardown could block forever joining a
daemon thread; and a killed test process could leave detached tmux alive. The
process-owning integration fixtures now register unique paths with a separate,
deadline-bound Python supervisor before launch. Daemons are supervised subprocesses,
fixture configuration is child-scoped, default agent executables are fake, and
shell homes are private too: `/bin/sh -l` must not read a developer's `.profile`.
The headless M1 wire test moved from the TUI crate to the binary's integrations so
it can use the real daemon executable and the same harness. It tests no TUI code.

The supervisor observes control-pipe loss, reaps owned children and tmux descendants,
checks process identity and registered directory identity, and preserves a manifest
on uncertain cleanup. Ordinary teardown also checks its exit status. Python 3 is
a development/test dependency only; no Python code enters the shipped application.
`ci/test-run.py` adds whole-command supervision, two-worker defaults, an overlap
lock, required-tmux semantics, and an exact-fixture audit. CI/release test callers
use it; the old CI glob of every Mesimon tmux socket was removed. If both owner and
runner are forcibly killed, automatic cleanup is not promised: inspect the retained
registry rather than signalling recycled PIDs or deleting changed paths.

The restart-skew tests invoke their real TUI client in a child-only helper so the
replacement daemon also inherits fixture settings, not personal shell rc files.
That helper is ignored in ordinary discovery but explicitly executed by both
restart tests. The existing live-release-network test remains opt-in.

No runtime provider or production state format changed in this checkpoint.

Linux validation also exposed a snooze e2e timing assumption: the test equated
the scheduled deadline with the actual wake timestamp. The daemon deliberately
stamps the tick that performs the wake. The test now bounds that timestamp between
deadline and observation and requires it to match `woke_at`; it still checks the
order, attention, persisted state, and deadline refusal. No snooze behavior changed.
The Docker check mounts a worktree's common Git metadata read-only at its original
path and caps the container at two CPUs, 4 GB RAM, and a 15-minute lifetime.

Review before the merge (T-235, the same day) changed four things. The concurrency caps
came off the default path: `.config/nextest.toml` keeps the slow and leak timeouts but no
`test-threads`, and `ci/test-run.py` caps build jobs and test threads only under `--jobs N`
— the two-worker default had turned the ~30 s parallel suite into 92 s and would have
throttled the release gate's build with it. `TestFixture::new` panics on a daemon seam found
in the test process (`common::DAEMON_SEAMS`), because the child's env is built from `set_env`
and a `std::env::set_var`, the recipe until now, was dropped silently — a test on default
timings passing for the wrong reason. A failing test echoes its children's `child-N.log`
into the captured output before the supervisor removes the root, so the daemon's stderr is
not lost with the fixture the way the in-process daemon's never was. And a clean audit
removes its `/tmp/msmn-test-run-*` registry instead of leaving one per run; a failed one
keeps it. The tester-facing TESTING.md carries none of this — it is CLAUDE.md's and
AGENTS.md's. The rebase onto main proved the guard the same hour: T-227's second merge-train
test had arrived with two `set_var`s before `Harness::boot` and now rides `boot_with_env`.

## The reload says what it is doing (2026-09-05, user: "the whole screen is very unindicative because it says `[detached (from session …)]` and no indication of something that's loading or failed")

**What happened.** Two `U` reloads on the simbly board in one afternoon, both while two other
sessions ran a full parallel e2e suite each (a hundred daemons, tmux servers and stubs forking at
once). The daemon side of the second one took two seconds — lock released at 17:29:21, the new
client subscribed at 17:29:22, every hook frame in that minute landed — and the user still
reported it as "stopped working", because what the screen showed was tmux's own
`[detached (from session ab34b94f6b574cdd)]` line and nothing else. The first outage that day
was real (a daemon unreachable for four minutes with thirty tool calls' hook frames dropped;
cause not captured, a read-only Hello watchdog now sits beside it), but the two looked identical
from the chair, which is the finding.

**Why.** `run` returns from the event loop, `restore_terminal` leaves the alt screen, and the
PRIMARY screen shows whatever was last printed there — on a board that has been into a session,
tmux's detach line from the last `Ctrl+]`. `reexec` then waits for the old daemon's lock (silent
for its first second) and execs; the new process runs the colour probe, loads prefs,
`Client::connect` (a five-second spawn budget once the lock frees, a ten-second Hello, a possible
build-skew restart of three plus eight seconds and another Hello) and `App::new`'s first snapshot
(ten more) — ALL before `init_terminal`. On an idle box that is milliseconds and the stale line
flashes; under load it is tens of seconds with a stale line as the only thing on screen. A slow
connect and a dead client were indistinguishable.

**What holds now.**
- `reexec` blanks the primary screen first (`blank_primary_screen`, the road the attach already
  takes) and prints `mesimon: reloading…`. `await_daemon_gone`'s `waiting for the daemon to
  finish shutting down…` still follows after a second when the SHUTDOWN is what is slow.
- `client::LateWord`: a sentence said once, on stderr, only if the wait it wraps outlasts its
  delay; dropping it in time says nothing. `run` holds one — `mesimon: connecting to the
  daemon…`, one second — across `Client::connect` and `App::new`, and drops it before
  `init_terminal`. A quick reload flashes nothing new; a slow one names the wait; a stuck one
  names it too and then opens the empty board with the `daemon_down` advisory, as before. Test:
  `a_late_word_is_said_only_past_its_delay` (channels, not sleeps).

**Not done.** The words are the primary-screen stopgap. The better home is the alt screen:
enter it before connecting and draw the empty board with the advisory row while the connect
runs — `Client::connect` already tolerates no daemon, so the plumbing exists; what stands in the
way is that the colour probe and the kitty probe are ordered around `init_terminal` today. And
the four-minute outage still has no captured cause; the daemon logs neither its start nor why it
stopped, and a `daemon.log` line for each would have answered it.

## The daemon keeps a journal (2026-09-05, user: "what do we do with the daemon.log from now?")

**What happened.** The same afternoon as the reload words above: a daemon left for four minutes
with a clean exit and nothing said why. `daemon.log` had been the detached spawn's stderr sink
since alpha-1 and had never held a byte (no daemon ever panicked); the activity feed's `seq`
restarting at 1 was the only trace of the restart, and a sequence names a start, never a reason.
Three hours of forensics — transcripts, inodes, `sample`, `lsof`, the unified log — could not
recover what one line would have said.

**What holds now.** `daemon/src/journal.rs`: `daemon.log` is the daemon's own record of what the
PROCESS did, where the feed is what the board did. Three kinds of line, UTC-stamped, written at
once (rare, and a crash a moment later must still find them), rotated by size at 1 MiB to
`daemon.log.1`:
- `started pid … build … exe mtime … len … repo … detached|foreground`, written the moment the
  flock is won (a loser writes nothing — under the reconnect cadence that would be a line every
  400 ms).
- `stopping: <why>` from `begin_shutdown(why)` — the one shutdown road — `SIGTERM`, or `shutdown
  asked by <client>` where the client is the `Hello` string the connection sent
  (`Daemon::clients`, keyed by the writer `Arc`'s address like `subscribers`, pruned on
  `ClientGone`); then `stopped ∙ shutdown took N ms` after the sockets go.
- `slow turn: <what> took N ms` for any writer-thread turn past `journal::SLOW_TURN` (1 s), named
  before it runs (`tick`, `hook Stop`, `request snapshot`, …); a tick's line adds `∙ slowest stage
  probe_activity 2100 ms`, from a `stage!` macro around every step of `on_tick`
  (`Daemon::tick_slowest`). This is the instrument that names a blocking site without a watchdog.
The stderr fd still points at the same file, so a panic lands beside the journal's lines (after a
rotation it follows the old inode into `.1`; the cap makes that a once-a-year event). A journal
that cannot open writes nothing and fails nothing. `mesimon doctor` prints a `daemon log` line:
the path and the last `stopping:` line, or `no stop recorded`. E2e: `shutdown_flush_e2e` asserts
the start, the `SIGTERM` reason and the end. Unit tests pin the calendar arithmetic (`iso_utc`,
dependency-free), the threshold, the rotation and the silent failure.

**Not done.** The journal says a turn was slow and which stage; it does not yet say what the
stage was waiting on (which tmux command, which git). A `hook` frame's own age (arrival minus
the hook's timestamp) would show ingest lag, and a `client connected` line would show a TUI
that dialled and gave up. Add them when a journal line asks for them, not before.

## A taken-over session reaches its tools (T-240, 2026-09-05)

**What was wrong.** `Daemon::handle_agent` gated the agent tier on `provenance != Spawned`,
while every other place the daemon asks "is this observe-only?" — focus, sleep, the working
set, the attach — asks `provenance == Adopted && argv.is_empty()`. A taken-over external
session keeps `Provenance::Adopted` for life (nothing ever flips it: `resume_session` sets the
argv and leaves provenance alone), and its takeover argv is built by the one `claude_argv`, so
it carries `--mcp-config` keyed on the record's uuid like any spawn's. The shim called in as
`Agent { session: rec.id }`, the record was found, and the gate answered `not a session mesimon
spawned` — seven tools handed out, seven refused. The comment over the gate already named the
right predicate ("no argv of ours and were never launched with the tool config"); the code under
it tested something narrower.

**What holds now.** The gate is the daemon's one observe-only predicate: adopted AND no argv.
An observe-only record is still refused (no tool config was ever handed out, so a call claiming
it is nobody mesimon started); a taken-over one passes and the `is_live()` check under it still
catches an exited record. No provenance is rewritten — `Adopted` still means "the conversation
began outside mesimon", which the badge word `external` reads. `m3_e2e` asserts both halves
around its takeover: the agent call refused on the observe-only record, `--mcp-config` in the
takeover argv, and `get_ticket` answering with the minted ticket's title afterwards; the same
assertion fails on the old gate with the exact message the ticket reported.

## The ask's drop hint moves into the placeholder (T-241, 2026-09-05)

**Refuted:** the delivery row under a reopened queued ask read `  queued  shift+tab ∙ blank enter
drops` — 39 cells, built as fixed text and never measured against the card. A column narrower
than that clipped it at the card's edge (`… blank enter dr`, the user's screenshot), and
`MIN_COL` is 26, so the row could never fit on a tight board.

**Built:** the clause is gone (user: "drop the hint"). `card::render_ask_mode` is two words at
any width, and `card::render_prompt` takes `reopened`: a field opened on a WAITING ask whose
text has been emptied shows the placeholder `enter drops` where a fresh field shows `ask
claude` — 11 cells, inside the prompt row's budget at `MIN_COL` (20). The gesture is still
taught only where it applies, on the one field whose blank Enter does something. Test
`test_an_emptied_queued_ask_says_enter_drops` renders it at 120 and at `MIN_W`.

## The second simplify pass (T-234, 2026-09-05)

The same four-angle pass as 2026-09-01 (reuse, simplification, efficiency, altitude), over
the 92 commits since it — 51 findings, about eight of them found twice. Nothing here changes
what the board does; the tests, the goldens and the e2e suite ran unchanged (919 passed).

**Applied.**
- **One clock**: `mesimon-core::clock::{now_ms, now_secs, epoch_ms}` replaces eleven private
  copies of the `SystemTime … UNIX_EPOCH … as_millis` idiom across the daemon and the TUI.
- **One `plural`**: `text::plural` (was private to `keymap`); `workspace::repos_word` reads it.
- **The snapshot road forks no git again.** `pending_items` and `train_pass` each forked
  `rev-parse` per merge candidate on the writer thread, on every `Snapshot`, though the flag
  worker's `for-each-ref` had every tip; `worktree::Flags.tip` now carries it and
  `Daemon::wt_tip` holds it beside `wt_ahead`. The hand `m` keeps its one-shot fork.
- **The PREVIEW zone renders its markdown once per document** (`App::rich_cache`,
  `ui::ticket::rendered`, keyed like the page scroll on `(doc_key, width, flavor)`) instead of
  parsing and wrapping the whole reply every frame — 60 fps through a glide.
  `PeekCache::peek` hands out `Rc<Peek>`: the board asked per open card per frame and cloned
  the reply text each time. `App::ctx()` no longer sorts every archived ticket to test
  emptiness, and computes `subject`, `undo_target` and `tag_cell` once.
- **One predicate for "mid-turn"**: `App::ticket_busy` and `merge_ticket`'s gate both read
  `quiet::is_working` — the TUI still counted a `!` shell (so `m` was inert on the board while
  the daemon would have merged), and neither counted `Idle{Background}` / an owed Enter, which
  the train's gate already did. `Ctx::ticket_has_claude` is `Board::live_claude`.
  `checkout_holders` / `board_busy` are one `working(cwd)`; the two quiet probes share
  `probed_running`.
- **One road back from the archive**: `Daemon::unarchive(id, land, needs_you)` serves the
  restore (order kept, restamp only on a column change) and the snooze's wake (`Top`, restamped,
  lit), which had been a second hand-rolled copy.
- **One builder for mesimon's argv pairs**: `mesimon_flags` (the MCP blob, the brief) feeds
  `claude_argv`, and `resume_argv` strips every owned pair and re-appends it behind
  `--settings` — the same argv as before for everything `claude_argv` ever built, and the
  drop / regenerate / insert rules stated once instead of three times.
- **One paste**: `paste_to_ticket` (the pane lookup and the two refusal sentences) under
  `merge_to_agent`, `note_to_agent` and `prompt_session`; the merge road's refusal now reads
  `start or wake one first` like the other two.
- `with_ticket` returns the `Response` (the eleven callers had each re-spelled `no such
  ticket`; `no_such_ticket()` is the one literal); `seen_ticket` / `set_manual_merge` look the
  ticket up once. The columns bar on a person's `CreateTicket` is judged inside
  `create_ticket`, the depth the agent's mint already used. `read_columns_file` under the two
  doctor readers; `git_bytes_ok` under `git_bytes` / `git_bytes_diff`; `Train::refused` gone
  (it was `refusal(..).is_some()`).
- TUI: `EditOps` (a trait both text fields implement) collapses nine `match ed.focus` arms in
  `key_editor` into `ed.focused().<op>()`; `set_pref` is the four Settings toggles' tail;
  `step` is the five list modes' clamp; `TagArm::new`; `Ground::word` (was spelled four
  times); `text::hash64` under the four `DefaultHasher` helpers; `ui::spans_width` under 21
  inline width sums; `eased` under the glide's and the dialog's identical curve; `Describe` is
  `NoteEdit`'s arm (off the ticket page there is no rail row); the `^S` gate reads the board's
  own `ticket_promptable` / `ticket_has_claude` — the editor's ticket IS the subject — so
  `Ctx::editor_claude_paned` / `editor_seat_empty` are gone; prefs' `Loaded` derives
  `Default`, its three bool setters (dead writes: `body()` rewrites every key) are fields.
  Fixed on the way: a status line carrying 32 embedded spaces, and a doc comment saying
  `KillSession` was unreachable from the TUI after `x` on a corpse started sending it.
- E2e: `common::{git, init_repo, git_of, files_of, pending_of, Shim}` — the shim client and
  the git helpers had been pasted into two and five test files respectively.

**Deferred, by design (do not re-derive):**
- The composer's four round-trips (`CreateTicket`, `SetWorkspace`, tags, `WriteNote`) vs the
  agent's atomic `AgentCreateTicket` — a wire change; the partial-failure window is real.
- The two owed-paste ledgers (`pending_prompt` + `pending_submit` + `submit_retry` vs
  `inflight`) — one `owed` map keyed by session would be the design, not a cleanup.
- `pane_reborn`'s tmux fork: passing `#{pane_id}` in the `pane-died` hook and matching by
  pane identity is the deeper fix (a hook + record change).
- One `Pager` for the diff, PREVIEW and RELEASES scroll states; the daemon computing
  `ClaudeMdStatus.offer`; `Pending.action` as an enum — each a wire or structure change.
- `snooze_blocked`'s hand-composed refusal sentence (move `sleep_eligible`'s pure clause
  into core) and the four framed list dialogs (`themes`, `menu`, archived, drawer) sharing one
  `dialog::list` — both fair, both touch goldens/status copy; next pass.
- Efficiency items that change when something is read, not how: sampling git only while a
  board is subscribed (a headless daemon forks `git status` per repo every 10 s), an mtime gate
  on `probe_status_files`, a byte budget on `untracked_adds`, `PeekCache` throttling its
  `metadata()` call, a per-frame `Ctx` built once in `ui::draw` (it is built 3–7 times).
- `journal::iso_utc` via `gmtime_r` (correct as is; unsafe is the author's call);
  `dialog::fit` vs `rich::clip` (they cut differently: `~` marker vs silent); `Journal` and
  `FeedWriter` sharing a `RotatingFile`; `refresh_status_line` reusing its `attention_queue`
  (the second call is `needs_you_count`, kept so the two numbers cannot disagree).

## A ticket says who filed it (T-253, 2026-09-05)

- **`Ticket.created_by`** is a new scalar after `created_at`: `local` for a person at the
  composer, `agent:<session-uuid>` for an agent's `create_ticket` — `Principal::note_author`'s
  vocabulary, so a ticket and its notes name an author the same way. `Daemon::mint_ticket`
  takes the principal and every mint threads it (`create_ticket`, `agent_create_ticket`, the
  drawer's import in `attach_external`). `#[serde(default, skip_serializing_if = empty)]`, no
  `TICKET_SCHEMA` bump: an older build rewriting the file drops a word, not a behaviour. Empty
  reads as UNKNOWN (`Ticket::agent_created` is false), never as a person.
- **`Ticket.created_from: Option<Ulid>`** beside it is the ticket the agent was bound to when it
  asked — `handle_agent`'s resolved binding, passed through `agent_create_ticket` to `mint_ticket`
  (`None` on a person's mint). A fact of the file rather than a join against the session record,
  which `delete_ticket` removes with the parent; a ULID, never a key, so a renumbered board cannot
  point it at the wrong card.
- **The ticket page's state row says `created 2d ago by claude on T-241`** on an agent's ticket —
  the parent's key resolved from the board, dropped when that ticket is gone — and nothing
  by a person's or a pre-field one — T-158 cut "created by you" as saying nothing in single-user
  v0.1, and that decision stands; the clause carries the word only where it is information.
  Before this the sole record of an agent's mint was the feed's coarse `agent` actor and, when a
  description was passed, the note's author.
- Not done: the card says nothing (line 1 has no room, see "The done mark decays once seen"),
  `get_ticket` does not carry it, and the feed line is unchanged.

## A late Stop is not an Esc (T-242, 2026-09-05)

**What was seen.** simbly T-11's claude finished a long turn at 19:56:33 and the card read
`interrupted` for the next 38 s, then went to REVIEW. The feed: the transcript's closing
`end_turn` record and Claude Code's `stop_hook_summary` land at 19:56:33, Claude Code stamps its
own `~/.claude/sessions/<pid>.json` `status: idle` 20 ms later, `probe_status_files` reads it at
19:56:36 and commits `Idle{Interrupted}` Medium — the row built for the recordless Esc — and the
`Stop` hook frame arrives at 19:57:14, 41 s late, committing `Idle{EndTurn}` High. Every earlier
Stop of that session (twenty) had landed within 100 ms.

**Why the hook was late.** Another mesimon session ran `cargo nextest run` in the mesimon
checkout at 19:55:34, relinking `target/debug/mesimon` — the binary every hook execs — at
19:55:46, with the 16 e2e tests running after it on a box at load 7.7. Every hook spawned after
the relink stalled: the two sync `PostToolUse` hooks hit Claude Code's 2 s timeout and were
LOST; the async `Stop` (no Claude timeout) delivered when exec finished. A `mesimon hook` that
delivers after 41 s without tripping its own 500 ms self-abort spent those seconds before
`main`, i.e. inside exec, and the unified log says where: AMFI notes the fresh binary has no
CMS blob (the linker's ad-hoc signature), syspolicyd logs `Couldn't find a cached target …
during malware scan` and opens an XProtect analysis connection every 6–8 s through the window,
and the kernel logs an AppleSystemPolicy check against that path for ~37 new pids a second —
the machine had 40 LEAKED e2e stub sessions (`claude-stub.sh` `sleep 1` loops from interrupt/m3/
worktree runs, the oldest four days old) in 40 leaked private tmux servers, each forking once a
second. So: a rebuilt, ad-hoc-signed binary's first execs are held for a malware scan, and under
an exec storm the hold was tens of seconds. The same shape is why the first run of a freshly
linked e2e binary fails its opening `wait_until` and passes on the rerun.

**The gap, and the fix.** The session file flips `idle` at the end of EVERY turn, milliseconds
before the Stop hook fires — the probe never distinguished "the turn closed and the hook is in
flight" from "the prompt was handed back to the box". Now it asks the transcript first:
`daemon/src/tail.rs::turn_done_since(path, since)` walks the last 64 KiB newest-first through
`core/src/adopt.rs::turn_edge`, where an `assistant` record with `stop_reason: end_turn` or a
`system`/`stop_hook_summary` (or `turn_duration`, which current Claude Code no longer writes)
is `Done(at)`, a `user` record or a mid-turn/aborted assistant record is `Open`, and latches,
attachments and other system records are `Unsaid` and skipped. `Done` at a stamp `>= since`
(the Running spell's `state_changed_at`) rides `Signal::StatusFileIdle { turn_done: true }` →
`Idle{EndTurn}` at Medium — the same row, the same leave-settle, and automove promotes Medium,
so the card reaches REVIEW without the hook; a Stop that arrives later commits High over it,
and a probe never overrides a pending stated leave (unchanged). The recordless Esc is intact:
its last closing record is the PREVIOUS turn's, older than the spell, so `turn_done` is false
and the row stays `Interrupted`. `adopt::iso_ms` parses Claude Code's `timestamp` (UTC `Z`
only, no guess for anything else). E2e: the second test in `interrupt_status_e2e.rs` writes
the closed turn and expects `EndTurn` Medium and REVIEW. Not done here: the leaked stubs and
servers were left for the author (`tmux -S /tmp/mesimon-501/<proj16>/tmux.sock kill-server`
each, never a sweep — real boards are among the sockets); the sync hooks' 2 s timeout stays.

## A ticket's notes carry its links (T-256, 2026-09-05)

**Built.** `^k` on the board (the cursor card) or the ticket page opens a LINKS dialog over the
ticket's notes — one row per target, `jk` / Enter / `c copy` / Esc, `^k` again closes — and
`^K` (ctrl+shift+k) opens the first link with no dialog. Three kinds: a URL (`http(s)://`, bare
or `[label](url)`), another ticket by its short key (`T-12`), and a file path that EXISTS under
the ticket's directory (its worktree when attached, else the repo root), with a `:LINE` suffix
kept. The recogniser is `core/src/links.rs::extract` — pure, unit-tested, document order, one row
per target; a path is only a candidate there and a key only a key: `App::ticket_links` resolves
both against the disk and the live board (`Board::ticket_by_key`, new; `board::KEY_PREFIX` names
the `T-` that `mint_ticket` and `store::recover_next_key` spelled as a literal). The use case on
the ticket: a card bound to a Jira issue, reached from the board without opening it.

**Derived on every press, never persisted, never on the snapshot.** The recogniser will grow
("more?"), and derived data on disk drifts from its deriver — `NoteMeta.name` gets away with it
because it is trivial. So the TUI fetches the note bodies the cache lacks through the existing
`Command::ReadNote` road (`App::fetch_links`; the board has none, the ticket page has the
description) and reads them: no wire command, no daemon change, no schema bump. The cost is one
to three small reads on the writer thread per press.

**Opening.** A URL rides `App::pending_open` to `lib.rs`, which spawns the opener DETACHED
(`tui/src/opener.rs::launch`: null stdio, reaped on a thread, no terminal handover) — the first
process the TUI starts without giving the terminal up. The ladder is `MESIMON_OPEN` (also the
seam: no test ever finds a browser, since `App::opener` is set in `lib.rs` like `editor_word`),
then `open` on macOS, `wslview` under WSL, `xdg-open` on PATH; `doctor` prints it as `opener`.
The status says `opening …`, never `opened` — the `asked`-not-`sent` rule. A FILE is judged by
git's rule at open time (`links::looks_text`: no NUL in the first 8 KiB; user, 2026-09-05: "if
editor appropriate use editor if not, os"): text goes to `$VISUAL`/`$EDITOR`/`vi` on the `^g`
road (`external::open_argv`, `+LINE` only for vi's family, nano, emacs, micro), parked on
`pending_attach` with the file's directory as cwd so the editor's exit status is never judged
(the `!` shell's shape); anything else goes to the opener. A TICKET from the board moves the
cursor (stay on the board); from a page, or when the target is archived and has no card, its
page opens; a target deleted since the note was written is a status line (re-resolved at open,
`undo_target`'s discipline).

**Keys.** `Ctrl('k')` is bound on Board and Ticket (siblings, like `^t`, never Global),
`avail: has_ticket && ticket_described`, `prio: 0` — overlay-only, the footer is the selection's;
the ticket page's state row carries ` ∙ ^k links` while a fetched body holds one (T-158's idiom,
`hint_for`; no count, since the page caches only two bodies). **`Key::Ctrl('K')` joins
`OFF_FLOOR` on `^S`'s clause**: a legacy terminal sends the bare `^k`, which opens the
dialog — the safe half of the same axis, Shift hardening the verb and never changing it —
`ctrl_shift_k_is_inert_without_rich_keys`. `Scope::Links` is the archived list's three shapes
plus `c` and the opening key as a second `Back`; nothing in it mutates (a link opening changes
nothing the daemon owns). Nothing to list is a status line (`no links in T-12`), never an empty
dialog. `Mode::Links` captures the list at open, so a snapshot mid-dialog cannot shrink it under
the cursor.

**Two markdown-link parsers now.** `rich.rs::link` draws a body and returns painted spans;
`links.rs::markdown_link` reads one. They agree on `[label](target)` and share nothing else, and
rich.rs's "a terminal cannot follow a link" comments now say the zone cannot. Goldens:
`links_120x30`; `test_no_drawn_structure` sweeps the dialog.

**Not built.** A link mark on the card; agents seeing links (`get_ticket` carries the
description, which is where they are); directories as links; OSC 8 hyperlinks in the preview
zone (SGR 4 stays the chip's, and an underline is not a link here). `~/` resolves through
`$HOME`; a relative path joins the ticket's dir with `./` and `../` folded.


## The release gate leaves room for a cold run (2026-09-06)

Cutting v0.1.0-alpha.15, `ci/release.sh`'s test step ran past `ci/test-run.py`'s 1200 s
deadline with every test green. A version bump relinks every crate and all ~35 e2e binaries, and
macOS holds an executable it has not seen before on its first exec (XProtect, in syspolicyd): the
audit registry's timestamps showed one e2e binary per 30–60 s of wall clock against 1–20 s of
reported test time, and the rerun with the same binaries already judged took four minutes. The
gate's suite is `cargo test --workspace`, which runs the e2e binaries one after another, so the
stalls added up. (Inferred: the system log that would name the scan is closed to a non-root
reader; a relinked `tags_e2e` alone took 24 s wall against 1.4 s of test time, and 1.8 s the
second time.)

**What was built.** `cargo test --workspace --no-run` before the wrapper, so the deadline is
spent on tests and never on a relink, and `--timeout 2400` on the release's run. The stall is
paid once per binary and nowhere else, so forty minutes still catches a hang.

**What was built first and REVERTED the same hour.** A warm-up step that exec'd every test
binary with `--list` at once. It took 3.5 min for thirty-six binaries of which two were fresh,
and while it ran BOTH live boards hung: their daemons' journals each hold one `slow turn: tick
took ~48000 ms ∙ slowest stage probe_activity` at that minute — the writer thread forking tmux,
waiting behind the scanner like every other exec on the machine. The serial cold run of the
first gate stalled no daemon (no slow turn in either journal for its twenty minutes). Serial
first execs are harmless to the boards; parallel ones are not, and a gate must never cost a
running board. **Do not warm test binaries in parallel on macOS.**

**Why not nextest.** It would overlap the first execs the same way — the same saturation — and it
changes what the gate runs (the hook, m3 and interrupt e2es have flaked under a full parallel run
on a loaded box). The serial suite has been the thing passing; it stays.

**Why not the machine.** System Settings → Privacy & Security → Developer Tools exempts an app's
descendants from the Gatekeeper assessment, and the author added iTerm2 on 2026-09-05. A
relinked `tags_e2e` still paid 22 s from a mesimon pane: the private tmux server is reparented
to launchd, so nothing under it descends from iTerm2 for the responsible-process check. A gate
that depends on a per-machine setting the machine cannot honour is no gate. Untried: adding the
tmux binary itself to that list; running the gate from a plain iTerm2 tab.

## The detach hint spells its keys the way the footer does (T-261, 2026-09-06)

The private tmux server's status line said ` Ctrl+]/^5 back ` — two spellings of "control"
in ten cells, on the one line a user reads while the board's own footer (`^k`, `^t`, `^s`;
`keymap`'s `Key::Ctrl(c)` renders `^c`) is out of sight. It reads ` ^]/^5 back ` now. The
literal lived twice — in `conf::render` for fresh servers and in `TmuxBackend::set_status_left`,
which pushes it live to servers that predate a conf change — and drifted apart is exactly how
it would go wrong again, so it is one constant, `conf::STATUS_RIGHT`, and
`the_detach_hint_speaks_the_footers_language` pins the caret spelling and the conf's use of
it. Prose keeps `Ctrl+]`: the first-run check's sentence and `doctor`'s `back to board` line
are sentences, not hints, and `Ctrl+5` is the keymap's own name for the banned atom's one
whitelisted spelling.

## The queued ask goes in board order (T-263, 2026-09-06, user: "so that user can sort while items are queued")

**What was wrong.** The 2026-09-04 queue was first-come: `drain_queue` walked `Daemon::queued` in
insertion order, so with three asks parked on one checkout the order they would go out in was
the order the user had typed them, shown nowhere and changeable only by dropping and re-queuing.
The merge train already read the board (`train::plan`: column order, then row order), and the
user asked for the same rule — "top first bottom last" — so the cards ARE the queue.

**What was built.** `Daemon::queue_order` ranks every queued entry by `(column, row)` off
`sorted_columns` / `column_tickets` at read time — never stored, so a hand move is the whole
edit and there is no second order to drift; a ticket the board no longer lists sorts last and
the sweep drops it. `drain_queue` walks that order and still pastes ONE ask per quiet checkout
per pass (the paste makes it busy again), taking the entries out highest index first so the
lower indexes stay valid. `ask_waits_on` is what `Response::Queued { behind }` and the
snapshot's `Pending.waits_on` carry now: the checkout's working tickets, then the asks queued
AHEAD of it on the same checkout in board order — so the card's `queued ∙ after T-3 +1` counts
the queue too and the `+N` falls as the card is moved up; the pending list itself is in board
order. `QueuedAsk.queued_at` stays on the record, unread. E2e `ask_queue_e2e::
queued_asks_go_in_board_order_and_a_move_resorts_them`: two asks on one held checkout, the
column reordered while they wait, the top card's ask lands first and the other keeps waiting
on it.

**Not changed.** Everything the first block says about dropping, replacing, in-flight and the
sweep; automove still parks a finished ticket at the TOP of its column, so left alone the last
ticket to finish still asks first — the same shape the train has.

## Columns own their automations (T-117, 2026-09-06, user: "the goal is to not have magic after this")

**What was wrong.** A column was `{ name, order }` and every automation the daemon ran was keyed
to a column-name literal: `automove.rs` held `TODO`/`IN PROGRESS`/`REVIEW`, `train.rs` read
`REVIEW`/`IN PROGRESS`, `server.rs` had `SLEEP_SAFE_COLUMN = "DONE"` and a literal `"DONE"` in the
merge gate and in `agent_allowed_columns`. Nothing on any screen said so, there was no wire
command to add, rename, reorder or delete a column, and a hand-edited rename in `columns.toml`
silently orphaned every ticket in it (`readd_missing_columns` runs only on the quarantine path).
The corpus had designed column policy files under a trust gate (D9/D11, docs/16 §4) with a leader
key that was never built; none of it shipped.

**Decisions (with the user, 2026-09-06).** Sort is ONE-SHOT — an action, not a standing order,
so every gesture keeps working after it. The column header is a CURSOR POSITION
(`cursor_row: Option<usize>`), not a leader menu or a menu-only door; an empty column is its own
header, which keeps every `has_ticket` gate as it was. "Start claude on arrival" fires on
CREATION only — a person at the composer — never on a move, an agent's `create_ticket`, a snooze
wake, an unarchive or an undo. "MCP permissions" is mesimon's OWN tools, tiered (`off`/`read`/
`annotate`/`full`), not a Claude Code tool deny list — that axis (D33j-2's tool capability) is
deferred and would be a second setting.

**What was built.** `ColumnSettings` flattened into `Column` (serde `flatten` through `toml`
round-trips; every field defaults and is skipped at its default, so a file says only what was
chosen); `COLUMNS_SCHEMA` 4 with the v3→v4 seeding by name through `board::template_settings`,
the one literal table, pruning a rule that names a column the board lacks; the six commands
(`AddColumn`, `RenameColumn` — every ticket file, archived included, every rule, the move gate,
the grace band — `DeleteColumn` refusing live tickets and the last column, `ReorderColumn`,
`SetColumnSettings` whole-struct so the two rule targets validate together, `SortColumn`);
`automove(&ColumnSettings, ..)`; `train::plan` on `TrainReach`; the DONE gate and the reclaim set
on the columns; `mint_ticket` stamping the column's workspace default onto the ticket. The
three new settings: `claude_mode` on `--permission-mode` (the enum has no bypass; the flag is now
in `resume_argv`'s `owned` list so a wake re-applies the column — before this a wake kept the mode
it was born with, which was fine while the mode came from nowhere but the user's file);
`agent_tools` listed by the shim off `--tools <word>` on the blob's argv and admitted by the
daemon at every call against the ticket's column NOW (a hand move narrows or widens a live
session; Claude caches `tools/list`, so a live pane keeps SHOWING what it was born with and
reads the tier in the refusal); `auto_run` as the composer's Shift+Enter fired by the daemon
inside `create_ticket`, which needed three ordering fixes — the composer's workspace rides
`CreateTicket { workspace }` (a later `SetWorkspace` would hit the spawn's lock), the brief is
read at PASTE time so the composer's note written after `Created` still travels (`brief_e2e`
unchanged), and `Created { started }` keeps the composer from starting a second. The TUI: the
header cursor and its four verbs (Enter, `r` in place, `HJKL`, `d d`), `O`, the dense dialog
(thirteen rows at one line each — `draw_list`'s two lines a row does not fit `MIN_H` 20),
`h`/`l` on the sort row only, the Name row as a text field with the scope `Input`, the menu's
two rows, the header's ` →` mark and cursor bar, pinned spines in `board_geometry`, the
composer starting at the column's default with ` (column default)` on its row. `doctor` prints
`columns`. The `X`/menu wording stopped naming `done`.

**Traps found on the way.** `every_menu_row_is_spelled` trims labels: a `format!("Name: {}", x)`
with an empty `Ctx` word is a stem. `clamp_screen` closes the dialog whose column is gone, so a
rename must move the dialog's subject BEFORE the refresh. The move gate refuses an agent undoing
a hand's move inside a minute, which is the gate's business and not the tier's (the
`agent_tools_e2e` moves to a third column). `MESIMON_CLAUDE_HOME` is the harness's scratch dir,
so a test that wants a `defaultMode` writes `settings.json` there.

**Not done.** WIP limits; per-column model/effort (D14); Claude Code's own tool deny overlay
(D33j-2, a `permissions.deny` in the `--settings` file); auto-run on an agent's `create_ticket`
(D32b — needs a spawn budget first); undo for column operations; a CHANGELOG entry (written at
release, as for every post-alpha.15 ticket).

## The tmux status line can sit at the top (T-264, 2026-09-06, user: "preference")

The private server's status line — the breadcrumb and ` ^]/^5 back ` — sat at the bottom of an
agent's pane because tmux's default put it there, and nothing had asked. It is a Settings row
now, `Status line at the bottom` / `at the top` (`Verb::StatusLine`, under the replies row), a
per-machine preference like the train's (`prefs.json::status_line_top`, off) that the daemon is
TOLD rather than reads: `Command::SetStatusLine { top }` (denied to agents — chrome over the
user's own panes; unlogged, since the feed is what the board did and this is neither) →
`TmuxBackend::set_status_position`, which takes BOTH roads at once because a running server
never re-reads its conf — the conf is re-rendered (`conf::render` takes the side now, so the
NEXT server comes up on it) and a live server gets the `set-option`. No server is not a
failure: the word is held and the conf carries it. The snapshot reports the side the daemon
holds (`Response::Board.status_top`, serde default bottom) and `App::reconcile_status_line`
pushes the preference whenever they disagree, on the train's 30 s back-off — in EITHER
direction, unlike the train, because the file is the machine's and bottom is a choice too; the
first push is from `lib.rs` before the event loop, so a daemon that outlived the last board
converges before the first attach. `set_status_left` re-sends the side beside the right-hand
hint on every breadcrumb change, so a server predating the daemon holding the preference lands
on the first focus. `doctor` prints a `status line` line. E2e `status_line_e2e`: set before any
server exists and the first server comes up on top (the conf road), set against it and it
moves (the live road). Golden `settings_120x30` grew the row; the marquee test's train row is
sixth now.

## A tool in flight survives a reload (T-265, 2026-09-06)

The board reloaded (`U`) while T-117's claude was two minutes into a `cargo build` + `cargo ut`
Bash call, and the card wore no working mark for a further minute and three-quarters, until the
tool returned and its `PostToolUse` frame promoted the idle record (T-228's rule). The restart
backfill (`server.rs::resting_hint`, the one look at how an `Unknown` session's transcript
RESTED) had read the trailing record — an `assistant` with only a `tool_use` block — as
`TailEvent::Other`, the bucket a trailing user or attachment record shares, and judged it on
the file's mtime: quiet past `TAIL_QUIET_MS` (45 s) is `StaleQuiet`, so the session was seeded
`Idle{Unknown}` at Low. But a tool in flight writes NOTHING to the transcript for its whole
duration — Claude Code records the call before it runs and the result after — so a build or a
suite keeps the file still for minutes while the pane is busy, and every reload during a tool
longer than 45 s read as idle. Nothing else could lift it sooner: the observer hooks no generic
`PreToolUse`, the status-file probe acts on `idle` only (the file said `busy`), and the pane
probe only ever demotes.

Now the record IS its own event: `adopt::classify_tail_record` returns `TailEvent::ToolInFlight`
for a textless assistant record carrying a `tool_use` block (the two human-facing tools keep
`NeedsHuman`; text beside the call is still `AssistantText`, which already meant Running), and
both roads seed `TailHint::ToolInFlight` → `Running` at Low — the backfill regardless of the
file's quiet, and the live poll for a lost session. The evidence is the BLOCK, never
`stop_reason: tool_use`: current Claude Code writes one record per content block and stamps the
whole message's stop reason on each, so a mid-turn thinking record carries it too (measured on
T-117's transcript: 72 thinking records with `tool_use`). A turn that really died mid-tool is
still caught — `probe_activity` demotes a Running pane quiet for `PANE_QUIET_MS`, and a tool in
flight keeps the pane painting through Claude's spinner, which is the clock a tool obeys where
the transcript's does not. Tests: `a_textless_tool_call_is_a_tool_in_flight` (adopt),
`transcript_hints_are_always_low_and_silent` (attention), `last_event_reads_a_trailing_tool_call_as_in_flight`
(tail). Not done: no e2e — the shape needs a restart mid-tool with a stub that holds a tool open,
and the three unit tests pin the whole road but the wiring.

## A branch merged upstream reads merged (T-267, 2026-09-06, user: "at work I don't merge to main. I use PR ∙ when PR is merged and local is fetched / pulled, I want the ticket to at least show that the worktree is considered merged ∙ PR merge can be squashed, so that's a thing to know")

**The bug.** M4's merged test was one line of the spec — "branch tip ancestor of default branch
(`merge-base --is-ancestor`)" — and `compute_flags` says the same thing in its counts: merged ⇔
nothing ahead. A pull request **squashed** on a forge leaves not one of the branch's commits
behind, so both answers are no, forever. The author's work board therefore showed every landed
ticket as `⎇↓ main moved`, offered `m ask the agent to rebase` on work that was done, would have
had the merge train ask for that rebase on a loop, and refused DONE with "worktree unmerged".
The base is also a LOCAL branch name, and the squash lands on `refs/remotes/origin/main` — so a
board where the user fetches rather than pulls could not see it even in principle.

**The rule now has two clauses.** Ancestry, as before; or the branch's **patch** is already on
the target — which is how git itself answers this (`git cherry`). The target is one ref per
pass: `origin/main` where there is one holding everything the local base holds, else the local
base. That condition answers the other case for free: a squash merged here and not pushed leaves
the base ahead of the upstream, and the base is then what to look in.

**It writes nothing, and that is the design constraint, not an accident.** The usual trick is
`git commit-tree` on the branch's tree to mint a dangling squash commit and hand it to `git
cherry`; that writes a loose object into the repository, which README promise 1 does not allow
and no user asked for. So `worktree::content_merged` computes both sides itself and compares
patch-ids: **ours** (the branch as one patch, `mb..branch` in a single `diff-tree`) against
**theirs** (`log -p` over the target since the merge base), and only if that misses, **each** of
the branch's own commits against the same set — which is what a rebase-merge lands. Restricting
the target side to the files the branch touched is not a narrowing: `patch-id --stable` sums the
file stanzas independently, so a squash that also touched a lockfile still matches a branch that
did not.

**Three things were measured, not assumed** (the design was pressure-tested against git 2.50.1
before it was built):

- **`--no-renames` must be on both sides.** Rename detection is on by default, it changes the
  id, and its pairing depends on which paths are in the diff — so the path filter on the target
  side would flip it there and nowhere else. The path list is read with `diff-tree --name-only
  -z --no-renames` and fed back under `--literal-pathspecs`, because `--name-only` quotes a
  non-ASCII path and a file may be called `x[1].txt`.
- **Four config knobs are asymmetric or fatal**, so `GIT_PINS` pins them on the command line:
  `diff.orderFile` naming a file that is gone kills every diff (and `-c diff.orderFile=` is
  equally fatal — it is `/dev/null`), `log.follow` reaches `log` and never `diff-tree`,
  `log.abbrevCommit` turns the commit-id column into forty zeros, and a `format.pretty` with an
  unindented body lets a commit message quoting a patch split one commit into two ids. The
  `log` side therefore pins `--pretty=tformat:commit %H` and `--no-abbrev-commit`.
  `--full-index` is for binaries, whose ids carry abbreviated blob oids and would otherwise
  drift as the repo grows.
- **A window into a growing history expires.** The first cut walked the newest 200 commits, and
  the reviewer's measurement is what killed it: `--max-count` takes the NEWEST N, so the squash
  falls out of the window once the base runs on, and a merged ticket would silently read
  unmerged again. Two answers, both kept: the walk is anchored to a WEEK before the branch's own
  last commit (`--since=@…`, from `%(committerdate:unix)` added to the `for-each-ref` that was
  already being made — on a busy repo that is hundreds of candidates down to a handful), and a
  verdict that NAMED a commit is re-affirmed forever with one `merge-base --is-ancestor` on that
  commit, because a squash commit never leaves the target's history.

**The cost stays where T-216 put it.** `ContentSeen` rides `FlagInput`/`Flags` in and out of the
sample, so the steady state is the same `2 + n` forks and three string comparisons; a fetch
moves the target and stales every memo at once, so `CONTENT_SCANS_PER_PASS` (2) caps how many
bindings may re-scan in one pass — a merge noticed 20 s late is invisible, thirteen walks of the
base's history on the worker are not. A branch whose tip moved is re-scanned outright: work
committed after the merge is work that has not landed.

**One flag, so everything downstream followed with no code.** `merged` is what the card's `⎇✓`,
`App::merge_stage`, `train::plan`'s skip and the ticket page all read, so the mark, the train and
the `m` offer changed together. The two gates — DONE (`requires_merge`) and delete — go through
`ticket_merged`, which now asks ancestry fresh and then falls back to the sample's verdict with
the branch tip re-read; **the gates must answer as the card does**, or a ticket that says merged
is refused DONE. `merge_ticket` was routed through the same oracle, so `m` says "already in
origin/main" instead of "main moved — rebase first". Teardown needed nothing: it deletes a merged
branch with `git branch -d`, git refuses that for a squashed branch, and the branch survives —
which is the conservative outcome, and `-D` still needs the user's explicit discard.

**And the sample had to learn to speak.** `on_worktree_flags` broadcast only when the merge
train had acted, so the flags could change on the tick and reach no board until the next thing
happened — which was survivable while every flag moved on the back of a commit or a hook, and is
not survivable for a merge made somewhere else: a fetch that lands a squash moves no session and
fires no hook. `absorb_worktree_flags` now returns whether any of what the board draws actually
changed (the memo is the sampler's own working note and does not count), and the tick's road
broadcasts on that, the way `on_git_sampled` already did for the header's own clause.

**The words.** The card says only `⎇✓`: the mark already means "this branch's work is in main",
and line 1 has no room for a second one. The ticket page's state row is where the news goes —
`∙ merged` stays exactly as it was for an ancestor of the checkout's own default branch, and
anything else names the ref and the commit: `∙ merged into origin/main as 1a2b3c4`. `merged_in`
is empty for the ordinary merge for that reason. `mesimon doctor` prints a `merge base` line.

**Not done.** No forge integration — no `gh`, no `glab`, no PR state, no push, by the author's
choice ("I don't necessarily want gitlab / github / pr integration"). The verdict lives in
memory, so a daemon restart re-scans (bounded by the same window). A merge older than the window
that mesimon never saw is not found — the safe way round. Two free corroborating signals were
found and left on the table: the branch's remote-tracking ref disappearing after `fetch --prune`
(what "delete branch on merge" leaves behind), and `--grep='(#N)'` where a ticket's notes carry
the PR link. Tests: six units beside `compute_flags` (squash, rebase-merge, durability across
later commits on the same files, a commit after the merge, the fetch-only upstream case with a
real bare remote, and the empty/missing negatives), `pr_merge_e2e`, golden
`ticket_merged_upstream_120x30`.

## `!` is the project's terminal (T-273, 2026-09-06)

Asked for as "pressing `!` will attach a global tmux for the current mesimon project — git fetch /
pull / push without the need to exit mesimon or create a tab; `!` on a worktree ticket page will
open tmux on the worktree dir". Until now `!` was the diff viewer's `WorktreeShell` (M4b): `$SHELL`
forked by the TUI in the FOREGROUND over the handover road, in the worktree, inert on the checkout
diff ("a shell in the checkout is one you already have"), and gone the moment it returned — no
tmux, no record, nothing to come back to.

What shipped: `Verb::Terminal`, one verb on three screens, and the SCREEN says the directory
(T-221's rule for `v`): the board opens the checkout root, a ticket's page its attached worktree
(else the checkout), the diff its own target. The daemon owns the pane —
`Command::OpenTerminal { ticket }` → `Daemon::open_terminal`, `Response::Attach`,
`Command::TerminalEnd` — so the same `!` finds the same shell from every screen and after a
reload.

- **Not a `SessionRecord`.** `SessionRecord.ticket` is a plain `Ulid`, and every card, rail,
  quiet gate (`checkout_holders`), reaper and worktree lock reads a session as a ticket's. Widening
  it to an `Option` touches every `s.ticket ==` site and the back-compat fixtures for a thing that
  is a place to stand, not work on a ticket. The precedent was already in the tree: the first-run
  GATE's `msmn-gate`, a named tmux session on the private server that the board never lists and
  `reconcile` returns as `foreign`, which the daemon never reads. The terminal is that with the
  user's shell: `msmn-term` for the root, `msmn-term-<ticket ulid>` for a worktree
  (`terminal_name`).
- **One per DIRECTORY, persistent.** Alive (listed, not `pane_dead`) is reused, so a `git pull`
  in flight survives a detach and a `U`; `exit` leaves a dead pane under `remain-on-exit`, and the
  next `!` kills and respawns it — the gate's own rule. Spawned through `Daemon::launch`
  (`mesimon exec --env`), so the shell has the user's exports and PATH; a worktree's also carries
  `MESIMON_TICKET` / `MESIMON_WORKTREE_BRANCH` (`session_vars`), the root's no ticket variable —
  it is global.
- **The ULID in the name, not the key.** Teardown (`process_teardowns`) kills the worktree's
  terminal before `worktree::remove` — never remove a live cwd, and the reaper never saw this pane
  because it is no session — and it runs after the ticket left the board, over a
  `worktree::Binding` that carries no key. A name derivable from the binding's own key alone is
  what makes the kill unconditional.
- **The focus token widened.** `Daemon::focus` was `Option<Uuid>`; it is `Option<Focus>` —
  `Session(uuid)` | `Terminal { ticket }` — so a focused session keeps the terminal out, the
  terminal keeps `FocusStart` out, and a second `!` while the terminal holds the token is
  allowed (it is the same target). The status-line breadcrumb names the ticket whose worktree a
  terminal stands in, or nothing at the root, with leaf `terminal`. In the TUI the same
  widening is `FocusTarget::Session(uuid, origin)` | `Terminal` in `pending_gate_then` /
  `focused_session_hint`, and `App::focus_target` is the one focus road (GATE, then the grant),
  which `focus_session` now takes too. The terminal's return sends `TerminalEnd` and stays on
  the screen the key was pressed on: no origin, because nothing was selected.
- **A worktree still provisioning is refused in words** (`worktree not ready yet`), never the
  root by surprise; a ticket page with no worktree is the checkout's, which is what a
  shared-checkout ticket's shell would be anyway.
- **Hints.** The board's binding is `prio: 0` and the header's git clause draws ` ! terminal`
  after ` v diff` — the row that names the checkout it opens, T-158's idiom — whatever the change
  count, since a fetch is what a clean checkout wants; it is the first rung dropped when the row
  is tight, before `v diff`, before the count. The ticket page's footer reads `! terminal` /
  `! terminal in worktree` (the rail's trailer names the rail's own sessions, and this is none of
  them); the diff's the same on `worktree_present`, and the checkout diff now offers it.
  `checkout_diff_says_uncommitted_and_offers_no_worktree_shell` became
  `…_offers_the_checkout_terminal`; `WorktreeItem.path` still travels for the ticket page's
  "has it a directory" and the `^k` road keeps `pending_attach_cwd`.
- Agents are denied both commands (`agent_allows`). No `doctor` line: the terminal is transient
  and `tmux ls` on the private socket says what exists.

Tests: `terminal_e2e` (attach argv, one pane in the checkout, no record on the board, the token
both ways, reuse, exit → respawn), the worktree case in `worktree_e2e` (in the worktree, killed at
the discard teardown), `the_terminal_opens_the_screens_directory_and_returns_to_it` (app), the
resolve test in `keymap.rs`, the three git-clause ladder tests, twenty goldens.

## The release gate honours a stamped pass (2026-09-06, user: "doesn't make sense to me how long all this takes (it's not the first time I'm talking about this, hurting us a lot)")

**What it cost.** alpha.16's gate spent ~25 minutes in `cargo test --workspace` for ~2.5 minutes of
tests, and the turn before it verified the tree twice — once before the version bump and once
after, because the bump relinks every crate — so one suite ran three times. The 40-minute deadline
alpha.15 added was a budget for the wait, not a fix for it.

**What the wait is, measured — and it is the directory, not the file.** A freshly linked 6 MB e2e
binary takes **25 s** on its first exec with `user 0.00 sys 0.00` (held before it runs) and 0 s on
the second. The first guesses were wrong in turn, and each was refuted by a measurement: not a
network timeout (the notarization lookup is one 76 ms HTTP round trip, status 200, then `Code did
not match any currently allowed policy`); not the size of the binary (`target/debug/mesimon` at
23.6 MB, relinked and exec'd by a hook the same second, scans in 0.5 s — 42 of 45 scans in the
log window took 0.3–0.6 s whatever their size); not the terminal's *Developer Tools* grant (24 s
from a plain iTerm2 tab with iTerm2 listed); not a warm-up window (two binaries linked together
and exec'd back to back: 25 s and 29 s). The live sample says where the time goes: **~15 s of
`syspolicyd` itself at 50–73% CPU** (14 s of CPU time) before it even calls XProtect, then a
**12 s XProtect YARA pass** at ~23%. And the decisive probe — one byte of a string patched so the
content is new, re-signed, the identical file exec'd from three directories — reads **0.56 s** from
an empty scratch dir, **0.35 s** from `target/debug/` (14 entries), **37 s** from
`target/debug/deps/`, which held **879,272 entries and 50 GB** — accumulated since 2026-08-30,
ONE WEEK: every relink of 36 e2e binaries leaves its split-debuginfo `.o` files and its old-hash
binary behind, and cargo collects nothing. So Gatekeeper's first-exec evaluation walks the
executable's directory (the "direct malware and dylib scan" looks at its siblings), and a directory
of 879 k files costs 25–37 s per fresh binary — which is also why the 2026-09-05 memory of "~30 s
per binary" was true and its explanation was not. `zsh` has a builtin `log`, so `/usr/bin/log show`
is the command that reads the unified log; every earlier `log show` in the session ran the builtin
and read nothing. The Docker Desktop VM had been at 104% CPU for four days throughout
(`com.apple.Virtualization.VirtualMachine`, parent `com.docker.virtualization`) — unrelated to
the hold, and reported to the author.

**The change.** `ci/test-run.py::stamp_pass` writes `target/suite-passed.json` after a clean run
of a FULL-workspace `cargo test`/`nextest` on a CLEAN tree: HEAD's sha, the tmux that drove it
(`MESIMON_TMUX_BIN` resolved) by path and sha256, and the time. A partial command never stamps
and a dirty tree says so and does not (the commit is not what ran). `ci/release.sh` honours the
stamp only when it names HEAD and the BUNDLED tmux by hash — a run on the homebrew tmux, or a
vendor tmux rebuilt since, is refused — and then skips the link and test steps with a step line
saying so; `MESIMON_RELEASE_RETEST=1` runs them regardless. The gate's meaning is unchanged: the
whole suite, at exactly the commit that ships, on the tmux that ships. What changed is that it is
proved once. Verified in a scratch git repo: partial command → no stamp; clean tree → stamp;
dirty tree → refused; the release's compare honours the match and refuses another HEAD, another
tmux path, and the same path with another hash.

**The release order now.** Bump + CHANGELOG → commit → `MESIMON_TMUX_BIN=$PWD/vendor/tmux/tmux
python3 -B ci/test-run.py --jobs 4` (nextest, parallel, bounded so the first-exec scans do not
stampede the machine — 36 at once hung both live boards on alpha.15) → tag → push →
`ci/release.sh`. Nothing is verified before the bump.

**The remedy, measured.** `cargo clean` removed 1,072,418 files / 84.8 GiB (four minutes of
deleting), the rebuild took 13 s and linking every test binary 10 s — the 83 s relinks of the
morning were the same directory tax on the linker — and a fresh e2e binary then exec'd in
**0.34 s**. The whole suite through `ci/test-run.py` (nextest, the bundled tmux, first execs
included) is **53 s** wall. The regrowth was then measured: a clippy pass adds ~230 entries, and
ONE edit to a core crate plus a relink of the tests adds **6,570 files, every one a
split-debuginfo `.rcgu.o`** named by a content hash — the old set is never removed. Of the
13,255 loose objects after that edit, 5,687 were named by a current binary's `OSO` stabs and
7,568 were orphans (none referenced-but-missing), so the orphan set is decidable from the
binaries themselves. `ci/prune-deps.py` deletes exactly that set under cargo's own build lock
(`target/debug/.cargo-lock`, so a link in progress is never robbed of an object), keeps every
referenced object (which is what keeps file:line in a panic's backtrace), and advises `cargo
clean` past 50k entries; `ci/test-run.py` runs it after every bounded check. `--jobs 4` is gone
from the release order: at 0.4 s a scan, parallel first execs are harmless.

**Not done.** Old-hash EXECUTABLES (a version bump changes every metadata hash) are kept by the
prune because their objects are still "referenced" — by them; they are 36 files a release and
the 50k advice catches the drift. `ci/__pycache__/test-run.cpython-314.pyc` is tracked in git
and should not be.

## The cursor is not put on a pinned column (T-276, 2026-09-06, user: "when entering / refreshing TUI, prefer not land on a collapsed column")

T-117's pinned column is a spine "unless the cursor is in it", and the cursor is in it whenever
`cursor_col` says so — including the two times nothing the user did put it there: the launch,
which starts on column 0 (the author's first column is a pinned `Automations`, so every board
opened with it unfolded), and a snapshot that pulls the cursor's column away (deleted, or
reordered by another client), where `clamp_cursor`'s `min` lands on whatever now holds the
index. `App::leave_pinned_column(was)` runs after both — `App::new` with `None`, `absorb` with
the name the cursor stood on BEFORE the board was replaced — and steps to the nearest expanded
column, rightward first (a deleted column's neighbours slide in from the right), leftward
otherwise, staying put on a board of nothing but spines. **The name is the gate**: a cursor
still on the column it was on is the user's own `h`/`l` into the spine or the collapse they just
chose on it from the column dialog, and both keep the cursor, so `golden_board_pinned_120`'s
"the cursor entering it expands it" still holds. `select_ticket` (Esc from a ticket page) is
untouched: it aims at a card, and the card is the point. No key, no `Ctx` field, no golden
moved. Pinned by `the_cursor_is_not_put_on_a_pinned_column`.

## The terminal's key is listed only in `?` (T-277, 2026-09-06)

Asked for as "remove `! terminal` hint, keep only on `?` help menu", a day after T-273 put it in
the board header's git clause (after ` v diff`), the ticket page's footer and the diff's footer.

What changed: all three `Verb::Terminal` bindings are `prio: 0` now and `chrome::git_clause`
draws only `v diff`. The words survive — `terminal` / `terminal in worktree` — as the row `?`
lists on each screen, so the overlay still says which directory the key opens. The drop order in
the git clause is the T-221 one again: `v diff` first, then the count, then the name to its
floor.

Why: `!` is a standing key — it is available on every board, whatever the state — and a hint
that is always there is not telling the user anything about the moment. The footer and the
header's clause are for what the SELECTION or the checkout's state makes possible now; a key that
never changes is what `?` exists to list. `test_git_clause_gives_way_to_the_offer` lost its
"terminal goes first" rung and `checkout_diff_says_uncommitted_and_offers_the_checkout_terminal`
asserts the overlay's row instead of the footer's. Eighteen goldens lost the cluster.

## The archive reclaims a landed worktree (T-278, 2026-09-06)

Until now `archive_ticket` never touched `self.worktrees` ("reversible, binding + branch
persist", the M4a block above): only a delete tore a worktree down, so the archive was where
the disk went to hide — on 2026-09-06 the mesimon board held 16 worktrees, 29 GB, and 14 of
them belonged to archived tickets whose branches were already ancestors of main, each carrying
a ~2 GB `target/`. Reclaimed by hand that day; now by the archive.

**The rule** (`Daemon::reclaim_on_archive`, the pure gate `worktree::reclaim_on_archive` with
a unit test): a ticket archived through `archive_ticket` whose binding is `Attached` (or
`Evicted` with a branch still to delete), whose branch is MERGED by the same oracle the card
and the DONE gate answer with — `ticket_merged`: an ancestor of the base, or the sample's
patch-id verdict (T-267), so a squashed PR counts — and on which nothing holds a pane (the
archive gate already refuses an awake session) is pushed onto `pending_teardown`, the delete's
own road. Unmerged work keeps its worktree exactly as before: the archive stays reversible
for work that has not landed. **A snooze takes none of this** — a snooze is a return, and its
ticket comes back to the same tree. `worktrees_barred` keeps everything standing (D26).

**The teardown entry grew a reason** (`Teardown { ticket, why: Deleted { discard } |
Archived, sids }`) because what stays behind differs. `process_teardowns` judges an archived
ticket's again on its turn — still archived (a restore inside the tick cancels it), still
merged (a T-273 terminal standing in the tree could have committed meanwhile) — then kills
the terminal, removes the directory (single `--force`) and tries `branch -d`; `-D` stays
behind the user's explicit discard, so git refuses a squash-merged branch and that is the
conservative outcome. **The binding follows the branch**: where the branch went, the binding
goes with it and the next spawn provisions fresh (the same name minted again off the base);
where it survived, the binding stays `Evicted` — "directory removed, branch kept", the status's
own meaning — and `queue_provision` replays it through `provision_existing`. Feed line
`worktree_torn_down` / `worktree_torn_down:branch_kept`, actor `automation`.

**A wake rebuilds the tree.** The ticket's sleeping and exited claudes keep their records, and
their `cwd` is now a directory that does not exist; `resume_session` used to refuse that
outright ("never silently relocate an agent"). Rebuilding the ticket's OWN worktree is not a
relocation: on a `Worktree` ticket whose cwd is gone the wake goes through `resolve_spawn_cwd`
like a first spawn — an attached binding is used at once, otherwise the wake is parked
(`pending_resumes: Vec<PendingResume>`, replayed by `on_provisioned` beside the parked spawns,
dropped on a failed provision) and answers `Response::Provisioning`; the record's `cwd` is
restamped when the pane opens. `prompt_sleeping` parks the ask's words on the entry
(`PendingResume.prompt`), and `Daemon::park_prompt` is the one place both roads land them. In
the TUI the wake road and the ask both take a `Provisioning` arm (`provisioning worktree ∙
claude wakes when ready`), and `settle_pending_spawn_focus` looks for a session with a PANE,
not a live one — a Sleeping record `is_live`, and focusing it would have parked the same wake
again every snapshot. A shared-checkout or adopted ticket keeps the old refusal.

Deferred: the ARCHIVED row says nothing yet (with the binding dropped the row cannot tell a
reclaimed worktree from one never provisioned; the feed line is the record), and a per-column
setting for the rule (T-117's table) — it ships board-wide. `mesimon doctor` unchanged. E2e
`archive_reclaim_e2e` (three tests: ff-merged → dir, branch and binding gone, restore + spawn
provisions fresh; unmerged → untouched; squash-merged → dir gone, branch and `Evicted` binding
kept, restore + wake rebuilds the tree with the record's cwd following).

## The board has a default column (T-279, 2026-09-06)

Asked for as "default column (omitting column in create ticket mcp will land there, instead of
first column default) — in settings". Before this, an agent's `create_ticket` with `column`
omitted landed in the board's FIRST column, a literal nobody could change short of reordering
the board.

What changed: `Board.default_column: Option<String>` — a column NAME, the foreign key every
other cross-column reference is — persisted as a scalar in `columns.toml` before the tables,
`#[serde(default)]`, absent until chosen and no `COLUMNS_SCHEMA` bump: a build that drops it
lands the agent's card in the first column, the old behaviour, and widens nothing an agent gets
(the `mcp_tools` bump exists for the opposite direction). `Board::landing_column()` is the ONE
reader — the chosen name while the board still has that column, else the first column — and
`Daemon::agent_create_ticket` asks it where `sorted_columns().first()` stood. `rename_column`
carries it, `prune_dangling_refs` clears it (so `delete_column` does, with a
`column_rule_cleared:default_column` feed line), and `Board::set_default_column` refuses a name
the board lacks — a dangling default would read as the first column while the row said
otherwise. `Command::SetDefaultColumn { column: Option<String> }` is a person's (`agent_allows`
denies it: an agent choosing where its own cards land would be choosing what the user sees
first; it names a column per call instead, in the open), `Mutate` + logged, barred under
`columns_barred`. The Settings row `Default column: TODO` (`Verb::DefaultColumn`, last in
`SETTINGS_ITEMS`, `Ctx::default_column` filled from `landing_column` UPPERCASED as the header
spells every column, and the row is absent while that word is empty) cycles the columns in board
order on Enter from the one the daemon would use now, wrapping — the week-start ring's shape.
The tool description says "omitted means the board's default column"; `doctor`'s `columns` line
appends `∙ an agent's create_ticket lands in X` only when one was chosen. Golden
`settings_120x30`; `mcp_e2e` chooses, renames, deletes and resets it on a column of its own.

Scope, deliberately: only the agent's omitted `column`. The unarchive fallback and the external
drawer's import still take the first column — the first is "the column is gone" and the second
was not asked for; a wider "wherever nothing chose" is one line each on `landing_column` if it
is ever wanted. The human's composer creates in the cursor's column and never asks.

## The reload execs the path, not the inode (T-280, 2026-09-06)

Reported from WSL with a screenshot: `mesimon: reloading…` then `Error: exec of the new binary
failed: No such file or directory (os error 2)`, the board gone and a shell prompt in its place.

The corpus and every reload path assumed `std::env::current_exe()` is a fact of the process. On
macOS it is (the path the process was started by, from `_NSGetExecutablePath`). On Linux it is
`readlink /proc/self/exe`, which names the INODE the process runs from — and `install.sh` (and
`release.rs::install`, the in-app download) land a new binary by renaming a new file over the
old, which unlinks that inode. From then on the kernel answers `/home/x/.local/bin/mesimon
(deleted)`. `update.rs` had cached the good path at startup, so it saw the new mtime and offered
`U`; `reexec` then asked again and exec'd the deleted name. The daemon had the same trap one
step wider: `hook_settings::mesimon_bin` and `spawn_detached` ask at spawn time, so a daemon
that outlived an install would have written the dead name into every new session's hook set and
the `pane-died` notify (macOS never showed it; the build-skew restart usually hid it).

What changed: `core/src/exe.rs::current_exe` is the one road to our own path — resolved on the
first call and cached for the life of the process (the TUI's update watch and the daemon's
`exe_stamp` both ask at startup), with a trailing ` (deleted)` stripped regardless, since the
path under the suffix is where the new binary now is and that is exactly what a reload wants to
exec and a hook set wants to name. Every caller goes through it (reload, update watch, release
eligibility and its build-tree guard, hook set, daemon respawn, the `pane-died` notify, the
bundled tmux's sibling lookup, doctor's `binary` line); the test seams (`MESIMON_HOOK_BIN`,
`MESIMON_DAEMON_BIN`) still outrank it at each caller. `no_source_line_asks_the_os_for_the_exe_directly`
walks the workspace's `src/` trees the way verdict's `permissionDecision` scan does, so the raw
call cannot come back under a new name; `tests/` is exempt (an e2e re-spawning its own test
binary is not shipped code). Verified on the kernel's documented behaviour (proc(5)) and the
unit tests; no Linux run, since the Docker gate is paused.

## A folded column's needs-you mark is painted (T-271, 2026-09-06)

A collapsed column is one cell wide, and its top cell is `!` when anything inside it needs you
(07 §3.1's `⊆ [!A-Z0-9 space]`). That `!` was drawn with `theme.attn_text()` — the attn colour as
a foreground stroke — which is the quietest form the one saturated colour has, on the narrowest
thing the board draws, standing in for a whole column of cards. The author asked for "a yellow
background for more aggressiveness".

It is now `theme.attn_row().add_modifier(BOLD)`: the same inverted treatment the needs-you TITLE
row and the header's `!N` chip already wear — `attn` ground, `attn_ink` on it, REVERSED in mono.
So this is a fourth CALL SITE of an existing role, not a fourth SGR-7 use: an unexpanded column
saying "needs you" is the same sentence a card's title row says, and painting the whole cell
spends the colour on the cell rather than on a stroke inside it. The bold is the header chip's,
for the same reason — one cell has no other way to get louder.

Nothing else moved. The expanded column header's off-screen `!N` badge (`ui/board.rs`) keeps
`attn_text()`: it sits in a row of other badges on the ground and is not the only mark for what
is behind it. The goldens are colourless so none drifted, and `test_attn_provenance_woke`'s
folded-board clause still holds — a spine only exists in a column the cursor is not in, and the
cell it paints is the column's own. `the_folded_column_paints_its_needs_you_mark` pins bg, fg
and weight over `Flavor::ALL`.

## An agent can ask for the user (T-107, 2026-09-06)

The loud register — the `!` glyph, the one saturated colour, the header's `!N`, the tmux status
line — had two producers: a session in the attention set (`attention::rank` 0–8, every one of
them hook-derived) and `Ticket::woke_at`, the snooze that asked to be seen. An agent ending an
ordinary turn produced neither: `Stop` → `Idle{EndTurn}` → automove to REVIEW, the unread done
mark, and nothing else. So on a board with twenty tickets and six agents, **"I finished the
refactor" and "I cannot proceed until somebody chooses an auth provider" looked identical**, and
the only way to tell them apart was to open every REVIEW card — the exact cost the board exists
to remove.

Claude Code's own `AskUserQuestion` already lights a card (`Signal::PreToolUse{AskUserQuestion}`
→ `RequiresAction{Question}`, rank 2), and it is the wrong shape for this: it FREEZES the turn on
a modal in the pane, holds the agent's context open, and takes its answer only there. The
non-blocking case — the turn is over, a person is owed a decision — had no channel at all.

**`raise_hand` is the eighth MCP tool**, `AgentTools::Annotate` (it writes on the caller's own
ticket, like a note and a tag), one required argument: `reason`, one line. It reaches exactly one
card — its own — and it is the only tool that reaches the loud register at all.

**The mark is the TICKET's, not the session's** (`Ticket::raised: Option<Raised { at, by,
reason }>`, a `[raised]` table with the tables, after `workspace` and before `[[tags]]`). A
`Reason` on `RequiresAction` was the obvious home and is wrong four times over: the `Stop` that
lands moments after the call would wipe it (the tool is called at the END of a turn — that is the
whole use case), the 15-minute stale demote would drop it silently, D28 pins the ranks forever,
and a daemon restart re-derives every session as `Unknown{DaemonRestarted}`. **The turn ending is
precisely what must not clear a raised hand**, which rules the session-state road out entirely.
As a ticket field it also survives the session being slept, killed or replaced, and it is on
disk, so a board that comes back an hour later still knows somebody is waiting.

**It is lowered by the person, never by the asker.** Three roads: leaving the ticket's PAGE
(`App::ack_hand`, `Command::LowerHand`), any `UserPromptSubmit` on that ticket's claude (hooked
beside `moves.asked_by_hand` / `ack_owed`, the one place every prompt road already ends), and the
ticket leaving the board. `LowerHand` is in the never-tier: an agent that could take its own mark
down could raise one every turn and clear it before anybody looked, and more simply, being
answered is not something the asker declares.

Two deliberate departures from `woke_at`, whose machinery this otherwise reuses whole:

- **The board cursor does not lower it.** `ack_woke` fires on every keypress that lands the cursor
  on a card, which is right for a snooze's return — novelty, discharged by a glance. An
  unanswered question is not discharged by a glance, and `!N` is only worth reading if it means
  "tickets waiting on an answer from me".
- **The page lowers it on the way OUT, not on the way in.** Clearing on arrival would blank the
  state row on the very frame the page draws, and the words are the reason the mark carries
  words at all. `App::on_key` captures the page's ticket before `handle_key` and compares after.

**The words.** Required rather than optional: a bare mark makes the user open the ticket to learn
anything, and requiring the line is the one honest way to ask the model whether it has something
to say — tool text may describe and may never instruct. Capped at `RAISE_REASON_MAX_BYTES` (160)
through `board::sanitize_reason`, `sanitize_title`'s idiom at a card row's size: the mark is a
POINTER and the transcript is the record, which is also why lowering it takes the words with it.
The receipt returns what was kept, so a trimmed line says so where the model can see it. Drawn on
the CURSOR card only, in the context row the snooze preset and the owed row share (`dim1`, a step
brighter than either — it is the agent's own sentence, not chrome), and on the ticket page's state
row as `∙ claude asked 4m ago ∙ <reason>`.

**The description could not use the product's own phrase.** `lint_tool_text` bans the substring
`"you "`, and "needs-you mark" contains it. The text says "waiting on a person" and "the board's
attention count" instead — the lint is right, and the model needs what the tool does, not the
product's vocabulary. 539 of `MAX_TOOL_BYTES`' 820; the surface is now 4264 bytes, ~1784 tokens.

**A raised hand takes the ticket off the merge train** (one clause in `train::plan`, beside
`manual_merge`): an agent that ended its turn asking for a person is saying a person looks before
this goes anywhere, and merging the branch — or asking it to rebase — would be the automation
answering a question addressed to somebody else. That also closes the one collision the
lower-on-prompt rule would otherwise have: the daemon cannot tell its own paste's ack from a line
the user typed (the queued ask states the same limit), and the train's merged-notice was the one
delivery that would have mattered. It cannot reach a raised hand now.

**`needs_you_count` counts TICKETS.** It was `attention_queue().len() + woke_tickets().len()`,
which double-counted a woken ticket whose claude was also at a permission prompt — one ticket
needing one person, shown as `!2`, matching nothing on screen. `Board::needs_you_tickets()` is
the set, `sort_column` reads the same one, and `card::needs_you` agrees with it term for term.

No `TICKET_SCHEMA` bump. The doctrine on the constant is "bump when an older build dropping the
field would WIDEN something or lose what cannot be recovered" (`mcp_tools`, `manual_merge`, the
snooze deadline). A dropped hand loses an alert whose content is still in the transcript and
whose ticket is still in REVIEW; barring every board's writes to protect an alert costs more than
the alert.

Deliberately out: no retract (a hand is lowered by the person; a `!` that clears itself is one
the user learns to distrust), no new key (opening and leaving the page is already the gesture),
no notification out of band (the tmux status line's `!N` picks it up for free). Goldens
`board_raised_120x30` / `ticket_raised_120x30`; the L3 colour law sweeps it over `Flavor::ALL`
(`test_attn_provenance_raised`, and `attn_stays_on_the_card` is the woke law's helper generalised
to take the lit card's title). E2e `raise_hand_e2e`, tier coverage in `agent_tools_e2e`.

## A merge the checkout refused says so (T-289, 2026-09-07)

Two REVIEW tickets sat wearing `auto-merge ∙ next` for hours and never merged. The train had
tried, three times, and been refused: another session in the shared checkout was holding
uncommitted work, and an ff-merge that would overwrite it is one git declines. The daemon had
the sentence — `merge_refusal_detail` maps git's "local changes"/"would be overwritten" to
*uncommitted changes in the main checkout — commit or stash them first* — and shipped it on
`Pending.text`. `App::pending_row` never read it. So the one channel that could have said why
was already built, already populated, and rendered nowhere: the card went on promising a merge
that the train had stopped attempting.

**The card says THAT, the advisory row says WHY.** `pending_row` gains one arm, before the two
that name what the merge waits on — `("merge", _) if p.text.is_some() => "auto-merge ∙ blocked"`
— because a blocked merge is not waiting for the board to go quiet, and `after T-3` there is a
promise about the wrong thing. The reason is a sentence and the owed row is 22 cells (the
`train_120x30` golden already truncates `auto-merge ∙ after T-3 ~`), so it goes where the flap
fuse's already goes: a standing `merge_train_blocked` notice in the advisory row, one per
distinct reason, naming its tickets in board order — *merge train held for T-282, T-283 —
uncommitted changes in the main checkout — commit or stash them first*. It is built from
`pending_items()`' own output rather than from the refusal map, so the row and the notice cannot
disagree; `snapshot` now computes `pending` before the notices block and hands the same value to
both. The ticket page's state row reads `auto-merge ∙ blocked` off the same function and stops
there — the row is already ~113 cells wide at 120 and only the branch name gives — and `m` is
one key away, which answers with the full sentence in the status line.

**And the fix the sentence asks for now works.** A refusal is remembered per `(branch tip, base
tip)` so a dirty checkout is not retried every bucket — but neither tip moves when the user
STASHES, which is half of what the sentence tells them to do, so the train would have stayed
stuck on a checkout it had already been cleaned out of. `on_git_sampled` calls
`Train::forget_refusals()` on the branch where the sample differs from the cache: the checkout
moved, so every verdict it handed down is stale. The cost of being wrong is one `git merge
--ff-only` that fails without writing anything; the feed stays quiet because the tip-pair key
still absorbs the passes in between. The git sample moved from `RSS_TICKS` to
`wt_refresh_ticks()` for it — the same number in production (`RSS_TICKS`), and now
`MESIMON_WT_REFRESH_TICKS` shortens the whole slow bucket rather than three quarters of it, so
an e2e that shortens the train's cadence shortens the thing that unsticks it too.

The user's first guess — that moving the running ticket to REVIEW and back while two tickets
waited on it had confused the train — was not it: a hand move clears the train's memory of THAT
ticket (`hand_touched`) and touches nothing else. The dirty checkout was.

Deliberately out: no expiry on the refusal (a doomed ff-merge every minute forever would write a
`merge_train_refused:merge` line into the feed every minute, and the feed is the record), and no
header word (the train still has no board-wide clause — the notice is a condition, not a state).
Golden `train_blocked_120x30`; e2e
`merge_train_e2e::a_merge_the_checkout_refuses_says_why_and_retries_once_it_is_clean`.

## The board says it out loud (T-282, 2026-09-06, user: "notifications (OS + sound effects)")

D15 is the decision this refutes, and it is worth saying exactly which half. `00-DECISIONS.md`
§D15, `11` §11.8.1 and `07` §2016 all say the same thing — *do not build a notification channel;
ship `mesimon watch --json` and let the user pipe it to the one they already chose* — with two
sanctioned opt-ins (`notify.attention_bell`, `notify.osc_request_attention`). The corpus's
REASONING was right and is kept whole below. Its CONCLUSION assumed the composable primitive
would ship; `watch --json` does not exist, so the board's quiet was total: an agent blocked on a
permission prompt was invisible from any other window, and the only remedy was to keep looking
at the board.

**What fires.** Two rising edges, both read off the snapshot in `App::absorb` — the one road
every snapshot lands through, and the only place the old board and the new one exist at once.
`Board::needs_you_tickets`' three roads are *needs you* — an attention-set session (rank 0–8 at
High|Medium), a snooze that woke a ticket (T-74), an agent's raised hand (T-107) — so a banner
and the `!N` chip are the same set and cannot disagree. The words beside it are
`attention::reason_word` for a session, the word the card prints; nothing for a snooze, which has
nobody to quote; and for a raised hand the AGENT'S OWN SENTENCE. That last one is why
`Event::why` is a `String` where a hint would be a `&'static str`: T-107 exists because "I
finished the refactor" and "I cannot proceed until somebody picks an auth provider" looked
identical on a board of twenty cards, and a banner that drops the sentence reintroduces exactly
that. T-107's own block says a raised hand takes "no notification out of band"; that was true
the hour it was written, and this is the out-of-band channel arriving.
`Idle{EndTurn}` is *a turn finished*, the state automove reads to move a card to REVIEW, so the
ding and the card move say the same thing. Two guards, both measured against real failure
shapes rather than imagined: **`EndTurn` counts only at High|Medium** because after a daemon
restart every session is `Unknown` and the transcript tail re-derives a finished turn at LOW for
each one — a burst of chimes for turns that ended hours ago; and **the first snapshot only
SEEDS** (`App::notify_primed`), because `U` restarts the process and an opening board must not
announce its own backlog. `App::spoke` (T-173) detects "the agent said something new" and was
the rival: it is transcript-bound (a 1 s poll, `pane_target` only) and cursor-coupled, where the
state edge arrives on the same snapshot as the needs-you edge and lets ONE differ serve both.

**Where it lives: the client.** `Change.attention_added` (`attention.rs`) is computed, deduped
against the 30 s re-emit suppression, and still has no consumer — a ready daemon-side hook, and
the wrong one. The daemon has no terminal, so the OSC rung is unreachable from it and the focus
rule unanswerable; a headless daemon raising banners for a board nobody has open is a surprise;
and client-side is zero wire commands, zero schema fields, zero daemon state and no e2e. A
closed board is a silent one, `train.rs`'s posture.

**What survives of D15, as constraints rather than a veto.** Default OFF, and the master switch
is a preference nobody's update turns on for them. **Coalesced** — `core/src/notify.rs`'s
`Coalescer`, at most one post per `WINDOW_MS` (5 s) carrying the aggregate, the window rolling
from the last thing SAID so a quiet board speaks at once and a busy one settles: twenty agents
finishing together are `20 agents finished ∙ T-1 T-2 T-3 T-4 +16`, and that multiplication is
the whole thing D15 was written about. **Quiet while you are looking**, below. What is NOT kept
is the conclusion: the user asked for the channel, and the tool that already had every fact
needed to coalesce it was the board.

**The focus rule is the user's own wording** — "banner suppressed, sound plays, opt out in
notification settings". So suppression takes the BANNER only: the card is already saying it in
the one saturated colour, and a banner over the card it duplicates is noise, but a chime is
still a cue. `notify::Presence` answers it, and its fallback DIRECTION is the design: a terminal
that reports focus (DECSET 1004, `EnableFocusChange` in `init_terminal`) is simply believed; one
that never does falls back to **keystroke presence** (a key inside `KEY_PRESENCE_MS`, 30 s), the
gate Claude Code itself uses; with no evidence at all it answers "away", so the banner fires.
Silence is the failure that would make the feature look broken, and it is the one this cannot
fall into.

**Two ladders, no new crate.** `opener.rs`'s shape throughout — env var, then the platform's
program, then a rung that always exists — resolved once in `lib.rs::run` and never `App::new`,
so no test app and no golden makes a noise. Banner: `MESIMON_NOTIFY` (`off` | `osc` | a program)
→ `terminal-notifier` → `osascript` on macOS → `notify-send` on Linux → **OSC 9** to our own
stdout, the rung that cannot fail to resolve; on a terminal that draws it the banner comes from
the terminal, on one that does not nothing happens, and an outer tmux of the user's own swallows
it — which is why a helper outranks it. No DCS wrap: the board is not inside mesimon's private
server (`15`, crossing ⑩). Sound: `MESIMON_SOUND` → `afplay` → `paplay` / `pw-play` /
`canberra-gtk-play` → the terminal bell. A crate was never in question: `notify-rust` reaches
`mac-notification-sys` and dbus, and `ci/build-linux.sh` cross-links with `rust-lld` precisely
because there is no C in the dependency graph — `osc.rs`'s hand-rolled base64 already records
the same rule for the same reason.

**The words ride argv, never a program's source.** `osascript` takes a PROGRAM, and the board's
title is a directory name the user chose, so the script is the constant
`-e 'on run argv' -e 'display notification (item 2 of argv) with title (item 1 of argv)' -e 'end run'`
and the words go past it as arguments — `workspace.rs`'s "argv arrays always". Every field
crosses `text::scrub_text` first, the boundary function for text leaving for another process,
which is also what makes the OSC rung safe: the ESC and BEL it strips are exactly what would
close the sequence early. Verified live, not only in a unit test: a body of
`" & (do shell script "echo pwned") & "` renders as literal text.

**Its own door, and why the row moved.** `menu.rs::draw_list` sizes a dialog at two lines a row
and DOES NOT SCROLL (`dialog::centred` clamps to `screen.height - 2`), and Settings already
outran a 20-row terminal at ten rows — at 80x20 it draws eight. Five more rows there would have
been five nobody can reach, so notifications are a submenu (`Scope::Notifications`,
`Mode::Notifications`, `keymap::NOTIFY_ITEMS`, the same `draw_list`), the move the preferences
themselves made out of the Esc menu. The door was first appended LAST per `SETTINGS_ITEMS`' own
convention and that put it in the clipped region — the feature's own entrance unreachable on a
short terminal — so it sits third instead, with the two other rows about what the board shows
YOU; `the_settings_subtitle_marquees` moved from idx 5 to 6 with it. **The parent list's
clipping is untouched and is now one row worse**: a windowing `draw_list` (the tag row's rule,
where the cell under the cursor is always drawn) is its own ticket.

Five rows, rows 2–5 gated on the first (`MergeTrainNotice`'s shape): the switch, `Also when a
turn finishes`, the two sound rings, and `Banner while the board is focused`. **A sound row
PLAYS what it names as you cycle it** — the theme picker's rule that the cursor is the preview,
delivered as a `Post` with an empty body down the same seam. Two sounds because the user asked
for the two events to be distinguishable without looking: `Sound` is a six-name ring plus off,
macOS's own filenames (`Glass` needs-you, `Tink` finished), collapsing on Linux to the three
freedesktop events that theme actually ships — honest about what is there rather than pretending
six. Five `prefs.json` keys, the two names taking the week-start shape (a name a newer build
wrote survives) and the three bools the unconditional one. `doctor`'s `notifications` line names
the rungs even while it is off, because "would it work if I turned it on" is what somebody reads
it to ask.

Nothing daemon-side moved: no `Command`, no `Snapshot` field, no schema, no e2e. The
`pending_notify` seam is `pending_open`'s — drained in `lib.rs::event_loop`, detached spawn or
one escape to our own stdout, between draws, nothing on screen moved.

## A column sorts by the picker's row (T-283, 2026-09-06)

T-117 gave the column settings dialog a `Sort now` row with four one-shot orders — newest
arrival, oldest, by key, needs-you first — and none of them could see a tag. On a board with a
real vocabulary that is the order you actually want: every bug together, every feature together,
so a column of thirty cards can be read by kind rather than by arrival. The only way to get it
was `HJKL`, one card at a time.

The gap was one variant wide, and the reason is worth recording: `SortBy` is a one-shot ORDER,
not a column setting. Nothing keeps a column sorted afterwards, every gesture keeps working, and
the value never reaches the disk — it lives in `Command::SortColumn` and the TUI's
`Mode::ColumnSettings.sort` and nowhere else. So there was no `ColumnSettings` field to add, no
`COLUMNS_SCHEMA` question to answer, and no daemon change at all: `Daemon::sort_column`,
`agent_allows`' never-tier arm, the `Sort now` row and its `h`/`l` cycle are already generic over
`SortBy`. The whole feature is `SortBy::Tag` plus one arm in `Board::sort_column`.

**The order is the PICKER's row, not the name.** A group's row is its registry entries in the
order the flat `Board.tags` vec holds them (`group_entries`; `group_tags`' doc comment already
called this "stable order, config order, never by recency"), and `MoveTag { to_index }` — the
picker's `HJKL` — is the one thing that arranges it. Sorting alphabetically would have ignored
the only ordering the user can already control; sorting by the row means carrying BUG left in
`^t` raises its cards, which makes the picker the place the sort is configured and needs no
second concept. Registry rank is taken once per sort into a `HashMap<(u8, &str), u8>` by walking
`self.tags` with a per-group counter — that IS `group_entries`' index, without allocating a `Vec`
per comparison.

**Every axis, group 1 deciding.** The key is `[u8; 10]`, one row-index per group, compared
lexicographically — which is exactly "axis 1 decides, axis 2 breaks its ties, and so on". That
needed no parameter on the command and degenerates to "axis 1 only" on a one-axis board, so the
alternative (a group argument on `SortColumn`, or a fixed axis 1) bought nothing. A group outside
`1..=10` is skipped rather than indexed: `TagRef.group` is a `u8` precisely so a value from a
newer daemon cannot break a client, and a sort is not the place to start panicking on one.

**Untagged last, by construction.** An axis a ticket wears nothing on ranks `u8::MAX`, so it
falls below every tag on that axis — the shape `NeedsYouFirst` already has, where the half that
matters rises. A `TagRef` whose registry entry has gone ranks `u8::MAX - 1`: after every real tag
but before the untagged, because it IS tagged — the same reasoning as `tint_of`'s hash fallback,
which exists so a card never renders a colourless band. `sort_by_key` is stable, so ties (two
cards wearing the same tags, or two untagged ones) keep the order the column already had, which
is the promise `sort_column`'s doc comment already made.

**`Tag` is appended to `SortBy::ALL`, never inserted.** The dialog's row opens on `ALL[0]` and
the `column_settings_*` goldens read `Sort now: newest first`; putting the new rung anywhere but
last would have churned two goldens and the TUI test that steps `l` twice to reach `by key`, for
nothing. If those goldens ever do diff on this row, the fix is the order in `ALL`, not
`MESIMON_UPDATE_GOLDEN=1`.

Pinned by `sort_column_by_tag_follows_the_picker_row` (`core/src/board.rs` — registers `FEATURE`
before `BUG` so a pass by name would fail, checks axis 2 breaking a tie, checks the untagged
sinking and ties holding, then moves a tag along the row and asserts the cards followed), the
extended `the_sort_row_steps_on_l_and_runs_on_enter` (the ring reaches `by tag` and wraps), and
the wire + disk round trip at the end of `tags_e2e`.

## Notifications speak from a thread of their own (T-291, 2026-09-07, dogfooding T-282: "notifications aren't available while focused on a tmux session")

T-282 shipped the channel and then could not use it in the one case it was built for. Heads-down
in an agent's pane, board invisible, nine other agents finishing behind it — silence until you
detach. Two independent bugs, both from that block above.

**Bug A: nothing fired at all during a handover.** `handover::run` blocks on `cmd.status()` for
the whole life of the child, so between `restore_terminal` and the return there is no `App::tick`
— no snapshot, no differ, no coalescer beat, no drain of the parked post. T-282's whole
architecture is "the attached board speaks", and during a handover the board is not running. The
hole was never attach-specific: `!` (the project terminal), `^g` (the external editor) and `^Z`
all ride the same road.

**Bug B: the focus rule asked the wrong question.** `notify::Presence` was defined as "the
terminal window has focus". While you are attached to a pane the terminal IS focused, so even
with the loop running the banner would have been suppressed and only the sound would have gone
out. The rule is now **the board is ON SCREEN and focused** — `Presence::looking`, with
`saw_board(bool)` set around every handover, and `focused` kept as the half it is built from
(`doctor` still reports whether the terminal ever answered). `a_board_off_screen_is_not_being_looked_at_however_focused_the_terminal_is`
is the law, and it names both failure shapes: a stale keypress into somebody else's pane is not
presence either.

**The fix is a thread, not the daemon.** `tui/src/notifier.rs` owns a second daemon connection
alive for the life of the board process: it subscribes, runs the differ and the coalescer, and
posts, and none of that depends on where the main loop is. The daemon road was considered and
refused for the reason T-282's own block gives — `Change.attention_added` is still sitting in
`attention.rs` with no consumer, and taking it would end "a closed board is silent", which is
D15's constraint and the sentence the Settings row says. The thread keeps the process dying as
the off switch, needs no wire command, no `Snapshot` field, no schema and no daemon state, and
does not spend the wire protocol on what is a view concern. Notifying with no board open at all
is a different feature, and a surprising one.

**What moved where.** The differ left `App` for `core/src/notify.rs::Differ` — it was always
pure, and its eight tests moved with it and read better against a `Board` fixture than against an
`App`. `App` keeps four forwarders and one field: `notifier`, set by `lib.rs` and never
`App::new`, the rule the two ladders and `opener` already follow — so no test app and no golden
raises a banner, makes a sound, or opens a second connection. The main loop pushes three things
the thread cannot know: presence (focus events, keypresses, `saw_board` around every handover),
the five preference fields (on every `App::set_pref`, so a row the user just took cannot be acted
on once more), and the terminal. `App::started` and `now_ms` went with the differ; the clock is
now `Shared`'s, because the main loop stamps a keypress with it and the thread reads that stamp
back thirty seconds later, and two `Instant`s of their own would disagree by however long the
board took to start.

**A second writer on stdout is the new hazard, and `notify::Console` is the answer.** Every rung
but two spawns a program with null stdio and is indifferent to what the terminal is doing; OSC 9
and the bell are writes into the same stream ratatui draws on. One lock over one fact — does the
board hold the terminal? — taken by the draw to draw, by a rung to write, and by a handover to
change hands. So an escape can neither land inside a frame nor reach a terminal that now belongs
to tmux. While the board is off screen those two rungs say **nothing**, and that is the honest
limit rather than a bug to chase: a banner held for the twenty minutes somebody stays in a pane
is worse than one never raised. The ladders already prefer a helper program, so a machine with
`terminal-notifier` or `notify-send` is fully fixed and a pure-OSC one is not; `doctor` says so
on the OSC rung's own line rather than leaving it to be discovered.

**Two things the observer connection must not do**, both learned from reading the roads it was
about to take. It never restarts the daemon on a build skew — `build_skew`'s ordering rule exists
precisely to stop two clients taking turns, and a second connection with no board behind it has
no business voting. And it never brings a daemon UP: `Client::connect_observer` reopens through
`open_existing` (a bare `UnixStream::connect`, no spawn, no lock wait, no budget), because `U`
asks the daemon to stop and then waits for it to be GONE — a thread respawning one behind
`reexec`'s wait would have turned every reload into thirty seconds of "still shutting down"
followed by a connect to a daemon nobody asked for. `run` also drops the notifier and takes the
escape rungs away before `restore_terminal`, so the reload's own words have the terminal to
themselves. `an_observer_subscribes_and_never_spawns_a_daemon` (in `restart_skew_e2e`, the one
e2e that drives the real client) pins both halves and the subscription itself, which is the only
new thing on the wire: two connections on one daemon, both told.

**Cost.** With notifications ON a board change is now two `Snapshot` round trips instead of one.
That is the price of the design and it is bounded by the switch: off — the default — the thread
holds no connection at all, dials nothing and asks nothing, and a beat on a quiet daemon is one
`try_recv` on the event channel.

## A ticket is quiet inside its own pane (T-292, 2026-09-07, user: "not show the notification (+ sound) if inside the tmux agent session of the ticket owning the notification")

T-291 made the board speak through a handover, which immediately made the obvious next thing
audible: attached to T-5's claude, talking to it, every turn it ends rings a bell and raises a
banner for the pane already filling the screen. The fix is the focus rule one level finer.

**The rule and why it is STRONGER than the focus rule.** Looking at the board takes the banner
and leaves the sound: a card is small, the chime says go look, and the two are different jobs.
Inside the agent's own pane there is no second job — the permission prompt IS the pane and the
finished turn IS the last thing printed in it — so `Presence::watching` takes both, and it is the
only suppression in the feature that does. Only that ticket goes quiet; the other nineteen agents
are exactly as invisible from inside a pane as they ever were, and T-291 exists to let them
through.

**Which handovers count.** An attach to that ticket's CLAUDE, and nothing else
(`App::watched_ticket`, read off `focused_session_hint` at the moment `lib.rs` gives the terminal
away). A SHELL session on the same ticket, the `!` terminal in its worktree and a `^g` editor all
show the user's own words rather than the agent's turn, so an agent's news there is news — the
test is whether the thing that happened is on the screen in front of the user, not whether the
user is thinking about that ticket. The GATE ceremony parks its real target in `pending_gate_then`
and leaves `focused_session_hint` None, so the first attach watches nothing and the second one
watches the pane; that falls out of the existing shape rather than needing a case. The answer is
a ticket id rather than a session id because a ticket holds one claude (2026-09-02) and `Event` is
ticket-keyed — a session field on `Event` would buy nothing this does not already say.

**`Coalescer::forget(ticket)`, every beat, not on the edge.** The window is five seconds long and
`c` on a card that just lit up lands inside it, so filtering only what the differ hands over would
still announce a line queued a moment before the attach. `forget` is called for the watched ticket
on every pass: it catches what was already held and what was offered this same beat, it is
idempotent, and it costs a retain over a Vec that is almost always empty. The DIFFER is untouched
— the mark stays — so detaching does not then announce what you sat and watched happen.

**Opt-out (the user's second ask).** A sixth Notifications row rather than a widening of
`notify_focused`: the two rules differ in strength (one keeps the sound, one does not), and
folding them would have made one label describe two behaviours. `prefs.json::notify_in_pane`,
default false = silent. It matters for one real workflow the suppression would otherwise break:
attached to a pane, terminal in the background, away from the desk, waiting for the ding — inside
a handover there is no way to tell that from "actively typing", because keystrokes go to tmux and
focus reporting is off for the duration. The row is the way to say which one you are. `doctor`
names it on the notifications line, and the dialog is 14 rows, which still clears `MIN_H`.

## What waits for the checkout may be the SESSION (T-294, 2026-09-06, user: "shift+enter on non started sessions should ask if now / queued when there is a running session")

**What was wrong.** The queued ask (2026-09-04) let a prompt bound for a live PANE wait for a
quiet checkout, and the two roads it did not cover were the two that add a WRITER to that
checkout rather than asking the one already in it. An empty claude seat spawned at once —
`dispatch` fell through to `start_composed`, no field, no choice — and a `Sleeping` claude was
woken at once, because `Ctx::ask_queueable` required a pane and so did `enqueue_ask` ("a queued
ask needs an awake claude — wake it first"). So on a busy shared checkout the presses that most
deserved to wait were the only ones that could not. The block above listed the second of them
under **Not done**.

**Three seats, one delivery.** `Daemon::seat_of` answers `QueuedSeat::{Pane(id), Wake(id),
Start}` — `has_pane` first, then `live_claude` (live and paneless is exactly `Sleeping`), then
nothing — and `Daemon::deliver` is the one road every ask takes, a send-now `PromptSession` and
`drain_queue` alike: paste, or `prompt_sleeping`, or `spawn_session(.., submit_prompt: true,
prompt)`. That is the point of extracting it: a queued ask and a sent one cannot disagree about
what "the ticket's claude" means, because they ask the same function. A `Start` and a `Wake` need
no `inflight` marker to hold the checkout — `Spawning` and an owed Enter are both WORKING already
— so the card shows the launching arc instead of `queued ∙ sending`.

**The words ride under the brief.** `spawn_session` takes a `prompt: Option<String>` and parks
`Parked { text, brief: true }`; `retry_pending_submits` composes description **then** user words,
in the order they were written — the ticket says what the work is, the user says what to do about
it first — and still stamps `ticket_read`. `pending_spawns` became a named `PendingSpawn` so a
worktree provisioning replay carries the words too. A `Start`'s `text` may be EMPTY, and that is
the one place `sanitize_prompt`'s blank refusal is lifted: its Enter lands on the ticket title the
spawn types, which is a turn the user did write, so "an empty paste would press Enter on a turn
the user never wrote" does not describe it.

**A seat that changed drops the entry.** `seat_stands` is the rule and `sweep_queue` and
`drain_queue` both ask it: a `Pane` must be the same pane, a `Wake` the same record (woken by hand
in the meantime still delivers — same session, same conversation), a `Start` needs the seat still
EMPTY. Unbranched, the old `pane_target(ticket) != q.session` test would have dropped every queued
start on the next tick. Nothing is ever redirected; that was already the rule and it now has three
arms. `Pending.action` carries the seat's word (`ask` | `wake` | `start` — `Pending::is_queued_ask`
holds the vocabulary so no screen spells it), which is what lets the card say `starts ∙ after T-3`
instead of that words are waiting: a session is about to exist there, which is louder than a paste.
No subject on that row — the card is the subject, and `claude starts ∙ after T-3` is 25 cells where
the row has 22; the status line has a whole row and names it.

**The field opens only where waiting means something.** `Ctx::checkout_busy` — `App::checkout_busy`,
the TUI's own read of `quiet::is_working` over shared-checkout sessions, plus the snapshot's
`in_flight` rows standing in for the daemon's private `inflight` — is a HINT and the module doc now
says so: it decides whether the press stops to ask, never how the words are delivered, and where it
disagrees with `checkout_holders` the cost is a field that opened where a spawn would have gone. A
quiet checkout keeps the one-key start (`ask claude the title`); a busy one opens the field at
`queued` (`start claude`). *(T-379 later opened the field on a quiet checkout too, at `now`.)* **A live pane keeps `now` either way**, deliberately: it has shipped that
way, and a person reaching for a working agent may well mean interrupt — only the two roads that
would start or wake a session take the new default. `ask_queueable` lost its pane clause, so the
`shift+tab now / queued` row is offered on all three seats.

**The one new gesture rule.** A blank Enter over a waiting entry drops it (T-241) — except on an
empty seat with the toggle moved to `now`, which is how a queued start jumps its own queue, since
its field is empty by nature and there are no words to retype. `commit_input` and the card's
placeholder judge it with the same expression, so `start on the title` and `enter drops` can never
say different things than the key does.

**Also.** `Command`, `Response`, `agent_allows`, `authorize` and every `*_SCHEMA` are untouched:
`Pending.action` was already an open word, and `PromptSession` already carried everything needed.
E2e `ask_queue_e2e::a_queued_start_waits_for_the_checkout_and_then_spawns_a_claude`, and
`prompt_e2e`'s two "no live claude" refusals are now starts — a killed record leaves an EMPTY seat,
and an empty seat is one this command fills. Goldens `board_prompt_start_120x30`,
`board_queued_start_120x30`.

**Not done.** Persistence across a restart (a queued start dies with the daemon like every other
entry); a queued start on a WORKTREE ticket, which `enqueue_ask` still refuses with "a worktree
ticket's checkout is its own" — right today, since that checkout has no one else in it, and wrong
the day worktrees share a machine's resources rather than a tree; the PTY budget, which is
deliberately not consulted at enqueue, so a start that cannot spawn says so at delivery in the feed
rather than at the press.

## A notification says the ticket's title and what the agent said (T-292, 2026-09-07, dogfooding T-282: "OS notification doesn't show ticket title. and no transcript")

T-282's banner said `T-12 needs you ∙ PERMISSION` and nothing else. The key is a pointer, not an
answer: to learn WHICH ticket that is and WHAT happened you had to go to the board — which is most
of the work the banner existed to save. `terminal-notifier` and `osascript`'s `display
notification` each have three fields and the channel was using two of them, so the fix was to say
the third thing rather than to build anything.

**The mapping, and why the verb stayed.** `title` is WHO AND WHICH BOARD — `mesimon - simbly`, the
product name and the checkout's directory. The second half was already there in T-282 (two boards
open at once); the first was added the same day this shipped, because the banner is posted under
the HELPER's identity — `terminal-notifier`'s own, or Script Editor's for `osascript` — so the
title row is the only place mesimon can say it is mesimon. A hyphen and not `∙`, since the
one-field rungs fold the whole post with `∙` and the source must not read as a peer of its news.
`subtitle` is
WHICH TICKET — `T-12 ∙ Add auth to the API`, the key first because the key is how a ticket is named
in a prompt or a commit and the title second because it is what a person recognises; `body` is WHAT
HAPPENED. The one thing the brief's own table got wrong is that the body could then be pure content
— just `PERMISSION`, just the reply. It cannot: the two moments are `needs you` and `finished`, the
subtitle no longer carries either, and the only other thing that tells them apart is the chime. So
the verb leads and content is APPENDED to it (`needs you ∙ PERMISSION`, `finished ∙ Tests pass`),
which has the second virtue that the sentence with nothing to append is the sentence that already
shipped: `needs you`, `finished a turn`. A withheld word is a shorter line, never a different one.

**Only a batch of ONE.** Three titles do not fit a banner, so several keep the count shape T-282
gave them (`20 agents finished ∙ T-1 T-2 T-3 T-4 +16`) with no subtitle at all. `core::notify::body`
is now the aggregate's alone and `names`/`said` are the single's; a mixed batch of one needs-you
beside one done is still two events and still names both keys inline.

**The words are resolved at POST time, and that is the whole design of the seam.** A turn's closing
record lands on the transcript around the moment the state flips, so reading the reply on the
rising edge races the writer that is producing it. The coalescer already holds a batch for up to
`WINDOW_MS` (5 s) before it says anything, so `Coalescer::due` takes a lookup — `&dyn Fn(Ulid) ->
Option<Detail>`, answering the ticket's title and its agent's last line — asked for ONE ticket and
only past the window check. A busy board therefore reads no transcripts at all, a held batch reads
none until it is finally said, and the cost at the top is one `stat` plus one ≤64 KiB tail read per
five seconds, on a thread that has nothing else to do. The wording stays in the pure module and the
freshness in the TUI; the core tests pass a stub map and a `Cell` counter proves the aggregate asks
nothing.

**The reply is taken only when `Peek::reply_key` is set.** `peek::latest_preview` falls back to the
user's OWN words prefixed `>` where the window holds no assistant record — right for a card, wrong
here: a banner that quotes your own prompt back at you says nothing, and says it as though the
agent had. `Worker` keeps the board it last scanned instead of dropping it (the lookup runs a beat
or twenty after the edge), and `detail_for` is a FREE function rather than a method so the closure
borrows `Worker::board` alone — `batch` is borrowed mutably in the same expression, and Rust only
splits disjoint field borrows when the closure names the field.

**A preference, because a banner lands on a lock screen.** `The agent's words: quoted | withheld`
(`prefs.json::notify_words`, default quoted) is the seventh notifications row, sitting THIRD —
whether at all, which moments, **what it says**, what it sounds like, then the two exceptions.
Withheld takes the agent's last line and a raised hand's own sentence; it does not take the ticket,
which is the half this ticket exists to add, and it does not take mesimon's own reason word, which
is from a fixed set and describes no work. Telling those two apart is why `Event` grew
`quoted: bool` — `why` is `attention::reason_word` on one road and `Raised::reason` on another, and
as strings they are indistinguishable. Off is also cheaper: `detail_for` does not open the
transcript at all. Seven rows is fourteen lines and `draw_list` fits eight rows at `layout::MIN_H`,
so the list still clears without the windowing that is still its own ticket.

**Rungs with one field fold, and none of them changed arity.** `Post::folded` is in the pure module
so the separator is decided once; `notify-send`, a user's own `MESIMON_NOTIFY` program and OSC 9
all get `subtitle ∙ body` as the body they already took, which is what keeps somebody's own
two-argument script working. `terminal-notifier` gains `-subtitle` and `osascript` a three-item
script — and only when the subtitle is non-empty, so an aggregate posts the argv that shipped
before, byte for byte, both scripts included. The words still ride argv and never the script:
`item 1` is the title, `item 2` the subtitle, `item 3` the body. `MAX_FIELD` (240 bytes) did not
need revisiting after all — it is the BACKSTOP, the pure module now clips a title at 72 characters
and a reply at 120 on a word boundary with an ellipsis because those two are read by a person, and
the folded line is two already-capped fields with a separator between them.

**`-group`, the freebie.** `terminal-notifier -group <id>` replaces the previous notification with
the same id, so one group per BOARD stops the board stacking a column of banners in Notification
Centre — the coalescing rule extended into the OS for one argument. Per board and not per ticket:
two boards open at once are two conversations, and the id is `mesimon-<proj16>`, the same
canonical-path hash the sockets and the state dir are keyed by, so two checkouts of one project are
two groups. **This is the argument T-293 will have to revisit**: with a per-board group only the
newest banner survives to be clicked, so "clicking a notification opens the board on that ticket"
either accepts that or narrows the group, and it should decide rather than inherit.

**A note on the number.** The block above this one is titled T-292 and is T-291's second half; it
was written before this ticket existed and its citations are left as they stand rather than
rewritten under another session's uncommitted work. `notify_in_pane` is T-291; `notify_words`, the
subtitle, `Detail`, `Voice` and `-group` are this one.

Deliberately out: no daemon change of any kind (no `Command`, no `Snapshot` field, no schema, no
e2e), which is T-282's posture; and no ellipsis on `MAX_FIELD`'s own cut, which stays a hard byte
prefix because by the time it fires the pure module's clip has already failed to hold and the
honest thing is to stop. Goldens `notifications_120x30`; tests in `core::notify`
(`one_event_names_its_ticket_and_quotes_its_agent`, `withholding_the_words_keeps_the_ticket`,
`an_aggregate_asks_nothing_of_the_lookup`), `tui::notify`
(`a_subtitle_rides_its_own_field_or_folds_into_the_body`, `no_subtitle_is_the_argv_that_always_was`,
`a_board_groups_its_own_banners`) and `tui::notifier`
(`the_banner_names_the_ticket_and_the_row_can_withhold_the_words`).

## A banner you can click raises the terminal (T-293, half one, 2026-09-07, dogfooding T-282: "clicking on OS notification should lead to the board with the item focused")

T-282 gave the board a voice, T-292 made the banner name its ticket — and then the gesture
everyone tries first did nothing. `terminal-notifier` was invoked with no action, so a click
activated terminal-notifier itself, an app with no window; `osascript`'s `display notification`
carries no action at all. The one accident that worked was OSC 9, where iTerm2 focuses its own
window and tab for free.

**The ticket has two halves and this is the first.** Raise the terminal (cheap, and most of the
value: you are elsewhere, the banner says which ticket, the click puts you back in front of the
board) and put the CURSOR on that ticket (expensive — there is no channel into a running TUI, so
it needs a `mesimon show <KEY>` subcommand parking a request file under the runtime dir for the
board to drain on its next tick). Only the first is built. The decisions taken for the second,
so they are not made twice: the verb is `show`, because `focus` already means "hand the terminal
to a pane" throughout the daemon; the click returns to the board from any screen but never over
a text field, since a half-typed composer must not be eaten by a stray click; an aggregate keeps
the raise and carries no target; and the parked request needs a timestamp, because `App::tick`
does not run during a handover and a click acted on twenty minutes later is a yank.

**One rung can carry a click, and it is already the preferred one.** The ticket flagged the
ladder's ordering as a decision to make deliberately — whether to prefer a clickable rung when
the preference asks for it. It needs no change: `find_banner` already puts `terminal-notifier`
above `osascript` on every platform, so a machine with the brew install gets the click for free.
The other rungs cannot follow, and that is a property rather than an omission: `osascript` has no
action parameter, and `notify-send --action` requires the process to stay alive and read the
chosen action off its stdout, which `opener::launch` — detached, null stdio, reaped on a thread —
deliberately is not. So `doctor` says which rung answered and whether it can deliver a click,
rather than leaving it to be found out by clicking.

**`-sender` is the rival and is refused.** It would give the banner the terminal's own icon
instead of terminal-notifier's, which is nicer — but its own README says it cannot be combined
with `-activate` or `-execute`, because those need the sender of the notification to BE
terminal-notifier. Read, not guessed. A banner that looks right and does nothing is the thing
this ticket exists to end, so the click wins and the icon is what it costs.

**Which terminal is a ladder of its own, and the interesting rung is a veto.**
`MESIMON_TERM_BUNDLE` (`off`, or a bundle id) → an outer tmux VETOES the question →
`__CFBundleIdentifier` → a `TERM_PROGRAM` table. Rung three is the one worth arguing for: macOS
LaunchServices stamps `__CFBundleIdentifier` on the app it launches and every child inherits it,
so it *is* the type `-activate` wants — a bundle id, not a name needing translation — and it
answers for kitty, Alacritty, Warp, Hyper and whatever ships next year, where a table answers for
however many rows somebody wrote. It is a private Apple variable, and the answer to that
objection is that its failure mode is graceful: absent, the table catches it. Measured on the
author's own board, `__CFBundleIdentifier=com.googlecode.iterm2`.

The veto sits ABOVE it because that is the one case where inheritance makes it wrong. In the
user's own tmux the variable is a plain inherited value naming whatever started the SERVER, not
the client attached now — start tmux from Terminal.app, attach from iTerm2, and it says
Terminal.app. And `-activate` goes through `NSWorkspace`, so it does not merely focus a running
app: it LAUNCHES one that is not. A stale id therefore opens a fresh window of a terminal nobody
asked for, which is strictly worse than doing nothing, because a notification is a promise.
`TERM_PROGRAM` is rewritten to `tmux` in every pane, which makes it the one thing visible from in
there that cannot be stale — a reliable negative. It could not be confirmed by measurement on
this machine (both live tmux servers are mesimon's own, and `shellenv.rs`'s `env_clear` means a
mesimon pane could not carry the variable whatever tmux did), so it ships as insurance that costs
nothing.

**`LC_TERMINAL` was built into the design and cut before it shipped.** iTerm2 sets it precisely so
it survives ssh and tmux, which reads like the rung that fixes the tmux case. But ask when it can
actually fire: only where `__CFBundleIdentifier` and `TERM_PROGRAM` are both absent, and on macOS
that is essentially ssh — where the stock `SendEnv LC_*` forwards it and the other two do not. In
that one live case the rung is *wrong*: it would raise iTerm2 on the remote Mac, which nobody is
looking at. A rung whose only reachable case is a wrong answer should not exist.

**`bundle_id` REJECTS where `field` scrubs, and the inversion is deliberate.** `scrub_text` drops
characters, which is right for a sentence and wrong for an identifier: a bundle id with a
character dropped is a different, possibly real bundle id, so a silent repair raises the wrong
application. A leading `-` is refused for a second reason — terminal-notifier parses
NSUserDefaults-style `-key value`, so `-activate -sound` is a flag with no value. Every rung,
`__CFBundleIdentifier` included, exits through it.

**Two things do not get the id.** `Banner::Custom` keeps its three-argument contract
(`<program> <title> <body>`): a fourth argv word would silently change what `$3` means to a
program somebody wrote against T-282, and `opener::launch` clears no environment, so a custom
notifier that wants the id can read `MESIMON_TERM_BUNDLE` or `__CFBundleIdentifier` itself. And
no rung may grow a flag in `Osascript`'s argv, ever: its words are found by POSITION (`item 1`,
`item 2`, `item 3`), so one inserted argument shifts the title, the subtitle and the body
together. `only_the_terminal_notifier_rung_grows_a_flag_for_the_click` asserts the argv is
byte-identical with and without an id for every rung but one, which is what catches that
regression before it is shipped rather than after.

`-activate` is appended BEFORE `-group` so `-group` stays last and
`a_board_groups_its_own_banners`' tail assertion keeps meaning what it meant. Nothing else moved:
no `Command`, no `Snapshot` field, no schema, no key, no preference, no dialog row, no golden, no
daemon code — and no README change, since nothing new is written to disk. Pinned by five tests in
`tui::notify` (`the_terminal_that_gets_raised_is_named_by_the_env_first_and_the_os_second`,
`an_outer_tmux_names_no_terminal_because_nothing_it_can_see_is_fresh`,
`a_bundle_id_that_is_not_one_is_refused_rather_than_scrubbed`,
`only_the_terminal_notifier_rung_grows_a_flag_for_the_click`,
`the_click_is_promised_only_on_the_rung_that_can_deliver_one`).

## Somebody has to still be in the pane (T-299, 2026-09-07, dogfooding T-292: "notification not shown when terminal is not focused and source again tmux session is attached")

T-292 shipped in the morning and was filed against by the afternoon, which is the right length of
feedback loop. Attached to T-5's claude, the user switched to a browser; T-5's agent hit a
permission prompt; the board said nothing at all — no banner, no chime, because the in-pane rule
is the one suppression that takes both. The pane was on the terminal and nobody was in front of
it.

**T-292's own block predicted this and got the conclusion wrong.** It says: "inside a handover
there is no way to tell that from 'actively typing', because keystrokes go to tmux and focus
reporting is off for the duration. The row is the way to say which one you are." The premise is
exactly right and the conclusion does not follow — *tmux* can tell, because tmux is the program
reading that terminal. A Settings row asks the user to predict, before they attach, whether they
are about to walk away.

**Half one: a focus report is evidence only while it can be REFUTED.** `restore_terminal` sends
`\e[?1004l` before every handover, so nothing is reported for its whole duration. `Presence.focus`
therefore held `Some(true)` — stamped the instant before the attach — for as long as the user
stayed in the pane, and `watching` believed it. `Presence::focused` now consults `self.focus` only
while `on_screen`; off screen, presence is keystrokes and nothing else. This changes no banner:
`looking()` already ANDs with `on_screen`, so the only reader that can see the difference is
`watching`. Leaving the report enabled through a handover was considered and refused — the
terminal's `\e[I`/`\e[O` would be read by the attached tmux CLIENT and typed into the agent as
literal escape bytes.

**Half two: the keystrokes are tmux's, and they are asked for.** `#{client_activity}` is the last
time tmux read input from a client, which is the missing half measured directly.
`Command::FocusQuiet` → `Daemon::focus_quiet` → `TmuxBackend::client_quiet_secs`, one
`list-clients -t <sid16> -F '#{client_activity}'`, freshest client wins, SECONDS because that is
tmux's resolution on this format (verified on 3.6a: it holds still across three idle seconds and
moves on a keypress from a pty client; a control-mode client is listed and stamped at attach but
its stdin carries commands, not keys, so it never moves — which is why the e2e asserts liveness
and not movement).

**The command takes no argument**, and that is the whole of its authorization story. The subject
is whatever the daemon holds the focus token on, so a stale ticket id from a client cannot make
the daemon answer for a pane nobody is in. It is a `Read` in `Command::meta`, it names
`Resource::Session` for the focused session so `authorize`'s existing denial reaches it, and
`mcp::agent_allows` refuses it — a session read at any tier, plus a fact about the PERSON, which
no agent has ever been able to ask for.

**`None` is every way of not knowing, and it reads as away.** Nothing focused, no such session,
nobody attached, tmux unable to answer — one answer for all of them, and `saw_pane_quiet(now,
None)` CLEARS the memory rather than keeping the last one. That is the direction the whole feature
falls in (`Presence`'s own doc: "silence is the failure that would make the feature look broken").
An answer PAST `KEY_PRESENCE_MS` is likewise stored as absence rather than as an old moment:
`now.saturating_sub(quiet_ms)` on a young monotonic clock saturates at zero, and zero is a
keypress at start-up, so an hour of silence would have read as typing-now for the board's first
thirty seconds.

**The fork is rare by construction.** `ask_who_is_typing` runs only when `Presence::attached()` is
Some AND `Coalescer::holds(ticket)` — an attach exists, and something is held about that very
ticket. A board nobody is attached to never forks; an attached board with nothing to say never
forks. It rides the daemon's writer thread for `pane_tail`'s reason: one small tmux fork, where
the off-thread treatment exists for git.

**The trade, stated.** Presence is keystrokes, so reading a long agent turn for thirty seconds
without touching the keyboard reads as away and the ticket chimes. Chosen over the alternative
(silence for the whole attach, which is the bug) with the user in the loop; `notify_in_pane`'s
Settings row is unchanged and still forces the loud direction. `#{client_activity}` was picked
over asking macOS which app is frontmost: that is a fork per beat, mac-only, and would have made
the rule unavailable on the platform `ci/test-linux.sh` covers.

Tests: `core::notify` (`tmux_says_whether_anybody_is_still_in_the_watched_pane`,
`the_pane_answer_does_not_outlive_the_attach`,
`a_long_silence_on_a_young_clock_is_absence_not_a_keypress`, and the T-291 test now asserting that
an unrefutable report is not evidence); `tui::notifier`
(`a_watched_pane_behind_another_window_speaks_after_all`,
`the_same_pane_with_somebody_in_it_stays_quiet`); e2e `focus_quiet_e2e`.
## A click lands in the board's own tab (T-301, 2026-09-07, dogfooding T-293: "OS notification click not leading to the correct mesimon tab in terminal")

T-293 raised the terminal and called that "puts you back in front of the board". It is not, on
any machine where the terminal has more than one tab — and the author's has two boards open in
one iTerm2. `-activate` names an APPLICATION, so the click brought the terminal forward showing
whatever tab happened to be in front of it, which is a coin toss dressed up as a feature. The
half T-293 named as owed is the CURSOR (a `mesimon show <KEY>` subcommand parking a request file
for the board to drain); this is a third thing neither half saw, and it is the one that makes the
first half true.

**The tab is a second flag on the same rung.** `-execute` is a `/bin/sh -c` line terminal-notifier
runs when the banner is clicked, and its source runs BOTH actions in order — `if (bundleID)
activateAppWithBundleID; if (command) executeShellCommand` — which is why the two are sent
together rather than one instead of the other: the application comes forward, then the script
picks this board's tab out of it, and if the script is refused permission to run, what is left is
exactly the click T-293 shipped. `Channels.activate: Option<String>` became `Channels.click:
Option<Click>` (`Click { app, reveal }`), which is also what keeps `Banner::argv` at four
parameters.

**Only a terminal knows where its tabs are, and it answers in AppleScript.** `Reveal` is the
second ladder, shaped like every other one in the file: `MESIMON_TERM_REVEAL` (`off`, or a
program of the user's own — a terminal with a remote control, `kitty @ focus-window` or `wezterm
cli activate-pane`, knows which window it means far better than a table here could; a PROGRAM,
as `MESIMON_NOTIFY` and `MESIMON_OPEN` both mean one, since the word is quoted whole and a
command line quoted whole is one program name with spaces in it) → iTerm2 by
the session uuid its dictionary calls `id of session`, which is the tail of `ITERM_SESSION_ID` →
Apple Terminal by a tab's `tty`, which is the one on our own stdin (`own_tty`, `libc::ttyname(0)`,
asked once from `find` on the main thread). Two rungs and no more, because every entry has to be
VERIFIED against the running application before it is added, and those are the two macOS
terminals with a scripting dictionary that can say where a tab is.

**The tmux veto needs no rung of its own here.** An outer tmux rewrites `TERM_PROGRAM` in every
pane, so the two-name table simply never answers — which is the right answer for the veto's own
reason: in there `ITERM_SESSION_ID` is INHERITED from whatever started the server and names a
session that is not this one, and selecting the wrong tab is the bug, not the fix.

**Three rules hold it together, and each is a test.**

- *A script talks to the application the click raises.* `Reveal::app()` names its own bundle id
  and `find_click` DROPS a reveal that is not the one `-activate` was given, so a
  `MESIMON_TERM_BUNDLE` naming some other application cannot leave an iTerm2 script attached to a
  click that raises something else. The user's own program is exempt: it names no application, so
  there is nothing to disagree with.
- *It raises a tab and never an application.* Both scripts are wrapped in `if application id … is
  running`, because `tell application` STARTS what is not running — a click on a banner that
  outlived its terminal would otherwise LAUNCH it and open an empty window, which is the tmux
  veto's failure by another road. Asking whether an application is running starts nothing, and
  that is also what makes the `activate` INSIDE the guard safe. That `activate` is not a duplicate
  of `-activate`'s: Apple Terminal reorders its windows only while it is the ACTIVE application
  (`set frontmost of w to true` in a background one returns success and does nothing — measured
  both ways, as is `set index of w to 1`), so without it the reveal would depend on another
  process's activation having already landed.
- *The command carries no word from a payload.* It is a constant script plus one id validated by
  `session_uuid` / `tty_path`, which REJECT where `field` scrubs, for `bundle_id`'s reason: a
  repaired session id names a DIFFERENT tab, and landing in one is the bug being fixed.
  `sh_line` single-quotes every word and refuses one holding a `'` rather than escaping it —
  inside single quotes `sh` reads every other byte literally, newlines included, which is what
  lets a whole AppleScript ride one word. It is the only place mesimon builds a shell command,
  and it exists because `-execute` takes a command where every other rung takes argv.

**The two dictionaries are not the same shape.** iTerm2 needs `select` on the window, the tab AND
the session, because a session may be one pane of a split and none of the three is implied by
another. Apple Terminal has no session and no `select` at all: a tab is `selected` and a window is
`frontmost`, both properties. An id nothing matches is a silent no-op in either — rc 0, nothing
moves — which is the right answer for a banner clicked after its tab was closed.

**What `doctor` says is which half it has.** `click raises com.googlecode.iterm2 and this board's
own tab`, or `click raises com.github.wez.wezterm, not this tab ∙ MESIMON_TERM_REVEAL names a
program that can` — the second is the honest sentence for every terminal neither rung fits, and it
teaches the escape hatch in the same breath. Unchanged: the rung that cannot carry a click at all
still says so.

**What this does not fix.** The cursor still does not move to the ticket — T-293's second half,
whose decisions are recorded there and are unchanged by this. And the first click of a machine's
life may raise a macOS Automation prompt (terminal-notifier asking to control the terminal),
because the responsible process for the script is terminal-notifier rather than mesimon; denied,
what is left is T-293's click, which is the same degradation as a terminal with no rung.

Nothing else moved: no `Command`, no `Snapshot` field, no schema, no key, no preference, no dialog
row, no golden, no daemon code, and no README change — nothing new is written to disk. Pinned by
six tests in `tui::notify` (`the_tab_is_named_by_the_env_first_and_the_terminal_second`,
`an_outer_tmux_names_no_tab_either`, `a_reveal_scripts_the_application_the_click_raises`,
`the_click_runs_a_constant_script_and_never_a_composed_one`,
`a_session_id_or_a_tty_that_is_not_one_is_refused_rather_than_scrubbed`,
`the_tab_is_a_second_flag_on_the_one_rung_that_can_carry_a_click`), and by the two scripts having
been run through `/bin/sh -c` exactly as terminal-notifier runs them, against both live
applications, before either was written down.

## A folded column reads its count at the top (T-302, 2026-09-07, user: "collapsed columns ticket count should be somewhere else ∙ bottom too far")

07 §3.1 put the collapsed column's count at the FOOT of its spine, bottom-aligned and never
dropped, and that is what shipped: the name ran down from row 2 and the number sat on the last
body row, twenty-odd rows below every other count on the board. Nothing else on the board is read
there. The user's own repair is the one taken — "when no ticket needs you it should render the
number on the uncollapsed columns line" — and generalised by one step, because the spine's row 0
was ALREADY the header row: it is where the `!` goes, and it is the row every expanded column
writes its own count on.

So the spine's top block is now what an expanded column's header row carries, in the same order:
the `!` (T-271's painted cell) iff the column holds a waiting ticket, then the count, a digit a
row, then one blank, then the name. With nothing waiting the count's first digit lands ON row 0,
level with `TODO … 2  IN PROGRESS … 2  REVIEW … 2   1`, and the four counts of a folded board read
as one row. A `!` claims that cell and the count takes the row under it — the mark is what the
folded column is standing in for, and it may not move for a number.

**The truncation reversed, and that is the improvement.** The old arithmetic reserved the digits
and the gap out of the body and let the NAME truncate into what was left; now the top block is
written first and `name_rows` is whatever the column has left, so a spine too short for both loses
letters of its name rather than its count. "Never dropped" survives literally, and stops costing a
subtraction three lines up from where it is spent (`draw_spine` lost its `used`/`body` bookkeeping
and its two `Vec<char>` collections with it).

**The stagger is deliberate.** A waiting spine's name starts one row lower than a calm one's, and
a column of ten or more starts one lower again. The alternative is a fixed name origin, which
means either reserving three rows for a block that is usually one, or clipping the count of the
one column that most needs to be counted. The name runs down from under the numbers; that is the
whole rule, and it is legible on a 1-cell column precisely because there is nothing else there.

Goldens `board_spine_100x24` and `board_pinned_120x30` moved (the count from the last body row to
the header row, nothing else). `the_folded_column_reads_its_count_at_the_top` pins both cases —
the digit on row 0 with a clean foot for a calm column, the `!` on row 0 with the count beneath it
for a waiting one, and the name under whatever the top block came to. T-271's
`the_folded_column_paints_its_needs_you_mark` is untouched and still passes: the `!` never left
row 0. Nothing else moved — no `Command`, no snapshot field, no schema, no key, no preference.

## The board's top row is a place the cursor can stand (T-305, 2026-09-07)

T-117 made a column HEADER a cursor position: `k` off the top card lands there, four verbs take
the column as their subject, and `j` comes back. Above it the board's own header row was a
read-only strip — facts, an offer chip, and since T-221 one key spelled out beside the fact it
opens (` v diff` after `∙ 3 changed`). The user asked for the obvious next step: `k` off the
column header focuses that row too.

**It is a SCOPE, not a screen.** `App::header_focus` plus `Screen::Board` answers `Scope::Header`
in `App::scope()`; nothing is drawn over the board, no mode is set, the columns are still there.
That is the whole reason it can be one keypress deep — the alternative shapes (a `Mode`, a
`Screen`) both imply something covering what you were looking at, and this covers nothing.

**One cursor on screen, and the column keeps its band.** `App::on_column_header` returns FALSE
while the row holds the cursor, so every consumer of that one predicate — `Ctx::col_header` and
with it Enter/`r`/`HJKL`/`d`'s column subjects, `board_enter`, `can_nudge`, and the header row's
own cursor BAR — stands down in a single edit. What the cursor column does NOT give up is its
painted band: that is what says where `j` returns to, and dropping both would have left the
board with no memory of where you came from. `App::at_column_header` is the wider question the
draw asks (the column shows its top row either way, so `j` lands on a visible card), and it is
the only new predicate.

**`k` off a column header, once — including on an empty one.** The gate is
`on_column_header()`, not `cursor_row.is_none()`: an empty column is its own header with
`cursor_row == Some(0)`, and keying on the spelling would have cost two presses there and one
everywhere else. The press is REFUSED where `git.sampled` is false — with no repository under
the board the clause is not drawn, and a cursor on nothing is worse than a key that does
nothing.

**One section, so three keys.** `h`, `l` and `k` are unbound in `Scope::Header`: there is
nothing beside the git clause to walk to and nothing above the top row, and an unbound key is
inert, which is the honest answer until a second section earns them. `j`/`Down` come back,
`q`/Esc pop (never the board's menu — a scope pops the way every other scope pops), and Enter is
`Verb::Act` routed to `open_checkout_diff` — the same verb every list's Enter carries, on the
screen's own subject, which is how `Verb::Act` already reads on five other scopes.

**The paint is the header chip's, one register down.** `chrome::git_clause` takes a `focused`
flag: the spans get `Theme::selected_row()` patched over them, a pad cell is appended so the
run has an edge on both sides rather than running flush into ` 7 tickets`, and the greys move
from the `rest` ramp to `sel` — the cursor column header's own treatment, one row up. The
arrows stay in the calm register (being under the cursor does not change what an arrow means),
and where a profile can neither paint a surface nor reverse (light-256, a phosphor at 16) the
clause goes bold instead, because unlike a card or a column header this row has no bar cell to
weight.

**And the hint left the header.** ` v diff` was T-221's "a hint lives where it operates"; a
section the cursor can stand on operates in the footer, and having it in both places is exactly
the duplication that idiom exists to prevent. The board's `v` is untouched — same key, same
verb, still `prio: 0`, still listed by `?` — only the header stopped spelling it. The clause's
give-way ladder lost its first rung with it and is now two: the name truncates to
`GIT_BRANCH_FLOOR`, then the count drops, then the clause stands aside whole. Freeing those
eight cells moved every width in `test_git_clause_gives_way_to_the_offer` (the offer's 80-column
case became 90).

**`Ctx::on_header` was renamed `col_header`** (`App::on_header` → `on_column_header`) in the same
edit. With `Scope::Header` in the file the old name had two readings a line apart, and the field
already sat under keymap.rs's `---- the column under the cursor (T-117) ----` heading with the
`col_*` family. Thirty-three sites, no behaviour.

Pinned by `keymap::the_top_row_owns_three_keys` (the three keys, the four inert ones, the board's
own keys not reaching up, Enter gated on the sample, and the footer's exact four spellings),
`app::k_off_the_column_header_lands_on_the_top_row` (the walk, the refusal with no sample, Esc,
and the empty column's single press), `ui::golden_board_header_bar_120` (the paint, the column's
band surviving, its bar cell gone, the footer's HEADER word),
`ui::enter_on_the_header_bar_opens_the_checkout_diff` and
`ui::the_header_no_longer_spells_the_diff_key`.

## The rail offers the session, and the ticket's shell is gated (T-300, 2026-09-07)

An empty rail taught its two spawn verbs the way T-158 taught every other key — a trailer under
the list, in the keymap's own words: `c start claude ∙ s shell`. It was correct and it read
badly. A first-time reader arriving at a fresh ticket was asked to choose between two words
before either had a meaning, and one of the two is the one they did not want: a shell on a
ticket is a niche second seat, and the board's whole proposition is the agent. Worse, the
gesture was different from every other gesture on that screen — everywhere else you put the
cursor on a row and press Enter.

So the offer became a ROW. `RailRow::NewClaude` is a phantom with no record behind it, drawn
`+ claude session`, and `Enter` on it spawns through the same `spawn_and_focus` the `c` key
takes. It sits AFTER the sessions and BEFORE the notes, which is two decisions:

- After the sessions, because sessions-first is an invariant `board_enter` and the focus return
  lean on — a position in `rail_sessions` IS a `rail_idx`. A row at the top would have been a
  silent off-by-one in three places.
- Before the notes, deliberately (the user's ask: "even if there is a note on the ticket, focus
  this instead"). On a ticket with no session the rail therefore OPENS on the offer, description
  or not. The description is still on screen — the band under the identity line draws it — so
  nothing is hidden by the cursor starting on the row that does something.

It stands exactly when a press on it would work, which is the daemon's two refusals mirrored
(`App::new_claude_row`): one claude per ticket, so a live OR parked one takes the seat, and never
on an archived ticket, which may not grow a pane no board surface shows. A resumable corpse is
neither, so a ticket whose claude died shows both — `enter` on the corpse resumes that
conversation, `enter` on the row starts a new one. The archived ticket page used to hint `c start
claude` and get "ticket archived — restore it first"; it now says nothing there, which is the
first time that screen has been honest.

**`c` on the ticket page keeps one word.** The hint is `wake claude` or nothing. A claude that is
up is a row already listed (author 2026-09-03) and an empty seat is now a row too, so both would
be a second spelling of something the reader is looking at. The key stays bound in every state —
`binding_for` returns `None` on an empty hint, so it leaves the trailer and `?` without leaving
the keymap.

**`jk` gates on ROWS, not on sessions.** `Ctx::ticket_rail_rows` replaced `ticket_has_sessions`
on that binding, and the hint says `select row` where there is no session to select. The old
predicate was already wrong — a ticket with three notes and no session had an unwalkable rail —
and T-300 made it common, since a described ticket with no agent now holds two rows. One row is
not a list, so the key is inert there and the footer says so.

**The shell is gated, not removed** (the user: "keep the feature but gate it for now"). The
feature is whole underneath: the daemon still spawns `SessionKind::Bash`, the rail lists one,
the preview zone reads its pane through `PaneTail`, `x` sleeps it, and every e2e that drives a
shell over the wire is untouched. What is shut is the two doors — `s` and `S` on the ticket page,
and the board's overlay-only `s`, because where a ticket may not grow a shell no screen may start
one. `Ctx::ticket_shells` is the gate; `MESIMON_TICKET_SHELLS=1` opens it, read in `lib.rs::run`
and never `App::new` (the rule `editor_word` and `opener` follow, so no test and no golden reads
a developer's environment), and `doctor` prints a `ticket shells` line so a finger that remembers
`s` has one place to look. `!` never went behind the gate: the project's terminal (T-273) is a
place to stand, not a session of the ticket.

Nothing daemon-side moved — no `Command`, no `Snapshot` field, no schema, no e2e. Pinned by
`a_ticket_shell_is_behind_the_gate` and `the_offer_is_a_row_and_the_key_that_said_it_stands_down`
(`core/src/keymap.rs`), `the_rail_opens_on_the_offer_and_enter_starts_claude` and
`a_ticket_shell_needs_the_seam` (`tui/src/app.rs`), and the golden `ticket_new_claude_120x30`.

## `^k` also lists what the agent just said (T-307, 2026-09-07)

**Built.** `App::ticket_links` reads one more body: the ticket's latest agent words, appended
after the notes'. Everything downstream followed with no change — the dialog, `^K`'s first row,
the ticket page's ` ∙ ^k links` hint, `c copy`, all three kinds of target and the worktree-rooted
path resolution are the notes' road, so a URL an agent prints at the end of a turn opens the same
way the description's Jira link does. Before it, the only road to that URL was attaching to the
pane and clicking in tmux.

**The source is the PEEK, not the transcript.** `App::latest_words` is
`peek_cache.peek(path)?.text` — EXACTLY the words the card's peek row and the ticket page's
PREVIEW zone already show, so what can be read can be opened and nothing is listed from a part of
the transcript nobody can see. Reading the whole file instead would have listed every path the
agent touched all turn, which is noise, and would have needed a second cost model; the peek's
already exists (its module doc), the cache is already warm on any screen showing that session,
and the read is local — `fetch_links` still round-trips only for note bodies. It follows that the
list moves with the peek: while a new turn is in flight the peek is the user's own prompt (`>
…`), and its links are listed for the same reason the row shows it — "nothing" is the one thing
neither may say while the transcript plainly has something.

**WHICH transcript: the ticket's claude, pane or no pane.** `App::latest_transcript` is
`Board::pane_target` — the session the peek row, the spoke mark, prompt delivery and Enter all
pick, so `^k` cannot disagree with the card — and, when no pane lives, the newest claude record
the ticket has by `state_changed_at` (`rail_sessions`' shape for a corpse). What an agent said
last outlives its pane: a parked or finished claude is exactly the one whose final URL is still
wanted. The ticket page's rail selection is deliberately NOT consulted — `ticket_links` takes a
ticket and is called from a draw that has one, and a per-screen answer would make the board's
list and the page's differ on the same ticket.

**Notes first, latest words last.** A reply rewrites itself every turn and the description does
not, so putting the agent's words on top would have made `^K` — open the first link, no dialog —
mean "whatever was last mentioned" instead of "this ticket's link". Dedup is by target and first
occurrence wins, so a URL in both keeps the note's row and its label. A path at a different
`:LINE` is a different target and lists twice, which is what the target-equality rule already
said.

**`Ctx::ticket_described` became `Ctx::ticket_linkable`.** The key's gate was "the ticket has a
description", the cheap board-side proxy for "there might be something to list" — the bodies are
not on the board. A ticket with no note and a claude that just spoke now has something, so the
gate is `description().is_some() || latest_transcript().is_some()`: still board-only, no disk
read, asked once a keypress and once a frame. The field had no other consumer, so it was renamed
rather than joined by a second one. A ticket whose words hold no link still gets the status line
(`no links in T-12`), never an empty dialog.

**Not built.** A kind word or a column saying which body a row came from (the dialog is 64 cells
and the row already spends six on `url`/`ticket`/`file`); links from anywhere but the latest
words; a link mark on the card. Pinned by `the_agents_latest_words_are_links_too` (order, and a
target the notes already listed not listing twice) and
`the_latest_words_outlive_the_pane_and_are_all_a_ticket_needs` (a Sleeping claude on a ticket
with no note at all opens the dialog — the case the old gate refused).

## The workspace choice stays open until work starts (T-309, 2026-09-07)

**Built.** `Shift+Tab` sets a ticket's workspace from the board and from the ticket page, not
only from inside the description editor — and a worktree asked for but not cut wears a mark on
the card. The user's words: *"shift+tab on a ticket with no session should let the user change,
unless provisioned already ∙ also from ticket page ∙ also from description composer"*, then
*"we also need a glyph indicating unprovisioned worktree on the ticket"*.

**The choice was never once-only; the KEY was.** M4a's note says "Workspace is chosen ONCE, in
the composer", and that was read off the composer being the only surface that offered the key —
but `Daemon::set_workspace` has always refused on exactly two facts (any session record, any
worktree binding) and taken the write otherwise, and T-163 had already put the same key in the
description editor on a ticket that exists. So nothing about the rule moved: what moved is that
`Ctx::workspace_open` stopped being an EDITOR fact (`EditorPurpose::Note`'s ticket) and became a
SUBJECT one — the board cursor's card, the ticket page's ticket, the editor's note — which is
the same ticket in the editor's case, since the editor takes every key while it is up. The field
moved out of Ctx's editor block into its worktree block with it.

**One road, four surfaces.** `App::set_ticket_workspace` is the whole toggle: read the ticket's
strategy, send the other one, and say where it landed. The editor's `Note` arm now calls it too,
so the composer's draft (which has no ticket to send about) is the only arm left that computes
anything of its own. The status is `T-9 gets a worktree of its own` / `T-9 works in the shared
checkout` and is set only when the board came back changed — the daemon's refusal has already
put its own sentence there, and the TUI's gate is a mirror, not the authority.

**The hint names the DESTINATION, and only on the ticket page.** `Ctx::workspace_worktree` picks
between `own worktree` and `shared checkout` — `t`'s idiom (`auto-merge` / `merge by hand`),
which is right here because the card's mark and the page's state row already say where the ticket
STANDS, so a hint repeating that would be the second spelling T-158 spends its rules avoiding.
The board's binding is `prio: 0`: its footer is at its width at 120 columns (` enter go to the
agent ∙ space ticket page ∙ o new ticket ∙ HJKL move card ∙ tab describe` plus the app cluster),
and this is the `n`/`s` treatment — bound, listed in `?`, unhinted on the row. The ticket page's
sits at 64, between `t` and `r`, with the worktree keys.

**`⎇·` is the gap between the pick and the tree.** Provisioning is lazy — the worktree is cut at
the first spawn — so a ticket set to `Worktree` and never started drew NOTHING on the card, and
the board's new key had no answer to show for itself. `card::worktree_mark` grew a no-binding arm
(it took the ticket, which `render` already had) returning one dot in a new `WtTone::Dormant`,
`dim3` on the resting ramp and `sel.dim3` under the cursor. One dot against `queued`'s three
reads as less than being provisioned, which is what it is; ASCII is `.`; the mark is two cells
like every other, so the glyph keeps its column (the anchor rule
`the_worktree_glyph_holds_one_column` states). A binding of any status takes the dot back — it is
not a second way to say "worktree". `·` U+00B7 is outside the banned box range, so the L1 law
needed no exception (unlike `▀` and `▎`).

**The press says why, and that is the second `m` (2026-09-07, dogfooding).** The first cut made
the key inert wherever the choice was locked — the ordinary "a key that is not available is
inert" rule — and the report back was *"i still can't do shift+tab to change, it doesn't change
anything"*, which is the rule's failure mode: on a board where most tickets have a session
record, the commonest press is the one that says nothing. So the binding took `m`'s shape —
`avail` is a card that is not archived, `keymap::workspace_hint` is EMPTY while the press cannot
act — and `App::set_ticket_workspace` answers in the ticket's own words: `T-9 has a session — the
workspace is fixed once work starts`, `T-9 already has a worktree`, `T-9 stays in the checkout —
this board is a workspace`. The invariant `m`'s comment names is unbroken: a key that is HINTED
always works. The workspace-board clause moved into the TUI with it, because the daemon would
TAKE that field — T-225 refuses a worktree ticket at `resolve_spawn_cwd`, not at `set_workspace`
— so silence there was the TUI's to break.

**The composer's ring lost its invisible stop (same day, same report).** T-117 gave the composer
three stops so a column defaulting to a worktree could still compose a shared ticket: `None`,
`Some(Worktree)`, `Some(SharedCheckout)`. Where the column has no workspace default — nearly
everywhere — `None` and `SharedCheckout` render the same word, so the ring read *shared,
worktree, shared, shared* and coming back from `worktree` cost two presses (*"requires two clicks
after returning to shared"*). `App::cycled_workspace` is two stops and always EXPLICIT: the two
strategies, resolving the current `None` through `board::DEFAULT_WORKSPACE` first. Naming the
pick costs nothing — `create_ticket` stamps the column's default only where the field is absent,
and both stops reach the same two outcomes — and the readout the third stop was standing in for
is the row's own `(column default)` tail, which is a comparison against the column and needs no
state of its own. The editor's Compose arm was a DIFFERENT two-stop ring (`None` ↔ `Worktree`,
which could not reach `shared` at all on a worktree-defaulting column); it calls the same helper
now.

**And the lock is the WORKTREE, not the conversation (same report).** Reading the author's own
board settled where the silence came from: of 44 live tickets, 26 were locked and only 9 of those
had a worktree — the other 17 were held by a session record alone, 15 of them a `Sleeping` claude
in the shared checkout, several sitting in BACKLOG waiting to be given a tree. That is the lock the
ticket named when it said *"unless provisioned already"*. `Daemon::set_workspace` now refuses on a
worktree binding, or on a session with a PANE (`SessionState::has_pane`, which is exactly "not
`Exited`, not `Sleeping`"), and the TUI mirrors it. The reasoning is the M4a one, applied
honestly: the lock exists so the choice never RELOCATES something, and a parked record cannot be
relocated — `resume_session` replays the record's own absolute cwd and re-resolves only when that
directory is gone (T-278). So the field means what it always meant, "where the next spawn goes",
and a sleeping conversation has no opinion about it. A live agent still locks: its directory is
where it is, and saying otherwise on the card would be a lie about a running process.

**Not built.** The daemon's `set_workspace` is the only thing that moved daemon-side — no
`Command`, no `Snapshot` field, no schema. The ticket page's state row was left
alone: it already says ` ∙ ⎇ worktree` for an unprovisioned one and says nothing for a shared
checkout, which is the right silence for the default. A shared-checkout ticket gets no card mark
for the same reason. And the daemon was NOT loosened to allow a switch over a dead session record
— a resumable corpse's cwd would move under it — so "no session" stays literal.

Pinned by `the_workspace_choice_is_open_until_work_starts` (the unhinted-but-live clause too) and
`a_workspace_board_offers_no_worktree_choice` (`core/src/keymap.rs`),
`shift_tab_on_a_card_sets_the_workspace_too` and
`the_composer_starts_at_the_columns_workspace_default` (`tui/src/app.rs`),
`a_worktree_asked_for_but_not_cut_wears_a_dormant_mark`, the goldens
`board_worktree_planned_120x30` / `ticket_new_claude_120x30`, and
`exit_parks_e2e::leaving_claude_parks_the_session`, which now asserts both sides of the lock — a
parked claude beside a dead shell opens it, a resumed one closes it.

## A turn that starts without a prompt answers the hand (T-311, 2026-09-07, dogfooding: "after answering, the ticket stayed as needs you")

An agent on the author's simbly board raised a hand asking for `gcloud auth login`, the user ran
it, the agent went back to work — and the card kept its `!` and its reason for the rest of the
session. The board's own feed is the whole diagnosis:

```
252 raise_hand              (agent)
253 PostToolUse             (the raise_hand call's own frame)
254 Stop
255 session_state running -> idle end_turn   high
256 automove                (→ REVIEW)
257 PostToolUse             28 s later
258 session_state idle -> running            high, hook PostToolUse
259 automove                (→ IN PROGRESS)
```

Not one `UserPromptSubmit`, and that hook was the only thing on the agent's side that lowered a
hand. The user answered with Claude Code's `!` bash — the command's output goes into the
conversation as a user message and the model takes a turn on it, firing no prompt hook whatever.
T-228 measured exactly this two days earlier and taught the ATTENTION machine about it (line 258
is T-228's rule working); the HAND never learned it, so the `!` stood on a card that was visibly
working again.

**The fix is the clause T-228 already built, read a second time.** `Daemon::apply_change` lowers
the hand on `Idle{EndTurn}` → `Running` at High. T-107's half is untouched and is why the rule is
worded as a turn STARTING rather than as a session going busy: the `Stop` that lands moments after
the call must not be an answer, and it still is not — nothing about the hand is derived from the
session's state, it is a ticket field with two writers and now three erasers.

**Three conditions, each excluding a road that is not a person answering.** Only from `EndTurn`:
`Idle{Background}` is a PARK, and a teammate's report resumes it with nobody involved (T-135). Only
at High: `SubagentStop` promotes a sub-High idle back to `Running` as a CORRECTION of a misread
tail, not as a new turn, and a hand may not come down on an inference — that is the same reasoning
that keeps the 15-minute stale demote away from it. And only into `Running`: a `RequiresAction` →
`Running` is a permission dialog resolving mid-turn, which answers a tool, not a person's question.

**Not built.** The board CURSOR still lowers nothing — a glance is not an answer (T-107), and this
report is the opposite case, a person who acted. No `Command`, no `Ctx` field, no schema, no key.
`lower_hand_on` grew a second caller and its doc comment now names both roads as one idea.

Pinned by `raise_hand_e2e::a_raised_hand_outlives_the_turn_and_is_lowered_by_the_person_or_the_next_turn`,
which sends the real `Stop` and then a real `PostToolUse` frame down the hook socket and asserts
the hand goes down and the card goes back to working. Removing the clause times the test out.

## The links key is listed only in `?` (T-312, 2026-09-07)

Asked for as "remove `^k` links hint from ticket page". T-256 put the key on the ticket page's
state row in T-158's idiom — a hint sitting beside the thing it operates on — and gated it on
`App::ticket_links` being non-empty, so the row named the key only while a fetched body held a
link.

What changed: `ui/ticket.rs` no longer builds that span. The binding is untouched (`prio: 0` on
`Scope::Ticket`, `avail: |c| c.ticket_linkable`), so `^k` still opens the dialog from the page and
`?` still lists `^k links` there, beside `^K open first link` and the screen's other overlay-only
keys. The board's copy was overlay-only from the start and never had a hint to lose.

Why: this is T-277's shape a second time. The state row says what the ticket IS — its column, its
age, its tags, its branch, a raised hand's sentence — and a key that blinks into that row when a
note body happens to arrive is a hint whose appearance depends on a fetch, not on the moment.
`?` is what exists to list a key that is simply there.

The test flipped rather than went:
`the_ticket_page_names_the_links_key_only_when_there_are_links` became
`the_ticket_page_never_names_the_links_key_on_its_state_row`, which asserts the row is silent with
a link fetched AND without one, and that `keymap::overlay` still carries the row. No golden moved
— no golden ever had a fetched note body with a link in it.
## The empty seat previews the session it would start (T-308, 2026-09-07, user: "new claude session preview should be nicer. maybe ascii art from claude code with 'Press Enter to ...'. your take?")

T-300 gave the ticket rail a phantom `+ claude session` row and made it the row the cursor opens
on. The PREVIEW zone beside it drew NOTHING — `draw_preview`'s chain is shell, then note, then
reply-or-working, and the offer row is none of the three — so the one row on the page whose whole
purpose is a press nobody has made yet sat next to the emptiest half of the screen.

**The zone previews the SESSION, not a document.** That is the reading of the word that made the
change worth more than decoration: everything else the zone shows is the last of a record, and
here there is no record, so what it shows is what the press would produce. `empty_seat` in
`tui/src/ui/ticket.rs` draws a mark, the press in the keymap's own words, and two or three
clauses — where it will run, what its column hands it, and who else is already writing in the
same checkout. Every fact is one the page already holds (`Ticket.workspace`, the column's
`ColumnSettings`, `Board.mcp_tools`, `App::checkout_busy`); nothing new crosses the wire, no
`Command`, no `Snapshot` field, no `Ctx` field, no key.

**The art is redrawn, not borrowed.** Claude Code's own welcome screen — measured on 2.1.263 —
is Clawd and a starfield built from `█ ░ ▒ ▓` and the quadrant blocks, sixteen rows tall inside a
58-cell box. All three of those are disqualifying here: that codepoint range is exactly what L1's
`test_no_drawn_structure` bans (two admissions, `▀` and `▎`, both already spent), sixteen rows is
more than this zone has at any terminal size worth drawing it in, and it is somebody else's brand
art in a third-party tool. What crosses over is the IDEA — a mark with a sparse field around it —
as `SPARK`, five rows of ASCII hand-authored like a palette and never generated. ASCII also means
one drawing for all four glyph tiers instead of a mono fallback, and the L1 sweeps now render it
so a future edit that reaches for a block glyph fails the law rather than the eye.

**Greyscale, and no motion.** The burst's star is the value step, its spokes one under, the field
one under that — the three brightnesses Claude Code gets from `░ ▒ ▓`, taken off the grey ramp
instead. Not one hue: the board's single saturated colour is needs-you's and a decoration may
never spend it. Nothing pulses either — D19's motion ban bends only for something that is moving,
and nothing here is.

**The words come from the keymap.** The press row is `binding_for(Scope::Ticket, Verb::Act)`
through `chrome::hint_spans`, the same binding the footer is drawing two rows down, so the zone
and the footer cannot disagree about what Enter does (T-158's one-home rule). It renders `enter
start claude` rather than the ticket's "Press Enter to …" — the house dialect, and it follows the
binding when the hint changes.

**What the clauses say, and what they refuse to say.** `starts in a worktree of its own` /
`starts in the checkout`, never the branch name: the state row four lines up already carries `⎇
msmn/T-3-slug`, and naming it here would be the only thing this clause could add, said twice. A
column's `claude_mode` and `agent_tools` are appended only where they differ from what a spawn by
hand would get — a column that changes nothing has nothing to preview — and `Board.mcp_tools` off
reads the same as `AgentTools::Off`, because from the seat's point of view it is. The second row,
`types the ticket title into its box, and sends nothing`, is the road's own contract: this press
is `spawn_session(.., submit_prompt: false)`, which types the title and stops, so the brief does
NOT travel (`server.rs` parks `Parked { brief: true }` only under `submit_prompt`) — the one place
where a reader can see the difference between this key and the composer's Shift+Enter at the
moment it matters. The third stands only under `App::checkout_busy`: this road does not queue, so
Enter here puts a second writer into a checkout somebody is already in — T-294's hazard, at the
press that can still cause it.

**The picture gives way before the words.** Under `SPARK_MIN_H` (12) rows of zone the art is
dropped and the sentences stay whole, which is how a described ticket on a 20-row terminal reads:
the description takes the rows off the top, and the press needs the sentence, not the picture. The
mark is centred over the TEXT block rather than over the zone — the zone is 87 cells at 120x30 and
the sentences are ~55, so centring in it left the picture floating off to the right of everything
it is about (built that way first, seen once, changed).

**Not built.** A hint pointing at the board's Shift+Enter for the description (a hint for another
screen's key, which is what T-158 removed); the same treatment for a freshly spawned claude whose
transcript has not landed yet, where the zone is also blank — a real gap, and a different fix; art
that pulses, scales with the zone, or has a second variant. Goldens `ticket_new_claude_120x30`
(plain) and `ticket_new_claude_worktree_120x30` (a worktree ticket in a column that narrows both
mode and tools), plus `the_empty_seat_drops_its_mark_before_its_words` for the give-way.

## And the session with nothing to read yet (T-308, second half, 2026-09-07, user: "fix the adjacent gap as well, think about a nice thing to show while claude is working but no transcript yet, also it might NOT be working, and no transcript yet")

The empty seat was one of two blanks. `draw_preview`'s chain was shell, then note, then
`reply.is_some() || working` — so a SELECTED session with no readable reply and no `Running`
state drew nothing at all, and `ticket_corpse_selected_120x30` was 87x16 cells of void beside a
row saying `enter resumes`. Six real states landed there: a claude coming up, one waiting for its
first prompt, a permission prompt raised before any assistant text was persisted, a sleeper or a
corpse whose transcript is gone, a session `Unknown` after a daemon restart, and a live shell
whose pane has not been captured yet (up to one second of `poll_shell_tail`'s clock).

**The chain became `reply` / `record` / `seat`.** `draw_preview` takes the `&SessionRecord`
now instead of its uuid, and `working` stopped being a branch condition — a working session is a
selected one, so the record arm covers it and `working_row` (the pulse plus the newest tool call's
own title) was lifted out to serve both the reply's closing indicator and this arm's headline.

**A report, not an invitation, which is why it has no press row.** `empty_seat` spells `enter
start claude` because the offer row carries no hint of its own; a session row is already spelled
twice — its own `enter resumes` badge and the footer — and a third would be T-158's rule broken by
the change that cites it. What this owes the reader instead is WHY there is nothing, which nothing
else on the page says. `quiet_words` is pure over the record so the sentences can be read in a test
without a frame, and it reuses `glyphs::state_word` wherever nothing better is true: this zone may
not invent a second name for a state the card and the rail already name.

**Before the first turn, the interesting fact is the BOX.** `spawn_session` types the ticket title
into the pane on the way up and then either stops or owes an Enter, so `waiting for you ∙ the
ticket title is in its box, unsent` and `starting up ∙ … ∙ mesimon presses enter when it is ready`
are the two halves of what actually happened, and `pending_submit` — T-224's retry clock — is what
tells them apart. The gate is `Idle { stop_reason: Unknown }`, the one `Idle` that
`SessionState::has_prompted` refuses: an `EndTurn` with no readable words is a FINISHED turn, not
a fresh box, and keeps the rail's `done` (built without that gate first, and `ticket_queued`'s and
`ticket_raised`'s goldens both said "waiting for you" over a `✓` row — the goldens caught it).

**A turn in flight gives the row to the pulse**, and `nothing said yet ∙ the first words land when
the turn does` under it. The mark is NOT drawn there: it was, for an hour, and `● working` under a
starburst read as "nothing here" beside a row saying something was happening. So the rule is the
narrower one — the mark stands while the conversation has not STARTED (`Spawning` or the never
prompted `Idle`, no transcript, claude), which makes it the empty seat's own face one press later
and means the zone does not blink between the press and the first prompt. Never over a corpse, a
sleeper, a failure or a raised prompt.

**A needs-you session's headline is its own question**, in the attention register the rail row
beside it already wears. Two copies of one sentence on a page is the cost; the rail cuts it to 26
cells and the zone has 87 and wraps to three, which is the card-versus-page split the peek row and
the PREVIEW zone already make — the index truncates, the reading surface reads. The law that
reserves the saturated colour is about what it MEANS, not how many cells spend it, and this is
needs-you by the same road the rail's copy is. The alternative considered and refused: greying the
zone's copy, which would have put the only loud copy on the truncated one.

**And a sleeper with no conversation says so before the press**, not after: `resume_session` mints
a FRESH conversation under a new uuid where there is no transcript to resume (D24 makes that free),
which is a thing worth knowing while the cursor is on the row rather than in the status line
afterwards. A shell is the inverse — it keeps no transcript by design, so "nothing to read" is
never news about one, and what the blank actually means there is `reading its pane`.

**Not built.** A press row (above); a countdown or elapsed clock (the rail's age slot has it); the
last USER prompt as a stand-in for the missing reply (`peek` already falls back to it prefixed `>`
and a zone-sized quote of your own words is not a preview of the agent's); wrapping the question
past three rows. Pinned by `the_quiet_zone_says_why_there_are_no_words` (the words, per state,
frameless), golden `ticket_starting_120x30`, and both L1 sweeps now render the question.

## The sleep canary's cheap half is gone (2026-09-07, user: "remove 'resume may lose context'")

`00-DECISIONS` §"The B-A22 sleep canary is deferred" shipped one half of it: at sleep time the
transcript copy was scanned for at least one `user` and one `assistant` record, and a copy without
both parked the record with `resume may lose context` in `SessionRecord.detail` instead of blocking
the sleep. That warning is removed, `transcript_has_conversation` with it, and the copy itself
stays. The doc keeps the decision as history — the code is the spec.

**It was wrong twice over.** The wake does not read that copy: `resume_session` replays argv with
`--resume`, and `resume_transcript_missing` checks `rec.transcript_path` and then every directory
under `~/.claude/projects/` — Claude's OWN store — so a thin snapshot in `<state>/transcripts/`
costs a wake exactly nothing. And where there genuinely is no conversation to resume, the wake
does not come back amnesiac either: `resume_session` mints a FRESH one under a newly minted uuid
and answers `Spawned { fresh: true }`, which is the thing the user is told. Losing context was
never the outcome the sentence named.

**And only one of the two sleep roads said it.** `park_on_exit` — the clean-exit park, the road a
Ctrl+C-out or `/exit` takes — has always made the same copy with no check and no detail, so one
gesture ("this session is parked") was answered two ways depending on which road reached it. The
two blocks are now identical, which is the form the next reader should find them in.

**`rec.detail = None` is load-bearing and stayed.** The old code assigned `warn` there, which
happened to CLEAR a stale detail as a side effect; deleting the assignment would have let a
`RequiresAction` question outlive its state on the parked record, since `apply_change` — which
wipes detail outside the attention states — is not on this road. It is now an explicit clear with
a comment saying why. T-308's `quiet_words` appends `detail` as a clause in its default arm, so a
survivor would have been drawn on the ticket page's PREVIEW zone.

**Not built.** The full B-A22 canary (a first-launch canary session and the per-machine capability
flag) stays deferred, as `00-DECISIONS` says. Nothing replaces the warning: a sleeper whose
conversation cannot be resumed already says so where it matters, in the PREVIEW zone's `no
conversation to resume ∙ waking it starts a fresh one` (T-308), which is read off
`transcript_path` at draw time rather than latched into a record at sleep time.

## Terminal history scrolls one line per wheel event (2026-09-08)

Mesimon enabled tmux mouse handling but inherited the five-line wheel step in both
`copy-mode` and `copy-mode-vi`. Each wheel event now scrolls one line in either direction,
retaining pane selection. Root-table forwarding is unchanged, so applications that handle
mouse input still receive it. The same binding definitions render into fresh configs and
are installed on surviving tmux servers at daemon startup; a reload does not require killing
agent sessions. `wheel_scrolls_one_line_on_fresh_and_surviving_servers` checks the bindings
against real tmux, including replacing old five-line bindings and repeated installation.


## The shin welcomes a session and carries notification attention (2026-09-08)

The identity plan's shin becomes the installer welcome, the ticket preview's
empty-seat mark, and a notification image (user: "yes and also do the notification
icon"). The terminal art is hand-tuned: 24 columns by 12 rows in the installer,
20 by 7 in the preview, where a literal copy would crowd out the start instructions.
The installer shows it only after installation succeeds, on a UTF-8 TTY that is
not dumb. Piped output keeps the normal text. `ci/mascot.py` owns both text drawings,
the filled SVG geometry, the antialiased PNGs, and the installer's embedded copy.

The preview replaces T-308's starburst in exactly its two homes: the offer of a
new Claude session and an unprompted session with no transcript. It stays out of
working conversations, sleepers, corpses, questions and card/header chrome. All
instructions are reserved before the art is admitted, including the busy-checkout
clause. The mark uses `dim1`, never attention color or motion; Mono uses the
wordmark. L1 now admits block glyphs at the shin's exact recorded cells, matched
to the checked-in drawing. It does not admit them across the preview rectangle,
let alone across the TUI. The existing content scrub and dialog-perimeter law stay.

Notification color follows a semantic `Post.needs_you` bit, derived from the
coalescer's loudest event. Quoted words and customizable sounds cannot choose
an attention icon. A mixed batch gets the detached amber tip; a completed turn
gets the resting silhouette. Both PNGs are embedded, published atomically under
the existing per-repository state allowlist, and named by their content digest.
Only an image-capable post materializes them: neither doctor, discovery, disabled
notifications nor sound previews write. An asset failure leaves the banner intact.

Linux passes `--icon` before notify-send's `--`. A positively identified
terminal-notifier 2 bundle gets `-appIcon`; version 3 removed that API, so newer
and unknown bundles get `-contentImage`. That is an attachment, not a promise of
a changed application identity. The bundle version is read without launching a
GUI helper. Click activation, tab reveal and per-board grouping survive; no sender
spoofing or helper-bundle rewriting is introduced. osascript, OSC and custom
programs keep their existing contracts. Notification images use a graphite tile
so their contrast is independent of the system notification's background.

Pinned by preview goldens and the size/Mono/state tests, the scoped L1 sweep,
`attention_presentation_follows_events_not_words_or_sound`, notification argument
and version tests, atomic asset/symlink tests, and the offline installer tests.


## The OS icon needs an app identity, not an accepted flag (2026-09-08)

The first shin implementation classified terminal-notifier by its bundle version
and trusted version 2's `-appIcon`. The user reported the unchanged OS icon. The
new code had already materialized the resting PNG on macOS 26.6.2, so an old build
or a missing asset was not the explanation. The flag sets private notification
properties; accepting it is not evidence that the OS paints them. A private copy
of the installed helper, given the `io.mesimon.notifications` identity, the Mesimon
name and an ICNS mascot, and ad-hoc signed, DID show the icon. The user confirmed
the real OS notification. This supersedes the earlier version-based icon rule.

`notification_app` now snapshots the discovered helper bundle, rejects symlinks
and special files, and hashes all its inputs plus the icon into an immutable
app generation. It copies ordinary files into an isolated staging directory,
changes only that copy's identity, then signs and verifies it with macOS tools.
Only a sealed generation is published; simultaneous boards can converge on the
same generation, and an upgraded helper never replaces an executable still
handling an old notification. Setup children share a ten-second deadline and
are reaped. Neither discovery nor doctor runs this preparation.

The home moves to `~/.local/state/mesimon/notifications/`, shared across boards
because an OS app identity is shared across boards. This stays inside the
existing write allowlist. The source helper remains untouched. The resting
mascot is the real application icon; attention posts additionally carry the
amber image. Unknown bundles retain an attachment fallback. There is no
`-appIcon` or `-sender`, and the click/reveal/group arguments are preserved.
A setup failure still posts the ordinary notification and reports the lost
app icon. macOS may request permission for Mesimon's separate identity.

Pinned by native signing/verification without posting, source-preservation,
upgrade, simultaneous-publication, failed-signature, symlink and timeout tests,
and the existing notification argument checks. The live visual confirmation
covers the actual system icon, which argv assertions never could.
The user also verified the finished integration on a running agent: the OS
notification displayed the mascot correctly.


## Tag bars reserve two cells and separate two colors (2026-09-09, user direction)

The single-cell half-block was too small to identify two tags without peeking. The user
accepted a fixed two-cell bar, then requested a narrow gap so the colors do not bleed into
one another. Every card now reserves `[bar 2][pad 1][content][pad 1]`, including untagged
cards, move previews, the title editor, and expanded cards. Column headings and editor
cursors use the same inset. This deliberately costs one title column; tag count never moves
text. The workspace selector uses `(default)` and only shows its inline key hint when the
whole hint fits.

One tag fills both cells with continuous paint; no tags keeps a neutral two-cell bar. Two
tags render as `▉▉` (U+2589 LEFT SEVEN EIGHTHS BLOCK), each in its tag color with the normal card
background behind it. The gutter keeps this contrast-checked background on attention and
delete rows too, so a tag near the attention hue cannot disappear into the title band. The unpainted eighth provides separation even when both tags
have the same tint. Each color keeps its column when a card opens; the old 70/30 vertical
stack and `▀` tag exception are retired. U+2589 is the deliberate replacement exception to
the block-glyph restriction. Its East Asian Width Ambiguous behavior still depends on the
terminal, as the previous half-block did. Profiles below TrueColor retain the underline and
ASCII/state-bar fallback at the same fixed width; no RGB colors or new block glyphs leak
into them. Additional tags are still named in the peek and ticket page.

Tag colors now keep full strength away from the cursor. Selection still has its title and
surface, and the neutral bar retains its two levels. The ten hue identities are shared across
all six themes, anchored to the former Graphite palette in OKLCH. Candidate lightness starts
at 0.72 on dark themes and 0.48 on light themes, chroma at 0.15; chroma is reduced at fixed
hue to fit sRGB, and lightness adjusted where needed to clear 4.5:1 against both page and
selection surfaces. The resulting RGB tables were reviewed in the comparison prototype.
Stored tint indices are unchanged, but Blue/Amber/Green colors intentionally move to the
common hue mapping once. No persistence migration or user file rewrite is needed.

This supersedes the mandatory tag fade, the tag chroma ceiling/2x attention margin, and the
per-theme forbidden hue bands. Needs-you retains its own token, glyph, and full title band.
Replacement color tests check every theme/index for 4.5:1 surface contrast, retained chroma,
pairwise CIE76 separation of at least 12, and cross-theme OKLCH hue drift at most one degree
after RGB rounding. Rendering tests check full strength off the cursor, equal-tint gaps,
row backgrounds, fixed editor geometry across color profiles, and stable columns when open.
Board/editor goldens intentionally move their content inset by one cell; the title and reply
truncation changes are the corresponding width cost.


## Column overflow uses counted cues instead of faded cards (2026-09-09, user screenshot)

The bottom overflow preview still rewrote the first span as a one-cell ghost bar. After the
tag bar grew to two cells, that shortened a single-tag preview and shifted its title left;
a two-tag preview also left the second glyph behind. More broadly, dimming a real ticket
made overflow look like a different ticket state, and placed it too close to the footer.

The copied/faded edge cards are removed. Whole visible cards retain their normal tag bars,
colors, and geometry. A separate `↑ N above` or `↓ N below` row names the hidden count at
its edge, with blank separation from cards. ASCII profiles use `^` and `v`. Counts include
the boundary cards omitted to make room for a cue; hidden needs-you cards remain counted by
the header's `!N`. The ordinary chevron counts in the header no longer duplicate the cues.

Columns reserve two additional blank rows above the board's existing footer gap, so both
cards and the bottom cue stop earlier. The selected card and live input take priority over
scroll margins: if a tall card needs the cue rows, the directional counts move into the
header. If a group exceeds the entire body height, scrolling keeps its live input visible.
This supersedes the original edge-ghost treatment, while retaining cursor-following scroll
and whole-card boundaries for ordinary cards.

Tests walk both directions across all six themes with collapsed and expanded cards, checking
exact hidden counts, title alignment, both tag cells, cursor visibility, and footer clearance.
Additional checks cover hidden attention, ASCII arrows, and a long expanded card with a live
prompt at minimum terminal height. Three new goldens cover overflow below, above, and both.

## Claude observation lab (research/claude-state-lab, 2026-09-09)

A separate worktree preserves baseline `5798976` and provides a compiled before/after
board comparison. `docs/claude-state-map.md` is the current observation map;
`docs/claude-state-lab.md` records the research protocol and reproduction commands;
`docs/claude-compatibility.json` distinguishes live evidence from untested behavior.
The JSON fixtures in `docs/state-scenarios/` run offline through the production hook
adapter, transcript classifier, attention reducer and automove eligibility rules.
Live Claude capture is explicit and bounded, restricted to Haiku/Sonnet. Interactive
capture uses a private tmux server and independent screen/file checkpoints; print-mode
turn/dollar limits are not represented as interactive limits.

Measured corrections in this branch:

- A successful Haiku 2.1.266 continuation emitted Stop(false), Stop(true), SessionEnd.
  `stop_hook_active` describes previous continuation, not a reason to discard the
  final Stop. Nested agent Stops remain excluded. Stop remains an attempted stopping
  point: another hook can still continue the session; this branch does not claim a
  universal final-stop oracle.
- Modern assistant `end_turn` and system `stop_hook_summary` completion are classified
  by the same structural reader used for status-file corroboration. Observe-tier
  completion remains Low and cannot automove. Trailing attachments do not erase a
  finished recovery hint. Current capture had end_turn records and attachments,
  without the old turn_duration record.
- Text accompanying a tool call no longer conceals the in-flight tool. Preview text
  has its own reader; census/preview behavior is retained. The cursor holds a tool-ID
  ledger, initialized from bounded history, and a quiet observe-only transcript does
  not demote known outstanding tools. Parallel results clear only their own IDs.
- Raising confidence on the same state now publishes a Change to the daemon. This
  persists the confirmation and reevaluates automove, without resetting the state's
  age/waiting timestamp or announcing duplicate attention. Previously the machine
  silently changed confidence while the persisted record and board stayed behind.
- The demo caught hook reader threads delivering PreToolUse before the earlier
  SessionStart/UserPromptSubmit connections. Ingestion now preserves accept order.
  Concurrent readers report a frame or a skipped slot; an absolute 750 ms poll
  deadline releases malformed/stalled senders. macOS rejected updating SO_RCVTIMEO
  after a peer closed with buffered data, so the bounded reader uses nonblocking
  poll instead. This preserves connection order, not unknowable upstream causality.

`state_decision` and `movement_decision` entries extend the existing rotating activity
log, with metadata-only state/pending/confidence projections. Repeated unchanged
passive probes are suppressed. `mesimon state explain [session-prefix]` reads current
state and recent decisions without starting a daemon; `state replay` runs fixtures;
`state compatibility <version>` reports measured coverage without changing config.
MCP tool text, board-state authorization, private tmux selection and rendering laws
remain in their existing layers. Live unknowns (quota, team combinations, compaction,
early recordless Esc, and source-generation correlation) are explicitly listed rather
than treated as covered by synthetic fixtures.

## 2026-09-09 — live Claude state verification and four additional gaps

The state lab now launches real Claude through Mesimon's production spawn path,
retains the generated observer registrations, adds non-deciding capture hooks,
and checks real board snapshots against independently reached UI/tool barriers.
`ci/claude-state-e2e.py` is explicitly invoked through `ci/test-run.py`; each run
has a registered `test_guard.py` owner. Ordinary tests never launch Claude.
The per-case evidence index and limits live in `docs/claude-live-verification.md`.

Controlled Haiku 2.1.266 observations refuted four assumptions:

- **Permission held after Esc.** The UI and transcript recorded interruption,
  but abort-only tail candidacy excluded RequiresAction. It now includes held
  attention. A newly minted cursor can recover only a timestamped current-spell
  abort, stopping at newer turn evidence; undated/old history cannot seed it.
- **An early Esc is outside a 250ms race window.** A real recordless cancellation
  wrote idle about 120ms after prompt delivery. Its timestamp was ignored forever.
  Idle inside the existing margin now requires two observations at least two
  seconds apart, with the same stamp newer than the spell. Busy, changed stamps
  and stale evidence cancel confirmation. This remains Medium inference, not a
  claimed supported Claude status API or a solution to arbitrary source reordering.
- **Tool completion adequately clears approval.** A controlled tool file barrier
  proved Bash was running while the card still said permission. The daemon can
  now clear Permission after observing the same live session waiting during that
  permission spell, then busy at a strictly newer timestamp. This qualified
  Medium signal cannot clear Question, Plan or other attention states. Explicit
  transcript cancellation continues to handle refusal/cancel instead.
- **Every compact restart continues a turn.** Manual `/compact` emitted
  PreCompact(manual), SessionStart(compact), PostCompact(manual), and returned to
  the prompt without a Stop. Mesimon now registers PreCompact/PostCompact,
  exposes Running during compaction, and restores the state/confidence from
  before manual compaction. Its saved context survives a late async compact
  SessionStart and duplicate PreCompact, and is cleared by new prompt/session
  activity. Without saved context, manual PostCompact falls back to Low
  Idle/unknown, never inventing completion. Automatic compaction keeps the
  continuation behavior; real automatic compaction has not been certified here.

The investigation also caught harness errors and an intended product refusal:
No was the third permission choice; MCP forms require confirming the field
before their Accept/Decline controls; early Esc restores multiline input which
one Ctrl+U does not fully replace; compaction needs enough conversation messages;
line wrapping changes dialog text; and rapid setup turns activated the existing
six-move automove fuse. These attempts are retained as failed/inconclusive
captures, with reviewed explanations, not silently counted as verification.
Compaction preparation temporarily disables REVIEW's on_working rule, restores
it before the measured operation, and records that configuration explicitly.
The fuse itself remains unchanged and has existing regression coverage.

A local non-forwarding HTTP server supplies authentication, rate-limit, server
and model-not-found errors to the real CLI using a placeholder credential. These
are labelled injected API tests; they do not certify real account quota behavior,
quota auto-resume, or upstream availability. The local MCP server uses the actual
elicitation/create protocol. Chrome is explicitly disabled in new captures:
strict MCP configuration alone did not disable the built-in Chrome integration.
No browser permission was granted in the inconclusive compaction attempt.

## Monitor identity and version-pinned live checks (2026-09-09)

Connected Haiku verification on Claude 2.1.267 refuted classification solely by
`background_tasks[].type`: a successful top-level Monitor returned
`tool_response.taskId`, then Stop listed that same ID as `type: shell`. With the
watch dormant and LAB_ARMED visible, Mesimon incorrectly stayed Idle/background
instead of completing. Capture `20260909T190552Z-e2e-monitor-wakeup-dfd43b` retains
the failure; `20260909T191130Z-e2e-monitor-wakeup-83f237` verifies the fix and an
actual monitor event/continuation/completion cycle.

The shared ingest adapter learns bounded exact IDs from successful top-level
Monitor results. The session record persists them, so daemon restart cannot
forget the classification. New/resumed conversations clear them; compaction
retains them; complete Stop task lists and TaskStop remove stale IDs. Unknown
shells and child results stay conservative. Matching does not inspect commands
or descriptions. Replay calls the same adapter, and `state explain` includes
the retained IDs. A real daemon test covers persistence and an ordinary build
beside the monitor; synthetic negative variants cover child/new-conversation
identity boundaries. No TUI rendering or model-input surface changed.

Automatic compaction, one-shot cron and self-paced loop wakeups also passed on
2.1.267. Compaction used a 100K configured window and generated Read inputs after
three small conversation turns; actual auto hooks, a saved compact boundary and
continued work were required. The observed path is reactive compaction with an
explicit threshold; default proactive behavior is not certified. All three
wake sources emitted UserPromptSubmit without driver submission. Full evidence
and remaining limits are in `docs/claude-live-verification.md`.

The installed Claude changed versions between launches. The live runner now pins
an exact resolved executable and records its hash, the script hash and displayed
version; the child disables automatic background updates. The suite pins once
per selected batch. New-version observations remain in a separate compatibility
entry. Raw transcripts/debug logs stay private. Plan files now use a valid path
inside the disposable project; the previous out-of-project plansDirectory was
rejected by Claude and could fall back to its normal plan directory.

The Linux suite exposed a housekeeping bug after its tests and process audit
passed: `ci/test-run.py` ran the Mach-O OSO pruner on Linux against the read-only
checkout. That pruner cannot establish ELF references and is now invoked only
on macOS. The rerun completed without that error. This is separate from process
cleanup, which was clean on both runs.

### 2026-09-09 — full-suite stamp respects the configured build directory

Clean-tree Linux verification after the state-lab rebase exposed a second
post-test runner error: the release success stamp used `target/` inside the
read-only checkout even when Cargo built in `/target`. Earlier dirty-tree runs
skipped stamping and did not exercise that path. `ci/test-run.py` now writes its
stamp under `CARGO_TARGET_DIR` when configured, retaining `target/` by default.
The tests and fixture audit had passed; the failed runner exit was not a passed
gate. Verification must include a clean-tree Linux run that writes the stamp.

### 2026-09-09 — quieter Git and composer chrome, deliberate header navigation

Empty checkout and branch diff titles now say `no changes`; nonempty diffs keep
their file and line counts. The title no longer includes the context-density
word; the existing density controls retain their own labels. Focusing the
board's Git clause changes its style only, leaving its surrounding gaps on the
page ground and preserving the positions of all following header content.
The expanded composer names its destination without the redundant `column`.

Up/`k` travel through tickets stops at the first ticket during repeated input.
A 650 ms quiet gap or another key ends this guard, so a deliberate subsequent
Up still reaches the column header, then the Git section. The quiet-gap fallback
also handles legacy terminals that report held keys as ordinary presses; it
covers the usual initial repeat delay as well as faster repeats. Navigation
and header geometry regressions cover these boundaries, and changed text
goldens were inspected. The TUI selects a steady bar cursor on entry and resume,
and restores the terminal's default cursor shape on exit and handover.

### 2026-09-09 — diff reading controls and release-aware Up navigation

The diff's file-navigation hints now sit beside FILES, or beside the file
heading when the narrow layout shows only the diff. Paging and line-scrolling
hints sit at the right of the hunk heading when its content overflows; the
footer no longer repeats them. Diff pages use the rendered viewport with one
row of overlap, replacing the fixed 20-row jump. They use the ticket preview's
eased page animation, continue from the visible position on another press,
clamp at both ends, and cancel on file changes, refresh, or hiding the pane.

The menu no longer includes Fetch remote, All keys on this screen, or Add a
column. `?` and `O` remain available; the board footer now teaches `O new column`
when a column header is selected.

User testing refuted the 650 ms Up guard above: it swallowed a fresh press after
releasing a held key. Terminals with the negotiated keyboard protocol now
report repeats and releases, so a fresh press can immediately leave the first
ticket while a repeat cannot. Legacy terminals use the time delta between
events with a 120 ms cutoff instead. They cannot distinguish a release/repress
inside that interval from a held key. Regressions cover both input paths,
viewport-sized paging, animation continuity, narrow layouts and hint placement.

### 2026-09-10 — column settings stay on the column header

Removed the Column settings row from the menu. Enter on a column header still
opens its settings, with the existing contextual footer hint. Menu regressions
check that the row is absent from both the menu and Settings.

### 2026-09-10 — settings groups and independent column offers

Settings now opens three groups: Appearance & notifications (theme,
notifications, status line), Behaviour (auto merge, its existing optional
post-merge notice, snooze, week start, default column), and Agents (brief,
tools). Show agent replies is removed from Settings; the board's reply keys
remain. Nested pickers and the brief review return to their own group, and
Esc returns to the parent on its selected row. "Auto merge" replaces "Merge
train" in the settings labels without changing its consent or reach.

Column settings removes Name and Delete; board `r` and the existing deletion
chord still act on the header. Agent behaviour groups mode, tools, starting an
agent on creation, and the working/end-turn transitions. New columns still
have a name field before they exist. The remaining root rows include auto
merge and a single Offer ring: off, sleep, archive, sleep + archive.

The optional `offers` column setting persists that independent choice through
the existing authorized SetColumnSettings command. Absent `offers` preserves
the legacy `reclaim` boolean's off/both meaning; an explicit choice overrides
it. Bulk sleep and bulk archive now use separate column sets, both for their
prices and for execution. Model and store round-trips, daemon restart and bulk
action tests cover compatibility and independence. Goldens cover each settings
group at 60×20 and 120×30, plus the column root and agent submenu.

The daemon test also caught the old sleep count surviving an offer change until
the 10-second RSS refresh. Changing offer eligibility now recomputes that count
immediately using the latest byte measurements, before the snapshot is broadcast.

### 2026-09-10 — adjacent ticket moves require the same key twice

On a board ticket, `>` previews the column immediately to its right and `<`
previews the column immediately to its left. Neither wraps at an edge. The
pending ghost and dimmed original card remain until another input: only the
same key confirms; every other key cancels and is consumed, including Enter,
arrows, digits, help and unbound keys. Bracketed paste cancels too. MOVE no
longer inherits global bindings or offers placement/Enter hints.

A confirmed move inserts at the top of the adjacent column and leaves the
cursor at the source row, selecting the ticket that was below it. If the last
row moved, the previous ticket is selected; an emptied column keeps its cursor.
A refused move leaves the original ticket selected and preserves the refusal.
Immediate HJKL/Alt-direction nudges and `.` retain their existing behavior.
This supersedes the earlier freely positioned, wrapping ghost gesture. Tests
cover cancellation, both directions, edges, source focus and refused moves;
the move and board-help goldens reflect the new gesture.


### 2026-09-10 — providers belong to accepted starts and persisted sessions

The project selects Claude Code (the legacy default) or Codex under Settings >
Agents. Ordinary starts, auto-start and empty-seat prompts capture that choice;
queued and provisioning starts keep the captured choice when the setting or
queued words change. A session's provider remains its own through wake and
resume. Both providers share one live ticket seat, scoped MCP authorization,
column movement, notifications and checkout coordination. Legacy records remain
Claude; columns/session schema bumps prevent an older writer dropping new data.

Provider adapters own launch configuration, native hook/event interpretation,
conversation identity, preview/history formats and recovery metadata. Claude
retains its existing missing-history fresh-start rule. Codex resumes only its
opaque native thread ID and never falls back to a replacement conversation.
Codex column sandbox and approval settings inherit unless explicitly selected;
Claude's permission mode is never translated into Codex privileges.

Codex 0.153.4's native terminal talks through an owned transparent Unix WebSocket
relay to its own app-server. The relay observes the native client's actual
selection/turn/request traffic, including native approval responses; it never
answers an approval or submits a model turn. A second resume subscriber was
refuted by bounded experiments: an empty thread has no persisted rollout yet,
and subscriptions introduce ownership/ordering problems. Native title-generation
system threads are not the selected user thread. Snapshots carry generation,
sequence and heartbeat; only the daemon writer applies normalized evidence.

Missing observation holds automatic checkout operations even when an older
projection said finished. Sleep and termination retain that hold until the
runtime confirms its owned server and descendants stopped. Cleanup follows exact
PID start identities and ancestry, including tools that create another process
group; a lost ancestry proof remains held. A process-owning regression verifies
that an escaped tool is stopped before acknowledgement. Pending settle evidence
is distinguished from consumed sequence so a restart cannot silently discard a
completion before its common state transition applies.

Native Enter during an active Codex turn was measured to steer it; native Tab
queues another turn in the client. Mesimon board prompts therefore wait for a
verified idle input, paste once and send one Enter, retaining the submit latch
until an observed new turn acknowledges it. Focused native interactions keep
their own semantics. Complete plan items can become authorized ticket notes;
plan deltas alone never assert approval or completion. Codex's structured write
gate checks every Add/Delete/Update/Move-to path while leaving native hook trust
and existing sandbox/approval policy intact.

Evidence and unfinished acceptance are tracked in CODEX-IMPLEMENTATION-STATUS.md
and spikes/codex-runtime-evidence.md. These decisions do not certify the still
open full acceptance matrix. The connected fixture isolation incident and its
exact corrective cleanup are recorded there too; no failed capture is a pass.


### 2026-09-10 — native Codex plan dialogs and cleanup reserve safety

Codex 0.153.4 opens its implementation choice locally after a successful plan
turn, without an app-server approval request. A completed plan therefore enters
Plan attention with a checkout hold. Streaming plan text alone remains work.
The native user's next turn clears the hold; declining the dialog clears it only
after the daemon observed that dialog and then the native composer. This yields
Idle/Unknown, never EndTurn or automatic completion movement. The seen/dismissed
turn markers persist through handover. Plans are saved as proposed ticket notes;
the note is not evidence of user approval. Native foreground title notifications
supply board names, with system-thread titles excluded.

A killed or sleeping Codex runtime reserves its seat and blocks worktree teardown
until its stop acknowledgement. Archive's empty explicit-session list cannot
bypass that per-ticket hold. An accepted resume behind worktree provisioning
reserves the same agent seat as an accepted start. Legacy shell hook handling is
preserved independently of Codex's structured event adapter.

Observe-only external sessions cannot accept a board prompt until explicit
takeover gives Mesimon a native pane. Both immediate and queued sends return an
explanation; automatic note/merge delivery uses the same guard, so no request can
be silently parked forever against a nonexistent runtime.


### 2026-09-10 — passive recovery is an adapter responsibility

AgentAdapter now creates opaque per-session AgentRecovery state. The daemon
schedules common startup, pane activity, status and history samples, but the
provider decides eligibility, native interpretation and recovery heuristics.
Claude owns its startup probe, status cache, transcript cursor and thresholds;
Codex supplies no passive fallback and cannot infer quiet from a silent pane or
stored history. Both signal and preview mutations cross the common authorization
chokepoint before application. Existing Claude replay/status/tail regressions
remain intact; the old public tail module is a compatibility re-export.

The native startup composer is checked once per Codex runtime generation (and
again for observed attention), preserving trust/auth attention on wake without
continuously capturing every idle terminal. External attachment uses the same
retiring-seat ownership predicate as ordinary starts, so a stopping record cannot
move to another ticket and release its former checkout's cleanup hold.

Linux verification now retains private audit registries on its named build volume;
the guard validates their configured parent, ownership and permissions before
allocating a fixture. Failed or abnormal cleanup remains a failed audit.

### Codex descendant audits and local plan dismissal (2026-09-10)

A completed child turn or terminal collab summary is not proof that its requests,
tools or hook continuations have stopped. The adapter retains bounded normalized
work even before the parent discovers the child, promotes descendant identities
transitively, and audits known closed children as well as every loaded non-system
thread in the dedicated app-server. Metadata reads never resume/subscribe or answer
an approval. Native outgoing mutations and incoming events invalidate in-flight
audits; reconciliation waits until both relay queues have drained. Missing,
oversized or changing evidence keeps the checkout held.

Changing the native foreground does not discard the previous conversation's
work. A terminal result withheld by such work can become eligible once audited;
an already published completion is never replayed after late activity. Native
plan dismissal is eligible only when independent work has drained: a local plan
UI cannot clear child, tool, old-conversation or pending-native-RPC holds. Pure
ledger and runtime regressions cover these cases; native child acceptance is
still tracked separately in CODEX-IMPLEMENTATION-STATUS.

The connected native Stop check now uses the actual common checkout policy:
Running/attention/unknown states hold work in addition to the independent
observation-hold flag. Real Stop-hook barriers and one automatic continuation
passed without premature completion; a prior fixture requiring that extra flag
at every instant was incorrect and its failed capture remains recorded.

### Explicit recovery after unverified Codex cleanup (2026-09-10)

Abrupt app-server death can reparent an unobserved escaped tool before the
supervisor samples it. Absence of the known processes is therefore not a clean
stop acknowledgement. Such records keep their agent seat and checkout hold;
automatic wake, queue delivery, worktree provisioning and teardown cannot clear
that uncertainty.

An explicit local user's resume may recover the exact conversation after two
separate gestures. The first checks the matching old generation, dead/absent
native pane, no known runtime/app-server/native owner or live endpoint, no
positive external writer, and an existing checkout; it warns that unknown child
processes may remain. The second acknowledges that specific generation. A daemon
restart or reappearing owner invalidates the offer. Old configuration, stopped=false
snapshot and a bounded diagnostic are retained privately as unverified evidence;
a new runtime starts with an observation hold. Failed native launches restore only
the prepared configuration with a generation check, leaving original failure
evidence intact. No automatic action manufactures cleanup success. This extends
the existing explicit resume override policy rather than interpreting missing
telemetry as permission to operate on the checkout.

### Shutdown response delivery is acknowledged (2026-09-10)

Queueing Shutdown's response to a client thread did not ensure it reached the
wire before the daemon process exited. The existing client writer now signals
after newline serialization/write/flush, and shutdown waits at most two seconds
for that receipt. Disconnected or stalled clients cannot prevent shutdown
indefinitely. Board-state mutation stays on the main thread; response writes stay
on the original client thread. A regression exercises eight restart/shutdown
cycles while four snapshot readers are active. Separately, the interrupt e2e now
awaits the one-way asynchronous hook input before asserting its initial Running
state; the later painting and interruption assertions are unchanged.

### Native manual compaction is maintenance (2026-09-10)

Codex 0.153.4's `/compact` creates a separate completed turn containing a
ContextCompaction item and no task output. Completing that maintenance returns
Mesimon to Idle/Unknown and does not trigger on_done or move the ticket. A turn
that also contains user input, assistant output, a plan or task tools retains
normal task completion semantics, including automatic compaction inside it.
Independent pending requests, descendants and hooks continue to hold the checkout.
The native manual-compaction capture `eec3f88a` verifies both the maintenance hold
and a subsequent successful prompt on the same conversation; earlier failures
remain documented in the runtime evidence.

### Precise native requests and API failures (2026-09-10)

Within each Codex thread, classified outstanding requests determine attention;
coarse waitingOnApproval/waitingOnUserInput status flags are a fallback. This
prevents a real MCP form from being mislabeled Permission and preserves Question
and Secret distinctions. Independent parent/child requests still compete under
the common urgency order. Resolving the last request exposes any remaining
coarse flag again; classification does not release a checkout hold.

The installed protocol's systemError thread status is recognized alongside the
authoritative failed turn. It does not erase that outcome as missing observation
or manufacture successful completion. Unknown future status variants still fail
closed. Regressions exercise both status/turn event orders and a subsequent turn.

### Inner-server loss and accepted adoption reservations (2026-09-10)

The Codex executable can be a launcher whose inner app-server exits first. A
surviving launcher is insufficient evidence of intact process ancestry. Relay
failure after launch preserves cleanup uncertainty, and a native successful exit
or Close requires a positively live dedicated upstream listener before normal
shutdown. Known-process cleanup cannot turn that uncertainty into stopped=true.
Native capture `1a9219d7` verifies inner-server loss, the retained checkout hold,
and explicit warned recovery of the exact conversation without another model
turn. The earlier incorrect acknowledgement remains recorded as a failure.

External adoption checks accepted pending agent starts and resumes before
rebinding a record to a ticket. An absent pane during worktree provisioning does
not make that ticket's reserved seat available. Paused-provisioning regressions
verify rejection and eventual launch of the originally captured provider.

### Failed startup requires positive conversation evidence (2026-09-10)

A missing Codex thread ID cannot prove that no conversation was created. Runtime
snapshots now persist BeforeSelection before native launch, SelectionPending
before forwarding a creation/selection request, and the Selected identity before
forwarding its response. Phase changes advance the observation sequence. Legacy
snapshots default to Unknown, never BeforeSelection.

The adapter permits a two-gesture local startup retry only when the matching old
generation positively proves no selection was forwarded and has no resume,
thread, turn or history identity. Known-owner absence and retained uncertain
cleanup evidence are still required. A saved native identity resumes exactly,
including an identity received just before the daemon missed its projection.
Pending or absent evidence refuses a fresh replacement. The same record/provider
and seat survive a valid retry despite a project provider switch. Four supervised
integration cases and durable pre-forward snapshot regressions cover this policy.

Native automatic compaction inside a task also passed (`873d7f68`) at a verified
private 14K threshold: one tool result, auto Pre/PostCompact on the same turn,
checkout hold through compaction, then task completion only after its actual
reply. This confirms the manual-maintenance/task distinction without exhausting
the default context window or changing user configuration.

## Explicit protocol-1 daemon upgrade (2026-09-10, user verification)

The Codex wire-version bump exposed a missing upgrade path: the new client was
refused before it could inspect the old daemon's build, while `U` required an
on-disk binary update and its Shutdown also passed through the rejected Hello.
Repeated reconnects therefore never made progress. A known protocol-1 refusal
now offers `U` even on a freshly started client. Only the human's explicit reload
can negotiate protocol 1, and that temporary connection sends only the unchanged
Local Hello and Shutdown envelopes. It never subscribes, reads a board, sends
ordinary mutations, or becomes the client's retained connection. Normal commands
still require protocol 2; unknown protocol versions remain refused. Existing
same-protocol build-skew ordering is unchanged. No mismatched daemon is upgraded
by opening a board or by the notification observer. The normal handover preserves
tmux sessions and reexecs the selected binary after daemon shutdown.

Rejected handshakes also used to leave a reader thread and socket alive while the
peer kept the connection open. Dropping a connection now shuts down both socket
directions. Socket regressions exercise the literal old refusal, EOF cleanup,
version validation and the explicit 1/2/3 control negotiation; a TUI regression
proves the upgrade is offered without a changed executable and waits for `U`.
The user exercised the rebuilt executable in `simbly`, observed the normal
shutdown wait, and confirmed the board worked after handover. The agent did not
restart the personal daemon or inspect personal conversations.

## Board ticket duplication (T-316, 2026-09-10)

On a selected board card, `y y` duplicates the ticket immediately below its
source in the same column. The first `y` is absent from both footer and help;
only after pressing it does the confirmation hint appear. Any other key cancels
and is consumed. The second press acts on the captured source ID, and the cursor
selects the copy without the composer's Enter-to-start shortcut.

The human-only daemon command copies the exact title, tags, workspace preference,
and all note bodies in their existing order. Notes retain authorship and revision
metadata but receive independent IDs. Ticket identity and creation stamps are
fresh; sessions, worktree bindings, archive/snooze/attention state, and manual-merge
state are not copied. Column auto-run does not fire. All source notes must be
read successfully before reserving a durable ticket key; all copied bodies are
saved before publishing the new ticket metadata and snapshot notification.
Failed writes refuse the operation and clean up only the new ticket directory.
Keymap/TUI regressions cover hint visibility and the confirmation/cancellation
flow; a supervised daemon integration covers ordering, durable content,
independent edits, agent refusal, and suppression of auto-run.

## Half-page brace navigation (T-318, 2026-09-10)

`{` and `}` move up and down by half the measured page in the diff hunk
pane, ticket preview (replies, notes, and shell tails), and release notes.
The distance rounds down with a one-row minimum for a nonzero page; a hidden
diff pane stays still. Physical Page Up/Down retain their full-page distance,
and braces remain ordinary text in the editor. Existing paging hints and
rendered geometry are unchanged.

The diff and preview keep their glide animation, repeated-key targets, and
end clamps. Reaching the bottom of a shell preview still resumes tail following.
Keymap and TUI regressions cover both directions, odd and tiny page heights,
full-page keys, repeated presses, boundaries, hidden panes, and tail following.

## Diff return shortcut (T-321, 2026-09-10)

`Ctrl+]` now leaves the diff viewer through the same Back action as `q` and
Escape, including the legacy terminal encoding `Ctrl+5`. A checkout diff returns
to the board; a ticket branch diff restores the ticket page and its rail row.
The shortcut also works after arming the `z` view chord. Existing hints and
geometry are unchanged. A TUI regression exercises both encodings, both origins,
and the armed chord, and checks that leaving clears the diff state without quitting.

## The board keeps the machine awake (T-288, 2026-09-07, user: "opt in ∙ indication like caffienated on title")

An agent mid-turn on a laptop that idle-sleeps is an agent stopped mid-turn — the pane freezes,
a build dies with the machine, and a long turn comes back to a stale board. Nothing in mesimon
touched power before this: no `caffeinate`, no `IOPMAssertion`, no `systemd-inhibit` anywhere in
the tree. Now a preference (`prefs.json::keep_awake`, OFF and deliberately — changing what a
machine does about power is a thing the user asks for, never a thing an update starts doing) holds
the SYSTEM's idle sleep off while `quiet::is_mid_turn` finds anything on the board, and `☕` in
the header says it is holding. The Settings row sits under BEHAVIOUR, beside the merge train:
appearance is what the board shows and says, this is something the board DOES. The display still sleeps, and so does a closed lid: that is not
idle sleep and no assertion prevents it.

**The BOARD holds it, and that is the whole shape.** `notifier.rs`'s argument, reused: a
daemon-side hold would keep a closed board's machine awake with nothing on screen to say so, and
here the process dying is the off switch. So there is no `Command`, no `Snapshot` field, no schema
and no daemon change of any kind — `tui/src/caffeine.rs`, a preference, a Settings row, a glyph,
and one block in `App::tick`. Close the board and the machine sleeps as it always did.

**Mid-turn is `is_working` minus one state.** `quiet::is_mid_turn` is `is_working` without
`RequiresAction`, written in terms of it so the two can only disagree about the one clause: a turn
stopped on a permission prompt is stopped on a PERSON, not on the machine, and holding a laptop
awake for it buys nothing (the user's own rule — *"running is when an agent is mid turn and not
waiting for user action"*). `App::anything_mid_turn` is the board-wide read, named apart from
`checkout_busy` because two similar names over two different predicates is how they would drift;
the snapshot's `in_flight` rows stand in for the daemon's own pastes, as they do next door. Being
written in terms of `is_working` is what carried it onto Codex with no edit when the native
provider landed — including that agent's unprovable-quiet hold, which errs AWAKE, and that is the
direction to err in here.

**macOS takes the assertion itself; nothing wraps `caffeinate(8)`.** `caffeinate` is a thin
wrapper over `IOPMAssertionCreateWithName`, so wrapping the wrapper would buy a process and lose
the crash safety: an assertion belongs to its owning process and powerd drops it when the task
dies, SIGKILL included. Two `cfg(target_os = "macos")` framework links (`IOKit`,
`CoreFoundation`), no crate added, so `ci/build-linux.sh`'s "nothing in the graph is C, therefore
rust-lld can cross-link" is untouched. Every constant was read off this machine's own
`IOPMLib.h` — `IOPMAssertionID` and `IOPMAssertionLevel` are `uint32_t`, `IOReturn` is
`kern_return_t`, `kIOPMAssertionLevelOn` is 255, success is 0, "no special privileges are
necessary" — and verified against the OS: while held, `pmset -g assertions` prints
`PreventUserIdleSystemSleep named: "mesimon: an agent is working"` against our pid, and nothing
after the release. `PreventUserIdleSystemSleep` and not `PreventSystemSleep` for a second reason
beyond the display: the latter is documented as valid only on AC power, and an agent on an
unplugged laptop is the case that needs this most.

**A spawned holder is held open by a PIPE, and `tail --pid` was refuted.** The Linux rung is
`systemd-inhibit --what=idle --who=mesimon --why=… --mode=block cat`, with `cat` reading a stdin
pipe we own — doing it ourselves means a D-Bus client, since logind's `Inhibit()` hands back a
file descriptor over SCM_RIGHTS, which is a dependency or a protocol implementation. The first
design guarded it with `tail --pid=<our pid>`, `caffeinate -w`'s idea, and it is WRONG here:
`exec` reuses the pid, so on the one edge that runs no `Drop` of ours and still must let go — the
`U` reload — the guard would never fire. Rust's pipes are `O_CLOEXEC`, so the write end closes on
the exec itself, and the same close covers a panic and a SIGKILL. `lib.rs` also drops the keeper
explicitly beside the notifier's, before `reexec`, which waits up to `HANDOVER_MAX` for the daemon
it asked to stop: holding the machine awake through that wait is precisely the bug.

**`drive` polls, it is not only an edge.** `systemd-inhibit` exists on PATH in plenty of places
with no logind to talk to (a container, an ssh session, WSL without systemd) and exits at once. A
board that kept drawing the mark over a dead child would be this feature's one unacceptable
failure — saying the machine is held when it is not — so every tick asks `try_wait` first, drops
the hold, and says so once. A refused acquire is said once too and not retried until the want goes
away and comes back; a machine with NO rung says nothing at all, because the Settings row already
carries that sentence and a status line every time an agent starts a turn is the same news ten
times a day.

**The WSL bridge is built and does not answer.** Inside WSL2 a Linux inhibitor governs the WSL VM,
not the host that decides when to sleep, so that rung would be one that only looks like it works.
`MESIMON_CAFFEINATE=windows` runs `powershell.exe` through interop holding
`SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`, with three independent releases —
EOF on the stdin pipe, an explicit kill with `taskkill.exe` behind it on the pid the holder prints,
and a four-hour cap it enforces on itself — because the failure it must not have is a host held
awake by a process nobody can see. It is opt-in because nobody has watched it work: this repo's own
rule for an unverified rung is `TERM_BUNDLES`', that a confident wrong answer is worse than a blank.
`doctor` says `unverified`. Making it a default rung is one line in `find_from`.

**The mark is `☕`, and it is the product's only emoji.** The author picked it over the one-cell
candidates (`☼` U+263C, `◉` U+25C9, `⏻` U+23FB). It costs TWO cells — `Emoji_Presentation=Yes`,
EAW=Wide — which the row's own `unicode_width` arithmetic already handles; being emoji-BY-DEFAULT
is what makes it safe to spend, where `✔`'s text-default-with-an-emoji-property is what forced
`done_unread`'s fallback. It hangs off the breadcrumb the way `!N` does, so it rides EVERY screen
— the state run beside the ticket count is the board's alone, and a ticket page is a screen
somebody sits on. `dim2`, D33e's register for a board-wide fact: `calm` says there is something
for you to do, and a held machine asks nothing of anybody; `attn` stays needs-you's, which
`test_the_awake_mark_is_never_attn` sweeps for over `Flavor::ALL` (`test_attn_provenance_calm`
cannot — `caffeinated` is false in every app it builds). ASCII is `@`, chosen because `*` is
`suggest_mark`'s and rides this same row.

**Not built, and the limits.** No daemon change, no wire command, no schema bump, no e2e — nothing
crosses a process boundary. A HANDOVER freezes the level: `handover::run` blocks the board's loop
for the whole life of an attached pane, so a hold taken before you attached stands until you come
back. That errs AWAKE, which is the direction the feature exists to protect, and the next tick
re-judges within 100 ms; the notifier went to a thread because it reports EDGES and a missed one is
missed forever, and this is a level. A dead daemon freezes it the same way, and deliberately: the
panes really are still running, and dropping the hold on a two-second reconnect blip is the failure
this exists to prevent.

Pinned by nine tests in `tui/src/caffeine.rs` — the whole ladder through a `find_from` that takes
`macos` as a PARAMETER (`opener::find_from` reads `cfg!` inline, and its test has to branch as a
result; here every rung is exercised on every platform), the pipe guard in the systemd argv, the
Windows script's constancy and its own cap, the edge, the once-said refusal, a holder that dies on
its own, and a real IOKit assertion taken and released —
`only_the_wait_on_a_person_is_not_mid_turn` (`core/src/quiet.rs`),
`anything_mid_turn_counts_the_machine_and_not_the_wait` and
`the_settings_row_keeps_the_machine_awake` (`tui/src/app.rs`, the second asserting nothing crossed
the wire), four header tests in `ui/tests.rs`, `the_awake_mark_is_two_cells_and_outside_the_banned_range`,
and the goldens `board_awake_120x30` and `settings_120x30`.


## T-288 review corrections: attachment, desktop suspend, and subprocess lifetime (2026-09-10)

The earlier handover argument was refuted: freezing the last level does not always err awake.
Attaching to an idle agent and then submitting a prompt left the hold off for the entire turn.
`caffeine_watch::Monitor` now observes the daemon independently of the terminal loop and of
notifications. It consumes complete snapshots, including in-flight pending submissions, through
an observer connection that cannot start or restart the daemon. The board loop reads the actual
hold for its header. Permission waits and completed turns release even while attached. A daemon
outage retains the last observed level, and reconnect refreshes it.

The holder is synchronized separately from network I/O. Disabling the preference or dropping the
board releases immediately, including during a stuck snapshot request; a generation check prevents
an old response from reviving a hold after a preference change. Re-enabling requires a fresh
snapshot. An unexpected observer exit releases the holder too. No daemon or wire changes.

The Linux backend now requests `idle:sleep`, not just `idle`. logind's idle lock does not block
GNOME's own idle timer calling Suspend. The sleep lock covers that path, but can also block
explicit suspend requests according to the desktop's override policy; README and doctor now state
that limit. The prior blanket closed-lid claim applies only to macOS. Sources checked during review:
https://systemd.io/INHIBITOR_LOCKS/ and GNOME's `plugins/power/gsd-power-manager.c`.

The suggested `MESIMON_CAFFEINATE=caffeinate` override was not crash-safe: bare caffeinate ignores
stdin EOF. It now runs `caffeinate -i cat`, including when an absolute path is supplied. Apple's
wrapper watches the pipe reader, which exits when the board dies or execs. Arbitrary custom
programs must release their assertion and children on stdin EOF; this is an explicit contract,
not a property Mesimon can enforce after its own SIGKILL. README, module docs and doctor say so.
The WSL bridge remains opt-in and unverified.

Regression coverage runs the observer on a thread with a fake daemon and no App ticks through
Claude/Codex work, permission waits, completion, and pending-submit transitions. Further tests
cover synchronous disable, fresh re-enable, disconnect/reconnect, and a late response after
monitor drop. A macOS subprocess test closes the guarded caffeinate pipe without calling release
or killing its parent and verifies that it exits. The Linux argv test pins both inhibitor types.

Validation: 14 focused caffeine tests passed on macOS; the full workspace nextest run passed
1333/1333 with a clean audit of 80 fixture owners. Two existing tests were skipped: the live
release-network test and the restart-skew subprocess helper (which its parent tests exercise).
`ci/test-linux.sh -p mesimon-tui --lib` passed the workspace library tests under Debian 12/tmux
3.3a with a clean audit; that script retains `--workspace` alongside the package selector.
Clippy at `-D warnings`, formatting and diff checks passed. The initial sandboxed focused run's
bodies passed but its cleanup audit failed because `ps` was denied; the elevated rerun's audit
was clean, and the earlier registry's two recorded processes were confirmed gone. No live desktop
suspend, WSL host, or interactive TUI verification was performed.


## T-288: fixed-width wake light and header settings shortcut (2026-09-10)

The coffee emoji appeared only during a hold, shifting the rest of the header at each turn
boundary. The user requested one cell with active/idle states, hidden only when the preference
is disabled. The enabled light now uses `●` (held, base text) and `○` (idle, dim3), with `@` and
`o` as ASCII fallbacks. Both states and the focus treatment occupy the same cell. The light
remains in the common header on ticket, diff and editor screens as well as the board.

On the board, Up/k from a column header reaches the repository clause as before; Left/h selects
the light and Right/l returns to the repository. With no git sample, the enabled light is still
reachable directly. Enter opens Behaviour settings at Keep this machine awake, without toggling
it. Enter there disables it. Removing the light moves header focus to git if available, otherwise
back to the board, so no invisible cursor target survives. Down/j and Escape retain the existing
header return behavior. The preference now has an explicit Behaviour mapping in for_verb.

Keymap and app regressions cover both navigation spellings, the unsampled case, the settings
selection, disable/focus cleanup, and the existing checkout diff action. Rendering checks cover
active/idle/disabled states, color and focus contrast, ASCII mode, and unchanged header text
positions at widths 60–160. Goldens show held, idle and focused states. No power-backend behavior
changes, daemon changes, or wire changes.

Validation: the regenerated TUI suite passed 604 tests with the existing live-network test
ignored; all changed/new golden diffs were inspected. The full workspace suite passed
1343/1343 with a clean audit of 81 fixture owners. Its two existing skips are the live release
network test and the restart-skew helper exercised by its parent tests. Clippy at `-D warnings`,
formatting, diff checks and `cargo build` passed in the main checkout. Linux and manual TUI
checks were not repeated for this rendering/navigation change; Unicode and ASCII geometry
and input behavior are covered by the automated tests.


## T-288: wake light beside the board ticket count (2026-09-10)

The user placed the wake light with the board's ticket count, rather than the repository
breadcrumb. It now follows the count (`7 tickets •` / `7 tickets ◦`), keeping the same fixed
width and enabled-only visibility. Other screens, which have no ticket count, retain their
breadcrumb marker. Header navigation follows the new visual order: Right/l from repository
status selects the light, Left/h returns to repository status, and Enter still opens its
setting. A running-ticket count is deferred to a separate task as requested.

Existing geometry, visibility and navigation tests now pin the count adjacency and direction;
the held, idle and focused board goldens reflect the new position.

The user's terminal screenshot showed the geometric hollow circle sitting below the adjacent
text. The Unicode pair now uses the text bullets `•` / `◦`, rather than geometric `●` / `○`,
for a midline text glyph. ASCII remains `@` / `o`; width, colors and navigation are unchanged.

Validation: the relocation passed 1343/1343 workspace tests on rerun with two existing skips
(live release network test and the subprocess helper exercised by its parent tests), and a clean
81-owner audit. The first run also passed every test but reported one leaky test; the diagnostic
rerun reported none. The final text-bullet adjustment passed the 604-test TUI suite, with its
existing live-network test ignored and clean fixture cleanup. All three golden diffs were
inspected; clippy, formatting, diff checks and the main build passed. Linux and manual terminal
checks were not repeated for this placement and glyph-only change.

### 2026-09-10 — Explicit coffee wake status beside the ticket count

The bullet wake indicator was too small and looked like the adjacent memory separator.
Use `☕ on ` / `☕ off` (`@ on ` / `@ off` in ASCII mode), padded to the same display width
so activity cannot move the rest of the header. The setting still controls visibility.
Idle uses dim3, including sel.dim3 on the focused surface; native color emoji may retain
their terminal colors, while the state text carries the dimming. Header navigation and
Enter to open the selected Behaviour setting are unchanged. Rendering tests cover both
states, focus, width stability with memory usage, all themes, and ASCII fallback.

Validation: the TUI suite passed 604 tests with its existing live-network test ignored;
all three deliberate golden diffs were inspected. The workspace suite passed 1343 tests
with two existing skips (the live-network test and a subprocess helper exercised by its
parent), and a clean 81-owner fixture audit. Clippy with warnings denied, formatting,
diff checks and the main build passed. Linux and manual terminal checks were not repeated
for this header-only change.

### 2026-09-10 — Wake status uses coffee and sleep glyphs without text

The author replaced the coffee on/off label with `☕` while held and `💤` while idle,
without state words. Both occupy two display cells, so the memory counter remains stable;
ASCII uses `@` / `z` at one cell each. Keep the idle dim3 style, including while focused,
though native color emoji rendering is terminal-dependent. Disabling still hides the
indicator, and Enter still opens its Behaviour setting.

Validation: 604 TUI tests passed, with the existing live-network test ignored and a clean
fixture audit. All three golden diffs were inspected. Workspace clippy with warnings
denied, formatting, diff checks, and the main build passed. The complete workspace suite,
Linux suite and manual terminal checks were not repeated for this glyph-only follow-up.

### 2026-09-10 — Emoji coffee and a dimmed moon

The terminal displayed bare coffee as small monochrome text and the sleep emoji as large
and colored. Explicitly request emoji presentation for coffee with VS16 (`☕️`). At the
author's suggestion, idle uses a dimmed text crescent `☾`, padded to match the coffee's
two display cells. ASCII stays `@` / `z`. The indicator remains selectable, keeps its
idle dimming when focused, and disappears when the setting is disabled.

Validation: 604 TUI tests passed, with the existing live-network test ignored and a clean
fixture audit. All three golden diffs were inspected; formatting, diff checks and the main
build passed. Full workspace, Linux and manual terminal checks were not repeated for this
glyph-only change.

### 2026-09-10 — T-325: board copies use the local clipboard

Link and agent-brief copy previously only emitted OSC 52, even on local desktops.
A successful terminal write does not mean the terminal accepted the clipboard request;
this caused silent failures and the misleading “copied ∙ if your terminal allows it”
status. Both actions now use `pbcopy` on macOS, `clip.exe` on WSL (UTF-16 with a BOM),
`wl-copy` on Wayland, or `xclip` / `xsel` on X11. Text goes through stdin, never a shell.
A native helper must consume the input and exit successfully before the status says
“link copied” or “brief copied”; errors are reported and helpers have a two-second
limit, including when they stop reading stdin.

SSH sessions deliberately bypass native helpers so they cannot copy to the remote
host's desktop. SSH and machines without a clipboard helper retain OSC 52, with an
explicit “copy requested from terminal” status and manual-selection guidance. This
fallback still cannot guarantee delivery through terminal or outer-tmux restrictions.
Dialogs stay open. No terminal settings or private-tmux containment are changed.
Regression tests cover platform selection, remote exclusion, Unicode, literal stdin,
helper errors, timeouts, and the distinction between confirmed and requested copying.

Validation: five focused clipboard tests passed with a clean fixture audit after the
sandbox blocked the initial runner's `ps` cleanup check; the retained registry listed
no sockets, and its two recorded processes had exited. Workspace nextest ran 1,348 tests
successfully with 81 owners audited clean. It flagged one archive-reclaim test as leaky
(output pipe held past the grace period); that exact test passed on rerun without a leak
and with a clean audit. The two existing skips are the live-network release test and a
subprocess helper exercised by its parent tests. Workspace clippy with warnings denied,
formatting, diff checks, and the main binary build passed. Linux/WSL desktop clipboard
integration and manual TUI checks were not run in this macOS managed pane; platform
selection and payload encoding are covered through pure seams and fake helpers.

## Teams research constraints and proposal (T-215, 2026-09-10)

The author requires a paid Teams tier, managed cloud **and** self-hosted deployment
at launch, and end-to-end encryption of board content with no service-side plaintext
processing. Cross-project sharing must work without the recipient cloning the repo;
scoped MCP ticket submission and opt-in agent questions must coexist with the future
Mesophon phone/browser control surface.

[The research proposal](proposals/T-215-teams-proposal.md),
[security design](proposals/T-215-teams-security.md), and
[176-scenario catalog](proposals/T-215-teams-scenarios.md) record the recommended
architecture and UX. They propose encrypted service authority, verified client key
custody, client-side search/MCP, inert remote intake, reviewed agent execution/replies,
and separate host-control capabilities. These are **proposed, not shipped or approved
implementation decisions**. D30's Git-sharing idea is evaluated, not implemented.

Code inspection confirms that local identity is caller-declared behind the UID
boundary, local ticket creation may auto-run, and current snapshots/models are not
safe remote projections. These facts must not be mistaken for enterprise security
capabilities. Crypto/history composition, browser code-delivery trust, revocation
freshness, recovery, and OS-enforced worker isolation require dedicated spikes and
independent security review before implementation commitments.

This entry records research and explicit product constraints only; no runtime,
protocol, license, or current README promise changes in this ticket.

## Teams approved scope and first implementation evidence (T-215, 2026-09-10)

The owner approved the proposal and then refined the launch contract: terminal
Teams only (phone/browser boards are Mesophon), readable local board/notes,
optional key backup, owner-started ordinary question tickets with Worktree default,
hard no automatic merge, private working drafts, and human-reviewed replies by
default with per-ticket fixed-audience automatic publication as an explicit opt-in.
There is no isolated worker runtime or promise that a worktree contains a hostile
agent. These decisions supersede the earlier T-215 research's browser-at-launch,
mandatory recovery and isolated-worker recommendations. The revised scenario
catalog has 182 rows; the six added rows cover these execution/reply constraints.
The complete [implementation plan](proposals/T-215-teams-implementation.md) replaces
a truncated ticket note.

Implemented generic core safety seam: `Ticket::execution_policy` defaults to existing
local automation, with an `OwnerOnly` floor that is separate from `manual_merge`.
The merge planner excludes it from merge and rebase; daemon merge/rebase boundaries
also consult core execution authorization. Creation auto-run checks the policy.
Copies retain it, the manual-merge toggle cannot clear it, and persistence keeps it
across daemon restart. Ticket schema 5 prevents schema-4 readers from dropping this
restriction. Unknown policy values fail decoding. Local human authority retains the
existing same-UID trust contract; this is not authenticated remote intake or a local
hostile-agent sandbox. Existing local creation defaults remain unchanged.

Paid work starts in the separate `team/` workspace, not the Apache crates. The
[OpenMLS validation report](spikes/T-215-mls-validation.md) records ten executable
scenarios and the [v0 storage contract](../team/docs/crypto-storage-contract-v0.md).
It demonstrates basic encrypted group journeys and exposes two application hazards:
corrupt input can consume receive state in the raw memory provider, and restoring
old valid storage re-enables replay. Test-only transaction rollback repairs the first
scenario; it supplies no production crash safety. MLS membership also permits
sending irrespective of application read-only roles. These findings are review
inputs, not security certification or production adoption. Complete temporary-file reload and a fixed application vector with public test keys
also pass on Linux at the Teams minimum toolchain. Core MSRV stays 1.85;
Teams declares 1.91 and pins its own dependencies and lockfile. A separate manual CI
workflow covers Teams; the root workspace suite does not include it.

Remaining: complete package-2 transactional disk/enrollment/snapshot/known-answer
validation and independent review; the rest of core remote identity/projection and
no-repo capability seams; service, broker/sync, terminal Teams UX, scoped reply MCP,
and enterprise rollout. No Teams service/client or production encryption is exposed
by this change. The owner will create a separate agent for review of this evidence.
The milestone remains in progress; the full cross-project journey has not shipped.


## Teams P2 persistence/enrollment continuation (T-331, 2026-09-10)

Prerequisites checked against the approved T-215 plan and ticket note: P1 scope
reconciliation and the existing ten-test OpenMLS workspace are present. Independent
review, protected platform key custody and a real freshness witness are absent;
they were not assumed. Production encryption integration remains blocked.

The separate test-only Teams workspace now exercises encrypted atomic image
replacement, exclusive ownership, complete provider reload, and 50 injected-error/
process-exit cases covering inbound acceptance/rejection, exact ciphertext outbox,
pending add-member commit/Welcome/tree and accepted membership receipts. Candidate
groups/providers are discarded after failed operations. A post-replacement error
means unknown acceptance; retries inspect the durable receipt and reuse exact
ciphertext. Authenticated invalid application actions consume their message and
record rejection atomically without applying content.

An independently pinned fixture authority endorses the complete device/key/role
roster. Signed snapshot actions bind scope, MLS sender, role, audience, schema and
revision; an independent head digest catches omitted or mixed content. Concurrent
add/removal rejects the stale add and keeps the removed author inactive. Optional
content-key backup recovers a body into fresh MLS state; no-backup plaintext restore
remains supported. These are validation models, not production enrollment or sync.

New evidence also preserves unresolved hazards: old authentic encrypted storage
can still roll back; a stale snapshot with a stale alleged head cannot prove its
own freshness. External working-group fixtures expired in 2024 and fail normal
validation. That rejection is recorded with pinned fixture provenance; it is not
counted as positive cross-implementation interoperation. No lifetime check, crypto
primitive, dependency feature, core MSRV or Apache boundary was changed to hide
these findings.

See the [acceptance/blocker matrix](spikes/T-215-mls-validation.md#t-331-continuation-prerequisites-results-and-open-gates)
and [v0 contract refinement](../team/docs/crypto-storage-contract-v0.md). Platform
custody, rollback/erasure, real enrollment/freshness, unexpired independently generated
vectors and owner-created independent review/remediation remain open. Process-exit
and injected-I/O tests do not prove power-loss durability or constitute review.
Teams-only macOS/Linux-minimum tests and scoped formatting/Clippy checks are the
verification scope; core/tmux and release checks were not rerun. P2 is incomplete;
no production integration, push, deploy or release is part of this change.


### Codex input readiness is independent of the status line (T-339, 2026-09-10)

T-332's queued start reached a native Codex pane, but its title and prompt never
landed; the same failure affected T-339's immediate Shift+Enter start. The observed
Codex 0.154.0 composer showed `mesimon · gpt-6-astra high · Context 0% used · weekly
42% left`. Input readiness recognized the default footer and custom status lines
with a directory, so this valid custom footer left `pending_prefill` and
`pending_submit` waiting indefinitely. Queue scheduling itself had delivered the start.

The first patch recognized that particular context-used footer. The user rejected
that approach: every user can configure a different status line. It is replaced
by the native application's visible text cursor inside the composer, combined
with the existing structured idle observation. No footer content is examined.
The tmux backend captures physical screen rows and cursor metadata in one command
queue; hidden cursors, dead panes and copy mode supply no application input cursor.
Wrapped and multiline composer rows preserve their two-column indentation.
A paste and its later single Enter each recheck readiness. Existing turn
acknowledgement and handover behavior remain unchanged.

Blind early keystroke streaming remains refuted by T-5's measured truncation and
lost-Enter startup race; an Enter can also answer a native dialog. Readiness must
come from the input destination, not the user's decorative status text. The
provider tests exercise immediate and queued starts with arbitrary and absent
status lines, exactly-once delivery, and an unknown dialog with a stale composer
that receives no input until its cursor returns. Backend coverage checks physical
row indexing, hidden cursors and copy mode. Native local-error verification also
supports empty/model-only status configuration and holding a prompt behind the
model-selection dialog, without importing credentials or making paid model calls.

Native capture `9e49ab04` verifies the model-only status configuration, a real
model dialog holding the board prompt without input, and one native turn after
Escape. Its owned HTTP endpoint returns 401; no credentials or successful/paid
model response are involved. Earlier captures `766cab06` and `6d4d1fdb` were
inconclusive (probe Enter timing and dialog-heading casing); `cb302e56` failed
because the probe counted blocked startup CONNECTs as model requests. These
outcomes remain retained, and the corrected assertion counts Responses requests.

### 2026-09-10 — repository build caches and disk-pressure guard

Seven owned Codex runtimes exited around 21:13 local time with `No space left on
device (os error 28)`; the daemon and private tmux server remained alive. Their
worktrees and transcript paths survived, but failed runtime supervision left
cleanup unverified, so resume required the existing human acknowledgment. This
was disk exhaustion, not evidence that seven agents independently quit.

The requested cleanup removed 17 untracked, nonsymlink Cargo `target/` directories
under this repository's ticket-worktree root (52 GiB reported by `du`). No Rust
build/test processes were active at inspection. All 17 targets were subsequently
absent, and free space was 77.7 GiB at cleanup completion. Ticket files, source,
branches, conversations and the main checkout's target directory were retained.

Prevention is scoped to developing this repository, not product-wide automatic
cache deletion. `[profile.dev] incremental = false` also applies to the inherited
test profile; dependency artifacts and line-table backtraces remain. Changed
workspace crates trade incremental rebuild speed for smaller per-checkout caches.
On this machine a parent `.cargo/config.toml` under this repository's managed
`worktrees/` sets `build.incremental = false` for existing branches too, without
editing those branches. `CARGO_INCREMENTAL=1` explicitly opts back in. Concurrent
ticket builds retain independent output directories.

`ci/test-run.py` now requires a 5 GiB free-space reserve before launching its
workload, checks each watched volume every second while it runs and once after
exit, and reports failure through its existing supervised cleanup path when the
reserve is breached. It watches the checkout, fixture/audit and Mesimon state
volumes, plus Cargo's configured output/intermediate directories (including an
explicit `--target-dir`). It deletes no caches. `--min-free-gib N` changes the
reserve; `0` disables it. This sampling guard is not a quota: an external writer
or a sufficiently large burst can still exhaust a disk, and direct Cargo commands
bypass the runner. Older ticket branches get the runner change when they adopt it.

Seven Python regressions cover reserve boundaries, absent output directories,
Cargo configuration and explicit output overrides, refusal before process launch,
and supervised failure/no success stamp when space runs low during execution.
An actual invocation with an intentionally impossible reserve refused before
starting the workload. No real disk-filling test is used.

Verification: seven Python regressions passed, plus bounded `cargo ut --
--test-threads=1` (1,276 passed, the existing real-release-download test ignored;
six fixture owners cleaned). The first sandboxed run failed socket/process access
and cleanup; its five inode-verified fixture directories were removed after an
elevated check proved their processes/listeners absent, retaining the failed audit.
The elevated parallel unit run hit the existing
`native_quit_requires_live_listener_not_a_stale_wrapper_socket` assertion; it
passed in isolation and in the serial unit run. `git diff --check` passed.
`cargo fmt --all -- --check` found existing formatting differences in
`execution_policy_e2e.rs`, core `authorize.rs` and core `train.rs`; they were left
untouched. Full integration/nextest, clippy, Linux and manual TUI gates were not
run for this repository-tooling change; no daemon replacement was needed.

## Idle terminal output does not hold the rebase train (2026-09-10)

T-319 sat in REVIEW with `rebase ask ∙ next` while the board had no working
agents. The daemon correctly observed its Codex session as high-confidence idle
after an end of turn, but `train_pass` separately required five seconds without
tmux `window_activity`. The idle prompt's moving dots refreshed that output
timestamp continuously. The first rebase candidate therefore never advanced,
and the remaining three candidates waited behind it. The pending row did not
represent this extra guard.

Remove the train's terminal-silence check for all providers. Terminal output is
neither agent work nor proof of user typing, so it cannot decide whether an idle
agent may receive a rebase request. The existing planner and board-busy checks
still govern eligibility: a confident end-of-turn idle seat, no working agents
or in-flight requests, an armed board connection, no manual-merge opt-out or
raised hand, and the existing per-tip ask memory and fuse. Prompt delivery keeps
its provider-specific path. This deliberately retires the old five-second
heuristic; it does not introduce a replacement detector for unfinished user input.

The merge-train lifecycle e2e now uses an agent stub that emits output every
100 ms even while idle. It must still receive one rebase request, merge after
rebasing, and stop automation when the arming board disconnects.

Verification: the regression timed out waiting for the rebase request before
the fix, then all three merge-train e2es passed after it. Bounded workspace
nextest passed 1,357 tests with clean fixture cleanup; the two existing skips
are the live-release download and the restart-skew subprocess helper (exercised
by its parent tests). Workspace Clippy with warnings denied, formatting checks
on the changed Rust files, and `git diff --check` passed. No manual TUI or Linux
run; the running daemon still needs the board's `U` handover.

## Refused merges hold later rebase requests (2026-09-11)

The live feed showed T-319's merge refused and T-322 asked to rebase 27 ms
later, then the same sequence from T-322 to T-340. The board snapshot named
uncommitted changes in the main checkout as the merge refusal. `train_pass`
tried merges first, but exhausted both fresh and remembered refusals and fell
through to a rebase ask. Those agents rebased onto the same base; landing one
would invalidate the others' work and require more rebase turns.

Pending merge candidates now hold further rebase requests. The pass still
tries other merge candidates in board order, but if none lands, it waits for
the merge blocker to clear or the candidates to leave the train. Once a merge
lands, the next sample plans against the advanced base. Existing refusal
invalidation on checkout changes supplies the retry; there is no new timer or
permission gate. A newly recorded refusal counts as a visible change so its
notice is broadcast even when the pass sends no prompt. Rebase pending rows
include the merge candidates in `waits_on`, using the existing `after T-N`
display rather than promising `next` while a merge is blocked.

The blocked-checkout e2e now has a ready merge and a second branch needing a
rebase. It checks that neither the initial refusal nor subsequent remembered
refusals sends a rebase request, that the pending row names the merge, and
that clearing the checkout produces exactly merge A → rebase B → merge B.
The new regression failed before the fix with `rebased past a blocked merge`.

Verification: all three merge-train e2es passed, then bounded workspace
nextest passed all 1,357 tests with clean cleanup of 86 fixture owners. The
two existing skips are the live-release network test and the restart-skew
subprocess helper exercised by its parents. Workspace Clippy with warnings
denied, formatting checks on the changed Rust files, and `git diff --check`
passed. Sandbox attempts failed on socket/process access and cleanup auditing;
the exact gates were rerun with scoped elevation, and the retained manifests
were inspected before removing only their verified inactive fixture dirs.
Linux and manual TUI checks were not run for this daemon scheduling change;
the existing display grammar is unchanged. The running daemon was not replaced:
the board's `U` handover is still needed, and conflicting main-checkout edits
remain a separate merge blocker.

## 2026-09-10 — T-320: Option+Arrow word navigation in text fields

Text input already accepted Alt/Ctrl+Left/Right, but ignored the Alt+b/f
(ESC b/f) sequences used by terminal profiles for Option+Left/Right. The TUI
now normalizes those sequences to arrows inside text fields, retaining Alt so
the existing word-boundary movement applies. This covers the composer and
other single-line inputs, editor title/body, tag names, and column names.
Board/picker navigation and Ctrl-letter precedence retain their existing meaning.

Regression tests reproduce the ignored sequence and exercise backward/forward
movement in all five text-entry paths, including UTF-8 words and a multiline
body. The same tests cover Alt/Ctrl arrows; adapter tests preserve plain b/f
text entry and Control precedence. Word boundaries and line-edge behavior are
unchanged.

Validation: the targeted regression failed before the fix and passed afterward.
The bounded workspace nextest run passed all 1,354 tests with a clean fixture
audit; two declared ignores cover the live-network release check and a subprocess
helper exercised by other tests. The first sandboxed attempt failed on socket
and process-inspection permissions; its registered fixtures were inspected and
cleaned. An elevated fail-fast attempt hit `m2_attention_headless`'s hook-state
assertion; it passed in the complete rerun. Workspace Clippy, TUI formatting,
and diff checks passed. Workspace formatting found existing differences in
`execution_policy_e2e.rs`, `authorize.rs`, and `train.rs`; those files were left
untouched. Linux and interactive terminal checks were not run for this TUI input
adapter change.


## Editor word wrapping (T-340, 2026-09-10)

Description and note bodies now soft-wrap at the editor's displayed width,
including the new-ticket composer. Wrapping prefers whitespace boundaries and
splits long tokens only between grapheme clusters. Explicit newlines, indentation,
and all stored text remain unchanged. Up/Down and PageUp/PageDown follow visual
rows with a sticky display column; the composer returns to its title only above
the first visual row. Home/End and word deletion retain their logical-line scope.
The cursor and vertical scrolling use the same byte-range layout as rendering,
which reflows on resize. A cluster wider than the entire viewport uses the existing
truncation marker. Unit tests cover navigation, editing, scrolling, resize, and
Unicode boundaries; editor goldens cover wrapped dialog and full-screen bodies.

## Push/pull commit lists on the checkout Git screen (T-322, 2026-09-10)

The board's `v` screen now offers `Tab` to switch between uncommitted changes and
an upstream comparison. The comparison lists **To push** and **To pull** separately,
with short object IDs and subjects, newest first. It uses the existing reading keys
(`j/k`, `{ }`, Page Up/Down), resets the scroll when switching views, and clamps it
when a newer snapshot shortens the history. Ticket branch diffs retain their existing
scope. The footer and help describe the switch through the core keymap.

The daemon's existing Git worker adds up to two read-only `git log` calls after
status, through the scrubbed Git helper. Each direction is capped at 100 commits and
subjects at 512 characters; exact ahead/behind counts remain visible and the view
names any omitted commits. Nested repositories whose dirty counts are merely summed
are not queried for history. A folder containing one repository inherits that
repository's comparison, matching its existing branch/count behavior.

`RepoGit::to_push` and `to_pull` are additive optional snapshot fields. An older
daemon or a failed history read produces “Commit list unavailable,” while a zero
count says “Nothing pending.” Unsampled, detached, and untracked branches have
explicit empty states. Incoming commits are based on locally fetched refs; the view
shows the existing fetch-age/error note and never fetches, pushes, or pulls on entry.
`R` refreshes the snapshot; ordinary daemon samples continue updating the lists.

Coverage includes a real diverged repository, newest-first ordering, the history cap,
detached HEAD, old snapshot decoding, direction-specific subjects through the daemon
wire, view switching, scrolling and shrinking lists. New goldens cover 60 and 120
columns; the existing checkout-diff golden changes only its footer's Tab hint.
## Remember completed column time (T-319, 2026-09-10)

The ticket page's state row now places `previously IN PROGRESS for 1h 1m`
between the current column's age and creation details. The daemon freezes the
most recent completed column stay strictly longer than 60 seconds on an authorized
column move and persists its column name and duration with the ticket. Shorter
visits and same-column reorders preserve that last qualifying stay; a later long
visit replaces it. This is one completed visit, not accumulated time across visits.
New tickets and duplicates start without history. Old files default to no history;
the first move uses the existing `column_since` fallback to creation when no arrival
stamp exists. Invalid or future timestamps do not create a remembered stay. Archive,
snooze and restore retain the memory without recording a new departure.

Core tests cover the strict boundary and invalid clocks; a daemon integration test
covers moves, reorder, short visits and restart persistence. TOML/JSON round trips
and ticket-page goldens at 120×30 and 100×24 cover storage and presentation.

T-319 rebase verification exposed the existing hook e2e's first-notification race:
a queued SessionStart/resource notification could be read before PermissionRequest
was ingested, leaving the immediate snapshot Running. The test now uses a bounded
state wait after its unprompted-notification assertion, matching the settle check
later in the same test. Permission state, metadata and automove assertions remain.

## The focus token comes back when its board dies (2026-09-11)

`Daemon::focus` was a bare `Option<Focus>`: the board took it with
`FocusStart`/`OpenTerminal` and gave it back with `FocusEnd`/`TerminalEnd`,
which `App::after_handover` sends once the handover RETURNS. A board that
never returns — cmd+W on the terminal window, a crash, a `kill` — sent
nothing, and the token stranded for the life of the daemon: every later
attach, from that board or the next one, answered `another session is
focused`. Dogfooded from a live board whose private tmux status line still
read ` mesimon > mesimon > memory issues > CLAUDE.md ` hours after the window
it named was closed. Restarting the daemon was the only cure, and nothing on
screen said so.

The token is now `FocusHold { what, by }` — a `Weak` on the client's writer,
the merge train's shape (`train.rs`), since the train had already answered the
same question for arming. `on_client_gone` releases a token held by the
connection that just went, and `focus_held()` is the second guard: a holder
whose `Weak` no longer upgrades is no holder, and it is the one road every
reader takes (the refusals, `focus_quiet`, the `Resource` the chokepoint
names, the status line's breadcrumb). Exclusivity is unchanged while the
board lives — that is half of `focus_quiet_e2e`'s new test, the other half
being that dropping the connection is enough.

The stale breadcrumb on the private server is left alone, as a plain
`FocusEnd` leaves it: nobody is attached to read it, and the next attach
rewrites it before its pane is on screen.

## The description is read in one place (T-344, 2026-09-11)

The ticket page held the description twice. `notes[0]` IS the description, so
the cursor on the rail's first note put a truncated copy of it in the header
band and the whole of it in the preview zone directly below — the same words,
one above the other, on the surface with the least room to spare.

The band's excerpt is CONTEXT: it says what the ticket is while the zone
beside it reads something else. So it now stands down for as long as the zone
is reading a NOTE — any note, not just the first — and the zone's heading
carries the role the band used to: `DESCRIPTION` on `notes[0]`, `NOTE` on the
rest, `PREVIEW` unchanged for a session, a shell's pane and the empty seat.
The heading never spells the note's own NAME, which is its body's first line:
that would put the same words in the row under it, which is this ticket again
one surface smaller. Applying it to every note is what keeps the shape of the
page from changing on one particular row as the cursor walks the list.

The rail does not move while that happens. `draw` splits the block's rows in
two: `extra` is the room the description OWNS and is what the rail is placed
under, drawn or not; `shown` is what the band spends this frame. The space
comes back on the LEFT — the zone starts where the block did and reads on for
another six to nine rows — so the row under the cursor stays where the eye
left it, and the price is that the rail's own heading sits below the zone's
while a note is open.

Below `TWO_ZONE_MIN_W` the band keeps the excerpt whatever is selected: there
is no zone there to read a note in, and the one thing the page must not do is
lose the description entirely.

The rail's first note row says `description` rather than its opening words —
the role is the one thing about that row the page said nowhere else, and its
name was a third copy of the same sentence. Nothing daemon-side moved: no
`Command`, no `Snapshot` field, no `Ctx` field, no key, no schema.

Goldens: `ticket_description_selected_120x30` is new; `ticket_note_selected`,
`ticket_description` and `ticket_new_claude` moved. The two L1 sweeps used to
render both markdown surfaces on one screen and now render one each.

## The ticket page's subtitle leads with tags (T-346, 2026-09-11)

The state row under the title carried four kinds of thing in one sentence —
column, ages, tags, and everything about the ticket's branch. Two of them were
in a fight over the width: the chips claimed the row first and the worktree
clause took what was left, so on a tagged ticket the branch name was cut to
`WT_BRANCH_FLOOR` (`⎇ msmn/T-5-graph~`) while sixteen cells of tag sat beside
it. The clause that gave way was the one nobody can guess the rest of.

So the row split. The subtitle is what the ticket IS — its tags, then its
column, its time there and its ages — and the row under it is where its code
lives: the strategy word until a binding exists, then the branch, its merge
state and detail, and what mesimon owes the ticket (`queued ∙ after T-3`,
`auto-merge ∙ off`). Two rows, two questions: what this is against what its
code is doing.

The tags lead the first row because that is what the eye comes to this page
for, and because the page is where you came to read — the card already makes
you decode a pip. They are still budgeted against the width and still give way
to the ages and the column (a row whose tags ate its column says less than one
whose tenth tag was dropped), and the ` ∙` separator still belongs to the
chips, pushed only once one is known to fit — T-163's `created 19m ago ∙ ∙ ⎇
msmn/…` is the regression that rule exists for, and it is the same rule read
from the other end now that the chips open the row: the first chip's own
leading space is the row's left pad.

The second row is drawn only when it has something on it, so a
shared-checkout ticket with nothing owed keeps the four-row band it had. Its
first span opens with that same pad instead of the bullet a clause carries
when it joins a line already in progress — the branch, the bare merge note,
the `⎇ worktree` strategy word, and the owed row when there is no branch for
it to join. `wt_row` is the row's height and every geometry below the band
adds it beside T-344's `extra` and `shown`: it is in neither of those groups,
because unlike the description it is drawn whenever it exists, so both zones
and the rail start under it.

The branch's truncation survived the move and got simpler: it is cut against
its own row now, which at 120 cells means the whole slug fits and `~` appears
where it always should have — when the name really is longer than the screen.
Nothing daemon-side moved: no `Command`, no `Snapshot` field, no `Ctx` field,
no key, no schema.

Goldens: `ticket_tags`, `ticket_tag_chord` (tags first), and
`ticket_merged_upstream`, `ticket_train_manual`, `ticket_queued`,
`ticket_new_claude_worktree` (the second row, and the branch name whole).
`the_state_row_never_shows_an_empty_tag_bullet` keeps the bullet rule and the
chips' new place; `the_workspace_row_keeps_the_branch_name` is its other half.

## The breadcrumb separator is `›` (T-348, 2026-09-11)

`mesimon > kanban-tui > Fix OSC-11 detection` now reads `mesimon › kanban-tui
› Fix OSC-11 detection`. The bare `>` was an operator sitting in a path: it is
the same character the cards spend on an age (`>1y`) and the diff spends on a
hunk, so the one mark that says "a level down" was the least distinctive thing
on the row. `›` U+203A is EAW=N — one cell, like `∙` and the branch arrows,
outside the 0x2500–0x259F range the L1 structure law bans — and it is in every
monospace font a terminal is likely to be running.

It is `glyphs::crumb(tier)`, so the ascii tier (Mono) keeps `>`, and both
breadcrumb sites in `chrome.rs` — the `mesimon ›` root and the leaf a diff or
a note hangs off — ask the same function. The daemon's tmux status line spells
the unicode form directly in its format string: it is theme-blind (06 §2.9)
and has no tier to ask, the same reason its needs-you chip wears graphite's
pair on every flavor.

`golden_board_header_bar_120` found the one real hazard. It located the git
clause with `row.find('⎇')` on a `String` of the row's symbols and used that
**byte** offset as a cell x — correct only while everything to the clause's
left was ASCII, which the breadcrumb no longer is. It scans cells now. A
similar `head.find('◦') > head.find("tickets")` in the header test is safe: it
compares two byte offsets in one string and never converts either to a column.

117 goldens carry the header row, and all of them moved. Nothing else did: no
`Command`, no `Snapshot` field, no `Ctx` field, no key, no schema, and no
width — `›` is one cell, so every hint, budget and truncation lands where it
landed before.

## The diff title says what is out of sync (T-347, 2026-09-11)

T-322 put the push/pull commit lists one `Tab` away from the checkout diff and
left the screen with nothing to say about them. The footer's `tab push / pull
commits` was an *availability*, not a state: it read exactly the same on a
branch level with its upstream and on one eleven commits ahead, so the only
way to learn there was anything there was to press the key and look. The board
header had the answer the whole time — `⎇ main ↑2 ↓1` — and the diff screen,
which is the screen you are on *because* you are asking about git, did not.

The identity row carries it now: `⎇ main ↑2 ↓1 ∙ uncommitted ∙ 2 files ∙ +14
-3`, and the commits view spells the same arrows before its own word. They are
`glyphs::ahead_mark`/`behind_mark` in `theme.calm_text()` — the glyphs doc's
"one home so the two surfaces cannot drift" now has three surfaces on it (the
header, the card's `⎇↑`, this row) and one function behind them. `sync_marks`
is empty on a ticket's branch diff, because a worktree branch is measured
against the base it forked from and `app.git` is the checkout's upstream;
empty before a sample lands, because an unknown must not read as "in sync";
and empty at zero, the same silence `git_clause` keeps.

`tab` moved to `prio: 0` and is drawn at the end of that row instead of in the
footer — the hint sits beside the state that is the reason to press it, the
way `n N file` sits beside FILES. Its words shortened to the OTHER view's
title word (`push / pull` / `uncommitted`) from `push / pull commits` /
`uncommitted changes`: the key and the row then read as one clause, and the
long pair did not fit a 60-column title. A row too tight for it drops it —
`hint_spans`' own arithmetic — and `?` lists it at every width, which is what
`?` is for.

The alternative was leaving `tab` in the footer and putting only the arrows on
the row. That keeps a standing cue below 65 columns and was refused because it
puts the fact and the key at opposite ends of the screen, which is the thing
the ticket was about (author 2026-09-11, asked and answered before any code).

Goldens: `diff_checkout_120x30` (now sampled and diverged, so the shipped
shape is what is minted), `git_commits_120x30`, `git_commits_60x30` — the row
gains the clause and the footer loses the hint in all three. Nothing else
moved: no `Command`, no `Snapshot` field, no `Ctx` field, no key, no schema.

## Board search — the picker behind `/` (T-349, 2026-09-11)

`/` on the board opens a ranked fuzzy picker over every ticket, live cards
first and archived ones under them. Telescope's shape, mesimon's register.

**The matcher is `nucleo-matcher` (MPL-2.0), not ours** — the engine behind
Helix's picker, chosen over writing one because the part that is hard is not
the greedy forward match, it is fzf's v2 scoring and the match *indices* that
make a highlight honest. It adds **no transitive dependency the tree did not
already carry** (`memchr`, `unicode-segmentation`), and `Pattern::parse`
brings fzf's query grammar with it for free: space-separated words are ANDed,
`'foo` is a literal substring, `^foo`/`foo$` anchor, `!foo` excludes. The
licence was the author's call, taken over MIT `fuzzy-matcher` (unmaintained
since 2020, and one new transitive dep) and over a hand-rolled fzf-v1.
MPL-2.0 is file-level: it binds nucleo's own files and reaches neither
mesimon's code nor the `team/` tier.

**The row IS the haystack.** A hit is scored against the exact text the picker
draws — `T-3 Fix auth redirect REVIEW FEATURE archived` — and the indices are
split back over the three fields at the joins. So every highlighted character
is one the reader can see, there is no hidden field that explains why a card
matched, and the column, the tags and the words `archived`/`snoozed` become
filters with no grammar to learn: typing `review feature` narrows to exactly
that. The column is uppercased into the trail so the row says it the way the
board does; smart case keeps `todo` matching `TODO`.

**Archived tickets are a TIER, not a penalty** (user's call): every live card
outranks every archived one, whatever the score says. A penalty makes "where
did that ticket go" a question about weights whose answer is "keep scrolling".
`tab` drops the archived half entirely and the title's `3/47` moves with it,
which is the readout that makes the toggle legible without a chip.

**No index, no debounce, no wire.** The whole board rides the snapshot,
archived tickets included, so ranking is client-side and synchronous.
`how_long_a_big_board_takes` (ignored; `--ignored` runs it) measures a debug
build over 500 tickets: 0.8 ms empty, 1.0 ms one word, 3.1 ms three. That
budget is what pays for re-ranking on every keystroke instead of filtering the
last result — a filter cannot recover a hit the previous keystroke dropped —
and again on every snapshot, so a card an agent archives under an open picker
cannot leave a row that sends Enter somewhere that is no longer there. The
cursor holds its **ULID** across a snapshot and resets to the best match on a
query edit (telescope's rule: typing is narrowing, not scrolling).

**`Scope::Search` is a text barrier** — `j`, `q`, `?` and the tag digits are
all query — with a picker's four keys: `^n`/`^p` and the arrows walk (and
wrap), Enter goes to the row, `tab` toggles the archived half, Esc closes.
`^n`/`^p` because `jk` are text; `Key::Ctrl('p')` joined `directional()`.
Enter puts the **board cursor** on a live card and closes; an archived hit has
no card to land on, so it opens the ticket page, which is the archived
dialog's own Enter reached from here.

`/` is bound on `Scope::Board` and on `Scope::Header` (the board's top row —
the only place a cursor can stand where it would otherwise be inert), and it
is **overlay-only** in both. Twice argued: the footer's left cluster is the
selection's and this key is not about the card under the cursor, and a slot at
120 columns costs another key its place — which would have been `tab
describe`. Trading a hint nobody can guess for the one hint everybody already
guesses is the wrong way round; `c`, `v`, `n`, `p`, `a`, `d`, `z` and `^k` sit
in the overlay on the same argument. It is not a menu row either, by
`menu_omits_fetch_and_actions_with_contextual_keys`.

**Two frames, and the preview is a card.** L1 admits a box glyph only on a
recorded `dialog::frame` perimeter, so the list and the preview are two frames
side by side (telescope draws three) rather than one surface with a rule
through it. The preview renders the real `card::render`, opened, plus the
column and time-in-column line and the ticket's note names — the board's own
vocabulary, nothing invented. Under 94 outer columns the preview goes rather
than being squeezed. The surface height is FIXED: one that followed the list
would move rows under the finger typing at them.

**Highlighting is the value ramp, never a colour.** The one saturated colour
is needs-you's (L2/L3), so matched characters come up to `base` and go bold
while their neighbours sit at `dim1`; on mono and 8-colour the ramp collapses
and the bold carries it alone.
`search_highlights_on_the_value_ramp_and_never_on_the_attn_colour` holds it,
and the picker is swept by both `test_no_banned_sgr` and
`test_no_drawn_structure`.

Trap found on the way in: `App::research` took the mode out with
`std::mem::replace` and returned early when it was not a picker, which closed
whatever dialog was open on the next snapshot (a column-settings dialog,
mid-sort, is how it surfaced). It guards before the take now.

The other one is `truncate`'s `~`. A row cut to its column ends in that
marker, and it is not part of the haystack: lighting it would be a highlight
naming a character the reader cannot see. `painted` counts how many of the
original characters the cut string still carries and stops there. What makes
the rest of the alignment safe is that nucleo indexes CHARACTERS while
`truncate` cuts GRAPHEMES — and a grapheme prefix is a character prefix, so
the indices line up for the whole of what is drawn.
`the_truncation_marker_is_never_lit` is a unit test on `painted` rather than a
render test on purpose: a render test would have to guess the pane geometry
that puts a match exactly at the cut.

Rebased onto T-348 (`›` as the breadcrumb separator, the same day). The two
commits each added a U+203A helper to `glyphs.rs` — `crumb` for a path step,
`prompt_mark` for the picker's prompt — and they stay two functions. They are
two roles that happen to agree today, and each has to be able to move without
dragging the other; every mark in that module is named for its role for the
same reason. The four search goldens carry the header row, so they moved with
the other 117.

Not done, deliberately: note BODIES are not searched. They are files the
snapshot does not carry, so full text needs a wire command, a debounce and a
cancel — docs/07 §10's design — and none of it is needed to find a ticket by
its title, key, column or tags. Goldens: `search_120x30`,
`search_open_120x30`, `search_80x24`, `search_no_matches_120x30`.

## The peek reads the words, not the state machine (T-350, 2026-09-11)

Every idle session's card preview and `p` peek showed the **user's own last
prompt** — `> commit` — instead of the agent's closing reply, for as long as
the session stayed idle. Reported against T-347's session; it was never about
that ticket. The transcript was read, in full, and the reply was in the window:
`latest_preview` walked right past it.

`classify_tail_record` gained a short-circuit on 2026-09-09 (2ce67c8):

```rust
if matches!(turn_edge(v), TurnEdge::Done(_)) { return TailEvent::TurnComplete; }
```

That is correct and load-bearing for the attention machine — `AssistantText`
maps to `Running`, `TurnComplete` to `Idle{EndTurn}`, and a finished turn's
last record must not read as work in flight. But a finished turn's last record
is an `assistant` record with `stop_reason: end_turn`, which is **exactly where
the agent's last words live**. The classifier stopped yielding
`TailEvent::AssistantText` for it, so `history.rs`'s reverse scan fell through
to the `user_prompt` above it and rendered `> <prompt>`.

`census.rs` and `recovery.rs` were migrated to `adopt::assistant_text` in that
same commit — recovery pulls the text out of `TurnComplete`/`ToolInFlight`
explicitly. `history.rs`, the peek and card-preview reader, was the one caller
left on the classifier. It now calls `assistant_text` too, which is what that
function's own doc has always demanded: *display text is independent of
lifecycle; callers must not use preview selection as a state detector.* The
rule is now one-directional and worth keeping: **`classify_tail_record` answers
what state a record puts a session in, and nothing that renders words may ask
it.**

Two branches widen in principle and neither fires in practice. A record holding
text *and* a `tool_use` would now show its words beside the step it took
instead of being skipped as `ToolInFlight`, and an `AskUserQuestion`/
`ExitPlanMode` record would show the sentence that introduced the choice.
Measured over 1,583 assistant records in 40 local transcripts: **0 carry both**
— Claude Code writes text and `tool_use` as separate records (366 text-only,
1,217 tool-only). So `history.rs`'s `ToolInFlight` branch was extracting text
from a shape that does not occur, the spoke mark (`reply_key`, T-173) sees no
new records, and if the shape ever does appear, showing the words beside the
step is the wanted answer anyway.

It shipped because **no fixture in `history.rs` carried a `stop_reason`** —
every `reply()` helper writes a bare content array, so the whole suite tested
only mid-turn records, which still classify as `AssistantText`.
`the_closing_reply_of_a_finished_turn_is_the_preview` writes the real 2.1.26x
shape (closing `end_turn` record → `stop_hook_summary` → latches) and fails
with `Some("> commit")` on the old line. Verified against the live 519-record
T-347 transcript: `Committed as d5d114e, working tree clean.`, no activity, a
reply key.

One file, one call. No `Command`, no `Snapshot` field, no key, no schema, no
golden.

## The train stops asking empty branches to rebase (T-351, 2026-09-11, user: "no need to rebase if no committed, no?")

A worktree session was handed *"Rebase your current branch msmn/T-351-… onto main, resolve any
conflicts, then run the tests and fix any failures before we merge."* Its branch had **zero
commits**. The rebase was a no-op fast-forward, and the rest of the sentence spent a whole agent
turn — a full suite run — to report a green branch with nothing on it.

**The asymmetry.** `train::plan`'s two arms disagreed about the same flag. The merge arm has read
`f.ahead > 0` since the first cut; the rebase arm read only `f.needs_rebase`, which is
`!merged && !ff` — true for any branch the base moved past, work or no work. A freshly cut
worktree sits in exactly that state for as long as main moves under it and the agent has not
committed: `ahead == 0`, `needs_rebase == true`.

`merge_ticket` has refused this same state since dogfood 2026-08-30 — *"no commits on the branch
yet — nothing to merge"*, keyed on `tip == base_oid`, with the note that ancestry would otherwise
call an untouched branch "already merged". The hand `m` road therefore could never reach a rebase
ask on an empty branch; it returns `Refused` before `ff_possible` is consulted. **Only the train
road had the hole**, and only the train road pays for it automatically, on a bucket, without
anybody pressing anything.

**The fix is one clause** — `&& f.ahead > 0` on the rebase arm — because the ask is not free.
It spends the agent's turn, it is recorded in `Train::asked` against the base tip, and it counts
toward the fuse (6 asks / 2 h). Six empty-branch asks would suspend the train for a ticket that
never had anything to rebase.

**Deliberately unchanged.** `merge_state_word` still answers `needs_rebase` for such a branch,
and the card still draws its behind-glyph: the branch *is* behind main, which is true and worth
seeing. What was wrong was spending an agent on it, not saying it. `WorktreeItem.needs_rebase`
feeds the TUI on its own road and was not touched, so no golden moves.

Test `train::tests::a_branch_with_no_commits_is_never_asked_to_rebase` pins both directions: the
empty branch is on neither list, and one commit puts it back on `rebase`.

## The three sentences mesimon writes are the user's to rewrite (T-353, 2026-09-11, user: "allow the user to change agent notify prompts (for rebase / merge / other things) — in settings, think where appropriate to put based on current settings structure")

Three of mesimon's own sentences reach a live agent, and until now all three were `format!`
literals in `daemon/src/server.rs`:

| when | was | now |
|---|---|---|
| the base moved past a branch (`MergeToAgent { Rebase }`, hand `m` and the train) | *"Rebase your current branch … onto …, resolve any conflicts, then run the tests and fix any failures before we merge."* | `AgentPrompt::Rebase` |
| the branch merged (`MergeToAgent { MergedNotice }`) | *"Your branch … has been merged into …. The main checkout now contains this work."* | `AgentPrompt::Merged` |
| a note on the ticket changed (`NoteToAgent`) | *"Note "…" on this ticket was just updated; read_note with id … returns the new text."* | `AgentPrompt::NoteUpdated` |

They are the only text mesimon authors into a conversation besides the agent brief and the MCP
tool definitions — README promise 3's two named exceptions plus these. Which made the binary the
author of the words that start somebody's turn, and the words are opinionated: "run the tests and
fix any failures" is a whole suite run, and T-351 had just finished removing one case where that
sentence was sent for nothing. The user's answer to the next case was not another clause. It was
**give me the sentence**.

**`core/src/prompts.rs`** holds `AgentPrompt` (the three, with `label`, `when`, `default_text`,
`fields`) and `PromptSet` (three `Option<String>`, `None` = mesimon's words). Three decisions
worth keeping:

- **A template is ONE LINE.** `sanitize_prompt` is what the text crosses on the way to a tty and
  it removes every newline, ESC and tab — so a multi-line editor would silently flatten what
  somebody wrote. The field is an `EditBuffer`, the column Name row's shape, not the note editor's
  `TextArea`. The daemon sanitizes on the way IN and stores the result: the bytes on disk are the
  bytes the tty receives.
- **A placeholder is `{name}` from a fixed per-prompt list**, substituted by literal replacement
  and nothing else. `{branch}`/`{base}` on the two merge prompts, `{note}`/`{id}` on the nudge. An
  unknown `{word}` is left exactly as written — guessing at somebody's text would be mesimon
  adding a token again, which is the promise this whole ticket is about.
- **`None` is the default, and the default is in the BINARY.** Clearing a field writes nothing
  (`skip_serializing_if`), so a default that improves in a later build reaches every board that
  never overrode it, and `doctor` can print "default wording" and mean it. Typing mesimon's own
  sentence back in is recognised as the same answer (`filter(|t| t != which.default_text())`) —
  there is no second row for "reset".

**Where it lives.** `Board::prompts`, per repo, `columns.toml` — beside `system_prompt` and
`default_column`, because what to say to an agent about a rebase is a property of the work and not
of the machine. Three SCALARS on `ColumnsFile` (`prompt_rebase`, `prompt_merged`,
`prompt_note_updated`) and not one `[prompts]` table: a TOML table may be followed by no scalar,
and `columns`/`tags` already own that ground. **No schema bump** — a build that drops a custom
template sends mesimon's own sentence, which is what every board sent before the field.

**In Settings.** Last row of **Agents**, under Provider / Agent brief / Agent tools: those three
decide *whether* mesimon says anything to an agent, this decides *what* it says once it does. It
is a door (`Scope::Prompts`, `Mode::Prompts`), the Notifications shape, because `draw_list` sizes
a list at two lines a row and three rows that each become a text field do not fit beside the rest.
The list itself is `draw_dense` — the column dialog's surface, whose `field` parameter grew a lead
string so the row's own name stays in front of the text being edited. The row's label says whose
words stand there; its detail says when it is sent and what it says, because the value of this
setting IS the sentence. Enter opens the field on that sentence with the **cursor at the start** (a
column's name is a word you append to; this is a sentence you read before you change it).

`mcp::agent_allows` denies `SetAgentPrompt` — an agent that could rewrite the rebase ask would be
writing the prompt that starts its own next turn, which is the one thing the tier exists to keep in
the user's hands. `doctor`'s new `agent prompts` record prints all three verbatim, whose they are,
the brief's rule and for the brief's reason: the only words mesimon adds must be answerable without
opening the TUI.

**Refactor that came with it.** `edit_buffer_key` is now a free function in `app.rs` — the raw-key
half of every in-place one-line field (the column Name row, the three prompt rows). Enter and Esc
stay the caller's, because what they save differs and the field does not know.

## `m` says it is merging (T-352, 2026-09-11, user: "it still says 'm to merge' ... which can cause the user to press m again")

The ticket page's second `m` sent `Command::MergeTicket` from inside the keypress, and the client's
loop is `draw → tick`: the frame on the screen through that request was the one drawn *before* it,
which read `merge 2 commit(s) of msmn/T-352-… ? m confirms`. So the whole wait looked like a board
that had not heard the key, and the answer to that is to press it again.

**The wait is real and it is not a bug to remove.** `Daemon::merge_ticket` runs `ff_merge` — a
`git merge --ff-only` in the root checkout, which rewrites the working tree — and then
`refresh_worktree_flags()` **synchronously**, on the writer thread, inside the response. The
flags pass is a `rev-list --left-right --count` per bound worktree plus up to
`CONTENT_SCANS_PER_PASS` patch-id scans: 0.28 s for the 24 branches on this board with nothing
else running, before the merge's own checkout and `persist_and_notify`. Moving it to
`queue_worktree_flags` would shorten the freeze and reintroduce the ticket: the reply would land
on flags that still say `2 to merge ∙ m merge`, which is the sentence the user is complaining
about, only now *after* the merge succeeded. The synchronous refresh is what makes the post-merge
state truthful on the first frame; it stays.

**So the frame is what moves.** `merge_key`'s `MergeStage::Merge` confirm now sets
`App::pending_merge` and the note `merging N commit(s)…` and returns; `lib.rs`'s loop calls
`App::run_pending_merge` in the one place a "working…" note can be seen — **after `terminal.draw`
and before `app.tick`** — and the request holds the loop with its own word on the screen. The
identity line needed nothing: a non-empty `merge_note` already replaces the whole branch-state
clause, so the row reads `⎇ msmn/T-352-… ∙ merging 2 commit(s)…` and the `m` offer is not on it.
The other two stages (`Rebase`, `Notify`) are a tmux paste and keep sending from the keypress.

**The second half is the harm the words caused.** Keys typed through the freeze are buffered by
the terminal and delivered afterwards, and `Merged` arms `MergeStage::Notify` deliberately — the
note promises `m tells the agent` and one press must deliver it. A user holding `m` down through
a two-second merge therefore pasted the merged notice into the agent and started a turn they
never asked for. `lib.rs::drop_typeahead` empties crossterm's queue after the request: a press
aimed at a frame that is already gone is not an answer to the frame that replaced it. Focus
reports are not typeahead and are passed to `App::saw_focus`; a dropped `Resize` costs nothing,
since `Terminal::draw` re-measures every frame and `tick` ignores the event anyway.

Tests: `ui::tests::a_confirmed_merge_says_it_is_merging` renders both frames (the confirm still
asks, the next one says `merging` and no longer names the key), and
`app::tests::merged_note_arms_notify_so_one_m_delivers` now asserts the confirm sends nothing
until `run_pending_merge` is called.
## The train's gate stops being the whole board (T-351, 2026-09-11, user: "if another worktree is still working … no need to wait for it before auto-merging other worktree")

`train_pass` held on `board_busy()` — `working(None)`, every mid-turn agent anywhere. One
grinding worktree therefore stopped every merge and every rebase ask on the board, and a busy
board merged nothing at all. The gate is now `train_busy`, and `board_busy` is gone: it had no
other caller.

**What an ff-merge can actually disturb.** Exactly one working tree, and only sometimes.
`worktree::ff_merge` runs `git merge --ff-only` in the ROOT checkout when the base is what is
checked out there; otherwise it is `git push . <branch>:refs/heads/<base>`, a ref update that
opens no file. Either way a worktree agent mid-turn is untouched by another ticket's merge —
different directory, different branch — so waiting for it bought nothing and cost every merge on
a busy board. `train_busy` holds for the root checkout's own workers (`checkout_holders` on
`paths.repo_root`, which is verbatim what `resolve_spawn_cwd` hands a `SharedCheckout` spawn).

**We hold for the root's workers whatever it has checked out.** The narrower question — is the
base actually HEAD there — is answerable from `git_cache.branch`, and was declined: it buys only
the case where somebody has the root on a non-base branch AND a shared-checkout agent working in
it, and being over-cautious there is free. The gate must never fork git (T-289's whole point),
and one rule that is always safe beats two that need a cache to be fresh.

**The one board-wide wait that survives** is a ticket MID-REBASE at this base tip: asked by us
(`Train::asked` at `base_tip`), its `needs_rebase` flag still true, its turn still running.
Advancing the base under it lands its rebase on a stale one and earns it a fresh ask at the new
tip — and six of those in two hours suspend the train for that ticket. The flag is what makes
this self-clearing: the moment the agent's rebase lands, `refresh_worktree_flags` (which runs
immediately before the pass) drops it and the hold goes with it.

**Concurrency is unchanged, and was never possible.** `train_pass` `return`s on the first
`Merged`, and the daemon is single-writer, so it does ONE thing per bucket (`RSS_TICKS`, ~10 s).
ff-only serialises the rest for free: when A lands, main moves past B, B's `ff` flag flips and it
leaves `plan.merge` for `plan.rebase`. Two independent worktrees can never merge back to back —
the second must rebase first, and mesimon asks rather than rebases. What T-351 changed is whether
the FIRST merge happens at all, not how many happen at once.

`pending_items` reads the same `train_busy`, so a card's `waits_on` names exactly what is holding
it and the row cannot disagree with the pass. E2e
`merge_train_e2e::a_grinding_worktree_does_not_hold_another_tickets_merge` pins it: A in REVIEW
merges while B grinds mid-turn in its own worktree, B's state untouched, and A's owed row claims
no wait. It fails on the old gate.

## The picker opens on the pages you were just in (T-355, 2026-09-12, user: "show previously entered tickets when searching (/) when there's no search query yet — subtitle 'viewed recently' or something similar")

`/` with nothing typed now lists the tickets whose PAGE was opened this run,
newest first, one copy of each, at most ten — under the subtitle `viewed
recently ∙ type to search the whole board` in the breathing row between the
prompt and the list. The first keystroke is the whole board again (T-349's
list, unchanged) and deleting back to nothing is the recent list again. With
no page opened yet, `/` opens on the board exactly as before, so the goldens
that existed did not move.

**Recorded after the keypress, not at the openers.** `App::handle_key` wraps
the dispatch and reads `ticket_page()` afterwards: a ticket the reader is now
looking at is a ticket they entered, whichever of the eight `Screen::Ticket`
assignments got them there (Enter on a card, the archived dialog, the picker,
a link, a focus refusal). Hooking each opener is the same list maintained by
hand.

**The rank pass still decides what is in.** The recent ids are a REORDERING
of `Searcher::rank`'s empty-query result, never a source of rows: a ticket
deleted since is gone, and one archived since disappears when `tab` hides the
archive, the way every archived row does. `Search::recent` is true exactly
when the narrowing applied, and it is the only thing the subtitle reads.

**In memory, like `prompt_history`.** The TUI owns no per-repo file — the
four state files are the daemon's and `prefs.json` is per machine — so the
list lives for the run. A per-repo TUI file under the state dir would be
within promise 1 and is the obvious next step if a restart losing the list
turns out to matter; a wire command to have the daemon remember views was
judged too much machinery for a convenience.
## Teams coordinated review and bounded P3/P4 seams (T-215, 2026-09-10)

The owner authorized subagent coordination. [Assignments, independent findings,
remediation and remaining gates](spikes/T-215-coordinator-review.md) are recorded;
passing a subagent's bounded task does not complete its milestone package.

Core now has a generic content-only capability floor, a strict title/selected-note
projection and owner-authorized inert import preparation. Historical/working notes
are absent unless explicitly selected. No full Ticket, session, local path,
provider/executable policy or remote claim of authority is accepted as content.
The writer supplies/reserves local identities and persists the prepared result.
No remote command or authenticated intake adapter is exposed. Source capabilities
are not yet connected to a no-repo terminal client.

Import provenance is durable opaque correlation, not a verified identity. Its
presence independently forces effective OwnerOnly at existing autorun/train and
merge/rebase gates, even if the stored execution-policy field was omitted. Copy
and restart preserve it and selected note bodies. Schema 6 refuses schema-5 readers
that could drop this restriction. Worktree remains the preparation default;
same-UID local trust is unchanged. TUI edits only initialize the new optional field.

The paid workspace adds a pure ciphertext service domain: current tenant/board/
device-grant checks precede receipt lookup and transitions; operation IDs bind
exact request bytes; current heads guard writes; revocation/read removal freezes
writes until rotation; bounded history/events retain opaque content and metadata.
Fourteen tests include read-downgrade/restore-before-rotation and recovery grant
reset. There is no authentication implementation, durable database, listener,
enterprise audit trail or deployment conformance yet. The actor constructor is
explicitly an adapter assertion, never evidence that request fields authenticate.

Crypto review repaired incomplete backup artifacts and panic-prone authenticated
semantic rejection. A fresh process now recovers only from the authenticated
envelope plus separate key; typed rejections and MLS consumption survive the
additional 50 failure/crash cases. Live pinned mls-rs/OpenMLS exchange supplies
positive protocol evidence for both creator directions, commits, messages, exporter
agreement, removal and stale epochs. Expired historical fixtures still reject.
Both implementations use RustCrypto; no claim of independent primitives or fixed
independent known-answer vectors is made.

The review also prevents an overclaim: roster comparison after helper merge is not
authorization before membership acceptance or Welcome release. That transactional
composition, enrollment/authority rotation, protected custody and independent
freshness remain unfinished. A removed mls-rs group can still encrypt old-state
bytes; current recipients reject them, and broker lifecycle must separately stop
group use. Production encryption integration stays gated. See the [final measured
checks](spikes/T-215-mls-validation.md#coordinated-review-follow-up).


## T-215 — runnable encrypted question/reply preview

The earlier statement that Teams has no running client/service is superseded by
`team/client` and `team/service-server`, separate unpublished paid-workspace crates.
The first complete local workflow now connects real PostgreSQL ciphertext/receipts,
pinned two-device MLS enrollment, no-checkout MCP questions, owner-bound daemon
intake, explicit worktree start, private drafts and exact-audience reply approval.
See [the executable evidence and limits](spikes/T-215-e2e-preview.md) and
[the testing guide](../team/client/README.md).

Core's generic `ImportTicket` remains local-owner authorized and is refused through
the untrusted MCP shim. Its single writer stages complete selected content beneath
`.mesimon/board/imports/` before atomic ticket materialization, recovers before board
load and retains exact retry receipts after deletion. Imports remain Worktree and
OwnerOnly, preserving their independent automatic-merge exclusion and private edits.
No network worker mutates the board or injects remote content into a conversation.

The preview accepts only contiguous verified history and author chains; unanswered
checkpoint disagreement stops sync. Reply retries use fully observed revisions,
so cancellation cannot be skipped by adopting a newer unread head. Unaccepted
cancelled replies remain private rejected drafts. Native custody selects encrypted
profile images, with no file-key fallback; same-UID Unix transport and fixed
two-member enrollment are explicit preview limits. Separate broker, terminal Teams
UX, broader membership/freshness/recovery, both enterprise deployments and security
review remain launch gates. The passing fixture uses test custody and a fixture
agent, not native-keychain or production-provider approval.

## T-215 — the Teams relay runs in a container, over TLS

The preview's "same-UID Unix transport" limit recorded in the block above is
superseded for the service: `team/Dockerfile` and `team/compose.yaml` run the
relay and its PostgreSQL as containers, and that image is what a self-hosted
installation deploys. The Unix socket remains, unchanged, for the local
same-user preview and for the vertical-slice test. `serve` takes `--socket` or
`--listen` with `--tls-cert`/`--tls-key`, never both.

**Peer UID cannot survive a container boundary, and did not need to.** The Unix
listener authenticates its caller with `getpeereid`/`SO_PEERCRED`; a client of a
containerized relay is not a same-host, same-user process. Over TCP the device
bearer credential is the authenticated principal, which is what `service-server`
already documented — the socket was never the device identity — and TLS protects
it in transit. Nothing about authorization changed; one transport simply stopped
having a local peer to inspect.

**The client pins the server certificate's SHA-256 rather than trusting a CA.**
A self-hosted install then needs no PKI, and the trust step is one the product
already asks of people: confirm a fingerprint out of band, exactly as device
fingerprints are confirmed. Pinning replaces name and chain validation only —
`PinnedCertificate` still delegates `verify_tls12_signature`/`verify_tls13_signature`
to the provider, so whoever answers must hold the pinned key, and the fingerprint
comparison is constant-time. `tls_admits_only_the_pinned_certificate` proves a
different certificate for the same name is refused.

**PostgreSQL is reached over a Unix socket shared through a volume, not TCP.**
`Server::connect` admits only a socket or a loopback host because it connects
with `NoTls`. A compose bridge would have put unencrypted database traffic on a
virtual network and made that check a formality, so the deployment was shaped to
keep the guarantee literally true instead of widening the check. PostgreSQL
publishes no host port.

Two container facts cost a debugging cycle each and are encoded in the files. A
fresh named volume inherits the ownership and mode of the image directory it
covers, so `/var/lib/relay/{tls,private,out}` are created and chowned before the
mount or the relay user cannot write them. And `provision-device --credential-out`
cannot target a host bind mount, because the relay refuses a credential path whose
parent it does not own at 0700 and a bind mount carries the host's ownership;
provisioning output lands in the `relayout` volume and is copied out deliberately.
The generated certificate lives in a volume because regenerating one invalidates
every pin a client already confirmed.

`Endpoint` is untagged, so a profile written before this change still loads: a
bare path deserializes as `Unix`. `tests/tls_relay.rs` is an `#[ignore]`d
deployment conformance suite that runs against any serving relay — compose,
self-hosted or managed — so one set of assertions covers every deployment model.
Verified against the running stack: TLS 1.3, the handshake certificate's SHA-256
equal to the printed pin, a provisioned credential reading its board, an
unprovisioned one answered `Denied` rather than dropped, and a wrong pin failing
the handshake before any credential is sent.

Connections are served one at a time, because the server owns a single writer.
A slow client delays others for up to its 5 s timeout: bounded, but not
production admission control. OIDC/PKCE, SCIM, entitlements, certificate rotation
and independent cryptographic review remain launch gates, and a local compose
stack is not evidence of a released Internet deployment.

## T-215 — Teams v1: the daemon is the client, MLS is out, the relay is one crate

The 2026-09-12 review of the ticket (note "Teams v1: TUI plan and scope
relaxation") found that the branch shipped an encrypted question-and-reply pipe
between two CLI binaries — title, question, reply and cancel; one owner plus one
teammate, hard-capped; no removal, rekey, second device or key backup; about 19
hand-typed commands per question — and none of it reachable from the TUI. The
owner accepted five relaxations and this block records the first package
(T-332) that implements them. The blocks above describe the preview they replace.

**A board key replaces MLS.** `crates/mesimon-team/src/crypto.rs`: one random
256-bit key per board, wrapped to each member's X25519 key with an ephemeral
ECDH and signed by the wrapper's Ed25519 key (`wrap`/`unwrap`), records sealed
with XChaCha20-Poly1305 under the key (`seal`/`open`) with the board, object and
revision as associated data and the author's signature over the result. A new
epoch is a new key; removal mints one and re-wraps it to whoever remains. Forward
secrecy is gone on purpose — a board is a shared document and a joiner must read
what is already there — and with it OpenMLS, Welcome and ratchet-tree handling,
the Rust 1.91 workspace split, `team/Cargo.toml`, `ci/test-teams.py`, the 2,200-line
`crypto-validation` crate, the mls-rs interop and the expired working-group vectors.
`a_removed_member_cannot_read_the_next_epoch` and
`a_wrapped_key_opens_for_its_recipient_and_no_one_else` are the two properties
that matter; a relay relabelling the sender of a wrapped key is refused there.

**An invite code replaces the fingerprint ceremony.** `invite.rs`: 20 bytes in
Crockford base32, `XXXX-XXXX-…` eight groups. Twelve bytes are a one-time secret
the relay knows by hash; eight commit to the owner's device id. The code already
travels out of band, which is the channel a fingerprint ceremony would have needed,
so the ceremony is free: the joiner checks the owner's key against the hint
(`names_owner`), the owner checks the joiner's key against an HMAC under the secret
(`proof`). Parsing accepts case, spaces and the Crockford look-alikes.

**The relay is one crate with current-state rows.** `team/relay` replaces
`service-domain` + `service-server`: `objects` holds the current sealed record per
object and `journal` the history, so a read is a query, not a replay of every
operation since the board was born, and the 4,096-operation cap is gone. Every
request is one transaction with the board row locked `FOR UPDATE`. The only
transport is TLS; the Unix socket and its peer-UID checks went, and with them the
second and third hand-written line framers — `wire::read_frame`/`write_frame`
are the one. The client trusts either a pinned certificate (self-hosted) or the
Mozilla roots (managed). Provisioning subcommands are gone: devices `register`
over the wire and boards come from `create_board`, so no operator ever types a
32-hex id. Authorization is `policy.rs`, pure and in `cargo ut`: outsiders and
former members get `not_found`, short roles get `denied`. `tests/postgres.rs`
runs the whole life of a board against real PostgreSQL — register, create,
invite, join with proof, keys, write, exact retry, changed retry, conflict,
viewer, revoke, freeze, rotation coverage, stale epoch, leave, unshare — and then
dumps every row of every table and asserts no typed word is in any of them; a
second test proves TLS admits only the pinned certificate and refuses a bad
credential with an answer rather than a dropped connection.

**Layout follows the licence line.** The daemon is the Teams client, so keys,
sealing, invite codes, the wire types and the TLS client are the Apache crate
`crates/mesimon-team`; only the relay stays under `team/`. The relay is a root
workspace member so `--workspace` commands and the release clippy cover it, and
not a default member so `cargo build`, `cargo run` and the release build skip it.
Its manifest carries `workspace = "../.."` because the old `team/Cargo.toml`
still exists until it is deleted, and cargo would otherwise let that stale
workspace capture the crate.

Not yet: the daemon does not speak any of this (T-333), nothing is on the
snapshot or the keymap (T-334 to T-336), and the display name is the one piece
of plaintext the relay holds, by design — the owner has to see who redeemed an
invite before wrapping them a key.

## T-215 — the daemon is the Teams client (T-333)

Second package of the v1 plan. Nothing here is on the keymap yet; the eight
team commands are answered `Ok` at once and their outcome is read from
`Snapshot.team`, which is what the T-334 to T-336 screens will draw. The
proof is `team/relay/tests/board_e2e.rs`: two real `mesimon daemon` processes
with separate HOMEs and a real TLS relay on real PostgreSQL. Amit signs in,
shares a repo board with one ticket and a note, mints an invite; Dana signs in
from a scratch directory, joins, opens the joined board, sees the ticket, the
note and the owner's columns, creates a ticket, renames Amit's and writes a
note; Amit sees all three, archives Dana's ticket and Dana sees it go; Amit
removes Dana, writes again under the rotated key, and Dana's board reads
`gone`. Every table is then dumped and no typed word is in any of them. Five
seconds end to end.

**Outgoing changes are found by diffing, not by instrumenting.**
`Daemon::broadcast()` ends by calling `team_after_broadcast`, which projects
the board (`team/project.rs`: one `SharedObject` per ticket, the column list
and the board's name when this daemon owns it) and queues every object whose
digest differs from what was last published. Notes are files, so only a note
whose `NoteMeta.rev` moved is read. A ticket that vanished is a tombstone,
once. That is one hook point for every mutation path there is or will be —
local, agent, automation, and the remote applies themselves, which publish a
record's digest *before* they touch the board so the broadcast they trigger
has nothing to send back.

**Every decision is on the writer.** `team/sync.rs` is a job executor with a
`RelayClient`, a credential and no opinions; `server/teamglue.rs` is a child
module of `server.rs` (so it reaches the daemon's private fields without
widening them) and holds every rule: what to send on the tick, what each
result means, how a record becomes a ticket. Results arrive as
`Msg::Team(Done)` like every other off-thread outcome.

**`Principal::Remote { member }` is the fourth principal**, and the first that
is a person other than `Local`. `is_human()` is true, so the move gate and
the flap fuse stand aside as they do for a keypress; `authorize()` denies it
sessions and the board's shape and allows tickets and notes, like an agent;
`authorize_execution` denies it always. It is minted only by `team_apply`
from a record whose signature verified against the member list, and `handle`
refuses it from the socket the way it refuses `Automation`. A ticket a
teammate created lands `OwnerOnly` with no workspace: data until the owner
starts it, whatever the column's `auto_run` says. Column changes go through
`place_ticket`, so the DONE gate holds against a remote move too; when it
refuses, the next diff sends the local truth back.

**A joined board is an ordinary root with no checkout.** `JoinBoard` creates
`~/.local/state/mesimon/team/boards/<board16>/`, writes that root's
`team.json` with `content_only: true`, and `mesimon open <dir>` runs the TUI
on it; `Paths::for_repo` and everything downstream work unchanged.
`spawn_session` refuses on such a root, and `BoardCapability` in core is
what the keymap will gate on. The owner's `Columns` object is applied there
(missing columns added, order theirs); a joined board never publishes it.

**Identity is per user, boards are per root.** `device.toml` (0600, one seed
that derives both keys, the relay address, the credential) lives under
`~/.local/state/mesimon/team/` beside `notifications/`; `team.json` (keys per
epoch, cursor, published digests, outbox, invite secrets, member cache) is
in each root's state dir. Signing in again to the same relay keeps the keys,
so the boards this device belongs to stay reachable; a different relay is a
new identity.

**Two things the e2e taught.** A join is a conversation, not a heartbeat: the
30 s member cadence stalled it on both sides, so a joiner without a key and
an owner with an invite out both poll members at the pull cadence. And a
rotation must name exactly the members the relay still counts, never a
cache: the first rotation after a revoke wrapped a key for the member just
removed and was refused, so `Revoke` now sets `rotate_after_members` and the
owner also heals any pending rotation the relay reports on `Head`.

Known v1 limits, on purpose: short keys are local (Dana's `T-3` is Amit's
`T-12` for the same ticket — the ids are shared, the numbers are not); tags
are not shared; a conflict is last writer wins by retry (a stale put pulls,
applies theirs, and sends the local body again); one request per relay
connection.

## T-215 — sign in, share, invite, revoke, from the TUI (T-334)

Third package of the v1 plan, and the first the person can reach: the eight
team commands T-333 answered `Ok` to now have keys. Nothing here is a new
mechanism — a settings list with two fields in place (the prompt list's
shape, T-353), a list dialog over the board (the archived list's), and the
menu row that is their door.

**Identity is a Settings row, not a dialog of its own.** `Settings › Team`
(`Scope::Team`, a fourth root row beside the three groups) is three or four
rows: `Relay`, `Display name` — Enter opens the draft as a field, Enter keeps
it, nothing is sent — and `Sign in`, which sends both words and says on its
own row what it needs (`needs the relay and a display name above`) or what
failed (`signing in: denied ∙ enter tries again`). Signed in, that row is
`Signed in as Dana on relay.example ∙ enter signs out`, and `Sign in again`
stands beside it only while the drafts differ from the identity — a new name
on the same relay keeps the key; a new relay is a new identity (T-333's
rule, now on the row). The drafts are seeded once from the snapshot's
device and survive a sign-out, so the next sign-in starts from the last
words. `TeamDevice.registered` is what "signed in" means: an identity minted
here that the relay never admitted (the first sign-in against a stale relay
container answered `invalid request`) stays a `Sign in` row with the failure
under it, rather than a `Signed in as` that every later command refuses.
A relay address and its pin arrive by paste, and `App::on_paste`
had no arm for a list row that is a field: it does now, for the team list,
the prompt list and the column name alike.

**The sharing row leads wherever sharing can go next.** The menu's `Share
this board` row is one row in three states: signed out its detail says
`needs a relay identity ∙ enter opens Settings › Team` and Enter opens the
team list (Esc returns to the row); signed in it opens the dialog; shared
it reads `Shared with 3 members ∙ synced` and opens the same dialog. A
joined board — shared, not owned — has no row: its screen is T-335's.

**The dialog's rows are the members, so the mode builds them.** `MenuItem`
is a static list with `fn(&Ctx)` words; a member list is not static. So
`App::share_rows` builds `ShareRow`s off the snapshot — `Publish` and
`Notes` while the board is only here; `Invite a contributor`, `Invite a
viewer`, the last `Invite code`, one row a member, `Stop sharing` once it is
shared — and `App::share_words` gives each its label, detail and the word
Enter's hint wears there. That word rides `Ctx::share_enter_word`, and the
Share scope's Enter is gated on it: a row that is only read (your own, the
owner's, a removed member's) has no word, so Enter is unhinted and inert
there in one predicate, the keymap's rule. Removing a member and stopping
the share arm on one press (`Remove Dana?` / `Stop sharing?`), disarm on
any motion, and act on the second — the column dialog's delete, not a
modal. The frame's title carries the sync word and the drafts waiting
(`SHARING ∙ OFFLINE ∙ 2 DRAFTS`). Members wear their state as a last
clause: `you`, `waiting for a key`, `unverified`, `removed`, `left`.

**Notes are a choice at publish time.** `ShareBoard { notes }` (serde
default `true`, so T-333's callers are unchanged) records `notes_withheld`
on the board's `team.json`; `project::ticket_body` then ships every ticket
with an empty note list and `team_after_broadcast` never reads a note body.
The dialog's `Notes: included ∙ enter keeps them on this machine` is the
switch, and the snapshot's `TeamBoard.notes_withheld` says what was chosen.
There is no switch after publishing: turning notes on later would mean
sealing every body at once and turning them off would mean tombstoning
what members already have — both are a decision for when someone asks.

Goldens: `team`, `team_editing`, `share`, `share_members`; `menu` and
`settings` reminted for the new row. Not yet: the team boards list, join,
the remote board screen and its footer (T-335); ask my agent (T-336).

## T-215 — team boards, join, and the remote board screen (T-335)

Fourth package of the v1 plan, and the joiner's side of T-334: a teammate
with a code gets onto a board and works on it from the TUI. Nothing new
crosses the relay; every piece here is a reading of what T-333 already
syncs, plus one map on the snapshot.

**The menu's `Team boards` row is the door.** Signed out it leads to
`Settings › Team`, the sharing row's rule. Signed in it opens
`Scope::TeamBoards`: a `Join a board with a code` row that is a text field
in place (the team list's shape, pastes included), then one row a board
the relay lists — this one (`open now`), one with a copy here (`enter opens
it in place of this board`), one joined or owned elsewhere (no word, inert
Enter). The join's answer is read off the snapshot: the row says
`Joining…` while the daemon's `busy` says so, an error starting `joining`
stays under the row, and the board the relay lists next with a root on
this machine is the one just joined and is opened. `mesimon join CODE`
does the same from the shell and prints the root.

**Opening a board is `exec mesimon open <root>`.** A board is one process
per root and a joined root is a different daemon, so the TUI leaves
(`App::pending_switch`, `lib.rs::switch_board`) the way `U` leaves for a
new binary — without asking this daemon to stop, because it was not the
one being replaced. The joined board is then the ordinary board screen.

**Two board-wide rules, in one predicate.** `Binding::live` (and
`MenuItem::live`) is now the only reader of a binding's `avail`, and it adds
what a joined board carries: on a **content-only** board (`TeamBoard.
repository == false`, `Ctx::content_only`) every binding in
`Group::Sessions` and `Group::Worktree` and every git verb is unbound and
unhinted, the rail has no `+ claude session` row, and the menu has no
session rows — the ticket page's Enter excepted, because on a note row it
is the note's; for a **viewer** (`Ctx::team_viewer`) every ticket mutation
and every chord prefix whose tail is one stands down, and the ticket
page's Enter on a note reads `read note`. The daemon holds the same line
at its chokepoint (`team_read_only`, right after `authorize`): a viewer's
write to a ticket or a note is refused with the owner's name to ask,
before it can stand on the local copy alone. Tags, seen-marks and the
column settings stay a viewer's own — none of them is shared.

**The footer says how the board stands.** `Synced ∙ 3 members`, `Offline ∙
2 drafts` (drafts outrank members: they are what is not done), and for a
viewer `∙ you read only` — the passive answer to "why is `r` not here". On
the board only; a dialog's frame carries its own word.

**Who changed it is on the snapshot, per object.** `Published.by` in
`team.json` is the member's name when the record was applied from the
relay and `None` when it was sent from here, cleared the moment
`team_after_broadcast` queues a local change — so the initials come off in
the same broadcast as the edit. `TeamBoard.edited_elsewhere` is that map
by ULID; a card wears the teammate's initials (`AO`, `Da`) beside the
worktree mark's slot, and a note editor whose note moved on since it
opened (`Editor::opened_rev`) keeps the draft and says `changed elsewhere
by Amit ∙ saving overwrites` — saving is still last writer wins, which is
the T-333 conflict rule, now with the reader warned. The board cursor
follows its TICKET across a snapshot (`App::follow_ticket`), so a card a
teammate moved or reordered keeps the selection.

**Two copies of a ticket project the same bytes, and a pending edit is not
written over.** The relay e2e found both. First, the author word: a maker's
own copy said `local` (or `agent:<uuid>`) and every other copy said
`member:<signer>`, so after any teammate's edit the receiving side's diff
saw a different digest and echoed its own projection back — which cleared
the teammate's name off the card a moment after it appeared, and on a
viewer's copy queued a draft the relay would refuse. `project::ticket_body`
now spells every maker `member:<name>` (`me` for this machine's own words),
and a ticket minted from a record keeps the record's word rather than its
signer's. Second, `team_apply` records the revision and digest of a record
for an object that has a local edit still in the outbox but does not apply
it to the board: the local edit is the later of the two, its put goes out
against their revision, and both copies end on it. Applying theirs first
was what lost a rename made here moments before an older record for the
same ticket arrived, and then echoed the lost state back as the local
truth. Conflicts remain last writer wins per object, as T-333 decided;
what changed is that "last" is now the edit that was made last, not the
one that happened to be retried last.

**Leaving is a sharing-dialog row.** A member's dialog lists the members
and ends in `Leave this board`, two presses like the owner's `Stop
sharing`; the menu row on a joined board reads `Shared by Amit ∙ synced`.
When the board goes, the dialog closes with `you left the board ∙ this
copy stays here` — the root and its tickets are still a board.

Goldens: `team_boards`, `team_boards_joining`, `board_remote`,
`board_viewer_offline`; `menu` and its two variants reminted for the new
row. The relay e2e (`team/relay/tests/board_e2e.rs`) now also proves the
per-object names on both copies, two clients converging on different
objects, a viewer refused by his own daemon, and a used and an unminted
code both failing without naming the board.

Not here: the breadcrumb on a joined board reads the owner's title for it
(`App::board_name`), but the short keys are still local (T-333's limit);
a contributor on a checkout of the same origin is T-338; ask my agent is
T-336. The CHANGELOG waits for the release that ships Teams, as it did for
T-333 and T-334.

## T-215 — one sharing dialog (T-335, user: "one sharing setting instead of Team")

T-334 put the identity under `Settings › Team` and the board's sharing
behind the menu's `Share this board`; T-335 added a third door, `Team
boards`. Three rows for one thing, and the owner asked for one. The menu's
single `Sharing` row now opens `Scope::Sharing`: one list in three
sections under quiet headings the cursor skips — `YOU` (the relay and the
name as fields in place, then `Sign in` or `Signed in as … ∙ enter signs
out`), `THIS BOARD` (publish and the notes switch, or the invite rows,
the members and `Stop sharing`, or the members and `Leave this board`),
and `BOARDS` (`Join a board with a code` as a field, then the boards the
relay lists). Signed out, only `YOU` is there and the dialog opens on the
relay row; signed in it opens on this board's first row. The row's label
names where the board stands (`Sharing: not signed in`, `Sharing`,
`Shared with 3 members ∙ synced`, `Shared by Amit ∙ synced`).

`Scope::Team`, `Scope::Share`, `Scope::TeamBoards`, their seven verbs, the
`TEAM_ITEMS` list and `Ctx::{team_relay, team_name, team_drafts_differ,
team_editing, share_enter_word, boards_enter_word}` are gone;
`Ctx::sharing_enter_word` is the one word Enter reads. `App::sharing_rows`
builds the rows, `sharing_words` their label, detail and Enter word, and
`sharing_act` acts — a field opens in place, a member removal, a stop and
a leave arm on one press, a board opens in place of this one. A heading's
Enter word is empty, so the keymap's rule keeps Enter inert there without
a case. `draw_rows` learned a heading row (one cell in, `dim3`). The
Settings list is back to three groups.

Goldens: `sharing_signed_out`, `sharing_editing`, `sharing_publish`,
`sharing_members`, `sharing_joined`, `sharing_joining` replace `team`,
`team_editing`, `share`, `share_members`, `team_boards`,
`team_boards_joining`; `menu`, its two variants and `settings` reminted.

## T-357 — a crashed Codex record on a deleted ticket held the machine awake (2026-09-12)

The simbly board showed `☕` for two days with nothing running. `pmset -g assertions`
named the board's own pid, held since its `U` reload. The holder was an invisible
record: a Codex session whose runtime had crashed at startup on 2026-09-10
(`No space left on device`, pane exit 1), which was dismissed, and whose ticket the
user then deleted. `delete_ticket` keeps an owned Codex record while `codex_stopping`
is set — on purpose, as the evidence that a separate app-server may still own the
checkout — and the observation loop clears the flag only when the runtime reports
`stopped`. A runtime that died before it could never reports anything, so the flag
was permanent; with the ticket gone, no gesture on the board reached the record.
`quiet::is_working` counts a stopping Codex record as working whatever its state,
`is_mid_turn` inherited that, and `caffeine_watch` held. The same record sat in
`Daemon::working` for the shared checkout, so a queued ask there would never have
delivered and the merge train never saw a quiet board.

**Two fixes, two different clauses.** `is_mid_turn` now also requires
`state.has_pane()`: the keep-awake question is about the MACHINE, and a record
with no pane has no process to keep it awake for. `is_working` is unchanged — a
stopping record still owns its checkout and its agent seat until cleanup is
confirmed; that is the doctrine and `codex_observation_loss_holds_a_finished_checkout_until_reconciled`
still pins it. So the two predicates now differ on two clauses, both named in the
doc comment: a wait on a person, and a record without a pane.

**The daemon releases the orphan itself, on positive evidence only.** A new tick
stage, `sweep_codex_orphans`, looks for an owned Codex record that is stopping, has
no pane, whose ticket is gone and out of the undo window. For each it runs the
rung a human resume relies on (`unverified_cleanup_resume_eligible`, minus the
launch-target part): pane absent, tmux endpoint absent when tmux answered nothing,
no live conversation owner, `recovery_owner_absent` (no listener on either runtime
socket, no same-user process naming the config or the sockets, a complete `ps`
inventory that saw this daemon). The `ps` fork runs on a worker thread and comes
back as `Msg::CodexOrphansChecked`; the writer re-judges the record (same
generation, still an orphan) before `retain`ing it away, journals `codex orphan
released`, and feeds `CodexOrphanReleased`. A refused check keeps the record,
journals the reason once (`codex orphan kept: … rechecked every 15s`), and retries
every `CODEX_ORPHAN_RETRY`. Lost evidence still never means done: a missing config
file or an incomplete inventory refuses forever, as it did before. What changed is
the acknowledgement: the person's deletion of the ticket, past its nine-second undo
window, stands in for the second `confirm` a resume would have asked for — there is
no other gesture left to ask with.

Pinned by `cleanup_with_no_pane_owns_the_checkout_but_is_not_mid_turn`
(`core/src/quiet.rs`), a Codex dismissed-with-stopping snapshot in
`observes_work_permission_and_completion_without_any_app_ticks`
(`tui/src/caffeine_watch.rs`), and
`a_deleted_tickets_crashed_codex_record_is_released_once_no_known_owner_remains`
(`provider_e2e.rs`): a runtime that exits without a stop ack, a listener bound on
its proxy socket through the undo window and one retry (the record stays and the
journal says why), then the listener dropped and the record gone, with the feed
and journal lines. No wire change, no schema change; `CHANGELOG.md` gets its line
at the next bump.

The two-day dogfood record on simbly is released by the first daemon of this build
on that board, after its first `ps`.

## A folded column's count is one cell (T-359, 2026-09-13, user: "collapsed column with 9+ items overflows ∙ should show 9 and immediately below it, instead of the space, some '+' tiny glyph")

T-302 wrote the count "a digit a row", so a folded column of twelve stacked `1` over `2`: two
rows that read as two counts, and the name a row lower than its neighbours'. The user's own
repair is the one taken: the digit is `9` at most, and the row under it — the row a smaller count
leaves blank to align with the blank under the headers — carries `⁺` when the column holds more.
`spine_count(n)` is the whole rule, `(digit, ' ')` through nine and `('9', '⁺')` past it, so the
name starts on the same row whatever the column holds and T-302's "count of ten or more starts
one lower again" stagger is gone. The superscript plus is the "tiny glyph" asked for; it is
outside the box-drawing range the L1 law bans and is drawn in the count's own `dim2`. The `!`
still claims row 0 when anything waits and pushes the pair down one row, unchanged.

`the_folded_column_caps_its_count_at_nine_plus` pins thirteen as `9` over `⁺` with `DONE` under
them on the rows a one-digit count uses. No golden moved — every fixture spine holds fewer than
ten. Nothing else moved — no `Command`, no snapshot field, no schema, no key, no preference.

## A held plan is not stale (T-363, 2026-09-13, user: "pending plan ticket stop showing 'needs you' mark after a while")

Three plan cards on the simbly board lost their mark with the agent still waiting on the plan.
The feed is exact: `requires_action ∙ plan` at seq 745, `unknown ∙ no_signal ∙ stale` at seq 817,
900,125 ms later — the machine's 15-minute stale demote, which measured a `RequiresAction`'s age
from entry and asked no evidence before dropping it. The user's approval landed at seq 830 on a
card that already read `?`. 11 §11.7.4's rule ("never latch red") was written for a wait that
lost its clearing event — an Esc on a dialog fires no hook — and it cannot tell that wait from a
person at lunch.

**The clock now runs from the last affirmation, not from entry.** `Machine` carries
`affirmed_at` beside `entered_at`; a commit sets both, and a signal whose target is the current
state — the re-affirmation branch that already cancelled a pending leave — resets `affirmed_at`.
`tick`'s demote reads `affirmed_at`. Nothing else in the machine moved: `STALE_DEMOTE_MS` is
still fifteen minutes, ranks are D28's, and `stale_demotes_after_15min_never_latches` still
passes, because a wait nobody restates demotes exactly as before.

**The transcript is what restates it.** While a Claude record holds `Plan` or `Question`, the
recovery adapter's transcript channel (already polling that record for the Esc's aborted record)
reads `tail::last_event` once a minute (`WAIT_AFFIRM_MS`) and, when the last uuid-bearing record
is that reason's own pending tool call, emits the matching `TranscriptHint`. Verified against a
live transcript: after the `ExitPlanMode` call Claude Code writes only uuid-less `last-prompt` and
`cost-state` latch records, which `last_event` skips, until the answer's `tool_result`. The hint
is Low and the machine's re-affirmation branch never lowers a High state, so the card stays as
the hook stated it. The clearing roads are untouched: the answer is a `PostToolUse` frame, the
Esc is the aborted record the same poll already catches, the next prompt is `UserPromptSubmit`.
A lost answer frame affirms nothing — the tail then ends in the `tool_result` — so that wait
still demotes at fifteen minutes, which is the case the clock was for.

**Not `Permission`.** A generic permission dialog leaves an ordinary tool call in the transcript,
indistinguishable from a tool that is running, so `pending_dialog` has no arm for it and a
permission left open still demotes at fifteen minutes. Claude's session file says `waiting` for
that dialog (`StatusProbe::permission_resumed` already reads it), and a later ticket can affirm a
held permission off that word the same way; it is a different evidence channel and was not asked
for here.

Pinned three ways: `a_restated_wait_re_arms_the_stale_clock` (core), `a_held_dialog_is_restated_off_the_tail_once_a_minute`
(the adapter: once a minute, only the matching tool, nothing once the answer is the last word,
nothing for a permission), and `docs/state-scenarios/held-plan-outlives-the-stale-clock.json`
(the field timeline through the production replay). No `Command`, no snapshot field, no schema,
no key; a running daemon picks it up on restart (`U`).

## One board overrides selected machine prefs (T-361, 2026-09-13)

Spun out of T-360. `columns.toml` is the board's and `prefs.json` the machine's, and nothing
could be set on one level and overridden on the other — the train's own block above lists "a
per-repo preference" under *Not done* and the T-343 block names "a per-repo TUI file under the
state dir" as the obvious next step. This is that file: `~/.local/state/mesimon/<proj16>/prefs.json`
(`Paths::prefs_file`), the TUI's second preference file, SPARSE — it holds only the keys this
board sets, an absent key is "inherit the machine's", and clearing an override removes the key.
The decisions, each argued with the user:

- **Under the state dir, not in `columns.toml`, not a map inside `prefs.json`.** Private to the
  machine: a joined team board (T-335) projects nothing from either file, so a teammate's board
  can never switch this machine's notifications on. No daemon change, no `Command`, no
  `COLUMNS_SCHEMA` argument, no wire — the daemon still reads no preference file.
- **Which keys.** The train and its notice, keep awake, all seven notification keys, the snooze
  return, and both theme slots. `PrefKey::board_overridable` (`core/src/prefs.rs`) is the list;
  the two it refuses are the tmux status line's side (about the terminal) and the week's first
  day (about the person). A machine-only key found in a board file is ignored by the overlay and
  kept by every save, in case a newer build made it overridable.
- **The resolved view.** `App::prefs` is now `machine_prefs.overlay(&board_prefs)`, so the ~40
  readers did not move. `save_prefs` writes `machine_prefs` — the one trap, pinned by
  `a_board_override_never_reaches_the_machine_file` — and a test seeds a preference through
  `seed_pref`, because assigning `prefs` is undone by the next resolve.
- **`b` is the scope switch**, hinted and live only where a row of the list can be set for this
  board (Appearance, Behaviour, the notifications list; `Ctx::pref_scope_offered`), the title
  gains `∙ THIS BOARD`, and the scope resets every time Settings opens. `Tab` was rejected: it
  is `Describe` on the board and `TagColor` in the tag chord. In board scope Enter cycles
  `inherit → on → off → inherit` (a sound: inherit, then each rung); `keymap::pref_key` names a
  row's key and `keymap::item_detail` is the ONE place the scope words are added, in front —
  `set here ∙ machine: off`, `inherited`, `(machine)` — because a long detail reveals its tail
  marquee-style and which scope holds the value is what the dialog exists to show. The theme
  row still opens its picker, which grows an `inherit` row first in board scope.
- **The train push.** `reconcile_train` pushes only an ON, so one board never disarms another's
  — but a board that SETS the train is a choice about this repo's own daemon, so `run` pushes
  `SetAutomation` at startup whenever the board file holds the key, off included; a daemon
  this board armed last session then hears the off.
- **Foreign values** in the board file follow the machine file's rule one level up: a name this
  build cannot parse reads as inherit (the machine's value stands where the machine file's
  "the default stands") and survives every save until a pick of that key replaces it.

`doctor -v` gained a `board prefs` line for the cwd's repo. Goldens: the `b` hint joined the
tail of `settings_appearance_*`, `settings_behaviour_*` and `notifications_120x30`; four new
ones show board scope (`settings_behaviour_board_120x30`, `settings_appearance_board_60x20`,
`notifications_board_120x30`, `theme_picker_board_120x30`). The reverse direction — the machine
overriding a board setting — stays out of scope, unargued.

## The read tools are pre-approved on argv and hinted read-only (T-362, 2026-09-13, user: "plan mode keep asking for permission for mesimon read ticket ∙ how to mitigate?")

`get_ticket`, the call every session is told to make first, asked for permission on every
turn of a plan-mode session. The first cut of this ticket added one pair to the argv `flags()`
already builds — `--allowedTools mcp__mesimon__get_ticket,mcp__mesimon__list_board,
mcp__mesimon__read_note` — on the docs' word that allow rules apply in every mode and that
Claude Code reads no MCP `readOnlyHint`. T-366 asked again with the flag on its argv, so the
claim was measured instead, headless on Claude Code 2.1.270 with a stub stdio server carrying
one hinted tool, one plain and one destructive-hinted:

- **Plan mode ignores allow rules for MCP tools** — `--allowedTools` on the plain tool still
  reads `Cannot call … while in plan mode` — and admits exactly the tools whose
  `annotations.readOnlyHint` is `true`, refusing the rest outright (no prompt headless; a
  prompt in the TUI).
- **Default and auto mode ignore `readOnlyHint`** — the hinted tool still prompts — and admit
  exactly what an allow rule names.

So the read rung carries both spellings, and they are one list: `mcp::allowed_tool_names(tier)`
is the read rung of `tools_for(tier)` as `--allowedTools` (the flag for default and auto mode),
and the three definitions in `tools()` carry `"annotations": { "readOnlyHint": true }` (the
annotation for plan mode). `read_rung_is_hinted_read_only` pins the annotation to the rung
both ways and admits no other annotation key — `annotations.title` is text the model reads
and docs/15 lists it as an injection surface; the boolean is not. Writers are never in either
list: `write_note` and `move_ticket` still prompt where the mode prompts and are refused in
plan mode, which is what plan mode is for. Empty at `Off`, where the flag is omitted with the
blob. Argv rather than a settings file, and rather than a `permissions.allow` block in the
generated hook settings, because promise 2 forbids touching the user's config and the hook
file should carry hooks. `--allowedTools` is variadic (it ate a positional prompt in the
headless probe), so it must sit before another `--flag` on the argv, which `flags()` does; it
joined the `owned` list in `resume`, so a wake refreshes it like the blob. A live pane keeps
its argv and its `tools/list`, so a session spawned before this build must be woken.

Verified against the real shim, headless, on 2.1.270: plan mode with the flag runs
`get_ticket` and `list_board` and refuses `write_note`; default mode with the flag runs both
reads unprompted. Pinned by `allowed_tools_are_the_read_rung_only` and
`read_rung_is_hinted_read_only` (core) and `agent_tools_e2e` (the flag and its value at
`Full`, its absence at `Off`); `every_tool_fits_the_budget` still holds with the annotation.
`doctor --mcp` prints the pair. No wire change, no schema change; `CHANGELOG.md` gets its line
at the next bump.

## The reply row is remembered (T-365, 2026-09-13, user: "remember peak setting between mesimon shutdowns")

`p` and `P` (T-237) set a view the board forgot on every launch; the `App::prefs` doc had
called carrying `p` into `prefs.json` "a follow-up" since the file was born. It is a key now:
`peek`, one of `off` / `cursor` / `all` (`PeekLevel`, `tui/src/prefs.rs`) — one value rather
than two flags, because the ladder has an invariant (all implies cursor) that two flags could
spell wrong. Absent is `off`, how every board opened before, so no schema move; a rung this
build does not know reads as `off` and survives every save until a press replaces it, the
week-start rule. **Machine-only**, the third key after the status line's side and the week's
first day: how one reads a board is about the person, and there is no Settings row to set it
per board — T-237's "no need to hint this" stands, the two keys stay overlay-only and
`settings_items` still omits them. `App::peek` and `App::peek_all` are now DERIVED in
`resolve_prefs` from the resolved view, so the two flags every reader asks come from the one
place the preference lands and a remembered rung opens the board before any key; the two arms
set the rung through `set_pref`, which is why a press now says `∙ saved` (or `for this
session` where the file is absent or barred). A golden still sets the flags directly for one
frame. `doctor` gained a `replies` line. Pinned by `the_reply_row_is_remembered_between_boards`
(app) and `the_reply_row_defaults_off_and_round_trips` (prefs); `CHANGELOG.md` gets its line at
the next bump.

## The terminal is per ticket and adoptable (T-366, 2026-09-13)

Asked for as "per ticket shell (`!`) and shell adopt in ticket page": if the shell was used,
show it as a ghost row under the agent session and adopt it with Enter twice; show its command
and output in the preview before and after adopting; treat a shell running a command as the
ticket running, below a recognised agent; recognise agents started in shells (deferred, own
ticket — T-369).

**`!` is the ticket's.** `App::terminal_ticket` names the ticket on EVERY ticket page now, not
only one with an attached worktree — T-273's collapse of a shared-checkout ticket's `!` to the
checkout's `msmn-term` would have made the ghost row, the preview and the adoption about a shell
that belongs to no ticket. The daemon already handled `Some(ticket)` with no binding (the
checkout, named `msmn-term-<ulid>`); only the TUI changed. The board's `!` is still the root's.

**The daemon reads its panes, and keeps what it reads in memory.** `refresh_titles` became
`refresh_panes` on the same 2 s bucket and the same one fork (`TmuxBackend::pane_facts`, which
is `titles()` grown two fields — `pane_dead` and `pane_current_command`, with the title still
last because it is the one field that may hold the separator). From it: the titles as before;
`Daemon::foregrounds` (record → the command its pane runs) and `Daemon::terminals` (directory
→ alive terminal → its command). `foreground_of` (`core/src/board.rs`) is the rule: tmux names
the foreground PROCESS, so the shell's own name is idle and anything else is a command — and
"the shell's own name" is the launched shell's basename OR any of `SHELL_NAMES`, because macOS's
`/bin/sh` execs `bash` and the pane says `bash` (the e2e found this: `sh` never came back). Both
maps ride the snapshot only — `Response::Board.terminals` and `SessionRecord.foreground` (serde
default, skipped when None) — and a change in them is broadcast directly WITHOUT reporting
`changed`, so `persist_sessions` never runs for a foreground and `sessions.json` never carries
one; a restart re-seeds `terminals` from the reconcile snapshot (`terminals_in`) and re-reads
the commands on the first poll. `open_terminal` enters its terminal into the map at once, so
the ghost row is on the rail when the handover returns rather than two seconds later.

**Adoption is a rename.** `Command::AdoptTerminal { ticket }` → `Daemon::adopt_terminal` mints
the Bash record a `SpawnSession { Bash }` would have (`[$SHELL]`, the terminal's directory,
`Running`, provenance `Spawned` — `Adopted` is the external drawer's word and badges
`external`), then `tmux rename-session` the pane from `msmn-term-<ulid>` to the record's
`sid16`. Nothing else moves: `pane_tail`, `sleep_eligible`'s live-children guard, `wake`,
`kill`, `refresh_rss`, the pane-died hook (tmux expands `#{session_name}` at fire time, so a
death reports the NEW name) and reconcile all key on the name. The rest mirrors
`spawn_session`'s tail — `machines.insert`, `lock_worktree`, `persist_and_notify`. Refused
while the focus token is held on that terminal (the user is inside it), on an archived or
content-only ticket, and when no alive pane of that exact name is in a fresh snapshot — the
snapshot check is the guard against `-t`'s prefix matching (`msmn-term` is a prefix of every
ticket's name; `rename_session`'s doc says so). `Command::TerminalTail { ticket, lines }` is
`PaneTail` for the unadopted pane, over the same `tail_of`. Both denied to agents.

**What changes for an adopted shell, on purpose:** it has a pane, so `archive_ticket` refuses
until it is slept and `set_workspace` locks — the unadopted ghost never did either. That is the
difference between a place to stand and a session of the ticket, and it is the reason adoption
takes two presses. `x` CLOSES it (see the amendment below; the live-children guard still
refuses while a command runs).

**The rail and the preview.** `RailRow::Terminal` sits between the sessions and the `+ claude
session` offer — a ghost in the dim register, the shell's `$` mark, the word `terminal` or the
command running (`$ cargo`), the spinner while one runs, and a second line that says what Enter
does (`enter adopts` / `enter again adopts`). `notes_start` and the offer's index
count it (`offer_at`), the bug the comment there records. `App::adopt_armed` is the two-press
state, disarmed beside `just_created` on any key but Enter; the `enter` binding's hint reads
`adopt shell` / `enter again adopts`. `ShellTail` is keyed by `TailKey { Session(uuid),
Terminal(ulid) }` and `poll_shell_tail` picks `PaneTail` or `TerminalTail` by it, on the same
1 s clock; the zone draws the terminal's pane under the same `PREVIEW` heading, keyed by
`terminal_key` for the scroll, and says `reading its pane` for the one poll before the first
capture. After adoption the row is a session row and everything is the shell's existing road.

**`!` after an adoption opens a fresh terminal, adoptable in turn** (the user, on the first
build: "pressing `!` after adopting shell should open a new shell, to adopt as well"). The
first build routed the key to the adopted shell (`Board::live_shell`, a `shell` hint), on the
reasoning that the same key should find the same shell; that made the key a focus for a row
Enter already reaches and left no way to grow a second shell without the gated `S`. So the
adopted shell is a session row and Enter is its road, `!` is always the terminal (the name
`msmn-term-<ulid>` is free again the moment the old pane is renamed), and a ticket may hold as
many adopted shells as the user cares to make. `live_shell` and `ticket_has_shell` were removed
with the routing.

**"Running" is the spinner and nothing more.** `glyphs::is_working` counts a Bash record
`Running` WITH a foreground; `card_glyph` gained `terminal_busy` for the unadopted ghost, OR-ed
into the working arm — so a busy shell sits under needs-you, failed and done, and over
launching, sleeping and unknown, which is "lower priority than recognised agents" by the
existing table. `session_glyph` and `age_slot` follow from `is_working`. NOT touched:
`quiet::is_working` (the checkout-quiet gate and the merge train — a `cargo build` in a
worktree is no reason to hold the checkout), `automove` (a shell's foreground never enters the
attention machine, so no `Running` edge fires and a shell command cannot move a BACKLOG ticket),
`is_hot` (board Enter still goes to the agent). A shell at its prompt still never spins
(`a_shell_never_spins` stands; `a_busy_shell_spins` is its complement).

**Not done: binding the agent inside the shell.** A `claude` typed into a ticket shell has no
`--session-id`, no hooks and no MCP; the row now SAYS `$ claude` (or `$ node`, for an
npm-installed one — the process name), and T-369 holds the binding: the census over the shell's
cwd, observe-only attach, `--resume` takeover on exit.

Not gated on `MESIMON_TICKET_SHELLS`: the seam gates STARTING a shell from `s`/`S`; adopting the
terminal the user already opened is the door this ticket asked for. `CHANGELOG.md` gets its
lines at the next bump: Added — adopt the ticket's terminal from its page, preview it before
adopting, a shell running a command spins the card; Changed — `!` on any ticket page is that
ticket's own terminal.

Tests: `terminal_adopt_e2e` (open, list, foreground, `TerminalTail`, adopt → rename, `PaneTail`,
sleep refused then parked and woken, foreground absent from `sessions.json`),
`enter_twice_adopts_the_terminal` and `bang_opens_another_terminal_beside_the_shell` (app),
`the_terminal_opens_the_screens_directory_and_returns_to_it` (the per-ticket rule),
`a_busy_shell_spins` (glyphs), `a_shell_at_its_prompt_is_idle_and_anything_else_is_a_command`
(core), the `parse_facts` case in the backend, goldens `ticket_terminal_120x30`,
`ticket_terminal_armed_120x30`, `ticket_shell_busy_120x30`; `help_ticket_120x30` is unchanged.

## A shell's sleep is its close (T-366 amendment, 2026-09-13)

The user, on the first build: "sleep of an adopted shell kills it, remove it from the records."
Sleeping a shell parked the record as `Sleeping` and killed its pane, and a wake respawned a
fresh `$SHELL` under the same row — a different shell wearing the same name, as the 2026-09-01
block already said. That row was a promise the board could not keep: nothing of the shell
survives its pane, so there is nothing to park.

**Built.** `sleep_one` on a `SessionKind::Bash` record, once `sleep_eligible` passes (the
live-children guard stands: a shell running a command is refused, as before), REMOVES the
record — `board.sessions`, `machines`, `recovery`, `foregrounds` — and sends the pane down the
same kill ladder as a parked agent's (`signal_session` + `reaping`); the journal says `shell
closed`. The worktree lock releases on the reaper's next pass, where a ticket with no live
session already released it. `persist_and_notify` in the `SleepSession` arm writes the board
without the record, so `sessions.json` never keeps a closed shell. `WakeSession` on the id says
`no such session`; the Bash wake arm stays for boards written before this that hold a
`Sleeping` shell (the UI fixture's session 71 is one).

**Bulk gestures park agents and leave shells alone.** `reclaim_all` and `reclaim_figures` take
agents only, so the header's sleep offer never prices or ends a shell; the board's `x` (sleep
the ticket's sessions) skips shells on the sleep side and still wakes a parked one from an old
board. The explicit close is the ticket rail's `x` on the shell's own row, hinted `close shell`
(`Ctx::sel_shell`).

**Not done, on purpose:** a shell whose pane died on its own (`exit` typed) still leaves an
`Exited{UserQuit}` record the rail hides, as `exit_parks_e2e` pins — the ask was about sleep,
and dropping records on pane death is a different road (the reaper's) worth its own line if
wanted.

Tests: `terminal_adopt_e2e` (sleep → no record, no pane, no line in `sessions.json`, wake
refused), golden `ticket_shell_busy_120x30` (`x close shell`).

## The Linux release needs a musl C toolchain (T-373, 2026-09-14)

**What broke.** The alpha.21 dry-run failed at `ci/build-linux.sh`: `ring` 0.17 could not find
`x86_64-linux-musl-gcc`. `ring` is rustls' crypto provider (`mesimon-team` picks
`features = ["std", "ring", "tls12"]`, T-332) and the daemon links `mesimon-team` unconditionally
— the Teams doors are release-gated, the client is not — so the shipped binary now carries C and
assembly. `build-linux.sh`'s header said the graph had no C and cross-linked with `rust-lld`
alone; that was true at alpha.20 (no `ring` in that lock) and false since T-332, and nobody ran a
release between. `cargo tree -i cc` confirms `ring` is the only C in the Linux graph.

**Refuted.** Apple clang as the C compiler: its `stddef.h` does `#include_next` into a libc this
Mac does not have, so it fails on both targets, hosted and `-ffreestanding`. A pure-Rust provider:
`rustls-rustcrypto` is `0.0.2-alpha` — not on a release day; worth its own ticket if the "no C"
property is wanted back. A macOS-only release: `install.sh`, `release.rs` and a unit test pin both
Linux artifact names, so skipping them is script surgery, not less work. Feature-gating Teams out
of release builds: `Principal::Remote` and `teamglue` run through the daemon; too large.

**Built.** The release machine carries the prebuilt musl toolchains from
`messense/macos-cross-toolchains` (`brew trust` the tap first — Homebrew refuses untrusted
third-party taps now — then `brew install messense/macos-cross-toolchains/<target>`); they put
`<arch>-linux-musl-gcc` and `-ar` on PATH, the exact names the `cc` crate looks for, so the build
needs no `CC_*` env. `build-linux.sh` checks for each target's gcc with a `die` that prints the
brew line, and its header tells the truth. Both targets build static; x86_64 as static-pie, as
before. `ci/dup-deps.allow` was regenerated the same day for the crypto/TLS duplicates
(`base64`, `chacha20`, `rand`, `rustix`, `sha2` and kin); `unicode-width` is unchanged, so D22/05
is not touched.

**Tests:** none — the gate is the release script, exercised by the dry-run.

## Shift+Enter on a column header asks every agent in it (T-378, 2026-09-14)

**The ask.** "stand on column — shift+enter to queue a message to all." The 2026-09-01 block
("Shift+Enter asks the agent from the board") closed with the field being deliberately per-card
and named a wider home "a second decision, not a free extension". This is that decision, taken.

**What it is.** With the cursor on a column header (T-117's position, `Ctx::col_header`) and at
least one agent seated in the column (`Ctx::col_seats`, paned or parked), Shift+Enter opens the
same one-line field, hanging UNDER THE HEADER — the header is what names where the words go, so it
stays whole, the way a card does under its own field — and Enter sends one wire command,
`PromptColumn { column, text, queued }`. The daemon walks `column_tickets` in board order and
puts the words in front of every seated agent through the one road (`deliver`): a pane is pasted
into, a parked agent is woken with the words held for its first tick. A ticket with no agent is
**skipped and counted, never started** — a column is not a place to spawn N claudes from one key,
and that refusal is what keeps the plural the same idea as the singular
(`shift_enter_asks_claude_at_every_stage` argues it; `a_header_offers_exactly_the_column_verbs`
pins that the header's verb list is unchanged because the binding was widened, not doubled —
`no_key_bound_twice_in_a_chain` forbids a second `ShiftEnter` in `BOARD`). The hint is `ask every
claude` / `ask every codex` on the board's default provider; the receipt `Response::Asked { sent,
woke, queued, skipped, failed }` becomes the status `asked 3 ∙ queued 2 ∙ 1 without claude`.

**Queued by default where the checkout is shared.** A column asked at once is the five-claudes
incident (2026-09-04) by construction, so the field opens at `queued` when any seated agent in
the column shares the checkout, and Shift+Tab flips it to `now` — the single ask's toggle, the
opposite default. Queued, each shared-checkout seat is parked through `park_ask` (the checks and
the push-or-replace split out of `enqueue_ask`, which is now `park_ask` + one drain + the receipt)
and the queue is drained ONCE with one broadcast, so the column goes one agent at a time as the
checkout quiets, in board order (T-263). A worktree ticket's checkout is its own and is sent now
either way, as `enqueue_ask` always refused it; a column of them opens at `now` with no toggle
row. A column ask replaces a ticket's waiting ask in place and a send-now drops it — the single
ask's own rules, per ticket.

**Wiring.** `Command::meta` says `Mutate, logged: false, subject: None`: the subject is single and
so is a feed line, so the handler logs `prompt_column` once and then `prompt_column_sent` /
`_woke` / `_failed` per ticket (parked seats keep `queued_ask` / `queued_wake`). The chokepoint
hears `Resource::Column`. `agent_allows` denies it beside `PromptSession` (N input boxes at
once). `viewer_edit` names it explicitly, since `subject: None` would otherwise let a viewer by.
The TUI's `InputPurpose::Prompt` gained `target: AskTarget::{Ticket, Column}` in place of the
ticket id, so the field's history walk, paste, mode word and Shift+Tab are the same code;
`draw_column` computes `head_rows` (header, field, delivery row, blank) where four places hard-
coded the two.

**Not done.** The worktree-sends-now case has no e2e of its own (it would need a provisioned
worktree); it is covered by `enqueue_ask`'s refusal and the TUI's inert toggle. A mixed
claude/codex column is hinted with the board's default word; the receipt counts seats.

Tests: keymap validators above; `shift_enter_on_a_header_asks_every_agent_in_the_column` and
three siblings in `app.rs`; goldens `board_column_prompt_120x30` / `_now_`; `column_ask_e2e`
(now: 1 sent, 1 woke, 1 skipped, nothing started, the woken pane reads on `SessionStart`;
queued: 2 parked in order, drained on the holder's settle; unknown column and blank text refused).

## Shift+Enter on an empty seat opens the ask field (T-379, 2026-09-14)

**The ask.** "shift+enter on pending ticket to show ask agent — instead of immediately starting,
it will allow the user to choose their prompt (empty for title), and for shared checkout timing
(queued, shift + tab)."

**What changed.** The board's Shift+Enter over a ticket with no claude opened no field: since
2026-09-03 it was the composer's second half a press late — `start_composed`, claude spawned with
the title submitted — and only a busy shared checkout (T-294) stopped to open the field at `queued`.
Now the empty seat opens the same one-line ask field every time, and the busy checkout changes
only the default it opens at: `now` on a quiet one, `queued` while another claude works in the
same checkout, Shift+Tab flipping either on a shared-checkout ticket. Nothing spawns until Enter.
Typed words are the first prompt (`PromptSession` → `deliver` → `QueuedSeat::Start` →
`spawn_session` with the words); a blank Enter is the title, which `prompt_session` already admitted
on an empty seat as the one blank it delivers (T-294), and the placeholder says so (`start on the
title`). A worktree ticket opens at `now` with no toggle row, as its checkout is its own. The hint is
`start + ask claude` / `codex` on both the quiet and the busy seat — the parked seat's `wake + ask`
shape, naming the extra thing the press does — where it read `ask claude the title` / `start claude`.

**Not changed.** The one-key start on the title is still the composer's Shift+Enter, the editor's
`^S`, the note-then-start road and the Enter-Enter fast path; `start_composed` remains theirs. The
`Verb::Prompt` dispatch arm lost its `checkout_busy` branch — the two empty-seat branches were the
same field with a different `queued`. No daemon change: the road the busy case already took.

Tests: `shift_enter_on_a_ticket_without_claude_opens_the_field_and_starts_on_enter` (quiet: field
at `now`, typed words start through the ask road, blank Enter starts on the title, Shift+Tab parks
the start, Esc starts nothing), `a_worktree_ticket_never_stops_to_ask` (no toggle, opens at `now`),
`shift_enter_on_an_empty_seat_starts_claude_on_the_title` in the keymap. No golden changes.

## Tab grows the ask field into the composer's room (T-380, 2026-09-14)

**The ask.** "tab on ask agent to show big composer to edit prompt like ticket creation." The
one-line ask field (Shift+Enter on a card or a column header) had no bigger room: a prompt longer
than a sentence was edited in a card-width window, and a line break was impossible — `sanitize_prompt`
dropped every control, `\n` included.

**What it is.** `Tab` in the ask field is the composer's `Tab` (`Verb::Describe`, the same binding
widened to `composing || prompting`, hinted `expand`): the same dialog grows out of the card, on a
new `EditorPurpose::Ask { target, queued }`. The field's text is the body with the cursor at its end
(pasted into a fresh `TextArea` rather than opened on, so the sentence continues), the title row is
the destination read-only — the ticket's title, or the column's name in capitals — and the frame's
edge says who the words reach (`ASK CLAUDE`, `ASK EVERY CODEX`, by seat or by board default). The
context row is the field's delivery row: `T-3 ∙ now  shift+tab`, or `2 claudes ∙ queued  shift+tab`
for a column. In the room Enter is a line break, `^s` is the field's Enter (`send` / `queue`,
through `commit_prompt` — history, seat word and receipt unchanged), Shift+Tab is the now/queued
toggle exactly where the one-line field offers it (gated on `ask_queueable`, never on the ticket's
workspace being open, which is the note editor's reason for the same key), and the two keys that
save something to the board are off: `^S` (a second send) and `^t` (a prompt wears no tags).
`^g` still hands the body to `$EDITOR` as `ask-T-3.md`. A clean Esc folds back into the one-line
field on the same text; a dirty one asks twice and drops the ask, the composer's own rule. A blank
room refuses to send and stays open. The header chip stays `BOARD` (a dialog over the board, like
the composer's) and the mode word is `ASK`.

**Line breaks travel.** Measured live 2026-09-14 (`/opt/homebrew/bin/tmux`, `load-buffer -` →
`paste-buffer -p`, then a separate Enter): claude's box shows `[Pasted text #1 +3 lines]` and the
Enter submits ONE turn whose reply reads all the lines; codex's box shows the lines and submits
them as one. So `sanitize_prompt` now crosses `text::scrub_lines` — `\r\n` becomes `\n`, `\n`
survives, every other control and a lone `\r` go; still only ever removes, so promise 3 holds
literally. The agent-prompt templates (T-353) leaned on the old stripping for their one-line law,
so they got their own twin, `prompts::sanitize_template` (the old behaviour, every newline gone),
at the daemon's `set_agent_prompt`, the Settings row and the TUI's fake — `brief_offer_e2e` caught
it. The history keeps the lines it went out with; recalled into the one-line field (`↑`)
they are spaces (`history_field`, `one_line`), and `Tab` reopens the room on that line. A WAITING
ask with lines reopens in the room, not the field, so nothing is flattened on the way back.

**Not done.** The room is board-only, as the field is. The stub agent in `prompt_e2e` reads a line
at a time, so the e2e asserts both lines arrive in order with no CR and cannot tell one paste from
two — the one-box claim rests on the live measurement above. No `↑` history inside the room. A mixed
claude/codex column is named by the board's default word, as T-378 left it.

Tests: `the_ask_room_sends_on_ctrl_s_and_keeps_the_saving_keys_off`,
`tab_opens_the_editor_from_the_composer_and_the_card` (keymap);
`lines_for_a_process_keep_their_breaks_and_nothing_else`, `sanitize_prompt_drops_what_a_tty_would_act_on`,
`a_template_is_one_line` (core); `tab_grows_the_ask_field_into_the_room_and_ctrl_s_sends_it`,
`the_ask_room_folds_back_clean_and_discards_dirty`,
`the_column_ask_room_keeps_the_delivery_toggle_and_sends_the_column`,
`a_queued_ask_with_lines_reopens_in_the_room` (app); goldens `editor_ask_120x30`,
`editor_ask_column_120x30`, and the five ask-field board goldens reminted for `tab expand`;
`prompt_e2e` grew the two-line clause.

## A wake over the daemon's own dying pane is one gesture (T-381, 2026-09-15)

**Reported.** On the work computer, `x x` on a ticket page (sleep, then wake) came back "running
elsewhere (pid N) — resume again to override"; the user read it as "this session does not belong
here". Enter twice got in. Every return to the page showed the corpse mark, and Enter twice again
restarted the conversation from its last message. The ticket never showed the live session.

**Why.** Two faults, one feeding the other. (1) Claude Code keeps its `~/.claude/sessions/<pid>.json`
live for as long as its exit hooks run after the sleep's SIGTERM, and `resume_guard` read that pid
as an external owner: the daemon's OWN previous pane, still going down, was "elsewhere", and the
wake needed a confirm. (2) The confirmed resume's kill-session took that process down under the
fresh pane, and the killed process's `SessionEnd{other}` — same session uuid, the record's — landed
on the new pane's record after its `SessionStart` had minted `Idle`, where the machine reads it as a
death: `Exited{Crashed}`, the `x` mark. The `pane_reborn` guard (2026-09-04) refuted only a
`PaneDied`, and only in `Spawning`. From there the loop closes on itself: Enter on the corpse is a
resume, the resume finds the LIVE claude's pid file and refuses as elsewhere, the confirm kills that
live claude and spawns another `--resume`, whose record the kill's straggler marks dead again.

**Shipped.** `AgentAdapter::external_owner` returns an `ExternalOwner { pid, label }` instead of a
string, and `resume_guard` exempts an owner whose pid is the record's own pane's (`own_pane_pid`:
tmux lists the sid16 alive and not dead — the pane's process IS the agent's, `mesimon exec` execs).
The wake's kill-session finishes what the sleep began. `pane_reborn` became `straggler_death`: a
death frame — `PaneDied`, or an AGENT record's `SessionEnd{other | prompt_input_exit}` — for a pane
born inside `STRAGGLER_WINDOW` (10 s, `Daemon::pane_born`, in memory only, stamped at the three
record spawn sites) or still `Spawning`, which tmux lists alive, is the previous tenant's and is
dropped. `logout`, `clear`, `resume` and a shell record's `SessionEnd` stay out of it (the shell's
frames are the agent-inside-a-shell compatibility road, and `exit_parks_e2e` drives one by hand). A
real death inside the window still lands: tmux lists that pane dead, and where the `SessionEnd`
beat the exit the pane-died behind it carries the status — same `ExitReason` either way.

**Not done.** A record already stranded as `Exited{Crashed}` over a live pane by an older daemon
takes one more restart-from-last-message on its first Enter after the upgrade (the guard no longer
asks, the resume still replaces the pane); the rule prevents the state, it does not repair it. The
interrupt probe's `status_file_for` can read the dying process's file for the beat both are alive.

Tests: `wake_straggler_e2e` (fails on the unpatched daemon at the wake refusal).
## A held `k` stops on the first ticket like a held `↑` (T-382, 2026-09-15, user: "holding up arrow key to prevent going above top ticket copy to k ∙ currently it only works for up arrow but not for k")

**Refuted**: that the rich-terminal path covered both keys. On a terminal speaking the kitty
keyboard protocol (iTerm2 here), `↑` is an escape code and a held one arrives as `Repeat`
events, which `on_terminal_key` swallows at row 0. But the protocol reports a key that
produces text as plain UTF-8 — presses only, no repeat and no release — unless every key is
requested as an escape code, and mesimon does not ask for that (it would change how every
text field hears its keys). So a held `k` arrived as a stream of `Press` events, and the
`Press if rich_keys` arm cleared `last_ticket_up` on each one: the 120 ms gap rule never
engaged, and the cursor climbed onto the column header and the board's top row.

**Shipped**: on a rich terminal a `Press` of a text key no longer clears the guard, so `k`
runs on the legacy gap rule everywhere — a fresh `k` after a release-sized pause still steps
onto the header, and `↑` keeps its immediate fresh press. The rule is one line in
`App::on_terminal_key` and is the only place the two keys diverge.

Tests: `held_k_on_a_rich_terminal_is_presses_and_still_stops_at_the_top` (app).

## `!` typed a string of letters into its shell (T-383, 2026-09-15)

**Symptom:** on the author's work MacBook (iTerm2), pressing `!` opened the ticket's shell with
a string of letters and digits already on the prompt line. The home machine never showed it.

**Cause: the release of the key that starts a handover, reported under the kitty flags and
typed by tmux.** The board runs with `DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES` pushed on
a terminal that supports the protocol (iTerm2 ≥ 3.5). The press of `!` arrives as plain text;
its *release*, 50–150 ms later, arrives as `CSI 49;2:3u`. Between the press and the attach the
TUI reads nothing more from stdin: it asks the daemon `GateStatus` and `OpenTerminal`, and the
latter kills and spawns a tmux session — several subprocesses, which on a machine with endpoint
security take longer than a keypress. So the release report was already in stdin when
`restore_terminal` popped the flags and the tmux client inherited the fd. tmux 3.6a's
`tty_keys_extended_key` scans digits and `;` only and returns "not a key" at the `:`, so the
sequence fell through to `first_key`: an Escape, then `49;2:3u` as literal keys into a zsh
that was still starting — typeahead, inserted on the first prompt. Reproduced against a
scratch tmux 3.6a with a pty client: the pane received `^[[49;2:3u` verbatim. Enter into an
agent pane is the same road but spawns nothing, which is why the ticket named `!`.

**Fix:** `restore_terminal` now fences after the pop. `settle_key_reports` writes a
cursor-position query (`CSI 6 n`, crossterm's `cursor::position`) and waits for the answer,
which the terminal can only give once it has processed the pop, then empties crossterm's event
queue. A release reported before the fence is dropped there; one after it is never sent. This
covers every road through `restore_terminal` — the focus handover, the `!` shell, the `^g`
editor, `^Z` and exit. Non-kitty terminals never reach it (no flags, no reports, and plain
typeahead is the shell's). The cost is one round-trip; a kitty-capable terminal that does not
answer CPR would cost crossterm's 2 s timeout once per handover, and none is known.

**Not done:** teaching the daemon to spawn faster, and keeping the flags through the handover
(refused for the same reason as the focus report in T-292's amendment: tmux would read them).
Not unit-tested — the fence is a tty conversation; the tmux half is the repro above and the
trap is recorded in CLAUDE.md.

## T-317 — Mesophon M1: pair, preview, prompt, revoke

The approved browser-first milestone rides on the shipped Teams device identity,
TLS discovery, invite format, and encryption primitives. Control routing is a
separate optional HTTPS/WebSocket listener in `team/relay`; browser/crypto/daemon
clients remain Apache code. It does not publish private boards to Teams or reuse
Teams membership as host authority. `MESIMON_MESOPHON=1` exposes the TUI entry and
enables control in the running daemon; each board requires explicit opt-in.

The board writer persists private versioned grants, authenticates the endpoint,
and mints a distinct paired principal. Only the dedicated snapshot, preview,
prompt-existing, and receipt protocol crosses this transport. Raw local envelopes
cannot mint paired authority. Existing Teams/agent authorization floors remain.
Per-connection keys bind board, grant, daemon incarnation, browser challenge,
direction and sequence. A pinned owner signature authenticates the greeting;
replayed greetings fail the new browser challenge. One-use pairing expires after
ten minutes and is lost on restart. The relay receives only a hash and a
device-bound proof, not the secret.

Connections receive disjoint command-number ranges within the daemon incarnation.
A high-water mark prevents replay after the bounded receipt cache evicts details;
reconnect can query a prior receipt but cannot execute an old command. The browser
never retries a prompt automatically. Restart loses receipts and therefore reports
unknown outcomes. Codex's deferred path retains device/grant attribution and
rechecks both grant and exact ticket/session before paste or Enter. Revocation
cancels undelivered text; partial delivery is reported as unknown and is not
retracted from the agent. Full control, agent lifecycle, interactive terminals,
board editing, and stopped-host discovery remain later milestones.

The dedicated browser projection excludes notes, paths, argv, and the local board
snapshot. Selected output polls at two seconds; text uses DOM text rendering.
There is no offline content cache, service worker, or command queue. IndexedDB
holds only device identity/credentials and grant pins. Since the relay serves the
browser code, its web deployment remains trusted client distribution. Native TLS
pinning does not replace browser certificate trust.

The real relay acceptance test exposed a routing bug where the Gone notification
for an old browser also closed the host connection. Gone now closes only the
addressed browser; the host remains connected and receives the departure event.

Validation: the workspace suite passes 1,555 tests. The six opt-in relay tests
also pass against disposable PostgreSQL, including Chromium/WebKit at desktop and
phone widths, Claude delivery, deferred Codex revoke/submit, daemon restart,
stale command/receipt eviction, and unchanged Teams sharing/TLS behavior. Clippy
passes; the two new Mesophon TUI goldens and browser screenshots were inspected.
The unrelated search timing benchmark and live release download remain opt-in;
the ignored restart-skew helper is exercised by its parent tests. Container
configuration and shell syntax were checked; Linux/container image builds and
release rehearsal were not run for this milestone.

### T-317 follow-up — Mesophon defaults on in debug builds

Debug builds now expose the Mesophon dialog and activate the daemon's control
capability without `MESIMON_MESOPHON`. Release builds retain the `=1` opt-in.
This is a feature-availability default: individual boards still require explicit
local enablement and browser pairing. The normal daemon acceptance test verifies
the no-environment default leaves a fresh board disabled; relay acceptance now
also exercises pairing and restored grants without the flag in debug builds.

### T-317 deployment follow-up — exclude nested build output from Docker

The first local relay upgrade hit Docker's disk limit because the build context
included 4.448 GB of untracked files, chiefly the old `team/target` directory.
The root `target/` exclusion did not cover it. Docker now excludes all nested
`target/` directories and the `mt/` research scratch directory. Building the
committed source archive also avoids this legacy local state. This changes the
build context only; no running volumes or stored board data are removed.

### T-317 usability follow-up — certificate-free localhost browser

Manual Keychain trust is not the local Mesophon onboarding flow. The default
relay certificate was designed for native pinning; macOS browser validation also
rejected its long lifetime and missing server-authentication EKU. For a browser
on the same Mac, the user chose `http://localhost:8444`. The browser listener and
native control client now support HTTP/WS only for exact loopback origins. Remote
origins remain HTTPS/WSS; the native Teams listener and saved pin are unchanged.
Compose publishes the browser port on loopback independently of its Teams bind
address, and refuses HTTP with a non-loopback publication setting. Custom
container deployments must preserve that publication boundary. Asset requests
require the configured Host, and browser WebSockets still require the exact
Origin. HTTP native connections use literal loopback and require a local relay.

`mesimon mesophon setup` discovers the existing signed-in relay, checks its browser
endpoint and opens it, without touching trust stores or enabling/pairing boards.
`--check` omits browser launch. Chromium/WebKit acceptance exercises both HTTPS
and local HTTP, with certificate exceptions disabled for HTTP, and checks Host
and Origin rejection in addition to pairing, preview, prompt, reconnect and revoke.
Phone access still requires a reachable HTTPS relay with a browser-trusted
certificate; localhost setup applies only to the browser on the host itself.
