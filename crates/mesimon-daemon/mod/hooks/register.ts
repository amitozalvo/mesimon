// mesimon's mod (T-574, phase 1 of the road T-573 measured; the turn roads
// T-575 and T-576). The daemon lays this folder under its state dir and
// passes it with `--plugin-dir` to a Claude session it starts on the mod
// road; nothing installs it anywhere.
//
// What it does, and all it does:
//  - RELAY: each event the generated hook set reports, by the same names and
//    the same matchers, goes up through the real `mesimon hook` binary with
//    `--road mod`. The daemon pairs it with the hook set's own frame of the
//    same event and says when they disagree (shadow mode); it ingests only
//    the hook set's.
//  - BRIDGE: one `mesimon mod-bridge` per session, spawned at session.start,
//    whose stdout is the daemon's commands to this session, one JSON line
//    each: `ping`, answered with a `ModPong` relay; `submit`, a prompt the
//    daemon delivers (a person's words, or the brief they wrote), submitted
//    whole as the person's own (`asUser: true`) and reported with
//    `ModSubmit`; `answer`, the answer a person or the crown chose for a
//    question this session's model asked, returned in the native dialog's
//    place and reported with `ModAnswer`.
//  - HOLD: every `AskUserQuestion` call of the session's own (no subagent's)
//    is raced between the native dialog, which is drawn as ever and which
//    the person may answer first, and the board's `answer`.
//
// It holds no policy. Every decision is the daemon's.
//
// Never (README promise 3, the T-573 never-list): no `$.session.append`, no
// `context` on any hook, no `prompt.compose` / `prompt.context` /
// `prompt.section`, no rewrite of a prompt's words, no submit without
// `asUser: true`, no rewrite of the model's tool arguments, no `deny` of a
// tool (its text would reach the model), no `tool.check → allow` beyond the
// one-shot a person consented to, no `$.model.*`, no `$.session.send`. Every
// hook here returns `next(e)` as it came, but the question's: that one
// returns the answer a person or the crown chose, by the dialog's own
// labels, which is what the native dialog would have returned.
//
// Environment, set by the daemon on the pane (`mesimon exec --set`):
//   MESIMON_MOD_BIN        the mesimon binary
//   MESIMON_MOD_HOOK_SOCK  the board's hook.sock
//   MESIMON_MOD_ORCH_SOCK  the board's orch.sock
//   MESIMON_MOD_SESSION    mesimon's id for this session (not Claude's
//                          session_id, which `/clear` changes)
import type { Register } from 'claude-code'

type Config = { bin: string; hookSock: string; orchSock: string; session: string }

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

// The kinds of command this mod reads, said to the daemon by the bridge
// (`mesimon_core::road::SPEAKS`): a session keeps the mod it was launched
// with, so a newer daemon sends it only these.
const SPEAKS = ['ping', 'submit', 'answer']

// `mesimon mod-bridge` exits so when the daemon refused it for good (the
// session is gone, the pane is not its own, another bridge took the seat):
// `mesimon_core::road::BRIDGE_REFUSED_EXIT`. Not respawned.
const BRIDGE_REFUSED_EXIT = 3
const BACKOFF_FIRST_MS = 1000
const BACKOFF_MAX_MS = 60000
// A bridge that lived this long was healthy; its successor starts over.
const HEALTHY_MS = 60000
const SEEN_MAX = 64

let config: Promise<Config | undefined> | undefined
let bridgeOn = false
const seen: string[] = []
// The questions held for the board's answer, by tool_use_id: each settles
// its race with the answer's labels.
const holds = new Map<string, (answers: Record<string, string>) => void>()

async function load($: any): Promise<Config | undefined> {
  const bin = await $.env.get('MESIMON_MOD_BIN')
  const hookSock = await $.env.get('MESIMON_MOD_HOOK_SOCK')
  const orchSock = await $.env.get('MESIMON_MOD_ORCH_SOCK')
  const session = await $.env.get('MESIMON_MOD_SESSION')
  if (!bin || !hookSock || !orchSock || !session) return undefined
  return { bin, hookSock, orchSock, session }
}

/** The pane's variables, read once: they do not change for the process. */
function settings($: any): Promise<Config | undefined> {
  config ??= load($)
  return config
}

/**
 * One frame up through `mesimon hook --road mod`, as the hook set's command
 * hook sends it. Not awaited unless `wait`: the event goes on at once and a
 * relay that fails is a missing twin the daemon reports, never an error here.
 */
async function relay($: any, event: string, reason: string | undefined, body: unknown, wait: boolean) {
  const c = await settings($)
  if (!c) return
  const argv = [c.bin, 'hook', '--sock', c.hookSock, '--session', c.session, '--event', event]
  if (reason !== undefined) argv.push('--reason', reason)
  argv.push('--road', 'mod')
  const run = $.process.run(argv, { stdin: JSON.stringify(body ?? {}), timeoutMs: 5000 })
  if (wait) {
    try {
      await run
    } catch {
      // The daemon reports the missing twin.
    }
  } else {
    void run.then(
      () => undefined,
      () => undefined,
    )
  }
}

/** The command hook's stdin for a `PreToolUse`, rebuilt from the envelope. */
export function preToolUseBody(e: any): Record<string, unknown> {
  const { tool, tool_use_id, agentId, ...tool_input } = e ?? {}
  void agentId
  return { hook_event_name: 'PreToolUse', tool_name: tool, tool_use_id, tool_input }
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
      if ((await bridgeOnce($, c)) === BRIDGE_REFUSED_EXIT) return
      if ((await $.clock.now()) - born >= HEALTHY_MS) backoff = BACKOFF_FIRST_MS
      await $.clock.sleep(backoff)
      backoff = Math.min(backoff * 2, BACKOFF_MAX_MS)
    }
  } catch {
    // The host went (the module unloading): so does the bridge.
  } finally {
    bridgeOn = false
  }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    // `session.start` may come again in the same process (a `/clear`); one
    // bridge serves them all.
    const c = await settings($)
    if (!bridgeOn && c) {
      bridgeOn = true
      void bridgeLoop($, c)
    }
    return started
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
    if (PRE_TOOL_USE_TOOLS.includes(tool)) void relay($, 'PreToolUse', undefined, preToolUseBody(e), false)
    return next(e)
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
  on('classic.PermissionRequest', relaySingle)
  on('classic.PermissionDenied', relaySingle)
  on('classic.Notification', relaySingle)
  on('classic.Elicitation', relaySingle)
  on('classic.ElicitationResult', relaySingle)
  on('classic.PreCompact', relaySingle)
  on('classic.PostCompact', relaySingle)
}
