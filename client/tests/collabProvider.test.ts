// The collaboration provider against an in-process stand-in of the server:
// a Yjs document per screen, y-sync answers, updates forwarded to the others,
// awareness relayed, and a restart that rebuilds the document from rows.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import * as Y from 'yjs'
import * as encoding from 'lib0/encoding'
import * as decoding from 'lib0/decoding'
import * as syncProtocol from 'y-protocols/sync'
import { CollabProvider, FIRST_JOIN_MS, LOCAL, type DesignSocket } from '../src/designer/collab/provider.ts'
import { loadRows, readDoc, writeDiff } from '../src/designer/collab/ydoc.ts'
import { applyEdit, type Edit } from '../src/designer/model.ts'
import { saveLayoutToAPI } from '../src/scada2/engine/layout.ts'

const sleep = (ms: number) => new Promise(r => setTimeout(r, ms))
const nameOf = () => 'W'

const LIMITS = { max_update: 262144, max_rate: 50, max_doc: 5e6, max_awareness: 4096, flush_ms: 2000, idle_ms: 120000 }

class FakeServer {
  doc = new Y.Doc()
  epoch = 'e1'
  conns = new Set<FakeSocket>()
  syncCount = new Map<FakeSocket, number>()
  /** Sync messages taken from each connection since its latest join (DesignFlushed `acked`). */
  taken = new Map<FakeSocket, number>()
  awareness = new Map<FakeSocket, Uint8Array>()
  limits = { ...LIMITS }
  /** Times and sizes of the sync messages each connection sent. */
  log = new Map<FakeSocket, { at: number; size: number }[]>()
  /** Times of every message each connection sent. */
  heard = new Map<FakeSocket, number[]>()
  /** Errors to answer the next sync message with. */
  failNext: { code: string; reload: boolean } | null = null
  /** Rows as of the last flush (what a restart comes back with). */
  flushedRows: ReturnType<typeof saveLayoutToAPI> | null = null
  constructor() {
    loadRows(this.doc, [{ equipment_id: 'a', equipment_type: 'big_number', equipment_name: 'A', x: 0, y: 0, width: 10, height: 10, rotation: 0, sensor_bindings: '[]', config: '{"z":0}' } as any], [])
    this.listen()
  }
  private listen() {
    this.doc.on('update', (u: Uint8Array, origin: unknown) => {
      const enc = encoding.createEncoder()
      syncProtocol.writeUpdate(enc, u)
      const data = encoding.toUint8Array(enc)
      for (const c of this.conns) if (c !== origin && c.joined) c.deliver({ type: 'DesignSync', screen: 's', data })
    })
  }
  /** A restart: the document is rebuilt from the rows it last had (or last wrote). */
  restart(fromFlushed = false) {
    const rows = fromFlushed && this.flushedRows ? this.flushedRows : saveLayoutToAPI(readDoc(this.doc), nameOf)
    this.doc = new Y.Doc()
    loadRows(this.doc, rows.equipment, rows.connections)
    this.listen()
    this.epoch = 'e2'
    for (const c of this.conns) c.joined = false
  }
  /** Drop a member, telling it with `code` (or not at all). */
  expel(c: FakeSocket, code?: string) {
    c.joined = false
    if (code) c.deliver({ type: 'DesignError', screen: 's', code, message: code, reload: false })
  }
  /** Write the draft and tell `c` how many of its messages that covers. */
  flushed(c: FakeSocket, acked = this.taken.get(c) ?? 0) {
    this.flushedRows = saveLayoutToAPI(readDoc(this.doc), nameOf)
    c.deliver({ type: 'DesignFlushed', screen: 's', seq: 1, at_ms: 0, users: [c.user], acked })
  }
  receive(c: FakeSocket, m: any) {
    const t = Date.now()
    this.heard.set(c, [...(this.heard.get(c) ?? []), t])
    if (m.type === 'DesignJoin') {
      c.joined = true
      this.taken.set(c, 0)
      c.deliver({ type: 'DesignJoined', screen: 's', epoch: this.epoch, resync: m.epoch != null && m.epoch !== this.epoch, user: c.user, color: '#f00', protocol: 1, limits: this.limits })
      const enc = encoding.createEncoder()
      syncProtocol.writeSyncStep1(enc, this.doc)
      c.deliver({ type: 'DesignSync', screen: 's', data: encoding.toUint8Array(enc) })
      for (const [o, a] of this.awareness) if (o !== c) c.deliver({ type: 'DesignAwareness', screen: 's', data: a })
    } else if (m.type === 'DesignSync') {
      if (!c.joined) { c.deliver({ type: 'DesignError', screen: 's', code: 'not_joined', message: 'join first', reload: false }); return }
      this.syncCount.set(c, (this.syncCount.get(c) ?? 0) + 1)
      const log = [...(this.log.get(c) ?? []), { at: t, size: m.data.length }]
      this.log.set(c, log)
      if (log.filter(e => e.at > t - 1000).length > this.limits.max_rate) { this.expel(c, 'rate_limited'); return }
      if (m.data.length > this.limits.max_update) { c.joined = false; c.deliver({ type: 'DesignError', screen: 's', code: 'too_large', message: 'too large', reload: true }); return }
      if (this.failNext) { const f = this.failNext; this.failNext = null; c.joined = !f.reload && c.joined; c.deliver({ type: 'DesignError', screen: 's', code: f.code, message: f.code, reload: f.reload }); return }
      this.taken.set(c, (this.taken.get(c) ?? 0) + 1)
      const enc = encoding.createEncoder()
      syncProtocol.readSyncMessage(decoding.createDecoder(m.data), enc, this.doc, c)
      if (encoding.length(enc) > 0) c.deliver({ type: 'DesignSync', screen: 's', data: encoding.toUint8Array(enc) })
    } else if (m.type === 'DesignAwareness') {
      this.awareness.set(c, m.data)
      for (const o of this.conns) if (o !== c && o.joined) o.deliver({ type: 'DesignAwareness', screen: 's', data: m.data })
    } else if (m.type === 'DesignFlush') {
      this.flushed(c)
    }
  }
}

class FakeSocket implements DesignSocket {
  up = true
  joined = false
  design = new Set<(m: any) => void>()
  connect = new Set<() => void>()
  disconnect = new Set<() => void>()
  sent: any[] = []
  server: FakeServer
  user: string
  constructor(server: FakeServer, user: string) { this.server = server; this.user = user; server.conns.add(this) }
  sendDesign(m: any) { if (!this.up) return false; this.sent.push(m); queueMicrotask(() => { if (this.up) this.server.receive(this, m) }); return true }
  deliver(m: any) { queueMicrotask(() => { if (this.up) this.design.forEach(cb => cb(m)) }) }
  canWrite() { return this.up }
  onDesign(cb: any) { this.design.add(cb) }
  offDesign(cb: any) { this.design.delete(cb) }
  onConnect(cb: any) { this.connect.add(cb) }
  offConnect(cb: any) { this.connect.delete(cb) }
  onDisconnect(cb: any) { this.disconnect.add(cb) }
  offDisconnect(cb: any) { this.disconnect.delete(cb) }
  drop() { this.up = false; this.joined = false; this.disconnect.forEach(cb => cb()) }
  back() { this.up = true; this.connect.forEach(cb => cb()) }
  joins() { return this.sent.filter(m => m.type === 'DesignJoin').length }
}

const edit = (p: CollabProvider, e: Edit) => { const cur = readDoc(p.doc); writeDiff(p.doc, cur, applyEdit(cur, e), nameOf, LOCAL) }
const xOf = (d: Y.Doc, id: string) => readDoc(d).widgets.find(w => w.id === id)?.x
const settle = () => sleep(120)

function setup() {
  const server = new FakeServer()
  const sa = new FakeSocket(server, 'ana'), sb = new FakeSocket(server, 'ben')
  const a = new CollabProvider(sa, 's'), b = new CollabProvider(sb, 's')
  return { server, sa, sb, a, b, stop: () => { a.stop(); b.stop() } }
}

test('two clients join, get the server document and see each other\'s edits', async () => {
  const { server, a, b, stop } = setup()
  a.start(); b.start()
  await settle()
  assert.equal(a.status, 'live'); assert.ok(a.synced && b.synced)
  assert.equal(xOf(a.doc, 'a'), 0)
  assert.equal(a.user, 'ana')
  edit(a, { type: 'patch', patches: { a: { x: 50 } } })
  edit(b, { type: 'patch', patches: { a: { y: 70 } } })
  await settle()
  for (const d of [a.doc, b.doc, server.doc]) {
    const w = readDoc(d).widgets[0]
    assert.deepEqual([w.x, w.y], [50, 70])
  }
  stop()
})

test('local updates are batched: a drag of many moves is a few messages', async () => {
  const { server, sa, a, b, stop } = setup()
  a.start(); b.start()
  await settle()
  const before = server.syncCount.get(sa) ?? 0
  for (let i = 1; i <= 30; i++) edit(a, { type: 'patch', patches: { a: { x: i } } })
  await settle()
  assert.ok((server.syncCount.get(sa) ?? 0) - before <= 2, 'merged into one or two updates')
  assert.equal(xOf(b.doc, 'a'), 30)
  assert.equal(a.pending, true, 'sent but not written to the draft yet')
  a.flushNow()
  await settle()
  assert.equal(a.pending, false)
  stop()
})

test('edits made offline merge on reconnect within the same session', async () => {
  const { sa, a, b, stop } = setup()
  a.start(); b.start()
  await settle()
  sa.drop()
  assert.equal(a.status, 'offline')
  edit(a, { type: 'patch', patches: { a: { x: 99 } } })
  edit(b, { type: 'patch', patches: { a: { y: 5 } } })
  await settle()
  assert.equal(a.pending, true)
  assert.equal(xOf(b.doc, 'a'), 0, 'not seen while offline')
  sa.back()
  await settle()
  assert.equal(a.status, 'live')
  for (const d of [a.doc, b.doc]) assert.deepEqual([readDoc(d).widgets[0].x, readDoc(d).widgets[0].y], [99, 5])
  stop()
})

test('after a server restart (new epoch) the objects changed offline are written again', async () => {
  const { server, sa, sb, a, b, stop } = setup()
  a.start(); b.start()
  await settle()
  sa.drop(); sb.drop()
  server.restart()
  const nw = { id: 'n', type: 'big_number' as const, sensorId: null, sensorIds: [], x: 1, y: 1, width: 5, height: 5, config: {} }
  edit(a, { type: 'add', widgets: [nw] })
  edit(a, { type: 'patch', patches: { a: { x: 7 } } })
  const oldDoc = a.doc
  sa.back(); sb.back()
  await settle()
  assert.notEqual(a.doc, oldDoc, 'a fresh document from the server')
  assert.equal(a.epoch, 'e2')
  for (const d of [a.doc, b.doc, server.doc]) {
    assert.deepEqual(readDoc(d).widgets.map(w => w.id), ['a', 'n'])
    assert.equal(xOf(d, 'a'), 7)
  }
  stop()
})

test('a DesignError with reload starts over from the server; disabled falls back', async () => {
  const { server, a, stop } = setup()
  a.start()
  await settle()
  const errors: string[] = []
  a.on('error', e => errors.push(e.code))
  server.failNext = { code: 'too_large', reload: true }
  edit(a, { type: 'patch', patches: { a: { x: 1234 } } })
  await settle()
  assert.deepEqual(errors, ['too_large'])
  assert.equal(a.status, 'live')
  assert.equal(xOf(a.doc, 'a'), 0, 'the refused edit is gone; the server state is back')
  const falls: string[] = []
  a.on('fallback', e => falls.push(e.code))
  server.failNext = { code: 'disabled', reload: false }
  edit(a, { type: 'patch', patches: { a: { x: 2 } } })
  await settle()
  assert.deepEqual(falls, ['disabled'])
  assert.equal(a.status, 'off')
  stop()
})

test('presence: names come from the server, cursors and selections reach the others', async () => {
  const { a, b, stop } = setup()
  a.start(); b.start()
  await settle()
  a.setPresence({ cursor: { x: 10, y: 20 }, selection: ['a'] })
  await settle()
  const peers = b.peers()
  assert.equal(peers.length, 1)
  assert.equal(peers[0].user, 'ana')
  assert.deepEqual(peers[0].cursor, { x: 10, y: 20 })
  assert.deepEqual(peers[0].selection, ['a'])
  stop()
})

test('the send rate stays under the server limit', async () => {
  const { server, sa, a, stop } = setup()
  a.start()
  await settle()
  const before = server.syncCount.get(sa) ?? 0
  const end = Date.now() + 1000
  let i = 0
  while (Date.now() < end) { edit(a, { type: 'patch', patches: { a: { x: ++i } } }); await sleep(5) }
  await settle()
  assert.ok((server.syncCount.get(sa) ?? 0) - before <= 40, `${(server.syncCount.get(sa) ?? 0) - before} messages in a second`)
  assert.equal(xOf(server.doc, 'a'), i)
  stop()
})

test('a server that never answers the first join: fall back to saving layouts', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const silent: DesignSocket = {
    sendDesign: () => true, canWrite: () => true,
    onDesign() {}, offDesign() {}, onConnect() {}, offConnect() {}, onDisconnect() {}, offDisconnect() {},
  }
  const p = new CollabProvider(silent, 's')
  const falls: string[] = []
  p.on('fallback', e => falls.push(e.code))
  p.start()
  assert.equal(p.status, 'connecting')
  t.mock.timers.tick(FIRST_JOIN_MS + 1)
  assert.deepEqual(falls, ['unavailable'])
  assert.equal(p.status, 'off')
  p.stop()
})

// ── Membership, pacing, acknowledgements, replay ──

/** Mocked clock: advance in steps, letting the fake sockets' microtasks run. */
function clock(t: { mock: { timers: { enable: (o: any) => void; tick: (ms: number) => void } } }) {
  t.mock.timers.enable({ apis: ['setTimeout', 'Date'] })
  const drain = () => new Promise<void>(r => setImmediate(r))
  return async (ms: number) => {
    await drain()
    for (let left = ms; left > 0; left -= 10) { t.mock.timers.tick(Math.min(10, left)); await drain() }
  }
}

for (const code of ['resync', 'not_joined', 'idle']) {
  test(`dropped from the session (${code}): joins again with its epoch and keeps its document`, async () => {
    const { server, sa, a, b, stop } = setup()
    a.start(); b.start()
    await settle()
    const rejoins: string[] = []
    a.on('rejoin', e => rejoins.push(e.code))
    const doc = a.doc
    // not_joined: the server dropped it without a word; the next message finds out.
    server.expel(sa, code === 'not_joined' ? undefined : code)
    await settle()
    edit(a, { type: 'patch', patches: { a: { x: 11 } } })
    await settle()
    assert.deepEqual(rejoins, [code])
    assert.equal(a.status, 'live')
    assert.equal(a.doc, doc, 'the same document')
    assert.equal(sa.sent.filter(m => m.type === 'DesignJoin').at(-1).epoch, 'e1')
    for (const d of [server.doc, b.doc]) assert.equal(xOf(d, 'a'), 11)
    // The edit rode the step 2 reply of the new join: pending until a flush covers it.
    assert.equal(a.pending, true)
    a.flushNow()
    await settle()
    assert.equal(a.pending, false)
    stop()
  })
}

test('rate_limited: waits before joining again, longer each time, and keeps the edits', async (t) => {
  const tick = clock(t)
  const { server, sa, a, stop } = setup()
  a.start()
  await tick(100)
  assert.equal(a.status, 'live')
  const rejoins: string[] = []
  a.on('rejoin', e => rejoins.push(e.code))
  server.expel(sa, 'rate_limited')
  await tick(20)
  assert.equal(a.status, 'connecting')
  edit(a, { type: 'patch', patches: { a: { x: 5 } } })
  await tick(500)
  assert.equal(sa.joins(), 1, 'no join before the wait is over')
  await tick(600)
  assert.equal(sa.joins(), 2)
  assert.equal(a.status, 'live')
  assert.equal(xOf(server.doc, 'a'), 5)
  // Again soon after: twice the wait.
  server.expel(sa, 'rate_limited')
  // Messages sent before the new join are refused as not_joined: ignored meanwhile.
  edit(a, { type: 'patch', patches: { a: { x: 6 } } })
  await tick(1500)
  assert.equal(sa.joins(), 2)
  await tick(600)
  assert.equal(sa.joins(), 3)
  assert.deepEqual(rejoins, ['rate_limited', 'rate_limited'])
  assert.equal(xOf(server.doc, 'a'), 6)
  stop()
})

const bigWidget = (id: string, i: number) => ({ id, type: 'big_number' as const, sensorId: null, sensorIds: [], x: i, y: i, width: 5, height: 5, config: { note: `${id}:${'n'.repeat(400)}` } })

test('pacing: never more than max_rate - 10 sync messages a second; an oversize merge goes out in parts that converge', async (t) => {
  const tick = clock(t)
  const { server, sa, a, b, stop } = setup()
  server.limits = { ...LIMITS, max_update: 2048, max_rate: 15 }
  const errors: string[] = []
  a.on('error', e => errors.push(e.code))
  a.start(); b.start()
  await tick(100)
  edit(a, { type: 'add', widgets: Array.from({ length: 40 }, (_, i) => bigWidget(`w${i}`, i)) })
  for (let i = 0; i < 20; i++) { edit(a, { type: 'patch', patches: { a: { x: i } } }); await tick(30) }
  await tick(6000)
  assert.deepEqual(errors, [])
  assert.equal(sa.joins(), 1, 'never dropped')
  const log = server.log.get(sa)!
  assert.ok(log.length > 10, `${log.length} messages`)
  for (const e of log) {
    assert.ok(e.size <= 2048, `a message of ${e.size} bytes`)
    const inWindow = log.filter(o => o.at > e.at - 1000 && o.at <= e.at).length
    assert.ok(inWindow <= 5, `${inWindow} messages within a second`)
  }
  assert.equal(a.pending, true)
  for (const d of [server.doc, b.doc]) assert.deepEqual(readDoc(d), readDoc(a.doc))
  assert.equal(readDoc(server.doc).widgets.length, 41)
  stop()
})

test('one change over the size limit is not sent: the user is told and the screen reloads', async () => {
  const { server, sa, a, stop } = setup()
  server.limits = { ...LIMITS, max_update: 1024 }
  a.start()
  await settle()
  const errors: string[] = []
  a.on('error', e => errors.push(e.code))
  const before = server.log.get(sa)!.length
  edit(a, { type: 'patch', patches: { a: { config: { note: 'x'.repeat(5000) } } } })
  await settle()
  assert.deepEqual(errors, ['too_large'])
  assert.ok(server.log.get(sa)!.slice(before).every(e => e.size <= 1024), 'nothing over the limit sent')
  assert.equal(a.status, 'live')
  assert.equal(readDoc(a.doc).widgets[0].config.note, undefined)
  assert.equal(a.pending, false)
  stop()
})

test('acked counts per message: a flush that crosses newer updates leaves them pending', async () => {
  const { server, sa, a, stop } = setup()
  a.start()
  await settle()
  edit(a, { type: 'patch', patches: { a: { x: 1 } } })
  await settle()
  const first = server.taken.get(sa)!
  edit(a, { type: 'patch', patches: { a: { x: 2 } } })
  await settle()
  assert.ok(server.taken.get(sa)! > first)
  // Written before the second update arrived.
  server.flushed(sa, first)
  await settle()
  assert.equal(a.pending, true, 'the second edit is not written yet')
  server.flushed(sa)
  await settle()
  assert.equal(a.pending, false)
  stop()
})

test('a flush without acked (older server) clears what was sent', async () => {
  const { sa, a, stop } = setup()
  a.start()
  await settle()
  edit(a, { type: 'patch', patches: { a: { x: 1 } } })
  await settle()
  sa.deliver({ type: 'DesignFlushed', screen: 's', seq: 2, at_ms: 0, users: ['ana'] })
  await settle()
  assert.equal(a.pending, false)
  stop()
})

test('sent but not written edits are applied again after a server restart', async () => {
  const { server, sa, sb, a, b, stop } = setup()
  a.start(); b.start()
  await settle()
  server.flushed(sa)
  await settle()
  edit(a, { type: 'patch', patches: { a: { x: 42 } } })
  await settle()
  assert.equal(xOf(server.doc, 'a'), 42, 'sent')
  assert.equal(a.pending, true, 'not written')
  const replayed: number[] = []
  a.on('replayed', e => replayed.push(e.objects))
  sa.drop(); sb.drop()
  server.restart(true)
  assert.equal(xOf(server.doc, 'a'), 0, 'lost with the restart')
  sb.back(); sa.back()
  await settle()
  assert.equal(a.epoch, 'e2')
  assert.deepEqual(replayed, [1])
  for (const d of [a.doc, b.doc, server.doc]) assert.equal(xOf(d, 'a'), 42)
  stop()
})

test('offline edits after a restart: an object someone else deleted stays deleted', async () => {
  const { server, sa, sb, a, b, stop } = setup()
  a.start(); b.start()
  await settle()
  sa.drop(); sb.drop()
  edit(a, { type: 'patch', patches: { a: { x: 5 } } })
  server.restart()
  sb.back()
  await settle()
  edit(b, { type: 'remove', ids: ['a'] })
  await settle()
  sa.back()
  await settle()
  for (const d of [a.doc, b.doc, server.doc]) assert.deepEqual(readDoc(d).widgets.map(w => w.id), [])
  stop()
})

test('offline edits after a restart: another field of the same widget edited meanwhile survives', async () => {
  const { server, sa, sb, a, b, stop } = setup()
  a.start(); b.start()
  await settle()
  sa.drop(); sb.drop()
  edit(a, { type: 'patch', patches: { a: { x: 5, config: { label: 'mine' } } } })
  server.restart()
  sb.back()
  await settle()
  edit(b, { type: 'patch', patches: { a: { y: 77, config: { unit: 'bar' } } } })
  await settle()
  sa.back()
  await settle()
  for (const d of [a.doc, b.doc, server.doc]) {
    const w = readDoc(d).widgets[0]
    assert.deepEqual([w.x, w.y, w.config.label, w.config.unit], [5, 77, 'mine', 'bar'])
  }
  stop()
})

test('a quiet member renews its presence well within idle_ms', async (t) => {
  const tick = clock(t)
  const { server, sa, a, stop } = setup()
  server.limits = { ...LIMITS, idle_ms: 3000 }
  a.start()
  await tick(100)
  const joinedAt = Date.now()
  await tick(12_000)
  const times = [joinedAt, ...server.heard.get(sa)!.filter(x => x >= joinedAt), Date.now()]
  const gaps = times.slice(1).map((x, i) => x - times[i])
  assert.ok(Math.max(...gaps) <= 1000, `silent for ${Math.max(...gaps)} ms`)
  assert.equal(sa.joins(), 1)
  stop()
})
