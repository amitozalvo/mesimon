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
