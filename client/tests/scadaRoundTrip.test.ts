import { test } from 'node:test'
import assert from 'node:assert/strict'
import { loadLayoutFromAPI, saveLayoutToAPI } from '../src/scada2/engine/layout.ts'

// Rows as GET /api/v1/screens/{id}/scada-equipment returns them. The server
// tests (screen_round_trip.rs, mcp.rs) check that these rows come back from
// REST and MCP unchanged; this checks the editor's half: load, then save.

const rows = (items: any[]) => items.map(r => ({ ...r, config: JSON.parse(r.config), sensor_bindings: JSON.parse(r.sensor_bindings) }))
const fallback = () => 'unnamed'

const editorRows = [
  { equipment_id: 'w_pump', equipment_type: 'symbol', equipment_name: 'P-301A', x: 120, y: 80, width: 80, height: 80, rotation: 90,
    sensor_bindings: '[1]', config: JSON.stringify({ symbol: 'pump', rotation: 90, label: 'P-301A', faceplate: 'pump', faceplateMap: { run: 1, cmd: 2 }, z: 0 }) },
  { equipment_id: 'w_pt', equipment_type: 'big_number', equipment_name: 'Outlet', x: 300, y: 80, width: 140, height: 70, rotation: 0,
    sensor_bindings: '[3]', config: JSON.stringify({ decimals: 2, z: 1 }) },
]
const connections = [
  { connection_id: 'c1', from_equipment: 'w_pump', to_equipment: 'w_pt', connection_type: 'flow', waypoints: '[[200,120],[300,120]]' },
]

test('rows the editor wrote load and save back identically', () => {
  const layout = loadLayoutFromAPI(editorRows as any, connections as any)
  const saved = saveLayoutToAPI(layout, fallback)
  assert.deepEqual(rows(saved.equipment), rows(editorRows))
  assert.deepEqual(saved.connections, connections)
})

test('rows made by the API or a project import keep their name, rotation and order', () => {
  // No label, rotation only in its column, no stacking order in config: as an
  // agent or a script may write them.
  const apiRows = [
    { equipment_id: 'b', equipment_type: 'label', equipment_name: 'Title', x: 0, y: 0, width: 100, height: 20, rotation: 0,
      sensor_bindings: '[]', config: JSON.stringify({ text: 'Pump station' }) },
    { equipment_id: 'a', equipment_type: 'symbol', equipment_name: 'P-302', x: 10, y: 40, width: 60, height: 60, rotation: 45,
      sensor_bindings: '[7]', config: JSON.stringify({ symbol: 'pump' }) },
  ]
  const layout = loadLayoutFromAPI(apiRows as any, [])
  assert.equal(layout.widgets[1].config.rotation, 45, 'the editor shows the column rotation')
  const saved = rows(saveLayoutToAPI(layout, fallback).equipment)
  const before = rows(apiRows)
  for (const [i, r] of saved.entries()) {
    const { config, ...rest } = r
    const { config: c0, ...rest0 } = before[i]
    assert.deepEqual(rest, rest0, `columns of ${r.equipment_id}`)
    // Only the stacking order (and a rotation the column already had) is added.
    assert.deepEqual(config, { ...c0, ...(rest0.rotation ? { rotation: rest0.rotation } : {}), z: i })
  }
  // Saved once more, nothing changes.
  const twice = rows(saveLayoutToAPI(loadLayoutFromAPI(saveLayoutToAPI(layout, fallback).equipment, []), fallback).equipment)
  assert.deepEqual(twice, saved)
})
