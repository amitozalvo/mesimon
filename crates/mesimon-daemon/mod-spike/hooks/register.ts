// T-573 research spike. A throwaway mod that OBSERVES every event mesimon's
// generated hook set observes today, and tries each replacement the ticket
// names, so each can be proven or refuted on the real Claude Code. It holds
// no policy and ships to nobody: `MESIMON_MOD_DIR` loads it, nothing else.
//
// Doctrine it keeps even as a spike (README promise 3): it never calls
// `$.session.append`, never attaches `context` on `prompt.submit`, never
// hooks `prompt.compose`/`prompt.context`, and never rewrites a prompt's
// words. Every prompt it submits is `asUser: true`.
//
// Environment (set by the driver or by the daemon's seam):
//   MESIMON_MOD_LOG         a directory: one JSON file per event (the record)
//   MESIMON_MOD_SPOOL       a directory `bridge.py` tails: commands down
//   MESIMON_MOD_BRIEF_FILE  a file submitted `asUser` at session.start (row 2)
//   MESIMON_MOD_GATE_BOARD  the board dir the gate refuses writes under (row 5)
//   MESIMON_MOD_RELAY_SOCK  + MESIMON_MOD_RELAY_BIN + MESIMON_MOD_SESSION:
//                           relay every classic event through the real
//                           `mesimon hook` binary (row 1's shadow road)
import type { Register } from 'claude-code'

type Answer = { answers: Record<string, string>; response?: string }
type Decision = 'allow' | 'ask' | 'deny'
type Command =
  | { kind: 'submit'; text: string; asUser?: boolean }
  | { kind: 'answer'; answers: Record<string, string>; response?: string }
  | { kind: 'plan'; mode: 'native' | 'result' | 'allow' }
  | { kind: 'check'; tool: string; decision: Decision | null }
  | { kind: 'usage' }
  | { kind: 'tools' }
  | { kind: 'ask'; question: string; options: string[] }
  | { kind: 'hold_permits'; on: boolean }
  | { kind: 'hold_check'; ms: number }

const LONG = 4000

/** Strings past LONG are cut; the record keeps their length. */
const trim = (_key: string, value: unknown) =>
  typeof value === 'string' && value.length > LONG
    ? `${value.slice(0, LONG)}…[${value.length} chars]`
    : value

let seq = 0
let logDir: Promise<string | undefined> | undefined
const holds = new Map<string, (a: Answer) => void>()
const checks = new Map<string, Decision>()
let planMode: 'native' | 'result' | 'allow' = 'native'
let holdPermits = false
let holdCheckMs = 0
let cwd = ''

async function log($: any, event: string, data: unknown) {
  logDir ??= $.env.get('MESIMON_MOD_LOG')
  const dir = await logDir
  if (!dir) return
  const n = ++seq
  const t = await $.clock.now()
  const name = `${dir}/${String(n).padStart(5, '0')}-${event.replace(/[^A-Za-z0-9._-]/g, '_')}.json`
  try {
    await $.fs.write(name, JSON.stringify({ seq: n, t, event, data }, trim, 1))
  } catch {
    // Nowhere to say so.
  }
}

async function submit($: any, text: string, asUser: boolean, why: string) {
  const t0 = await $.clock.now()
  await log($, 'prompt.submit.call', { why, asUser, chars: text.length })
  try {
    const r = await $.prompt.submit(asUser ? { text, asUser: true } : { text })
    await log($, 'prompt.submit.resolved', { why, ms: (await $.clock.now()) - t0, origin: r.origin, drop: r.drop, chars: r.text?.length })
  } catch (err) {
    await log($, 'prompt.submit.rejected', { why, error: String(err) })
  }
}

async function handle($: any, line: string) {
  let cmd: Command
  try {
    cmd = JSON.parse(line)
  } catch {
    await log($, 'bridge.bad_line', line)
    return
  }
  await log($, 'bridge.command', cmd)
  switch (cmd.kind) {
    case 'submit':
      void submit($, cmd.text, cmd.asUser !== false, 'spool')
      break
    case 'answer': {
      const first = holds.entries().next()
      if (first.done) {
        await log($, 'answer.nothing_held', cmd)
      } else {
        first.value[1]({ answers: cmd.answers, response: cmd.response })
      }
      break
    }
    case 'plan':
      planMode = cmd.mode
      break
    case 'hold_permits':
      holdPermits = cmd.on
      break
    case 'hold_check':
      holdCheckMs = cmd.ms
      break
    case 'check':
      if (cmd.decision) checks.set(cmd.tool, cmd.decision)
      else checks.delete(cmd.tool)
      break
    case 'usage':
      await log($, 'session.usage', await $.session.usage())
      break
    case 'tools':
      await log($, 'tool.list', (await $.tool.list()).map((t: any) => t.name))
      break
    case 'ask':
      try {
        await log($, 'ui.ask.answer', await $.ui.ask(cmd.question, cmd.options))
      } catch (err) {
        await log($, 'ui.ask.rejected', String(err))
      }
      break
  }
}

async function runBridge($: any, spool: string, root: string) {
  const bridge = $.process.spawn({ argv: ['python3', `${root}/bridge.py`, spool] })
  await log($, 'bridge.spawned', { spool })
  let buf = ''
  try {
    for await (const { stream, text } of bridge) {
      if (stream !== 'stdout') {
        await log($, 'bridge.stderr', text)
        continue
      }
      buf += text
      let nl: number
      while ((nl = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, nl)
        buf = buf.slice(nl + 1)
        if (line.trim()) void handle($, line)
      }
    }
    await log($, 'bridge.ended', await bridge.result)
  } catch (err) {
    await log($, 'bridge.failed', String(err))
  }
}

async function facts($: any) {
  const out: Record<string, unknown> = {}
  const take = async (k: string, f: () => Promise<unknown>) => {
    try {
      out[k] = await f()
    } catch (err) {
      out[k] = `ERR ${String(err)}`
    }
  }
  await take('id', () => $.session.id())
  await take('cwd', () => $.session.cwd())
  await take('root', () => $.session.root())
  await take('model', () => $.session.model())
  await take('turns', () => $.session.turns())
  await take('surfaces', () => $.session.surfaces())
  await take('env.CLAUDE_CONFIG_DIR', () => $.env.get('CLAUDE_CONFIG_DIR'))
  await take('env.HOME', () => $.env.get('HOME'))
  await take('env.TMUX', () => $.env.get('TMUX'))
  await take('env.TMUX_PANE', () => $.env.get('TMUX_PANE'))
  return out
}
async function agents($: any) {
  try {
    return await $.agent.list()
  } catch (err) {
    return `ERR ${String(err)}`
  }
}

export const register: Register = on => {
  // ---- Row 1: every classic event, in-process, relayed through the real
  // hook binary when asked (one `$.process.run` per event: `$.process.spawn`
  // takes stdin as one string and closes it, so a long-lived up-channel does
  // not exist in 2.1.287; see the block).
  on('classic.*', async ($, e, next) => {
    const name = next.event
    const t0 = await $.clock.now()
    await log($, name, e)
    const sock = await $.env.get('MESIMON_MOD_RELAY_SOCK')
    if (sock) {
      const bin = (await $.env.get('MESIMON_MOD_RELAY_BIN')) ?? 'mesimon'
      const session = (await $.env.get('MESIMON_MOD_SESSION')) ?? ''
      const event = name.slice('classic.'.length)
      const input: any = e
      const reason =
        event === 'SessionStart' ? input.source
        : event === 'SessionEnd' ? input.reason
        : event === 'StopFailure' ? input.error
        : undefined
      const argv = [bin, 'hook', '--sock', sock, '--session', session, '--event', event]
      if (typeof reason === 'string') argv.push('--reason', reason)
      void $.process
        .run(argv, { stdin: JSON.stringify(e), timeoutMs: 5000 })
        .then(
          async r => log($, 'relay.done', { event, ms: (await $.clock.now()) - t0, exitCode: r.exitCode, stderr: r.stderr }),
          async err => log($, 'relay.failed', { event, error: String(err) }),
        )
    }
    return next(e)
  })

  // ---- Engine events the hook set has no name for.
  on('session.start', async ($, e, next) => {
    cwd = e.cwd
    let version: unknown
    try {
      version = await $.session.version()
    } catch (err) {
      version = String(err)
    }
    await log($, 'session.start', { e, version, root: $.plugin.root, facts: await facts($), origin: next.origin })
    // Row 7: a tool registered in-process, no MCP shim.
    try {
      const reg = await $.tool.register({
        name: 'spike_ping',
        description: 'Answers pong with the note given. A spike tool, nothing more.',
        inputSchema: { type: 'object', properties: { note: { type: 'string' } } },
      })
      await log($, 'tool.register', reg)
    } catch (err) {
      await log($, 'tool.register.failed', String(err))
    }
    const started = await next(e)
    await log($, 't651.session.start.result', started)
    // Row 2: the launch brief, submitted as the person's words, no tty.
    const brief = await $.env.get('MESIMON_MOD_BRIEF_FILE')
    if (brief) {
      const text = await $.fs.read(brief)
      void submit($, text, true, 'launch')
    }
    const spool = await $.env.get('MESIMON_MOD_SPOOL')
    if (spool) void runBridge($, spool, $.plugin.root)
    return started
  })
  on('session.end', async ($, e, next) => {
    await log($, 'session.end', { e, facts: await facts($), budget: next.budget })
    return next(e)
  })
  on('session.compact', async ($, e, next) => {
    await log($, 'session.compact', { trigger: e.trigger, agentId: e.agentId, messages: e.messages.length, instructions: e.instructions, keys: Object.keys(e), facts: await facts($) })
    const r = await next(e)
    await log($, 'session.compact.result', { keys: Object.keys(r as any), skip: (r as any).skip, messages: (r as any).messages?.length, tokensBefore: (r as any).tokensBefore, tokensAfter: (r as any).tokensAfter, facts: await facts($) })
    return r
  })
  on('agent.spawn', async ($, e, next) => {
    await log($, 'agent.spawn', e)
    const r = await next(e)
    await log($, 'agent.spawn.result', { r, agents: await agents($) })
    return r
  })
  on('turn.start', async ($, e, next) => {
    await log($, 'turn.start', { e: { ...e, text: e.text.slice(0, 200), chars: e.text.length }, facts: await facts($), agents: await agents($) })
    const r = await next(e)
    await log($, 'turn.start.result', r)
    return r
  })
  on('turn.complete', async ($, e, next) => {
    const r = await next(e)
    await log($, 'turn.complete', { ...e, answer: String(e.answer).slice(0, 200), result: { ...(r as any), text: String((r as any)?.text ?? '').slice(0, 100) }, agents: await agents($), facts: await facts($) })
    return r
  })
  // Row 8: what each request cost, as the API reported it.
  on('turn.step', async function* ($, e, next) {
    const r = yield* next(e)
    await log($, 'turn.step', { turnId: e.turnId, index: e.index, model: e.model, agentId: e.agentId, usage: r.usage, stopReason: r.stopReason, tools: r.toolUses.map(t => t.name) })
    return r
  })
  on('prompt.submit', async ($, e, next) => {
    await log($, 'prompt.submit', { origin: e.origin, turnId: e.turnId, wait: e.wait, chars: e.text.length, head: e.text.slice(0, 80), context: e.context, attachments: e.attachments, keys: Object.keys(e) })
    const r = await next(e)
    await log($, 'prompt.submit.result', { origin: r.origin, drop: r.drop, chars: r.text?.length, context: r.context })
    return r
  })
  on('ui.render', { component: 'UserMessage' }, async ($, e, next) => {
    await log($, 'ui.render.UserMessage', { requestId: e.requestId, origin: (e.props as any).origin, from: (e.props as any).from, text: String((e.props as any).text).slice(0, 160) })
    return next(e)
  })
  on('ui.render', { component: 'AskUserQuestion' }, async ($, e, next) => {
    await log($, 'ui.render.AskUserQuestion', { requestId: e.requestId, surface: e.surface, props: e.props })
    return next(e)
  })

  // ---- Row 4: the permission verdict.
  on('tool.check', async ($, e, next) => {
    const t0 = await $.clock.now()
    if (holdCheckMs > 0 && e.tool !== 'AskUserQuestion' && e.tool !== 'ExitPlanMode') {
      const ms = holdCheckMs
      holdCheckMs = 0
      await log($, 't651.tool.check.hold', { tool: e.tool, tool_use_id: e.tool_use_id, ms })
      try {
        await $.process.run(['sleep', String(ms / 1000)], { timeoutMs: ms + 5000 })
      } catch (err) {
        await log($, 't651.tool.check.hold_failed', String(err))
      }
    }
    const core = await next(e)
    const want = checks.get(e.tool) ?? (e.tool === 'ExitPlanMode' && planMode === 'allow' ? 'allow' : undefined)
    await log($, 'tool.check', { tool: e.tool, input: e.input, tool_use_id: e.tool_use_id, core, override: want, origin: next.origin, keys: Object.keys(e), ms: (await $.clock.now()) - t0, trace: (next as any).trace })
    return want ? { decision: want, reason: `mesimon spike said ${want}` } : core
  })

  // ---- Row 4, the one-shot allow beside an open dialog: the in-process twin
  // of `mesimon approve`. The hold sits inside a `$.process.run` (a `$` call
  // in flight does not count against the hook's 10 s budget; a promise of
  // the hook's own would), which waits for the daemon's decision (here: a
  // file under the spool) and prints it, as the approve binary answers.
  on('classic.PermissionRequest', async ($, e, next) => {
    await log($, 't651.classic.PermissionRequest.agents', { agents: await agents($), facts: await facts($) })
    const spool = await $.env.get('MESIMON_MOD_SPOOL')
    if (!holdPermits || !spool || e.agent_id || e.tool_name === 'AskUserQuestion' || e.tool_name === 'ExitPlanMode') {
      return next(e)
    }
    const t0 = await $.clock.now()
    await log($, 'permit.hold', { tool: e.tool_name, input: e.tool_input, suggestions: e.permission_suggestions })
    let r: { exitCode: number; stdout: string; stderr: string }
    try {
      r = await $.process.run(['python3', `${$.plugin.root}/wait.py`, `${spool}/permits`], { timeoutMs: 45000 })
    } catch (err) {
      await log($, 'permit.wait_failed', { ms: (await $.clock.now()) - t0, error: String(err), aborted: next.signal.aborted })
      return next(e)
    }
    await log($, 'permit.wait_done', { ms: (await $.clock.now()) - t0, exitCode: r.exitCode, stdout: r.stdout, aborted: next.signal.aborted })
    if (r.stdout.trim()) {
      const decision = JSON.parse(r.stdout)
      await log($, 'permit.answered', { decision, aborted: next.signal.aborted })
      return { decision }
    }
    return next(e)
  })

  // ---- Row 3: the question, held until the daemon (here: the spool) or the
  // person answers, whichever is first.
  on('tool.call', { tool: 'AskUserQuestion' }, async ($, e, next) => {
    const t0 = await $.clock.now()
    const id = e.tool_use_id
    await log($, 'ask.call', { tool_use_id: id, agentId: e.agentId, questions: e.questions })
    let settle: (a: Answer) => void = () => {}
    const remote = new Promise<Answer>(resolve => { settle = resolve })
    holds.set(id, settle)
    const native = next(e).then(
      r => ({ who: 'native' as const, r }),
      err => ({ who: 'native_rejected' as const, err: String(err) }),
    )
    const spool = remote.then(a => ({ who: 'spool' as const, a }))
    const first = await Promise.race([native, spool])
    holds.delete(id)
    const ms = (await $.clock.now()) - t0
    if (first.who === 'spool') {
      const result: any = { questions: e.questions, answers: first.a.answers }
      if (first.a.response) result.response = first.a.response
      await log($, 'ask.answered_by_spool', { ms, result })
      void native.then(n => log($, 'ask.native_after_spool', n))
      return { result }
    }
    if (first.who === 'native') {
      await log($, 'ask.native', { ms, r: first.r })
      return first.r
    }
    await log($, 'ask.native_rejected', { ms, err: first.err })
    throw new Error(first.err)
  })

  // ---- Row 5: the gate, local and static. No daemon is asked.
  on('tool.call', { tool: ['Write', 'Edit', 'NotebookEdit'] }, async ($, e, next) => {
    const tool = e.tool
    const input: any = e
    const path: unknown = input.file_path ?? input.notebook_path
    // The guarded root is given, as `mesimon gate` gets `--deny-board`;
    // the cwd's `.mesimon` stands in while nothing names one.
    const board = (await $.env.get('MESIMON_MOD_GATE_BOARD')) ?? `${cwd}/.mesimon`
    const root = board.endsWith('/') ? board : `${board}/`
    if (typeof path === 'string' && (path.startsWith(root) || path === root.slice(0, -1))) {
      await log($, 'gate.deny', { tool, path })
      return { deny: `mesimon: ${path} is board state under .mesimon; the board writes it, an agent does not.` }
    }
    return next(e)
  })

  // ---- Row 6: the plan dialog.
  on('tool.call', { tool: 'ExitPlanMode' }, async ($, e, next) => {
    await log($, 'plan.call', { mode: planMode, e })
    if (planMode === 'result') {
      const input: any = e
      const r = { result: { plan: input.plan ?? null, isAgent: false, filePath: input.planFilePath } }
      await log($, 'plan.answered_by_mod', r)
      return r as any
    }
    const r = await next(e)
    await log($, 'plan.native', r)
    return r
  })

  // ---- T-651: the native events that pass the security default, measured
  // for the hook set's facts. Observe only: every hook returns next(e) as it
  // came. `facts` is what `$.session` answers at that moment.
  on('session.measure', async ($, e, next) => {
    await log($, 't651.session.measure', e)
    return next(e)
  })
  on('session.receive', async ($, e, next) => {
    await log($, 't651.session.receive', { ...e, text: String(e.text).slice(0, 300) })
    const r = await next(e)
    await log($, 't651.session.receive.result', { keys: Object.keys(r as any), consumed: (r as any).consumed })
    return r
  })
  // Every tool call: the envelope, the tool's own keys, and the result's shape.
  on('tool.call', async ($, e, next) => {
    const { tool, tool_use_id, agentId, consent, ...input } = e as any
    const t0 = await $.clock.now()
    await log($, 't651.tool.call', { tool, tool_use_id, agentId, consent, input, origin: next.origin })
    let r: any
    try {
      r = await next(e)
    } catch (err) {
      await log($, 't651.tool.call.threw', { tool, tool_use_id, agentId, ms: (await $.clock.now()) - t0, error: String(err), aborted: next.signal.aborted })
      throw err
    }
    const shape: any = { tool, tool_use_id, agentId, ms: (await $.clock.now()) - t0, keys: Object.keys(r ?? {}), deny: r?.deny, isError: r?.isError, isReadOnly: r?.isReadOnly }
    shape.result = r?.result
    shape.text = typeof r?.text === 'string' ? r.text.slice(0, 400) : r?.text
    await log($, 't651.tool.call.result', shape)
    return r
  })
  on('classic.Stop', async ($, e, next) => {
    await log($, 't651.classic.Stop.agents', { agent_id: (e as any).agent_id, agents: await agents($), facts: await facts($) })
    return next(e)
  })
  on('classic.SubagentStop', async ($, e, next) => {
    await log($, 't651.classic.SubagentStop.agents', { agent_id: (e as any).agent_id, agents: await agents($), facts: await facts($) })
    return next(e)
  })
  on('classic.SubagentStart', async ($, e, next) => {
    await log($, 't651.classic.SubagentStart.agents', { agent_id: (e as any).agent_id, agents: await agents($), facts: await facts($) })
    return next(e)
  })
  on('classic.TeammateIdle', async ($, e, next) => {
    await log($, 't651.classic.TeammateIdle.agents', { agent_id: (e as any).agent_id, agents: await agents($), facts: await facts($) })
    return next(e)
  })
  on('classic.Notification', async ($, e, next) => {
    await log($, 't651.classic.Notification.agents', { agent_id: (e as any).agent_id, agents: await agents($), facts: await facts($) })
    return next(e)
  })
  on('classic.StopFailure', async ($, e, next) => {
    await log($, 't651.classic.StopFailure.agents', { agent_id: (e as any).agent_id, agents: await agents($), facts: await facts($) })
    return next(e)
  })

  // ---- Row 7: serving the registered tool.
  on('tool.call', { tool: 'mcp__mesimon-spike__spike_ping' }, async ($, e, next) => {
    await log($, 'spike_ping.call', e)
    return { result: `pong ${(e as any).note ?? ''}`.trim() } as any
  })

}
