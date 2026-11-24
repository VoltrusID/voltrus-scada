import { test } from 'node:test'
import assert from 'node:assert/strict'
import { MAX_PENS, addPen, normalisePoints, penAxis, timeRange, yOf } from '../src/trend/trendModel.ts'

const pts = (vals: number[]) => vals.map((value, i) => ({ ts: 1000 * i, value }))

test('each pen is autoscaled to its own data', () => {
  // A 0-5 bar pressure and a 1000-2000 m3/h flow on one chart.
  const pressure = penAxis(pts([3.8, 4.1, 5.4]))!
  const flow = penAxis(pts([1100, 1184, 1900]))!
  assert.ok(pressure.lo < 3.8 && pressure.hi > 5.4 && pressure.hi < 6)
  assert.ok(flow.lo < 1100 && flow.hi > 1900 && flow.lo > 1000)
  // Both use the whole height: their extremes land inside, near the edges.
  for (const [axis, lo, hi] of [[pressure, 3.8, 5.4], [flow, 1100, 1900]] as const) {
    const top = yOf(axis, hi, 100)
    const bottom = yOf(axis, lo, 100)
    assert.ok(top > 0 && top < 10, `top ${top}`)
    assert.ok(bottom < 100 && bottom > 90, `bottom ${bottom}`)
  }
})

test('flat, digital and empty pens get sensible axes', () => {
  const flat = penAxis(pts([20, 20, 20]))!
  assert.equal(yOf(flat, 20, 100), 50)
  const digital = penAxis(pts([0, 1, 1, 0]))!
  assert.deepEqual([digital.lo, digital.hi], [-0.1, 1.1])
  assert.equal(penAxis([]), null)
  assert.equal(penAxis(pts([NaN])), null)
})

test('pens are added once, up to the limit', () => {
  let pens: number[] = []
  for (let i = 1; i <= MAX_PENS + 2; i++) pens = addPen(pens, i)
  assert.equal(pens.length, MAX_PENS)
  assert.equal(pens[pens.length - 1], MAX_PENS + 2)
  assert.deepEqual(addPen([1, 2], 2), [1, 2])
})

test('history in seconds or ms becomes ms, sorted, and spans all pens', () => {
  assert.deepEqual(normalisePoints([{ ts: 20, value: 2 }, { ts: 10, value: 1 }, { ts: 30, value: NaN }]),
    [{ ts: 10000, value: 1 }, { ts: 20000, value: 2 }])
  assert.deepEqual(timeRange([pts([1, 2]), [{ ts: 5000, value: 1 }]]), [0, 5000])
  assert.equal(timeRange([[]]), null)
})
