import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { TemplatesPanel } from '../../src/components/TemplatesPanel'
import { RoleContext } from '../../src/RoleContext'
import { READ_ONLY } from '../../src/roles'

const list = { templates: [{ id: 'wtp', name: 'Water plant', industry: 'water', description: 'd', tag_count: 3, alarm_count: 1, flow_count: 0, has_oee: false }], industries: [{ id: 'water', name: 'Water' }] }
const detail = { id: 'wtp', name: 'Water plant', industry: 'water', description: 'd', equipment_type: 'x', tags: [], alarms: [], flows: [], oee_config: null }

function renderAs(abilities: typeof READ_ONLY) {
  vi.stubGlobal('fetch', vi.fn(async (url: string) => new Response(JSON.stringify(url.endsWith('/wtp') ? detail : list), { status: 200 })))
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(<QueryClientProvider client={qc}><RoleContext.Provider value={abilities}><TemplatesPanel /></RoleContext.Provider></QueryClientProvider>)
}

describe('template preview and the configuration role', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('a viewer can browse but is not offered preview or instantiate', async () => {
    renderAs(READ_ONLY)
    fireEvent.click(await screen.findByRole('button', { name: /Water plant/ }))
    await screen.findByText(/needs the configuration role/)
    expect(screen.queryByRole('button', { name: /Preview/ })).toBeNull()
    expect(screen.getByRole('tab', { name: 'Instantiate' }).getAttribute('aria-disabled')).toBe('true')
  })

  it('a configurer gets the preview', async () => {
    renderAs({ canOperate: true, canConfigure: true, isAdmin: false })
    fireEvent.click(await screen.findByRole('button', { name: /Water plant/ }))
    expect(await screen.findByRole('button', { name: /Preview/ })).toBeTruthy()
  })
})
