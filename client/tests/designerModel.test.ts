import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  alignPatches, distributePatches, duplicate, emptyHistory, expandToGroups, groupPatches, historyReducer, isDirty,
  lockPatches, marqueeSelect, matchSizePatches, movePatches, ungroupPatches, bindPrimaryPatch,
  type Doc, type History, type HistoryAction,
} from '../src/designer/model.ts'
import { snapBox, snapValue, rulerStep } from '../src/designer/snap.ts'
import { loadLayoutFromAPI, saveLayoutToAPI } from '../src/scada2/engine/layout.ts'
import { withBinding } from '../src/designer/bindings.ts'

const w = (id: string, x: number, y: number, width = 100, height = 50, config: Record<string, any> = {}) =>
  ({ id, type: 'big_number' as const, sensorId: null, sensorIds: [], x, y, width, height, config })

const doc = (...ws: ReturnType<typeof w>[]): Doc => ({ widgets: ws, connections: [] })
const run = (h: History, ...as: HistoryAction[]) => as.reduce(historyReducer, h)
const loaded = (d: Doc) => historyReducer(emptyHistory, { type: 'load', doc: d })
const STAGE = { w: 1920, h: 1080 }

test('every edit is one undo step; undo and redo walk back and forth', () => {
  let h = loaded(doc(w('a', 0, 0)))
  assert.equal(isDirty(h), false)
  h = run(h,
    { type: 'edit', edit: { type: 'add', widgets: [w('b', 200, 0)] } },
    { type: 'edit', edit: { type: 'patch', patches: { a: { x: 40 } } } },
    { type: 'edit', edit: { type: 'remove', ids: ['b'] } },
  )
  assert.equal(h.past.length, 3)
  assert.deepEqual(h.present.widgets.map(x => x.id), ['a'])
  h = run(h, { type: 'undo' })
  assert.deepEqual(h.present.widgets.map(x => x.id), ['a', 'b'])
  h = run(h, { type: 'undo' }, { type: 'undo' })
  assert.equal(h.present.widgets.length, 1)
  assert.equal(h.present.widgets[0].x, 0)
  assert.equal(isDirty(h), false, 'undoing everything returns to the saved document')
  h = run(h, { type: 'redo' }, { type: 'redo' })
  assert.equal(h.present.widgets[0].x, 40)
  // A new edit clears the redo stack.
  h = run(h, { type: 'edit', edit: { type: 'patch', patches: { a: { y: 5 } } } })
  assert.equal(h.future.length, 0)
  assert.equal(run(h, { type: 'redo' }), h)
})

test('edits that change nothing add no undo step; coalesced edits are one step', () => {
  let h = loaded(doc(w('a', 0, 0)))
  h = run(h, { type: 'edit', edit: { type: 'patch', patches: { a: { x: 0 } } } })
  assert.equal(h.past.length, 0)
  h = run(h,
    { type: 'edit', edit: { type: 'patch', patches: { a: { config: { label: 'P' } } } }, coalesce: 'a:label' },
    { type: 'edit', edit: { type: 'patch', patches: { a: { config: { label: 'P-1' } } } }, coalesce: 'a:label' },
    { type: 'edit', edit: { type: 'patch', patches: { a: { config: { label: 'P-101' } } } }, coalesce: 'a:label' },
  )
  assert.equal(h.past.length, 1)
  assert.equal(h.present.widgets[0].config.label, 'P-101')
  h = run(h, { type: 'undo' })
  assert.equal(h.present.widgets[0].config.label, undefined)
  // Saving marks the document clean; a later edit makes it dirty again.
  h = run(h, { type: 'redo' }, { type: 'saved' })
  assert.equal(isDirty(h), false)
})

test('a config patch merges keys and removes keys set to undefined', () => {
  let h = loaded(doc(w('a', 0, 0, 100, 50, { label: 'x', group: 'g1' })))
  h = run(h, { type: 'edit', edit: { type: 'patch', patches: { a: { config: { group: undefined, decimals: 2 } } } } })
  assert.deepEqual(h.present.widgets[0].config, { label: 'x', decimals: 2 })
})

test('align to the selection bounds, skipping locked widgets', () => {
  const d = doc(w('a', 10, 10, 100, 40), w('b', 200, 80, 60, 20), w('c', 50, 300, 80, 80, { locked: true }))
  assert.deepEqual(alignPatches(d, ['a', 'b'], 'left'), { b: { x: 10, y: 80 } })
  assert.deepEqual(alignPatches(d, ['a', 'b'], 'right'), { a: { x: 160, y: 10 } })
  assert.deepEqual(alignPatches(d, ['a', 'b'], 'top'), { b: { x: 200, y: 10 } })
  assert.deepEqual(alignPatches(d, ['a', 'b'], 'bottom'), { a: { x: 10, y: 60 } })
  assert.deepEqual(alignPatches(d, ['a', 'b'], 'hcenter'), { a: { x: 85, y: 10 }, b: { x: 105, y: 80 } })
  assert.deepEqual(Object.keys(alignPatches(d, ['a', 'b', 'c'], 'left')), ['b'], 'locked c is not moved')
  assert.deepEqual(alignPatches(d, ['a'], 'left'), {}, 'one widget has nothing to align to')
})

test('distribute gives equal gaps and keeps the outer widgets', () => {
  const d = doc(w('a', 0, 0, 100, 10), w('b', 130, 0, 50, 10), w('c', 400, 0, 100, 10))
  // span 0..500, widths 250, gap (500-250)/2 = 125 → b at 225.
  assert.deepEqual(distributePatches(d, ['c', 'a', 'b'], 'h'), { b: { x: 225 } })
  assert.deepEqual(distributePatches(d, ['a', 'b'], 'h'), {})
})

test('match size takes the first selected widget as reference', () => {
  const d = doc(w('a', 0, 0, 120, 60), w('b', 0, 0, 80, 20))
  assert.deepEqual(matchSizePatches(d, ['a', 'b'], 'both'), { b: { width: 120, height: 60 } })
  assert.deepEqual(matchSizePatches(d, ['b', 'a'], 'width'), { a: { width: 80 } })
})

test('moving a selection keeps it on the stage as one shape', () => {
  const d = doc(w('a', 10, 10), w('b', 300, 10))
  assert.deepEqual(movePatches(d, ['a', 'b'], -50, 0, STAGE), { a: { x: 0, y: 10 }, b: { x: 290, y: 10 } })
  assert.deepEqual(movePatches(d, ['a', 'b'], 0, -10, STAGE), { a: { x: 10, y: 0 }, b: { x: 300, y: 0 } })
})

test('groups select together, ungroup and survive save and load', () => {
  let h = loaded(doc(w('a', 0, 0), w('b', 200, 0), w('c', 400, 0)))
  h = run(h, { type: 'edit', edit: { type: 'patch', patches: groupPatches(h.present, ['a', 'b'], 'g1') } })
  assert.deepEqual(expandToGroups(h.present, ['b']), ['a', 'b'])
  assert.deepEqual(expandToGroups(h.present, ['c']), ['c'])
  assert.deepEqual(marqueeSelect(h.present, { x: 150, y: 0, width: 100, height: 10 }), ['a', 'b'], 'a marquee touching one member takes the group')

  // Through the same rows the REST API and project files use.
  const rows = saveLayoutToAPI(h.present, () => 'x')
  const back = loadLayoutFromAPI(rows.equipment, rows.connections)
  assert.deepEqual(expandToGroups(back, ['a']), ['a', 'b'])
  assert.deepEqual(saveLayoutToAPI(back, () => 'x'), rows, 'loaded and saved again, nothing changes')

  h = run(h, { type: 'edit', edit: { type: 'patch', patches: ungroupPatches(h.present, ['a', 'b']) } })
  assert.deepEqual(expandToGroups(h.present, ['a']), ['a'])
  h = run(h, { type: 'undo' })
  assert.deepEqual(expandToGroups(h.present, ['a']), ['a', 'b'])
})

test('locked widgets are skipped by marquee and move', () => {
  let d = doc(w('a', 0, 0), w('b', 200, 0))
  d = historyReducer(loaded(d), { type: 'edit', edit: { type: 'patch', patches: lockPatches(d, ['a'], true) } }).present
  assert.equal(d.widgets[0].config.locked, true)
  assert.deepEqual(marqueeSelect(d, { x: 0, y: 0, width: 400, height: 100 }), ['b'])
  assert.deepEqual(Object.keys(movePatches(d, ['a', 'b'], 10, 0, STAGE)), ['b'])
  assert.deepEqual(lockPatches(d, ['a'], false), { a: { config: { locked: undefined } } })
})

test('z-order: front, back, forward and backward keep the selection order', () => {
  let h = loaded(doc(w('a', 0, 0), w('b', 0, 0), w('c', 0, 0), w('d', 0, 0)))
  const order = () => h.present.widgets.map(x => x.id).join('')
  h = run(h, { type: 'edit', edit: { type: 'order', ids: ['a', 'b'], to: 'front' } })
  assert.equal(order(), 'cdab')
  h = run(h, { type: 'edit', edit: { type: 'order', ids: ['b'], to: 'back' } })
  assert.equal(order(), 'bcda')
  h = run(h, { type: 'edit', edit: { type: 'order', ids: ['c'], to: 'forward' } })
  assert.equal(order(), 'bdca')
  h = run(h, { type: 'edit', edit: { type: 'order', ids: ['a'], to: 'forward' } })
  assert.equal(order(), 'bdca', 'already in front: no step')
  h = run(h, { type: 'edit', edit: { type: 'order', ids: ['c', 'a'], to: 'backward' } })
  assert.equal(order(), 'bcad')
  assert.equal(h.past.length, 4)
  // Stacking order is saved as config.z and comes back.
  const rows = saveLayoutToAPI(h.present, () => 'x')
  assert.deepEqual(loadLayoutFromAPI([...rows.equipment].reverse(), []).widgets.map(x => x.id).join(''), 'bcad')
})

test('duplicate gives new ids, a new group and copies inner connections', () => {
  const a = w('a', 0, 0, 10, 10, { group: 'g1', locked: true })
  const b = w('b', 50, 0, 10, 10, { group: 'g1' })
  const conn = { id: 'c1', fromWidgetId: 'a', toWidgetId: 'b', connectionType: 'flow' as const, waypoints: [[5, 5]] as [number, number][] }
  const out = duplicate([a, b], [conn, { ...conn, id: 'c2', toWidgetId: 'zz' }], 20)
  assert.equal(out.widgets.length, 2)
  assert.notEqual(out.widgets[0].id, 'a')
  assert.equal(out.widgets[0].x, 20)
  assert.equal(out.widgets[0].config.group, out.widgets[1].config.group)
  assert.notEqual(out.widgets[0].config.group, 'g1')
  assert.equal(out.widgets[0].config.locked, undefined)
  assert.equal(a.config.group, 'g1', 'the original is untouched')
  assert.equal(out.connections.length, 1)
  assert.deepEqual([out.connections[0].fromWidgetId, out.connections[0].toWidgetId], out.widgets.map(x => x.id))
  assert.deepEqual(out.connections[0].waypoints, [[25, 25]])
})

test('binding the primary tag and a property is undoable and saved in the row', () => {
  let h = loaded(doc({ ...w('a', 0, 0), name: 'Old name' }))
  h = run(h, { type: 'edit', edit: { type: 'patch', patches: { a: bindPrimaryPatch(7) } } })
  assert.deepEqual([h.present.widgets[0].sensorId, h.present.widgets[0].sensorIds, h.present.widgets[0].name], [7, [7], undefined])
  const config = withBinding(h.present.widgets[0].config, 'label', { kind: 'expr', expr: 'text({wtp/flow}, 1)' })
  h = run(h, { type: 'edit', edit: { type: 'patch', patches: { a: { config } } } })
  const row = saveLayoutToAPI(h.present, () => 'x').equipment[0]
  assert.equal(row.sensor_bindings, '[7]')
  assert.deepEqual(JSON.parse(row.config).bindings, { label: { kind: 'expr', expr: 'text({wtp/flow}, 1)' } })
  h = run(h, { type: 'undo' }, { type: 'undo' })
  assert.equal(h.present.widgets[0].sensorId, null)
  assert.equal(h.present.widgets[0].config.bindings, undefined)
  assert.equal(h.present.widgets[0].name, 'Old name')
})

test('snapping: object edges and guides win over the grid within the threshold', () => {
  const others = [{ x: 100, y: 100, width: 100, height: 50 }]
  const o = { grid: 10, threshold: 6, objects: true, guides: [{ axis: 'y' as const, pos: 403 }], stage: STAGE }
  // Left edge 3 px right of the other's right edge (200) → snaps to 200.
  const r = snapBox({ x: 203, y: 600, width: 40, height: 40 }, others, o)
  assert.equal(r.x, 200)
  assert.equal(r.y, 600, 'no y target near: grid')
  assert.deepEqual(r.lines, [{ axis: 'x', pos: 200 }])
  // Centre of the moving box near the other's centre (150).
  assert.equal(snapBox({ x: 128, y: 0, width: 40, height: 10 }, others, o).x, 130)
  // Guide at y 403: bottom edge 401 snaps.
  assert.equal(snapBox({ x: 700, y: 361, width: 10, height: 40 }, others, o).y, 363)
  // Nothing close: grid.
  assert.deepEqual(snapBox({ x: 714, y: 716, width: 3, height: 3 }, [], { ...o, guides: [] }), { x: 710, y: 720, lines: [] })
  assert.equal(snapValue(197, 'x', others, o).v, 200)
  assert.equal(snapValue(244, 'x', others, { ...o, objects: false }).v, 240)
  assert.equal(rulerStep(1).major, 100)
  assert.equal(rulerStep(0.25).major, 500)
})
