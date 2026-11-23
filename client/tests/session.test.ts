import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  activityIntervalMs, createActivityThrottle, isPasswordChangeRequired, isPasswordChangeRequiredError,
  parseSessionPolicy, PasswordChangeRequiredError, sessionEndMessage, sessionEndReason,
} from '../src/session.ts'

test('the session policy is read from the login and /auth/me bodies', () => {
  assert.deepEqual(parseSessionPolicy({ session: { idle_timeout_secs: 900, absolute_timeout_secs: 43200 } }), {
    idleTimeoutSecs: 900, absoluteTimeoutSecs: 43200,
  })
  assert.deepEqual(parseSessionPolicy({ session: { idle_timeout_secs: null, absolute_timeout_secs: null } }), {
    idleTimeoutSecs: null, absoluteTimeoutSecs: null,
  })
  assert.deepEqual(parseSessionPolicy({ role: 'admin' }), { idleTimeoutSecs: null, absoluteTimeoutSecs: null })
  assert.deepEqual(parseSessionPolicy(null), { idleTimeoutSecs: null, absoluteTimeoutSecs: null })
  assert.deepEqual(parseSessionPolicy({ session: { idle_timeout_secs: 0 } }), { idleTimeoutSecs: null, absoluteTimeoutSecs: null })
})

test('activity is reported at most once per min(60 s, idle / 3)', () => {
  assert.equal(activityIntervalMs(900), 60_000)
  assert.equal(activityIntervalMs(120), 40_000)
  assert.equal(activityIntervalMs(1), 1_000)
})

test('the throttle lets one report through per interval', () => {
  const t = createActivityThrottle(60_000)
  assert.equal(t.shouldSend(1_000), true)
  assert.equal(t.shouldSend(30_000), false)
  assert.equal(t.shouldSend(60_999), false)
  assert.equal(t.shouldSend(61_000), true)
})

test('a refused refresh says why the session ended', () => {
  assert.equal(sessionEndReason({ code: 'session_idle' }), 'session_idle')
  assert.equal(sessionEndReason({ code: 'session_expired' }), 'session_expired')
  assert.equal(sessionEndReason({ error: 'Unauthorized' }), null)
  assert.equal(sessionEndReason(null), null)
})

test('the login screen explains the sign-out', () => {
  assert.equal(sessionEndMessage('session_idle', 900), 'You were signed out after 15 minutes of inactivity.')
  assert.equal(sessionEndMessage('session_idle', 60), 'You were signed out after 1 minute of inactivity.')
  assert.match(sessionEndMessage('session_idle', null), /inactivity/)
  assert.match(sessionEndMessage('session_expired', 900), /maximum length/)
})

test('a 403 password_change_required is told apart from other refusals', () => {
  assert.equal(isPasswordChangeRequired(403, { code: 'password_change_required', error: 'Change your password' }), true)
  assert.equal(isPasswordChangeRequired(403, { error: 'Current password is incorrect' }), false)
  assert.equal(isPasswordChangeRequired(403, { code: 'signature_invalid' }), false)
  assert.equal(isPasswordChangeRequired(401, { code: 'password_change_required' }), false)
  assert.equal(isPasswordChangeRequired(403, null), false)
})

test('the password-change error is recognised so nothing toasts it', () => {
  assert.equal(isPasswordChangeRequiredError(new PasswordChangeRequiredError()), true)
  const copy = new Error('x'); copy.name = 'PasswordChangeRequiredError'
  assert.equal(isPasswordChangeRequiredError(copy), true)
  assert.equal(isPasswordChangeRequiredError(new Error('Request failed: 403')), false)
})

test('any refused refresh ends the session, with or without a reason code', async () => {
  const { refreshRefused } = await import('../src/session.ts')
  assert.equal(refreshRefused(401), true)
  assert.equal(refreshRefused(500), false, 'a restarting server keeps the session')
  assert.equal(refreshRefused(502), false)
  assert.equal(refreshRefused(200), false)
  assert.equal(sessionEndMessage('session_ended', 900), 'Your session has ended. Please sign in again.')
})

test('login, /auth/me and SSO answers give the same session fields', async () => {
  const { sessionFromAuthBody } = await import('../src/session.ts')
  const me = sessionFromAuthBody({
    authenticated: true, username: 'op', role: 'operator', shared: false,
    must_change_password: false, password_expired: true,
    session: { idle_timeout_secs: 600, absolute_timeout_secs: null },
  })
  assert.deepEqual(me, {
    authenticated: true, username: 'op', role: 'operator', shared: false,
    mustChangePassword: false, passwordExpired: true,
    policy: { idleTimeoutSecs: 600, absoluteTimeoutSecs: null },
  })
  // A login response has no `authenticated`; the typed name is the fallback.
  const login = sessionFromAuthBody({ role: 'admin' }, 'alice')
  assert.equal(login.authenticated, true)
  assert.equal(login.username, 'alice')
  assert.equal(sessionFromAuthBody({ authenticated: false }).authenticated, false)
  assert.equal(sessionFromAuthBody(null).role, 'user')
})
