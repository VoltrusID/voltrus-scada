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

export type SessionEndReason = 'session_idle' | 'session_expired'

/** The reason in a refused refresh's body, or null when it is an ordinary 401. */
export function sessionEndReason(body: unknown): SessionEndReason | null {
  const code = body && typeof body === 'object' ? (body as Record<string, unknown>).code : null
  return code === 'session_idle' || code === 'session_expired' ? code : null
}

/** Text for the login screen after the server ended the session. */
export function sessionEndMessage(reason: SessionEndReason, idleTimeoutSecs: number | null): string {
  if (reason === 'session_expired') return 'Your session reached its maximum length. Please sign in again.'
  if (idleTimeoutSecs) {
    const mins = Math.max(1, Math.round(idleTimeoutSecs / 60))
    return `You were signed out after ${mins} minute${mins === 1 ? '' : 's'} of inactivity.`
  }
  return 'You were signed out after a period of inactivity.'
}
