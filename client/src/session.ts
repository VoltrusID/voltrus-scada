// Session policy handling, React-free so it can be tested.

export interface SessionPolicy {
  idleTimeoutSecs: number | null
  absoluteTimeoutSecs: number | null
}

export const NO_SESSION_POLICY: SessionPolicy = { idleTimeoutSecs: null, absoluteTimeoutSecs: null }

const positive = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : null)

/** Reads `session` from the login or `/auth/me` body; older servers send none. */
export function parseSessionPolicy(body: unknown): SessionPolicy {
  const s = body && typeof body === 'object' ? (body as Record<string, unknown>).session : null
  if (!s || typeof s !== 'object') return NO_SESSION_POLICY
  const { idle_timeout_secs, absolute_timeout_secs } = s as Record<string, unknown>
  return { idleTimeoutSecs: positive(idle_timeout_secs), absoluteTimeoutSecs: positive(absolute_timeout_secs) }
}

/**
 * How often user input may be reported to `POST /auth/activity`: at most once
 * per min(60 s, idle timeout / 3), so the server hears of an active user well
 * before the idle limit while a busy user does not flood it.
 */
export function activityIntervalMs(idleTimeoutSecs: number): number {
  return Math.max(1, Math.min(60, idleTimeoutSecs / 3)) * 1000
}

/** Rate limiter for activity reports: `shouldSend(now)` is true at most once per interval. */
export function createActivityThrottle(intervalMs: number) {
  let last = -Infinity
  return {
    shouldSend(now: number): boolean {
      if (now - last < intervalMs) return false
      last = now
      return true
    },
  }
}

/**
 * Why the session ended: the server's idle or absolute limit, or `session_ended`
 * when the refresh was refused without a reason (revoked, server restarted with
 * new keys, an older server).
 */
export type SessionEndReason = 'session_idle' | 'session_expired' | 'session_ended'

/** The reason in a refused refresh's body, or null when it is an ordinary 401. */
export function sessionEndReason(body: unknown): SessionEndReason | null {
  const code = body && typeof body === 'object' ? (body as Record<string, unknown>).code : null
  return code === 'session_idle' || code === 'session_expired' ? code : null
}

/**
 * Whether the refresh endpoint's status means the session is gone: any 401,
 * with or without a reason code. Network errors and 5xx are not: the server may
 * be restarting and the session may still be good.
 */
export function refreshRefused(status: number): boolean {
  return status === 401
}

/** Thrown by requests made after the session ended; the login screen says why, so nothing else should. */
export class SessionExpiredError extends Error {
  constructor(message = 'Session expired') {
    super(message)
    this.name = 'SessionExpiredError'
  }
}

export function isSessionExpiredError(err: unknown): boolean {
  return err instanceof SessionExpiredError || (err instanceof Error && err.name === 'SessionExpiredError')
}

/** What the app keeps of a login, `/auth/me` or SSO answer. */
export interface AuthSession {
  authenticated: boolean
  username: string
  role: string
  shared: boolean
  mustChangePassword: boolean
  passwordExpired: boolean
  policy: SessionPolicy
}

/** Reads an auth answer; `authenticated` defaults to true for login responses, which have no such field. */
export function sessionFromAuthBody(body: unknown, fallbackUsername = ''): AuthSession {
  const d = (body && typeof body === 'object' ? body : {}) as Record<string, unknown>
  return {
    authenticated: d.authenticated === undefined ? true : d.authenticated === true,
    username: typeof d.username === 'string' && d.username ? d.username : fallbackUsername,
    role: typeof d.role === 'string' && d.role ? d.role : 'user',
    shared: d.shared === true,
    mustChangePassword: d.must_change_password === true,
    passwordExpired: d.password_expired === true,
    policy: parseSessionPolicy(d),
  }
}

/** Text for the login screen after the server ended the session. */
export function sessionEndMessage(reason: SessionEndReason, idleTimeoutSecs: number | null): string {
  if (reason === 'session_expired') return 'Your session reached its maximum length. Please sign in again.'
  if (reason === 'session_ended') return 'Your session has ended. Please sign in again.'
  if (idleTimeoutSecs) {
    const mins = Math.max(1, Math.round(idleTimeoutSecs / 60))
    return `You were signed out after ${mins} minute${mins === 1 ? '' : 's'} of inactivity.`
  }
  return 'You were signed out after a period of inactivity.'
}

/** The `code` of the 403 a session gets while its user must change their password. */
export const PASSWORD_CHANGE_REQUIRED = 'password_change_required'

/** Whether a response (status and parsed JSON body) says the password must be changed first. */
export function isPasswordChangeRequired(status: number, body: unknown): boolean {
  if (status !== 403 || !body || typeof body !== 'object') return false
  return (body as Record<string, unknown>).code === PASSWORD_CHANGE_REQUIRED
}

/**
 * Thrown for a request the server refused until the password is changed. The
 * app shows the change-password screen instead, so nothing should be reported.
 */
export class PasswordChangeRequiredError extends Error {
  constructor(message = 'Change your password before you continue') {
    super(message)
    this.name = 'PasswordChangeRequiredError'
  }
}

export function isPasswordChangeRequiredError(err: unknown): boolean {
  return err instanceof PasswordChangeRequiredError || (err instanceof Error && err.name === 'PasswordChangeRequiredError')
}
