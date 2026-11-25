// Version history in the Designer (docs/designer.md, phase 3): the History
// pane and its filters, previewing a past state, reverts (with "also restore"
// for a dangling connection), blame, versions and compare, publishing (with
// approval and signatures), the conflict dialog's "who changed what", and the
// runtime reading the published screen while editors read the draft.
import React, { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from '@tanstack/react-router'
import { Designer } from '../../src/designer/Designer'
import { ScadaView } from '../../src/scada2/ScadaView'
import { useScadaRows } from '../../src/queries/scada'
import { createDashboardStore, DashboardStoreProvider } from '../../src/store/dashboardStore'
import { answerSignaturePrompt, currentSignaturePrompt, subscribeSignaturePrompt } from '../../src/signature/prompt'

const screens = [{ id: 's1', name: 'Pumps' }]
const sensors = [{ id: 1, key: 'level', name: 'Level', device_id: 'wtp', unit: '%', min_val: 0, max_val: 100, decimals: 1 }]
const row = (id: string, x: number) => ({
  screen_id: 's1', equipment_id: id, equipment_type: 'label', equipment_name: '', x, y: 100, width: 120, height: 40, rotation: 0,
  sensor_bindings: '[]', config: JSON.stringify({ text: id, label: `L-${id}` }),
})
const NOW = Date.now()
const change = (seq: number, batch: string, object_id: string, kind: 'add' | 'update' | 'remove', username = 'ana', object_type = 'widget') => ({
  seq, batch, screen_id: 's1', object_type, object_id, kind,
  before: kind === 'add' ? null : row(object_id, 100), after: kind === 'remove' ? null : row(object_id, kind === 'update' ? 140 : 100),
  username, source: username === 'bob' ? 'mcp' : 'designer', operation: 'save', note: '', at_ms: NOW - (20 - seq) * 60_000, squashed: 0,
})
const version = (id: number, name: string, extra: Record<string, any> = {}) => ({
  id, screen_id: 's1', name, message: `message of ${name}`, upto_seq: 5, username: 'ana', at_ms: NOW - 3600_000, status: 'saved',
  requested_by: null, reviewed_by: null, review_note: null, reviewed_at_ms: null, published_at_ms: null, published_by: null,
  signature: null, current: false, widgets: 2, connections: 0, ...extra,
})

interface Server {
  calls: { method: string; url: string; body: any; headers: Record<string, string> }[]
  status: Record<string, any>
  history: any[]
  versions: any[]
  layoutGets: number
  me: { username: string; role: string }
  put: (body: any) => Response
  revert: (body: any, n: number) => Response
  publish: (body: any, n: number) => Response
}
let server: Server
const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { 'Content-Type': 'application/json' } })

function stubServer() {
  server = {
    calls: [],
    status: { screen_id: 's1', seq: 10, revision: 'r1', publishing: true, approval: 'none', published: null, changes_since_published: 0, last_version: null, changes_since_version: 3, pending: [] },
    history: [change(10, 'B3', 'b', 'update', 'bob'), change(9, 'B3', 'c1', 'add', 'bob', 'connection'), change(7, 'B2', 'a', 'update'), change(5, 'B1', 'a', 'add'), change(4, 'B1', 'b', 'add')],
    versions: [version(1, 'v1')],
    layoutGets: 0,
    me: { username: 'admin', role: 'admin' },
    put: () => json({ ok: true, revision: 'r-saved', seq: 11, batch: 'B4' }),
    revert: () => json({ ok: true, batch: 'B5', seq: 12, revision: 'r5', removed_connections: [], ungrouped: [] }),
    publish: (b) => json({ status: 'published', version: version(2, b.name || 'v2', { status: 'published', current: true }) }),
  }
  let reverts = 0, publishes = 0
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const u = String(url)
    const method = init?.method ?? 'GET'
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : null
    server.calls.push({ method, url: u, body, headers: (init?.headers ?? {}) as Record<string, string> })
    const path = u.replace(/\?.*$/, '')
    if (path.endsWith('/api/v1/screens')) return json(screens)
    if (path.endsWith('/api/v1/data-sources')) return json(sensors)
    if (path.endsWith('/api/v1/faceplates')) return json([])
    if (path.endsWith('/api/v1/auth/me')) return json({ authenticated: true, ...server.me })
    if (path.endsWith('/s1/scada-layout') && method === 'GET') {
      server.layoutGets++
      return json({ equipment: [row('a', 100), row('b', 400)], connections: [], revision: 'r1', seq: 10, view: 'draft' })
    }
    if (path.endsWith('/s1/scada-layout') && method === 'PUT') return server.put(body)
    if (path.endsWith('/s1/scada-equipment')) return json({ items: [row('a', 100)], view: u.includes('published=1') ? 'published' : 'draft' })
    if (path.endsWith('/s1/scada-connections')) return json({ items: [], view: u.includes('published=1') ? 'published' : 'draft' })
    if (path.endsWith('/s1/design-status')) return json(server.status)
    if (path.endsWith('/s1/history')) {
      const q = new URL(u, 'http://x').searchParams
      const rows = server.history.filter(c => (!q.get('object') || c.object_id === q.get('object')) && (!q.get('user') || c.username === q.get('user')))
      return json({ changes: rows, next_before_seq: null, users: [...new Set(server.history.map(c => c.username))].sort() })
    }
    if (path.endsWith('/s1/blame')) {
      return json({ objects: [{ object_type: 'widget', object_id: 'a', exists: true, seq: 7, batch: 'B2', kind: 'update', username: 'ana', source: 'designer', operation: 'save', note: '', at_ms: NOW - 5 * 60_000, changes: 2, created_by: 'bob', created_at_ms: Date.UTC(2026, 9, 3, 12) }] })
    }
    if (path.endsWith('/s1/versions') && method === 'GET') return json({ versions: server.versions })
    if (path.endsWith('/s1/versions') && method === 'POST') {
      const v = version(server.versions.length + 1, body.name || `v${server.versions.length + 1}`, { message: body.message })
      server.versions = [v, ...server.versions]
      return json({ version: v }, 201)
    }
    if (path.endsWith('/s1/state')) return json({ state: { kind: 'seq' }, equipment: [row('a', 100)], connections: [] })
    if (path.endsWith('/s1/diff')) {
      return json({
        from: { kind: 'version' }, to: { kind: 'draft' },
        diff: {
          widgets: { added: [row('b', 400)], removed: [], changed: [{ id: 'a', fields: ['x'], before: row('a', 100), after: row('a', 140) }] },
          connections: { added: [], removed: [], changed: [] },
          summary: { added: 1, removed: 0, changed: 1 },
        },
        yaml: { from: 'widgets:\n- id: a\n  x: 100\n', to: 'widgets:\n- id: a\n  x: 140\n- id: b\n  x: 400\n' },
        rows: { from: { equipment: [row('a', 100)], connections: [] }, to: { equipment: [row('a', 140), row('b', 400)], connections: [] } },
      })
    }
    if (path.endsWith('/s1/revert')) return server.revert(body, ++reverts)
    if (path.endsWith('/s1/publish')) return server.publish(body, ++publishes)
    const review = /\/s1\/versions\/([^/]+)\/(approve|reject)$/.exec(path)
    if (review) return json({ status: review[2] === 'approve' ? 'published' : 'rejected', version: version(Number(review[1]), 'v2') })
    return json([])
  }))
}

function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const root = createRootRoute({ component: () => <Designer screen="s1" onOpenScreen={() => {}} onClose={() => {}} onOpenPage={() => {}} /> })
  const routes = ['/', '/designer'].map(p => createRoute({ getParentRoute: () => root, path: p, component: () => null }))
  const router = createRouter({ routeTree: root.addChildren(routes), history: createMemoryHistory({ initialEntries: ['/designer?screen=s1'] }) })
  render(<QueryClientProvider client={qc}><DashboardStoreProvider value={createDashboardStore()}><RouterProvider router={router as any} /></DashboardStoreProvider></QueryClientProvider>)
  return { qc }
}

const T = { timeout: 10_000 }
const key = (k: string, opts: KeyboardEventInit = {}) => act(() => { fireEvent.keyDown(document.body, { key: k, ...opts }) })
const calls = (re: RegExp, method = 'GET') => server.calls.filter(c => c.method === method && re.test(c.url))

async function loaded() {
  const r = setup()
  await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(2), T)
  return r
}

/** Select one component from the Project pane. */
function selectLayer(label: string) {
  fireEvent.click(within(screen.getByTestId('designer-project')).getByText(label))
}

async function openTab(tab: 'history' | 'versions' | 'properties') {
  fireEvent.click(screen.getByTestId(`designer-tab-${tab}`))
}

beforeEach(() => stubServer())
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks() })

describe('History pane', () => {
  it('lists the saves of the screen grouped by batch, with user, source and objects', async () => {
    await loaded()
    await openTab('history')
    await waitFor(() => expect(screen.getAllByTestId('designer-history-batch')).toHaveLength(3), T)
    const [b3] = screen.getAllByTestId('designer-history-batch')
    expect(b3.textContent).toContain('bob')
    expect(within(b3).getByTestId('designer-history-source').textContent).toBe('MCP')
    expect(b3.textContent).toContain('1 added · 1 changed')
    expect(screen.getByTestId('designer-history-scope').textContent).toBe('whole screen')
    expect(calls(/\/history\?/).at(-1)!.url).not.toContain('object=')
  }, 20_000)

  it('follows the selected component, "Whole screen" widens it, and the user filter narrows it', async () => {
    await loaded()
    selectLayer('L-a')
    await openTab('history')
    await waitFor(() => expect(screen.getAllByTestId('designer-history-batch')).toHaveLength(2), T)
    expect(screen.getByTestId('designer-history-scope').textContent).toBe('L-a')
    expect(calls(/\/history\?/).at(-1)!.url).toMatch(/object=a&object_type=widget/)
    fireEvent.click(screen.getByTestId('designer-history-whole'))
    await waitFor(() => expect(screen.getAllByTestId('designer-history-batch')).toHaveLength(3), T)
    fireEvent.change(screen.getByTestId('designer-history-user'), { target: { value: 'bob' } })
    await waitFor(() => expect(screen.getAllByTestId('designer-history-batch')).toHaveLength(1), T)
    expect(calls(/\/history\?/).at(-1)!.url).toMatch(/user=bob/)
    // Everyone seen stays in the list after filtering.
    expect([...screen.getByTestId('designer-history-user').querySelectorAll('option')].map(o => o.textContent)).toEqual(['Everyone', 'ana', 'bob'])
  }, 20_000)

  it('clicking a batch previews that state read-only; editing is off until Back to draft', async () => {
    await loaded()
    await openTab('history')
    await waitFor(() => expect(screen.getAllByTestId('designer-history-batch')).toHaveLength(3), T)
    fireEvent.click(within(screen.getAllByTestId('designer-history-batch')[1]).getAllByRole('button')[0])
    const banner = await screen.findByTestId('designer-preview-banner')
    expect(banner.textContent).toMatch(/after change #7 by ana/)
    await waitFor(() => expect(calls(/\/state\?at=seq%3A7/)).toHaveLength(1))
    await waitFor(() => expect(within(screen.getByTestId('designer-preview-stage')).getAllByTestId('designer-widget')).toHaveLength(1))
    // Keys do not edit the draft meanwhile.
    key('a', { ctrlKey: true })
    key('Delete')
    fireEvent.click(screen.getByTestId('designer-preview-back'))
    await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(2))
    expect(screen.getByTestId('designer-dirty').textContent).toBe('All changes saved')
  }, 20_000)
})

describe('revert', () => {
  it('a connection whose widget is gone: the 409 offers "Also restore", which resends both in one request', async () => {
    await loaded()
    server.revert = (body, n) => n === 1
      ? json({ error: 'Connection c1 would end on P-101', code: 'dangling_connection', connection: 'c1', missing: ['P-101'] }, 409)
      : json({ ok: true, batch: 'B5', seq: 12, revision: 'r5', removed_connections: [], ungrouped: [] })
    await openTab('history')
    await waitFor(() => expect(screen.getAllByTestId('designer-history-batch')).toHaveLength(3), T)
    fireEvent.click(within(screen.getAllByTestId('designer-history-batch')[0]).getAllByRole('button')[0])
    fireEvent.click(await screen.findByTestId('designer-revert-object-c1'))
    fireEvent.click(await screen.findByTestId('designer-revert-confirm'))
    const dlg = await screen.findByTestId('designer-dangling-dialog')
    expect(dlg.textContent).toContain('P-101')
    const before = server.layoutGets
    fireEvent.click(within(dlg).getByTestId('designer-revert-also-restore'))
    await waitFor(() => expect(calls(/\/revert$/, 'POST')).toHaveLength(2))
    const [first, second] = calls(/\/revert$/, 'POST').map(c => c.body)
    // The state after the whole save (#10), not after the connection's own row (#9).
    expect(first).toEqual({ object: 'c1', object_type: 'connection', to_seq: 10, base_seq: 10 })
    expect(second).toEqual({
      objects: [{ object_id: 'c1', object_type: 'connection', to_seq: 10 }, { object_id: 'P-101', object_type: 'widget', to_seq: 10 }],
      base_seq: 10,
    })
    // A revert is new draft changes: the layout is loaded again.
    await waitFor(() => expect(server.layoutGets).toBeGreaterThan(before))
  }, 20_000)

  it('Revert screen to here sends to: seq:<n>; with unsaved edits it asks to save first', async () => {
    await loaded()
    key('a', { ctrlKey: true })
    key('ArrowRight')
    await openTab('history')
    await waitFor(() => expect(screen.getAllByTestId('designer-history-batch')).toHaveLength(3), T)
    fireEvent.click(within(screen.getAllByTestId('designer-history-batch')[2]).getAllByRole('button')[0])
    // Previewing does not touch the unsaved draft; back to it first.
    fireEvent.click(await screen.findByTestId('designer-preview-revert'))
    fireEvent.click(await screen.findByTestId('designer-needsave-save'))
    await waitFor(() => expect(calls(/scada-layout$/, 'PUT')).toHaveLength(1))
    fireEvent.click(await screen.findByTestId('designer-revert-confirm'))
    await waitFor(() => expect(calls(/\/revert$/, 'POST')).toHaveLength(1))
    // Based on the seq the save returned.
    expect(calls(/\/revert$/, 'POST')[0].body).toEqual({ to: 'seq:5', base_seq: 11 })
    await waitFor(() => expect(screen.queryByTestId('designer-preview-banner')).toBeNull())
  }, 20_000)
})

describe('blame', () => {
  it('the Properties pane says who changed the selected component last; History opens its history', async () => {
    await loaded()
    selectLayer('L-a')
    const blame = await screen.findByTestId('designer-blame')
    await waitFor(() => expect(blame.textContent).toMatch(/Last changed by ana · 5 min ago · via Designer \(2 changes\) · added by bob on 3 Oct 2026/))
    fireEvent.click(screen.getByTestId('designer-blame-history'))
    await waitFor(() => expect(screen.getByTestId('designer-history-scope').textContent).toBe('L-a'))
  }, 20_000)
})

describe('publish', () => {
  it('title bar shows the draft state; Publish sends name, message and the base seq', async () => {
    server.status = { ...server.status, published: { version_id: 1, name: 'v1', upto_seq: 5, at_ms: NOW, by: 'ana', draft_differs: true }, changes_since_published: 3 }
    await loaded()
    await waitFor(() => expect(screen.getByTestId('designer-publish-state').textContent).toBe('Draft · 3 changes since v1'))
    fireEvent.click(screen.getByTestId('designer-publish'))
    const dlg = await screen.findByTestId('designer-publish-dialog')
    await waitFor(() => expect(within(dlg).getByTestId('designer-publish-summary').textContent).toContain('1 added · 1 changed'))
    expect((within(dlg).getByTestId('designer-publish-name') as HTMLInputElement).placeholder).toBe('v2')
    fireEvent.change(within(dlg).getByTestId('designer-publish-name'), { target: { value: 'v2' } })
    fireEvent.change(within(dlg).getByTestId('designer-publish-message'), { target: { value: 'New pump' } })
    fireEvent.click(within(dlg).getByTestId('designer-publish-confirm'))
    const result = await screen.findByTestId('designer-publish-result')
    expect(result.dataset.status).toBe('published')
    expect(calls(/\/publish$/, 'POST')[0].body).toEqual({ name: 'v2', message: 'New pump', base_seq: 10 })
  }, 20_000)

  it('with approval required the button asks for it and the result is pending; unsaved edits are saved first', async () => {
    server.status = { ...server.status, approval: 'required' }
    server.publish = b => json({ status: 'pending', version: version(2, 'v2', { status: 'pending', requested_by: 'admin', message: b.message }) })
    await loaded()
    await waitFor(() => expect(screen.getByTestId('designer-publish').textContent).toBe('Request publish'))
    expect(screen.getByTestId('designer-publish-state').textContent).toBe('Not published (operators see the draft)')
    key('a', { ctrlKey: true })
    key('ArrowRight')
    fireEvent.click(screen.getByTestId('designer-publish'))
    const confirm = await screen.findByTestId('designer-publish-confirm')
    expect(confirm.textContent).toBe('Save & request publish')
    fireEvent.click(confirm)
    const result = await screen.findByTestId('designer-publish-result')
    expect(result.dataset.status).toBe('pending')
    expect(calls(/scada-layout$/, 'PUT')).toHaveLength(1)
    expect(calls(/\/publish$/, 'POST')[0].body.base_seq).toBe(11)
  }, 20_000)

  it('a 428 asks for an electronic signature and sends it with the publish', async () => {
    server.publish = (b, n) => n === 1
      ? json({ code: 'signature_required', signature_required: { action: 'design_publish', meaning: 'Publish screen s1' } }, 428)
      : json({ status: 'published', version: version(2, 'v2', { status: 'published', current: true }) })
    const stop = subscribeSignaturePrompt(() => {
      const p = currentSignaturePrompt()
      if (p) answerSignaturePrompt({ password: 'pw', meaning: p.request.meaning, reason: 'go live' })
    })
    try {
      await loaded()
      await waitFor(() => expect(screen.getByTestId('designer-publish')).toBeTruthy(), T)
      fireEvent.click(screen.getByTestId('designer-publish'))
      fireEvent.click(await screen.findByTestId('designer-publish-confirm'))
      await screen.findByTestId('designer-publish-result')
      const posts = calls(/\/publish$/, 'POST')
      expect(posts).toHaveLength(2)
      expect(posts[1].body.signature).toEqual({ password: 'pw', meaning: 'Publish screen s1', reason: 'go live' })
    } finally {
      stop()
    }
  }, 20_000)

  it('no publish UI when publishing is off for the site', async () => {
    server.status = { ...server.status, publishing: false }
    await loaded()
    await waitFor(() => expect(calls(/design-status/).length).toBeGreaterThan(0))
    await new Promise(r => setTimeout(r, 50))
    expect(screen.queryByTestId('designer-publish')).toBeNull()
    expect(screen.queryByTestId('designer-publish-state')).toBeNull()
  }, 20_000)
})

describe('versions', () => {
  it('creates a version with a name and message', async () => {
    await loaded()
    await openTab('versions')
    fireEvent.change(await screen.findByTestId('designer-version-name'), { target: { value: 'before-shutdown' } })
    fireEvent.change(screen.getByTestId('designer-version-message'), { target: { value: 'Last good state' } })
    fireEvent.click(screen.getByTestId('designer-version-create'))
    await waitFor(() => expect(calls(/\/versions$/, 'POST')).toHaveLength(1))
    expect(calls(/\/versions$/, 'POST')[0].body).toEqual({ name: 'before-shutdown', message: 'Last good state' })
    await waitFor(() => expect(screen.getAllByTestId('designer-version').map(v => v.dataset.version)).toEqual(['before-shutdown', 'v1']), T)
  }, 20_000)

  it('compare with the draft opens the diff viewer: both canvases outlined and the YAML line diff', async () => {
    await loaded()
    await openTab('versions')
    fireEvent.click(await screen.findByTestId('designer-version-vs-draft-v1', {}, T))
    const cmp = await screen.findByTestId('designer-compare')
    await waitFor(() => expect(within(cmp).getByTestId('designer-compare-summary').textContent).toBe('1 added · 1 changed'))
    expect(calls(/\/diff\?/)[0].url).toMatch(/from=1&to=draft&yaml=1/)
    expect(within(cmp).getByTestId('designer-compare-title').textContent).toBe('v1 → Draft')
    // Old side: a changed; new side: a changed and b added.
    expect(within(within(cmp).getByTestId('designer-compare-from')).getAllByTestId(/designer-outline-/).map(e => e.getAttribute('data-testid'))).toEqual(['designer-outline-changed'])
    expect(within(within(cmp).getByTestId('designer-compare-to')).getAllByTestId(/designer-outline-/).map(e => e.getAttribute('data-testid')).sort())
      .toEqual(['designer-outline-added', 'designer-outline-changed'])
    const yaml = within(cmp).getByTestId('designer-yaml-diff')
    expect(within(yaml).getAllByTestId('designer-yaml-del').map(e => e.textContent)).toEqual(['3−  x: 100'])
    expect(within(yaml).getAllByTestId('designer-yaml-add').map(e => e.textContent)).toEqual(['3+  x: 140', '4+- id: b', '5+  x: 400'])
    expect(within(cmp).getAllByTestId('designer-compare-object').map(o => `${o.dataset.object}:${o.dataset.kind}`)).toEqual(['b:added', 'a:changed'])
    // Publish this version from the comparison.
    fireEvent.click(within(cmp).getByTestId('designer-compare-publish'))
    const dlg = await screen.findByTestId('designer-publish-dialog')
    expect(screen.queryByTestId('designer-compare')).toBeNull()
    fireEvent.click(within(dlg).getByTestId('designer-publish-confirm'))
    await screen.findByTestId('designer-publish-result')
    expect(calls(/\/publish$/, 'POST')[0].body).toEqual({ version: '1', message: '' })
  }, 20_000)

  it('pending requests: another admin may approve or reject, the requester may not', async () => {
    server.versions = [version(2, 'v2', { status: 'pending', requested_by: 'carol' }), version(1, 'v1')]
    await loaded()
    await openTab('versions')
    fireEvent.click(await screen.findByTestId('designer-version-approve-v2', {}, T))
    await waitFor(() => expect(calls(/\/versions\/2\/approve$/, 'POST')).toHaveLength(1))
    fireEvent.click(screen.getByTestId('designer-version-reject-v2'))
    fireEvent.change(await screen.findByTestId('designer-reject-note'), { target: { value: 'not yet' } })
    fireEvent.click(screen.getByTestId('designer-reject-confirm'))
    await waitFor(() => expect(calls(/\/versions\/2\/reject$/, 'POST')).toHaveLength(1))
    expect(calls(/\/versions\/2\/reject$/, 'POST')[0].body).toEqual({ note: 'not yet' })
  }, 20_000)

  it('the requester sees no Approve or Reject on their own request', async () => {
    server.versions = [version(2, 'v2', { status: 'pending', requested_by: 'admin' })]
    await loaded()
    await openTab('versions')
    await waitFor(() => expect(screen.getAllByTestId('designer-version')).toHaveLength(1), T)
    expect(screen.queryByTestId('designer-version-approve-v2')).toBeNull()
    expect(screen.getByTestId('designer-version').textContent).toContain('another administrator approves it')
  }, 20_000)
})

describe('saving and conflicts', () => {
  it('saves send base_seq and base_revision, marked as made in the Designer', async () => {
    await loaded()
    key('a', { ctrlKey: true })
    key('ArrowRight')
    key('s', { ctrlKey: true })
    await waitFor(() => expect(calls(/scada-layout$/, 'PUT')).toHaveLength(1))
    const put = calls(/scada-layout$/, 'PUT')[0]
    expect(put.body.base_seq).toBe(10)
    expect(put.body.base_revision).toBe('r1')
    expect(put.headers['X-Voltrus-Client']).toBe('designer')
  }, 20_000)

  it('the conflict dialog says who changed what since you loaded; overwrite resends on their seq', async () => {
    await loaded()
    server.put = body => body.base_seq === 10
      ? json({
        error: 'This screen was changed by someone else since you opened it', code: 'conflict', revision: 'r2', seq: 14,
        changes_since: [{ batch: 'B9', username: 'bob', source: 'mcp', operation: 'save', note: 'tuned setpoints', at_ms: NOW - 60_000, objects: [{ object_type: 'widget', object_id: 'P-101', kind: 'update', seq: 14 }] }],
      }, 409)
      : json({ ok: true, revision: 'r3', seq: 15 })
    key('a', { ctrlKey: true })
    key('ArrowRight')
    key('s', { ctrlKey: true })
    const dlg = await screen.findByTestId('designer-conflict-dialog')
    const change = within(dlg).getByTestId('designer-conflict-change')
    expect(change.textContent).toContain('bob via MCP')
    expect(change.textContent).toContain('1 changed: P-101')
    expect(change.textContent).toContain('tuned setpoints')
    fireEvent.click(within(dlg).getByTestId('designer-conflict-overwrite'))
    await waitFor(() => expect(screen.getByTestId('designer-dirty').textContent).toBe('All changes saved'))
    expect(calls(/scada-layout$/, 'PUT').map(c => [c.body.base_revision, c.body.base_seq])).toEqual([['r1', 10], ['r2', 14]])
  }, 20_000)
})

describe('draft and published reads', () => {
  const NONE: any[] = []
  const noSave = async () => {}
  const noClick = () => {}
  function Runtime() {
    const rows = useScadaRows('s1', true, 'scada')
    // The app shell shows the 2D view's toolbar (Edit) in its header.
    const [toolbar, setToolbar] = useState<React.ReactNode>(null)
    return (
      <>
        <div>{toolbar}</div>
        <ScadaView sensors={sensors as any} activeScreen="s1" scadaEquipment={rows.equipment.data ?? NONE} scadaConnections={rows.connections.data ?? NONE}
          loaded={rows.loaded} onSave={noSave} onSensorClick={noClick} isAdmin onEditModeChange={rows.setEditing} toolbarSlotSetter={setToolbar} />
      </>
    )
  }

  it('after Edit → Done on a published screen the draft is announced, with a link to publish it', async () => {
    const openDesigner = vi.fn()
    function Published() {
      const rows = useScadaRows('s1', true, 'scada')
      const [toolbar, setToolbar] = useState<React.ReactNode>(null)
      return (
        <>
          <div>{toolbar}</div>
          <ScadaView sensors={sensors as any} activeScreen="s1" scadaEquipment={rows.equipment.data ?? NONE} scadaConnections={rows.connections.data ?? NONE}
            loaded={rows.loaded} onSave={noSave} onSensorClick={noClick} isAdmin onEditModeChange={rows.setEditing} toolbarSlotSetter={setToolbar}
            onOpenDesigner={openDesigner} />
        </>
      )
    }
    const published = { version_id: 1, name: 'v1', upto_seq: 5, at_ms: NOW, by: 'ana', draft_differs: false }
    server.status = { ...server.status, published }
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(<QueryClientProvider client={qc}><DashboardStoreProvider value={createDashboardStore()}><Published /></DashboardStoreProvider></QueryClientProvider>)
    await waitFor(() => expect(calls(/s1\/design-status/).length).toBeGreaterThan(0), T)
    expect(screen.queryByTestId('scada-draft-differs')).toBeNull()
    const edit = await screen.findByText('Edit')
    await waitFor(() => expect((edit as HTMLButtonElement).disabled).toBe(false), T)
    fireEvent.click(edit)
    await waitFor(() => expect(calls(/scada-equipment$/)).toHaveLength(1), T)
    await waitFor(() => expect(screen.queryByTestId('scada-loading-draft')).toBeNull(), T)
    // The edits were saved to the draft: it differs from what operators see now.
    server.status = { ...server.status, published: { ...published, draft_differs: true } }
    const done = await screen.findByText('Done')
    await waitFor(() => expect((done as HTMLButtonElement).disabled).toBe(false), T)
    fireEvent.click(done)
    await screen.findByText('Edit', {}, T)
    const note = await screen.findByTestId('scada-draft-differs', {}, T)
    expect(note.textContent).toContain('v1')
    fireEvent.click(within(note).getByTestId('scada-draft-publish'))
    expect(openDesigner).toHaveBeenCalled()
    expect(within(note).getByTestId('scada-draft-publish').textContent).toBe('Publish in the Designer')
  }, 20_000)

  it('the 2D runtime reads the published screen; its Edit mode and the Designer read the draft', async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(<QueryClientProvider client={qc}><DashboardStoreProvider value={createDashboardStore()}><Runtime /></DashboardStoreProvider></QueryClientProvider>)
    await waitFor(() => expect(calls(/scada-equipment/)).toHaveLength(1), T)
    expect(calls(/scada-equipment/)[0].url).toMatch(/\/s1\/scada-equipment\?published=1$/)
    expect(calls(/scada-connections/)[0].url).toMatch(/\/s1\/scada-connections\?published=1$/)
    // Edit mode: the draft, a separate cache entry.
    const edit = await screen.findByText('Edit')
    await waitFor(() => expect((edit as HTMLButtonElement).disabled).toBe(false), T)
    fireEvent.click(edit)
    await waitFor(() => expect(calls(/scada-equipment$/)).toHaveLength(1), T)
    expect(calls(/scada-connections$/)).toHaveLength(1)
    // The Designer reads the draft layout.
    setup()
    await waitFor(() => expect(calls(/\/s1\/scada-layout$/).length).toBeGreaterThan(0), T)
    expect(calls(/scada-layout/).every(c => !c.url.includes('published'))).toBe(true)
  }, 20_000)
})
