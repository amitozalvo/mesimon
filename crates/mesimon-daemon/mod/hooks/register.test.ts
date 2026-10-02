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
