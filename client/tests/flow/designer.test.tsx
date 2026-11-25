// The Designer in a real router: load, save with the base revision, the
// conflict dialog on a 409, the unsaved-changes guard on navigation, its
// keyboard (no app shortcut leaks in), and who may open /designer.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import {
  createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider, useNavigate, useRouterState,
} from '@tanstack/react-router'
import { Designer } from '../../src/designer/Designer'
import { useKeyboardNavigation } from '../../src/hooks/useKeyboardNavigation'
import { useDesignerGuard } from '../../src/hooks/useDesignerGuard'
import { createDashboardStore, DashboardStoreProvider } from '../../src/store/dashboardStore'

const screens = [{ id: 's1', name: 'Pumps' }, { id: 's2', name: 'Tanks' }]
const sensors = [{ id: 1, key: 'level', name: 'Level', device_id: 'wtp', unit: '%', min_val: 0, max_val: 100, decimals: 1 }]
const row = (id: string, x: number) => ({
  screen_id: 's1', equipment_id: id, equipment_type: 'label', equipment_name: '', x, y: 100, width: 120, height: 40, rotation: 0,
  sensor_bindings: '[]', config: JSON.stringify({ text: id }),
})

interface Server { revision: string; puts: any[]; respond: (body: any) => Response }
let server: Server

function stubServer() {
  server = {
    revision: 'r1',
    puts: [],
    respond: () => new Response(JSON.stringify({ ok: true, revision: 'r-saved' }), { status: 200 }),
  }
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const u = String(url)
    const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { 'Content-Type': 'application/json' } })
    if (u.endsWith('/api/v1/screens')) return json(screens)
    if (u.endsWith('/api/v1/data-sources')) return json(sensors)
    if (u.endsWith('/api/v1/faceplates')) return json([])
    const m = /\/api\/v1\/screens\/([^/]+)\/scada-layout$/.exec(u)
    if (m && (init?.method ?? 'GET') === 'GET') return json({ equipment: m[1] === 's1' ? [row('a', 100), row('b', 400)] : [], connections: [], revision: server.revision })
    if (m && init?.method === 'PUT') {
      const body = JSON.parse(String(init.body))
      server.puts.push(body)
      return server.respond(body)
    }
    return json([])
  }))
}

function setup(path = '/designer?screen=s1') {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  function Host() {
    const navigate = useNavigate()
    const loc = useRouterState({ select: s => s.location })
    const search = loc.search as { screen?: string }
    return (
      <div>
        <div data-testid="where">{loc.pathname}?{search.screen ?? ''}</div>
        {loc.pathname === '/designer' && (
          <Designer
            screen={search.screen ?? 'all'}
            onOpenScreen={id => navigate({ to: '/designer', search: { screen: id } })}
            onClose={() => navigate({ to: '/', search: { screen: search.screen } })}
            onOpenPage={page => navigate({ to: `/${page}` as '/' })}
          />
        )}
      </div>
    )
  }
  const root = createRootRoute({ component: Host })
  const routes = ['/', '/designer', '/tags'].map(p => createRoute({ getParentRoute: () => root, path: p, component: () => null }))
  const router = createRouter({ routeTree: root.addChildren(routes), history: createMemoryHistory({ initialEntries: [path] }) })
  render(<QueryClientProvider client={qc}><DashboardStoreProvider value={createDashboardStore()}><RouterProvider router={router as any} /></DashboardStoreProvider></QueryClientProvider>)
  return { router, qc }
}

const key = (k: string, opts: KeyboardEventInit = {}) => act(() => { fireEvent.keyDown(document.body, { key: k, ...opts }) })
const where = () => screen.getByTestId('where').textContent
const dirtyText = () => screen.getByTestId('designer-dirty').textContent

async function loadedDesigner(path?: string) {
  const r = setup(path)
  await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(2), { timeout: 10_000 })
  return r
}

/** Select everything and nudge it: an unsaved change. */
function makeChange() {
  key('a', { ctrlKey: true })
  key('ArrowRight')
  expect(dirtyText()).toBe('Unsaved changes')
}

beforeEach(() => stubServer())
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks() })

describe('Designer load and save', () => {
  it('saves equipment and connections in one request based on the loaded revision', async () => {
    await loadedDesigner()
    makeChange()
    key('s', { ctrlKey: true })
    await waitFor(() => expect(dirtyText()).toBe('All changes saved'))
    expect(server.puts).toHaveLength(1)
    expect(server.puts[0].base_revision).toBe('r1')
    expect(server.puts[0].equipment.map((e: any) => [e.equipment_id, e.x])).toEqual([['a', 101], ['b', 401]])
    expect(server.puts[0].connections).toEqual([])
    // The next save is based on the revision the server returned.
    key('ArrowRight')
    key('s', { ctrlKey: true })
    await waitFor(() => expect(server.puts).toHaveLength(2))
    expect(server.puts[1].base_revision).toBe('r-saved')
  })

  it('Ctrl+S with nothing to save sends nothing', async () => {
    await loadedDesigner()
    key('s', { ctrlKey: true })
    await new Promise(r => setTimeout(r, 20))
    expect(server.puts).toHaveLength(0)
  })

  it('a 409 opens the conflict dialog; overwriting resends based on the revision the server reported', async () => {
    await loadedDesigner()
    server.respond = body => body.base_revision === 'r1'
      ? new Response(JSON.stringify({ error: 'This screen was changed by someone else since you opened it', code: 'conflict', revision: 'r2' }), { status: 409 })
      : new Response(JSON.stringify({ ok: true, revision: 'r3' }), { status: 200 })
    makeChange()
    key('s', { ctrlKey: true })
    await screen.findByTestId('designer-conflict-dialog')
    expect(dirtyText()).toBe('Unsaved changes')
    fireEvent.click(screen.getByTestId('designer-conflict-overwrite'))
    await waitFor(() => expect(dirtyText()).toBe('All changes saved'))
    expect(server.puts.map(p => p.base_revision)).toEqual(['r1', 'r2'])
  })

  it('a 409 answered with "Load theirs" drops the local changes and loads the server version', async () => {
    await loadedDesigner()
    server.respond = () => new Response(JSON.stringify({ error: 'changed', code: 'conflict', revision: 'r2' }), { status: 409 })
    makeChange()
    key('s', { ctrlKey: true })
    await screen.findByTestId('designer-conflict-dialog')
    server.revision = 'r2'
    fireEvent.click(screen.getByTestId('designer-conflict-reload'))
    await waitFor(() => expect(dirtyText()).toBe('All changes saved'))
    expect(screen.queryByTestId('designer-conflict-dialog')).toBeNull()
    expect(server.puts).toHaveLength(1)
  })
})

describe('a save made elsewhere', () => {
  it('while editing: a banner offers their version instead of silently replacing the work', async () => {
    const { qc } = await loadedDesigner()
    makeChange()
    server.revision = 'r9'
    await act(async () => { await qc.invalidateQueries({ queryKey: ['scadaLayout', 's1'] }) })
    await screen.findByTestId('designer-remote-changed')
    expect(dirtyText()).toBe('Unsaved changes')
    fireEvent.click(screen.getByTestId('designer-remote-reload'))
    await waitFor(() => expect(dirtyText()).toBe('All changes saved'))
    expect(screen.queryByTestId('designer-remote-changed')).toBeNull()
  })

  it('without local changes: the new version just loads', async () => {
    const { qc } = await loadedDesigner()
    server.revision = 'r9'
    await act(async () => { await qc.invalidateQueries({ queryKey: ['scadaLayout', 's1'] }) })
    expect(screen.queryByTestId('designer-remote-changed')).toBeNull()
  })
})

describe('leaving with unsaved changes', () => {
  it('Close asks; Cancel stays, Discard leaves', async () => {
    await loadedDesigner()
    makeChange()
    fireEvent.click(screen.getByTestId('designer-close'))
    await screen.findByTestId('designer-leave-dialog')
    fireEvent.click(screen.getByTestId('designer-leave-cancel'))
    await waitFor(() => expect(screen.queryByTestId('designer-leave-dialog')).toBeNull())
    expect(where()).toBe('/designer?s1')
    fireEvent.click(screen.getByTestId('designer-close'))
    await screen.findByTestId('designer-leave-dialog')
    fireEvent.click(screen.getByTestId('designer-leave-discard'))
    await waitFor(() => expect(where()).toBe('/?s1'))
    expect(server.puts).toHaveLength(0)
  })

  it('switching screens or following a link asks too; Save saves, then goes', async () => {
    const { router } = await loadedDesigner()
    makeChange()
    // Any navigation (Back, Project pane links) is guarded, not only the Designer's own buttons.
    act(() => { void router.navigate({ to: '/tags' }) })
    await screen.findByTestId('designer-leave-dialog')
    fireEvent.click(screen.getByTestId('designer-leave-cancel'))
    fireEvent.click(screen.getByText('Tanks'))
    await screen.findByTestId('designer-leave-dialog')
    fireEvent.click(screen.getByTestId('designer-leave-save'))
    await waitFor(() => expect(where()).toBe('/designer?s2'))
    expect(server.puts).toHaveLength(1)
    expect(server.puts[0].base_revision).toBe('r1')
  })

  it('without changes nothing asks', async () => {
    await loadedDesigner()
    fireEvent.click(screen.getByTestId('designer-close'))
    await waitFor(() => expect(where()).toBe('/?s1'))
  })
})

describe('keyboard', () => {
  it('Ctrl+0 is Fit inside the Designer and does not navigate; Ctrl+P is left to the browser', async () => {
    await loadedDesigner()
    makeChange()
    key('0', { ctrlKey: true })
    key('2', { ctrlKey: true })
    expect(where()).toBe('/designer?s1')
    expect(screen.queryByTestId('designer-leave-dialog')).toBeNull()
    const ev = new KeyboardEvent('keydown', { key: 'p', ctrlKey: true, bubbles: true, cancelable: true })
    act(() => { document.body.dispatchEvent(ev) })
    expect(ev.defaultPrevented).toBe(false)
    expect(screen.queryByTestId('designer-preview')).toBeNull()
    key('F5')
    expect(screen.getByTestId('designer-preview')).toBeTruthy()
  })

  it('the app navigation shortcuts are off while the Designer is open', () => {
    const setActiveScreen = vi.fn(), setMode = vi.fn(), setSelectedSensor = vi.fn()
    const args = (enabled: boolean) => ({
      shortcuts: {}, sensors: [], screens: [{ id: 's1' }], navShortcuts: {},
      effectiveNavShortcuts: { 'screen:system': 'Mod+0', 'screen:s1': 'Mod+2' },
      setSelectedSensor, setActiveScreen, setViewMode: vi.fn(), setMode, setShowAlarmPanel: vi.fn(), mode: 'live' as const, enabled,
    })
    const { rerender } = renderHook(({ enabled }) => useKeyboardNavigation(args(enabled)), { initialProps: { enabled: false } })
    fireEvent.keyDown(document.body, { key: '0', ctrlKey: true })
    fireEvent.keyDown(document.body, { key: '2', ctrlKey: true })
    expect(setMode).not.toHaveBeenCalled()
    expect(setActiveScreen).not.toHaveBeenCalled()
    rerender({ enabled: true })
    fireEvent.keyDown(document.body, { key: '0', ctrlKey: true })
    expect(setMode).toHaveBeenCalledWith('system')
  })
})

describe('who may open /designer', () => {
  it('a viewer is sent to the 2D view; a configurer stays; nothing happens before the session is known', () => {
    const leave = vi.fn()
    const { rerender } = renderHook(p => useDesignerGuard(p), {
      initialProps: { routePage: 'designer', sessionKnown: false, canConfigure: false, leave },
    })
    expect(leave).not.toHaveBeenCalled()
    rerender({ routePage: 'designer', sessionKnown: true, canConfigure: true, leave })
    expect(leave).not.toHaveBeenCalled()
    rerender({ routePage: 'designer', sessionKnown: true, canConfigure: false, leave })
    expect(leave).toHaveBeenCalledTimes(1)
    rerender({ routePage: '', sessionKnown: true, canConfigure: false, leave })
    expect(leave).toHaveBeenCalledTimes(1)
  })
})
