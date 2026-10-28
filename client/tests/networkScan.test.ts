import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  cidrError, defaultScanRange, deviceIdFor, hostCount, isAlreadyAdded, newDevicePayload, scanSize, estimatedSeconds,
  type DiscoveredDevice,
} from '../src/networkScan.ts'

test('cidrError accepts what the server accepts and explains the rest', () => {
  for (const ok of ['192.168.1.0/24', '10.0.0.5/30', '10.0.0.5/32', ' 192.168.1.0/24 ', '172.16.5.9/25']) {
    assert.equal(cidrError(ok), null, ok)
  }
  assert.match(cidrError('10.0.0.0/16')!, /65,534 hosts.*\/24/)
  assert.match(cidrError('0.0.0.0/0')!, /largest range/)
  assert.match(cidrError('')!, /Enter a range/)
  assert.match(cidrError('192.168.1.0')!, /address\/prefix/)
  assert.match(cidrError('192.168.1/24')!, /not an IPv4 address/)
  assert.match(cidrError('300.1.1.1/24')!, /not an IPv4 address/)
  assert.match(cidrError('10.0.0.0/abc')!, /not a prefix/)
  assert.match(cidrError('10.0.0.0/33')!, /0 to 32/)
  assert.match(cidrError('10.0.0.0/31')!, /no usable host/)
})

test('hostCount and scanSize match the server', () => {
  assert.equal(hostCount(24), 254)
  assert.equal(hostCount(30), 2)
  assert.equal(hostCount(32), 1)
  assert.equal(scanSize('192.168.1.0/24'), 254)
  assert.equal(scanSize('10.0.0.0/16'), 0)
  assert.equal(estimatedSeconds(254), 9)
  assert.equal(estimatedSeconds(1), 1.5)
})

test('the default range is the /24 of the first real IPv4 address', () => {
  assert.equal(
    defaultScanRange([
      { name: 'lo', addresses: ['127.0.0.1/8', '::1/128'] },
      { name: 'docker0', addresses: ['172.17.0.1/16'] },
      { name: 'eth0', addresses: ['fe80::1/64', '192.168.7.42/24'] },
    ]),
    '192.168.7.0/24',
  )
})

test('a wider subnet is narrowed to /24 and a narrower one is kept', () => {
  assert.equal(defaultScanRange([{ name: 'eth0', addresses: ['10.20.30.40/16'] }]), '10.20.30.0/24')
  assert.equal(defaultScanRange([{ name: 'eth0', addresses: ['10.20.30.70/26'] }]), '10.20.30.64/26')
})

test('no usable address means no default', () => {
  assert.equal(defaultScanRange([]), null)
  assert.equal(
    defaultScanRange([
      { name: 'lo', addresses: ['127.0.0.1/8'] },
      { name: 'wlan0', addresses: ['169.254.3.4/16', 'fe80::2/64'] },
      { name: 'eth1', addresses: ['10.0.0.9/32'] },
    ]),
    null,
  )
})

const found = (ip: string, port = 502, unit = 1): DiscoveredDevice => ({
  ip, port, unit_id: unit,
  responsive_registers: [{ address: 0, value: 12, raw: [12] }, { address: 3, value: 7, raw: [7] }],
})

test('already-added devices are recognised by address and port', () => {
  const existing = [{ ip: '192.168.1.20', port: 502 }]
  assert.equal(isAlreadyAdded(found('192.168.1.20'), existing), true)
  assert.equal(isAlreadyAdded(found('192.168.1.20', 5020), existing), false)
  assert.equal(isAlreadyAdded(found('192.168.1.21'), existing), false)
})

test('device ids are readable and never collide', () => {
  assert.equal(deviceIdFor(found('192.168.1.20'), new Set()), 'modbus-192-168-1-20')
  assert.equal(deviceIdFor(found('192.168.1.20', 5020), new Set()), 'modbus-192-168-1-20-5020')
  assert.equal(deviceIdFor(found('192.168.1.20', 502, 2), new Set()), 'modbus-192-168-1-20-u2')
  const taken = new Set(['modbus-192-168-1-20', 'modbus-192-168-1-20-2'])
  assert.equal(deviceIdFor(found('192.168.1.20'), taken), 'modbus-192-168-1-20-3')
})

test('the create request carries the registers that answered, by offset', () => {
  const p = newDevicePayload(found('192.168.1.20'), 'modbus-192-168-1-20', 'Modbus 192.168.1.20')
  assert.equal(p.ip, '192.168.1.20')
  assert.equal(p.interval_ms, 1000)
  assert.deepEqual(p.registers.map(r => [r.name, r.offset, r.register_type]), [
    ['reg_00000', 0, 'uint16'],
    ['reg_00003', 3, 'uint16'],
  ])
})
