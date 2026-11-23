import { completeSignature } from '../signature/logic'
import { canPromptForSignature, requestSignature } from '../signature/prompt'
import {
  isPasswordChangeRequired, PasswordChangeRequiredError, refreshRefused, SessionExpiredError,
  sessionEndReason, type SessionEndReason,
} from '../session'

// Global in-flight refresh promise to prevent multiple simultaneous refresh attempts
let refreshing: Promise<boolean> | null = null

type SessionEndedListener = (reason: SessionEndReason) => void
const sessionEndedListeners = new Set<SessionEndedListener>()

type PasswordChangeListener = () => void
const passwordChangeListeners = new Set<PasswordChangeListener>()

/** Called when the server refuses a request until the user changes their password. */
export function onPasswordChangeRequired(fn: PasswordChangeListener): () => void {
  passwordChangeListeners.add(fn)
  return () => { passwordChangeListeners.delete(fn) }
}

/**
 * A 403 `password_change_required` (an admin-forced change, or a password the
 * edition reports expired): tell the app, which shows the change-password
 * screen, and throw so callers do not report it as an error.
 */
export async function checkPasswordChange(r: Response): Promise<Response> {
  if (r.status !== 403) return r
  const body = await r.clone().json().catch(() => null)
  if (!isPasswordChangeRequired(r.status, body)) return r
  passwordChangeListeners.forEach(fn => fn())
  const message = typeof body?.error === 'string' && body.error ? body.error : undefined
  throw new PasswordChangeRequiredError(message)
}

/**
 * Called when the server refuses to refresh the session: it hit its idle or
 * absolute limit, or it is gone for another reason (`session_ended`).
 */
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
      if (refreshRefused(r.status)) {
        const reason = sessionEndReason(await r.json().catch(() => null)) ?? 'session_ended'
        sessionEndedListeners.forEach(fn => fn(reason))
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

export async function fetchWithRefresh(
  url: string,
  init?: RequestInit,
): Promise<Response> {
  const r = await fetch(url, { credentials: 'include', ...init })
  if (r.status === 401) {
    const ok = await doRefresh()
    if (ok) {
      return checkPasswordChange(await fetch(url, { credentials: 'include', ...init }))
    }
    // Don't hard-redirect — throw so React Query handles it gracefully.
    // The App component's auth gate will show the login screen.
    throw new SessionExpiredError()
  }
  return checkPasswordChange(r)
}

/**
 * fetchWithRefresh plus electronic signatures: when the server answers that
 * the action must be signed, the shared prompt asks the user and the request
 * is sent again with the signature in its JSON body. Cancelling throws
 * SignatureCancelledError.
 */
export async function fetchSigned(url: string, init?: RequestInit): Promise<Response> {
  const r = await fetchWithRefresh(url, init)
  if ((r.status !== 428 && r.status !== 403) || !canPromptForSignature()) return r
  return completeSignature(r, init, {
    send: signed => fetchWithRefresh(url, signed),
    prompt: requestSignature,
  })
}

export async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const r = await fetchWithRefresh(url, init)
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
