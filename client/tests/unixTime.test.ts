// SCADA-15 W5: the API-key panel sent expires_at in milliseconds (keys never expired on the
// seconds-based server) and formatted second timestamps as milliseconds.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { fmtUnixSecs, isPastUnixSecs, unixSecsFromInput } from '../src/unixTime.ts'

test('expiry input becomes Unix seconds', () => {
  assert.equal(unixSecsFromInput('2026-11-01T00:00:00Z'), 1793491200)
  assert.equal(unixSecsFromInput('not a date'), undefined)
})

test('past check compares seconds with now', () => {
  const now = Date.UTC(2026, 9, 27, 12)
  assert.equal(isPastUnixSecs(now / 1000 - 60, now), true)
  assert.equal(isPastUnixSecs(now / 1000 + 60, now), false)
})

test('Unix seconds format with their own date', () => {
  // 2026-06-15 is mid-month, so the year survives any time zone.
  assert.match(fmtUnixSecs(1781524800), /2026/)
  assert.doesNotMatch(fmtUnixSecs(1781524800), /1970/)
})
