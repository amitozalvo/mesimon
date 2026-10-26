# crates/mesimon-daemon

Loaded when a session works under this directory; the root `CLAUDE.md` still applies.

## Attention flow and the mod road

**Attention flow (M2).** Claude sessions spawn with `--settings <state>/hooks/<uuid>.json`, a
32-entry generated hook set (`daemon/src/hook_settings.rs`; its unit tests encode Claude Code's
silent-failure traps). Thirty-one entries exec `mesimon hook`, a **pure observer** that reads
stdin to EOF first, **never writes stdout** (stdout is injected into the agent's context), exits
0 always, and self-aborts at 500 ms. `daemon/src/ingest.rs` distills frames into `Signal`s;
`core/src/attention.rs` is the pure, time-injected state machine (fixed precedence ranks 0–16,
ranks 0–8 the attention set; enters at 0 ms, leaves after a 1500 ms settle, 15-min stale
demote). **No polling for exits** — tmux's `pane-died` hook is the only exit signal, with a 15 s
server-alive guard for wholesale tmux death. **A death frame names its pane** (`pane` in the hook
header: `<server pid>:<pane id>`, from `--pane` or the pane's own `TMUX`/`TMUX_PANE`) and the
record keeps the key it spawned (`SessionRecord.pane_key`); a wake reuses the session name and
the session uuid, never the pane, so a death frame from another pane is dropped
(`straggler_death`) and no tmux is asked. A frame or record without a key is trusted. After a daemon restart our sessions sit at
`Unknown{DaemonRestarted}` and re-derive from the transcript tail at Low confidence until a hook
re-asserts; reconcile never trusts stale claims. **One thing on that tail is not a claim** (T-663):
a turn the last daemon saw open whose close the transcript dates after that spell began is the
turn's end, at Medium (`Signal::TranscriptTurnEnded`, `recovery::turn_closed_in_the_gap`), so
`on_done` and the crown hear a turn whose `Stop` fell into the restart's window — `mesimon hook`
fails open, so that `Stop` was lost, never queued. An undated close, or one older than the spell,
stays the Low hint.

**The mod road carries a Claude session alone (T-577), and the road is `auto`, never a setting
(T-588).** A Claude launch on a Claude Code ≥ 2.1.287 whose `claude plugin validate` passes (the
probe, cached per binary in `<state>/mod/probe.json`, warmed when the shell env lands) loads the
mod laid at `<state>/mod/<version>-<digest8>/` and passes **no `--settings`, no `--mcp-config`
and no `--allowedTools`**: the mod relays the hook set's events in order through `mesimon hook
--road mod` (`Msg::ModHook`, ingested where `SessionRecord::frames_by_mod`), refuses structured
writes itself (the gate, below), runs `mesimon approve` from `classic.PermissionRequest`, and
registers the board's tools with `$.tool.register`, serving each through `mesimon mcp --call`.
A registered tool asks no permission in any mode, so the daemon's tier check is the only gate.
A mod-alone launch with no `SessionStart` twice the bridge wait after its spawn has its words
armed on the paste road (`rescue_silent_mods`, feed `mod_silent`): no hook set would ever send one.
**`auto` means the mod only where it is proven to load (T-598):** the probe's third step is
`claude plugin test` on a one-test folder (`modroad::LOAD_PROBE`), whose refusal is a `ModsOff`
verdict; and under `auto` a silent mod launch whose bridge never polled is relaunched on the hook
set first (`relaunch_silent_mods`, feed `claude_road_relaunch`), once per record per daemon life,
writing `ModsOff` too. `ModsOff` is asked again at the next daemon start and after 6 h. **And only where its hook
events arrive (T-650):** a Team or Enterprise account, or managed settings, seats Claude Code's
`cc-plugin-sec-default` outermost, whose `classic.*` hook hands every classic event past a
person's plugins, so the mod loads, serves the tools and takes a `submit` and never hears
`SessionStart`. The probe's fourth step reads it where it can: `claude auth status --json`'s
`subscriptionType` (`team`, `enterprise`) or a managed-settings file on the machine is
`ClassicOff` before any launch, and every launch then is **native** (T-658): the mod with
`MESIMON_MOD_NATIVE=1`, which relays from Claude Code's own events in the hook set's names, and
`--settings` naming a one-entry file, `PermissionRequest` alone (`HookSet::PermissionOnly`;
`SessionRecord::native`, so `frames_by_mod` holds with `--settings` on argv and `on_hook` takes
that one hook-set event). What the probe cannot see (remotely served managed settings) a launch
finds: a mod launch whose bridge polled and whose `SessionStart` has not come a bridge wait later
is relaunched the same way and writes the same verdict, same clock as `ModsOff`. A `submit` the mod reports `entered` clears `unsent`: the agent
has the words.
**The mod's half of that road (T-657)**: under `MESIMON_MOD_NATIVE=1` the mod's `classic.*` relays
go silent and the same frames, by the hook set's names and shapes (`road::NATIVE_EVENTS`), are
built from `session.*`, `turn.*`, `tool.call` and `agent.spawn`, which the security default lets
through; the `Stop`'s task list is the one adapter in the mod, a `StopFailure` comes with
`error: unknown` and `native: true` (the daemon reads the class off the transcript, T-659), and
`PermissionRequest` is that one-entry hook set's, never a `tool.check` the mod holds.
The hook set, `mesimon gate` and `mesimon mcp`'s server stay whole for an older Claude Code and
Codex. The daemon talks to the mod only through `mesimon mod-bridge`'s long poll (`ModNext`),
never a push from the writer thread. The mod spells nothing on promise 3's never-list (a unit
test scans it); its two `deny`s are the gate's words and a board tool's refusal.
**The turn roads ride it (T-575):** for a session whose mod speaks `submit` (`mod_speaks`, from
the bridge's `--speaks`), every prompt goes down as `$.prompt.submit({ text, asUser: true })`
and a question's answer as one `answer` frame, confirmed by the mod's `ModAnswer` report alone
(the engine fires no `PostToolUse` for a dialog the mod closed). Launch words wait for the
pane's `SessionStart` AND the mod's first poll; the paste road, the composer wait and the
dialog keys stay for the hook set, Codex, a mod that never polled or an older one, a refusal,
and the plan accept (T-579). **Each turn's end goes up as `ModUsage` (T-581)**: the engine's
count of the turn and the main loop's rate-limit windows. A mod session's ticket is counted from
those reports past a per-conversation fence, the transcript read beside them into a check and
never the hours (`cost::Ledger::count_mod`); the windows merge into `usage.json`.

## Traps that were measured

- **Claude Code 2.1.287 wraps a launch's pasted brief in `<pasted_content>`** and follows
  instructions inside only where the person's own words (the typed title) ask: a worker given a
  bare title declined a four-step brief (T-588). A one-line paste alone arrives as plain text.
- **`$.prompt.submit` refuses a text that begins with `/`**, leading blanks included ("would run
  a command as the user", 2.1.287): a slash command keeps the paste road on the mod road too
  (`road::is_command`, T-575).
- **Claude Code's remote flag `tengu_plugin_hooks_modules` turns mods off with nothing local
  changing** (2.1.288): `claude plugin validate` passes and `--plugin-dir` is taken, and the mod
  never loads. `claude plugin test` refuses in its words ("hooks modules are turned off"). No
  local override exists in the public build; `DISABLE_GROWTHBOOK=1` reads every flag's default,
  this one's on, and is the rig's pin alone (`MESIMON_RIG_NO_FLAGS`). Never write `~/.claude.json`.
- **Claude Code seats its `cc-plugin-sec-default` outermost on a Team or Enterprise account, or
  under managed settings** (2.1.289, T-650), and that plugin's `classic.*` hook is one line,
  `next.to(e, "append")`: a person's mod, `--plugin-dir` included, gets `session.start`,
  `tool.call` and `turn.complete` and never a classic hook event. `claude plugin validate` and
  `claude plugin test` pass all the same; the feed shows `ModUsage` and `ModSubmit` and no
  `SessionStart`. The seating rule is `qjr` in the binary; `allowManagedHooksOnly` and
  `disableAllHooks` are a different thing (the module is not loaded at all).
- **A mod's `submit` fires `UserPromptSubmit` as a typed prompt does, but an answer the mod
  returns in a dialog's place fires no `PostToolUse`**: the mod's `ModAnswer` report is that edge.
- **A `$.prompt.submit` made mid-turn resolves only when its words are consumed** (2.1.309,
  T-934): held to the running turn's end and run as a turn of its own, its `entered` is that
  turn starting, an hour later in T-930's case. A crown ask down the mod reads `sent` until then
  (`crownask`, T-938); a composer Enter's `UserPromptSubmit` is its enqueue, not its absorption.
- **Claude's `prompt_id` names the turn, not the words** (2.1.310, T-983): it is on every hook
  frame and is every `user` record's `promptId`. Words typed mid-turn fire `UserPromptSubmit`
  under the running turn's id, and a prompt's own record lands 80–180 ms after its hook.
- **A mod's `$` call in flight fails with its dispatch when that dispatch is abandoned**
  (interrupt, a hook above settled first, budget), and the first read rides the first event's:
  the mod keeps a value only once read whole and never caches a failed read (T-594), and no
  event awaits another event's read: a shared in-flight read fails with the dispatch that began
  it (T-577's three starts at once left two mods silent).
- **A `tool.call` hook that throws is skipped** (2.1.287, T-577): a built-in tool then runs
  unjudged, and a registered tool reads to the model as "Claude requested permissions to use …
  but you haven't granted it yet". A gate or a tool hook never throws; it denies.
- **`claude plugin validate` reads a hooks module by its spelling** (2.1.289, T-651): every `$`
  call is `$.noun.event(...)` at the call site (never `$.session[k]()` or a noun passed as a
  value), `on` takes a string literal, `$.env.get` a literal name, and an event registered twice
  with no matcher is refused. A helper that loops over names fails validation and never loads.
- **A `tool.check` hook that holds its verdict holds the permission dialog undrawn** (2.1.289,
  T-651): the pane reads `Waiting…` until the hook resolves `ask`, so a decision awaited there is
  phone-or-terminal, never both. And no `Notification{permission_prompt}` fires for a dialog on
  2.1.289; `tool.check → ask` is that moment.
- **A registered tool answers with `{ result: <text> }`** (the model reads it verbatim) or an
  array of API content blocks (an image is `{ type: 'image', source: { type: 'base64', … } }`);
  an MCP-shaped `{ content, isError }` is refused ("does not match its output shape"), and a
  hook's `{ isError: true }` is not marked an error: only `{ deny }` is.
- **A call a mod's `tool.call` hook answers fires no `PostToolUse`** (a registered tool's, a
  question the mod answered): only the engine's own path fires it.
- **`turn.complete.usage` says neither a cache write's lifetime nor fast mode** (2.1.288): a mod
  report's writes and speed follow what the transcript tail last read (`cost::Hint`, T-581).
- **`classic.PreToolUse` carries no `agentId`**; the `tool.call` beneath it does, by
  `tool_use_id`.
- **`$.mcp.call` on a server the mod's manifest lists is refused without an allow rule**, and that
  server's tools are listed to the model: not an up-channel for mesimon.
- **The hook exec form is `"command": <exe>` plus `"args": [...]`** — docs/11's bare-`args`
  example fails the live validator. No `if` off tool events; matchers only where supported.
- **`git diff --raw HEAD` writes the destination blob as forty zeros**, so a mode-only test
  cannot be blob equality.
- **`git diff --no-index` exits 1 on differences**, which is not an error.
- **`-uall`, never `-unormal`**: `-unormal` collapses an untracked directory to one unopenable
  `? dir/` row.
- **patch-id comparison needs `--no-renames` on *both* sides** (a path filter would flip rename
  detection on one only) and the git pins on the command line (`diff.orderFile` gone is fatal,
  `log.follow` reaches `log` but not `diff-tree`, `log.abbrevCommit` zeroes the commit column).
- **`--max-count` takes the newest N**, so a walk must be anchored by date, not by count.
- **A `!` command in Claude's composer fires no hook while it runs** (2.1.295, T-707): no
  `UserPromptSubmit`, nothing to the mod, nothing in the transcript until it ends (then
  `<bash-input>` and `<bash-stdout>` land together), and the pane's `#{pane_current_command}`
  keeps naming `claude` because the shell is a detached child with no tty. The pane process's
  children are the one live fact (`bash_mode_foreground`); only a shell younger than the idle
  spell is the command, since the MCP servers and a background task's shell are older. **And
  the output is then sent to the model**: a short turn follows with no `UserPromptSubmit` and a
  `Stop` at its end.
- **A human refusing a dialog fires no hook.** "No, keep planning", an Esc out of a question or a
  denied permission emits no `PostToolUse`, no `PostToolUseFailure` and no `PermissionDenied`
  (that one is auto mode's classifier). The agent's next `PreToolUse` is the first frame that
  says the dialog is gone, and it is the refusal road (`Signal::ToolStarted`, T-447).
- **A tool in flight keeps the transcript still for its whole duration**, so an mtime-quiet rule
  reads a 3.5-minute `cargo` call as a dead turn.
- **Claude Code's session file flips `idle` at the end of every turn**, milliseconds before the
  `Stop` hook fires, so a status probe races the hook on every turn.
- **A teammate is listed `running` in every later `Stop` payload for its whole life**, idle or
  not, so teammates are counted and never classed.
- **`automove` is edge-triggered**, and a second `UserPromptSubmit` to an already-`Running`
  session produces no edge — an e2e assertion resting on one passes for the wrong reason. The
  attention machine also arms a flap pin after >4 committed changes in 20 s; it drops inferred
  signals only, and a stated High signal commits through it.
- **`probe_activity` only scans `Running`**, which is what makes `Idle{Background}` immune to it.
- **A `SubagentStop`'s `background_tasks` is the whole session's list**, not the subagent's
  (2.1.283 fills both Stop hooks from one task registry), and its type labels spell the Bash
  tool's background commands and the Monitor tool's watches both `shell`. Only the tool result
  that started a task (`backgroundTaskId`, `taskId`) says which it is (T-483).
- **Plan mode admits an MCP tool only on `annotations.readOnlyHint: true` and ignores allow
  rules; default and auto mode admit only on an allow rule and ignore the hint** (2.1.270). A
  read tool needs both, and `read_rung_is_hinted_read_only` keeps them one list.
