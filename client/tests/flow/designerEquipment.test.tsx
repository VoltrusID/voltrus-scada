// Equipment types in the Designer: the palette lists the server's types,
// dropping one opens the instance picker, creating an instance previews it
// with a dry run and places a symbol bound exactly as the server says, and
// the property editor shows the instance of a selected symbol.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from '@tanstack/react-router'
import { Designer } from '../../src/designer/Designer'
import { createDashboardStore, DashboardStoreProvider } from '../../src/store/dashboardStore'

const sensors = [
  { id: 7, key: 'p101a_run', name: 'Intake Pump P-101A', device_id: 'wtp', unit: '', min_val: 0, max_val: 1, decimals: 0 },
  { id: 8, key: 'p101b_run', name: 'Intake Pump P-101B', device_id: 'wtp', unit: '', min_val: 0, max_val: 1, decimals: 0 },
  { id: 9, key: 'p101b_fault', name: 'P-101B Trip', device_id: 'wtp', unit: '', min_val: 0, max_val: 1, decimals: 0 },
]
const pumpType = {
  id: 'pump', name: 'Pump', category: 'Pumps', symbol: 'isa_pump', faceplate: 'pump', instance_count: 1,
  tags: [
    { role: 'run', kind: 'state', key: '{prefix}_run', required: true },
    { role: 'fault', kind: 'fault', key: '{prefix}_fault' },
    { role: 'cmd', kind: 'command', key: '{prefix}_cmd', fallback: 'run' },
  ],
  bindings: { primary: 'run', fault: 'fault' }, symbol_config: { showStatus: true, writable: true },
}
const resolved = (name: string, prefix: string, run: number, fault: number | null) => ({
  prefix, symbol: 'isa_pump', faceplate: 'pump',
  roles: [
    { role: 'run', kind: 'state', key: `${prefix}_run`, ref: `wtp/${prefix}_run`, tag_id: run, status: 'bound' },
    { role: 'fault', kind: 'fault', key: `${prefix}_fault`, ref: `wtp/${prefix}_fault`, tag_id: fault, status: fault == null ? 'missing' : 'bound' },
    { role: 'cmd', kind: 'command', key: `${prefix}_cmd`, ref: `wtp/${prefix}_run`, tag_id: run, status: 'fallback' },
  ],
  widget: {
    equipment_type: 'symbol', sensor_id: run,
    config: {
      symbol: 'isa_pump', faceplate: 'pump', showStatus: true, writable: true,
      faceplateMap: fault == null ? { run, cmd: run } : { run, fault, cmd: run }, ...(fault == null ? {} : { faultSensorId: fault }),
    },
    remove: fault == null ? ['faultSensorId', 'runSensorId', 'manualSensorId', 'interlockSensorId'] : ['runSensorId', 'manualSensorId', 'interlockSensorId'],
  },
})
const p101a = { name: 'P-101A', type: 'pump', device: 'wtp', base: null, prefix: 'p101a', overrides: {}, resolved: resolved('P-101A', 'p101a', 7, null) }

interface Server { puts: any[]; posts: { url: string; body: any }[]; instances: any[] }
let server: Server

function stubServer() {
  server = { puts: [], posts: [], instances: [p101a] }
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const u = String(url)
    const method = init?.method ?? 'GET'
    const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { 'Content-Type': 'application/json' } })
    if (u.endsWith('/api/v1/screens')) return json([{ id: 's1', name: 'Pumps' }])
    if (u.endsWith('/api/v1/data-sources')) return json(sensors)
    if (u.endsWith('/api/v1/faceplates')) return json([])
    if (u.endsWith('/api/v1/modbus-devices')) return json([{ id: 'wtp', name: 'WTP PLC' }])
    if (u.endsWith('/api/v1/equipment-types')) return json([pumpType])
    if (u.endsWith('/api/v1/equipment-instances') && method === 'GET') return json(server.instances)
    if (u.includes('/api/v1/equipment-instances?dry_run=')) {
      const body = JSON.parse(String(init!.body))
      server.posts.push({ url: u, body })
      const inst = { ...body, resolved: resolved(body.name, body.prefix ?? body.name.toLowerCase().replace(/[^a-z0-9]/g, ''), 8, 9), changes: { tags: [], registers_added: [], flows: [], widgets: [] } }
      if (u.endsWith('dry_run=false')) server.instances = [...server.instances, inst]
      return json(inst, u.endsWith('dry_run=false') ? 201 : 200)
    }
    const m = /\/api\/v1\/screens\/([^/]+)\/scada-layout$/.exec(u)
    if (m && method === 'GET') return json({ equipment: [], connections: [], revision: 'r1' })
    if (m && method === 'PUT') {
      server.puts.push(JSON.parse(String(init!.body)))
      return json({ ok: true, revision: 'r2' })
    }
    return json([])
  }))
}

function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const root = createRootRoute({ component: () => <Designer screen="s1" onOpenScreen={() => {}} onClose={() => {}} onOpenPage={() => {}} /> })
  const routes = ['/', '/designer'].map(p => createRoute({ getParentRoute: () => root, path: p, component: () => null }))
  const router = createRouter({ routeTree: root.addChildren(routes), history: createMemoryHistory({ initialEntries: ['/designer?screen=s1'] }) })
  render(<QueryClientProvider client={qc}><DashboardStoreProvider value={createDashboardStore()}><RouterProvider router={router as any} /></DashboardStoreProvider></QueryClientProvider>)
}

const key = (k: string, opts: KeyboardEventInit = {}) => act(() => { fireEvent.keyDown(document.body, { key: k, ...opts }) })

async function openPicker() {
  setup()
  const tile = await screen.findByTestId('designer-equipment-pump', {}, { timeout: 10_000 })
  fireEvent.doubleClick(tile)
  return screen.findByTestId('designer-instance-picker')
}

const savedRow = async () => {
  key('s', { ctrlKey: true })
  await waitFor(() => expect(server.puts).toHaveLength(1))
  const rows = server.puts[0].equipment
  expect(rows).toHaveLength(1)
  return { ...rows[0], config: JSON.parse(rows[0].config), sensor_bindings: JSON.parse(rows[0].sensor_bindings) }
}

beforeEach(() => stubServer())
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks() })

describe('equipment types in the Designer', () => {
  it('places an existing instance as a symbol bound to its tags and faceplate', async () => {
    const dlg = await openPicker()
    // With instances, the picker opens on the list.
    fireEvent.click(within(dlg).getByTestId('designer-instance-existing-P-101A'))
    await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(1))
    expect(screen.queryByTestId('designer-instance-picker')).toBeNull()
    const row = await savedRow()
    expect(row.equipment_type).toBe('symbol')
    expect(row.sensor_bindings).toEqual([7])
    expect(row.config).toMatchObject({ equipment: 'P-101A', symbol: 'isa_pump', faceplate: 'pump', faceplateMap: { run: 7, cmd: 7 }, label: 'P-101A', showStatus: true })
    expect(row.config.faultSensorId).toBeUndefined()
  })

  it('creates a new instance after a dry-run preview and places it', async () => {
    const dlg = await openPicker()
    fireEvent.click(within(dlg).getByTestId('designer-instance-tab-new'))
    const name = within(dlg).getByTestId('designer-instance-name') as HTMLInputElement
    // The next free name after P-101A.
    expect(name.value).toBe('P-101B')
    expect((within(dlg).getByTestId('designer-instance-device') as HTMLSelectElement).value).toBe('wtp')
    // The dry run lists what the instance binds.
    await waitFor(() => expect(within(dlg).getAllByTestId('designer-instance-role')).toHaveLength(3), { timeout: 5000 })
    expect(server.posts.at(-1)!.url).toMatch(/dry_run=true$/)
    expect(server.posts.at(-1)!.body).toEqual({ name: 'P-101B', type: 'pump', device: 'wtp', base: null, prefix: null })
    fireEvent.click(within(dlg).getByTestId('designer-instance-create'))
    await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(1))
    expect(server.posts.at(-1)!.url).toMatch(/dry_run=false$/)
    const row = await savedRow()
    expect(row.sensor_bindings).toEqual([8])
    expect(row.config).toMatchObject({ equipment: 'P-101B', faultSensorId: 9, faceplateMap: { run: 8, fault: 9, cmd: 8 } })
  })

  it('refuses a name that exists and an address that is not a number', async () => {
    const dlg = await openPicker()
    fireEvent.click(within(dlg).getByTestId('designer-instance-tab-new'))
    fireEvent.change(within(dlg).getByTestId('designer-instance-name'), { target: { value: 'p-101a' } })
    fireEvent.change(within(dlg).getByTestId('designer-instance-base'), { target: { value: '4x' } })
    expect(within(dlg).getByText(/exists; pick it from the list/)).toBeTruthy()
    expect(within(dlg).getByText(/Base must be a whole register address/)).toBeTruthy()
    expect((within(dlg).getByTestId('designer-instance-create') as HTMLButtonElement).disabled).toBe(true)
    // Escape closes the picker and places nothing.
    key('Escape')
    await waitFor(() => expect(screen.queryByTestId('designer-instance-picker')).toBeNull())
    expect(screen.queryAllByTestId('designer-widget')).toHaveLength(0)
  })

  it('shows the instance, its type and tags per role for a selected symbol', async () => {
    const dlg = await openPicker()
    fireEvent.click(within(dlg).getByTestId('designer-instance-existing-P-101A'))
    await waitFor(() => expect(screen.getAllByTestId('designer-widget')).toHaveLength(1))
    // Placing selects the new symbol.
    const section = await screen.findByTestId('designer-props-equipment')
    expect(within(section).getByTestId('designer-equipment-name').textContent).toBe('P-101A')
    expect(within(section).getByTestId('designer-equipment-type').textContent).toBe('Pump')
    const roles = within(section).getAllByTestId('designer-equipment-role').map(r => [r.getAttribute('data-role'), r.getAttribute('data-status')])
    expect(roles).toEqual([['run', 'bound'], ['fault', 'missing'], ['cmd', 'fallback']])
    // The project tree lists the instance with its count on this screen.
    fireEvent.click(within(screen.getByTestId('designer-equipment-tree')).getByText('Equipment'))
    fireEvent.click(screen.getByTestId('designer-equipment-type-pump'))
    expect(screen.getByTestId('designer-equipment-instance-P-101A').textContent).toContain('×1')
  })
})
