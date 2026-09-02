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
  behind a one-entry (len, mtime) cache (`tui/src/peek.rs`) — steady cost is one `stat`
  per frame for the single peeking card. Text is sanitized before render: control chars and
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
`›`/`◊` were rejected; the row's detail says `your pick for a dark terminal`. The menu row sits
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
