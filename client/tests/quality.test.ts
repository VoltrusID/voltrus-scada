import assert from 'node:assert/strict'
import { test } from 'node:test'
import { highLimits, qualityFamily, qualityLabel, qualityMarker } from '../src/quality.ts'

test('quality families', () => {
  assert.equal(qualityFamily(undefined), 'good')
  assert.equal(qualityFamily('good'), 'good')
  assert.equal(qualityFamily('uncertain_stale'), 'uncertain')
  assert.equal(qualityFamily('bad_comm_failure'), 'bad')
  assert.equal(qualityMarker('bad_device_error'), '✕')
  assert.equal(qualityMarker('uncertain'), '?')
  assert.equal(qualityMarker('good'), '')
  assert.equal(qualityLabel('good'), '')
  assert.match(qualityLabel('uncertain_stale'), /Stale/)
})

test('high limits: legacy 0 is unset, explicit flags allow 0 and negatives', () => {
  assert.deepEqual(highLimits({ warn_val: 0, crit_val: 90, source_config: '{}' }), { warn: null, crit: 90 })
  assert.deepEqual(
    highLimits({ warn_val: -10, crit_val: 0, source_config: '{"warn_enabled":true,"crit_enabled":true}' }),
    { warn: -10, crit: 0 },
  )
  assert.deepEqual(highLimits({ warn_val: 80, crit_val: 90, source_config: '{"warn_enabled":false}' }), { warn: null, crit: 90 })
  assert.deepEqual(highLimits({ warn_val: 80, crit_val: 90, source_config: 'not json' }), { warn: 80, crit: 90 })
})
