import { test } from 'node:test'
import assert from 'node:assert/strict'
import { hasAlarmLimits, offerOnDelay } from '../src/alarmSuggestion.ts'

const tag = (over: Partial<{ warn_val: number; crit_val: number; source_config: string; on_delay_secs: number }>) => ({
  warn_val: 0, crit_val: 0, source_config: '{}', on_delay_secs: 0, ...over,
})

test('a tag counts as alarmed with a high or a low limit', () => {
  assert.equal(hasAlarmLimits(tag({})), false)
  assert.equal(hasAlarmLimits(tag({ warn_val: 18 })), true)
  assert.equal(hasAlarmLimits(tag({ source_config: '{"low_warn": 40}' })), true)
  // An explicit switch turns a limit off even when its value is set.
  assert.equal(hasAlarmLimits(tag({ warn_val: 18, source_config: '{"warn_enabled": false}' })), false)
})

test('the suggestion is offered only when it would change the delay', () => {
  assert.equal(offerOnDelay(tag({ warn_val: 18 }), { on_delay_secs: 5 }), true)
  assert.equal(offerOnDelay(tag({ warn_val: 18, on_delay_secs: 5 }), { on_delay_secs: 5 }), false)
  assert.equal(offerOnDelay(tag({ warn_val: 1 }), { on_delay_secs: 0 }), false)
})
