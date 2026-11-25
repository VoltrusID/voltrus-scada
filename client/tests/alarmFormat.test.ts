import assert from 'node:assert/strict'
import { test } from 'node:test'
import { formatAlarmLimit, formatAlarmNumber, formatAlarmReading, tidyAlarmMessage } from '../src/alarmFormat.ts'

test('alarm values use the tag decimals', () => {
  assert.equal(formatAlarmNumber(2.602250099182129, 2), '2.60')
  assert.equal(formatAlarmNumber(1166.8177490234375, 0), '1167')
  assert.equal(formatAlarmNumber(0.1234, 3), '0.123')
  // Out-of-range settings are clamped, not thrown on.
  assert.equal(formatAlarmNumber(2.6, -1), '3')
  assert.equal(formatAlarmNumber(2.602250099182129, 40), '2.602250')
})

test('without the tag, decimals follow the size of the value', () => {
  assert.equal(formatAlarmNumber(2.602250099182129), '2.60')
  assert.equal(formatAlarmNumber(25.04), '25.0')
  assert.equal(formatAlarmNumber(1601.52), '1602')
  assert.equal(formatAlarmNumber(Number.NaN, 2), 'NaN')
})

test('readings carry the unit like the server message', () => {
  assert.equal(formatAlarmReading(2.602250099182129, 2, 'm'), '2.60 m')
  assert.equal(formatAlarmReading(96.04, 1, '%'), '96.0%')
  assert.equal(formatAlarmReading(21.46, 1, '°C'), '21.5°C')
  assert.equal(formatAlarmReading(1, 0, ''), '1')
  assert.equal(formatAlarmReading(1, 0, null), '1')
})

test('old alarm messages lose their float noise', () => {
  assert.equal(
    tidyAlarmMessage('Filter F-302 Headloss reached 2.3550000190734863m (warning: 2.2m)', 2),
    'Filter F-302 Headloss reached 2.36m (warning: 2.2m)',
  )
  assert.equal(
    tidyAlarmMessage('intake_flow reached 1166.8177490234375 (critical: 90)'),
    'intake_flow reached 1167 (critical: 90)',
  )
  // Messages the server already rounds are unchanged.
  const fresh = 'Filter F-301 Headloss reached 2.60 m (critical: 2.60 m)'
  assert.equal(tidyAlarmMessage(fresh, 2), fresh)
})

test('rounding matches the server: halves away from zero, no negative zero', () => {
  assert.equal(formatAlarmNumber(2.5, 0), '3')
  assert.equal(formatAlarmNumber(0.125, 2), '0.13')
  assert.equal(formatAlarmNumber(-2.5, 0), '-3')
  assert.equal(formatAlarmNumber(-0.001, 2), '0.00')
  assert.equal(formatAlarmNumber(-0, 1), '0.0')
  assert.equal(formatAlarmNumber(-12.345, 1), '-12.3')
  assert.equal(formatAlarmNumber(Number.POSITIVE_INFINITY, 1), 'inf')
  assert.equal(formatAlarmNumber(Number.NEGATIVE_INFINITY), '-inf')
})

test('limits keep the digits they need', () => {
  assert.equal(formatAlarmLimit(0.5, 0, ''), '0.5')
  assert.equal(formatAlarmLimit(0.05, 1, 'mg/L'), '0.05 mg/L')
  assert.equal(formatAlarmLimit(2, 2, 'm'), '2.00 m')
  assert.equal(formatAlarmLimit(0.1 + 0.2, 1, ''), '0.300000')
})

test('only the value and limit of a generated message are rewritten', () => {
  assert.equal(
    tidyAlarmMessage('Tank 2.123456789 level dropped to -0.0000001% (critical low: 5.0%)', 1),
    'Tank 2.123456789 level dropped to 0.0% (critical low: 5.0%)',
  )
  const custom = 'Calibration offset 0.1234567 applied by maintenance'
  assert.equal(tidyAlarmMessage(custom, 2), custom)
})
