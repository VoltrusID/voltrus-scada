import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import { renderHook, waitFor, act } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createDashboardStore, DashboardStoreProvider } from '../../src/store/dashboardStore'
import { useDashboardData } from '../../src/hooks/useDashboardData'
import type { SensorConfig } from '../../src/types'

const sensor = { id: 1, key: 'flow' } as SensorConfig
const sensors = [sensor]

function setup() {
  const store = createDashboardStore()
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}><DashboardStoreProvider value={store}>{children}</DashboardStoreProvider></QueryClientProvider>
  )
  return { store, wrapper }
}

/** A /history answer whose single point carries `value`. */
const answer = (value: number) => new Response(JSON.stringify({ data: { '1': [{ ts: 1000, value }] } }), { status: 200 })

describe('history requests', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('a slow answer for an old range never overwrites the new one', async () => {
    const pending: { url: string; resolve: (r: Response) => void; signal?: AbortSignal | null }[] = []
    vi.stubGlobal('fetch', vi.fn((url: string, init?: RequestInit) => new Promise<Response>((resolve, reject) => {
      if (url.includes('/history?range=30m') && !url.includes('screen')) { resolve(new Response('{"data":{}}')); return }
      init?.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')))
      pending.push({ url, resolve, signal: init?.signal })
    })))
    const { store, wrapper } = setup()
    const { rerender } = renderHook(
      ({ range }) => useDashboardData(false, 'history', 's1', sensors, sensors, range, 'auto', 'sampling', false),
      { wrapper, initialProps: { range: '1h' } },
    )
    await waitFor(() => expect(pending.length).toBe(1))
    rerender({ range: '24h' })
    await waitFor(() => expect(pending.length).toBe(2))
    expect(pending[0].signal?.aborted).toBe(true)
    await act(async () => { pending[1].resolve(answer(24)) })
    await act(async () => { pending[0].resolve(answer(1)) })
    expect(store.getState().historyMap?.['1']?.[0].value).toBe(24)
  })

  it('a failed request is reported and can be retried', async () => {
    let fail = true
    vi.stubGlobal('fetch', vi.fn(async () => fail ? new Response('{}', { status: 500 }) : answer(5)))
    const { store, wrapper } = setup()
    const { result } = renderHook(
      () => useDashboardData(false, 'history', 's1', sensors, sensors, '1h', 'auto', 'sampling', false),
      { wrapper },
    )
    await waitFor(() => expect(result.current.historyError).toMatch(/500/))
    fail = false
    act(() => result.current.retryHistory())
    await waitFor(() => expect(store.getState().historyMap?.['1']?.[0].value).toBe(5))
    expect(result.current.historyError).toBeNull()
  })
})
