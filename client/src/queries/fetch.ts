import { completeSignature } from '../signature/logic'
import { canPromptForSignature, requestSignature } from '../signature/prompt'
import { sessionEndReason, type SessionEndReason } from '../session'

// Global in-flight refresh promise to prevent multiple simultaneous refresh attempts
let refreshing: Promise<boolean> | null = null

type SessionEndedListener = (reason: SessionEndReason) => void
const sessionEndedListeners = new Set<SessionEndedListener>()

/** Called when a refresh is refused because the session hit its idle or absolute limit. */
export function onSessionEnded(fn: SessionEndedListener): () => void {
  sessionEndedListeners.add(fn)
  return () => { sessionEndedListeners.delete(fn) }
}

async function doRefresh(): Promise<boolean> {
  if (refreshing) {
    return refreshing
  }
  refreshing = (async () => {
    try {
      const r = await fetch('/api/v1/auth/refresh', {
        method: 'POST',
        credentials: 'include',
      })
      if (r.status === 401) {
        const reason = sessionEndReason(await r.json().catch(() => null))
        if (reason) sessionEndedListeners.forEach(fn => fn(reason))
      }
      return r.ok
    } catch {
      return false
    } finally {
      refreshing = null
    }
  })()
  return refreshing
}

async function fetchWithRefresh(
  url: string,
  init?: RequestInit,
): Promise<Response> {
  const r = await fetch(url, { credentials: 'include', ...init })
  if (r.status === 401) {
    const ok = await doRefresh()
    if (ok) {
      return fetch(url, { credentials: 'include', ...init })
    }
    // Don't hard-redirect — throw so React Query handles it gracefully.
    // The App component's auth gate will show the login screen.
    throw new Error('Session expired')
  }
  return r
}

/**
 * fetchWithRefresh plus electronic signatures: when the server answers that
 * the action must be signed, the shared prompt asks the user and the request
 * is sent again with the signature in its JSON body. Cancelling throws
 * SignatureCancelledError.
 */
async function fetchSigned(url: string, init?: RequestInit): Promise<Response> {
  const r = await fetchWithRefresh(url, init)
  if ((r.status !== 428 && r.status !== 403) || !canPromptForSignature()) return r
  return completeSignature(r, init, {
    send: signed => fetchWithRefresh(url, signed),
    prompt: requestSignature,
  })
}

export async function fetchJson<T>(url: string): Promise<T> {
  const r = await fetchWithRefresh(url)
  if (!r.ok) throw new Error(`fetch ${url} failed: ${r.status}`)
  return r.json()
}

export async function fetchOk(url: string, init?: RequestInit): Promise<Response> {
  const r = await fetchSigned(url, init)
  if (!r.ok) {
    const body = await r.json().catch(() => ({}))
    throw new Error(body.error || `Request failed: ${r.status}`)
  }
  return r
}
