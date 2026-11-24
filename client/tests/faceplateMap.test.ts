import { test } from 'node:test'
import assert from 'node:assert/strict'
import { FACEPLATE_PRESETS, commandWords, faceplateOf, missingRoles, primaryTag, withRole } from '../src/scada2/hmi/faceplateMap.ts'

const pump = FACEPLATE_PRESETS.find(p => p.id === 'pump')!

test('a symbol opens a faceplate through its config, keeping only numeric tag ids', () => {
  assert.equal(faceplateOf({}), null)
  assert.equal(faceplateOf({ faceplate: '' }), null)
  assert.deepEqual(faceplateOf({ faceplate: 'pump', faceplateMap: { run: 4, fault: 'x', cmd: 9 } }), { id: 'pump', map: { run: 4, cmd: 9 } })
})

test('one definition serves many symbols, each with its own mapping', () => {
  const p301a = { faceplate: 'pump', faceplateMap: { run: 1, cmd: 3 } }
  const p301b = { faceplate: 'pump', faceplateMap: { run: 11, cmd: 13 } }
  assert.equal(primaryTag(pump, faceplateOf(p301a)!.map), 1)
  assert.equal(primaryTag(pump, faceplateOf(p301b)!.map), 11)
  assert.deepEqual(missingRoles(pump, {}), ['run'])
  assert.deepEqual(missingRoles(pump, faceplateOf(p301b)!.map), [])
})

test('mapping and unmapping a role keeps the others', () => {
  const c = { faceplate: 'pump', faceplateMap: { run: 1, cmd: 3 } }
  assert.deepEqual(withRole(c, 'fault', 2), { faceplateMap: { run: 1, cmd: 3, fault: 2 } })
  assert.deepEqual(withRole(c, 'cmd', null), { faceplateMap: { run: 1 } })
  assert.deepEqual(commandWords('valve'), ['OPEN', 'CLOSE'])
  assert.deepEqual(commandWords('pump'), ['START', 'STOP'])
})
