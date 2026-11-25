import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  bindInstancePatch, configOf, defaultPrefix, instanceFieldErrors, instanceOf, suggestName, typeInitials, widgetsByInstance,
} from '../src/designer/equipmentModel.ts'
import { historyReducer, emptyHistory } from '../src/designer/model.ts'
import { loadLayoutFromAPI, saveLayoutToAPI } from '../src/scada2/engine/layout.ts'

const resolvedWidget = {
  equipment_type: 'symbol' as const,
  sensor_id: 12,
  config: { symbol: 'isa_pump_centrifugal', faceplate: 'pump', faceplateMap: { run: 12, cmd: 12 }, showStatus: true, writable: true, label: 'P-101A' },
  remove: ['faultSensorId', 'runSensorId', 'manualSensorId', 'interlockSensorId'],
}

test('the prefix of an instance name is its lower-case letters and digits, as the server computes it', () => {
  assert.equal(defaultPrefix('P-101A'), 'p101a')
  assert.equal(defaultPrefix('FCV 320'), 'fcv320')
  assert.equal(defaultPrefix('--'), '')
})

test('binding a new symbol to an instance applies the server binding as is', () => {
  const p = bindInstancePatch(null, 'P-101A', resolvedWidget)
  assert.equal(p.type, 'symbol')
  assert.equal(p.sensorId, 12)
  assert.deepEqual(p.sensorIds, [12])
  assert.equal(p.name, undefined)
  const c = configOf(p)
  assert.equal(c.equipment, 'P-101A')
  assert.equal(c.symbol, 'isa_pump_centrifugal')
  assert.deepEqual(c.faceplateMap, { run: 12, cmd: 12 })
  assert.equal(c.label, 'P-101A')
  // Removed keys are not in a new widget's config at all.
  assert.ok(!('faultSensorId' in c))
})

test('re-binding keeps a label the engineer typed and removes keys the type no longer binds', () => {
  const w = { config: { label: 'Intake pump A', faultSensorId: 9, x: 1 } }
  const p = bindInstancePatch(w, 'P-101A', { ...resolvedWidget, config: { ...resolvedWidget.config, label: undefined as any } })
  assert.equal(p.config!.label, undefined)
  assert.ok('faultSensorId' in p.config! && p.config!.faultSensorId === undefined, 'fault tag removed by the patch')
  // Applied through the model, the patch merges per key and drops undefined ones.
  const doc = { widgets: [{ id: 'w1', type: 'symbol' as const, sensorId: null, sensorIds: [], x: 0, y: 0, width: 90, height: 100, config: w.config }], connections: [] }
  let h = historyReducer(emptyHistory, { type: 'load', doc })
  h = historyReducer(h, { type: 'edit', edit: { type: 'patch', patches: { w1: p } } })
  const out = h.present.widgets[0]
  assert.equal(out.config.label, 'Intake pump A')
  assert.equal(out.config.equipment, 'P-101A')
  assert.ok(!('faultSensorId' in out.config))
  assert.equal(out.sensorId, 12)
})

test('an instance-bound symbol saves and loads with its instance, faceplate and tags', () => {
  const p = bindInstancePatch(null, 'P-101A', resolvedWidget)
  const w = { id: 'w1', type: 'symbol' as const, sensorId: p.sensorId!, sensorIds: p.sensorIds!, x: 10, y: 20, width: 90, height: 104, config: configOf(p) }
  const rows = saveLayoutToAPI({ widgets: [w], connections: [] }, () => 'P-101A')
  const row = rows.equipment[0]
  assert.equal(row.equipment_type, 'symbol')
  assert.deepEqual(JSON.parse(row.sensor_bindings), [12])
  const cfg = JSON.parse(row.config)
  assert.equal(cfg.equipment, 'P-101A')
  assert.equal(cfg.faceplate, 'pump')
  const back = loadLayoutFromAPI(rows.equipment, rows.connections)
  assert.equal(instanceOf(back.widgets[0]), 'P-101A')
  assert.equal(back.widgets[0].sensorId, 12)
})

test('symbols are grouped by the instance they show', () => {
  const ws = [
    { id: 'a', config: { equipment: 'P-101A' } }, { id: 'b', config: {} }, { id: 'c', config: { equipment: 'P-101A' } }, { id: 'd', config: { equipment: '' } },
  ] as any
  const m = widgetsByInstance(ws)
  assert.deepEqual([...m.entries()], [['P-101A', ['a', 'c']]])
  assert.equal(instanceOf({ config: { equipment: 3 } }), null)
})

test('the next instance name follows the last one of the type, or the type initials', () => {
  const pump = { id: 'pump', name: 'Pump' }
  assert.equal(typeInitials({ id: 'control_valve', name: 'Control valve' }), 'CV')
  assert.equal(suggestName(pump, []), 'P-101')
  assert.equal(suggestName(pump, [{ name: 'P-101A', type: 'pump' }]), 'P-101B')
  assert.equal(suggestName(pump, [{ name: 'P-101A', type: 'pump' }, { name: 'P-101B', type: 'pump' }]), 'P-101C')
  assert.equal(suggestName(pump, [{ name: 'P-204', type: 'pump' }]), 'P-205')
  assert.equal(suggestName(pump, [{ name: 'P-204', type: 'pump' }, { name: 'P-205', type: 'valve' }]), 'P-206')
  assert.equal(suggestName(pump, [{ name: 'P-101', type: 'valve' }]), 'P-102')
})

test('new instance fields are checked before anything is sent', () => {
  assert.deepEqual(instanceFieldErrors({ name: 'P-1', device: 'wtp', base: '' }, []), [])
  assert.deepEqual(instanceFieldErrors({ name: ' ', device: '', base: 'x' }, []),
    ['Name is required', 'Device is required', 'Base must be a whole register address, or empty'])
  assert.match(instanceFieldErrors({ name: 'p-1', device: 'wtp', base: '4' }, [{ name: 'P-1' }])[0], /exists/)
})
