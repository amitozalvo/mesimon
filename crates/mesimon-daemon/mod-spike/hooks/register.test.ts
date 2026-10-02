// `claude plugin test`: the spike's hooks against the engine's own `$`, with
// the test's hooks standing beneath for the engine. What a row measured on
// the live Claude Code is in STALE-MAP; these hold the hooks' shapes.
import { expect, mock, test } from 'claude-code/testing'

const BOARD = '/repo/.mesimon'
const QUESTIONS = [{ question: 'Which color?', header: 'Color', options: [{ label: 'Red', description: 'r' }, { label: 'Blue', description: 'b' }], multiSelect: false }]

test('row 5: a Write under the board dir is denied before anything beneath runs', async ($, on) => {
  mock.env(on, { MESIMON_MOD_GATE_BOARD: BOARD })
  let ran = 0
  on('tool.call', { tool: 'Write' }, () => {
    ran += 1
    return { result: { type: 'create', filePath: 'x', content: 'hi', structuredPatch: [] } } as any
  })
  const denied = await $.tool.call({ tool: 'Write', file_path: `${BOARD}/columns.toml`, content: 'hi' })
  expect(denied.deny).toMatch(/board state/)
  expect(ran).toBe(0)
  const ok = await $.tool.call({ tool: 'Write', file_path: '/repo/src/main.rs', content: 'hi' })
  expect(ok.deny).toBeUndefined()
  expect(ran).toBe(1)
})

test('row 5: the board dir itself and a sibling that only shares the prefix', async ($, on) => {
  mock.env(on, { MESIMON_MOD_GATE_BOARD: BOARD })
  on('tool.call', { tool: 'Edit' }, () => ({ result: { filePath: 'x', oldString: 'a', newString: 'b', originalFile: 'a', structuredPatch: [], userModified: false, replaceAll: false } }) as any)
  expect((await $.tool.call({ tool: 'Edit', file_path: BOARD, old_string: 'a', new_string: 'b' })).deny).toMatch(/board state/)
  expect((await $.tool.call({ tool: 'Edit', file_path: '/repo/.mesimon-notes/a.md', old_string: 'a', new_string: 'b' })).deny).toBeUndefined()
})

test('row 3: the person answering first wins and the hold is released', async ($, on) => {
  mock.env(on, {})
  on('tool.call', { tool: 'AskUserQuestion' }, () => ({ result: { questions: QUESTIONS, answers: { 'Which color?': 'Red' } } }) as any)
  const r: any = await $.tool.call({ tool: 'AskUserQuestion', questions: QUESTIONS })
  expect(r.deny).toBeUndefined()
  expect(r.result.answers['Which color?']).toBe('Red')
})

test('row 4: tool.check passes the engine verdict through untouched', async ($, on) => {
  mock.env(on, {})
  on('tool.check', () => ({ decision: 'ask', reason: 'the mode asks' }))
  const v = await $.tool.check({ tool: 'Bash', input: { command: 'ls' } })
  expect(v.decision).toBe('ask')
  expect(v.reason).toBe('the mode asks')
})

test('row 6: ExitPlanMode goes to the native dialog unless told otherwise', async ($, on) => {
  mock.env(on, {})
  on('tool.call', { tool: 'ExitPlanMode' }, () => ({ result: { plan: 'the plan', isAgent: false } }) as any)
  const r: any = await $.tool.call({ tool: 'ExitPlanMode' })
  expect(r.result.plan).toBe('the plan')
})
