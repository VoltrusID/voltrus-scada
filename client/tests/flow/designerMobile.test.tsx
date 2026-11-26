// The Designer's Mobile pane (docs/designer.md, phase 5): the generated
// phone layout, hiding, moving and half width as undoable edits of
// `config.mobile`, saved with the base revision like any other change, and
// regeneration keeping them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider, useNavigate, useRouterState } from '@tanstack/react-router'
import { Designer } from '../../src/designer/Designer'
import { createDashboardStore, DashboardStoreProvider } from '../../src/store/dashboardStore'

const screens = [{ id: 's1', name: 'Pumps' }]
const sensors = [
  { id: 1, key: 'p1_run', name: 'P-1 run', device_id: 'wtp', unit: '', min_val: 0, max_val: 1, decimals: 0 },
  { id: 2, key: 'level', name: 'Level', device_id: 'wtp', unit: '%', min_val: 0, max_val: 100, decimals: 1 },
]
const eq = (id: string, type: string, x: number, y: number, config: Record<string, any>, bindings: number[] = [], w = 120, h = 60) => ({
  screen_id: 's1', equipment_id: id, equipment_type: type, equipment_name: '', x, y, width: w, height: h, rotation: 0,
  sensor_bindings: JSON.stringify(bindings), config: JSON.stringify(config),
})
const ROWS = [
  eq('area1', 'area', 0, 0, { title: 'Intake', z: 0 }, [], 900, 500),
  eq('p1', 'symbol', 50, 80, { symbol: 'isa_pump', label: 'P-1', z: 1 }, [1]),
  eq('lvl', 'big_number', 300, 80, { label: 'Level', z: 2 }, [2]),
  eq('tr', 'trend', 50, 250, { label: 'Level trend', z: 3 }, [2], 500, 160),
  eq('note', 'label', 600, 80, { text: 'Static', z: 4 }),
  eq('pipe', 'pipe', 180, 100, { z: 5 }),
]

let puts: any[] = []

beforeEach(() => {
  puts = []
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const u = String(url)
    const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { 'Content-Type': 'application/json' } })
    if (u.endsWith('/api/v1/screens')) return json(screens)
    if (u.endsWith('/api/v1/data-sources')) return json(sensors)
    if (u.endsWith('/api/v1/faceplates')) return json([])
    if (/\/scada-layout$/.test(u) && (init?.method ?? 'GET') === 'GET') return json({ equipment: ROWS, connections: [], revision: 'r1', seq: 4 })
    if (/\/scada-layout$/.test(u) && init?.method === 'PUT') {
      puts.push(JSON.parse(String(init.body)))
      return json({ ok: true, revision: 'r2', seq: 5 })
    }
    return json([])
  }))
})
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks() })

function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  function Host() {
    const navigate = useNavigate()
    const loc = useRouterState({ select: s => s.location })
    return loc.pathname === '/designer'
      ? <Designer screen="s1" onOpenScreen={() => {}} onClose={() => navigate({ to: '/' })} onOpenPage={() => {}} />
      : null
  }
  const root = createRootRoute({ component: Host })
  const routes = ['/', '/designer'].map(p => createRoute({ getParentRoute: () => root, path: p, component: () => null }))
  const router = createRouter({ routeTree: root.addChildren(routes), history: createMemoryHistory({ initialEntries: ['/designer?screen=s1'] }) })
  render(<QueryClientProvider client={qc}><DashboardStoreProvider value={createDashboardStore()}><RouterProvider router={router as any} /></DashboardStoreProvider></QueryClientProvider>)
}

const key = (k: string, opts: KeyboardEventInit = {}) => act(() => { fireEvent.keyDown(document.body, { key: k, ...opts }) })
const itemOrder = () => screen.getAllByTestId(/^designer-mobile-item-/).map(e => e.dataset.testid!.replace('designer-mobile-item-', ''))
const savedMobile = (put: any) => Object.fromEntries(put.equipment.map((e: any) => [e.equipment_id, JSON.parse(e.config).mobile]).filter(([, m]: any) => m !== undefined))

async function openPane() {
  setup()
  await waitFor(() => expect(screen.getAllByTestId('designer-widget').length).toBeGreaterThan(0), { timeout: 10_000 })
  fireEvent.click(screen.getByTestId('designer-tab-mobile'))
  await screen.findByTestId('designer-mobile')
}

describe('Designer Mobile pane', () => {
  it('lists the generated layout: areas as sections, reading order, decoration dropped, plain labels hidden', async () => {
    await openPane()
    const section = screen.getByTestId('designer-mobile-section-area1')
    expect(within(section).getByText('Intake')).toBeTruthy()
    expect(itemOrder()).toEqual(['p1', 'lvl', 'note', 'tr'])
    expect(screen.queryByTestId('designer-mobile-item-pipe')).toBeNull()
    expect(screen.getByTestId('designer-mobile-item-note').dataset.hidden).toBe('true')
    expect(screen.getByTestId('designer-mobile-adjusted').textContent).toBe('0 adjusted')
  })

  it('hide, move and half width are one undo step each and are saved as config.mobile with the base revision', async () => {
    await openPane()
    fireEvent.click(screen.getByTestId('designer-mobile-hide-lvl'))
    expect(screen.getByTestId('designer-mobile-item-lvl').dataset.hidden).toBe('true')
    fireEvent.click(screen.getByTestId('designer-mobile-up-tr'))
    expect(itemOrder()).toEqual(['p1', 'lvl', 'tr', 'note'])
    fireEvent.click(screen.getByTestId('designer-mobile-span-p1'))
    expect(screen.getByTestId('designer-mobile-span-p1').textContent).toBe('½')
    expect(screen.getByTestId('designer-mobile-adjusted').textContent).toBe('4 adjusted')

    // Undo takes back the half width only.
    key('z', { ctrlKey: true })
    expect(screen.getByTestId('designer-mobile-span-p1').textContent).toBe('1')
    key('y', { ctrlKey: true })

    key('s', { ctrlKey: true })
    await waitFor(() => expect(puts).toHaveLength(1))
    expect(puts[0].base_revision).toBe('r1')
    expect(savedMobile(puts[0])).toEqual({
      p1: { order: 0, span: 1 },
      lvl: { hide: true, order: 1 },
      tr: { order: 2 },
      note: { order: 3 },
    })
    // Nothing else of those rows changed.
    const p1 = JSON.parse(puts[0].equipment.find((e: any) => e.equipment_id === 'p1').config)
    expect(p1.symbol).toBe('isa_pump')
    expect(p1.label).toBe('P-1')
  })

  it('moving back to the generated order and showing again remove the key; Reset removes every adjustment', async () => {
    await openPane()
    fireEvent.click(screen.getByTestId('designer-mobile-up-tr'))
    fireEvent.click(screen.getByTestId('designer-mobile-down-tr'))
    expect(screen.getByTestId('designer-mobile-adjusted').textContent).toBe('0 adjusted')
    fireEvent.click(screen.getByTestId('designer-mobile-hide-note'))
    expect(screen.getByTestId('designer-mobile-item-note').dataset.hidden).toBe('false')
    fireEvent.click(screen.getByTestId('designer-mobile-hide-area1'))
    expect(screen.getByTestId('designer-mobile-adjusted').textContent).toBe('2 adjusted')
    fireEvent.click(screen.getByTestId('designer-mobile-reset'))
    expect(screen.getByTestId('designer-mobile-adjusted').textContent).toBe('0 adjusted')
    expect(screen.getByTestId('designer-mobile-item-note').dataset.hidden).toBe('true')
  })

  it('keeps adjustments when the screen changes: a moved component keeps its override, a deleted one leaves nothing behind', async () => {
    await openPane()
    fireEvent.click(screen.getByTestId('designer-mobile-hide-lvl'))
    // Select the trend in the pane and delete it from the stage.
    fireEvent.click(within(screen.getByTestId('designer-mobile-item-tr')).getByText('Level trend'))
    key('Delete')
    await waitFor(() => expect(screen.queryByTestId('designer-mobile-item-tr')).toBeNull())
    expect(screen.getByTestId('designer-mobile-item-lvl').dataset.hidden).toBe('true')
    key('s', { ctrlKey: true })
    await waitFor(() => expect(puts).toHaveLength(1))
    expect(savedMobile(puts[0])).toEqual({ lvl: { hide: true } })
  })

  it('the preview draws the operator view in a phone frame', async () => {
    await openPane()
    fireEvent.click(screen.getByTestId('designer-mobile-preview'))
    const frame = await screen.findByTestId('designer-mobile-phone')
    expect(within(frame).getByTestId('operator-view')).toBeTruthy()
    expect(within(frame).getByTestId('operator-item-p1')).toBeTruthy()
    expect(within(frame).queryByTestId('operator-item-note')).toBeNull()
  })
})
