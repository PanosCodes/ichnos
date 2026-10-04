import type { Register } from 'claude-code'

import { ask, event } from './account'
import { profiles } from './profiles'

export const register: Register = (on, options) => {
  const profile = typeof options.debug_profile === 'string' ? profiles[options.debug_profile] : undefined

  if (profile === undefined) {
    return
  }

  on('turn.complete', async ($, e, next) => {
    const result = await next(e)

    if (e.reason !== 'answer' || e.agentId !== undefined) {
      return result
    }

    // Not awaited: the account is asked for and logged after the turn, without holding it up.
    // What fails before `ichnos log` can store it goes to the plugin's lines in the debug log.
    void (async () => {
      const account = await ask(prompt => $.model.fork({ prompt }), profile)
      const run = await $.process.run(['ichnos', 'log'], { stdin: event(await $.session.id(), e.turnId, account) })
      if (run.exitCode !== 0) {
        throw new Error(`ichnos log exited ${run.exitCode}: ${run.stderr.trim()}`)
      }
    })().catch((error: unknown) => {
      $.ui.log(`no Debug event for turn ${e.turnId}: ${error instanceof Error ? error.message : String(error)}`, {
        to: 'debug',
      })
    })

    return result
  })
}
