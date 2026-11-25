// The Designer in a live session (phase 4) against a stand-in server: the
// shared document replaces load and save, other people show as avatars,
// cursors and selection outlines, and a server without collaboration keeps
// the phase 1/3 behaviour.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from '@tanstack/react-router'
import * as Y from 'yjs'
import * as encoding from 'lib0/encoding'
import * as decoding from 'lib0/decoding'
import * as syncProtocol from 'y-protocols/sync'
import * as awarenessProtocol from 'y-protocols/awareness'
import { Designer } from '../../src/designer/Designer'
import { createDashboardStore, DashboardStoreProvider } from '../../src/store/dashboardStore'
import { setWsInstance } from '../../src/api/websocket'
import { loadRows, readDoc } from '../../src/designer/collab/ydoc'

const row = (id: string, x: number) => ({
  screen_id: 's1', equipment_id: id, equipment_type: 'label', equipment_name: '', x, y: 100, width: 120, height: 40, rotation: 0,
  sensor_bindings: '[]', config: JSON.stringify({ text: id }),
})

/** The server end of one browser's socket, and a second person (ben) on the same document. */
class Stand {
  doc = new Y.Doc()
  ben = new Y.Doc()
  benAwareness = new awarenessProtocol.Awareness(this.ben)
  cbs = { design: new Set<(m: any) => void>(), connect: new Set<() => void>(), disconnect: new Set<() => void>() }
  received: any[] = []
  /** Browser sync messages since its latest join (DesignFlushed `acked`). */
  taken = 0
  /** Answer DesignFlush (off: edits stay sent but unwritten). */
  flushes = true
  constructor() {
    loadRows(this.doc, [row('a', 100), row('b', 400)] as any, [])
    Y.applyUpdate(this.ben, Y.encodeStateAsUpdate(this.doc))
    this.doc.on('update', (u: Uint8Array, origin: unknown) => {
      if (origin === 'browser') { Y.applyUpdate(this.ben, u, 'server'); return }
      const enc = encoding.createEncoder()
      syncProtocol.writeUpdate(enc, u)
      this.deliver({ type: 'DesignSync', screen: 's1', data: encoding.toUint8Array(enc) })
    })
    this.ben.on('update', (u: Uint8Array, origin: unknown) => { if (origin !== 'server') Y.applyUpdate(this.doc, u, 'ben') })
  }
  deliver(m: any) { queueMicrotask(() => this.cbs.design.forEach(cb => cb(m))) }
  // VoltrusWebSocket's surface
  isConnected() { return true }
  canWrite() { return true }
  onConnect(cb: () => void) { this.cbs.connect.add(cb) }
  offConnect(cb: () => void) { this.cbs.connect.delete(cb) }
  onDisconnect(cb: () => void) { this.cbs.disconnect.add(cb) }
  offDisconnect(cb: () => void) { this.cbs.disconnect.delete(cb) }
  onDesign(cb: (m: any) => void) { this.cbs.design.add(cb) }
  offDesign(cb: (m: any) => void) { this.cbs.design.delete(cb) }
  sendDesign(m: any) {
    this.received.push(m)
    queueMicrotask(() => {
      if (m.type === 'DesignJoin') {
        this.taken = 0
        this.deliver({ type: 'DesignJoined', screen: 's1', epoch: 'e1', resync: false, user: 'ana', color: '#2563eb', protocol: 1, limits: { max_update: 262144, max_rate: 50, max_doc: 5e6, max_awareness: 4096, flush_ms: 2000 } })
        const enc = encoding.createEncoder()
        syncProtocol.writeSyncStep1(enc, this.doc)
        this.deliver({ type: 'DesignSync', screen: 's1', data: encoding.toUint8Array(enc) })
        this.deliver({ type: 'DesignSessions', sessions: [{ screen: 's1', users: [{ user: 'ana', color: '#2563eb' }, { user: 'ben', color: '#e11d48' }] }, { screen: 's2', users: [{ user: 'ben', color: '#e11d48' }] }] })
      } else if (m.type === 'DesignSync') {
        this.taken++
        const enc = encoding.createEncoder()
        syncProtocol.readSyncMessage(decoding.createDecoder(m.data), enc, this.doc, 'browser')
        if (encoding.length(enc) > 0) this.deliver({ type: 'DesignSync', screen: 's1', data: encoding.toUint8Array(enc) })
      } else if (m.type === 'DesignFlush' && this.flushes) {
        this.deliver({ type: 'DesignFlushed', screen: 's1', seq: 9, at_ms: Date.now(), users: ['ana'], acked: this.taken })
      }
    })
    return true
  }
  /** Ben points at the stage and selects a widget. */
  benPresence(cursor: { x: number; y: number }, selection: string[]) {
    this.benAwareness.setLocalState({ user: { name: 'ben', color: '#e11d48' }, cursor, selection })
    this.deliver({ type: 'DesignAwareness', screen: 's1', data: awarenessProtocol.encodeAwarenessUpdate(this.benAwareness, [this.ben.clientID]) })
  }
  benMoves(id: string, x: number) {
    this.ben.transact(() => { (this.ben.getMap('widgets').get(id) as Y.Map<any>).set('x', x) })
  }
}

let stand: Stand
let puts: any[]
let collaboration: any
/** Publish requests, and the draft head the diff reports. */
let publishes: any[]
let draftSeq: number
let publishConflicts: number
let publishing: boolean

function stubServer() {
  puts = []
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const u = String(url)
    const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { 'Content-Type': 'application/json' } })
    if (u.endsWith('/api/v1/screens')) return json([{ id: 's1', name: 'Pumps' }, { id: 's2', name: 'Tanks' }])
    if (u.endsWith('/api/v1/auth/me')) return json({ authenticated: true, username: 'ana', role: 'admin' })
    if (/design-status$/.test(u)) return json({ screen_id: 's1', seq: 1, revision: 'r1', publishing, approval: 'none', published: null, changes_since_published: 0, last_version: null, changes_since_version: 0, pending: [], ...(collaboration ? { collaboration } : {}) })
    if (/\/comments/.test(u)) return json({ comments: [] })
    if (/\/diff\?/.test(u)) {
      const side = { added: [], removed: [], changed: [] }
      return json({ from: { kind: 'published' }, to: { kind: 'draft', seq: draftSeq }, diff: { widgets: side, connections: side, summary: { added: 0, removed: 0, changed: 2 } } })
    }
    if (/\/publish$/.test(u)) {
      publishes.push(JSON.parse(String(init!.body)))
      if (publishConflicts > 0) { publishConflicts--; draftSeq++; return json({ code: 'conflict', error: 'the draft moved', seq: draftSeq }, 409) }
      return json({ status: 'published', version: { id: 3, name: 'v3' } })
    }
    if (/scada-layout$/.test(u) && (init?.method ?? 'GET') === 'GET') return json({ equipment: [row('a', 100), row('b', 400)], connections: [], revision: 'r1', seq: 1 })
    if (/scada-layout$/.test(u)) { puts.push(JSON.parse(String(init!.body))); return json({ ok: true, revision: 'r2' }) }
    return json([])
  }))
}

let history: ReturnType<typeof createMemoryHistory>

function setup() {
  history = createMemoryHistory({ initialEntries: ['/designer?screen=s1'] })
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const root = createRootRoute({ component: () => <Designer screen="s1" onOpenScreen={() => {}} onClose={() => {}} onOpenPage={() => {}} /> })
  const router = createRouter({ routeTree: root.addChildren([createRoute({ getParentRoute: () => root, path: '/designer', component: () => null })]), history })
  render(<QueryClientProvider client={qc}><DashboardStoreProvider value={createDashboardStore()}><RouterProvider router={router as any} /></DashboardStoreProvider></QueryClientProvider>)
}

const key = (k: string, opts: KeyboardEventInit = {}) => act(() => { fireEvent.keyDown(document.body, { key: k, ...opts }) })

/** Whether closing the tab would ask first (the router's beforeunload blockers). */
const unloadBlocked = () => (history as any)._getBlockers().some((b: any) => (typeof b.enableBeforeUnload === 'function' ? b.enableBeforeUnload() : b.enableBeforeUnload ?? true))

beforeEach(() => {
  publishes = []
  draftSeq = 7
  publishConflicts = 0
  publishing = false
  collaboration = { enabled: true, protocol: 1, editors: [] }
  stubServer()
  stand = new Stand()
  setWsInstance(stand as any)
})
afterEach(() => { setWsInstance(null); stand.benAwareness.destroy(); vi.unstubAllGlobals(); vi.restoreAllMocks() })

describe('Designer in a live session', () => {
  it('edits the shared document: no PUT, the other person sees the change, Ctrl+S flushes', async () => {
    setup()
    await waitFor(() => expect(screen.getByTestId('designer-collab-status').textContent).toContain('Live'), { timeout: 10_000 })
    await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(2))
    key('a', { ctrlKey: true })
    key('ArrowRight')
    await waitFor(() => expect(readDoc(stand.ben).widgets.map(w => w.x)).toEqual([101, 401]), { timeout: 2000 })
    expect(screen.getByTestId('designer-dirty').textContent).toBe('Saving…')
    key('s', { ctrlKey: true })
    await waitFor(() => expect(screen.getByTestId('designer-dirty').textContent).toBe('All changes saved'))
    expect(stand.received.some(m => m.type === 'DesignFlush')).toBe(true)
    expect(puts).toHaveLength(0)
    // Undo is mine only, and travels too.
    key('z', { ctrlKey: true })
    await waitFor(() => expect(readDoc(stand.ben).widgets.map(w => w.x)).toEqual([100, 400]), { timeout: 2000 })
  })

  it('shows the other person: avatar, cursor with name, selection outline, and their edits', async () => {
    setup()
    await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(2), { timeout: 10_000 })
    act(() => stand.benPresence({ x: 300, y: 200 }, ['b']))
    await waitFor(() => expect(screen.getByTestId('designer-peer-avatar').getAttribute('data-user')).toBe('ben'))
    expect(screen.getByTestId('designer-peer-cursor').textContent).toContain('ben')
    expect(screen.getAllByTestId('designer-peer-selection')).toHaveLength(1)
    // Who edits which screen, in the Project pane.
    expect(screen.getByTestId('designer-screen-editors-s2')).toBeTruthy()
    act(() => stand.benMoves('a', 640))
    await waitFor(() => expect(screen.getByTestId('designer-collab-status').textContent).toContain('2 editing'))
    await waitFor(() => {
      const a = screen.getAllByTestId('designer-widget').find(e => e.getAttribute('data-wid') === 'a') as HTMLElement
      expect(a.style.left).toBe('640px')
    })
  })

  it('closing the tab asks first while edits are sent but not written to the draft', async () => {
    setup()
    await waitFor(() => expect(screen.getByTestId('designer-collab-status').textContent).toContain('Live'), { timeout: 10_000 })
    await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(2))
    expect(unloadBlocked()).toBe(false)
    stand.flushes = false
    key('a', { ctrlKey: true })
    key('ArrowRight')
    await waitFor(() => expect(readDoc(stand.ben).widgets.map(w => w.x)).toEqual([101, 401]), { timeout: 2000 })
    key('s', { ctrlKey: true })
    await act(() => new Promise(r => setTimeout(r, 100)))
    expect(screen.getByTestId('designer-dirty').textContent).toBe('Saving…')
    expect(unloadBlocked()).toBe(true)
    stand.flushes = true
    key('s', { ctrlKey: true })
    await waitFor(() => expect(screen.getByTestId('designer-dirty').textContent).toBe('All changes saved'))
    expect(unloadBlocked()).toBe(false)
  })

  it('publishing while live checks against the draft the dialog showed; a moved draft is shown again', async () => {
    publishConflicts = 1
    publishing = true
    setup()
    await waitFor(() => expect(screen.getByTestId('designer-collab-status').textContent).toContain('Live'), { timeout: 10_000 })
    fireEvent.click(screen.getByTestId('designer-publish'))
    await waitFor(() => expect((screen.getByTestId('designer-publish-confirm') as HTMLButtonElement).disabled).toBe(false))
    // Opening it writes pending live edits first.
    expect(stand.received.some(m => m.type === 'DesignFlush')).toBe(true)
    fireEvent.click(screen.getByTestId('designer-publish-confirm'))
    await waitFor(() => expect(screen.getByTestId('designer-publish-error').textContent).toContain('draft changed'))
    expect(publishes[0].base_seq).toBe(7)
    await waitFor(() => expect(screen.getByTestId('designer-publish-confirm').textContent).toBe('Publish'))
    await act(() => new Promise(r => setTimeout(r, 50)))
    fireEvent.click(screen.getByTestId('designer-publish-confirm'))
    await waitFor(() => expect(screen.getByTestId('designer-publish-result')).toBeTruthy())
    expect(publishes[1].base_seq).toBe(8)
  })

  it('without collaboration on the server the Designer saves whole layouts as before', async () => {
    collaboration = null
    setup()
    await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(2), { timeout: 10_000 })
    expect(screen.queryByTestId('designer-presence')).toBeNull()
    key('a', { ctrlKey: true })
    key('ArrowRight')
    expect(screen.getByTestId('designer-dirty').textContent).toBe('Unsaved changes')
    key('s', { ctrlKey: true })
    await waitFor(() => expect(puts).toHaveLength(1))
    expect(stand.received).toHaveLength(0)
  })
})
