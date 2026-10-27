// SCADA-16 W4/W6: the audit log filtered and rendered Unix-second timestamps as milliseconds,
// and the script run history showed second timestamps as milliseconds.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { auditRangeParams, fmtUnixSecsClock } from '../src/unixTime.ts'

test('audit filters are sent in Unix seconds', () => {
  assert.deepEqual(auditRangeParams('2026-11-01T00:00:00Z', '2026-11-02T00:00:00Z'), {
    from: 1793491200,
    to: 1793577600,
  })
})

test('empty or invalid audit filters are omitted', () => {
  assert.deepEqual(auditRangeParams('', ''), {})
  assert.deepEqual(auditRangeParams('2026-11-01T00:00:00Z', ''), { from: 1793491200 })
  assert.deepEqual(auditRangeParams('nope', '2026-11-02T00:00:00Z'), { to: 1793577600 })
})

test('script run time of day comes from seconds, not milliseconds', () => {
  const secs = Date.UTC(2026, 5, 15, 12, 34, 56) / 1000
  const shown = fmtUnixSecsClock(secs)
  assert.match(shown, /^\d{2}:\d{2}:\d{2}$/)
  assert.equal(shown, new Date(secs * 1000).toLocaleTimeString('en-US', {
    hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit',
  }))
  // Read as milliseconds, the same value would land in January 1970 at a different clock time.
  assert.notEqual(shown, new Date(secs).toLocaleTimeString('en-US', {
    hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit',
  }))
})
