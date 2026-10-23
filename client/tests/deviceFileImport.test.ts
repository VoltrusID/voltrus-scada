// Run with `npm test` (Node's built-in runner, TypeScript types stripped).
// tests/fixtures/modbus-poll-big-map.json is the "Voltrus SCADA device" export of Modbus Poll (packages/mt-export,
// tests/golden/modbus-high.voltrus-scada.json) for this tag list, which has addresses above 30000:
//
//   Edge_30000 holding 30000 uint16 | Float_35000 holding 35000 float32 x0.5 | Count_40000 holding 40000 int32
//   Input_30001 input 30001 int16 | Input_65534 input 65534 uint16 | Coil_40000 coil 40000 | Discrete_50000 discrete 50000
//   Low_5 holding 5 float64
// The Rust side (src/storage/tests/modbus_tests.rs) stores the same file and checks the wire addresses.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { deviceFileKind, parseBacnetDeviceFile, parseModbusDeviceFile } from '../src/deviceFileImport.ts'

const fixture = readFileSync(new URL('./fixtures/modbus-poll-big-map.json', import.meta.url), 'utf8')

const tags = [
  ['Edge_30000', 'holding', 30000, 'uint16', 1],
  ['Float_35000', 'holding', 35000, 'float32', 0.5],
  ['Count_40000', 'holding', 40000, 'int32', 1],
  ['Input_30001', 'input', 30001, 'int16', 1],
  ['Input_65534', 'input', 65534, 'uint16', 1],
  ['Coil_40000', 'coil', 40000, 'uint16', 1],
  ['Discrete_50000', 'discrete', 50000, 'uint16', 1],
  ['Low_5', 'holding', 5, 'float64', 1],
] as const

test('a Modbus Poll export imports every tag with its explicit offset', () => {
  assert.equal(deviceFileKind(fixture), 'modbus')
  const { devices, errors } = parseModbusDeviceFile(fixture)
  assert.deepEqual(errors, [])
  assert.equal(devices.length, 1)
  const [d] = devices
  assert.deepEqual(
    { id: d.id, name: d.name, ip: d.ip, port: d.port, unit_id: d.unit_id, interval_ms: d.interval_ms, endianness: d.endianness },
    { id: 'big_map', name: 'Big map', ip: '10.1.2.3', port: 1502, unit_id: 1, interval_ms: 2000, endianness: 'swapped' },
  )
  assert.deepEqual(
    d.registers,
    tags.map(([name, register_area, offset, register_type, scale]) => ({ name, register_area, offset, register_type, scale })),
  )
})

test('the legacy address form still imports', () => {
  const file = JSON.stringify({
    id: 'old', name: 'Old', ip: '10.0.0.1',
    registers: [{ name: 'v', address: 40001, register_type: 'float32', register_area: 'holding', scale: 1 }, { name: 'w', address: 12 }],
  })
  const { devices, errors } = parseModbusDeviceFile(file)
  assert.deepEqual(errors, [])
  assert.deepEqual(devices[0].registers, [
    { name: 'v', address: 40001, register_type: 'float32', register_area: 'holding', scale: 1 },
    { name: 'w', address: 12, register_type: 'float32', register_area: 'holding', scale: 1 },
  ])
})

test('a register needs exactly one of address and offset, within 0-65535', () => {
  const parse = (r: object) => parseModbusDeviceFile(JSON.stringify({ id: 'd', name: 'D', ip: '1.1.1.1', registers: [r] })).errors
  assert.match(parse({ name: 'a' })[0], /no address or offset/)
  assert.match(parse({ name: 'a', address: 1, offset: 1 })[0], /both address and offset/)
  assert.match(parse({ name: 'a', offset: 65536 })[0], /offset 65536, not 0-65535/)
  assert.match(parse({ name: 'a', offset: -1 })[0], /offset -1/)
  assert.deepEqual(parse({ name: 'a', offset: 65535 }), [])
})

test('BACnet files still import', () => {
  const file = JSON.stringify({ name: 'AHU', ip: '10.0.0.2', device_instance: 7, objects: [{ name: 'T', object_type: 'analogInput', object_instance: 1 }] })
  assert.equal(deviceFileKind(file), 'bacnet')
  const { devices, errors } = parseBacnetDeviceFile(file)
  assert.deepEqual(errors, [])
  assert.equal(devices[0].objects[0].property, 'presentValue')
})
