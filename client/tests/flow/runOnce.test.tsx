import { describe, expect, it, vi, beforeEach } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { Flow, FlowCatalogue } from '../../src/types'
import { FlowManager } from '../../src/FlowManager'
import { FlowEditor } from '../../src/components/flow-editor/FlowEditor'
import { parseRunInput } from '../../src/components/flow-editor/RunOnceDialog'
import { SignatureDialog } from '../../src/signature/SignatureDialog'

const catalogue: FlowCatalogue = {
  triggers: [{ type: 'timer', label: 'Timer', fields: [] }],
  nodes: [{ type: 'filter', label: 'Filter', category: 'logic', outputs: ['pass', 'reject'], fields: [] }],
}

const flow: Flow = {
  id: 'flow_1', name: 'Boiler', enabled: false, created_at: 1, updated_at: 1,
  trigger_config: JSON.stringify({ type: 'timer', interval_ms: 1000 }),
  nodes: JSON.stringify({ version: 2, nodes: [{ id: 'n1', type: 'filter', x: 0, y: 0, condition: 'true' }], edges: [{ from: 'trigger', port: 0, to: 'n1' }] }),
}

const signatureRequired = {
  code: 'signature_required',
  error: 'Signature required',
  signature_required: { action: 'flow_run', meaning: 'I start a manual run of flow Boiler' },
}

/** Fetch stub: the flow list, empty runs, and `/run` answered by `run`. */
function stubFetch(run: (body: Record<string, unknown>) => Response) {
  const runs: Record<string, unknown>[] = []
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const u = String(url)
    if (u.endsWith('/run')) {
      const body = JSON.parse(String(init?.body ?? '{}'))
      runs.push(body)
      return run(body)
    }
    if (u.endsWith('/runs')) return new Response(JSON.stringify({ runs: [] }), { status: 200 })
    if (u.endsWith('/node-types')) return new Response(JSON.stringify(catalogue), { status: 200 })
    if (u.endsWith('/api/v1/flows')) return new Response(JSON.stringify({ flows: [flow] }), { status: 200 })
    return new Response('{}', { status: 200 })
  }))
  return runs
}

function wrap(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <div style={{ width: 1200, height: 800 }}>{ui}</div>
      <SignatureDialog />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  stubFetch(() => new Response(JSON.stringify({ ok: true }), { status: 200 }))
})

describe('parseRunInput', () => {
  it('treats empty as no input, a payload key as the full input, and anything else as the payload', () => {
    expect(parseRunInput('  ')).toBeUndefined()
    expect(parseRunInput('{bad')).toBeNull()
    expect(parseRunInput('{"payload": 5, "metadata": {"a": 1}}')).toEqual({ payload: 5, metadata: { a: 1 } })
    expect(parseRunInput('{"t": 90}')).toEqual({ payload: { t: 90 }, metadata: {} })
  })
})

describe('Run once in the flow list', () => {
  it('is offered to operators and not to viewers', async () => {
    const { unmount } = wrap(<FlowManager isAdmin={false} canOperate />)
    const row = await screen.findByText('Boiler')
    expect(within(row.closest('[role="row"]') as HTMLElement).getByRole('button', { name: 'Run once' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'View' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Edit' })).toBeNull()
    unmount()

    wrap(<FlowManager isAdmin={false} canOperate={false} />)
    await screen.findByText('Boiler')
    expect(screen.queryByRole('button', { name: 'Run once' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Start' })).toBeNull()
  })

  it('runs with the typed input', async () => {
    const runs = stubFetch(() => new Response(JSON.stringify({ ok: true }), { status: 200 }))
    wrap(<FlowManager isAdmin={false} canOperate />)
    fireEvent.click(await screen.findByRole('button', { name: 'Run once' }))
    fireEvent.change(screen.getByLabelText('Run input JSON'), { target: { value: '{"t": 90}' } })
    fireEvent.click(screen.getByRole('button', { name: 'Run now' }))
    await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Ran “Boiler” once.'))
    expect(runs).toEqual([{ input: { payload: { t: 90 }, metadata: {} } }])
    expect(screen.queryByLabelText('Run input JSON')).toBeNull()
  })

  it('refuses input that is not JSON without calling the server', async () => {
    const runs = stubFetch(() => new Response('{}', { status: 200 }))
    wrap(<FlowManager isAdmin={false} canOperate />)
    fireEvent.click(await screen.findByRole('button', { name: 'Run once' }))
    fireEvent.change(screen.getByLabelText('Run input JSON'), { target: { value: '{bad' } })
    fireEvent.click(screen.getByRole('button', { name: 'Run now' }))
    expect(screen.getByRole('alert').textContent).toBe('The input is not valid JSON.')
    expect(runs).toEqual([])
  })

  it('asks for a signature on 428 and sends the run again signed', async () => {
    const runs = stubFetch(body => body.signature
      ? new Response(JSON.stringify({ ok: true }), { status: 200 })
      : new Response(JSON.stringify(signatureRequired), { status: 428 }))
    wrap(<FlowManager isAdmin={false} canOperate />)
    fireEvent.click(await screen.findByRole('button', { name: 'Run once' }))
    fireEvent.click(screen.getByRole('button', { name: 'Run now' }))
    await screen.findByText('Electronic signature required')
    expect(screen.getByText('I start a manual run of flow Boiler')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Your password'), { target: { value: 'secret' } })
    fireEvent.change(screen.getByLabelText(/Reason/i), { target: { value: 'batch 7' } })
    fireEvent.click(screen.getByRole('button', { name: 'Sign' }))
    await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Ran “Boiler” once.'))
    expect(runs).toHaveLength(2)
    expect(runs[0]).toEqual({})
    expect(runs[1].signature).toEqual({ password: 'secret', meaning: 'I start a manual run of flow Boiler', reason: 'batch 7' })
  })

  it('keeps the dialog open, with no error, when the signature is cancelled', async () => {
    stubFetch(() => new Response(JSON.stringify(signatureRequired), { status: 428 }))
    wrap(<FlowManager isAdmin={false} canOperate />)
    fireEvent.click(await screen.findByRole('button', { name: 'Run once' }))
    fireEvent.click(screen.getByRole('button', { name: 'Run now' }))
    const prompt = (await screen.findByText('Electronic signature required')).closest('form') as HTMLElement
    fireEvent.click(within(prompt).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(screen.queryByText('Electronic signature required')).toBeNull())
    await waitFor(() => expect((screen.getByRole('button', { name: 'Run now' }) as HTMLButtonElement).disabled).toBe(false))
    expect(screen.queryByRole('alert')).toBeNull()
  })
})

describe('Run once in the read-only editor', () => {
  it('shows for operators, not for viewers, and editors keep Run', async () => {
    const { unmount } = wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly canRun onSave={async () => {}} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n1')).toBeTruthy())
    expect(screen.getByRole('button', { name: 'Run once' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Run' })).toBeNull()
    unmount()

    const viewer = wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly onSave={async () => {}} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n1')).toBeTruthy())
    expect(screen.queryByRole('button', { name: 'Run once' })).toBeNull()
    viewer.unmount()

    wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly={false} canRun onSave={async () => {}} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n1')).toBeTruthy())
    expect(screen.getByRole('button', { name: 'Run' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Run once' })).toBeNull()
  })

  it('opens the signature prompt on 428', async () => {
    const runs = stubFetch(body => body.signature
      ? new Response(JSON.stringify({ ok: true }), { status: 200 })
      : new Response(JSON.stringify(signatureRequired), { status: 428 }))
    wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly canRun onSave={async () => {}} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n1')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Run once' }))
    fireEvent.click(screen.getByRole('button', { name: 'Run now' }))
    await screen.findByText('Electronic signature required')
    fireEvent.change(screen.getByLabelText('Your password'), { target: { value: 'secret' } })
    fireEvent.change(screen.getByLabelText(/Reason/i), { target: { value: 'test' } })
    fireEvent.click(screen.getByRole('button', { name: 'Sign' }))
    await waitFor(() => expect(screen.getByText('Run started.')).toBeTruthy())
    expect(runs).toHaveLength(2)
  })
})
