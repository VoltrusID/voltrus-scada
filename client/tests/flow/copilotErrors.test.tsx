import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'

import { DashboardLayoutWizard } from '../../src/copilot/DashboardLayoutWizard'
import { TemplateWizard } from '../../src/copilot/TemplateWizard'
import { ProposalReview } from '../../src/copilot/ProposalReview'
import type { AiProposal } from '../../src/types'

function wrap(children: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>
}

const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json', ...headers } })

function proposal(over: Partial<AiProposal>): AiProposal {
  return {
    id: 1, proposal_type: 'alarm_thresholds', title: 'Thresholds', description: '', status: 'pending',
    proposed_config: [], applied_config: null, applied_at: null, reviewed_at: null, reviewed_by: null,
    model_used: null, tokens_used: null, error_message: null, created_at: 0, updated_at: 0, ...over,
  }
}

describe('copilot wizards show why generating failed', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('a rate limit says when to try again, and no proposal is opened', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      json(429, { error: 'quota', code: 'ai_rate_limited', retry_after_secs: 20 }, { 'Retry-After': '20' })))
    const onProposal = vi.fn()
    render(wrap(<DashboardLayoutWizard onProposal={onProposal} />))
    fireEvent.change(screen.getByPlaceholderText(/water treatment plant/), { target: { value: 'two pumps' } })
    fireEvent.click(screen.getByRole('button', { name: 'Generate Layout' }))
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Try again in 20 s'))
    expect(onProposal).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: 'Generate Layout' })).toBeTruthy()
  })

  it('a reply cut off at the token limit says to send less', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json(502, { error: 'cut', code: 'ai_truncated' })))
    render(wrap(<DashboardLayoutWizard onProposal={() => {}} />))
    fireEvent.change(screen.getByPlaceholderText(/water treatment plant/), { target: { value: 'two pumps' } })
    fireEvent.click(screen.getByRole('button', { name: 'Generate Layout' }))
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Send less at a time'))
  })

  it('the template wizard shows the server refusal', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => url.includes('/ai/templates')
      ? json(200, [{ id: 3, name: 'Pump', industry: 'water', equipment_type: 'pump', description: '', registers: [] }])
      : json(400, { error: 'x', code: 'ai_invalid_api_key' })))
    render(wrap(<TemplateWizard onProposal={() => {}} />))
    fireEvent.click(await screen.findByRole('button', { name: 'Use' }))
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('rejected the API key'))
  })
})

describe('proposal review', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('a failed apply shows the error and the proposal as the server now has it', async () => {
    let applied = false
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      if (url.endsWith('/apply')) {
        applied = true
        return json(500, { error: 'Undoing the changes made before the failure also failed' })
      }
      return json(200, applied
        ? proposal({ status: 'failed', error_message: 'Check the configuration: tag t1 was not put back' })
        : proposal({ status: 'approved' }))
    }))
    const onBack = vi.fn()
    render(wrap(<ProposalReview proposal={proposal({ status: 'approved' })} onBack={onBack} />))
    fireEvent.click(screen.getByRole('button', { name: 'Apply to Production' }))
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Undoing the changes'))
    // Refetched: the status and its message are the server's, and it cannot be applied again.
    await waitFor(() => expect(screen.getByText('failed')).toBeTruthy())
    expect(screen.getByText(/tag t1 was not put back/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Apply to Production' })).toBeNull()
    expect(onBack).not.toHaveBeenCalled()
  })

  it('an old error is cleared when the reviewer edits or tries again', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => url.endsWith('/approve')
      ? json(400, { error: 'The edited configuration is not valid: threshold 1' })
      : json(200, proposal({}))))
    render(wrap(<ProposalReview proposal={proposal({})} onBack={() => {}} />))
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('not valid: threshold 1'))
    fireEvent.change(screen.getByPlaceholderText(/Paste modified JSON/), { target: { value: '[]' } })
    expect(screen.queryByRole('alert')).toBeNull()
  })
})
