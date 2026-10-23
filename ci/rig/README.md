# Real agent verification

`python3 -B ci/rig.py` runs the Claude crown/worker rig. Codex has a separate
native adapter behind the same entry point:

```sh
python3 -B ci/rig.py --provider codex --lay
python3 -B ci/rig.py --provider codex --live
python3 -B ci/rig.py --provider codex --live --only mcp,sleep-resume
python3 -B ci/rig.py --provider codex --live --failed
```

Run in a ticket worktree on an `msmn/` branch. The live command builds Mesimon
(unless `--no-build`), enters `ci/test-run.py` automatically, and runs each case
sequentially under the existing cleanup supervisor. Python 3, tmux, Codex and an
existing file-based Codex login are required. The fixture copies authentication
into a private disposable Codex home; it does not log in or copy personal
configuration, plugins or conversations.

The default smoke suite covers completion and column movement, scoped MCP,
additive brief and exact wake, queued prompts, sleep/resume across a provider
switch, and daemon handover. Its scenario allowance is ten turns. `--lay` lists
the selection without building, creating a board, accessing credentials or
calling a model. Each live case prints an external-terminal command for watching
its disposable board. Fixtures are reaped after each case, including failures;
Codex does not use Claude's persistent rig board or its `--reset`/`--stale` flags.

The rig defaults to `gpt-6-luna` with low reasoning effort. `--model` and
`--effort` override those choices. Before any scenario submits a turn, the actual
authenticated app-server must advertise that exact model and effort through
`model/list`, including pagination. There is no automatic model fallback. The
rig also checks the model and effort recorded in actual native turn contexts.
Both ordinary and plan-mode effort are pinned, along with child model/effort
defaults, using Codex's [configuration settings](https://learn.chatgpt.com/docs/config-file/config-sample).

`--max-turns` defaults to 12 and must cover the selected scenarios, including
their explicit child, continuation or compaction allowances. `--max-tokens`
defaults to 600000 observed total tokens across cases, including cached input,
raised to cover the selected crown cases (1M for `delegate`, 2M for `forge`,
each also that case's own stop);
manifests also retain input, cached input, output and reasoning token readings.
The runner checks native
usage during observation and before further submissions. Usage can arrive late
and native overhead can be unreported: this is a stop threshold, not a hard token
or dollar cap. `--case-timeout` defaults to each case's own (180 seconds, 300 for the crown cases) and the automatic outer
supervisor caps the entire command at 20 minutes. A failed or inconclusive
case stops the suite; remaining cases are recorded as `not_run`, and `--failed`
selects them too, preserving the previous model, effort and limits unless explicitly
overridden. No failed case is retried automatically.

Extended cases are opt-in:

```sh
python3 -B ci/rig.py --provider codex --live --only guard,plan,plan-launch,interrupt,steer
python3 -B ci/rig.py --provider codex --live --only child,stop,compact,identity,question
python3 -B ci/rig.py --provider codex --live --only delegate
python3 -B ci/rig.py --provider codex --live --only forge
python3 -B ci/rig.py --provider codex --live --suite extended --max-turns 33
```

These reuse the native gate, plan, interruption, child and lifecycle checks.
The plan-launch case starts a ticket in plan mode the way `^p` does and checks that
`/plan` took before the title, that the turn stopped on a plan, and that the board's
accept moved the same thread on. The steer case sends words into a running turn the way the board and Remote
Control's Steer do, and checks that the same turn replies to them; its manifest
records when the model read them against the running command's end.
Only the child case enables model-created child agents. The question case
(three turns) starts its worker from a crowned Codex seat kept busy on a `sleep`
turn, and the crown's `answer_agent` walks the real `request_user_input` dialog
by its keys, confirmed by the request leaving the runtime's snapshot (T-846);
the plan case is still answered through Codex's terminal. These checks do not
claim Claude mod-road parity.

The two crown cases (T-916) measure what a crowned Codex seat decides, not the
wire: the person's one request is in a person's words and names no tool, and
the verdict reads the board, the feed and the native histories, never the
crown's prose. Each starts a crowned seat on the checkout of a board whose
columns run Codex in workspace-write with on-request approvals, so the crown
could do the work itself with no dialog in its way; the board's tools and its
brief are on. The rig answers approvals as the person at the desk: a call to
a board tool, and a worker's plain `git` in its own worktree, are allowed once
each and listed in the manifest (`approvals`); anything else is left standing
and listed (`approvals_left`), and a crown kept waiting on one fails.

- `delegate`: the person asks for a small module and its test. It passes only
  when the crown's own session files a board ticket from its ticket (the
  ticket's `created_by` and `created_from`, and an agent's `create_ticket` and
  `start_agent` for it in the feed) and starts a Codex worker on it in that
  ticket's own worktree (the snapshot's worktree path is the worker's resolved
  cwd, away from the checkout), and nothing shows the crown doing the work: no
  native child (Codex's own child agents are on, `multi_agent`, its default, one
  thread, pinned to the selected model and effort), no `apply_patch`, no shell
  command that writes naming the work's files or the worker's worktree, and
  the checkout's HEAD and working tree where the request found them (a HEAD the
  board's own merge left there aside: a worktree's `merged_oid`, or the tip of
  a board worktree's branch, which is where a fast-forward leaves it and which
  names no `merged_oid`). Then the worker's turn ends, the board
  wakes the crown with it, and the crown's turn on that wake ends, the same
  no-implementation evidence holding over every crown turn. The crown's calls,
  the worker's files and the wake's cause are recorded, not judged.
- `forge`: the board merges through pull requests, the fixture repository has
  a github.com remote it never reaches, and the Claude rig's stub `gh` comes
  first on the daemon's PATH and on its login shell's (the forge looks for
  `gh` on the captured login-shell PATH first). The person asks the crown to
  start a ticket in its own worktree and look after its pull request, getting
  CI fixed if it fails. The start must be the crown's, in that ticket's own
  worktree. Once the worker's first turn and the crown's turn on its wake end,
  the stub opens PR 42 on the worker's branch, running, then fails its one job,
  `test-linux`. It passes when the board wakes the crown with the forge's
  news, the crown's `ask_agent` reaches the worker, the words the worker reads
  are the ask's own words and name the job or PR 42 and that it failed, and no
  turn of the crown's from the news on waits on the pull request (`--watch`,
  `gh run watch`, `sleep`, Codex's own sleep tool, or the same forge read such
  as `gh pr checks 42` run twice; one read to learn which job failed is no
  wait), patches, writes by shell naming the README, the worktree or the
  branch, or moves the checkout. Where the ask's words cannot be read off the
  crown's call, the case ends `inconclusive`, never passed.

The shell-write test reads a command's words: output redirected to a file
(not `2>&1`, `>/dev/null`, `=>` or `>=`), `tee`, a heredoc, `cp`, `mv`,
`touch`, `sed -i`, an interpreter's `-c`/`-e`, and git's writes. A write it cannot see (a script
that writes a file it does not name) is caught only by the checkout's HEAD and
tree, and in the worker's worktree, where the worker's own writes are expected,
not at all.

The crown cases' daemon runs on a HOME of its own inside the fixture. A
fixture daemon on the real `HOME` offers the person's machine tiers
(`$HOME/.local/state/mesimon/tiers.toml`) to a crown, which picks among them: a
first run's crown started its worker on the person's own Claude tier, whose
`claude` then stopped at its trust dialog. In the fixture's HOME only the built-in
tiers exist there, `claude` refuses to start, git has a fixture identity, no
personal Codex skills are listed, and the captured login shell is the rig's
PATH.

Each crown case allows one person's submission; what follows is the crown's
doing and the board's wakes, so each also stops at a count of native turns
across the fixture's threads, children included (20 and 24), and takes a
300-second deadline unless `--case-timeout` says otherwise. They do not prove
that a Codex crown delegates every request, nor that it merges, parks or
closes a delivered worker: one run on one model is one sample of a choice.

Latest results are `target/rig/codex/verdicts.json` and `verdicts.md`. Every run
also retains its own directory with per-case manifests, board snapshots,
checkpoints, terminal observations and owned native histories. Treat these local
artifacts as private. Model usage and cleanup outcomes are independent of the
scenario verdict; missing evidence and cleanup failures never count as passes.
The historical compatibility manifest is not automatically certified by a smoke
run on a newer CLI.

Offline runner tests: `python3 -B ci/codex_rig_test.py`.
