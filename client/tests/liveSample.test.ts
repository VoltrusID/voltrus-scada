import assert from 'node:assert/strict'
import { test } from 'node:test'
import { isRepeatSample } from '../src/liveSample.ts'
import type { SensorData } from '../src/types.ts'

const reading = (over: Partial<SensorData> = {}): SensorData => ({
  id: 1, key: 'flow', value: 12.5, warn: 80, crit: 90, source_type: 'modbus',
  status: 'normal', quality: 'good', ts_ms: 1_000, ...over,
})

test('the same sample sent again is a repeat', () => {
  assert.equal(isRepeatSample(reading(), reading()), true)
})

test('a new sample time, value, quality or status is news', () => {
  assert.equal(isRepeatSample(reading(), reading({ ts_ms: 2_000 })), false)
  assert.equal(isRepeatSample(reading(), reading({ value: 12.6 })), false)
  assert.equal(isRepeatSample(reading(), reading({ quality: 'uncertain_stale' })), false)
  assert.equal(isRepeatSample(reading(), reading({ status: 'warning' })), false)
})

test('nothing shown yet, or no sample time, is never a repeat', () => {
  assert.equal(isRepeatSample(undefined, reading()), false)
  assert.equal(isRepeatSample(reading({ ts_ms: undefined }), reading({ ts_ms: undefined })), false)
})

test('a NaN reading repeated is a repeat', () => {
  assert.equal(isRepeatSample(reading({ value: NaN }), reading({ value: NaN })), true)
})
