// SCADA-16 W7/W8: the S7, EtherNet/IP and OPC UA clients sent ip/port, area/offset and lowercase
// enum names the server rejects, and the BACnet object list was read as {objects: [...]}.
// The expected bodies below are the serde shapes in src/models/protocol_device_types.rs.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  OPCUA_MODES, OPCUA_POLICIES, bareList, createdId, enipDeviceBody, enipTagBody, joinHostPort,
  opcuaBrowseNodes, opcuaDeviceBody, s7DeviceBody, s7TagBody, splitHostPort,
} from '../src/protocolPayloads.ts'
import { parseWritePolicy } from '../src/writePolicy.ts'

test('host and port are joined into the url the server stores and split back', () => {
  assert.equal(joinHostPort(' 10.0.0.5 ', 102), '10.0.0.5:102')
  assert.deepEqual(splitHostPort('10.0.0.5:1102', 102), { host: '10.0.0.5', port: 1102 })
  assert.deepEqual(splitHostPort('plc.local', 102), { host: 'plc.local', port: 102 })
  assert.deepEqual(splitHostPort('plc.local:abc', 44818), { host: 'plc.local:abc', port: 44818 })
})

test('S7 device body has url, not ip/port, and no string id', () => {
  const body = s7DeviceBody({ name: '', host: '10.0.0.5', port: 102, rack: 0, slot: 2, enabled: true })
  assert.deepEqual(body, { name: '10.0.0.5', url: '10.0.0.5:102', rack: 0, slot: 2, enabled: true })
  assert.ok(!('ip' in body) && !('port' in body) && !('id' in body))
})

test('S7 tag body carries tag_address instead of area/db_number/offset', () => {
  const body = s7TagBody({ name: ' flow ', tag_address: ' DB1.DBD0 ', data_type: 'float32' })
  assert.deepEqual(body, { tag_address: 'DB1.DBD0', name: 'flow', display_name: '', data_type: 'float32' })
})

test('EtherNet/IP device and tag bodies match the server structs', () => {
  assert.deepEqual(
    enipDeviceBody({ name: 'Logix', host: '192.168.1.10', port: 44818, slot: 1, assembly_instance: 0, enabled: false }),
    { name: 'Logix', url: '192.168.1.10:44818', slot: 1, assembly_instance: 0, enabled: false },
  )
  assert.deepEqual(
    enipTagBody({ name: 'flow', tag_name: 'Program.Flow', data_type: 'dint32' }),
    { tag_name: 'Program.Flow', name: 'flow', display_name: '', data_type: 'dint32', assembly_offset: 0, assembly_length: 0 },
  )
})

test('OPC UA selects use the server enum spellings and the default body deserialises', () => {
  assert.deepEqual(OPCUA_POLICIES.map(o => o.value), ['None', 'basic256sha256'])
  assert.deepEqual(OPCUA_MODES.map(o => o.value), ['None', 'Sign', 'signandencrypt'])
  const body = opcuaDeviceBody({
    name: ' srv ', url: 'opc.tcp://h:4840', security_policy: 'none', security_mode: 'sign',
    username: '', password: '', interval_ms: 0,
  })
  // The old lowercase spellings are not valid server variants and must not be sent.
  assert.equal(body.security_policy, 'None')
  assert.equal(body.security_mode, 'None')
  assert.deepEqual(body, {
    name: 'srv', url: 'opc.tcp://h:4840', security_policy: 'None', security_mode: 'None',
    username: null, password: null, interval_ms: 1000,
  })
  assert.equal(opcuaDeviceBody({ ...body, username: 'u', password: 'p', security_policy: 'basic256sha256', security_mode: 'signandencrypt' }).security_mode, 'signandencrypt')
})

test('OPC UA browse answer is read from nodes', () => {
  assert.deepEqual(opcuaBrowseNodes({ nodes: [{ node_id: 'ns=2;i=1' }], count: 1 }), [{ node_id: 'ns=2;i=1' }])
  assert.deepEqual(opcuaBrowseNodes(undefined), [])
})

test('create answers {ok, id} with a numeric id', () => {
  assert.equal(createdId({ ok: true, id: 7 }), 7)
  assert.throws(() => createdId({ ok: true }))
})

test('BACnet object list is the bare array the server answers', () => {
  const objects = [{ id: 1, object_type: 'analogInput' }]
  assert.deepEqual(bareList(objects), objects)
  assert.deepEqual(bareList([]), [])
  // {objects: [...]} is the shape the web used to expect; it is not what the server sends.
  assert.throws(() => bareList({ objects }))
})

test('write policy answer is read into the editor shape', () => {
  assert.deepEqual(
    parseWritePolicy({ writable: true, write_min: 0, write_max: 100.5, write_max_rate: 10, write_requires_reason: true, write_deadband: null }),
    { writable: true, write_min: 0, write_max: 100.5, write_max_rate: 10, write_requires_reason: true, write_deadband: null },
  )
  assert.throws(() => parseWritePolicy({ error: 'Data source not found' }))
  assert.throws(() => parseWritePolicy({ writable: true, write_requires_reason: false, write_min: '5' }))
})
