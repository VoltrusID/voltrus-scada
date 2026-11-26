import { test } from 'node:test'
import assert from 'node:assert/strict'
import { isValid, keyBetween, nKeysBetween } from '../src/designer/collab/fractional.ts'

test('the same keys as collab/src/fractional.rs', () => {
  assert.equal(keyBetween(null, null), 'V')
  assert.equal(keyBetween('V', null), 'l')
  assert.equal(keyBetween(null, 'V'), 'G')
  assert.equal(keyBetween('V', 'W'), 'VV')
  assert.equal(keyBetween('z', null), 'zV')
  assert.deepEqual(nKeysBetween(null, null, 3), ['G', 'V', 'l'])
})

test('keys sort between their bounds, also after many inserts at one place', () => {
  let lo = keyBetween(null, null)
  const hi = keyBetween(lo, null)
  for (let i = 0; i < 200; i++) {
    const k = keyBetween(lo, hi)
    assert.ok(lo < k && k < hi && isValid(k), `${lo} < ${k} < ${hi}`)
    lo = k
  }
  let top = hi
  for (let i = 0; i < 200; i++) { const k = keyBetween(null, top); assert.ok(k < top && isValid(k)); top = k }
})

test('many keys are ordered and short; bad bounds still give a key', () => {
  const keys = nKeysBetween(null, null, 1000)
  assert.equal(keys.length, 1000)
  assert.ok(keys.every((k, i) => i === 0 || keys[i - 1] < k))
  assert.ok(keys.every(k => isValid(k) && k.length <= 4))
  const inner = nKeysBetween(keys[3], keys[4], 10)
  assert.ok(keys[3] < inner[0] && inner[9] < keys[4])
  assert.ok(isValid(keyBetween('b', 'a')))
  assert.ok(isValid(keyBetween('a0', null)))
  assert.ok(isValid(keyBetween('!', '')))
})
