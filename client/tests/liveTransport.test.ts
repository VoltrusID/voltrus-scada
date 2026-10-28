import { test, beforeEach, afterEach } from 'node:test'
import assert from 'node:assert/strict'
import { encode } from '@msgpack/msgpack'
import {
  VoltrusWebSocket,
  WS_FAILURES_BEFORE_FALLBACK,
  SSE_PROBE_INTERVAL_MS,
  type TagUpdate,
} from '../src/api/websocket.ts'

// A WebSocket and an EventSource the test drives by hand.
class FakeWebSocket {
  static instances: FakeWebSocket[] = []
  static OPEN = 1
  readyState = 0
  binaryType = ''
  sent: unknown[] = []
  onopen: (() => void) | null = null
  onmessage: ((e: { data: ArrayBuffer }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  url: string
  constructor(url: string) { this.url = url; FakeWebSocket.instances.push(this) }
  send(data: unknown) { this.sent.push(data) }
  close() { this.readyState = 3; this.onclose?.() }
  open() { this.readyState = 1; this.onopen?.() }
  hear(msg: Record<string, unknown>) {
    const bytes = encode(msg)
    this.onmessage?.({ data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer })
  }
  /** The upgrade is refused or the connection drops before the server says anything. */
  fail() { this.readyState = 3; this.onclose?.() }
}

class FakeEventSource {
  static instances: FakeEventSource[] = []
  static CLOSED = 2
  readyState = 0
  closed = false
  listeners = new Map<string, ((e: { data: string }) => void)[]>()
  onerror: (() => void) | null = null
  url: string
  init?: { withCredentials?: boolean }
  constructor(url: string, init?: { withCredentials?: boolean }) { this.url = url; this.init = init; FakeEventSource.instances.push(this) }
  addEventListener(name: string, fn: (e: { data: string }) => void) {
    this.listeners.set(name, [...(this.listeners.get(name) ?? []), fn])
  }
  close() { this.closed = true; this.readyState = 2 }
  emit(name: string, msg: Record<string, unknown>) {
    this.readyState = 1
    for (const fn of this.listeners.get(name) ?? []) fn({ data: JSON.stringify(msg) })
  }
}

// Manual timers, so a test moves time forward exactly.
type Timer = { at: number; fn: () => void; id: number }
let now = 0
let timers: Timer[] = []
let nextTimer = 1
const realSetTimeout = globalThis.setTimeout
const realClearTimeout = globalThis.clearTimeout

function advance(ms: number) {
  const end = now + ms
  for (;;) {
    const due = timers.filter(t => t.at <= end).sort((a, b) => a.at - b.at)[0]
    if (!due) break
    timers = timers.filter(t => t !== due)
    now = due.at
    due.fn()
  }
  now = end
}

const g = globalThis as Record<string, unknown>

beforeEach(() => {
  FakeWebSocket.instances = []
  FakeEventSource.instances = []
  now = 0
  timers = []
  g.WebSocket = FakeWebSocket
  g.EventSource = FakeEventSource
  g.setTimeout = ((fn: () => void, ms = 0) => {
    const id = nextTimer++
    timers.push({ at: now + ms, fn, id })
    return id
  }) as unknown
  g.clearTimeout = ((id: number) => { timers = timers.filter(t => t.id !== id) }) as unknown
})

afterEach(() => {
  g.setTimeout = realSetTimeout
  g.clearTimeout = realClearTimeout
})

const WS_URL = 'ws://host/api/v1/ws'
const SSE_URL = 'http://host/api/v1/stream'
const lastWs = () => FakeWebSocket.instances.at(-1)!
const lastEs = () => FakeEventSource.instances.at(-1)!

/** Fail the WebSocket attempts until the client gives up on it. */
function failUntilFallback() {
  for (let i = 0; i < WS_FAILURES_BEFORE_FALLBACK; i++) {
    if (i > 0) advance(60_000)
    lastWs().fail()
  }
}

test('a WebSocket that connects is used and never touches SSE', () => {
  const c = new VoltrusWebSocket()
  const batches: TagUpdate[][] = []
  c.onTagBatch(b => batches.push(b))
  c.connect(WS_URL, undefined, SSE_URL)
  lastWs().open()
  lastWs().hear({ type: 'AuthOk', user: 'admin', permissions: ['read'] })
  lastWs().hear({ type: 'TagUpdateBatch', updates: [{ tag_id: '1', value: 2, quality: 'good', ts: 1, simulated: false }] })

  assert.equal(c.getTransport(), 'ws')
  assert.equal(c.isConnected(), true)
  assert.equal(c.canWrite(), true)
  assert.equal(FakeEventSource.instances.length, 0)
  assert.equal(batches.length, 1)
  assert.equal(batches[0][0].value, 2)
})

test('after repeated WebSocket failures the client falls back to SSE and reads the same updates', () => {
  const c = new VoltrusWebSocket()
  const transports: string[] = []
  const batches: TagUpdate[][] = []
  const acks: string[] = []
  let connects = 0
  c.onTransportChange(t => transports.push(t))
  c.onTagBatch(b => batches.push(b))
  c.onCommandAck(a => acks.push(`${a.command_id}:${a.status}`))
  c.onConnect(() => { connects += 1 })
  c.connect(WS_URL, undefined, SSE_URL)

  // The first failures only retry the WebSocket.
  for (let i = 0; i < WS_FAILURES_BEFORE_FALLBACK - 1; i++) {
    lastWs().fail()
    assert.equal(FakeEventSource.instances.length, 0, `still trying the WebSocket after ${i + 1}`)
    advance(60_000)
  }
  lastWs().fail()

  assert.equal(c.getTransport(), 'sse')
  assert.deepEqual(transports, ['sse'])
  assert.equal(lastEs().url, SSE_URL)
  assert.equal(lastEs().init?.withCredentials, true, 'the session cookie goes with the stream')
  assert.equal(c.isConnected(), false, 'not live until the server greets')

  lastEs().emit('AuthOk', { type: 'AuthOk', user: 'admin', permissions: ['read'] })
  assert.equal(c.isConnected(), true)
  assert.equal(connects, 1)
  assert.equal(c.canWrite(), false, 'the stream is read-only')

  lastEs().emit('TagUpdateBatch', { type: 'TagUpdateBatch', updates: [{ tag_id: '7', value: 21.5, quality: 'good', ts: 5, simulated: false }] })
  lastEs().emit('CommandAck', { type: 'CommandAck', command_id: 'c1', status: 'acked' })
  assert.equal(batches[0][0].tag_id, '7')
  assert.equal(batches[0][0].value, 21.5)
  assert.deepEqual(acks, ['c1:acked'])
})

test('a WebSocket that opens but is closed before the server speaks counts as a failure', () => {
  const c = new VoltrusWebSocket()
  c.connect(WS_URL, undefined, SSE_URL)
  for (let i = 0; i < WS_FAILURES_BEFORE_FALLBACK; i++) {
    lastWs().open()
    lastWs().fail()
    advance(60_000)
  }
  assert.equal(c.getTransport(), 'sse')
})

test('a WebSocket that spoke before dropping resets the failure count', () => {
  const c = new VoltrusWebSocket()
  c.connect(WS_URL, undefined, SSE_URL)
  for (let round = 0; round < 3; round++) {
    lastWs().fail()
    advance(60_000)
    lastWs().fail()
    advance(60_000)
    lastWs().open()
    lastWs().hear({ type: 'AuthOk', user: 'a', permissions: [] })
    lastWs().fail() // dropped after working
    advance(60_000)
  }
  assert.equal(c.getTransport(), 'ws', 'two failures per round never reach the limit')
  assert.equal(FakeEventSource.instances.length, 0)
})

test('without an SSE url the client only retries the WebSocket', () => {
  const c = new VoltrusWebSocket()
  c.connect(WS_URL)
  for (let i = 0; i < WS_FAILURES_BEFORE_FALLBACK + 2; i++) {
    lastWs().fail()
    advance(60_000)
  }
  assert.equal(c.getTransport(), 'ws')
  assert.equal(FakeEventSource.instances.length, 0)
})

test('while on SSE the WebSocket is tried again and wins once the server answers', () => {
  const c = new VoltrusWebSocket()
  const transports: string[] = []
  let batches = 0
  c.onTransportChange(t => transports.push(t))
  c.onTagBatch(() => { batches += 1 })
  c.connect(WS_URL, undefined, SSE_URL)
  failUntilFallback()
  const es = lastEs()
  es.emit('AuthOk', { type: 'AuthOk', user: 'admin', permissions: ['read'] })
  const wsCount = FakeWebSocket.instances.length

  // A probe that fails leaves SSE running and is retried a minute later.
  advance(SSE_PROBE_INTERVAL_MS)
  assert.equal(FakeWebSocket.instances.length, wsCount + 1)
  lastWs().fail()
  assert.equal(c.getTransport(), 'sse')
  assert.equal(es.closed, false)
  es.emit('TagUpdateBatch', { type: 'TagUpdateBatch', updates: [{ tag_id: '1', value: 1, quality: 'good', ts: 1, simulated: false }] })
  assert.equal(batches, 1, 'SSE still delivers while probing')

  advance(SSE_PROBE_INTERVAL_MS)
  assert.equal(FakeWebSocket.instances.length, wsCount + 2)
  const probe = lastWs()
  probe.open()
  assert.equal(c.getTransport(), 'sse', 'open is not enough, the server must speak')
  assert.equal(es.closed, false)

  probe.hear({ type: 'AuthOk', user: 'admin', permissions: ['read'] })
  assert.equal(c.getTransport(), 'ws')
  assert.deepEqual(transports, ['sse', 'ws'])
  assert.equal(es.closed, true, 'the SSE stream is closed once the WebSocket carries the data')
  assert.equal(c.canWrite(), true)

  probe.hear({ type: 'TagUpdateBatch', updates: [{ tag_id: '1', value: 2, quality: 'good', ts: 2, simulated: false }] })
  assert.equal(batches, 2)

  // No further probing once back on the WebSocket.
  const total = FakeWebSocket.instances.length
  advance(SSE_PROBE_INTERVAL_MS * 3)
  assert.equal(FakeWebSocket.instances.length, total)
})

test('a refused SSE stream is retried, and disconnect stops everything', () => {
  const c = new VoltrusWebSocket()
  c.connect(WS_URL, undefined, SSE_URL)
  failUntilFallback()
  const first = lastEs()
  first.readyState = FakeEventSource.CLOSED // a 401 or a proxy answering with HTML
  first.onerror?.()
  assert.equal(FakeEventSource.instances.length, 1)
  advance(1000)
  assert.equal(FakeEventSource.instances.length, 2, 'a new stream was opened')
  assert.equal(first.closed, true)

  const second = lastEs()
  c.disconnect()
  assert.equal(second.closed, true)
  const ws = FakeWebSocket.instances.length
  advance(10 * SSE_PROBE_INTERVAL_MS)
  assert.equal(FakeWebSocket.instances.length, ws, 'no probe after disconnect')
  assert.equal(FakeEventSource.instances.length, 2)
})

test('registering the same handler twice delivers each update once', () => {
  const c = new VoltrusWebSocket()
  let n = 0
  const handler = () => { n += 1 }
  c.onTagBatch(handler)
  c.onTagBatch(handler)
  c.connect(WS_URL)
  lastWs().open()
  lastWs().hear({ type: 'TagUpdateBatch', updates: [{ tag_id: '1', value: 1, quality: 'good', ts: 1, simulated: false }] })
  assert.equal(n, 1)
})
