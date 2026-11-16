import { afterEach, describe, expect, it, vi } from 'vitest'
import { fetchJson, fetchOk, onPasswordChangeRequired } from '../../src/queries/fetch'
import { isPasswordChangeRequiredError } from '../../src/session'

function answer(status: number, body: unknown) {
  return vi.fn(async () => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }))
}

describe('a request refused until the password is changed', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('tells the app and throws an error nothing reports', async () => {
    vi.stubGlobal('fetch', answer(403, { code: 'password_change_required', error: 'Change your password before you continue' }))
    const heard = vi.fn()
    const stop = onPasswordChangeRequired(heard)
    const err = await fetchJson('/api/v1/screens').catch(e => e)
    stop()
    expect(isPasswordChangeRequiredError(err)).toBe(true)
    expect(err.message).toBe('Change your password before you continue')
    expect(heard).toHaveBeenCalledTimes(1)
  })

  it('leaves other 403s to the caller', async () => {
    vi.stubGlobal('fetch', answer(403, { error: 'Current password is incorrect' }))
    const heard = vi.fn()
    const stop = onPasswordChangeRequired(heard)
    const err = await fetchOk('/api/v1/auth/change-password', { method: 'POST', body: '{}' }).catch(e => e)
    stop()
    expect(isPasswordChangeRequiredError(err)).toBe(false)
    expect(err.message).toBe('Current password is incorrect')
    expect(heard).not.toHaveBeenCalled()
  })
})
