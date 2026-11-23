import i18n from 'i18next'
import { completeSignature } from '../signature/logic'
import { canPromptForSignature, requestSignature } from '../signature/prompt'
import {
  isPasswordChangeRequired, PasswordChangeRequiredError, refreshRefused, SessionExpiredError,
  sessionEndReason, type SessionEndReason,
} from '../session'

/** `refused`: the server said the session is over. `failed`: no usable answer (offline, 5xx). */
type RefreshOutcome = 'ok' | 'refused' | 'failed'

// Global in-flight refresh promise to prevent multiple simultaneous refresh attempts
let refreshing: Promise<RefreshOutcome> | null = null

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

async function doRefresh(): Promise<RefreshOutcome> {
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
        return 'refused'
      }
      return r.ok ? 'ok' : 'failed'
    } catch {
      return 'failed'
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
    const outcome = await doRefresh()
    if (outcome === 'ok') {
      return checkPasswordChange(await fetch(url, { credentials: 'include', ...init }))
    }
    // Refused: the session is over and the login screen says so; nothing else should.
    if (outcome === 'refused') throw new SessionExpiredError()
    // The server could not be asked: the session may still be good, so report it like any failure.
    throw new Error(i18n.t('session.renewFailed', { defaultValue: 'Could not reach the server to renew the session' }))
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
