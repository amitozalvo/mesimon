// `claude plugin test`: the mod's hooks against the engine's own `$`, with
// the test's hooks beneath standing in for the host (`process.run`,
// `process.spawn`). What the live Claude Code does is measured in STALE-MAP
// (T-573, T-574); these hold the hooks' shapes.
import { expect, mock, test } from 'claude-code/testing'

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
