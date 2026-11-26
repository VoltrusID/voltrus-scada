// Splitting an update over the size limit: the parts, applied in order to a
// fresh replica or to one that has the base, give the same document.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import * as Y from 'yjs'
import { splitUpdate } from '../src/designer/collab/updates.ts'
import { loadRows, readDoc, writeDiff } from '../src/designer/collab/ydoc.ts'
import { applyEdit, type Edit } from '../src/designer/model.ts'
import { LOCAL } from '../src/designer/collab/provider.ts'

const nameOf = () => 'W'
const run = (d: Y.Doc, e: Edit) => { const cur = readDoc(d); writeDiff(d, cur, applyEdit(cur, e), nameOf, LOCAL) }
const row = (id: string, i: number) => ({ equipment_id: id, equipment_type: 'label', equipment_name: id, x: i, y: 0, width: 10, height: 10, rotation: 0, sensor_bindings: '[]', config: JSON.stringify({ text: id, z: i }) }) as any

test('an update over the limit is split into parts under it that converge', () => {
  const base = new Y.Doc()
  loadRows(base, Array.from({ length: 30 }, (_, i) => row(`r${i}`, i)), [])
  const local = new Y.Doc()
  Y.applyUpdate(local, Y.encodeStateAsUpdate(base))
  const updates: Uint8Array[] = []
  local.on('update', (u: Uint8Array) => updates.push(u))
  run(local, { type: 'add', widgets: Array.from({ length: 50 }, (_, i) => ({ id: `n${i}`, type: 'label' as const, sensorId: null, sensorIds: [], x: i, y: i, width: 5, height: 5, config: { text: 't'.repeat(300 + i) } })) })
  run(local, { type: 'remove', ids: Array.from({ length: 30 }, (_, i) => (i % 2 ? `r${i}` : `n${i}`)) })
  for (let i = 0; i < 10; i++) run(local, { type: 'patch', patches: { r0: { x: 100 + i, config: { bindings: { text: { kind: 'tag', tag: `t${i}` } } } } } })
  const merged = Y.mergeUpdates(updates)
  const max = 1500
  assert.ok(merged.length > max * 5)
  const parts = splitUpdate(merged, max)!
  assert.ok(parts.length > 5)
  for (const p of parts) assert.ok(p.length <= max, `part of ${p.length} bytes`)

  const onBase = new Y.Doc()
  Y.applyUpdate(onBase, Y.encodeStateAsUpdate(base))
  for (const p of parts) Y.applyUpdate(onBase, p)
  assert.deepEqual(readDoc(onBase), readDoc(local))

  const fresh = new Y.Doc()
  Y.applyUpdate(fresh, Y.encodeStateAsUpdate(base))
  const viaMerged = new Y.Doc()
  Y.applyUpdate(viaMerged, Y.encodeStateAsUpdate(base))
  Y.applyUpdate(viaMerged, merged)
  for (const p of parts) Y.applyUpdate(fresh, p)
  assert.deepEqual(Y.encodeStateVector(fresh), Y.encodeStateVector(viaMerged))
  assert.deepEqual(fresh.getMap('widgets').toJSON(), viaMerged.getMap('widgets').toJSON())
})

test('an update under the limit is left whole; one struct over it cannot be split', () => {
  const d = new Y.Doc()
  const updates: Uint8Array[] = []
  d.on('update', (u: Uint8Array) => updates.push(u))
  d.getMap('widgets').set('a', 'x'.repeat(4000))
  const u = Y.mergeUpdates(updates)
  assert.deepEqual(splitUpdate(u, 10_000), [u])
  assert.equal(splitUpdate(u, 1000), null)
})
