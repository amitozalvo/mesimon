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
- **Card anatomy** (`07` §4): the running/spawning/idle-other states render NO badge glyph —
  `07` §4.1's "glyph pair present only when abnormal" wins over `06` §10.3's gallery, which
  shows `▸`/`◦` on resting cards. Session liveness appears in the meta-strip dots instead.
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
