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
defaults to 600000 observed total tokens across cases, including cached input;
manifests also retain input, cached input, output and reasoning token readings.
The runner checks native
usage during observation and before further submissions. Usage can arrive late
and native overhead can be unreported: this is a stop threshold, not a hard token
or dollar cap. `--case-timeout` defaults to 180 seconds and the automatic outer
supervisor caps the entire command at 20 minutes. A failed or inconclusive
case stops the suite; remaining cases are recorded as `not_run`, and `--failed`
selects them too, preserving the previous model, effort and limits unless explicitly
overridden. No failed case is retried automatically.

Extended cases are opt-in:

```sh
python3 -B ci/rig.py --provider codex --live --only guard,plan,plan-launch,interrupt,steer
python3 -B ci/rig.py --provider codex --live --only child,stop,compact,identity,question
python3 -B ci/rig.py --provider codex --live --suite extended --max-turns 25
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

Latest results are `target/rig/codex/verdicts.json` and `verdicts.md`. Every run
also retains its own directory with per-case manifests, board snapshots,
checkpoints, terminal observations and owned native histories. Treat these local
artifacts as private. Model usage and cleanup outcomes are independent of the
scenario verdict; missing evidence and cleanup failures never count as passes.
The historical compatibility manifest is not automatically certified by a smoke
run on a newer CLI.

Offline runner tests: `python3 -B ci/codex_rig_test.py`.
