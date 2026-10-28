import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import {
  HISTORY_KEYS, applyHistoryDeadband, describeHistoryDeadband, historyDeadbandWarnings,
  readHistoryDeadband, validateHistoryDeadband, type HistoryDeadbandForm,
} from '../src/historyDeadband.ts'
import { formatBytes, formatDuration } from '../src/storageFormat.ts'

const form = (over: Partial<HistoryDeadbandForm>): HistoryDeadbandForm => ({ type: 'absolute', value: '', maxInterval: '', ...over })

test('the keys written are the ones the server reads', () => {
  const rust = readFileSync(new URL('../../src/deadband.rs', import.meta.url), 'utf8')
  for (const key of HISTORY_KEYS) assert.ok(rust.includes(`"${key}"`), key)
  assert.ok(rust.includes('"percent"'))
})

test('applying a form sets the keys and keeps every other one', () => {
  const cfg = { deadband: 2, low_warn: 5, expression: 'a+b' }
  const out = applyHistoryDeadband(cfg, form({ type: 'percent', value: '0.5', maxInterval: '300' }))
  assert.deepEqual(out, { ...cfg, history_deadband: 0.5, history_deadband_type: 'percent', history_max_interval_secs: 300 })
  // the alarm hysteresis is untouched, and the input object is not mutated
  assert.equal(out.deadband, 2)
  assert.deepEqual(cfg, { deadband: 2, low_warn: 5, expression: 'a+b' })
})

test('absolute is the default type and is not written', () => {
  assert.deepEqual(applyHistoryDeadband({}, form({ value: '0' })), { history_deadband: 0 })
})

test('an empty form or null removes the history keys', () => {
  const cfg = { deadband: 1, history_deadband: 3, history_deadband_type: 'percent', history_max_interval_secs: 60 }
  assert.deepEqual(applyHistoryDeadband(cfg, null), { deadband: 1 })
  assert.deepEqual(applyHistoryDeadband(cfg, form({})), { deadband: 1 })
})

test('reading mirrors the server rules', () => {
  assert.deepEqual(readHistoryDeadband({ history_deadband: 1.5, history_max_interval_secs: 120 }), { type: 'absolute', value: '1.5', maxInterval: '120' })
  assert.deepEqual(readHistoryDeadband({ history_deadband: 2, history_deadband_type: 'percent' }), { type: 'percent', value: '2', maxInterval: '' })
  // negative, non-number: the server ignores the setting, so the form shows none
  assert.equal(readHistoryDeadband({ history_deadband: -1 }).value, '')
  assert.equal(readHistoryDeadband({ history_deadband: '2' }).value, '')
  // the server reads the heartbeat with as_i64, so a fraction would be dropped
  assert.equal(readHistoryDeadband({ history_deadband: 1, history_max_interval_secs: 1.5 }).maxInterval, '')
  assert.equal(readHistoryDeadband({ deadband: 4 }).value, '')
})

test('validation rejects what the server would ignore', () => {
  assert.deepEqual(validateHistoryDeadband(form({})), [])
  assert.deepEqual(validateHistoryDeadband(form({ value: '0' })), [])
  assert.equal(validateHistoryDeadband(form({ value: '-1' })).length, 1)
  assert.equal(validateHistoryDeadband(form({ value: 'abc' })).length, 1)
  assert.equal(validateHistoryDeadband(form({ value: 'Infinity' })).length, 1)
  assert.equal(validateHistoryDeadband(form({ type: 'percent', value: '101' })).length, 1)
  assert.deepEqual(validateHistoryDeadband(form({ type: 'percent', value: '100' })), [])
  assert.equal(validateHistoryDeadband(form({ value: '1', maxInterval: '0' })).length, 1)
  assert.equal(validateHistoryDeadband(form({ value: '1', maxInterval: '2.5' })).length, 1)
  assert.equal(validateHistoryDeadband(form({ maxInterval: '60' })).length, 1)
  assert.throws(() => applyHistoryDeadband({}, form({ value: '-3' })))
})

test('a band without a heartbeat is flagged, a band of zero is not', () => {
  assert.equal(historyDeadbandWarnings(form({ value: '1' })).length, 1)
  assert.equal(historyDeadbandWarnings(form({ value: '1', maxInterval: '300' })).length, 0)
  assert.equal(historyDeadbandWarnings(form({ value: '0' })).length, 0)
  assert.equal(historyDeadbandWarnings(form({ value: '-1' })).length, 0)
})

test('one-line description', () => {
  assert.equal(describeHistoryDeadband({}), null)
  assert.equal(describeHistoryDeadband({ history_deadband: 0.5, history_deadband_type: 'percent', history_max_interval_secs: 300 }), '0.5 %, heartbeat 300 s')
  assert.equal(describeHistoryDeadband({ history_deadband: 2 }), '2')
})

test('storage panel formatting', () => {
  assert.equal(formatDuration(7776000), '90 days')
  assert.equal(formatDuration(3600), '1 hour')
  assert.equal(formatDuration(600), '10 minutes')
  assert.equal(formatDuration(null), 'forever')
  assert.equal(formatDuration(90), '90 seconds')
  assert.equal(formatBytes(512), '512 B')
  assert.equal(formatBytes(1536), '1.50 KB')
  assert.equal(formatBytes(5 * 1024 ** 3), '5.00 GB')
})
