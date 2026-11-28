import test from 'node:test'
import assert from 'node:assert/strict'
import { EMPTY_FILTERS, MAX_FRAMES, fieldAt, formatLatency, hexBytes, mergeFrames, nextAfter, pathTo, trafficQuery, type TrafficField, type TrafficFrame } from '../src/traffic.ts'

const frame = (seq: number, tsUs: number, extra: Partial<TrafficFrame> = {}): TrafficFrame => ({
  seq, ts_us: tsUs, dir: 'tx', proto: 'modbus_tcp', summary: '', pair_id: null, latency_us: null, error: null, ...extra,
})

test('the query carries only valid filters', () => {
  assert.equal(trafficQuery(EMPTY_FILTERS), 'limit=500')
  assert.equal(
    trafficQuery({ function: '3', errors: true, space: 'holding', addrFrom: ' 100 ', addrTo: 'x' }, 41),
    'function=3&errors=true&space=holding&addr_from=100&after=41&limit=500',
  )
})

test('polling revisits the last seconds so pairs and timeouts arrive', () => {
  assert.equal(nextAfter([]), undefined)
  const frames = [frame(1, 0), frame(2, 1_000_000), frame(3, 9_000_000), frame(4, 9_500_000)]
  assert.equal(nextAfter(frames), 2, 'frames within 7 s of the newest are fetched again')
})

test('merging replaces frames by seq and stays bounded', () => {
  const old = [frame(1, 0), frame(2, 1)]
  const merged = mergeFrames(old, [frame(2, 1, { pair_id: 3 }), frame(3, 2, { dir: 'rx' })])
  assert.deepEqual(merged.map(f => f.seq), [1, 2, 3])
  assert.equal(merged[1].pair_id, 3)
  const many = Array.from({ length: MAX_FRAMES + 5 }, (_, i) => frame(i, i))
  assert.equal(mergeFrames([], many).length, MAX_FRAMES)
  assert.equal(mergeFrames([], many)[0].seq, 5)
})

test('a byte selects the deepest field that covers it', () => {
  const reg: TrafficField = { id: 'modbus.regval_uint16', label: 'Register Value (UINT16)', value: 1, display: '', offset: 9, len: 2 }
  const group: TrafficField = { id: '', label: '', value: null, display: 'Register 0', offset: 9, len: 2, children: [reg] }
  const fields: TrafficField[] = [
    { id: '', label: '', value: null, display: 'Modbus/TCP', offset: 0, len: 7 },
    { id: '', label: '', value: null, display: 'Modbus', offset: 7, len: 4, children: [group] },
  ]
  assert.equal(fieldAt(fields, 10), reg)
  assert.equal(fieldAt(fields, 2)?.display, 'Modbus/TCP')
  assert.equal(fieldAt(fields, 40), undefined)
  assert.deepEqual(pathTo(fields, reg)?.map(f => f.display), ['Modbus', 'Register 0', ''])
})

test('hex and latency formatting', () => {
  assert.deepEqual(hexBytes('00ff10'), [0, 255, 16])
  assert.deepEqual(hexBytes(undefined), [])
  assert.equal(formatLatency(null), '')
  assert.equal(formatLatency(850), '850 µs')
  assert.equal(formatLatency(2_250), '2.25 ms')
  assert.equal(formatLatency(48_400), '48 ms')
})
