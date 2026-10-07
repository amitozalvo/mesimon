// `claude plugin test`: the mod's hooks against the engine's own `$`, with
// the test's hooks beneath standing in for the host (`process.run`,
// `process.spawn`). What the live Claude Code does is measured in STALE-MAP
// (T-573, T-574); these hold the hooks' shapes.
import { expect, mock, test } from 'claude-code/testing'
import { rowNotified } from './register'

const ENV = {
  MESIMON_MOD_BIN: '/bin/mesimon',
  MESIMON_MOD_HOOK_SOCK: '/rt/hook.sock',
  MESIMON_MOD_ORCH_SOCK: '/rt/orch.sock',
  MESIMON_MOD_SESSION: '00000000-0000-0000-0000-000000000001',
}

type Run = { argv: readonly string[]; stdin: string }

/** Every `$.process.run` the mod makes, answered as a quiet success. */
function recordRuns(on: any): Run[] {
  const runs: Run[] = []
  on('process.run', ($: any, e: any) => {
    runs.push({ argv: e.argv, stdin: e.init?.stdin ?? '' })
    return { value: { exitCode: 0, stdout: '', stderr: '' } }
  })
  return runs
}

const settle = () => new Promise(resolve => setTimeout(resolve, 20))

test('a classic event goes up through mesimon hook on the mod road, unchanged', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  on('classic.Stop', () => ({}) as any)
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(runs.length).toBe(1)
  expect(runs[0].argv).toEqual([
    '/bin/mesimon', 'hook', '--sock', '/rt/hook.sock',
    '--session', ENV.MESIMON_MOD_SESSION, '--event', 'Stop', '--road', 'mod',
  ])
  expect(JSON.parse(runs[0].stdin).stop_hook_active).toBe(false)
})

test('the matchers are the hook set\'s: the reason is the matched word, the rest is not relayed', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  on('classic.SessionStart', () => ({}) as any)
  on('classic.StopFailure', () => ({}) as any)
  await $.classic.SessionStart({ source: 'resume' } as any)
  await $.classic.StopFailure({ error: 'rate_limit' } as any)
  await $.classic.StopFailure({ error: 'a_class_this_build_never_registered' } as any)
  await settle()
  expect(runs.map(r => r.argv.slice(7, 10))).toEqual([
    ['SessionStart', '--reason', 'resume'],
    ['StopFailure', '--reason', 'rate_limit'],
  ])
})

test('PreToolUse is relayed for the two dialog tools only, rebuilt as the hook\'s stdin', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  on('tool.call', () => ({ result: { questions: [], answers: {} } }) as any)
  const questions = [{ question: 'Which?', header: 'W', options: [{ label: 'A', description: 'a' }, { label: 'B', description: 'b' }], multiSelect: false }]
  await $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'toolu_1', questions } as any)
  await $.tool.call({ tool: 'Read', tool_use_id: 'toolu_2', file_path: '/x' } as any)
  await settle()
  const pre = runs.filter(r => r.argv.includes('PreToolUse'))
  expect(pre.length).toBe(1)
  const body = JSON.parse(pre[0].stdin)
  expect(body.tool_name).toBe('AskUserQuestion')
  expect(body.tool_use_id).toBe('toolu_1')
  expect(body.tool_input).toEqual({ questions })
})

test('a relay that fails never stops the event', async ($, on) => {
  mock.env(on, ENV)
  on('process.run', () => {
    throw new Error('no such binary')
  })
  let reached = 0
  on('classic.UserPromptSubmit', () => {
    reached += 1
    return {} as any
  })
  on('classic.SessionEnd', () => {
    reached += 1
    return {} as any
  })
  await $.classic.UserPromptSubmit({ prompt: 'go' } as any)
  await $.classic.SessionEnd({ reason: 'other' } as any)
  expect(reached).toBe(2)
})

test('without the pane\'s variables nothing is relayed', async ($, on) => {
  mock.env(on, {})
  const runs = recordRuns(on)
  on('classic.Stop', () => ({}) as any)
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(runs.length).toBe(0)
})

/** `$.env.get` from ENV, but the first `fails` reads throw (an abandoned dispatch's). */
function envFailing(on: any, fails: number) {
  on('env.get', ($: any, e: any) => {
    if (fails > 0) {
      fails -= 1
      throw new Error('the dispatch was abandoned')
    }
    return { value: (ENV as Record<string, string>)[e.name] }
  })
}

test('a read of the variables that fails is not kept: the next event reads again and says so', async ($, on) => {
  envFailing(on, 1)
  const runs = recordRuns(on)
  on('classic.SessionStart', () => ({}) as any)
  on('classic.Stop', () => ({}) as any)
  await $.classic.SessionStart({ source: 'startup' } as any)
  await settle()
  expect(runs.length).toBe(0)
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  const events = runs.map(r => r.argv[r.argv.indexOf('--event') + 1])
  expect(events.sort()).toEqual(['ModLoadFailed', 'Stop'])
  const report = runs.find(r => r.argv.includes('ModLoadFailed'))!
  expect(report.argv[report.argv.indexOf('--reason') + 1]).toBe('recovered')
  const body = JSON.parse(report.stdin)
  expect(body.reads).toBe(1)
  expect(body.at).toBe('SessionStart')
  expect(typeof body.error).toBe('string')
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(runs.filter(r => r.argv.includes('ModLoadFailed')).length).toBe(1)
})

test('a session.start whose read failed leaves the bridge to the next event that reads', async ($, on) => {
  envFailing(on, 1)
  mock.clock(on)
  recordRuns(on)
  let spawns = 0
  on('process.spawn', async function* () {
    spawns += 1
    return { value: { code: 3, signal: null } }
  } as any)
  on('session.start', () => ({ cwd: '/repo' }) as any)
  on('classic.Stop', () => ({}) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  expect(spawns).toBe(0)
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(spawns).toBe(1)
  // Refused for good: no later event starts another.
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(spawns).toBe(1)
})

test('the bridge starts at session.start and a ping comes back as a pong, once', async ($, on) => {
  mock.env(on, ENV)
  mock.clock(on)
  const runs = recordRuns(on)
  const spawned: (readonly string[])[] = []
  on('process.spawn', async function* ($: any, e: any) {
    spawned.push(e.argv)
    yield { stream: 'stdout', text: '{"id":"01A","kind":"ping"}\n{"id":"01A","kind":"ping"}\n{"id":"01B","kin' }
    yield { stream: 'stdout', text: 'd":"ping"}\n' }
    return { value: { code: 3, signal: null } }
  } as any)
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  expect(spawned).toEqual([[
    '/bin/mesimon', 'mod-bridge', '--sock', '/rt/orch.sock', '--session', ENV.MESIMON_MOD_SESSION,
    '--speaks', 'ping,submit,answer,fill',
  ]])
  const pongs = runs.filter(r => r.argv.includes('ModPong')).map(r => r.argv[r.argv.indexOf('--reason') + 1])
  expect(pongs).toEqual(['01A', '01B'])
})

test('a bridge that dies is respawned after a second; one refused for good is not', async ($, on) => {
  mock.env(on, ENV)
  recordRuns(on)
  const clock = mock.clock(on)
  const codes = [0, 3]
  let spawns = 0
  on('process.spawn', async function* () {
    spawns += 1
    return { value: { code: codes[spawns - 1] ?? 3, signal: null } }
  } as any)
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  expect(spawns).toBe(1)
  await clock.advance(999)
  expect(spawns).toBe(1)
  await clock.advance(1)
  await settle()
  expect(spawns).toBe(2)
  await clock.advance(120000)
  await settle()
  expect(spawns).toBe(2)
})

/** A bridge whose stdout is `lines`, then refused for good (not respawned). */
function bridgeSaying(on: any, lines: string[]) {
  mock.clock(on)
  on('process.spawn', async function* () {
    yield { stream: 'stdout', text: lines.map(l => l + '\n').join('') }
    return { value: { code: 3, signal: null } }
  } as any)
}

const reasonOf = (r: Run) => r.argv[r.argv.indexOf('--reason') + 1]

test('a submit is the person\'s own prompt, whole, and its end is reported', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  const got: any[] = []
  on('prompt.submit', ($: any, e: any) => {
    got.push({ text: e.text, origin: e.origin, context: e.context })
    return { text: e.text }
  })
  const brief = 'R1 · a title\n\nLine one of the brief.\n\nAnd its last line.'
  bridgeSaying(on, [JSON.stringify({ id: '01S', kind: 'submit', text: brief })])
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  expect(got.length).toBe(1)
  expect(got[0].text).toBe(brief)
  expect(got[0].origin.asUser).toBe(true)
  expect(got[0].context).toBe(undefined)
  const reports = runs.filter(r => r.argv.includes('ModSubmit'))
  expect(reports.map(reasonOf)).toEqual(['01S'])
  expect(JSON.parse(reports[0].stdin)).toEqual({ outcome: 'entered' })
})

test('a submit the engine drops is reported dropped, with its reason', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  on('prompt.submit', () => ({ drop: 'blocked by a hook' }) as any)
  bridgeSaying(on, [JSON.stringify({ id: '01D', kind: 'submit', text: 'go' })])
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  const reports = runs.filter(r => r.argv.includes('ModSubmit'))
  expect(JSON.parse(reports[0].stdin)).toEqual({ outcome: 'dropped', reason: 'blocked by a hook' })
})

test('a fill puts the words, whole, in an empty composer and says so (T-601)', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  const filled: any[] = []
  on('prompt.read', () => ({ value: { text: '', cursor: 0 } }) as any)
  on('prompt.fill', ($: any, e: any) => {
    filled.push({ text: e.text, mode: e.mode })
    return { isFilled: true } as any
  })
  const words = 'Stop: the rebrief.\n\nKeep the tests.'
  bridgeSaying(on, [JSON.stringify({ id: '01F', kind: 'fill', text: words })])
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  expect(filled).toEqual([{ text: words, mode: 'replace' }])
  const reports = runs.filter(r => r.argv.includes('ModFill'))
  expect(reports.map(reasonOf)).toEqual(['01F'])
  expect(JSON.parse(reports[0].stdin)).toEqual({ outcome: 'filled' })
})

test('a fill never replaces a person\'s draft (T-601)', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  let fills = 0
  on('prompt.read', () => ({ value: { text: 'half a thought', cursor: 14 } }) as any)
  on('prompt.fill', () => {
    fills += 1
    return { isFilled: true } as any
  })
  bridgeSaying(on, [JSON.stringify({ id: '01G', kind: 'fill', text: 'go' })])
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  expect(fills).toBe(0)
  const reports = runs.filter(r => r.argv.includes('ModFill'))
  expect(JSON.parse(reports[0].stdin)).toEqual({ outcome: 'refused', reason: 'draft' })
})

test('a fill the engine does not take is reported refused, with a reason (T-601)', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  on('prompt.read', () => ({ value: { text: '', cursor: 0 } }) as any)
  // The engine's own refusal word (`dialog`, `no_composer`) is its core's;
  // a test hook beneath can only say the box was not filled.
  on('prompt.fill', () => ({ isFilled: false }) as any)
  bridgeSaying(on, [JSON.stringify({ id: '01H', kind: 'fill', text: 'go' })])
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  const reports = runs.filter(r => r.argv.includes('ModFill'))
  expect(reports.map(reasonOf)).toEqual(['01H'])
  expect(JSON.parse(reports[0].stdin)).toEqual({ outcome: 'refused', reason: 'not_filled' })
})

const QUESTIONS = [{
  question: 'Which colour?', header: 'Colour', multiSelect: false,
  options: [{ label: 'red', description: '' }, { label: 'blue', description: '' }],
}]

test('the board\'s answer closes a held question in the dialog\'s place, and says so', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  // The native dialog, standing until it is aborted: the board answers first.
  on('tool.call', () => new Promise(() => undefined) as any)
  bridgeSaying(on, [JSON.stringify({ id: '01Q', kind: 'answer', tool_use_id: 'toolu_9', answers: { 'Which colour?': 'blue' } })])
  on('session.start', () => ({ cwd: '/repo' }) as any)
  const call = $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'toolu_9', questions: QUESTIONS } as any)
  await settle()
  await $.session.start({ cwd: '/repo' } as any)
  const r: any = await call
  expect(r.result).toEqual({ questions: QUESTIONS, answers: { 'Which colour?': 'blue' } })
  await settle()
  const reports = runs.filter(r => r.argv.includes('ModAnswer'))
  expect(reports.map(reasonOf)).toEqual(['answered'])
  const body = JSON.parse(reports[0].stdin)
  expect(body.hook_event_name).toBe('PostToolUse')
  expect(body.tool_name).toBe('AskUserQuestion')
  expect(body.tool_use_id).toBe('toolu_9')
  expect(body.tool_response.answers).toEqual({ 'Which colour?': 'blue' })
})

test('the person\'s answer first wins, and a late answer finds nothing held', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  const native = { result: { questions: QUESTIONS, answers: { 'Which colour?': 'red' } } }
  on('tool.call', () => native as any)
  bridgeSaying(on, [JSON.stringify({ id: '01L', kind: 'answer', tool_use_id: 'toolu_7', answers: { 'Which colour?': 'blue' } })])
  on('session.start', () => ({ cwd: '/repo' }) as any)
  const r: any = await $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'toolu_7', questions: QUESTIONS } as any)
  expect(r.result.answers).toEqual({ 'Which colour?': 'red' })
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  const reports = runs.filter(r => r.argv.includes('ModAnswer'))
  expect(reports.map(reasonOf)).toEqual(['nothing_held'])
})

test('a subagent\'s question is never held', async ($, on) => {
  mock.env(on, ENV)
  recordRuns(on)
  let native = 0
  on('tool.call', () => {
    native += 1
    return { result: { questions: QUESTIONS, answers: {} } } as any
  })
  await $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'toolu_5', agentId: 'a1', questions: QUESTIONS } as any)
  expect(native).toBe(1)
})

test('a question the person refuses is reported declined, and its result stands', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  on('tool.call', () => ({ isError: true, result: undefined, text: 'The user declined.' }) as any)
  const r: any = await $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'toolu_3', questions: QUESTIONS } as any)
  expect(r.isError).toBe(true)
  await settle()
  const reports = runs.filter(r => r.argv.includes('ModAnswer'))
  expect(reports.map(reasonOf)).toEqual(['declined'])
  expect(JSON.parse(reports[0].stdin).tool_use_id).toBe('toolu_3')
})

// ---- The gate (T-577): `mesimon gate`'s rules, judged in-process.

const GATE_ENV = {
  ...ENV,
  MESIMON_MOD_GATE_BOARD: '/repo/.mesimon',
  MESIMON_MOD_GATE_STATE: '/state',
  MESIMON_MOD_GATE_ALLOW: '/state/worktrees',
}

/**
 * The disk as `$.fs.stat` answers it: the folders that exist, and links
 * (a spelling and where it lands). Anything else is missing.
 */
function disk(on: any, dirs: string[], links: Record<string, string> = {}) {
  on('fs.stat', ($: any, e: any) => {
    const path: string = e.path
    for (const [link, to] of Object.entries(links)) {
      if (path === link || path.startsWith(`${link}/`)) {
        const real = to + path.slice(link.length)
        if (dirs.includes(real)) return { value: { kind: 'dir', size: 0, mtimeMs: 0, isLink: path === link, realPath: real } }
      }
    }
    if (dirs.includes(path)) return { value: { kind: 'dir', size: 0, mtimeMs: 0, isLink: false, realPath: path } }
    throw new Error(`ENOENT: ${path}`)
  })
}

const DIRS = ['/', '/repo', '/repo/src', '/repo/.mesimon', '/repo/.mesimon/board', '/state', '/state/worktrees', '/state/worktrees/T-1-x']

/** The native tool beneath: counts the calls that reached it. */
function nativeWrites(on: any) {
  const reached: string[] = []
  on('tool.call', ($: any, e: any) => {
    reached.push(e.file_path ?? e.notebook_path)
    return { result: { type: 'create', filePath: e.file_path } }
  })
  return reached
}

const BOARD_WORDS =
  'mesimon owns .mesimon/ — the board is edited through mesimon, not by writing its files. Use mesimon\'s scoped MCP tools instead.'
const STATE_WORDS =
  'mesimon owns its state directory — sessions, worktree bindings and hook settings are not editable by an agent.'

test('a write under the board dir is refused in mesimon gate\'s words, then reported', async ($, on) => {
  mock.env(on, GATE_ENV)
  const runs = recordRuns(on)
  disk(on, DIRS)
  const reached = nativeWrites(on)
  const r: any = await $.tool.call({ tool: 'Write', tool_use_id: 't1', file_path: '/repo/.mesimon/board/columns.toml', content: 'x' } as any)
  expect(r.deny).toBe(BOARD_WORDS)
  expect(reached).toEqual([])
  await settle()
  const gate = runs.filter(r => r.argv.includes('GateDenied'))
  expect(gate.map(reasonOf)).toEqual(['board_dir'])
  expect(gate[0].argv).toContain('mod')
  expect(JSON.parse(gate[0].stdin)).toEqual({ file_path: '/repo/.mesimon/board/columns.toml' })
})

test('the state dir is refused, its worktrees and ordinary source are not', async ($, on) => {
  mock.env(on, GATE_ENV)
  recordRuns(on)
  disk(on, DIRS)
  const reached = nativeWrites(on)
  const state: any = await $.tool.call({ tool: 'Edit', tool_use_id: 't2', file_path: '/state/sessions.json', old_string: 'a', new_string: 'b' } as any)
  expect(state.deny).toBe(STATE_WORDS)
  for (const file_path of ['/state/worktrees/T-1-x/src/main.rs', '/repo/src/main.rs', '/repo/.mesimon-notes/x.md']) {
    const r: any = await $.tool.call({ tool: 'Write', tool_use_id: 't3', file_path, content: 'x' } as any)
    expect(r.deny).toBe(undefined)
  }
  expect(reached).toEqual(['/state/worktrees/T-1-x/src/main.rs', '/repo/src/main.rs', '/repo/.mesimon-notes/x.md'])
  await settle()
})

test('.. cannot walk in sideways, and a link into the board dir is followed', async ($, on) => {
  mock.env(on, GATE_ENV)
  recordRuns(on)
  disk(on, DIRS, { '/repo/src/board-link': '/repo/.mesimon' })
  const reached = nativeWrites(on)
  for (const file_path of ['/repo/src/../.mesimon/board/new.toml', '/repo/src/board-link/board/new.toml']) {
    const r: any = await $.tool.call({ tool: 'Write', tool_use_id: 't4', file_path, content: 'x' } as any)
    expect(r.deny).toBe(BOARD_WORDS)
  }
  expect(reached).toEqual([])
  await settle()
})

test('a notebook is judged by its notebook_path', async ($, on) => {
  mock.env(on, GATE_ENV)
  recordRuns(on)
  disk(on, DIRS)
  nativeWrites(on)
  const r: any = await $.tool.call({ tool: 'NotebookEdit', tool_use_id: 't5', notebook_path: '/repo/.mesimon/n.ipynb', new_source: 'x' } as any)
  expect(r.deny).toBe(BOARD_WORDS)
  await settle()
})

test('a dead daemon still denies: the decision asks nothing of it', async ($, on) => {
  mock.env(on, GATE_ENV)
  on('process.run', () => {
    throw new Error('connection refused')
  })
  disk(on, DIRS)
  nativeWrites(on)
  const r: any = await $.tool.call({ tool: 'Write', tool_use_id: 't6', file_path: '/repo/.mesimon/x', content: 'x' } as any)
  expect(r.deny).toBe(BOARD_WORDS)
  await settle()
})

test('without the gate\'s roots a structured write is refused, never let through', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  disk(on, DIRS)
  const reached = nativeWrites(on)
  const r: any = await $.tool.call({ tool: 'Write', tool_use_id: 't7', file_path: '/repo/src/main.rs', content: 'x' } as any)
  expect(r.deny).toBe('Mesimon write guard context is unavailable')
  expect(reached).toEqual([])
  await settle()
  expect(runs.filter(r => r.argv.includes('GateDenied')).length).toBe(0)
})

// ---- The tools (T-577): registered from `mesimon mcp --list`, served by
// `mesimon mcp --call`.

const TOOLS_ENV = { ...ENV, MESIMON_MOD_TOOLS: 'read' }
const SPECS = [
  { name: 'get_ticket', description: 'Returns the ticket.', inputSchema: { type: 'object', properties: { key: { type: 'string' } } } },
  { name: 'read_attachment', description: 'Returns an image.', inputSchema: { type: 'object' } },
]

/** `mesimon mcp`: `--list` prints SPECS; `--call` prints `answer(argv, stdin)`. */
function mesimonMcp(on: any, answer: (argv: readonly string[], stdin: string) => unknown) {
  const runs: Run[] = []
  on('process.run', ($: any, e: any) => {
    runs.push({ argv: e.argv, stdin: e.init?.stdin ?? '' })
    if (e.argv[1] === 'mcp' && e.argv.includes('--list')) return { value: { exitCode: 0, stdout: JSON.stringify(SPECS) + '\n', stderr: '' } }
    if (e.argv[1] === 'mcp') return { value: { exitCode: 0, stdout: JSON.stringify(answer(e.argv, e.init?.stdin ?? '')) + '\n', stderr: '' } }
    return { value: { exitCode: 0, stdout: '', stderr: '' } }
  })
  return runs
}

test('the tier\'s tools are registered at session.start, word for word, before the bridge starts', async ($, on) => {
  mock.env(on, TOOLS_ENV)
  mock.clock(on)
  const runs = mesimonMcp(on, () => ({}))
  const got: any[] = []
  const order: string[] = []
  on('tool.register', ($: any, e: any) => {
    got.push(e)
    order.push(`register ${e.name}`)
    return { value: { tool: `mcp__mesimon__${e.name}` } }
  })
  on('process.spawn', async function* () {
    order.push('bridge')
    return { value: { code: 3, signal: null } }
  } as any)
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  // The daemon sends no word before the bridge's first poll: the tools are
  // listed before any turn it starts.
  expect(order).toEqual(['register get_ticket', 'register read_attachment', 'bridge'])
  expect(runs[0].argv).toEqual(['/bin/mesimon', 'mcp', '--list', '--tools', 'read'])
  expect(got.map(g => ({ name: g.name, description: g.description, inputSchema: g.inputSchema }))).toEqual(SPECS)
})

test('no tier, no tools', async ($, on) => {
  mock.env(on, ENV)
  mock.clock(on)
  const runs = mesimonMcp(on, () => ({}))
  let registered = 0
  on('tool.register', () => {
    registered += 1
    return { value: { tool: 'x' } }
  })
  on('process.spawn', async function* () {
    return { value: { code: 3, signal: null } }
  } as any)
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  expect(registered).toBe(0)
  expect(runs.filter(r => r.argv.includes('--list')).length).toBe(0)
})

async function startWithTools($: any, on: any) {
  mock.clock(on)
  on('tool.register', ($: any, e: any) => ({ value: { tool: `mcp__mesimon__${e.name}` } }))
  on('process.spawn', async function* () {
    return { value: { code: 3, signal: null } }
  } as any)
  on('session.start', () => ({ cwd: '/repo' }) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
}

test('a call goes to the daemon through mesimon mcp --call, and its text comes back whole', async ($, on) => {
  mock.env(on, TOOLS_ENV)
  const runs = mesimonMcp(on, () => ({ content: [{ type: 'text', text: '{\n  "key": "T-1"\n}' }], isError: false }))
  await startWithTools($, on)
  const r: any = await $.tool.call({ tool: 'mcp__mesimon__get_ticket', tool_use_id: 'toolu_1', key: 'T-1' } as any)
  expect(r.result).toBe('{\n  "key": "T-1"\n}')
  const call = runs.find(r => r.argv.includes('--call'))!
  expect(call.argv).toEqual([
    '/bin/mesimon', 'mcp', '--sock', '/rt/orch.sock', '--session', ENV.MESIMON_MOD_SESSION,
    '--call', 'get_ticket', '--tool-use-id', 'toolu_1',
  ])
  expect(JSON.parse(call.stdin)).toEqual({ key: 'T-1' })
})

test('a refusal is an error result in the daemon\'s words; an image is an image', async ($, on) => {
  mock.env(on, TOOLS_ENV)
  mesimonMcp(on, argv =>
    argv.includes('read_attachment')
      ? { content: [{ type: 'image', mimeType: 'image/png', data: 'iVBOR' }], isError: false }
      : { content: [{ type: 'text', text: 'no column named NOPE' }], isError: true },
  )
  await startWithTools($, on)
  const refused: any = await $.tool.call({ tool: 'mcp__mesimon__get_ticket', tool_use_id: 'toolu_2', key: 'NOPE' } as any)
  expect(refused.deny).toBe('no column named NOPE')
  const image: any = await $.tool.call({ tool: 'mcp__mesimon__read_attachment', tool_use_id: 'toolu_3' } as any)
  expect(image.result).toEqual([{ type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'iVBOR' } }])
})

test('a call that cannot be made is refused in plain words, never thrown', async ($, on) => {
  mock.env(on, TOOLS_ENV)
  on('process.run', ($: any, e: any) => {
    if (e.argv.includes('--list')) return { value: { exitCode: 0, stdout: JSON.stringify(SPECS), stderr: '' } }
    throw new Error('the binary is gone')
  })
  await startWithTools($, on)
  const r: any = await $.tool.call({ tool: 'mcp__mesimon__get_ticket', tool_use_id: 'toolu_4' } as any)
  expect(String(r.deny)).toContain('mesimon could not answer the call')
})

test('a tool this session did not register is not served', async ($, on) => {
  mock.env(on, TOOLS_ENV)
  const runs = mesimonMcp(on, () => ({ content: [], isError: false }))
  let native = 0
  on('tool.call', () => {
    native += 1
    return { result: 'theirs' } as any
  })
  await startWithTools($, on)
  const r: any = await $.tool.call({ tool: 'mcp__mesimon__move_ticket', tool_use_id: 'toolu_5' } as any)
  expect(r.result).toBe('theirs')
  expect(native).toBe(1)
  expect(runs.filter(r => r.argv.includes('--call')).length).toBe(0)
})

test('a gate whose roots cannot be read refuses, and the next write reads them again', async ($, on) => {
  let reads = 0
  on('env.get', ($: any, e: any) => {
    if (String(e.name ?? e.key ?? e).includes('GATE')) {
      reads += 1
      if (reads <= 3) throw new Error('the dispatch was abandoned')
    }
    return { value: (GATE_ENV as any)[e.name ?? e.key ?? e] }
  })
  recordRuns(on)
  disk(on, DIRS)
  const reached = nativeWrites(on)
  const first: any = await $.tool.call({ tool: 'Write', tool_use_id: 't8', file_path: '/repo/src/main.rs', content: 'x' } as any)
  expect(first.deny).toBe('Mesimon write guard context is unavailable')
  const second: any = await $.tool.call({ tool: 'Write', tool_use_id: 't9', file_path: '/repo/src/main.rs', content: 'x' } as any)
  expect(second.deny).toBe(undefined)
  expect(reached).toEqual(['/repo/src/main.rs'])
  await settle()
})

// ---- The frames alone (T-577): ordered, a subagent's named, and the
// permission bridge the hook set carried.

test('relays go one after another, in the order the events came', async ($, on) => {
  mock.env(on, ENV)
  const clock = mock.clock(on)
  const started: string[] = []
  let release: () => void = () => undefined
  on('process.run', ($: any, e: any) => {
    const event = e.argv[e.argv.indexOf('--event') + 1]
    started.push(event)
    // The first relay is slow; nothing after it may start before it ends.
    if (event === 'UserPromptSubmit') return new Promise(resolve => {
      release = () => resolve({ value: { exitCode: 0, stdout: '', stderr: '' } })
    }) as any
    return { value: { exitCode: 0, stdout: '', stderr: '' } }
  })
  on('classic.UserPromptSubmit', () => ({}) as any)
  on('classic.Stop', () => ({}) as any)
  await $.classic.UserPromptSubmit({ prompt: 'go' } as any)
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(started).toEqual(['UserPromptSubmit'])
  release()
  await settle()
  expect(started).toEqual(['UserPromptSubmit', 'Stop'])
  void clock
})

test('a subagent\'s PreToolUse names its agent, as the hook set\'s stdin did', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  on('tool.call', () => ({ result: { plan: 'p' } }) as any)
  await $.tool.call({ tool: 'ExitPlanMode', tool_use_id: 'toolu_6', agentId: 'a7', plan: 'p' } as any)
  await settle()
  const pre = runs.filter(r => r.argv.includes('PreToolUse'))
  expect(JSON.parse(pre[0].stdin).agent_id).toBe('a7')
})

test('a permission dialog is put to mesimon approve, and a person\'s answer from the phone is its decision', async ($, on) => {
  mock.env(on, ENV)
  const runs: Run[] = []
  let answer = '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"the phone said no"}}}'
  on('process.run', ($: any, e: any) => {
    runs.push({ argv: e.argv, stdin: e.init?.stdin ?? '' })
    return { value: { exitCode: 0, stdout: e.argv[1] === 'approve' ? answer : '', stderr: '' } }
  })
  on('classic.PermissionRequest', () => ({}) as any)
  const body = { hook_event_name: 'PermissionRequest', tool_name: 'Bash', tool_input: { command: 'touch x' } }
  const r: any = await $.classic.PermissionRequest(body as any)
  expect(r.decision).toEqual({ behavior: 'deny', message: 'the phone said no' })
  const approve = runs.find(r => r.argv[1] === 'approve')!
  expect(approve.argv).toEqual(['/bin/mesimon', 'approve', '--sock', '/rt/hook.sock', '--session', ENV.MESIMON_MOD_SESSION,
    '--hold', '540', '--renew'])
  expect(JSON.parse(approve.stdin)).toMatchObject(body)
  // No answer leaves the dialog to whatever else answers it.
  answer = ''
  const none: any = await $.classic.PermissionRequest(body as any)
  expect(none.decision).toBe(undefined)
  await settle()
  expect(runs.filter(r => r.argv.includes('PermissionRequest')).length).toBe(2)
})

test('a round that ran out with the dialog still held runs the next, and the phone\'s answer in it is taken', async ($, on) => {
  mock.env(on, ENV)
  // T-632: a mod's process lives ten minutes at most, so `mesimon approve`
  // holds in rounds; exit 75 is "the daemon still holds it", exit 0 with
  // nothing the end of the hold.
  const exits = [75, 75, 0]
  let rounds = 0
  let answer = '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}'
  on('process.run', ($: any, e: any) => {
    if (e.argv[1] !== 'approve') return { value: { exitCode: 0, stdout: '', stderr: '' } }
    rounds += 1
    const exitCode = exits.shift() ?? 0
    const stdout = exitCode === 0 && answer ? answer : ''
    return { value: { exitCode, stdout, stderr: '' } }
  })
  on('classic.PermissionRequest', () => ({}) as any)
  const body = { hook_event_name: 'PermissionRequest', tool_name: 'Bash', tool_input: { command: 'touch x' } }
  const r: any = await $.classic.PermissionRequest(body as any)
  expect(rounds).toBe(3)
  expect(r.decision).toEqual({ behavior: 'allow' })
  // Released with no answer: the hold ends, and no further round runs.
  exits.push(75, 0, 75)
  answer = ''
  rounds = 0
  const none: any = await $.classic.PermissionRequest(body as any)
  expect(rounds).toBe(2)
  expect(none.decision).toBe(undefined)
})

test('a session.start whose read failed leaves the tools to the next event that reads', async ($, on) => {
  let fails = 1
  on('env.get', ($: any, e: any) => {
    if (fails > 0) {
      fails -= 1
      throw new Error('the dispatch was abandoned')
    }
    return { value: (TOOLS_ENV as Record<string, string>)[e.name] }
  })
  mock.clock(on)
  mesimonMcp(on, () => ({}))
  const got: string[] = []
  on('tool.register', ($: any, e: any) => {
    got.push(e.name)
    return { value: { tool: `mcp__mesimon__${e.name}` } }
  })
  on('process.spawn', async function* () {
    return { value: { code: 3, signal: null } }
  } as any)
  on('session.start', () => ({ cwd: '/repo' }) as any)
  on('classic.Stop', () => ({}) as any)
  await $.session.start({ cwd: '/repo' } as any)
  await settle()
  expect(got).toEqual([])
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(got).toEqual(['get_ticket', 'read_attachment'])
  // Once, whatever comes after.
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(got).toEqual(['get_ticket', 'read_attachment'])
})

test('two events at once read for themselves: one read failing leaves the other\'s standing', async ($, on) => {
  let fails = 4
  on('env.get', ($: any, e: any) => {
    if (fails > 0) {
      fails -= 1
      throw new Error('the dispatch was abandoned')
    }
    return { value: (ENV as Record<string, string>)[e.name] }
  })
  const runs = recordRuns(on)
  on('classic.SessionStart', () => ({}) as any)
  on('classic.UserPromptSubmit', () => ({}) as any)
  // Both in flight together: the first event's four reads fail, the
  // second's do not, and the second relays.
  await Promise.all([
    $.classic.SessionStart({ source: 'startup' } as any),
    $.classic.UserPromptSubmit({ prompt: 'go' } as any),
  ])
  await settle()
  const events = runs.map(r => r.argv[r.argv.indexOf('--event') + 1])
  expect(events).toContain('UserPromptSubmit')
})

test('a permission hold the daemon released with no decision leaves the dialog the person\'s', async ($, on) => {
  mock.env(on, ENV)
  // `mesimon approve` waits on the daemon; a person answering the dialog is
  // the PostToolUse edge, on which the daemon closes the wait unanswered
  // (T-581): empty stdout, and the hook returns what answered beneath it.
  let release: () => void = () => undefined
  on('process.run', ($: any, e: any) => {
    if (e.argv[1] !== 'approve') return { value: { exitCode: 0, stdout: '', stderr: '' } }
    return new Promise(resolve => {
      release = () => resolve({ value: { exitCode: 0, stdout: '', stderr: '' } })
    }) as any
  })
  on('classic.PermissionRequest', () => ({ decision: { behavior: 'deny', message: 'the person said no' } }) as any)
  const body = { hook_event_name: 'PermissionRequest', tool_name: 'Bash', tool_input: { command: 'touch x' } }
  let done: any
  const held = $.classic.PermissionRequest(body as any).then(r => (done = r))
  await settle()
  expect(done).toBe(undefined)
  release()
  await held
  expect(done.decision).toEqual({ behavior: 'deny', message: 'the person said no' })
})

test('a turn\'s end goes up as ModUsage: its count, and the main loop\'s rate-limit windows', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  let reads = 0
  on('session.usage', () => {
    reads += 1
    return {
      value: {
        startedAt: 0,
        context: { window: 200000 },
        rateLimits: [{ kind: 'five_hour', percentUsed: 9, resetsAt: '2026-10-03T12:00:00Z' }],
        cost: { usd: 0.42 },
      },
    } as any
  })
  on('turn.complete', () => ({ text: 'done' }) as any)
  const usage = { input_tokens: 3, output_tokens: 40, cache_read_input_tokens: 900, cache_creation_input_tokens: 120, model: 'claude-sonnet-5-5' }
  const r: any = await $.turn.complete({ answer: 'done', durationMs: 1200, isAborted: false, turnId: 't1', reason: 'answer', usage } as any)
  expect(r.text).toBe('done')
  await $.turn.complete({ answer: '', durationMs: 300, isAborted: false, turnId: 't2', agentId: 'a7', reason: 'answer', usage } as any)
  await settle()
  const frames = runs.filter(r => r.argv.includes('ModUsage'))
  expect(frames.map(f => f.argv.slice(7, 10))).toEqual([
    ['ModUsage', '--reason', 'answer'],
    ['ModUsage', '--reason', 'answer'],
  ])
  const main = JSON.parse(frames[0].stdin)
  expect(main).toEqual({
    turnId: 't1',
    reason: 'answer',
    durationMs: 1200,
    usage,
    rateLimits: [{ kind: 'five_hour', percentUsed: 9, resetsAt: '2026-10-03T12:00:00Z' }],
  })
  expect(main.answer).toBe(undefined)
  // A subagent's turn: its count and its agent, never the account's windows.
  const sub = JSON.parse(frames[1].stdin)
  expect(sub.agentId).toBe('a7')
  expect(sub.rateLimits).toBe(undefined)
  expect(reads).toBe(1)
})

test('a turn whose windows could not be read still sends its count', async ($, on) => {
  mock.env(on, ENV)
  const runs = recordRuns(on)
  on('session.usage', () => {
    throw new Error('the dispatch was abandoned')
  })
  on('turn.complete', () => ({ text: '' }) as any)
  await $.turn.complete({ answer: '', durationMs: 5, isAborted: true, turnId: 't3', reason: 'aborted' } as any)
  await settle()
  const frame = runs.find(r => r.argv.includes('ModUsage'))!
  expect(frame.argv.slice(7, 10)).toEqual(['ModUsage', '--reason', 'aborted'])
  expect(JSON.parse(frame.stdin)).toEqual({ turnId: 't3', reason: 'aborted', durationMs: 5 })
})

// ---- The native road (T-657): under `MESIMON_MOD_NATIVE=1` the classic
// relays are silent and the same frames, by the hook set's names and
// shapes, come from the engine's own events. What the live Claude Code
// fires is measured in STALE-MAP (T-651, T-657); these hold the shapes.

const NATIVE_ENV = { ...ENV, MESIMON_MOD_NATIVE: '1' }
const eventsOf = (runs: Run[]) => runs.map(r => r.argv[r.argv.indexOf('--event') + 1])
const bodyOf = (r: Run) => JSON.parse(r.stdin)
const only = (runs: Run[], event: string) => runs.filter(r => eventsOf([r])[0] === event)

/** A native session: the variables, a bridge that is refused for good, and `$.session.id()` from `id`. */
function nativeSession(on: any, id = { value: 'sid-1' }) {
  mock.env(on, NATIVE_ENV)
  mock.clock(on)
  on('process.spawn', async function* () {
    return { value: { code: 3, signal: null } }
  } as any)
  on('session.id', () => ({ value: id.value }) as any)
  on('session.start', () => ({ cwd: '/repo' }) as any)
  on('turn.start', ($: any, e: any) => ({ turnId: e.turnId }) as any)
  on('turn.complete', () => ({ text: '' }) as any)
  on('session.end', ($: any, e: any) => ({ sessionId: e.sessionId }) as any)
  return recordRuns(on)
}

const TURN = { answer: 'done', durationMs: 1200, isAborted: false, turnId: 't1', reason: 'answer' }
// A compaction's transcript: one message in, one left (the engine refuses an empty list either way).
const MESSAGES = [{ role: 'user', text: 'Reply ONE.', toolUses: [] }]

test('native: the classic events relay nothing, and the permission dialog is left to the hook set beside the mod', async ($, on) => {
  const runs = nativeSession(on)
  on('classic.Stop', () => ({}) as any)
  on('classic.SessionStart', () => ({}) as any)
  on('classic.UserPromptSubmit', () => ({}) as any)
  on('classic.PermissionRequest', () => ({}) as any)
  on('classic.SessionEnd', () => ({}) as any)
  await $.classic.Stop({ stop_hook_active: false } as any)
  await $.classic.SessionStart({ source: 'startup' } as any)
  await $.classic.UserPromptSubmit({ prompt: 'go' } as any)
  const r: any = await $.classic.PermissionRequest({ tool_name: 'Bash', tool_input: { command: 'touch x' } } as any)
  await $.classic.SessionEnd({ reason: 'other' } as any)
  await settle()
  expect(r.decision).toBe(undefined)
  expect(runs.filter(r => r.argv[1] === 'approve').length).toBe(0)
  expect(eventsOf(runs)).toEqual([])
})

test('native: off the switch (unset, or any word but 1) nothing native relays and the classic road is as it was', async ($, on) => {
  mock.env(on, { ...ENV, MESIMON_MOD_NATIVE: '0' })
  const runs = recordRuns(on)
  on('session.id', () => ({ value: 'sid-1' }) as any)
  on('session.end', () => ({ sessionId: 'sid-1' }) as any)
  on('turn.start', () => ({ turnId: 't1' }) as any)
  on('turn.complete', () => ({ text: '' }) as any)
  on('agent.spawn', () => ({ model: 'm', agentId: 'a1' }) as any)
  on('session.receive', ($: any, e: any) => ({ text: e.text }) as any)
  on('session.compact', () => ({ messages: MESSAGES, tokensBefore: 10, tokensAfter: 2 }) as any)
  on('tool.call', () => ({ result: { type: 'text', file: { filePath: '/x', content: '', numLines: 0, startLine: 1, totalLines: 0 } } }) as any)
  on('classic.Stop', () => ({}) as any)
  await $.session.end({ reason: 'clear', sessionId: 'sid-1', resume: { id: 'sid-1' } } as any)
  await $.turn.start({ text: 'go', turnId: 't1' } as any)
  await $.turn.complete(TURN as any)
  await $.agent.spawn({ tool_use_id: 'ta', prompt: 'p', description: 'd', subagentType: 'Explore', provider: { plugin: 'engine', tier: 'core' } } as any)
  await $.session.receive({ origin: { kind: 'peer', teammate: 'scout' }, text: '{"type":"idle_notification"}' } as any)
  await $.session.compact({ trigger: 'manual', messages: MESSAGES } as any)
  await $.tool.call({ tool: 'Read', tool_use_id: 'tr', file_path: '/x' } as any)
  await $.classic.Stop({ stop_hook_active: false } as any)
  await settle()
  expect(eventsOf(runs)).toEqual(['ModUsage', 'Stop'])
})

test('native: session.start is the SessionStart{startup}, with the id and the cwd and no derived path', async ($, on) => {
  const runs = nativeSession(on)
  await $.session.start({ cwd: '/repo', surface: 'terminal', isInteractive: true } as any)
  await settle()
  const start = only(runs, 'SessionStart')
  expect(start.length).toBe(1)
  expect(start[0].argv.slice(7, 12)).toEqual(['SessionStart', '--reason', 'startup', '--road', 'mod'])
  expect(bodyOf(start[0])).toEqual({ hook_event_name: 'SessionStart', source: 'startup', session_id: 'sid-1', cwd: '/repo' })
})

test('native: a /clear is the SessionEnd with its reason, then at the next turn with a new id a SessionStart{clear} before the UserPromptSubmit', async ($, on) => {
  const id = { value: 'sid-1' }
  const runs = nativeSession(on, id)
  await $.session.start({ cwd: '/repo', surface: 'terminal', isInteractive: true } as any)
  await $.session.end({ reason: 'clear', sessionId: 'sid-1', resume: { id: 'sid-1' } } as any)
  id.value = 'sid-2'
  await $.turn.start({ text: 'Reply TWO.', turnId: 't2' } as any)
  await $.turn.start({ text: 'Reply THREE.', turnId: 't3' } as any)
  await settle()
  expect(eventsOf(runs)).toEqual(['SessionStart', 'SessionEnd', 'SessionStart', 'UserPromptSubmit', 'UserPromptSubmit'])
  const end = only(runs, 'SessionEnd')[0]
  expect(end.argv.slice(7, 10)).toEqual(['SessionEnd', '--reason', 'clear'])
  expect(bodyOf(end)).toEqual({ hook_event_name: 'SessionEnd', reason: 'clear', session_id: 'sid-1' })
  const clear = only(runs, 'SessionStart')[1]
  expect(clear.argv.slice(7, 10)).toEqual(['SessionStart', '--reason', 'clear'])
  expect(bodyOf(clear)).toEqual({ hook_event_name: 'SessionStart', source: 'clear', session_id: 'sid-2', cwd: '/repo' })
  const prompts = only(runs, 'UserPromptSubmit').map(bodyOf)
  expect(prompts).toEqual([
    { hook_event_name: 'UserPromptSubmit', prompt: 'Reply TWO.', session_id: 'sid-2' },
    { hook_event_name: 'UserPromptSubmit', prompt: 'Reply THREE.', session_id: 'sid-2' },
  ])
  // An in-session /resume is the same edge, with the end's own word.
  await $.session.end({ reason: 'resume', sessionId: 'sid-2', resume: { id: 'sid-2' } } as any)
  id.value = 'sid-3'
  await $.turn.start({ text: 'go', turnId: 't4' } as any)
  await settle()
  expect(only(runs, 'SessionStart')[2].argv.slice(7, 10)).toEqual(['SessionStart', '--reason', 'resume'])
})

test('native: a turn\'s end is the Stop before the ModUsage; a subagent\'s is its SubagentStop by its spawn\'s type; an API error is a StopFailure with no class; an interrupt is nothing', async ($, on) => {
  const runs = nativeSession(on)
  on('agent.list', () => ({ value: [] }) as any)
  on('agent.spawn', () => ({ model: 'm', agentId: 'a7' }) as any)
  await $.agent.spawn({ tool_use_id: 'ta', prompt: 'p', description: 'Quick test agent', subagentType: 'Explore', provider: { plugin: 'engine', tier: 'core' } } as any)
  await $.turn.complete(TURN as any)
  await $.turn.complete({ ...TURN, turnId: 't2', agentId: 'a7' } as any)
  await $.turn.complete({ ...TURN, turnId: 't3', answer: '', reason: 'error' } as any)
  await $.turn.complete({ ...TURN, turnId: 't4', answer: '', isAborted: true, reason: 'aborted' } as any)
  await settle()
  expect(eventsOf(runs)).toEqual([
    'SubagentStart', 'Stop', 'ModUsage', 'SubagentStop', 'ModUsage', 'StopFailure', 'ModUsage', 'ModUsage',
  ])
  expect(bodyOf(only(runs, 'SubagentStart')[0])).toEqual({ hook_event_name: 'SubagentStart', agent_id: 'a7', agent_type: 'Explore' })
  expect(bodyOf(only(runs, 'Stop')[0])).toEqual({ hook_event_name: 'Stop', stop_hook_active: false, background_tasks: [] })
  expect(bodyOf(only(runs, 'SubagentStop')[0])).toEqual({ hook_event_name: 'SubagentStop', agent_id: 'a7', agent_type: 'Explore' })
  const failure = only(runs, 'StopFailure')[0]
  expect(failure.argv.slice(7, 10)).toEqual(['StopFailure', '--reason', 'unknown'])
  expect(bodyOf(failure)).toEqual({ hook_event_name: 'StopFailure', error: 'unknown', native: true })
})

test('native: every tool call is a PreToolUse and its result a PostToolUse as it came; a refusal or a tool\'s error is the PreToolUse alone', async ($, on) => {
  const runs = nativeSession(on)
  const file = { filePath: '/x', content: 'hi', numLines: 1, startLine: 1, totalLines: 1 }
  on('tool.call', ($: any, e: any) => {
    if (e.tool === 'Read') return { result: { type: 'text', file } } as any
    if (e.tool === 'Write') return { deny: 'no' } as any
    return { isError: true, result: 'Error: exit 1', text: 'Error: exit 1' } as any
  })
  await $.tool.call({ tool: 'Read', tool_use_id: 't1', file_path: '/x', consent: 'The user pressed "1: Yes"' } as any)
  await $.tool.call({ tool: 'Write', tool_use_id: 't2', file_path: '/y', content: 'z' } as any)
  await $.tool.call({ tool: 'Bash', tool_use_id: 't3', agentId: 'a1', command: 'false' } as any)
  await settle()
  expect(eventsOf(runs)).toEqual(['PreToolUse', 'PostToolUse', 'PreToolUse', 'PreToolUse'])
  const [pre, post, denied, errored] = runs.map(bodyOf)
  expect(pre).toEqual({ hook_event_name: 'PreToolUse', tool_name: 'Read', tool_use_id: 't1', tool_input: { file_path: '/x' } })
  expect(post).toEqual({
    hook_event_name: 'PostToolUse', tool_name: 'Read', tool_use_id: 't1', tool_input: { file_path: '/x' },
    tool_response: { type: 'text', file },
  })
  expect(denied.tool_name).toBe('Write')
  // A subagent's call names its agent, as the hook set's stdin did.
  expect(errored).toEqual({ hook_event_name: 'PreToolUse', tool_name: 'Bash', tool_use_id: 't3', tool_input: { command: 'false' }, agent_id: 'a1' })
})

test('native: a refused plan is the dialog\'s dismissal, said as ModAnswer declined; a refused question is said once', async ($, on) => {
  const runs = nativeSession(on)
  on('tool.call', () => ({ isError: true, result: 'Error: The user doesn\'t want to proceed with this tool use.', text: 'The user doesn\'t want to proceed.' }) as any)
  await $.tool.call({ tool: 'ExitPlanMode', tool_use_id: 'tp', plan: '# Plan\n\n1. a.txt', planFilePath: '/h/.claude/plans/p.md' } as any)
  await $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'tq', questions: QUESTIONS } as any)
  // A subagent's refused plan is not the session's dialog.
  await $.tool.call({ tool: 'ExitPlanMode', tool_use_id: 'ts', agentId: 'a1', plan: 'p' } as any)
  await settle()
  expect(eventsOf(runs)).toEqual(['PreToolUse', 'ModAnswer', 'PreToolUse', 'ModAnswer', 'PreToolUse'])
  const declined = only(runs, 'ModAnswer')
  expect(declined.map(reasonOf)).toEqual(['declined', 'declined'])
  expect(bodyOf(declined[0])).toEqual({ hook_event_name: 'PostToolUse', tool_name: 'ExitPlanMode', tool_use_id: 'tp', tool_input: { plan: '# Plan\n\n1. a.txt' } })
  expect(bodyOf(declined[1]).tool_name).toBe('AskUserQuestion')
})

test('native: a question the board answers is its ModAnswer alone, with no PostToolUse twin; one the person answers is a PostToolUse', async ($, on) => {
  mock.env(on, NATIVE_ENV)
  const runs = recordRuns(on)
  let person = false
  on('tool.call', () => (person ? { result: { questions: QUESTIONS, answers: { 'Which colour?': 'red' } }, text: 'answered', isReadOnly: true } : new Promise(() => undefined)) as any)
  bridgeSaying(on, [JSON.stringify({ id: '01Q', kind: 'answer', tool_use_id: 'toolu_9', answers: { 'Which colour?': 'blue' } })])
  on('session.start', () => ({ cwd: '/repo' }) as any)
  on('session.id', () => ({ value: 'sid-1' }) as any)
  const call = $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'toolu_9', questions: QUESTIONS } as any)
  await settle()
  await $.session.start({ cwd: '/repo', surface: 'terminal', isInteractive: true } as any)
  const r: any = await call
  expect(r.result.answers).toEqual({ 'Which colour?': 'blue' })
  await settle()
  expect(only(runs, 'ModAnswer').map(reasonOf)).toEqual(['answered'])
  expect(only(runs, 'PostToolUse').length).toBe(0)
  expect(only(runs, 'PreToolUse').length).toBe(1)
  person = true
  await $.tool.call({ tool: 'AskUserQuestion', tool_use_id: 'toolu_10', questions: QUESTIONS } as any)
  await settle()
  expect(only(runs, 'PostToolUse').map(r => bodyOf(r).tool_use_id)).toEqual(['toolu_10'])
})

test('native: the task ledger lists a backgrounded command on the Stop as a shell until its notification, a background agent as a subagent until the list says it is over, and nothing a TaskStop ended', async ($, on) => {
  const runs = nativeSession(on)
  let agents: any[] = []
  on('agent.list', () => ({ value: agents }) as any)
  on('agent.spawn', () => ({ model: 'm', agentId: 'a9' }) as any)
  on('tool.call', ($: any, e: any) => {
    if (e.tool === 'Bash') return { result: { stdout: '', stderr: '', interrupted: false, isImage: false, noOutputExpected: false, backgroundTaskId: 'bsh1' } } as any
    if (e.tool === 'Monitor') return { result: { taskId: 'mon1', timeoutMs: 0, persistent: true } } as any
    if (e.tool === 'Agent') return { result: { isAsync: true, status: 'async_launched', agentId: 'a9', description: 'Background agent test', prompt: 'p' } } as any
    return { result: { message: 'stopped' } } as any
  })
  const stops = () => only(runs, 'Stop').map(r => bodyOf(r).background_tasks)
  await $.tool.call({ tool: 'Bash', tool_use_id: 'tb', command: 'sleep 6; echo bg_done', run_in_background: true } as any)
  await $.turn.complete(TURN as any)
  await settle()
  expect(stops()).toEqual([[{ id: 'bsh1', type: 'shell', status: 'running', description: 'sleep 6; echo bg_done', command: 'sleep 6; echo bg_done' }]])
  const done = '<task-notification>\n<task-id>bsh1</task-id>\n<tool-use-id>tb</tool-use-id>\n<status>completed</status>\n</task-notification>'
  await $.turn.start({ text: done, turnId: 't2' } as any)
  await $.turn.complete({ ...TURN, turnId: 't2' } as any)
  await settle()
  expect(stops()[1]).toEqual([])
  // The agent: its spawn gives the type, the tool result starts the row.
  await $.agent.spawn({ tool_use_id: 'ta', prompt: 'p', description: 'Background agent test', subagentType: 'Explore', provider: { plugin: 'engine', tier: 'core' } } as any)
  await $.tool.call({ tool: 'Agent', tool_use_id: 'ta', description: 'Background agent test', prompt: 'p', subagent_type: 'Explore', run_in_background: true } as any)
  agents = [{ id: 'a9', description: 'Background agent test', type: 'Explore', status: 'running' }]
  await $.turn.complete({ ...TURN, turnId: 't3' } as any)
  await settle()
  expect(stops()[2]).toEqual([{ id: 'a9', type: 'subagent', status: 'running', description: 'Background agent test', agent_type: 'Explore' }])
  agents = [{ id: 'a9', description: 'Background agent test', type: 'Explore', status: 'completed' }]
  await $.turn.complete({ ...TURN, turnId: 't4' } as any)
  await settle()
  expect(stops()[3]).toEqual([])
  // A watch, stopped by hand.
  await $.tool.call({ tool: 'Monitor', tool_use_id: 'tm', command: 'tail -f x', description: 'watch x' } as any)
  await $.turn.complete({ ...TURN, turnId: 't5' } as any)
  await $.tool.call({ tool: 'TaskStop', tool_use_id: 'tx', task_id: 'mon1' } as any)
  await $.turn.complete({ ...TURN, turnId: 't6' } as any)
  await settle()
  expect(stops()[4]).toEqual([{ id: 'mon1', type: 'shell', status: 'running', description: 'watch x' }])
  expect(stops()[5]).toEqual([])
  // A subagent's own backgrounded command is not the lead's work (T-483).
  await $.tool.call({ tool: 'Bash', tool_use_id: 'tb2', agentId: 'a1', command: 'sleep 9', run_in_background: true } as any)
  await $.turn.complete({ ...TURN, turnId: 't7' } as any)
  await settle()
  expect(stops()[6]).toEqual([])
})

test('native: a notification folded into a running turn ends its tasks, every one it names; a quote of one ends nothing (T-691)', async () => {
  const note = (id: string) => `<task-notification>\n<task-id>${id}</task-id>\n<status>completed</status>\n</task-notification>`
  // The engine's row for a delivery into a running turn, as `session.append` hands it.
  const row = (kind: string, ...text: string[]) => ({
    message: { type: 'attachment', name: 'queued_command', role: 'user', isMeta: true, content: text.map(t => ({ type: 'text', text: t })) },
    door: 'delivery',
    origin: { kind },
    uuid: 'u1',
  })
  expect(rowNotified(row('engine', [note('bsh1'), note('bsh2')].join('\n'), note('mon1')))).toEqual(['bsh1', 'bsh2', 'mon1'])
  expect(rowNotified(row('task-notification', note('bsh3')))).toEqual(['bsh3'])
  expect(rowNotified(row('tool', note('bsh1')))).toEqual([])
  expect(rowNotified(row('model', note('bsh1')))).toEqual([])
  expect(rowNotified(row('composer', 'build it'))).toEqual([])
  expect(rowNotified({ message: { type: 'user', content: [{ type: 'image' }] }, origin: { kind: 'engine' } })).toEqual([])
})

test('native: a teammate is a SubagentStart by its name, a teammate row on the Stop with the list\'s status, a TeammateIdle with its team from its idle notice, and a SubagentStop at its turn\'s end', async ($, on) => {
  const runs = nativeSession(on)
  let status = 'running'
  on('agent.list', () => ({ value: [{ id: 'ascout-1', teammateId: 'scout@team-9', description: 'Test agent', type: 'general-purpose', status, name: 'scout' }] }) as any)
  on('agent.spawn', () => ({ model: 'm', agentId: 'ascout-1', teammateId: 'scout@team-9' }) as any)
  on('session.receive', ($: any, e: any) => ({ text: e.text }) as any)
  await $.agent.spawn({ tool_use_id: 'ta', prompt: 'Reply hello.', description: 'Test agent', subagentType: 'general-purpose', provider: { plugin: 'engine', tier: 'core' } } as any)
  await $.turn.complete(TURN as any)
  await $.turn.complete({ ...TURN, turnId: 't2', agentId: 'ascout-1', answer: 'hello' } as any)
  await $.session.receive({ origin: { kind: 'peer', teammate: 'scout', isVerified: true }, text: '{"type":"idle_notification","from":"scout","idleReason":"available","result":"hello"}' } as any)
  // A teammate's message that is no idle notice, and a delivery from elsewhere: nothing.
  await $.session.receive({ origin: { kind: 'peer', teammate: 'scout', isVerified: true }, text: 'plain words' } as any)
  await $.session.receive({ origin: { kind: 'task-notification' }, text: '{"type":"idle_notification"}' } as any)
  status = 'idle'
  await $.turn.complete({ ...TURN, turnId: 't3' } as any)
  await settle()
  expect(eventsOf(runs)).toEqual(['SubagentStart', 'Stop', 'ModUsage', 'SubagentStop', 'ModUsage', 'TeammateIdle', 'Stop', 'ModUsage'])
  expect(bodyOf(only(runs, 'SubagentStart')[0])).toEqual({ hook_event_name: 'SubagentStart', agent_id: 'ascout-1', agent_type: 'scout' })
  expect(bodyOf(only(runs, 'Stop')[0]).background_tasks).toEqual([{ id: 'ascout-1', type: 'teammate', status: 'running', description: 'Test agent' }])
  expect(bodyOf(only(runs, 'SubagentStop')[0])).toEqual({ hook_event_name: 'SubagentStop', agent_id: 'ascout-1', agent_type: 'scout' })
  expect(bodyOf(only(runs, 'TeammateIdle')[0])).toEqual({ hook_event_name: 'TeammateIdle', teammate_name: 'scout', team_name: 'team-9' })
  expect(bodyOf(only(runs, 'Stop')[1]).background_tasks).toEqual([{ id: 'ascout-1', type: 'teammate', status: 'idle', description: 'Test agent' }])
})

test('native: a compaction is PreCompact, SessionStart{compact} and PostCompact in the hook set\'s order; a vetoed one is the PreCompact alone; a precompute is nothing', async ($, on) => {
  const runs = nativeSession(on)
  let veto = false
  on('session.compact', () => (veto ? { skip: 'nothing to compact' } : { messages: MESSAGES, tokensBefore: 35536, tokensAfter: 3780 }) as any)
  await $.session.compact({ trigger: 'manual', messages: MESSAGES } as any)
  await settle()
  expect(eventsOf(runs)).toEqual(['PreCompact', 'SessionStart', 'PostCompact'])
  expect(bodyOf(runs[0])).toEqual({ hook_event_name: 'PreCompact', trigger: 'manual' })
  expect(runs[1].argv.slice(7, 10)).toEqual(['SessionStart', '--reason', 'compact'])
  expect(bodyOf(runs[1])).toEqual({ hook_event_name: 'SessionStart', source: 'compact', session_id: 'sid-1' })
  expect(bodyOf(runs[2])).toEqual({ hook_event_name: 'PostCompact', trigger: 'manual' })
  veto = true
  await $.session.compact({ trigger: 'auto', messages: MESSAGES } as any)
  await settle()
  expect(eventsOf(runs).slice(3)).toEqual(['PreCompact'])
  veto = false
  await $.session.compact({ trigger: 'precompute', messages: MESSAGES } as any)
  await settle()
  expect(eventsOf(runs).length).toBe(4)
})

test('native: tool.check is not hooked: the verdict beneath passes through and nothing is relayed', async ($, on) => {
  const runs = nativeSession(on)
  on('tool.check', () => ({ decision: 'ask', reason: 'touch needs approval' }) as any)
  const r: any = await $.tool.check({ tool: 'Bash', input: { command: 'touch one.txt' }, tool_use_id: 't1' } as any)
  await settle()
  expect(r.decision).toBe('ask')
  expect(eventsOf(runs)).toEqual([])
})
