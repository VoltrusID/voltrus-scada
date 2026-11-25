// The Designer's live-collaboration provider (docs/collaboration.md): a
// screen's Y.Doc kept in sync with the server over the app's own WebSocket.
// No y-websocket server: y-sync v1 messages and awareness updates ride
// DesignSync / DesignAwareness frames.

import * as Y from 'yjs'
import * as encoding from 'lib0/encoding'
import * as decoding from 'lib0/decoding'
import * as syncProtocol from 'y-protocols/sync'
import * as awarenessProtocol from 'y-protocols/awareness'
import type { DesignClientMessage, DesignLimits, DesignServerMessage, DesignSessionInfo } from '../../api/websocket'
import { connectionsOf, touchedKeys, widgetsOf } from './ydoc.ts'

export const PROTOCOL = 1
/** Origin of edits made here: the undo manager tracks it. */
export const LOCAL = 'local'
/** Origin of edits that came from the server. */
export const REMOTE = 'remote'
const RESET = 'reset'
/** Local updates are merged and sent at most this often. */
export const SEND_EVERY_MS = 40
/** A first join that has not completed by then falls back to saving whole layouts. */
export const FIRST_JOIN_MS = 6000
/** Cursor and viewport changes reach the others at most this often. */
export const AWARENESS_EVERY_MS = 50

export const DEFAULT_LIMITS: DesignLimits = { max_update: 256 * 1024, max_rate: 50, max_doc: 5 * 1024 * 1024, max_awareness: 4096, flush_ms: 2000 }

/** What the provider needs of the WebSocket (VoltrusWebSocket has it all). */
export interface DesignSocket {
  sendDesign(msg: DesignClientMessage): boolean
  canWrite(): boolean
  onDesign(cb: (m: DesignServerMessage) => void): void
  offDesign(cb: (m: DesignServerMessage) => void): void
  onConnect(cb: () => void): void
  offConnect(cb: () => void): void
  onDisconnect(cb: () => void): void
  offDisconnect(cb: () => void): void
}

export type CollabStatus = 'off' | 'connecting' | 'live' | 'offline'

export interface Peer {
  clientId: number
  user: string
  color: string
  cursor?: { x: number; y: number } | null
  selection?: string[]
  viewport?: { cx: number; cy: number; scale: number } | null
}

export interface ProviderEvents {
  status: CollabStatus
  /** The document object was replaced (a resync or a reload): rebind to it. */
  doc: Y.Doc
  synced: boolean
  peers: Peer[]
  flushed: Extract<DesignServerMessage, { type: 'DesignFlushed' }>
  rejected: Extract<DesignServerMessage, { type: 'DesignRejected' }>
  sessions: DesignSessionInfo[]
  comment: Extract<DesignServerMessage, { type: 'DesignComment' }>
  /** Collaboration is not available: `disabled`, `forbidden` or `protocol`. */
  fallback: { code: string; message: string }
  error: { code: string; message: string }
  pending: boolean
}

type Listener<K extends keyof ProviderEvents> = (v: ProviderEvents[K]) => void

interface Snapshot { kind: 'widget' | 'connection'; id: string; json: Record<string, any> | null }

function snapshotMap(m: Y.Map<any>): Record<string, any> {
  const out: Record<string, any> = {}
  m.forEach((v, k) => { out[k] = v instanceof Y.AbstractType ? { __ymap: v instanceof Y.Map, value: v.toJSON() } : v })
  return out
}

function restoreMap(json: Record<string, any>): Y.Map<any> {
  const m = new Y.Map<any>()
  for (const [k, v] of Object.entries(json)) {
    if (v && typeof v === 'object' && '__ymap' in v) {
      const nested = new Y.Map<any>()
      for (const [nk, nv] of Object.entries(v.value as Record<string, any>)) {
        if (nk === 'bindings' && nv && typeof nv === 'object' && !Array.isArray(nv)) {
          const b = new Y.Map<any>()
          for (const [p, bv] of Object.entries(nv)) b.set(p, bv)
          nested.set(nk, b)
        } else nested.set(nk, nv)
      }
      m.set(k, nested)
    } else m.set(k, v)
  }
  return m
}

export class CollabProvider {
  doc: Y.Doc
  awareness: awarenessProtocol.Awareness
  status: CollabStatus = 'off'
  synced = false
  epoch: string | null = null
  user = ''
  color = '#888'
  limits: DesignLimits = DEFAULT_LIMITS
  /** Local updates not sent yet. */
  private queue: Uint8Array[] = []
  private sendTimer: ReturnType<typeof setTimeout> | null = null
  private awarenessTimer: ReturnType<typeof setTimeout> | null = null
  private firstJoinTimer: ReturnType<typeof setTimeout> | null = null
  /** Whether a join ever completed: afterwards a lost socket means offline, not unavailable. */
  private everSynced = false
  private awarenessDirty = false
  private sent: number[] = []
  /** Sent edits the server has not written to the draft yet. */
  unflushed = false
  /** Objects changed while not live (kept over a resync). */
  private offlineTouched = { widgets: new Set<string>(), connections: new Set<string>() }
  private listeners = new Map<keyof ProviderEvents, Set<(v: any) => void>>()
  private stopped = false
  private replay: Snapshot[] | null = null

  private socket: DesignSocket
  readonly screen: string
  private now: () => number

  constructor(socket: DesignSocket, screen: string, now: () => number = () => Date.now()) {
    this.socket = socket
    this.screen = screen
    this.now = now
    this.doc = new Y.Doc()
    this.awareness = new awarenessProtocol.Awareness(this.doc)
    this.bindDoc()
    socket.onDesign(this.onMessage)
    socket.onConnect(this.onConnect)
    socket.onDisconnect(this.onDisconnect)
  }

  on<K extends keyof ProviderEvents>(k: K, cb: Listener<K>): () => void {
    let s = this.listeners.get(k)
    if (!s) { s = new Set(); this.listeners.set(k, s) }
    s.add(cb)
    return () => { s!.delete(cb) }
  }

  private emit<K extends keyof ProviderEvents>(k: K, v: ProviderEvents[K]) {
    this.listeners.get(k)?.forEach(cb => cb(v))
  }

  private setStatus(s: CollabStatus) {
    if (this.status === s) return
    this.status = s
    this.emit('status', s)
  }

  /** Whether local edits wait to be sent or to be written to the draft. */
  get pending(): boolean {
    return this.queue.length > 0 || this.unflushed || (this.status !== 'live' && (this.offlineTouched.widgets.size + this.offlineTouched.connections.size) > 0)
  }

  start(): void {
    this.stopped = false
    // A server that never answers (an older one, or only the read-only event
    // stream gets through a proxy): the Designer saves whole layouts instead.
    this.firstJoinTimer = setTimeout(() => {
      this.firstJoinTimer = null
      if (!this.stopped && !this.everSynced) this.fallBack('unavailable', 'the server did not answer in time')
    }, FIRST_JOIN_MS)
    if (this.socket.canWrite()) this.join()
    else this.setStatus('offline')
  }

  stop(): void {
    if (this.stopped) return
    this.stopped = true
    if (this.status === 'live' || this.status === 'connecting') this.socket.sendDesign({ type: 'DesignLeave', screen: this.screen })
    this.socket.offDesign(this.onMessage)
    this.socket.offConnect(this.onConnect)
    this.socket.offDisconnect(this.onDisconnect)
    if (this.sendTimer) clearTimeout(this.sendTimer)
    if (this.awarenessTimer) clearTimeout(this.awarenessTimer)
    if (this.firstJoinTimer) clearTimeout(this.firstJoinTimer)
    this.awareness.destroy()
    this.status = 'off'
  }

  /** Ask the server to write pending edits to the draft now (Ctrl+S). */
  flushNow(): void {
    this.sendQueued()
    if (this.status === 'live') this.socket.sendDesign({ type: 'DesignFlush', screen: this.screen })
  }

  /** Set fields of this client's presence (cursor, selection, viewport). */
  setPresence(fields: Partial<Omit<Peer, 'clientId' | 'user' | 'color'>>): void {
    const cur = this.awareness.getLocalState() ?? {}
    this.awareness.setLocalState({ ...cur, ...fields })
  }

  peers(): Peer[] {
    const out: Peer[] = []
    this.awareness.getStates().forEach((s: any, id) => {
      if (id === this.doc.clientID || !s || !s.user) return
      out.push({ clientId: id, user: String(s.user.name ?? ''), color: String(s.user.color ?? '#888'), cursor: s.cursor ?? null, selection: Array.isArray(s.selection) ? s.selection : [], viewport: s.viewport ?? null })
    })
    return out.sort((a, b) => a.user.localeCompare(b.user) || a.clientId - b.clientId)
  }

  // ── Document ──

  private onUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin === REMOTE || origin === RESET) return
    if (this.status !== 'live') return
    this.queue.push(update)
    this.scheduleSend()
    this.emit('pending', true)
  }

  private onWidgets = (events: Y.YEvent<any>[], txn: Y.Transaction) => {
    if (txn.origin === REMOTE || txn.origin === RESET || this.status === 'live') return
    touchedKeys(events).forEach(k => this.offlineTouched.widgets.add(k))
  }

  private onConnections = (events: Y.YEvent<any>[], txn: Y.Transaction) => {
    if (txn.origin === REMOTE || txn.origin === RESET || this.status === 'live') return
    touchedKeys(events).forEach(k => this.offlineTouched.connections.add(k))
  }

  private onAwareness = ({ added, updated, removed }: { added: number[]; updated: number[]; removed: number[] }, origin: unknown) => {
    if (origin === 'local') {
      this.awarenessDirty = true
      this.scheduleAwareness()
    }
    const me = this.doc.clientID
    if ([...added, ...updated, ...removed].some(id => id !== me)) this.emit('peers', this.peers())
  }

  private bindDoc() {
    this.doc.on('update', this.onUpdate)
    widgetsOf(this.doc).observeDeep(this.onWidgets)
    connectionsOf(this.doc).observeDeep(this.onConnections)
    this.awareness.on('update', this.onAwareness)
  }

  /** Start over with an empty document (the old one is left to its owner). */
  private replaceDoc() {
    const local = this.awareness.getLocalState()
    this.doc.off('update', this.onUpdate)
    widgetsOf(this.doc).unobserveDeep(this.onWidgets)
    connectionsOf(this.doc).unobserveDeep(this.onConnections)
    this.awareness.off('update', this.onAwareness)
    this.awareness.destroy()
    this.doc = new Y.Doc()
    this.awareness = new awarenessProtocol.Awareness(this.doc)
    if (local) this.awareness.setLocalState(local)
    this.bindDoc()
    this.queue = []
    this.emit('doc', this.doc)
  }

  /** The objects changed while offline, as they are here (null: removed). */
  private captureOffline(): Snapshot[] {
    const out: Snapshot[] = []
    const ws = widgetsOf(this.doc), cs = connectionsOf(this.doc)
    for (const id of this.offlineTouched.widgets) { const m = ws.get(id); out.push({ kind: 'widget', id, json: m instanceof Y.Map ? snapshotMap(m) : null }) }
    for (const id of this.offlineTouched.connections) { const m = cs.get(id); out.push({ kind: 'connection', id, json: m instanceof Y.Map ? snapshotMap(m) : null }) }
    return out
  }

  private applyReplay(snaps: Snapshot[]) {
    if (!snaps.length) return
    this.doc.transact(() => {
      const ws = widgetsOf(this.doc), cs = connectionsOf(this.doc)
      for (const s of snaps) {
        const map = s.kind === 'widget' ? ws : cs
        if (s.json) map.set(s.id, restoreMap(s.json))
        else map.delete(s.id)
      }
    }, LOCAL)
  }

  // ── Socket ──

  private join() {
    if (this.stopped) return
    this.synced = false
    this.setStatus('connecting')
    if (!this.socket.sendDesign({ type: 'DesignJoin', screen: this.screen, protocol: PROTOCOL, epoch: this.epoch })) this.setStatus('offline')
  }

  private onConnect = () => { if (!this.stopped && this.status !== 'off') this.join() }

  private onDisconnect = () => {
    if (this.stopped || this.status === 'off') return
    this.synced = false
    this.setStatus('offline')
    const others = [...this.awareness.getStates().keys()].filter(id => id !== this.doc.clientID)
    awarenessProtocol.removeAwarenessStates(this.awareness, others, REMOTE)
    // Whatever was queued is covered by the sync exchange on the next join.
    this.queue = []
  }

  private sendSync(bytes: Uint8Array) {
    this.socket.sendDesign({ type: 'DesignSync', screen: this.screen, data: bytes })
    this.sent.push(this.now())
  }

  private onMessage = (msg: DesignServerMessage) => {
    if (this.stopped) return
    if (msg.type === 'DesignSessions') { this.emit('sessions', msg.sessions); return }
    if (msg.screen !== this.screen) return
    switch (msg.type) {
      case 'DesignJoined': {
        if (msg.protocol !== PROTOCOL) { this.fallBack('protocol', 'The server speaks another collaboration protocol; reload the page'); return }
        const changedEpoch = this.epoch != null && this.epoch !== msg.epoch
        if (msg.resync || changedEpoch) {
          this.replay = this.captureOffline()
          this.replaceDoc()
        }
        this.epoch = msg.epoch
        this.user = msg.user
        this.color = msg.color
        this.limits = { ...DEFAULT_LIMITS, ...msg.limits }
        this.setStatus('live')
        const enc = encoding.createEncoder()
        syncProtocol.writeSyncStep1(enc, this.doc)
        this.sendSync(encoding.toUint8Array(enc))
        const cur = this.awareness.getLocalState() ?? {}
        this.awareness.setLocalState({ ...cur, user: { name: msg.user, color: msg.color } })
        return
      }
      case 'DesignSync': {
        const dec = decoding.createDecoder(msg.data)
        const enc = encoding.createEncoder()
        let type: number
        try {
          type = syncProtocol.readSyncMessage(dec, enc, this.doc, REMOTE)
        } catch (e) {
          console.warn('collab: bad sync message', e)
          return
        }
        if (encoding.length(enc) > 0) this.sendSync(encoding.toUint8Array(enc))
        if (type === syncProtocol.messageYjsSyncStep2 && !this.synced) {
          this.synced = true
          this.everSynced = true
          const replay = this.replay
          this.replay = null
          if (replay) this.applyReplay(replay)
          this.offlineTouched = { widgets: new Set(), connections: new Set() }
          this.emit('synced', true)
        }
        return
      }
      case 'DesignAwareness':
        try { awarenessProtocol.applyAwarenessUpdate(this.awareness, msg.data, REMOTE) } catch (e) { console.warn('collab: bad awareness', e) }
        return
      case 'DesignError':
        if (msg.code === 'disabled' || msg.code === 'forbidden' || msg.code === 'protocol') { this.fallBack(msg.code, msg.message); return }
        this.emit('error', { code: msg.code, message: msg.message })
        if (msg.reload) {
          this.epoch = null
          this.replay = null
          this.offlineTouched = { widgets: new Set(), connections: new Set() }
          this.replaceDoc()
          this.join()
        }
        return
      case 'DesignRejected': this.emit('rejected', msg); return
      case 'DesignFlushed':
        if (!this.queue.length) this.unflushed = false
        this.emit('flushed', msg)
        this.emit('pending', this.pending)
        return
      case 'DesignComment': this.emit('comment', msg); return
    }
  }

  private fallBack(code: string, message: string) {
    this.setStatus('off')
    this.emit('fallback', { code, message })
  }

  // ── Sending ──

  private scheduleSend() {
    if (this.sendTimer) return
    this.sendTimer = setTimeout(() => { this.sendTimer = null; this.sendQueued() }, SEND_EVERY_MS)
  }

  /** Sync messages sent within the last second. */
  private rate(): number {
    const t = this.now() - 1000
    this.sent = this.sent.filter(s => s > t)
    return this.sent.length
  }

  private sendQueued() {
    if (!this.queue.length || this.status !== 'live') return
    // Leave room for the sync handshake and awareness under the server's limit.
    if (this.rate() >= Math.max(1, this.limits.max_rate - 10)) { this.scheduleSend(); return }
    const merged = Y.mergeUpdates(this.queue)
    const parts = merged.length + 16 <= this.limits.max_update ? [merged] : this.queue
    this.queue = []
    for (const u of parts) {
      const enc = encoding.createEncoder()
      syncProtocol.writeUpdate(enc, u)
      this.sendSync(encoding.toUint8Array(enc))
    }
    this.unflushed = true
    this.emit('pending', this.pending)
  }

  private scheduleAwareness() {
    if (this.awarenessTimer) return
    this.awarenessTimer = setTimeout(() => {
      this.awarenessTimer = null
      if (!this.awarenessDirty || this.status !== 'live') return
      this.awarenessDirty = false
      const update = awarenessProtocol.encodeAwarenessUpdate(this.awareness, [this.doc.clientID])
      if (update.length <= this.limits.max_awareness + 64) this.socket.sendDesign({ type: 'DesignAwareness', screen: this.screen, data: update })
    }, AWARENESS_EVERY_MS)
  }
}
