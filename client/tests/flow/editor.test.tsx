import { describe, expect, it, vi, beforeEach } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { Flow, FlowCatalogue, FlowField } from '../../src/types'
import { FieldForm } from '../../src/components/flow-editor/FieldForm'
import { FlowEditor } from '../../src/components/flow-editor/FlowEditor'
import { validateFields } from '../../src/components/flow-editor/model'

const catalogue: FlowCatalogue = {
  triggers: [
    { type: 'timer', label: 'Timer', fields: [{ key: 'interval_ms', label: 'Interval', kind: 'duration_ms', required: true, default: 5000 }] },
    { type: 'webhook', label: 'Webhook', fields: [] },
  ],
  nodes: [
    { type: 'filter', label: 'Filter', category: 'logic', outputs: ['pass', 'reject'], fields: [
      { key: 'condition', label: 'Condition', kind: 'expression', required: true, help: 'Uses the expression language' },
    ] },
    { type: 'switch', label: 'Switch', category: 'logic', outputs: ['otherwise'], fields: [
      { key: 'rules', label: 'Rules', kind: 'json', required: true, default: [] },
    ] },
    { type: 'http_out', label: 'HTTP', category: 'output', outputs: ['out'], side_effect: true, fields: [
      { key: 'url', label: 'URL', kind: 'string', required: true },
    ] },
  ],
}

const flow: Flow = {
  id: 'flow_1', name: 'Boiler', enabled: true, created_at: 1, updated_at: 1,
  trigger_config: JSON.stringify({ type: 'timer', interval_ms: 1000 }),
  nodes: JSON.stringify({
    version: 2,
    nodes: [
      { id: 'n1', type: 'filter', x: 0, y: 0, condition: 'payload.t > 1' },
      { id: 'n2', type: 'switch', x: 300, y: 0, rules: [{ name: 'hot', expression: 'payload.t > 90' }] },
      { id: 'n3', type: 'http_out', x: 600, y: 0, url: 'https://x.test' },
    ],
    edges: [{ from: 'trigger', port: 0, to: 'n1' }, { from: 'n1', port: 0, to: 'n2' }, { from: 'n2', port: 1, to: 'n3' }],
  }),
}

function wrap(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={qc}><div style={{ width: 1200, height: 800 }}>{ui}</div></QueryClientProvider>)
}

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    if (String(url).endsWith('/runs')) return new Response(JSON.stringify({ runs: [] }), { status: 200 })
    return new Response('[]', { status: 200 })
  }))
})

describe('FieldForm', () => {
  const fields: FlowField[] = [
    { key: 'url', label: 'URL', kind: 'string', required: true, help: 'Where to POST' },
    { key: 'timeout_ms', label: 'Timeout', kind: 'duration_ms', default: 5000 },
    { key: 'retry', label: 'Retry', kind: 'bool' },
    { key: 'mode', label: 'Mode', kind: 'select', options: [{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }] },
    { key: 'sev', label: 'Severity', kind: 'severity' },
    { key: 'src', label: 'Source', kind: 'code' },
    { key: 'headers', label: 'Headers', kind: 'json' },
    { key: 'tag', label: 'Tag', kind: 'tag' },
  ]

  it('renders one input per field kind, with help text and required markers', () => {
    wrap(<FieldForm fields={fields} config={{ url: 'https://a', timeout_ms: 1500, mode: 'b', headers: { a: 1 } }} errors={{}} onChange={() => {}} />)
    expect((screen.getByLabelText(/URL/) as HTMLInputElement).value).toBe('https://a')
    expect(screen.getByText('Where to POST')).toBeTruthy()
    expect(screen.getByText('1.5 s')).toBeTruthy()
    expect((screen.getByLabelText('Retry') as HTMLInputElement).type).toBe('checkbox')
    expect((screen.getByLabelText('Mode') as HTMLSelectElement).value).toBe('b')
    expect(within(screen.getByLabelText('Severity')).getByText('Critical')).toBeTruthy()
    expect(screen.getByLabelText('Source').tagName).toBe('TEXTAREA')
    expect((screen.getByLabelText('Headers') as HTMLTextAreaElement).value).toContain('"a": 1')
    expect(screen.getByLabelText('Tag').tagName).toBe('SELECT')
  })

  it('shows validation errors in place of help text', () => {
    const errors = validateFields(fields, { url: '' })
    wrap(<FieldForm fields={fields} config={{ url: '' }} errors={errors} onChange={() => {}} />)
    expect(screen.getByRole('alert').textContent).toBe('URL is required')
    expect(screen.getByLabelText(/URL/).getAttribute('aria-invalid')).toBe('true')
    expect(screen.queryByText('Where to POST')).toBeNull()
  })

  it('reports changes with typed values and holds back invalid JSON', () => {
    const onChange = vi.fn()
    wrap(<FieldForm fields={fields} config={{}} errors={{}} onChange={onChange} />)
    fireEvent.change(screen.getByLabelText('Timeout'), { target: { value: '250' } })
    expect(onChange).toHaveBeenCalledWith('timeout_ms', 250)
    onChange.mockClear()
    fireEvent.change(screen.getByLabelText('Headers'), { target: { value: '{bad' } })
    expect(onChange).not.toHaveBeenCalled()
    expect(screen.getByText(/Invalid JSON/)).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Headers'), { target: { value: '{"x": 2}' } })
    expect(onChange).toHaveBeenCalledWith('headers', { x: 2 })
  })

  it('edits list fields with item_fields as a form per item', () => {
    const onChange = vi.fn()
    const rules: FlowField = {
      key: 'rules', label: 'Rules', kind: 'json', required: true,
      item_fields: [
        { key: 'gt', label: 'Greater than', kind: 'number' },
        { key: 'lt', label: 'Less than', kind: 'number' },
        { key: 'set', label: 'Set fields', kind: 'json' },
      ],
    }
    wrap(<FieldForm fields={[rules]} config={{ rules: [{ gt: 50, set: { level: 'high' }, keep: 1 }] }} errors={{}} onChange={onChange} />)
    expect(screen.getAllByTestId('item-row')).toHaveLength(1)
    expect((screen.getByLabelText('Greater than') as HTMLInputElement).value).toBe('50')
    expect((screen.getByLabelText('Set fields') as HTMLTextAreaElement).value).toContain('"level": "high"')
    fireEvent.change(screen.getByLabelText('Less than'), { target: { value: '90' } })
    expect(onChange).toHaveBeenLastCalledWith('rules', [{ gt: 50, lt: 90, set: { level: 'high' }, keep: 1 }])
    fireEvent.click(screen.getByRole('button', { name: '+ Add rule' }))
    expect(onChange).toHaveBeenLastCalledWith('rules', [{ gt: 50, set: { level: 'high' }, keep: 1 }, {}])
    fireEvent.click(screen.getByRole('button', { name: 'Remove rule 1' }))
    expect(onChange).toHaveBeenLastCalledWith('rules', [])
  })

  it('picks several tags for a `multiple` tag field', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify([
      { id: 3, name: 'Temp', key: 'temp', unit: 'C' }, { id: 4, name: 'Flow', key: 'flow' }, { id: 5, name: 'Level', key: 'level' },
    ]), { status: 200 })))
    const onChange = vi.fn()
    const tags: FlowField = { key: 'data_source_ids', label: 'Tags', kind: 'tag', required: true, multiple: true }
    // A single id (older flows) shows as one chosen tag.
    wrap(<FieldForm fields={[tags]} config={{ data_source_ids: 3 }} errors={{}} onChange={onChange} />)
    await waitFor(() => expect(screen.getAllByTestId('tag-chip')[0].textContent).toContain('Temp (C)'))
    const select = screen.getByLabelText(/Tags/) as HTMLSelectElement
    expect(within(select).queryByText(/Temp/)).toBeNull()
    fireEvent.change(select, { target: { value: '5' } })
    expect(onChange).toHaveBeenLastCalledWith('data_source_ids', [3, 5])
    fireEvent.click(screen.getByRole('button', { name: 'Remove Temp (C)' }))
    expect(onChange).toHaveBeenLastCalledWith('data_source_ids', [])
  })

  it('edits switch rules as rows and reports how ports move', () => {
    const onRules = vi.fn()
    const rules = [{ name: 'a', expression: 'x > 1' }, { name: 'b', expression: 'x > 2' }]
    wrap(<FieldForm fields={catalogue.nodes[1].fields} nodeType="switch" config={{ rules }} errors={{}} onChange={() => {}} onRules={onRules} />)
    expect(screen.getAllByTestId('rule-row')).toHaveLength(2)
    fireEvent.click(screen.getByLabelText('Move rule 2 up'))
    const [next, map] = onRules.mock.calls[0]
    expect(next.map((r: { name: string }) => r.name)).toEqual(['b', 'a'])
    expect([map(0), map(1), map(2)]).toEqual([1, 0, 2])
    fireEvent.click(screen.getByLabelText('Remove rule 1'))
    const [, rm] = onRules.mock.calls[1]
    expect([rm(0), rm(1), rm(2)]).toEqual([null, 0, 1])
    fireEvent.click(screen.getByText('+ Add rule'))
    const [added, am] = onRules.mock.calls[2]
    expect(added).toHaveLength(3)
    expect(am(2)).toBe(3)
  })

  it('is read-only when asked', () => {
    wrap(<FieldForm fields={fields} config={{ url: 'u' }} errors={{}} readOnly onChange={() => {}} />)
    expect((screen.getByLabelText(/URL/) as HTMLInputElement).readOnly).toBe(true)
    expect((screen.getByLabelText('Retry') as HTMLInputElement).disabled).toBe(true)
    expect((screen.getByLabelText('Mode') as HTMLSelectElement).disabled).toBe(true)
  })
})

describe('FlowEditor', () => {
  it('renders the graph with labelled ports, switch outputs from its rules', async () => {
    wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly={false} onSave={async () => {}} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n2')).toBeTruthy())
    expect(screen.getByTestId('flow-node-trigger')).toBeTruthy()
    expect(screen.getByTestId('port-n1-0').textContent).toBe('pass')
    expect(screen.getByTestId('port-n1-1').textContent).toBe('reject')
    expect(screen.getByTestId('port-n2-0').textContent).toBe('hot')
    expect(screen.getByTestId('port-n2-1').textContent).toBe('otherwise')
    expect(screen.getByRole('complementary', { name: 'Node palette' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Save' })).toBeTruthy()
  })

  it('gives viewers a read-only canvas', async () => {
    wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly onSave={async () => {}} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n1')).toBeTruthy())
    expect(screen.queryByRole('complementary', { name: 'Node palette' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Save' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull()
    expect(screen.queryByText('Auto layout')).toBeNull()
    expect(screen.queryByText('Test run')).toBeNull()
    expect((screen.getByLabelText('Flow name') as HTMLInputElement).readOnly).toBe(true)
    expect(screen.getByText('read-only')).toBeTruthy()
    const node = screen.getByTestId('flow-node-n1').closest('.react-flow__node')!
    expect(node.classList.contains('draggable')).toBe(false)
    expect(document.querySelector('.react-flow__handle.connectable')).toBeNull()
  })

  it('blocks save on missing required settings and points at the node', async () => {
    const onSave = vi.fn(async () => {})
    const bad = { ...flow, nodes: flow.nodes.replace('https://x.test', '') }
    wrap(<FlowEditor flow={bad} catalogue={catalogue} saved readOnly={false} onSave={onSave} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n3')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(onSave).not.toHaveBeenCalled()
    expect(screen.getByText(/1 node has missing or invalid settings/)).toBeTruthy()
    expect(screen.getAllByText('URL is required').length).toBeGreaterThan(0)
  })

  it('saves v2 JSON and shows a server 400 on the node it names', async () => {
    const { FlowApiError } = await import('../../src/queries/flows')
    const onSave = vi.fn(async (_f: Flow) => { throw new FlowApiError("node 'n2': rule 1 expression does not parse", 400) })
    wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly={false} onSave={onSave} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n2')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(onSave).toHaveBeenCalled())
    const sent = JSON.parse(onSave.mock.calls[0][0].nodes)
    expect(sent.version).toBe(2)
    expect(sent.edges).toContainEqual({ from: 'n2', port: 1, to: 'n3' })
    await waitFor(() => expect(screen.getByTestId('flow-node-n2').getAttribute('data-error')).toBe('true'))
  })

  it('highlights the path of a test run and lists the trace', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (String(url).endsWith('/flows/test')) {
        return new Response(JSON.stringify({ ok: true, outputs_reached: ['n3'], trace: [
          { node_id: 'n1', type: 'filter', input: {}, output: {}, port: 0, error: null, duration_us: 12, side_effect_skipped: false },
          { node_id: 'n2', type: 'switch', input: {}, output: {}, port: 1, error: null, duration_us: 30, side_effect_skipped: false },
          { node_id: 'n3', type: 'http_out', input: {}, output: null, port: null, error: null, duration_us: 2, side_effect_skipped: true },
        ] }), { status: 200 })
      }
      return new Response(JSON.stringify({ runs: [] }), { status: 200 })
    }))
    wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly={false} onSave={async () => {}} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n1')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Test' }))
    fireEvent.click(screen.getByRole('button', { name: 'Run test (dry run)' }))
    await waitFor(() => expect(screen.getByTestId('flow-node-n3').getAttribute('data-ran')).toBe('true'))
    expect(screen.getByText(/skipped \(dry run\): would POST to https:\/\/x.test/)).toBeTruthy()
    expect(screen.getByText('1 output reached: n3')).toBeTruthy()
    expect(screen.getByTestId('port-n2-1').style.fontWeight).toBe('700')
    fireEvent.click(screen.getByTestId('trace-row-n2'))
    await waitFor(() => expect(screen.getByRole('complementary', { name: 'Node settings' }).textContent).toContain('switch · n2'))
  })
  it('shows why a dry run failed and what skipped nodes would have done', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (String(url).endsWith('/flows/test')) {
        return new Response(JSON.stringify({ ok: false, error: "node 'n3': refused", outputs_reached: ['n3'], trace: [
          { node_id: 'n1', type: 'filter', input: {}, output: {}, port: 0, ports: [0], error: null, duration_us: 12, side_effect_skipped: false },
          { node_id: 'n2', type: 'switch', input: {}, output: {}, port: 1, ports: [1], error: null, duration_us: 30, side_effect_skipped: false },
          { node_id: 'n3', type: 'http_out', input: {}, output: { would_post: { url: 'https://server.test/x', headers: [] } }, port: 0, ports: [0],
            error: null, duration_us: 2, side_effect_skipped: true },
        ] }), { status: 200 })
      }
      return new Response(JSON.stringify({ runs: [] }), { status: 200 })
    }))
    wrap(<FlowEditor flow={flow} catalogue={catalogue} saved readOnly={false} onSave={async () => {}} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('flow-node-n1')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Test' }))
    fireEvent.click(screen.getByRole('button', { name: 'Run test (dry run)' }))
    await waitFor(() => expect(screen.getByText('Failed')).toBeTruthy())
    expect(screen.getByText("node 'n3': refused")).toBeTruthy()
    expect(screen.getByText(/would POST to https:\/\/server.test\/x/)).toBeTruthy()
  })
})
