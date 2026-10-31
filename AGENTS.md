# Mesimon development contract

Mesimon is a Rust terminal kanban coordinating coding agents through a per-repository
daemon and private tmux server. This contract applies to Codex and Claude, including
crowns developing Mesimon. Implementation references are loaded only when relevant:
`docs/DEVELOPMENT.md` (architecture, measured traps, test infrastructure) by area, the
nested `CLAUDE.md` beside the code you change, `.claude/skills/adding-things/SKILL.md`
for the places a change touches, `.claude/skills/release/SKILL.md` for a release.

## Context and decisions

- Check `git status`; preserve user and concurrent changes.
- If `MESIMON_TICKET` is set and `get_ticket` is available, read the ticket before work.
  The person's prompts can refine its scope; a crown's empty description is not a blocker.
- **The code and its tests are the spec for what the product does.** A failing test is
  changed only by the decision that changes the behaviour, never to make a run green; the
  person's request and `README.md`'s promises (full text in `docs/PROMISES.md`) say what the
  intended outcome is, and changing a promise or an authorization boundary needs an explicit
  decision. *Process* rules — what to run, how often, how much to verify — are different:
  reevaluate a burdensome one by its original failure, current evidence, cost and a smaller
  effective alternative, and say so.
- Search `docs/STALE-MAP.md` for the affected feature or ticket and read the matching blocks;
  do not load the whole record. **Append a block when you ship something**: what changed, what
  was refuted and why. That is how a decision survives; routine edits need no essay.

## Where you are running

- **A branch starting with `msmn/` is a ticket worktree** cut by mesimon, with `MESIMON_TICKET`
  and `MESIMON_WORKTREE_BRANCH` set. Work and commit on that branch; **commit when done**
  (uncommitted changes cannot merge and block cleanup). Never `git checkout main`, never merge
  or push to main yourself: the person merges through mesimon, fast-forward only, so a green
  rebased branch is the deliverable. A rebase, when asked, is onto the default branch, then
  `cargo ut` + `cargo build --workspace`, plus an area's e2e only when a resolved conflict was
  in that area's code. The worktree is the ticket's workspace setting: never relocate the
  session with an agent's native worktree command (Claude Code's `EnterWorktree`).
- **You may be inside mesimon** (a pane the daemon this repo builds spawned). `pkill -f
  "mesimon daemon"` does not kill you and loses no state. Never `cargo run` in your pane: it
  wedges a nested fullscreen client. Verify with tests and goldens; the person drives the TUI.
- Rebuilds do not update a running daemon: press `U` in the TUI (clean handover) or stop only
  the daemon under test, never every mesimon socket or a broad process-name match. Say so
  before opening Docker Desktop or any GUI app.

## Boundaries

- The daemon's main thread alone mutates board state; async inputs arrive as `Msg` variants.
  **The tick forks nothing on the writer** (T-857): a tmux or `ps` read a stage wants is a
  `want_*` on `panes::Query`, absorbed from `Msg::PaneSampled`; a git read rides a worker.
  Mutations cross `authorize()` with a real principal, action and resource. The MCP shim is an
  untrusted transport; authorization and persistence stay in the daemon and core.
- Keep `mesimon-core` free of I/O. Daemon git calls use `crate::git::git(repo)`. Text from a
  person or an agent crosses `scrub_cells` (drawn) or `scrub_text` (leaves the process); never
  write a sanitizer. Every new persisted field is `#[serde(default)]`.
- Preserve the README's write allowlist and conversation ownership; `doctor` diagnoses and
  changes nothing. Agent-facing text states what the board does and what is the person's,
  never how to work (T-930); the disclosed startup brief is `core/brief.rs`.
- Paid relay internals and weaknesses belong in the private `mesimon-relay` repository, never
  here. Relay changes use their own worktree. A client contract change needs compatible relay
  validation; merely touching a client file does not require an empty relay branch.
- Do not publish, release, push or merge to main as a side effect of development. Public text
  (README, site, docs, release notes) is about mesimon, never its author.

## Verification proportional to the change

Choose checks from the behaviour and dependencies changed, and say what each proves.

- Iterate with the smallest relevant test. A Rust change finishes with `cargo ut` plus the
  `<area>_e2e` binaries the diff can reach. Docs-only changes need link and diff review;
  Python or web changes need their own checks, not a Rust workspace run.
- Run the full workspace suite once, at the end, when the diff changes the daemon's shared
  lifecycle, the wire (`core/src/command.rs`, `core/src/mcp.rs`), a state file's schema,
  authorization, the tmux backend or the e2e harness. A TUI-, docs- or `ci/`-only diff never
  needs it. Release verification is the complete suite on both Claude transport roads.
- Once the necessary checks pass, finish. Repeat only for changed inputs, a failure or a stated
  hypothesis. A flaky failure is not proof of a defect and a green retry is not proof of its
  repair; name every rerun. Report unrelated failures separately.
- Prefer deterministic state/clock tests for permutations and real-process tests for transport
  and representative end-to-end paths. New tests should discriminate a meaningful failure.
- **A change to what an agent or the crown is told or does** — tool text, `get_ticket`'s
  answer, the brief, a wake, a turn road — gets a rig case (`ci/rig/tests.toml` for Claude,
  the Codex rig's cases) and one live run on the affected provider at the tier the board really
  uses: crowns on Opus- or Astra-class models, never tuned on a budget model, whose results
  establish behaviour on that model only. A stub proves the wire, not that a model finds the
  words. A cut to crown text is judged blind, before and after (`ci/rig/README.md`). Never on
  every build or release; only when such behaviour changes.

Process-owning tests run under `python3 -B ci/test-run.py -- <command>` for deadlines, cleanup
and disk checks. Checks run once by default; `--both-roads` adds the mod pass to a full
workspace suite and is required for a release stamp. `MESIMON_TEST_ROAD=mod` targets one road.

```sh
cargo ut                                              # every unit test, ~1 s: the inner loop
cargo test -p mesimon-core attention                  # one module
python3 -B ci/test-run.py -- cargo test -p mesimon --test hook_e2e   # one e2e; skips without tmux
python3 -B ci/test-run.py -- cargo nextest run --workspace           # everything, e2es in parallel
cargo clippy --workspace --all-targets -- -D warnings # the release gate's clippy
MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui     # remint goldens after a deliberate visual change
python3 -B ci/rig.py --only R3                        # Claude rig, in a ticket worktree
python3 -B ci/rig.py --provider codex --live --only delegate --model <model> --effort <effort>
ci/test-linux.sh                                      # the suite on Debian in Docker
python3 -B ci/release-watch.py                        # on a claude/codex version change
git diff --check
```

The Codex rig owns disposable boards and runs from any checkout; the Claude rig uses a
persistent board and requires an `msmn/` worktree. Neither replaces checking the worker's
actual deliverable. Report checks run, failures, reruns and skipped coverage.

## Rendering and the web client

Exactly one saturated colour on the board, reserved for needs-you; preserve the geometry and
colour laws, update goldens only for an intended visual change and inspect the diff. A moving
TUI change is plan-first, with frames the person approves. Mesophon is served without a build
step under a no-inline-style/script CSP: `web/mesophon/CLAUDE.md`.
