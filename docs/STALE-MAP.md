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
