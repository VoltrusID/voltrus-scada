// SCADA-16 B4/B5: the web asked for every tag of a screen when one tag's detail was open,
// and dropped the min/max arrays of agg=minmax.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { bandPoints, historyUrl } from '../src/historyQuery.ts'
import { expandCompactData } from '../src/utils.ts'

test('history URL names the screen, or the one tag of a detail view', () => {
  assert.equal(historyUrl({ range: '30m', screen: 'all' }), '/api/v1/history?range=30m')
  assert.equal(historyUrl({ range: '1h', screen: 'plant' }), '/api/v1/history?range=1h&screen=plant')
  assert.equal(
    historyUrl({ range: '6h', sensor: 'inv.p', detail: 'auto', agg: 'minmax' }),
    '/api/v1/history?range=6h&sensor=inv.p&agg=minmax',
  )
})

test('full detail sends no aggregation', () => {
  assert.equal(
    historyUrl({ range: '24h', sensor: 'a', detail: 'full', agg: 'minmax' }),
    '/api/v1/history?range=24h&sensor=a&detail=full',
  )
})

test('tag keys are escaped in the URL', () => {
  assert.equal(
    historyUrl({ range: '30m', sensor: 'a&b c' }),
    '/api/v1/history?range=30m&sensor=a%26b+c',
  )
})

test('min and max arrays become the lo/hi of each point', () => {
  const out = expandCompactData({
    p: { values: [2, 4], min: [1, 3], max: [3, 5], start: 1000, interval: 500, count: 2 },
    q: { values: [7], start: 0, interval: 1000, count: 1 },
  })
  assert.deepEqual(out.p, [
    { ts: 1000, value: 2, lo: 1, hi: 3 },
    { ts: 1500, value: 4, lo: 3, hi: 5 },
  ])
  assert.deepEqual(out.q, [{ ts: 0, value: 7 }], 'no band without min/max')
})

test('a band of the wrong length is ignored', () => {
  const out = expandCompactData({ p: { values: [2, 4], min: [1], max: [3], start: 0, interval: 1 } })
  assert.deepEqual(out.p, [{ ts: 0, value: 2 }, { ts: 1, value: 4 }])
})

test('band points are epoch seconds, ordered, and need both edges', () => {
  const band = bandPoints([
    { ts: 3000, value: 2, lo: 1, hi: 3 },
    { ts: 1000, value: 2, lo: 0, hi: 4 },
    { ts: 2000, value: 2, lo: 1 },
    { ts: 4000, value: 2 },
    { ts: 5000, value: 2, lo: NaN, hi: 3 },
  ])
  assert.deepEqual(band, [
    { time: 1, lo: 0, hi: 4 },
    { time: 3, lo: 1, hi: 3 },
  ])
})
