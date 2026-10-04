import type { ModelForkResult } from 'claude-code'
import { describe, expect, test } from 'claude-code/testing'

import { ask, check, event, extract } from '../hooks/account'
import { profiles } from '../hooks/profiles'

const skills = profiles.skills!
const quiet = '{"v":2,"s":{"c":[],"s":[],"g":[],"u":[]},"r":{"c":[]},"o":{"s":"done","r":[],"u":[],"k":[]}}'

/** A fork that answers with `replies`, one per call, recording each prompt. */
function forking(replies: string[]) {
  const prompts: string[] = []
  const fork = async (prompt: string) => {
    prompts.push(prompt)
    const text = replies.shift()
    return (
      text === undefined ? { isAnswered: false, reason: 'empty-reply' } : { isAnswered: true, text, usage: {} }
    ) as ModelForkResult
  }
  return { fork, prompts }
}

describe('check', () => {
  test('takes a fitting answer, also inside a fence', () => {
    expect(check(quiet, skills.schema)).toEqual({ value: JSON.parse(quiet) })
    expect(check('```json\n' + quiet + '\n```', skills.schema)).toEqual({ value: JSON.parse(quiet) })
  })

  test('names where an answer breaks the schema', () => {
    expect(check('no idea', skills.schema)).toEqual({ error: 'no JSON object' })
    expect(check(quiet.replace('"s":"done"', '"s":"maybe"'), skills.schema)).toEqual({
      error: '$.o.s: not one of "done", "partial", "blocked", "question"',
    })
    expect(check(quiet.replace('"r":{"c":[]},', ''), skills.schema)).toEqual({ error: '$: missing "r"' })
    expect(check(quiet.replace('"v":2', '"v":2,"x":1'), skills.schema)).toEqual({ error: '$: unknown key "x"' })
    expect(check(quiet.replace('"c":[]', '"c":[{"k":"fixup","l":"no","r":"why"}]'), skills.schema)).toEqual({
      error: '$.s.c[0].l: not a boolean',
    })
  })

  test('extract finds no object in plain text', () => {
    expect(extract('done')).toBeUndefined()
  })
})

describe('ask', () => {
  test('a fitting first answer is the account', async () => {
    const { fork, prompts } = forking([quiet])
    expect(await ask(fork, skills)).toEqual(JSON.parse(quiet))
    expect(prompts).toEqual([skills.prompt])
  })

  test('a broken answer is asked again once, naming the problem', async () => {
    const { fork, prompts } = forking(['{"v":2}', quiet])
    expect(await ask(fork, skills)).toEqual(JSON.parse(quiet))
    expect(prompts[1]).toContain('$: missing "s"')
  })

  test('two broken answers are kept as invalid', async () => {
    const { fork } = forking(['nope', 'still nope'])
    expect(await ask(fork, skills)).toEqual({ invalid: true, raw: 'still nope', error: 'no JSON object' })
  })

  test('no answer is an invalid account naming why', async () => {
    const { fork } = forking([])
    expect(await ask(fork, skills)).toEqual({ invalid: true, raw: '', error: 'no reply: empty-reply' })
  })

  test('an API error names its status and kind', async () => {
    const fork = async () =>
      ({ isAnswered: false, reason: 'api-error', status: 400, error: 'invalid_request', usage: {} }) as ModelForkResult
    expect(await ask(fork, skills)).toEqual({
      invalid: true,
      raw: '',
      error: 'no reply: api-error (400, invalid_request)',
    })
  })
})

test('the event names the session, the turn and the account', () => {
  const line = event('s1', 't1', { v: 2 })
  expect(line.endsWith('\n')).toBe(true)
  expect(JSON.parse(line)).toEqual({ event: 'Debug', session_id: 's1', extra: { turn_id: 't1', debug: { v: 2 } } })
})
