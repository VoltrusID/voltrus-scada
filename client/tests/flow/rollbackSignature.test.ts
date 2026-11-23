import { afterEach, describe, expect, it, vi } from 'vitest'
import { rollback } from '../../src/api/config-snapshots'
import { answerSignaturePrompt, currentSignaturePrompt, subscribeSignaturePrompt } from '../../src/signature/prompt'
import { isSignatureCancelled } from '../../src/signature/logic'

describe('config snapshot rollback', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  function serverWantingSignature() {
    const bodies: (string | null)[] = []
    vi.stubGlobal('fetch', vi.fn(async (_url: string, init?: RequestInit) => {
      const body = typeof init?.body === 'string' ? init.body : null
      bodies.push(body)
      if (body && JSON.parse(body).signature) return new Response('{"ok":true}', { status: 200 })
      return new Response(JSON.stringify({
        code: 'signature_required',
        signature_required: { action: 'mode_switch', meaning: 'Roll back configuration to snapshot 7' },
      }), { status: 428 })
    }))
    return bodies
  }

  it('asks for a signature when the server wants one and resends it in the JSON body', async () => {
    const bodies = serverWantingSignature()
    // A mounted dialog: answer the prompt as the user would.
    const stop = subscribeSignaturePrompt(() => {
      const p = currentSignaturePrompt()
      if (p) answerSignaturePrompt({ password: 'pw', meaning: p.request.meaning, reason: 'restore yesterday' })
    })
    await rollback(7)
    stop()
    expect(bodies).toHaveLength(2)
    expect(JSON.parse(bodies[1]!)).toEqual({
      signature: { password: 'pw', meaning: 'Roll back configuration to snapshot 7', reason: 'restore yesterday' },
    })
  })

  it('cancelling the prompt abandons the rollback quietly', async () => {
    const bodies = serverWantingSignature()
    const stop = subscribeSignaturePrompt(() => { if (currentSignaturePrompt()) answerSignaturePrompt(null) })
    const err = await rollback(7).catch(e => e)
    stop()
    expect(isSignatureCancelled(err)).toBe(true)
    expect(bodies).toHaveLength(1)
  })
})
