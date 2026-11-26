// The 2D view on a phone (docs/designer.md, phase 5): run mode shows the
// generated operator layout; taps open the same faceplates, controls write
// through the same command path and only with the operate permission;
// without the live stream every value shows its age and nothing is shown as
// live; the last-known snapshot and the offline start-up view; the site
// manifest link.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { ScadaView } from '../../src/scada2/ScadaView'
import { OfflineOperator } from '../../src/scada2/mobile/OfflineOperator'
import { createDashboardStore, DashboardStoreProvider } from '../../src/store/dashboardStore'
import { RoleContext } from '../../src/RoleContext'
import { roleAbilities } from '../../src/roles'
import * as ws from '../../src/api/websocket'
import { clearSnapshots, loadSnapshot, setSnapshotUser } from '../../src/scada2/mobile/lastKnown'
import { applySiteManifest, SITE_MANIFEST, STATIC_MANIFEST } from '../../src/sw-register'

const sensors = [
  { id: 1, key: 'p1_run', name: 'P-1 run', device_id: 'wtp', unit: '', min_val: 0, max_val: 1, decimals: 0 },
  { id: 2, key: 'level', name: 'Clearwell level', device_id: 'wtp', unit: '%', min_val: 0, max_val: 100, decimals: 1 },
  { id: 3, key: 'sp', name: 'Setpoint', device_id: 'wtp', unit: '%', min_val: 0, max_val: 100, decimals: 1 },
]
const eq = (id: string, type: string, x: number, y: number, config: Record<string, any>, bindings: number[] = [], w = 120, h = 60) => ({
  screen_id: 's1', equipment_id: id, equipment_type: type, equipment_name: '', x, y, width: w, height: h, rotation: 0,
  sensor_bindings: JSON.stringify(bindings), config: JSON.stringify(config),
})
const ROWS = [
  eq('area1', 'area', 0, 0, { title: 'Clearwell', z: 0 }, [], 900, 500),
  eq('p1', 'symbol', 50, 80, { symbol: 'isa_pump', label: 'P-1', writable: true, z: 1 }, [1]),
  eq('lvl', 'big_number', 300, 80, { label: 'Level', z: 2 }, [2]),
  eq('sp', 'number_input', 500, 80, { label: 'Level setpoint', z: 3 }, [3]),
  eq('hidden', 'big_number', 50, 300, { label: 'Hidden', mobile: { hide: true }, z: 4 }, [2]),
  eq('pipe', 'pipe', 180, 100, { z: 5 }),
]

function narrow(on: boolean) {
  window.matchMedia = ((q: string) => ({
    matches: on && q.includes('max-width'), media: q, onchange: null,
    addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {}, dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia
}

function setup(role = 'operator', opts: { connected?: boolean } = {}) {
  const store = createDashboardStore()
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const now = Date.now()
  store.setWsConnected(opts.connected ?? true)
  store.mergeSensorData([
    { id: 1, points: [], value: { id: 1, key: 'p1_run', value: 1, warn: 0, crit: 0, source_type: 'modbus', status: 'ok', ts_ms: now } },
    { id: 2, points: [], value: { id: 2, key: 'level', value: 72.25, warn: 0, crit: 0, source_type: 'modbus', status: 'ok', ts_ms: now - 30 * 60_000 } },
  ])
  const pair = vi.fn()
  const r = render(
    <QueryClientProvider client={qc}>
      <DashboardStoreProvider value={store}>
        <RoleContext.Provider value={roleAbilities(role)}>
          <ScadaView sensors={sensors as any} activeScreen="s1" scadaEquipment={ROWS as any} scadaConnections={[]} loaded
            onSave={async () => {}} onSensorClick={() => {}} isAdmin={false} screenName="Clearwell" onPairPhone={pair} />
        </RoleContext.Provider>
      </DashboardStoreProvider>
    </QueryClientProvider>,
  )
  return { store, pair, ...r }
}

beforeEach(() => {
  narrow(true)
  sessionStorage.clear()
  localStorage.clear()
  vi.stubGlobal('fetch', vi.fn(async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } })))
})
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); narrow(false) })

describe('the 2D view on a phone', () => {
  it('shows the generated operator layout: section, rows, no decoration, no hidden items', async () => {
    const { pair } = setup()
    const view = await screen.findByTestId('operator-view')
    expect(within(view).getByText('Clearwell', { selector: 'h2' })).toBeTruthy()
    expect(screen.getByTestId('operator-item-p1')).toBeTruthy()
    expect(screen.getByTestId('operator-state-p1').textContent).toContain('Running')
    expect(screen.getByTestId('operator-value-lvl').textContent).toBe('72.3%')
    expect(screen.queryByTestId('operator-item-hidden')).toBeNull()
    expect(screen.queryByTestId('operator-item-pipe')).toBeNull()
    expect(screen.queryByTestId('scada-canvas')).toBeNull()
    expect(screen.getByTestId('operator-live').textContent).toBe('Live')
    // The alarm hint points at the phone app, not at browser notifications.
    fireEvent.click(screen.getByTestId('operator-pair-phone'))
    expect(pair).toHaveBeenCalled()
  })

  it('a value older than a minute shows its age even while live', async () => {
    setup()
    await screen.findByTestId('operator-view')
    expect(screen.getByTestId('operator-value-lvl').dataset.live).toBe('false')
    expect(screen.getByTestId('operator-age-lvl').textContent).toBe('30 min ago')
    expect(screen.getByTestId('operator-state-p1').dataset.live).toBe('true')
  })

  it('tapping equipment opens its faceplate as a sheet; its command goes through the usual write path', async () => {
    const write = vi.spyOn(ws, 'wsWriteTagResult').mockResolvedValue('ok' as any)
    setup('operator')
    fireEvent.click(await screen.findByTestId('operator-item-p1'))
    const sheet = document.querySelector('.vt-operator-sheet')!
    expect(sheet).toBeTruthy()
    expect(within(sheet as HTMLElement).getByText(/P-1/)).toBeTruthy()
    // Arm and confirm, as on the stage.
    fireEvent.click(within(sheet as HTMLElement).getByRole('button', { name: 'STOP' }))
    fireEvent.click(within(sheet as HTMLElement).getByRole('button', { name: 'Confirm STOP' }))
    await waitFor(() => expect(write).toHaveBeenCalledWith(1, 0))
    // Tapping the backdrop closes it.
    fireEvent.click(document.querySelector('.vt-operator-backdrop')!)
    expect(document.querySelector('.vt-operator-sheet')).toBeNull()
  })

  it('controls write only with the operate permission, through the usual write path', async () => {
    const write = vi.spyOn(ws, 'wsWriteTagResult').mockResolvedValue('ok' as any)
    const viewer = setup('viewer')
    // A viewer gets the value but no buttons to change it.
    expect(within(await screen.findByTestId('operator-item-sp')).queryByText('+')).toBeNull()
    expect(write).not.toHaveBeenCalled()
    viewer.unmount()
    setup('operator')
    fireEvent.click(within(await screen.findByTestId('operator-item-sp')).getByText('+'))
    await waitFor(() => expect(write).toHaveBeenCalledWith(3, 1))
  })

  it('without the live stream nothing is shown as live: offline chip, banner, ages', async () => {
    const { store } = setup('operator', { connected: false })
    await screen.findByTestId('operator-view')
    expect(screen.getByTestId('operator-live').textContent).toBe('Offline')
    expect(screen.getByTestId('operator-offline')).toBeTruthy()
    expect(screen.getByTestId('operator-value-lvl').dataset.live).toBe('false')
    expect(screen.getByTestId('operator-state-p1').dataset.live).toBe('false')
    // Back online: live again.
    act(() => store.setWsConnected(true))
    await waitFor(() => expect(screen.getByTestId('operator-live').textContent).toBe('Live'))
    expect(screen.queryByTestId('operator-offline')).toBeNull()
    expect(screen.getByTestId('operator-state-p1').dataset.live).toBe('true')
  })

  it('the operator may switch to the full stage and back', async () => {
    setup()
    fireEvent.click(await screen.findByTestId('operator-show-stage'))
    expect(screen.queryByTestId('operator-view')).toBeNull()
    fireEvent.click(screen.getByTestId('operator-phone-layout'))
    expect(await screen.findByTestId('operator-view')).toBeTruthy()
  })

  it('a wide screen keeps the stage', async () => {
    narrow(false)
    setup()
    await new Promise(r => setTimeout(r, 20))
    expect(screen.queryByTestId('operator-view')).toBeNull()
  })
})

describe('last-known values', () => {
  it('keeps a snapshot while live, for the signed-in user only', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    setSnapshotUser('olga')
    setup()
    await screen.findByTestId('operator-view')
    await act(async () => { vi.advanceTimersByTime(2500) })
    const snap = loadSnapshot(localStorage, 's1', 'olga')
    expect(snap).toBeTruthy()
    expect(snap!.screenName).toBe('Clearwell')
    expect(snap!.values['2'].value).toBe(72.25)
    expect(snap!.sensors.map(s => s.id).sort()).toEqual([1, 2, 3])
    expect(loadSnapshot(localStorage, 's1', 'someone-else')).toBeNull()
    clearSnapshots(localStorage)
    expect(loadSnapshot(localStorage)).toBeNull()
    setSnapshotUser('')
  })

  it('the offline start-up view shows the snapshot as not live, with its age, and nothing to tap', async () => {
    const now = Date.now()
    render(
      <DashboardStoreProvider value={createDashboardStore()}>
        <OfflineOperator onRetry={() => {}} onSignIn={() => {}} snapshot={{
          screen: 's1', screenName: 'Clearwell', savedAt: now - 5 * 60_000, equipment: ROWS as any, connections: [],
          sensors: sensors as any, values: { 1: { value: 1, ts_ms: now - 5 * 60_000 }, 2: { value: 64, ts_ms: now - 6 * 60_000 } },
        }} />
      </DashboardStoreProvider>,
    )
    await screen.findByTestId('offline-operator')
    expect(screen.getByTestId('operator-live').textContent).toBe('Offline')
    expect(screen.getByTestId('operator-offline').textContent).toContain('5 min ago')
    expect(screen.getByTestId('operator-value-lvl').dataset.live).toBe('false')
    expect(screen.getByTestId('operator-age-lvl').textContent).toBe('6 min ago')
    expect(screen.getByTestId('operator-item-p1').tagName).toBe('DIV')
  })
})

describe('site manifest', () => {
  it('links the static manifest before sign-in and the site manifest, with credentials, after', () => {
    document.head.innerHTML = '<link rel="manifest" href="/manifest.json">'
    applySiteManifest(document, true)
    const link = document.querySelector('link[rel="manifest"]')!
    expect(link.getAttribute('href')).toBe(SITE_MANIFEST)
    expect(link.getAttribute('crossorigin')).toBe('use-credentials')
    applySiteManifest(document, false)
    expect(link.getAttribute('href')).toBe(STATIC_MANIFEST)
    expect(link.hasAttribute('crossorigin')).toBe(false)
    document.head.innerHTML = ''
    applySiteManifest(document, true)
    expect(document.querySelector('link[rel="manifest"]')!.getAttribute('href')).toBe(SITE_MANIFEST)
  })
})
