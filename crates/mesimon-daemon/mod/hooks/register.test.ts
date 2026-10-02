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
    '--speaks', 'ping,submit,answer',
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
