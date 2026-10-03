// mesimon's mod (T-574, phase 1 of the road T-573 measured; the turn roads
// T-575 and T-576). The daemon lays this folder under its state dir and
// passes it with `--plugin-dir` to a Claude session it starts on the mod
// road; nothing installs it anywhere.
//
// What it does, and all it does:
//  - TOOLS: the board's tools for this session's tier, registered at
//    session.start with `$.tool.register` by the names, descriptions and
//    schemas `mesimon mcp --list` prints (the shim's own, `doctor --mcp`),
//    and each call served by `mesimon mcp --call`, which asks the daemon as
//    the shim does. The daemon checks the tier and the session at every call
//    (T-577): a registered tool runs without Claude Code's permission check.
//  - GATE: a structured write (`Write`, `Edit`, `NotebookEdit`) under the
//    board dir or the state dir, the worktrees under it excepted, is refused
//    at `tool.call` with `mesimon gate`'s own words, decided here and from
//    the pane's variables alone, and reported for the feed (T-577).
//  - RELAY: each event the generated hook set reports, by the same names and
//    the same matchers, goes up through the real `mesimon hook` binary with
//    `--road mod`, one after another in the order the events came. Since
//    T-577 a mod launch carries no hook set, so these are the frames the
//    daemon ingests.
//  - APPROVE: a permission dialog is put to `mesimon approve` as the hook
//    set's entry did, and a person's one-shot answer from Remote Control is
//    returned as the dialog's decision; no answer leaves the dialog alone.
//  - BRIDGE: one `mesimon mod-bridge` per session, spawned at session.start
//    (or by the first event after it, when its read of the variables failed),
//    whose stdout is the daemon's commands to this session, one JSON line
//    each: `ping`, answered with a `ModPong` relay; `submit`, a prompt the
//    daemon delivers (a person's words, or the brief they wrote), submitted
//    whole as the person's own (`asUser: true`) and reported with
//    `ModSubmit`; `fill`, the same words put in the session's EMPTY composer
//    (never over a person's draft) for Claude Code's send-now, which the
//    daemon presses on the `ModFill` this reports (T-601); `answer`, the
//    answer a person or the crown chose for a question this session's model
//    asked, returned in the native dialog's place and reported with
//    `ModAnswer`.
//  - HOLD: every `AskUserQuestion` call of the session's own (no subagent's)
//    is raced between the native dialog, which is drawn as ever and which
//    the person may answer first, and the board's `answer`.
//  - COST: each turn's end (`turn.complete`, a subagent's too) goes up as
//    `ModUsage`: the engine's count of the turn's tokens and, for the main
//    loop, the rate-limit windows `$.session.usage()` reads at that moment
//    (T-581). Observed only: the turn's result passes as it came.
//  - LOAD: the pane's variables are read by the first event and kept once
//    read whole; a read that fails is tried again by the next event, and the
//    one that succeeds reports the failures before it as `ModLoadFailed`
//    (T-594).
//
// It holds no policy. Every decision is the daemon's.
//
// Never (README promise 3, the T-573 never-list): no `$.session.append`, no
// `context` on any hook, no `prompt.compose` / `prompt.context` /
// `prompt.section`, no rewrite of a prompt's words, no submit without
// `asUser: true`, no fill but into an empty composer and whole, no rewrite of the model's tool arguments, no `deny` of a
// tool but two: the gate's (its words are `mesimon gate`'s, which the hook
// set already hands the model; deny or nothing, never allow) and a board
// tool's refusal (the daemon's words, the shim's error result), no `tool.check →
// allow` beyond the one-shot a person consented to, no `$.model.*`, no
// `$.session.send`. Every hook here returns `next(e)` as it came, but the
// question's and the gate's: the question's returns the answer a person or
// the crown chose, by the dialog's own labels, which is what the native
// dialog would have returned.
//
// Environment, set by the daemon on the pane (`mesimon exec --set`):
//   MESIMON_MOD_BIN        the mesimon binary
//   MESIMON_MOD_HOOK_SOCK  the board's hook.sock
//   MESIMON_MOD_ORCH_SOCK  the board's orch.sock
//   MESIMON_MOD_SESSION    mesimon's id for this session (not Claude's
//                          session_id, which `/clear` changes)
//   MESIMON_MOD_GATE_BOARD the board dir, `<repo>/.mesimon` (the gate's)
//   MESIMON_MOD_GATE_STATE the state dir
//   MESIMON_MOD_GATE_ALLOW the worktrees under it, the agent's own
//   MESIMON_MOD_TOOLS      the tier of tools to register (off: unset)
import type { Register } from 'claude-code'

type Config = { bin: string; hookSock: string; orchSock: string; session: string }
type Roots = { board: string; state: string; allow: string }

// The hook set's matchers (`hook_settings.rs`), so each relayed frame has a
// twin. The daemon's unit test holds these lists to the Rust ones.
const SESSION_START_SOURCES = ['startup', 'resume', 'clear', 'compact', 'fork']
const SESSION_END_REASONS = ['clear', 'resume', 'logout', 'prompt_input_exit', 'other']
const STOP_FAILURE_MATCHERS = [
  'rate_limit',
  'overloaded',
  'authentication_failed',
  'oauth_org_not_allowed',
  'billing_error',
  'invalid_request',
  'model_not_found',
  'server_error',
  'max_output_tokens',
  'unknown',
]
const PRE_TOOL_USE_TOOLS = ['AskUserQuestion', 'ExitPlanMode']

// ---- The gate (T-577): `mesimon gate`'s rules, in-process. The structured
// writes it judges, and what the model reads when one is refused: each
// rule's text is `mesimon_core::verdict::RuleId::reason`, held to it by the
// daemon's unit test, and the rule's tag is `RuleId::tag`.
const GATE_TOOLS = ['Write', 'Edit', 'NotebookEdit']
const RULE_BOARD = 'board_dir'
const RULE_STATE = 'state_dir'
const REASON_BOARD =
  'mesimon owns .mesimon/ — the board is edited through mesimon, not by writing its files. Use mesimon\'s scoped MCP tools instead.'
const REASON_STATE =
  'mesimon owns its state directory — sessions, worktree bindings and hook settings are not editable by an agent.'
// Losing the roots must not turn a guarded write into no opinion
// (`mesimon gate`'s trusted-environment rule).
const REASON_NO_ROOTS = 'Mesimon write guard context is unavailable'

// ---- The tools (T-577). The plugin is `mesimon`, so a registered tool is
// `mcp__mesimon__<name>`: the shim's names, unchanged for the model.
const TOOL_PREFIX = 'mcp__mesimon__'
// `answer_agent` and `accept_plan` wait for their delivery (`mesimon mcp`'s
// ANSWER_WAIT_SECS, 75 s); every other call answers in seconds.
const TOOL_CALL_TIMEOUT_MS = 90000

// A relay's own bound (`mesimon hook --road mod` has no other), and the
// permission bridge's: `mesimon approve` gives up at 47 s, the hook set gave
// it 50.
const RELAY_TIMEOUT_MS = 5000
const APPROVE_TIMEOUT_MS = 50000

// The kinds of command this mod reads, said to the daemon by the bridge
// (`mesimon_core::road::SPEAKS`): a session keeps the mod it was launched
// with, so a newer daemon sends it only these.
const SPEAKS = ['ping', 'submit', 'answer', 'fill']

// `mesimon mod-bridge` exits so when the daemon refused it for good (the
// session is gone, the pane is not its own, another bridge took the seat):
// `mesimon_core::road::BRIDGE_REFUSED_EXIT`. Not respawned.
const BRIDGE_REFUSED_EXIT = 3
const BACKOFF_FIRST_MS = 1000
const BACKOFF_MAX_MS = 60000
// A bridge that lived this long was healthy; its successor starts over.
const HEALTHY_MS = 60000
const SEEN_MAX = 64

// The pane's variables once a read found all four. A read that threw or
// came back short is never kept: the next event reads again (T-594).
let config: Config | undefined
// The reads that failed before one succeeded, said to the daemon then
// (`ModLoadFailed`): the one road that can say it is this one, once open.
let failed: { reads: number; error: string; at: string } | undefined
let bridge: 'off' | 'on' | 'refused' = 'off'
let started = false
// The last relay: the next one starts once it has gone.
let tail: Promise<unknown> = Promise.resolve()
const seen: string[] = []
// The questions held for the board's answer, by tool_use_id: each settles
// its race with the answer's labels.
const holds = new Map<string, (answers: Record<string, string>) => void>()
// The tools this session registered, by full name: the only calls served.
const registered = new Set<string>()
// Whether the tools are registered (or there were none to register), and
// the registration in flight.
let toolsDone = false
let toolsRun: Promise<void> | undefined
// The subagent each dialog tool's call ran in, by its tool_use_id, from the
// `tool.call` beneath its `classic.PreToolUse` (which carries none).
const agents = new Map<string, string>()
const AGENTS_MAX = 64

/** The four variables, read at once; a short read throws, naming what is unset. */
async function load($: any): Promise<Config> {
  const [bin, hookSock, orchSock, session] = await Promise.all([
    $.env.get('MESIMON_MOD_BIN'),
    $.env.get('MESIMON_MOD_HOOK_SOCK'),
    $.env.get('MESIMON_MOD_ORCH_SOCK'),
    $.env.get('MESIMON_MOD_SESSION'),
  ])
  if (!bin || !hookSock || !orchSock || !session) {
    const unset = Object.entries({ bin, hookSock, orchSock, session }).filter(([, v]) => !v)
    throw new Error(`unset: ${unset.map(([k]) => k).join(', ')}`)
  }
  return { bin, hookSock, orchSock, session }
}

/**
 * The pane's variables, which do not change for the process: kept once read
 * whole. A `$` call fails with the dispatch it rides when that dispatch is
 * abandoned, and the first read rides the first event's, so a failed read
 * is said in the debug log and tried again by the next event, never kept:
 * a kept failure was a session whose mod relayed nothing for its whole
 * life (T-594). The first read that succeeds after a failure reports it.
 */
async function settings($: any, at: string): Promise<Config | undefined> {
  if (config) {
    if (started && !toolsDone) void bringUp($, config)
    return config
  }
  // Each event reads for itself (T-577): a read shared with another event
  // failed with that event's dispatch, and three sessions started at once
  // left two whose every event had awaited the one read session.start's
  // abandoned dispatch took down.
  try {
    config ??= await load($)
  } catch (err) {
    failed ??= { reads: 0, error: String(err), at }
    failed.reads += 1
    void $.ui.log(`mesimon: the pane's variables were not read at ${at} (${String(err)}); the next event reads them again`, {
      to: 'debug',
    })
    return undefined
  }
  if (failed) {
    const report = failed
    failed = undefined
    void relay($, 'ModLoadFailed', 'recovered', report, false)
  }
  // `session.start` brings the tools and the bridge up; one whose read
  // failed left both to the first event that reads them.
  if (started) void bringUp($, config)
  return config
}

/**
 * The gate's roots, absolute, read at every guarded call: three variables
 * are cheap, and nothing read once can be a failure kept for the session's
 * life (T-594's lesson).
 */
async function gateRoots($: any): Promise<Roots | undefined> {
  const [board, state, allow] = await Promise.all([
    $.env.get('MESIMON_MOD_GATE_BOARD'),
    $.env.get('MESIMON_MOD_GATE_STATE'),
    $.env.get('MESIMON_MOD_GATE_ALLOW'),
  ])
  if (![board, state, allow].every(r => typeof r === 'string' && r.startsWith('/'))) return undefined
  return { board, state, allow }
}

/** `.` and `..` folded by spelling, before anything is asked of the disk:
 * `<repo>/src/../.mesimon/x` names a guarded file whatever exists. */
export function fold(path: string): string {
  const absolute = path.startsWith('/')
  const out: string[] = []
  for (const part of path.split('/')) {
    if (part === '' || part === '.') continue
    if (part === '..') {
      if (out.length && out[out.length - 1] !== '..') out.pop()
      else if (!absolute) out.push('..')
      continue
    }
    out.push(part)
  }
  return (absolute ? '/' : '') + out.join('/')
}

/**
 * Where a path lands: the deepest ancestor that exists, every link
 * followed, with the rest put back. A file about to be written has no real
 * path of its own; its folder usually has. A relative path is the session's
 * working directory's, as the tool reads it.
 */
async function placed($: any, path: string): Promise<string> {
  const parts = fold(path).split('/')
  const tail: string[] = []
  while (parts.length) {
    const probe = parts.join('/') || (path.startsWith('/') ? '/' : '.')
    let real: string | undefined
    try {
      real = (await $.fs.stat(probe, { resolve: true }))?.realPath
    } catch {
      real = undefined
    }
    if (typeof real === 'string') {
      const base = real.replace(/\/+$/, '')
      return tail.length ? `${base}/${tail.reverse().join('/')}` : base || '/'
    }
    const name = parts.pop()
    if (name) tail.push(name)
  }
  return fold(path)
}

const under = (path: string, root: string) => path === root || path.startsWith(root === '/' ? root : `${root}/`)

/**
 * Which rule a structured write lands under, if any: `mesimon gate`'s
 * `guarded_by`. The worktrees under the state dir are the agent's own and are
 * judged first; then the board dir, then the state dir.
 */
export async function guardedBy($: any, path: string, r: Roots): Promise<string | undefined> {
  const target = await placed($, path)
  if (under(target, await placed($, r.allow))) return undefined
  if (under(target, await placed($, r.board))) return RULE_BOARD
  if (under(target, await placed($, r.state))) return RULE_STATE
  return undefined
}

/**
 * The board's tools for this session's tier, registered before the first
 * turn. No tier (`MESIMON_MOD_TOOLS` unset: the column's tools are off)
 * registers nothing; a list that cannot be read registers nothing either,
 * and the session runs without the tools rather than not at all.
 */
function registerTools($: any, c: Config): Promise<void> {
  if (toolsDone) return Promise.resolve()
  toolsRun ??= registerOnce($, c)
  return toolsRun
}

/** The tools, then the bridge: no word goes down before the tools are listed. */
async function bringUp($: any, c: Config) {
  await registerTools($, c)
  startBridge($, c)
}

async function registerOnce($: any, c: Config) {
  try {
    const tier = await $.env.get('MESIMON_MOD_TOOLS')
    if (!tier) {
      toolsDone = true
      return
    }
    const out = await $.process.run([c.bin, 'mcp', '--list', '--tools', tier], { timeoutMs: 10000 })
    const specs = JSON.parse(String(out?.stdout ?? '[]'))
    toolsDone = true
    if (!Array.isArray(specs)) return
    for (const spec of specs) {
      try {
        const r = await $.tool.register({ name: spec.name, description: spec.description, inputSchema: spec.inputSchema })
        if (r && typeof r.tool === 'string') registered.add(r.tool)
      } catch {
        // That one tool is missing; the others stand.
      }
    }
  } catch {
    // The list was not read: the next event tries again, and the session
    // works without the tools meanwhile.
  } finally {
    toolsRun = undefined
  }
}

/**
 * A `tools/call` result, as `mesimon mcp --call` printed it, in the form a
 * registered tool answers the model: its text whole (an object of content
 * blocks is refused by the engine's output check, measured), an image as
 * the API's image block, and a refusal as an error result in the daemon's
 * own words (`{ isError }` from a hook is not marked an error; a deny is).
 */
export function toolAnswer(out: any): any {
  const content: any[] = Array.isArray(out?.content) ? out.content : []
  const text = content
    .filter(b => b?.type === 'text' && typeof b.text === 'string')
    .map(b => b.text)
    .join('\n')
  if (out?.isError === true) return { deny: text || 'mesimon: the call failed' }
  if (content.some(b => b?.type === 'image')) {
    return {
      result: content.map(b =>
        b?.type === 'image'
          ? { type: 'image', source: { type: 'base64', media_type: b.mimeType, data: b.data } }
          : { type: 'text', text: String(b?.text ?? '') },
      ),
    }
  }
  return { result: text }
}

/** What the model reads for a refused write: the rule's own text. */
function denial(rule: string): string {
  if (rule === RULE_BOARD) return REASON_BOARD
  if (rule === RULE_STATE) return REASON_STATE
  return REASON_NO_ROOTS
}

/** The path a structured write names: `file_path`, or a notebook's. */
export function gatePath(e: any): string | undefined {
  const path = e?.file_path ?? e?.notebook_path
  return typeof path === 'string' && path !== '' ? path : undefined
}

/**
 * One frame up through `mesimon hook --road mod`, as the hook set's command
 * hook sends it. Not awaited unless `wait`: the event goes on at once and a
 * relay that fails is a missing twin the daemon reports, never an error here.
 */
async function relay($: any, event: string, reason: string | undefined, body: unknown, wait: boolean) {
  try {
    const c = await settings($, event)
    if (!c) return
    const argv = [c.bin, 'hook', '--sock', c.hookSock, '--session', c.session, '--event', event]
    if (reason !== undefined) argv.push('--reason', reason)
    argv.push('--road', 'mod')
    const run = runAfter($, tail, argv, JSON.stringify(body ?? {}))
    tail = run.then(
      () => undefined,
      () => undefined,
    )
    if (wait) await run
  } catch {
    // A frame that never left: the daemon is down or the host went.
  }
}

/**
 * One relay, once the one before it has gone: the hook socket orders frames
 * by when it accepted them, and two `mesimon hook` processes started a few
 * milliseconds apart may connect in either order (T-577). Each waits at most
 * the one before's own bound.
 */
async function runAfter($: any, before: Promise<unknown>, argv: string[], stdin: string) {
  await before
  return $.process.run(argv, { stdin, timeoutMs: RELAY_TIMEOUT_MS })
}

/**
 * `mesimon approve`, as the hook set's PermissionRequest entry ran it: a
 * person answering the dialog from Remote Control, once. Its decision is
 * the daemon's, passed through whole; no decision (no phone, no answer in
 * time, the daemon down) is none, and the dialog stays the person's.
 */
async function approve($: any, e: unknown): Promise<unknown> {
  try {
    const c = await settings($, 'PermissionRequest')
    if (!c) return undefined
    const argv = [c.bin, 'approve', '--sock', c.hookSock, '--session', c.session]
    const out = await $.process.run(argv, { stdin: JSON.stringify(e ?? {}), timeoutMs: APPROVE_TIMEOUT_MS })
    const decision = JSON.parse(String(out?.stdout || 'null'))?.hookSpecificOutput?.decision
    return decision && typeof decision === 'object' ? decision : undefined
  } catch {
    return undefined
  }
}

/**
 * The command hook's stdin for a `PreToolUse`, rebuilt from the envelope,
 * with a subagent's `agent_id` as the hook set spelled it: the daemon tells
 * a subagent's dialog from the session's own by it. `classic.PreToolUse`
 * carries no `agentId` (T-574); the `tool.call` beneath it does, and is
 * noted by its call's id (`agents`).
 */
export function preToolUseBody(e: any, agentId?: string): Record<string, unknown> {
  const { tool, tool_use_id, agentId: own, ...tool_input } = e ?? {}
  const body: Record<string, unknown> = { hook_event_name: 'PreToolUse', tool_name: tool, tool_use_id, tool_input }
  const agent = typeof own === 'string' ? own : agentId
  if (typeof agent === 'string') body.agent_id = agent
  return body
}

/** The `ModAnswer` report's body: a `PostToolUse` as the hook set spells it. */
export function answeredBody(id: string, questions: unknown, response?: unknown): Record<string, unknown> {
  const body: Record<string, unknown> = {
    hook_event_name: 'PostToolUse',
    tool_name: 'AskUserQuestion',
    tool_use_id: id,
    tool_input: { questions },
  }
  if (response !== undefined) body.tool_response = response
  return body
}

/**
 * A prompt the daemon delivers, submitted as the person's own words and
 * whole: never framed by the plugin's name, never with context, never
 * rewritten. Not awaited by the bridge (it resolves when the prompt's turn
 * starts, a whole turn later behind a running one); its end is reported.
 */
async function submitPrompt($: any, id: string, text: string) {
  let report: Record<string, unknown>
  try {
    const r: any = await $.prompt.submit({ text, asUser: true })
    report = r && r.drop !== undefined ? { outcome: 'dropped', reason: String(r.drop) } : { outcome: 'entered' }
  } catch (err) {
    report = { outcome: 'rejected', error: String(err) }
  }
  await relay($, 'ModSubmit', id, report, false)
}

/**
 * Words for Claude Code's send-now (T-601): into the session's composer,
 * whole, and only while the box is empty, so a person's draft is never
 * replaced; the daemon presses the send-now keys on `filled`. A plugin's
 * `submit` waits for the running turn's end, and the send-now sends only
 * what stands in the composer, so this is the mod's road to it.
 */
async function fillPrompt($: any, id: string, text: string) {
  let report: Record<string, unknown>
  try {
    const box: any = await $.prompt.read()
    if (typeof box?.text === 'string' && box.text.trim() !== '') {
      report = { outcome: 'refused', reason: 'draft' }
    } else {
      const r: any = await $.prompt.fill({ text, mode: 'replace' })
      report = r?.isFilled
        ? { outcome: 'filled' }
        : { outcome: 'refused', reason: String(r?.refusal ?? 'not_filled') }
    }
  } catch (err) {
    report = { outcome: 'refused', error: String(err) }
  }
  await relay($, 'ModFill', id, report, false)
}

async function relaySingle($: any, e: any, next: any) {
  const event = String(next.event).replace(/^classic\./, '')
  void relay($, event, undefined, e, false)
  return next(e)
}

async function handle($: any, line: string) {
  let frame: any
  try {
    frame = JSON.parse(line)
  } catch {
    return
  }
  const id = typeof frame?.id === 'string' ? frame.id : undefined
  if (!id || seen.includes(id)) return
  seen.push(id)
  if (seen.length > SEEN_MAX) seen.shift()
  switch (frame.kind) {
    case 'ping':
      await relay($, 'ModPong', id, {}, false)
      break
    case 'submit':
      if (typeof frame.text === 'string' && frame.text) void submitPrompt($, id, frame.text)
      break
    case 'fill':
      if (typeof frame.text === 'string' && frame.text) await fillPrompt($, id, frame.text)
      break
    case 'answer': {
      const call = typeof frame.tool_use_id === 'string' ? frame.tool_use_id : ''
      const answers = frame.answers && typeof frame.answers === 'object' ? frame.answers : undefined
      const settle = holds.get(call)
      if (settle && answers) {
        holds.delete(call)
        settle(answers)
      } else {
        // The person answered or refused first: the daemon settles on theirs.
        await relay($, 'ModAnswer', 'nothing_held', { tool_use_id: call }, false)
      }
      break
    }
    default:
      // A kind from a newer daemon than this mod: not ours to guess at.
      break
  }
}

/** One bridge, read to its end; resolves with its exit code. */
async function bridgeOnce($: any, c: Config): Promise<number | null> {
  let buf = ''
  try {
    const child = $.process.spawn({
      argv: [c.bin, 'mod-bridge', '--sock', c.orchSock, '--session', c.session, '--speaks', SPEAKS.join(',')],
    })
    for await (const { stream, text } of child) {
      if (stream !== 'stdout') continue
      buf += text
      let nl: number
      while ((nl = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, nl)
        buf = buf.slice(nl + 1)
        if (line.trim()) await handle($, line)
      }
    }
    return (await child.result).code
  } catch {
    return null
  }
}

/** The bridge, for the session's life: respawned with backoff when it dies. */
async function bridgeLoop($: any, c: Config) {
  let backoff = BACKOFF_FIRST_MS
  try {
    for (;;) {
      const born = await $.clock.now()
      if ((await bridgeOnce($, c)) === BRIDGE_REFUSED_EXIT) {
        bridge = 'refused'
        return
      }
      if ((await $.clock.now()) - born >= HEALTHY_MS) backoff = BACKOFF_FIRST_MS
      await $.clock.sleep(backoff)
      backoff = Math.min(backoff * 2, BACKOFF_MAX_MS)
    }
  } catch {
    // The host went (the module unloading): so does the bridge.
  }
  bridge = 'off'
}

/** The bridge, unless one runs or the daemon refused this session's for good. */
function startBridge($: any, c: Config) {
  if (bridge !== 'off') return
  bridge = 'on'
  void bridgeLoop($, c)
}

/**
 * The `ModUsage` report's body (T-581): the turn's own fields as the engine
 * gave them, and the main loop's rate-limit windows. Never the answer's
 * text: the daemon reads counts, not words.
 */
export function usageBody(e: any, usage: unknown, limits?: unknown): Record<string, unknown> {
  const body: Record<string, unknown> = { turnId: e?.turnId, reason: e?.reason, durationMs: e?.durationMs }
  if (typeof e?.agentId === 'string') body.agentId = e.agentId
  if (usage && typeof usage === 'object') body.usage = usage
  if (Array.isArray(limits)) body.rateLimits = limits
  return body
}

/**
 * A turn's end, observed: its count and, for the main loop, the account's
 * windows. `$.session.usage()` is read before the hook returns, while the
 * event's dispatch stands (a `$` call fails with an abandoned one, T-594);
 * a read that failed sends the count alone.
 */
async function turnComplete($: any, e: any, next: any) {
  const result = await next(e)
  let limits: unknown
  if (typeof e?.agentId !== 'string') {
    try {
      limits = (await $.session.usage())?.rateLimits
    } catch {
      limits = undefined
    }
  }
  void relay($, 'ModUsage', String(e?.reason ?? 'answer'), usageBody(e, e?.usage ?? result?.usage, limits), false)
  return result
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const result = await next(e)
    // `session.start` may come again in the same process (a `/clear`); one
    // bridge serves them all. The read brings the tools and then the bridge
    // up (`bringUp`), not awaited here: the daemon sends no word before the
    // bridge's first poll, so the tools are listed before any turn it
    // starts, and a session.start held by them is one more dispatch to lose.
    started = true
    await settings($, 'session.start')
    return result
  })

  // ---- The relay, event by event: the hook set's names, not `classic.*`,
  // which also delivers the verbose tier mesimon never registers.
  on('classic.SessionStart', async ($, e, next) => {
    const source = (e as any).source
    if (SESSION_START_SOURCES.includes(source)) void relay($, 'SessionStart', source, e, false)
    return next(e)
  })
  on('classic.SessionEnd', async ($, e, next) => {
    // Awaited: the process may be gone before an unawaited relay runs.
    const reason = (e as any).reason
    if (SESSION_END_REASONS.includes(reason)) await relay($, 'SessionEnd', reason, e, true)
    return next(e)
  })
  on('classic.StopFailure', async ($, e, next) => {
    const error = (e as any).error
    if (STOP_FAILURE_MATCHERS.includes(error)) void relay($, 'StopFailure', error, e, false)
    return next(e)
  })
  on('classic.PreToolUse', async ($, e, next) => {
    const tool = (e as any).tool
    if (PRE_TOOL_USE_TOOLS.includes(tool)) {
      const id = (e as any).tool_use_id
      const agent = typeof id === 'string' ? agents.get(id) : undefined
      if (typeof id === 'string') agents.delete(id)
      void relay($, 'PreToolUse', undefined, preToolUseBody(e, agent), false)
    }
    return next(e)
  })
  // Noted before anything else runs for the call: which subagent it is.
  on('tool.call', { tool: PRE_TOOL_USE_TOOLS }, async ($, e, next) => {
    const { tool_use_id: id, agentId } = e as any
    if (typeof id === 'string' && typeof agentId === 'string') {
      agents.set(id, agentId)
      if (agents.size > AGENTS_MAX) agents.delete(agents.keys().next().value as string)
    }
    return next(e)
  })
  // ---- The gate (T-577): deny or nothing, local and static. Nothing is
  // asked of the daemon, so a dead one still refuses; the denial is reported
  // afterwards for the feed and may be lost. Bash is not judged: a command
  // string is no path (docs/USING.md).
  // A hook that threw would be skipped and the write would run, so a
  // decision that could not be made is a refusal.
  on('tool.call', { tool: GATE_TOOLS }, async ($, e, next) => {
    const path = gatePath(e)
    if (path === undefined) return next(e)
    let r: Roots | undefined
    let rule: string | undefined
    try {
      r = await gateRoots($)
      rule = r ? await guardedBy($, path, r) : 'no_roots'
    } catch {
      r = undefined
      rule = 'no_roots'
    }
    if (rule === undefined) return next(e)
    // The path, and nothing else: the one fact the feed's line carries.
    if (r) void relay($, 'GateDenied', rule, { file_path: path }, false)
    return { deny: denial(rule) }
  })

  // ---- The tools (T-577): each call of a tool this session registered
  // goes to the daemon through `mesimon mcp --call`, the model's arguments
  // as they came (the envelope's own keys aside), and its answer comes
  // back. A hook that threw would be skipped and the model told it lacked a
  // permission, so nothing here throws: a call that could not be made is a
  // refusal in plain words.
  on('tool.call', { tool: /^mcp__mesimon__/ }, async ($, e, next) => {
    const { tool, tool_use_id, agentId, consent, ...args } = e as any
    void agentId
    void consent
    if (!registered.has(tool)) return next(e)
    let c: Config | undefined
    try {
      c = await settings($, 'tool.call')
    } catch {
      c = undefined
    }
    if (!c) return toolAnswer({ content: [{ type: 'text', text: 'mesimon could not answer the call: the pane\'s variables were not read' }], isError: true })
    const argv = [c.bin, 'mcp', '--sock', c.orchSock, '--session', c.session, '--call', tool.slice(TOOL_PREFIX.length)]
    if (typeof tool_use_id === 'string') argv.push('--tool-use-id', tool_use_id)
    let out: any
    try {
      const run = await $.process.run(argv, { stdin: JSON.stringify(args), timeoutMs: TOOL_CALL_TIMEOUT_MS })
      out = JSON.parse(String(run?.stdout ?? ''))
    } catch (err) {
      out = { content: [{ type: 'text', text: `mesimon could not answer the call: ${String(err)}` }], isError: true }
    }
    return toolAnswer(out)
  })

  // ---- The question (T-576): the native dialog is drawn as ever, and the
  // first of the person's answer and the board's wins. When the board's
  // does, the mod returns it as the dialog would have (the model reads the
  // tool's own text) and the engine fires no PostToolUse, so the mod says
  // so; when the person refuses the dialog, no hook says that either.
  on('tool.call', { tool: 'AskUserQuestion' }, async ($, e, next) => {
    const call = (e as any).tool_use_id
    if (typeof call !== 'string' || (e as any).agentId) return next(e)
    let settle: (answers: Record<string, string>) => void = () => undefined
    const board = new Promise<Record<string, string>>(resolve => {
      settle = resolve
    })
    holds.set(call, settle)
    // An interrupt abandons the call: the dispatch goes on without this
    // hook, and the dialog is gone with no answer, as a refusal's is.
    const aborted = new Promise<{ who: 'aborted' }>(resolve => {
      if (next.signal.aborted) resolve({ who: 'aborted' })
      else next.signal.addEventListener('abort', () => resolve({ who: 'aborted' }), { once: true })
    })
    const native = next(e).then(
      r => ({ who: 'native' as const, r }),
      err => ({ who: 'failed' as const, err }),
    )
    const first = await Promise.race([
      native,
      aborted,
      board.then(answers => ({ who: 'board' as const, answers })),
    ])
    holds.delete(call)
    const questions = (e as any).questions
    if (first.who === 'board') {
      const result = { questions, answers: first.answers }
      void relay($, 'ModAnswer', 'answered', answeredBody(call, questions, result), false)
      void native.then(() => undefined)
      return { result } as any
    }
    if (first.who === 'aborted' || (first.who === 'native' && (first.r as any)?.isError)) {
      void relay($, 'ModAnswer', 'declined', answeredBody(call, questions), false)
    }
    if (first.who === 'aborted') throw new Error('the question was interrupted')
    if (first.who === 'failed') throw first.err
    return first.r
  })

  on('classic.PostToolUse', relaySingle)
  on('classic.UserPromptSubmit', relaySingle)
  on('classic.Stop', relaySingle)
  on('classic.SubagentStart', relaySingle)
  on('classic.SubagentStop', relaySingle)
  on('classic.TeammateIdle', relaySingle)
  on('classic.PermissionRequest', async ($, e, next) => {
    void relay($, 'PermissionRequest', undefined, e, false)
    // Beside whatever else answers the dialog, as the hook set's two
    // entries ran side by side; a person's phone answer is the one taken.
    const theirs = next(e)
    const ours = await approve($, e)
    if (ours) {
      void theirs.then(
        () => undefined,
        () => undefined,
      )
      return { decision: ours } as any
    }
    return theirs
  })
  on('classic.PermissionDenied', relaySingle)
  on('classic.Notification', relaySingle)
  on('classic.Elicitation', relaySingle)
  on('classic.ElicitationResult', relaySingle)
  on('classic.PreCompact', relaySingle)
  on('classic.PostCompact', relaySingle)

  // ---- The cost (T-581): each turn's count, and the main loop's windows.
  on('turn.complete', turnComplete)
}
