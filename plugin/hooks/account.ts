import type { ModelForkResult } from 'claude-code'

import type { Schema } from './validate'
import { validate } from './validate'

/** Characters of an answer that didn't fit kept as its `raw` text. */
const MAX_RAW = 500

/** The account Ichnos stores: the model's compact JSON, or why there is none. */
export type Account = Record<string, unknown> | { invalid: true; raw: string; error: string }

/** The JSON object in `text`, also when the model wrapped it in a fence or a sentence. */
export function extract(text: string): unknown {
  const start = text.indexOf('{')
  const end = text.lastIndexOf('}')
  if (start === -1 || end < start) {
    return undefined
  }
  try {
    return JSON.parse(text.slice(start, end + 1))
  } catch {
    return undefined
  }
}

/** Where `text` fails as an answer: no JSON object, or one that breaks `schema`. */
export function check(text: string, schema: Schema): { value: Record<string, unknown> } | { error: string } {
  const value = extract(text)
  if (value === undefined) {
    return { error: 'no JSON object' }
  }
  const error = validate(value, schema)
  return error === undefined ? { value: value as Record<string, unknown> } : { error }
}

/**
 * Ask the session's model, through `fork` (a fork of the session that just finished a turn),
 * for its account of the turn. An answer that breaks the schema is asked again once, naming
 * what was wrong; a fork that doesn't answer is an invalid account naming why, so the turn
 * still shows that the account is missing.
 */
export async function ask(
  fork: (prompt: string) => Promise<ModelForkResult>,
  profile: { prompt: string; schema: Schema },
): Promise<Account> {
  let prompt = profile.prompt
  let last = { raw: '', error: '' }
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const reply = await fork(prompt)
    if (!reply.isAnswered) {
      return { invalid: true, raw: last.raw, error: `no reply: ${why(reply)}` }
    }
    const checked = check(reply.text, profile.schema)
    if ('value' in checked) {
      return checked.value
    }
    last = { raw: reply.text.slice(0, MAX_RAW), error: checked.error }
    prompt = `${profile.prompt}\n\nYour previous answer didn't fit (${checked.error}):\n${last.raw}\n\nAnswer again with the JSON object only.`
  }
  return { invalid: true, ...last }
}

/** Why `reply` carries no text: its reason, and for an API error its HTTP status and kind. */
function why(reply: Extract<ModelForkResult, { isAnswered: false }>): string {
  return reply.reason === 'api-error' ? `api-error (${reply.status ?? 'no response'}, ${reply.error})` : reply.reason
}

/** The `Debug` event that hands `account` of turn `turnId` to `ichnos log`. */
export function event(sessionId: string, turnId: string, account: Account): string {
  return `${JSON.stringify({ event: 'Debug', session_id: sessionId, extra: { turn_id: turnId, debug: account } })}\n`
}
