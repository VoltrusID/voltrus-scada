import { test } from 'node:test'
import assert from 'node:assert/strict'
import * as Y from 'yjs'
import { loadRows, readDoc } from '../src/designer/collab/ydoc.ts'
import { CollabUndo, editDoc } from '../src/designer/collab/undo.ts'
import type { Edit } from '../src/designer/model.ts'

const nameOf = () => 'W'
const rows = [
  { equipment_id: 'a', equipment_type: 'big_number', equipment_name: 'A', x: 0, y: 0, width: 10, height: 10, rotation: 0, sensor_bindings: '[]', config: '{"z":0}' },
  { equipment_id: 'b', equipment_type: 'big_number', equipment_name: 'B', x: 100, y: 0, width: 10, height: 10, rotation: 0, sensor_bindings: '[]', config: '{"z":1}' },
] as any

function replicas() {
  const a = new Y.Doc(), b = new Y.Doc()
  loadRows(a, rows, [])
  Y.applyUpdate(b, Y.encodeStateAsUpdate(a))
  // Live: every update reaches the other replica as a remote change.
  a.on('update', (u: Uint8Array, o: unknown) => { if (o !== 'remote') Y.applyUpdate(b, u, 'remote') })
  b.on('update', (u: Uint8Array, o: unknown) => { if (o !== 'remote') Y.applyUpdate(a, u, 'remote') })
  return { a, b, ua: new CollabUndo(a), ub: new CollabUndo(b) }
}
const step = (d: Y.Doc, u: CollabUndo, e: Edit, key?: string, at = Date.now()) => { u.before(key, at); editDoc(d, readDoc(d), e, nameOf) }
const x = (d: Y.Doc, id: string) => readDoc(d).widgets.find(w => w.id === id)!.x

test('undo reverts only my own edits, not the other person\'s', () => {
  const { a, b, ua } = replicas()
  step(a, ua, { type: 'patch', patches: { a: { x: 10 } } })
  step(b, new CollabUndo(b), { type: 'patch', patches: { b: { x: 200 } } })
  ua.undo()
  assert.equal(x(a, 'a'), 0)
  assert.equal(x(a, 'b'), 200, 'ben\'s move stands')
  assert.equal(x(b, 'a'), 0, 'and ben sees the undo')
  ua.redo()
  assert.equal(x(b, 'a'), 10)
})

test('nudges with one key in quick succession are one step; a pause or another key starts a new one', () => {
  const { a, ua } = replicas()
  const t = 1_000_000
  step(a, ua, { type: 'patch', patches: { a: { x: 1 } } }, 'nudge:a', t)
  step(a, ua, { type: 'patch', patches: { a: { x: 2 } } }, 'nudge:a', t + 100)
  step(a, ua, { type: 'patch', patches: { a: { x: 3 } } }, 'nudge:a', t + 200)
  step(a, ua, { type: 'patch', patches: { a: { y: 9 } } }, undefined, t + 300)
  assert.equal(ua.um.undoStack.length, 2)
  ua.undo()
  assert.equal(readDoc(a).widgets[0].y, 0)
  assert.equal(x(a, 'a'), 3)
  ua.undo()
  assert.equal(x(a, 'a'), 0)
})

test('undoing a delete brings the widget and its stacking place back', () => {
  const { a, b, ua } = replicas()
  step(a, ua, { type: 'remove', ids: ['a'] })
  assert.deepEqual(readDoc(b).widgets.map(w => w.id), ['b'])
  ua.undo()
  assert.deepEqual(readDoc(b).widgets.map(w => w.id), ['a', 'b'])
})
