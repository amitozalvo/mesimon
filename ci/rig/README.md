# Real agent verification

## Checking a Claude case before launch

Run `python3 -B ci/rig.py --check` for the whole TOML suite, or add
`--only SE2,SP1` to check selected cases. Live runs apply the same check before
building or touching the board. Once connected, the runner checks upcoming
worker branch names, including a declared `start_title` for a ticket the crown
renames before launch. A collision reports the case and branch before filing.

- A scripted `call start_agent for {key}` produces a worker. For a start
  expressed in the person's words, or multiple starts in one step, list the
  targets in `starts`, using `key`, `key2`, etc., or `filed` for a new child.
  An omitted `until` waits for an idle worker on the primary ticket. Use
  `until = { crown_only = true }` for a crown read that starts nobody.
- Use `background_at_stop = true` when a worker should still have a task at
  Stop. The board reads that state as `working`. The checker also recognizes
  the measured foreground Bash sleep refusal in a crown prompt. Other shell
  permissions and model decisions are observed on the live run.
- For an exact count, declare `wake_counts = "delivered"` for `wake_count`
  (words in the crown's transcript) or `"owed"` for `woke_once` (causes in
  the board feed).
- Worktree starts declare `merge_guard = "train_off"` or `"fixture"`. The first
  requires the board's train to be off. The second selects a ticket-specific
  merge branch with independent history and its own checkout. A `train_on`
  step can set `block` to exercise a refused fast-forward or `notice = true`
  to land a commit and tell the worker. With `hold = true` the crown owns
  that ticket's merge while the train stays armed.
  The uncrowned and rebase fixtures already own their merge targets.

These fields describe the fixture to the runner. Only the existing prompt,
brief and verdict fields reach the model. The real-tier live run still measures
changes to agent behavior.

## Merge notices

Run `python3 -B ci/rig.py --only MN2` and `python3 -B ci/rig.py --only MN3`
separately for a fresh Opus crown in each case. Both workers verify the
landed commit in a notice turn. A file in the worker's private Git directory
lets the rig release that turn after observing the crown.

| Case | Who lands the commit | What the rig reads |
| --- | --- | --- |
| MN1 | Person | The crown reports the landing while verification runs, then reports the finish. |
| MN2 | Train | The worker commits on the crown's ask. The crown receives the ask-ended wake and reads `merge_state: merged` with a keyed `get_ticket` during the notice turn. |
| MN3 | Crown | The held ticket's `merge_ticket` receipt reports `outcome: merged` and `notice: sent`. |

MN2 and MN3 check the fixture branch's tip against the worker's commit.
After release, the worker's Stop must precede the finish prompt in the crown's
transcript, and the crown must answer it. The fixture disarms the train and
holds the worker before restoring the board's merge target.

## Linked crowns

`python3 -B ci/rig.py --only X` selects the Claude crown communication cases.
They use one real crown and a linked board with a stub crown; the peer costs
no model tokens. They do not claim coverage of two models reasoning together.

- **X1:** a request for the other board's work reaches its crown's pane;
  its reply arrives as an attributed prompt and the crown reports the answer.
- **X2:** an unsolicited request arrives from the peer. The board's own
  prompt supplies the return route; the rig's brief does not name the tool.
  The crown's answer must reach the peer through `ask_board`.
- **X3:** a closed linked board returns the actual `not open` refusal and
  the crown raises its hand for the person.

Each case requires exactly one `ask_board` call and forbids starting the
peer's work locally. X1/X2 check both receipts' ask/reply classification,
the destination crown, attributed prompts and actual peer-pane delivery.
A fresh reference supplied only by the peer must survive the answer, so
an old answer or a receipt alone cannot pass. A second call fails, including
an acknowledgement that starts a reply loop. X1 waits for delivery before
the stub answers. Blind test cards omit acceptance checks as well as expected
behavior, so reading the card cannot supply the intended tool choice.

No-model checks:

```sh
python3 -B -m unittest ci/rig_test.py
cargo build -p mesimon
python3 -B ci/test-run.py -- python3 -B ci/rig_peer_test.py
python3 -B ci/test-run.py -- cargo test -p mesimon --test tools boardlink_e2e::
```

`rig_peer_test.py` exercises the rig's actual peer fixture against another
shell stub in a disposable HOME. The peer paints the current composer and
title and emits a completed turn before it is linked: merely queueing a
message to a stub stuck at startup is not delivery. Both daemons, their private
tmux servers and fixture directories belong to the cleanup supervisor.

`boardlink_e2e` also covers authorization, supervised holding, queue acknowledgement,
missing crowns and unlinking with two real daemons and stub sessions. These
checks exercise transport and the rig's verdicts; a live `--only X` run is
still needed to measure the model's choices on a particular build.

## Native Codex rig

`python3 -B ci/rig.py` runs the Claude crown/worker rig. Codex has a separate
native adapter behind the same entry point:

```sh
python3 -B ci/rig.py --provider codex --lay
python3 -B ci/rig.py --provider codex --live
python3 -B ci/rig.py --provider codex --live --only mcp,sleep-resume
python3 -B ci/rig.py --provider codex --live --failed
```

Run from any Mesimon checkout; Codex uses disposable repositories, not the
Claude rig's persistent board or branch cleanup. The live command builds Mesimon
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

`--binary /path/to/mesimon` evaluates an already built binary without rebuilding.
Use it to compare baseline and candidate binaries on the same rig and model;
the manifest records the evaluated binary's hash. `--no-build` alone keeps using
the checkout's `target/debug/mesimon`.

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
raised to cover the selected crown cases (1M each for `delegate` and `restart-idle`, 2M for `forge`,
each also that case's own stop);
manifests also retain input, cached input, output and reasoning token readings.
The runner checks native
usage during observation and before further submissions. Usage can arrive late
and native overhead can be unreported: this is a stop threshold, not a hard token
or dollar cap. `--case-timeout` defaults to each case's own (180 seconds, 300 for the crown cases; at most 900, which Sol's
`forge` needs) and the automatic outer
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
python3 -B ci/rig.py --provider codex --live --only restart-idle --model gpt-6.2-astra --effort high
python3 -B ci/rig.py --provider codex --live --only read-other
python3 -B ci/rig.py --provider codex --live --suite extended --max-turns 41
```

These reuse the native gate, plan, interruption, child and lifecycle checks.
The `restart-idle` case waits for the crown to hear a worker finish, withholds that
worker's heartbeat file until the board reports observation loss, then restarts the daemon.
It checks that the board sees the same completed turn and leaves the crown quiet.
The plan-launch case starts a ticket in plan mode the way `^p` does and checks that
`/plan` took before the title, that the turn stopped on a plan, and that the board's
accept moved the same thread on. The steer case sends words into a running turn the way the board and Remote
Control's Steer do, and checks that the same turn replies to them; its manifest
records when the model read them against the running command's end.
The read-other case (T-1089) names another ticket's key and note id in a worker's prompt:
the reply must carry the code word only that note holds, and the worker's keyed
`write_note` must leave the note as it was.
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
a board tool, and a worker's plain `git` in its own folder (its worktree, or
the checkout for a worker on the shared checkout; plain shapes joined by `&&`,
as Sol writes `git add`, `git commit` and `git status`, count as one, inside a login
shell's `-lc` too, with paths and messages that hold nothing a shell expands), are
allowed once each and listed in the manifest (`approvals`); any other request of a
worker's (a push, the forge's CLI, the network) is declined with the dialog's own
Esc, as the person at the desk would, and listed (`declined`, T-930: Sol's forge
worker pushes and calls `gh` to open its PR, and the fixture reaches no forge); anything else is left standing
and listed (`approvals_left`), and a crown kept waiting on one fails.

- `delegate`: the person asks for a small module and its test, on a ticket of
  its own by its own agent (T-930: whether to delegate is otherwise the crown's
  judgement, so the outcome is in the person's words). It passes only
  when the crown's own session files a board ticket from its ticket (the
  ticket's `created_by` and `created_from`, and an agent's `create_ticket` and
  `start_agent` for it in the feed) and starts a Codex worker on it, the one
  agent seat on that ticket. Where it works is the crown's choice, since the
  request names none and the crown is offered both (T-929): in that ticket's
  own worktree (the snapshot's worktree path is the worker's resolved cwd,
  away from the checkout), or on the shared checkout (the worker's cwd is the
  checkout and no worktree is bound to the ticket). Nothing may show the crown
  doing the work: in the crown's own calls no native child (Codex's own child
  agents are on, `multi_agent`, its default, one thread, pinned to the
  selected model and effort), no `apply_patch`, no shell command that writes
  naming the work's files or the worker's worktree; and no move of the
  checkout's HEAD or working tree that is the crown's (a HEAD the board's own
  merge left there aside: a worktree's `merged_oid`, or the tip of a board
  worktree's branch, which is where a fast-forward leaves it and which names
  no `merged_oid`). Then the worker's turn ends, the board
  wakes the crown with it, and the crown's turn on that wake ends, the same
  no-implementation evidence holding over every crown turn. The crown's calls,
  the worker's files and the wake's cause are recorded. The delivered module
  must return the requested greeting for ordinary, empty and Unicode names;
  its unittest file must contain tests and pass. This independent check runs
  in a supervised child with a 15-second bound. Earlier runs without a
  `delivery_check` record prove delegation only, not implementation quality.
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

Which checkout moves are the crown's is read by time (`crown_changes`; the
Claude rig's `crown_wrote_nothing` reads it the same way). The rig looks at
the checkout on every pass of its loop (HEAD, and each `status` line with a
digest of the file's bytes, so an edit to a file already dirty is a move too),
noting each time whether the crown and a worker it started are in a turn. A
move between two looks is the crown's, except when the crown was out of a turn
at both looks and a worker was in one at either: a worker on the shared
checkout writes and commits there, and those moves are its own. The limits: a
move made while both are in a turn counts against the crown, so a worker that
writes before the crown's turn on the request has ended fails the case (the
case relies on the crown ending that turn seconds after `start_agent`, before
a fresh worker's first write); and a process the crown left running that
writes while only the worker works reads as the worker's.

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

Measured so far (2026-10-23, Codex CLI 0.163, effort low, one run each, no
retries; STALE-MAP's T-916 block has the details):

| case | model | verdict | tokens | the crown's calls |
|---|---|---|---|---|
| `delegate` | `gpt-6-luna` | failed | 91,340 | `get_ticket`, then `apply_patch` writing `greet.py` and `test_greet.py` in the checkout; no worker |
| `forge` | `gpt-6-luna` | passed | 579,292 | `start_agent`; on the forge's wake `get_ticket`, `ask_agent` naming PR #42 and `test-linux` |
| `delegate` | `gpt-6-sol` | passed (T-929's rule) | 123,297 | `get_ticket`, `list_board`, `create_ticket` (T-2, `workspace: "shared_checkout"`), `get_ticket` T-2, `start_agent` T-2 on `shared_checkout`; no write of its own |

Sol's run was graded failed when it ran, because the case then asked for the
ticket's own worktree. The person judged it a pass ("sol did ok", 2026-10-23):
the workspace is the crown's choice, and the case no longer grades it (T-929).
Luna's `delegate` stays a fail under the same rule, because it wrote the code
itself. One rerun on Luna under the new rule (Codex CLI 0.164, low, 95,987
tokens) failed the same way: `get_ticket`, then `apply_patch` adding
`greet.py` and `test_greet.py` in the checkout, and no worker. Sol's was not
rerun. `forge` was not run on Sol.

Latest results are `target/rig/codex/verdicts.json` and `verdicts.md`. Every run
also retains its own directory with per-case manifests, board snapshots,
checkpoints, terminal observations and owned native histories. Treat these local
artifacts as private. Model usage and cleanup outcomes are independent of the
scenario verdict; missing evidence and cleanup failures never count as passes.
The historical compatibility manifest is not automatically certified by a smoke
run on a newer CLI.

Offline runner tests: `python3 -B ci/codex_rig_test.py`.

## agy rig (T-1232)

```sh
python3 -B ci/rig.py --provider agy           # startup cases, no model call
python3 -B ci/rig.py --provider agy --only worktree-trust
python3 -B ci/rig.py --provider agy --live    # the brief case on agy's own default model
python3 -B ci/rig.py --provider agy --live --model gemini-4-neon-high
python3 -B -m unittest ci/rig_agy_test.py     # the verdict, offline
```

One disposable board
per case under `ci/test-run.py`; agy keeps the person's own home and sign-in, so
a live brief case leaves one conversation in agy's list. Each fixture provides
its own trust list through `MESIMON_AGY_HOME`. The brief fixture trusts its
checkout. The worktree-trust fixture leaves its checkout untrusted and reads
the checkout path from the board and `get_ticket` while agy awaits a decision.
The `--only sign-in` fixture gives the installed agy a fresh home and reads
its login stop without sending a key. The Codex rig's `--only agy-sign-in`
case asks a live crown who can complete that step.
An explicit `--model` selects a tier on the disposable board. With no selection agy uses its default.
The verdict records the model from the transcript.

Without `--live` the brief case checks that agy lists mesimon's tools: agy starts an
MCP server only at the first prompt or when `/mcp` lists them, and `/mcp` calls
no model. The live `brief` case submits the prefilled title directly on a
fresh seat. The transcript must show that title alone, the injected brief,
and the native `mcp_mesimon_get_ticket` as the first tool call. The answer
must carry the nonce from the ticket description without a permission prompt.

On agy 1.5.13, a passing `/mcp` check and a failing cold `brief` case can
occur with the same workspace configuration. Interactive agy does not start
workspace MCP servers while waiting at its prompt box; it spawns them lazily
when `/mcp` opens or when the first user message is sent, and on that send it
resolves the first turn's agent config without blocking (`blocking:false`), so
holding submit cannot warm the server either. The first request can therefore
omit even an eager `get_ticket`. Opening `/mcp` before submitting the live
title would warm the server and invalidate the cold-start check. [Antigravity CLI 1.1.9 changelog](https://antigravity.google/docs/changelog?tab=cli#:~:text=Improved%20interactive%20startup%20so%20a%20slow)

## Judging crown text (T-930)

A change to what the crown is told is judged by what a crown on its real tier
decides, before and after the change. The method is the same for worker text,
with workers on their own tier.

- **The crown's tier.** The Claude rig's crown runs on its own board tier,
  `rig-crown`, by default `claude-opus-5-5` at high effort (`--crown-model`,
  `--crown-effort`). Workers stay on `rig`, Sonnet at low effort. A crown whose
  transcript names another model stops the run. Codex crowns run with
  `--model gpt-6-sol`. That fixture pins one model for every seat, so its
  workers run on Sol too. A crown on Sonnet or Luna is no evidence for crown
  wording.
- **Blind tests.** Most Claude cases script every call (the rig's crown brief
  says to make exactly the calls named), so they measure the wire, not the
  text. A case that judges the crown's own choice is `blind = true`: the
  crown's brief, the test card and the ticket's title carry nothing of
  `expect`, and the brief's scripting rules (no hand, no file, the one line on
  a wake) are lifted in it, archiving aside. Without the flag, the crown is told
  each test's acceptance up front and does only what the steps name.
- **Fresh crown per sample.** One `ci/rig.py` run is one crown conversation,
  so each sample of a case is its own run (`--no-build --only <id>`). A later
  test in the same run would read an earlier one's turns.
- **Two arms, fixed in advance.** Arm A is the text before the change and arm
  B the text after it, on the same rig. Each case's sample count is set before
  the first run, three for a case that a cut puts at risk. Every run is
  reported. A regression is a drop of two or more passes out of three. N = 3
  sees only large effects. A rig fix made during the runs applies to both
  arms, which then run again under it: a run kept from before the fix is
  reported raw and apart, never regraded into the comparison.
- **Attribution.** One arm with every cut. A case that regresses is bisected
  alone, one cluster of cut sentences restored at a time. No words are added
  back to make a case pass.
- **Measures.** Each verdict note and the run's table carry what the crown
  spent: its model, turns, tool calls (how many failed, and how many came
  before its first start), tokens (input, cache read, cache write, output),
  the bytes of every result and of each result that carries the crown's rules
  wherever they ride, and the bytes of each `start_agent` and `ask_agent`
  receipt. The worker's model is in the note beside them. A Codex run's
  manifest carries `models` and `crown_results` (results, bytes, the rules'
  results, failed scripts) beside its token usage.

## Native compaction (T-1069)

`python3 -B ci/rig.py --only CX2,NX1,NX2` measures a typed `/compact` and what the board does
around its compaction window, on real Claude Code with the real mod, the crown on Opus and its
workers on Sonnet. NX1 and NX2 put the daemon and every launch, the crown's included, on the native
road (T-658): `crown_road` with road `native` runs Claude Code through a wrapper whose `auth status`
says an enterprise account, so the probe's own verdict is classic-off. CX2 is the ordinary mod
road's baseline.

- **NX2** is that native road as it stands on an unmanaged account: Claude Code still hands the mod
  `session.compact`. Its frames are `PreCompact`, `SessionStart{compact}`, `PostCompact`, once each.
- **NX1** is a **simulation** of a seated security default: `passed_over = true` loads a copy of
  the mod whose `session.compact` hook passes the event on unread. Since T-1297 nothing then reports
  the compaction, the typed `/compact` included (T-1069's `command.run` fallback left a managed seat
  reading compacting long after it ended), and NX1 checks that: the worker's transcript shows the
  manual boundary (`await_compaction`), no compaction frame came (`compaction_frames:none`), its
  record never read compacting on the way (`never_compacting`), and it answers the crown's next ask.
  A person's plugin cannot stand in for the real seat (`next.to` loads only in a managed plugin),
  and the public build ignores `CLAUDE_CODE_MANAGED_SETTINGS_PATH`; no managed machine is involved,
  and nothing here proves what a managed machine does beyond what its binary's table says.

NX2 checks that a person's prompt queued once the worker's `PreCompact` came is sent by the board
while the window is still open (`sent_in_compaction`, T-1297: no road holds words for a compaction)
and that Claude Code runs it once, after the `PostCompact` (`held_by_compaction`, which fails a
prompt queued outside the window as unexercised), that the worker answers after the compaction, and that the crown's own
`/compact` gives its seat the same frames (`crown_rules_after_compact`: the last board result before
the boundary carries no rules and the first after it does); NX1 that doctor's road line warns of compaction
(`compactions unseen` on a release read as passing it over, `compaction coverage unknown` past the
last one read). An automatic compaction is not measured: it cannot be triggered on purpose,
and on such a road it reaches neither the mod nor the board.
