// SCADA-16 W11: the compare panel sent the older half as period 1 ("current") and both
// halves included the midpoint.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { comparePeriods } from '../src/analyticsCompare.ts'

test('current is the recent half and previous the older half', () => {
  const { current, previous } = comparePeriods(1000, 2000)
  assert.deepEqual(current, [1500, 2000])
  assert.deepEqual(previous, [1000, 1499])
})

test('the halves are disjoint and cover the range with inclusive ends', () => {
  for (const [from, to] of [[0, 100], [7, 100], [1000, 1001], [3, 10]]) {
    const { current, previous } = comparePeriods(from, to)
    assert.equal(previous[0], from)
    assert.equal(current[1], to)
    assert.equal(previous[1] + 1, current[0], `no shared or skipped second in ${from}..${to}`)
  }
})
