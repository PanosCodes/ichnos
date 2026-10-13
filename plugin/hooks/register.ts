import type { Register } from 'claude-code'

import { ask, event } from './account'
import { profiles } from './profiles'

/**
 * The `Stop` event that ends a turn the user interrupted, with what the model had answered by
 * then. Claude Code runs no Stop hook for such a turn, so without it the viewer shows the turn
 * running until the next prompt.
 */
export function interrupted(sessionId: string, answer: string): string {
  const extra = { interrupted: true, last_assistant_message: answer }
  return `${JSON.stringify({ event: 'Stop', session_id: sessionId, extra })}\n`
}

export const register: Register = (on, options) => {
  const profile = typeof options.debug_profile === 'string' ? profiles[options.debug_profile] : undefined

  on('turn.complete', async ($, e, next) => {
    const result = await next(e)

    if (e.agentId !== undefined) {
      return result
    }

    // Not awaited: the event is built and logged after the turn, without holding it up.
    // What fails before `ichnos log` can store it goes to the plugin's lines in the debug log.
    const send = (kind: string, line: () => Promise<string>) => {
      void (async () => {
        const run = await $.process.run(['ichnos', 'log'], { stdin: await line() })
        if (run.exitCode !== 0) {
          throw new Error(`ichnos log exited ${run.exitCode}: ${run.stderr.trim()}`)
        }
      })().catch((error: unknown) => {
        $.ui.log(`no ${kind} event for turn ${e.turnId}: ${error instanceof Error ? error.message : String(error)}`, {
          to: 'debug',
        })
      })
    }

    if (e.reason === 'aborted') {
      send('Stop', async () => interrupted(await $.session.id(), e.answer))
    } else if (e.reason === 'answer' && profile !== undefined) {
      send('Debug', async () =>
        event(await $.session.id(), e.turnId, await ask(prompt => $.model.fork({ prompt }), profile)),
      )
    }

    return result
  })
}
