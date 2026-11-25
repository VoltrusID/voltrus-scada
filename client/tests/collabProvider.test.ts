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

class FakeServer {
  doc = new Y.Doc()
  epoch = 'e1'
  conns = new Set<FakeSocket>()
  syncCount = new Map<FakeSocket, number>()
  awareness = new Map<FakeSocket, Uint8Array>()
  /** Errors to answer the next sync message with. */
  failNext: { code: string; reload: boolean } | null = null
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
  /** A restart: the document is rebuilt from the rows it last had. */
  restart() {
    const rows = saveLayoutToAPI(readDoc(this.doc), nameOf)
    this.doc = new Y.Doc()
    loadRows(this.doc, rows.equipment, rows.connections)
    this.listen()
    this.epoch = 'e2'
    for (const c of this.conns) c.joined = false
  }
  receive(c: FakeSocket, m: any) {
    if (m.type === 'DesignJoin') {
      c.joined = true
      c.deliver({ type: 'DesignJoined', screen: 's', epoch: this.epoch, resync: m.epoch != null && m.epoch !== this.epoch, user: c.user, color: '#f00', protocol: 1, limits: { max_update: 262144, max_rate: 50, max_doc: 5e6, max_awareness: 4096, flush_ms: 2000 } })
      const enc = encoding.createEncoder()
      syncProtocol.writeSyncStep1(enc, this.doc)
      c.deliver({ type: 'DesignSync', screen: 's', data: encoding.toUint8Array(enc) })
      for (const [o, a] of this.awareness) if (o !== c) c.deliver({ type: 'DesignAwareness', screen: 's', data: a })
    } else if (m.type === 'DesignSync') {
      this.syncCount.set(c, (this.syncCount.get(c) ?? 0) + 1)
      if (this.failNext) { const f = this.failNext; this.failNext = null; c.deliver({ type: 'DesignError', screen: 's', code: f.code, message: f.code, reload: f.reload }); return }
      const enc = encoding.createEncoder()
      syncProtocol.readSyncMessage(decoding.createDecoder(m.data), enc, this.doc, c)
      if (encoding.length(enc) > 0) c.deliver({ type: 'DesignSync', screen: 's', data: encoding.toUint8Array(enc) })
    } else if (m.type === 'DesignAwareness') {
      this.awareness.set(c, m.data)
      for (const o of this.conns) if (o !== c && o.joined) o.deliver({ type: 'DesignAwareness', screen: 's', data: m.data })
    } else if (m.type === 'DesignFlush') {
      c.deliver({ type: 'DesignFlushed', screen: 's', seq: 1, at_ms: 0, users: [c.user] })
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
