# Mesimon development contract

Mesimon is a Rust terminal kanban coordinating coding agents through a per-repository
daemon and private tmux server. This contract applies to Codex and Claude, including
crowns developing Mesimon. Implementation references are loaded only when relevant.

## Context and decisions

- Check `git status`; preserve user and concurrent changes.
- If `MESIMON_TICKET` is set and `get_ticket` is available, read the ticket before work.
  The person's prompts can refine its scope; a crown's empty description is not a blocker.
- Code and tests describe current behavior, not proof that it is desirable. The person's
  request and the product promises define the intended outcome. Reevaluate a burdensome
  rule by its original failure, current evidence, cost and a smaller effective alternative.
- Search `docs/STALE-MAP.md` for the affected feature or ticket, then read matching sections.
  Do not load the whole historical record or repeat old experiments without a question.
  Record a consequential behavior/design decision briefly; routine edits need no essay.
- `README.md` and `docs/PROMISES.md` define product commitments. Changing a commitment or
  authorization boundary needs an explicit decision, not merely updating its tests.

## Boundaries

- The daemon's main thread alone mutates board state. Async inputs arrive as messages.
  Mutations cross core authorization with a real principal, action and resource. The MCP
  shim is an untrusted transport; authorization and persistence stay in the daemon/core.
- Keep `mesimon-core` free of I/O policy. Daemon Git calls use `crate::git::git(repo)`.
  Server and client must resolve the same private tmux binary.
- Preserve documented write boundaries and conversation ownership; doctor diagnoses
  rather than changing configuration. Agent-facing text describes mechanics and authority,
  leaving working method to the person. The disclosed startup brief is in core/brief.rs.
- Paid relay internals and weaknesses belong in the private `mesimon-relay` repository,
  never here. Relay changes use its own worktree. Client contract changes need compatible
  relay validation; merely touching a client file does not require an empty relay branch.
- Do not publish, release, or push/merge to main as a side effect of development.
  Commit finished ticket work on its branch; rebase an `msmn/` branch only when asked.

## Verification proportional to the change

Choose checks from the behavior and dependencies changed, and say what each proves.

- Iterate with the smallest relevant test. A Rust change normally finishes with `cargo ut`
  plus affected integration tests. Docs-only changes need link/diff review; Python or web
  changes need their own relevant checks, not an unrelated Rust workspace run.
- Run the full workspace suite once when changing shared wire/schema/authorization,
  cross-cutting lifecycle behavior or test infrastructure used by the whole suite, or when
  a concrete unresolved risk spans areas. A localized daemon edit alone is not a full-suite
  trigger. Release verification remains the complete suite on both Claude transport roads.
- After a rebase, verify affected code and resolved conflicts. Unchanged evidence remains
  useful; a documentation-only conflict is not a reason to rerun Rust tests.
- Once the necessary checks pass, finish. Repeat only for changed inputs, a failure or a
  stated diagnostic hypothesis. A flaky failure is not proof of a product defect, and a
  green retry is not proof of its repair. Report unrelated failures separately.
- Prefer deterministic state/clock tests for permutations, with real-process tests for
  transport and representative end-to-end paths. Avoid exact prose assertions unless the
  bytes are a public contract. New tests should discriminate a meaningful failure.
- Agent-behavior changes need an appropriate outcome evaluation. Use existing cases where
  they cover the change; add one for a missing outcome, not every wording edit. Scripted
  tool calls prove integration, not judgment. Select the affected provider and explicitly
  pin model/effort; respect the person's quota and budget. Budget-model results establish
  behavior on that model only. No automatic paid reruns until green.

Process-owning tests use `python3 -B ci/test-run.py -- <command>` for deadlines,
cleanup and disk checks. Checks run once by default. `--both-roads` explicitly adds
the mod pass to a recognized full workspace suite and is required for release coverage.
For a targeted mod check, set `MESIMON_TEST_ROAD=mod`; `--one-road` still means one pass.

```sh
cargo ut
python3 -B ci/test-run.py -- cargo test -p mesimon --test <area> <filter>
python3 -B ci/test-run.py -- cargo nextest run --workspace
python3 -B ci/rig.py --provider codex --live --only delegate --model gpt-6-luna --effort low
git diff --check
```

The Codex rig owns disposable boards and can run from any checkout. The Claude rig
uses a persistent board and requires an `msmn/` worktree. Neither is a substitute for
checking the worker's actual deliverable. Report checks run, failures and skipped coverage.

## Runtime and focused references

- Never launch a nested interactive board in a managed agent pane, sweep all Mesimon sockets,
  or kill by broad process-name match. Test with disposable boards. Rebuilds do not update
  a running daemon: use a deliberate handover before manual verification.
- A board-managed agent keeps its assigned workspace; do not use an agent's native
  worktree command to relocate it behind Mesimon's ticket/workspace setting.
- Keep build directories per worktree. Incremental compilation is an optional local tradeoff
  (`CARGO_INCREMENTAL=1`), not a universal rule; the runner requires 5 GiB free by default.
- For sandbox-denial symptoms, rerun only the necessary command with elevated permission;
  do not weaken tests or change personal agent configuration.
- `docs/DEVELOPMENT.md`: architecture, measured traps and test infrastructure; search by area.
- `.claude/skills/adding-things/SKILL.md`: change recipes; nested `CLAUDE.md`: area constraints.
- Rendering: preserve geometry/color laws, update goldens only for intended visual changes
  and inspect diffs. Mesophon is served without a build step under a no-inline-style/script
  CSP; see `web/mesophon/CLAUDE.md` and `vendor/README.md`.
- Release work: `.claude/skills/release/SKILL.md`; live-agent evaluation: `ci/rig/README.md`.
  Public text is about the product; keep private relay details out of public records.
