import { afterEach, describe, expect, it, vi } from 'vitest'
import { fetchJson, onSessionEnded } from '../../src/queries/fetch'
import { isSessionExpiredError } from '../../src/session'
import { renderHook, act } from '@testing-library/react'
import { toast, useToasts, TOAST_DEDUPE_MS } from '../../src/components/Toast'

function respond(map: (url: string) => [number, unknown]) {
  return vi.fn(async (url: string) => {
    const [status, body] = map(url)
    return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
  })
}

describe('a session the server no longer accepts', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('ends on a plain 401 from refresh, without a reason code', async () => {
    vi.stubGlobal('fetch', respond(() => [401, { error: 'Unauthorized' }]))
    const heard = vi.fn()
    const stop = onSessionEnded(heard)
    const err = await fetchJson('/api/v1/screens').catch(e => e)
    stop()
    expect(isSessionExpiredError(err)).toBe(true)
    expect(heard).toHaveBeenCalledWith('session_ended')
  })

  it('keeps the server reason when there is one', async () => {
    vi.stubGlobal('fetch', respond(url => url.endsWith('/refresh') ? [401, { code: 'session_idle' }] : [401, {}]))
    const heard = vi.fn()
    const stop = onSessionEnded(heard)
    await fetchJson('/api/v1/screens').catch(() => {})
    stop()
    expect(heard).toHaveBeenCalledWith('session_idle')
  })

  it('does not end on a refresh the server could not answer, and reports that as an error', async () => {
    vi.stubGlobal('fetch', respond(url => url.endsWith('/refresh') ? [503, {}] : [401, {}]))
    const heard = vi.fn()
    const stop = onSessionEnded(heard)
    const err = await fetchJson('/api/v1/screens').catch(e => e)
    stop()
    expect(heard).not.toHaveBeenCalled()
    expect(isSessionExpiredError(err)).toBe(false)
    expect(err.message).toBe('Could not reach the server to renew the session')
  })

  it('a refresh that never reached the server is an ordinary error too', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (url.endsWith('/refresh')) throw new TypeError('Failed to fetch')
      return new Response('{}', { status: 401 })
    }))
    const err = await fetchJson('/api/v1/screens').catch(e => e)
    expect(isSessionExpiredError(err)).toBe(false)
    expect(err.message).toBe('Could not reach the server to renew the session')
  })
})

describe('toasts', () => {
  it('show the same message once per window', () => {
    vi.useFakeTimers()
    const { result } = renderHook(() => useToasts())
    act(() => { toast.error('Request failed'); toast.error('Request failed'); toast.error('Other') })
    expect(result.current.map(t => t.text)).toEqual(['Request failed', 'Other'])
    act(() => { vi.advanceTimersByTime(TOAST_DEDUPE_MS) })
    act(() => { toast.error('Request failed') })
    expect(result.current.map(t => t.text)).toEqual(['Request failed'])
    vi.useRealTimers()
  })
})
