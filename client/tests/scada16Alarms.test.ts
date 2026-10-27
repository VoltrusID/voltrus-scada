// SCADA-16 W3: the alarm-flood banner summed 24-hour per-source trigger counts.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { detectFlood } from '../src/alarmFlood.ts'

const NOW = 1_800_000_000
const raised = (agoSecs: number[]) => agoSecs.map(a => ({ triggered_at: NOW - a }))

test('a flood counts the alarms raised in the last five minutes', () => {
  const alarms = raised([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 299])
  assert.deepEqual(detectFlood(alarms, NOW, 50), { count: 12, capped: false })
})

test('older alarms do not count toward a flood', () => {
  // 11 old alarms, 3 recent: the old ones must not push the count over the threshold.
  const alarms = raised([1, 2, 3, 301, 400, 600, 900, 1200, 3600, 7200, 80000])
  assert.equal(detectFlood(alarms, NOW, 50), null)
})

test('ten alarms is not a flood, eleven is', () => {
  assert.equal(detectFlood(raised([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]), NOW, 50), null)
  assert.equal(detectFlood(raised([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]), NOW, 50)?.count, 11)
})

test('the window moves with the clock', () => {
  const alarms = raised([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])
  assert.notEqual(detectFlood(alarms, NOW, 50), null)
  assert.equal(detectFlood(alarms, NOW + 600, 50), null, 'ten minutes later they are out of the window')
})

test('a list cut at the limit with every row recent is a lower bound', () => {
  const alarms = raised(Array.from({ length: 50 }, (_, i) => i))
  assert.deepEqual(detectFlood(alarms, NOW, 50), { count: 50, capped: true })
})
