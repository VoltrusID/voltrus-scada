import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'

vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => () => {},
  useRouterState: () => ({}),
}))

import { AlarmPanel } from '../../src/AlarmPanel'
import { UnifiedBell } from '../../src/components/UnifiedBell'

function wrap(children: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>
}

describe('alarms the server cannot deliver', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('the panel says alarms are unavailable instead of "No alarms"', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 502 })))
    render(wrap(<AlarmPanel onClose={() => {}} />))
    await waitFor(() => expect(screen.getAllByRole('alert').some(a => a.textContent?.includes('Alarms unavailable'))).toBe(true))
    expect(screen.queryByText('No alarms')).toBeNull()
  })

  it('the panel still says "No alarms" when the server answers with none', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => new Response(url.includes('summary') ? '{"active":0}' : '[]', { status: 200 })))
    render(wrap(<AlarmPanel onClose={() => {}} />))
    await waitFor(() => expect(screen.getByText('No alarms')).toBeTruthy())
  })

  it('the bell shows a warning badge, not a calm zero', () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('[]', { status: 200 })))
    render(wrap(<UnifiedBell authed activeAlarmCount={0} alarmsUnavailable onOpenAlarmPanel={() => {}} />))
    const bell = screen.getByRole('button', { name: /alarms unavailable/i })
    expect(bell.textContent).toContain('!')
    fireEvent.click(bell)
    expect(screen.getByRole('alert').textContent).toContain('Alarms unavailable')
    expect(screen.queryByText('No active alarms')).toBeNull()
  })
})
