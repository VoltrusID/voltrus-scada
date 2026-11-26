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
import { CONNECTIONS, connectionsOf, WIDGETS, widgetsOf } from './ydoc.ts'
import { applyReplay, captureReplay, objectsOf, recordEvents, Touched, type ReplayOp } from './replay.ts'
import { splitUpdate } from './updates.ts'

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

export const DEFAULT_LIMITS: DesignLimits = { max_update: 256 * 1024, max_rate: 50, max_doc: 5 * 1024 * 1024, max_awareness: 4096, flush_ms: 2000, idle_ms: 120_000 }
/** Errors (reload: false) after which this connection is no longer a member: join again, keeping the document. */
const REJOIN = new Set(['resync', 'not_joined', 'rate_limited', 'idle'])
/** Waits before joining again after `rate_limited` (doubling), and the most. */
export const REJOIN_BACKOFF_MS = 1000
const REJOIN_BACKOFF_MAX_MS = 8000

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
  /** Dropped from the session (`resync`, `idle`, …): joining again, edits kept. */
  rejoin: { code: string; message: string }
  /** After a session restart, this many objects' unsaved edits were written again. */
  replayed: { objects: number }
  pending: boolean
}

type Listener<K extends keyof ProviderEvents> = (v: ProviderEvents[K]) => void

export class CollabProvider {
  doc: Y.Doc
  awareness: awarenessProtocol.Awareness
  status: CollabStatus = 'off'
  synced = false
  epoch: string | null = null
  user = ''
  color = '#888'
  limits: DesignLimits = DEFAULT_LIMITS
  /** Local updates not sent yet (or parts of one left for the next second). */
  private queue: Uint8Array[] = []
  private sendTimer: ReturnType<typeof setTimeout> | null = null
  private awarenessTimer: ReturnType<typeof setTimeout> | null = null
  private firstJoinTimer: ReturnType<typeof setTimeout> | null = null
  private rejoinTimer: ReturnType<typeof setTimeout> | null = null
  private keepTimer: ReturnType<typeof setTimeout> | null = null
  /** Whether a join ever completed: afterwards a lost socket means offline, not unavailable. */
  private everSynced = false
  private awarenessDirty = false
  /** Send times of the DesignSync messages of the last second. */
  private sent: number[] = []
  private lastSent = 0
  /** DesignSync messages sent since the latest DesignJoined (the server counts the same). */
  private msgNo = 0
  /** Fields changed here and not in a sent message yet (offline ones too). */
  private unsent = new Touched()
  /** Sent messages that carried local edits, by number, until a DesignFlushed `acked` covers them. */
  private inflight: { n: number; touched: Touched }[] = []
  /** Unwritten edits of the previous membership, for the step 2 reply of a same-epoch join. */
  private carry: Touched | null = null
  /** Unwritten edits of a session that is gone, written into the new document once it arrives. */
  private replay: ReplayOp[] | null = null
  /** A join was asked for (or is about to be) and DesignJoined has not come. */
  private joining = false
  private backoff = 0
  private lastRateLimited = -Infinity
  private listeners = new Map<keyof ProviderEvents, Set<(v: any) => void>>()
  private stopped = false

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
    return this.queue.length > 0 || this.unsent.size > 0 || this.inflight.length > 0
      || (this.carry?.size ?? 0) > 0 || (this.replay?.length ?? 0) > 0
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
    for (const t of [this.sendTimer, this.awarenessTimer, this.firstJoinTimer, this.rejoinTimer, this.keepTimer]) if (t) clearTimeout(t)
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
    // Not live: the sync exchange of the next join carries it.
    if (this.status === 'live') {
      this.queue.push(update)
      this.scheduleSend()
    }
    this.emit('pending', true)
  }

  private onWidgets = (events: Y.YEvent<any>[], txn: Y.Transaction) => {
    if (txn.origin !== REMOTE && txn.origin !== RESET) recordEvents(WIDGETS, events, this.unsent)
  }

  private onConnections = (events: Y.YEvent<any>[], txn: Y.Transaction) => {
    if (txn.origin !== REMOTE && txn.origin !== RESET) recordEvents(CONNECTIONS, events, this.unsent)
  }

  private onAwareness = ({ added, updated, removed }: { added: number[]; updated: number[]; removed: number[] }, origin: unknown) => {
    // y-protocols renews the local state every 15 s with origin 'local': it goes out too.
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

  /** Every local edit not written to the draft yet; forgotten here. */
  private takeUnwritten(): Touched {
    const t = new Touched()
    for (const e of this.inflight) t.addAll(e.touched)
    t.addAll(this.carry)
    t.addAll(this.unsent)
    this.inflight = []
    this.carry = null
    this.unsent = new Touched()
    return t
  }

  /** Drop the document and every unwritten edit, and join afresh. */
  private reset() {
    this.epoch = null
    this.replay = null
    this.takeUnwritten()
    this.replaceDoc()
    this.join()
  }

  // ── Socket ──

  private join() {
    if (this.rejoinTimer) { clearTimeout(this.rejoinTimer); this.rejoinTimer = null }
    if (this.stopped) return
    this.synced = false
    this.joining = true
    this.setStatus('connecting')
    if (!this.socket.sendDesign({ type: 'DesignJoin', screen: this.screen, protocol: PROTOCOL, epoch: this.epoch })) {
      this.joining = false
      this.setStatus('offline')
    }
  }

  /** No longer a member: join again, keeping the document and the epoch. */
  private rejoin(code: string, message: string) {
    if (this.stopped || this.status === 'off' || this.status === 'offline') return
    // While joining, only a refused join matters (the session closed under it);
    // errors about messages sent before the join are stale.
    if (this.joining && (code !== 'resync' || this.rejoinTimer)) return
    const now = this.now()
    let wait = 0
    if (code === 'rate_limited' || this.joining) {
      this.backoff = now - this.lastRateLimited < 30_000 ? Math.min(REJOIN_BACKOFF_MAX_MS, Math.max(REJOIN_BACKOFF_MS, this.backoff * 2)) : REJOIN_BACKOFF_MS
      this.lastRateLimited = now
      wait = this.backoff
    }
    if (!this.joining) this.emit('rejoin', { code, message })
    this.joining = true
    this.synced = false
    this.setStatus('connecting')
    if (wait) this.rejoinTimer = setTimeout(() => { this.rejoinTimer = null; this.join() }, wait)
    else this.join()
  }

  private onConnect = () => { if (!this.stopped && this.status !== 'off') this.join() }

  private onDisconnect = () => {
    if (this.stopped || this.status === 'off') return
    this.synced = false
    this.joining = false
    if (this.rejoinTimer) { clearTimeout(this.rejoinTimer); this.rejoinTimer = null }
    this.setStatus('offline')
    const others = [...this.awareness.getStates().keys()].filter(id => id !== this.doc.clientID)
    awarenessProtocol.removeAwarenessStates(this.awareness, others, REMOTE)
    // Whatever was queued is covered by the sync exchange on the next join.
    this.queue = []
  }

  /** Send a y-sync message; its number since the latest join. */
  private sendSync(bytes: Uint8Array): number {
    this.socket.sendDesign({ type: 'DesignSync', screen: this.screen, data: bytes })
    this.sent.push(this.now())
    this.lastSent = this.now()
    return ++this.msgNo
  }

  private onMessage = (msg: DesignServerMessage) => {
    if (this.stopped) return
    if (msg.type === 'DesignSessions') { this.emit('sessions', msg.sessions); return }
    if (msg.screen !== this.screen) return
    switch (msg.type) {
      case 'DesignJoined': {
        if (msg.protocol !== PROTOCOL) { this.fallBack('protocol', 'The server speaks another collaboration protocol; reload the page'); return }
        this.joining = false
        const changedEpoch = this.epoch != null && this.epoch !== msg.epoch
        const unwritten = this.takeUnwritten()
        this.msgNo = 0
        this.queue = []
        if (msg.resync || changedEpoch) {
          // The session the edits went to is gone: write them again into the new one.
          this.replay = [...(this.replay ?? []), ...captureReplay(this.doc, unwritten)]
          this.replaceDoc()
        } else if (unwritten.size) {
          // Same session: the step 2 reply carries whatever the server lacks.
          this.carry = unwritten
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
        this.keepAlive()
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
        if (encoding.length(enc) > 0) {
          const n = this.sendSync(encoding.toUint8Array(enc))
          if (type === syncProtocol.messageYjsSyncStep1 && this.carry) {
            this.inflight.push({ n, touched: this.carry })
            this.carry = null
          }
        }
        if (type === syncProtocol.messageYjsSyncStep2 && !this.synced) {
          this.synced = true
          this.everSynced = true
          const replay = this.replay
          this.replay = null
          if (replay?.length) {
            applyReplay(this.doc, replay, LOCAL)
            this.emit('replayed', { objects: objectsOf(replay) })
          }
          this.emit('synced', true)
          this.emit('pending', this.pending)
        }
        return
      }
      case 'DesignAwareness':
        try { awarenessProtocol.applyAwarenessUpdate(this.awareness, msg.data, REMOTE) } catch (e) { console.warn('collab: bad awareness', e) }
        return
      case 'DesignError':
        if (msg.code === 'disabled' || msg.code === 'forbidden' || msg.code === 'protocol') { this.fallBack(msg.code, msg.message); return }
        if (msg.reload) {
          this.emit('error', { code: msg.code, message: msg.message })
          this.reset()
        } else if (REJOIN.has(msg.code)) this.rejoin(msg.code, msg.message)
        else this.emit('error', { code: msg.code, message: msg.message })
        return
      case 'DesignRejected': this.emit('rejected', msg); return
      case 'DesignFlushed': {
        const acked = msg.acked
        if (typeof acked === 'number') this.inflight = this.inflight.filter(e => e.n > acked)
        else if (!this.queue.length) this.inflight = []
        this.emit('flushed', msg)
        this.emit('pending', this.pending)
        return
      }
      case 'DesignComment': this.emit('comment', msg); return
    }
  }

  private fallBack(code: string, message: string) {
    this.setStatus('off')
    this.emit('fallback', { code, message })
  }

  // ── Sending ──

  private scheduleSend(ms = SEND_EVERY_MS) {
    if (this.sendTimer) return
    this.sendTimer = setTimeout(() => { this.sendTimer = null; this.sendQueued() }, ms)
  }

  /** Sync messages sent within the last second. */
  private rate(): number {
    const t = this.now() - 1000
    this.sent = this.sent.filter(s => s > t)
    return this.sent.length
  }

  /** Until the oldest message of the last second leaves the window. */
  private untilRoom(): number {
    return Math.max(SEND_EVERY_MS, (this.sent[0] ?? this.now()) + 1001 - this.now())
  }

  private sendQueued() {
    if (!this.queue.length || this.status !== 'live') return
    // Leave room for the sync handshake under the server's limit.
    let room = Math.max(1, this.limits.max_rate - 10) - this.rate()
    if (room <= 0) { this.scheduleSend(this.untilRoom()); return }
    const max = this.limits.max_update - 64
    const parts = splitUpdate(this.queue.length === 1 ? this.queue[0] : Y.mergeUpdates(this.queue), max)
    if (!parts) {
      this.emit('error', { code: 'too_large', message: `a change is over the ${Math.round(this.limits.max_update / 1024)} KB limit and was not saved; the screen was reloaded` })
      this.reset()
      return
    }
    let n = 0, i = 0
    for (; i < parts.length && room > 0; i++, room--) {
      const enc = encoding.createEncoder()
      syncProtocol.writeUpdate(enc, parts[i])
      n = this.sendSync(encoding.toUint8Array(enc))
    }
    this.queue = parts.slice(i)
    if (this.queue.length) this.scheduleSend(this.untilRoom())
    else if (this.unsent.size) {
      this.inflight.push({ n, touched: this.unsent })
      this.unsent = new Touched()
    }
    this.emit('pending', this.pending)
  }

  private scheduleAwareness() {
    if (this.awarenessTimer) return
    this.awarenessTimer = setTimeout(() => {
      this.awarenessTimer = null
      if (!this.awarenessDirty || this.status !== 'live') return
      this.awarenessDirty = false
      const update = awarenessProtocol.encodeAwarenessUpdate(this.awareness, [this.doc.clientID])
      if (update.length <= this.limits.max_awareness + 64) {
        this.socket.sendDesign({ type: 'DesignAwareness', screen: this.screen, data: update })
        this.lastSent = this.now()
      }
    }, AWARENESS_EVERY_MS)
  }

  /** The server drops a member silent for `idle_ms`: renew the presence well before. */
  private keepAlive() {
    if (this.keepTimer) clearTimeout(this.keepTimer)
    const every = Math.max(500, Math.floor((this.limits.idle_ms ?? DEFAULT_LIMITS.idle_ms!) / 6))
    this.keepTimer = setTimeout(() => {
      this.keepTimer = null
      if (this.stopped) return
      if (this.status === 'live' && this.now() - this.lastSent >= every) this.awareness.setLocalState(this.awareness.getLocalState() ?? {})
      this.keepAlive()
    }, every)
  }
}
