import { test } from 'node:test'
import assert from 'node:assert/strict'
import * as Y from 'yjs'
import { loadLayoutFromAPI, saveLayoutToAPI } from '../src/scada2/engine/layout.ts'
import { applyEdit, type Doc, type Edit } from '../src/designer/model.ts'
import { loadRows, orderOf, readDoc, widgetsOf, writeDiff } from '../src/designer/collab/ydoc.ts'
import { LOCAL } from '../src/designer/collab/provider.ts'

const eq = (id: string, x: number, config: Record<string, any>, extra: Record<string, any> = {}) => ({
  equipment_id: id, equipment_type: 'symbol', equipment_name: id.toUpperCase(), x, y: 10, width: 90, height: 100, rotation: 0,
  sensor_bindings: '[1]', config: JSON.stringify(config), ...extra,
}) as any

const rows = () => ({
  equipment: [
    eq('a', 0, { symbol: 'isa_pump', z: 1, bindings: { label: { kind: 'tag', tag: 'd/k' } } }),
    eq('b', 100, { label: 'B', z: 0 }),
    eq('r', 300, { z: 2 }, { rotation: 90, equipment_name: '' }),
  ],
  connections: [{ connection_id: 'c1', from_equipment: 'a', to_equipment: 'b', connection_type: 'flow', waypoints: '[[1,2]]' }] as any,
})

const nameOf = () => 'Fallback'
const yd = () => { const d = new Y.Doc(); const r = rows(); loadRows(d, r.equipment, r.connections); return d }

/** Two replicas that start from the same state. */
function pair(): [Y.Doc, Y.Doc] {
  const a = yd()
  const b = new Y.Doc()
  Y.applyUpdate(b, Y.encodeStateAsUpdate(a))
  return [a, b]
}
const exchange = (a: Y.Doc, b: Y.Doc) => {
  Y.applyUpdate(b, Y.encodeStateAsUpdate(a, Y.encodeStateVector(b)))
  Y.applyUpdate(a, Y.encodeStateAsUpdate(b, Y.encodeStateVector(a)))
}
const run = (d: Y.Doc, e: Edit) => { const cur = readDoc(d); writeDiff(d, cur, applyEdit(cur, e), nameOf, LOCAL) }
const strip = (d: Doc) => JSON.parse(JSON.stringify(d))

test('rows -> Y.Doc -> model is what loadLayoutFromAPI gives', () => {
  const r = rows()
  const fromRows = loadLayoutFromAPI(r.equipment, r.connections)
  const fromY = readDoc(yd())
  assert.deepEqual(strip(fromY), strip(fromRows))
  assert.deepEqual(fromY.widgets.map(w => w.id), ['b', 'a', 'r'])
  assert.equal(fromY.widgets[2].config.rotation, 90)
  assert.equal(fromY.widgets[2].name, undefined, 'an empty name is no name')
  // And back to the same rows as the old save path.
  assert.deepEqual(saveLayoutToAPI(fromY, nameOf), saveLayoutToAPI(fromRows, nameOf))
})

test('readDoc keeps unchanged objects (React skips them)', () => {
  const d = yd()
  const first = readDoc(d)
  assert.equal(readDoc(d, first), first)
  run(d, { type: 'patch', patches: { a: { x: 5 } } })
  const next = readDoc(d, first)
  assert.notEqual(next, first)
  assert.equal(next.widgets.find(w => w.id === 'b'), first.widgets.find(w => w.id === 'b'))
})

test('concurrent edits of different properties and bindings of one widget merge', () => {
  const [a, b] = pair()
  run(a, { type: 'patch', patches: { a: { x: 40, config: { label: 'Pump', bindings: { label: { kind: 'tag', tag: 'd/k' }, visible: { kind: 'tag', tag: 'd/v' } } } } } })
  run(b, { type: 'patch', patches: { a: { y: 70, config: { opacity: 0.5, bindings: { label: { kind: 'tag', tag: 'd/k' }, fillColor: { kind: 'expr', expr: '1' } } } } } })
  exchange(a, b)
  const da = readDoc(a), db = readDoc(b)
  assert.deepEqual(strip(da), strip(db))
  const w = da.widgets.find(x => x.id === 'a')!
  assert.equal(w.x, 40); assert.equal(w.y, 70)
  assert.equal(w.config.label, 'Pump'); assert.equal(w.config.opacity, 0.5)
  assert.deepEqual(Object.keys(w.config.bindings).sort(), ['fillColor', 'label', 'visible'])
})

test('the first bindings of a widget, made by two people at once, both stay', () => {
  // Widget b has no bindings yet: the document still holds an empty map for them.
  const [a, b] = pair()
  run(a, { type: 'patch', patches: { b: { config: { bindings: { visible: { kind: 'tag', tag: 'd/v' } } } } } })
  run(b, { type: 'patch', patches: { b: { config: { bindings: { label: { kind: 'tag', tag: 'd/l' } } } } } })
  exchange(a, b)
  const w = readDoc(a).widgets.find(x => x.id === 'b')!
  assert.deepEqual(Object.keys(w.config.bindings).sort(), ['label', 'visible'])
  assert.deepEqual(strip(readDoc(a)), strip(readDoc(b)))
  // Unbinding the last one leaves no bindings key in the model.
  run(a, { type: 'patch', patches: { b: { config: { bindings: undefined } } } })
  assert.equal(readDoc(a).widgets.find(x => x.id === 'b')!.config.bindings, undefined)
})

test('the same field edited twice converges on one value', () => {
  const [a, b] = pair()
  run(a, { type: 'patch', patches: { b: { x: 1 } } })
  run(b, { type: 'patch', patches: { b: { x: 2 } } })
  exchange(a, b)
  assert.equal(readDoc(a).widgets.find(w => w.id === 'b')!.x, readDoc(b).widgets.find(w => w.id === 'b')!.x)
})

test('a reorder and a concurrent delete agree; z order comes out as 0, 1, 2 in rows', () => {
  const [a, b] = pair()
  run(a, { type: 'order', ids: ['b'], to: 'front' })
  run(b, { type: 'remove', ids: ['a'] })
  exchange(a, b)
  assert.deepEqual(orderOf(a), orderOf(b))
  assert.deepEqual(readDoc(a).widgets.map(w => w.id), ['r', 'b'])
  const saved = saveLayoutToAPI(readDoc(a), nameOf)
  assert.deepEqual(saved.equipment.map(e => [e.equipment_id, JSON.parse(e.config).z]), [['r', 0], ['b', 1]])
  assert.equal(readDoc(a).connections.length, 0, 'the connection to the removed widget went with it')
})

test('only widgets whose place changed get a new stacking index', () => {
  const d = yd()
  const zi = () => Object.fromEntries([...widgetsOf(d).entries()].map(([id, m]) => [id, m.get('zi')]))
  const before = zi()
  run(d, { type: 'order', ids: ['b'], to: 'forward' })
  const after = zi()
  assert.deepEqual(readDoc(d).widgets.map(w => w.id), ['a', 'b', 'r'])
  assert.equal(['a', 'b', 'r'].filter(id => before[id] !== after[id]).length, 1)
  run(d, { type: 'add', widgets: [{ id: 'n', type: 'big_number', sensorId: null, sensorIds: [], x: 0, y: 0, width: 10, height: 10, config: {} }] })
  assert.deepEqual(readDoc(d).widgets.map(w => w.id), ['a', 'b', 'r', 'n'])
  assert.equal(widgetsOf(d).get('n')!.get('name'), 'Fallback', 'name as saveLayoutToAPI writes it')
})

test('two people adding widgets at once keep both, each on top in a stable order', () => {
  const [a, b] = pair()
  const nw = (id: string) => ({ id, type: 'big_number' as const, sensorId: null, sensorIds: [], x: 0, y: 0, width: 10, height: 10, config: {} })
  run(a, { type: 'add', widgets: [nw('pa')] })
  run(b, { type: 'add', widgets: [nw('pb')] })
  exchange(a, b)
  assert.deepEqual(orderOf(a), orderOf(b))
  assert.deepEqual(orderOf(a).slice(0, 3), ['b', 'a', 'r'])
})

test('connection edits merge field by field', () => {
  const [a, b] = pair()
  run(a, { type: 'patchConnection', id: 'c1', patch: { connectionType: 'power' } })
  run(b, { type: 'patchConnection', id: 'c1', patch: { waypoints: [[5, 5]] } })
  exchange(a, b)
  const c = readDoc(a).connections[0]
  assert.equal(c.connectionType, 'power')
  assert.deepEqual(c.waypoints, [[5, 5]])
})
