import { describe, expect, it } from 'vitest'
import type { FlowGraphV2, FlowNodeTypeDef } from '../../src/types'
import {
  TRIGGER_ID, autoLayout, copySelection, editorToGraph, findErrorNode, graphToEditor, legacyToV2, outputsFor,
  parseGraph, pasteClip, portOf, remapPorts, runTrace, toEdge, traceHighlight, triggerPosition, validateFields,
} from '../../src/components/flow-editor/model'

const switchDef: FlowNodeTypeDef = {
  type: 'switch', label: 'Switch', category: 'logic', outputs: ['otherwise'],
  fields: [{ key: 'rules', label: 'Rules', kind: 'json', required: true }],
}

const v2: FlowGraphV2 = {
  version: 2,
  nodes: [
    { id: 'n1', type: 'filter', x: 120, y: 80, name: 'Hot only', condition: 'payload.temp > 80' },
    { id: 'n2', type: 'switch', x: 400, y: 80, rules: [{ name: 'high', expression: 'payload.temp > 90' }, { name: 'mid', expression: 'payload.temp > 85' }] },
    { id: 'n3', type: 'alarm_out', x: 680, y: 0, severity: 'critical', message: 'Very hot', extra: { keep: [1, 2] } },
    { id: 'n4', type: 'http_out', x: 680, y: 160, url: 'https://x.test/hook' },
  ],
  edges: [
    { from: 'trigger', port: 0, to: 'n1' },
    { from: 'n1', port: 0, to: 'n2' },
    { from: 'n2', port: 0, to: 'n3' },
    { from: 'n2', port: 2, to: 'n4' },
    { from: 'n1', port: 1, to: 'n4' },
  ],
}

describe('v2 JSON <-> editor graph', () => {
  it('round-trips nodes, labels, config, positions and ports', () => {
    const g = graphToEditor(parseGraph(JSON.stringify(v2)))
    expect(g.nodes).toHaveLength(4)
    expect(g.nodes[0].data).toEqual({ type: 'filter', name: 'Hot only', config: { condition: 'payload.temp > 80' } })
    expect(g.edges.find(e => e.source === 'n2' && e.target === 'n4')?.sourceHandle).toBe('out-2')
    expect(g.edges.find(e => e.source === 'n1' && e.target === 'n4')?.sourceHandle).toBe('out-1')
    const back = editorToGraph(g.nodes, g.edges)
    expect(back).toEqual(v2)
  })

  it('names switch outputs after its rules, then "otherwise"', () => {
    const g = graphToEditor(v2)
    const sw = g.nodes.find(n => n.id === 'n2')!
    expect(outputsFor('switch', sw.data.config, switchDef)).toEqual(['high', 'mid', 'otherwise'])
    expect(outputsFor('switch', { rules: [{ name: '', expression: 'x' }] }, switchDef)).toEqual(['rule 1', 'otherwise'])
    expect(outputsFor('filter', {}, { ...switchDef, type: 'filter', outputs: ['pass', 'reject'] })).toEqual(['pass', 'reject'])
  })

  it('keeps wires with their rule when rules move or are removed', () => {
    const { edges } = graphToEditor(v2)
    // swap rule 0 and 1: the wire on port 0 (high) moves to port 1
    const swapped = remapPorts(edges, 'n2', p => (p === 0 ? 1 : p === 1 ? 0 : p))
    expect(swapped.find(e => e.source === 'n2' && e.target === 'n3')?.sourceHandle).toBe('out-1')
    expect(swapped.find(e => e.source === 'n2' && e.target === 'n4')?.sourceHandle).toBe('out-2')
    // remove rule 0: its wire goes, "otherwise" moves from port 2 to 1
    const removed = remapPorts(edges, 'n2', p => (p === 0 ? null : p - 1))
    expect(removed.some(e => e.source === 'n2' && e.target === 'n3')).toBe(false)
    expect(removed.find(e => e.source === 'n2' && e.target === 'n4')?.sourceHandle).toBe('out-1')
    expect(editorToGraph(graphToEditor(v2).nodes, removed).edges).toContainEqual({ from: 'n2', port: 1, to: 'n4' })
  })

  it('treats bad JSON or unknown versions as an empty graph', () => {
    expect(parseGraph('nope')).toEqual({ version: 2, nodes: [], edges: [] })
    expect(parseGraph('{"version":3}')).toEqual({ version: 2, nodes: [], edges: [] })
  })
})

describe('legacy array fallback', () => {
  it('converts the old step list to a chain from the trigger, laid out vertically', () => {
    const legacy = [
      { id: 'f', type: 'filter', condition: 'payload.a > 1' },
      { type: 'expression', output_field: 'payload.b', expr: '1' },
      { id: 'f', type: 'alarm_out', severity: 'warning' },
    ]
    const g = parseGraph(JSON.stringify(legacy))
    expect(g.nodes.map(n => n.id)).toEqual(['f', 'n2', 'n3'])
    expect(g.nodes[0]).toMatchObject({ type: 'filter', condition: 'payload.a > 1', x: 0 })
    expect(new Set(g.nodes.map(n => n.x))).toEqual(new Set([0]))
    expect(g.nodes[0].y).toBeLessThan(g.nodes[1].y)
    expect(g.nodes[1].y).toBeLessThan(g.nodes[2].y)
    expect(g.edges).toEqual([
      { from: 'trigger', port: 0, to: 'f' },
      { from: 'f', port: 0, to: 'n2' },
      { from: 'n2', port: 0, to: 'n3' },
    ])
    expect(legacyToV2([])).toEqual({ version: 2, nodes: [], edges: [] })
  })
})

describe('trace -> highlight', () => {
  const { edges } = graphToEditor(v2)
  it('marks the nodes that ran and the edges on the chosen ports', () => {
    const h = traceHighlight([
      { node_id: 'n1', port: 0, error: null },
      { node_id: 'n2', port: 2, error: null },
      { node_id: 'n4', port: null, error: 'timeout' },
    ], edges)
    expect([...h.ran].sort()).toEqual(['n1', 'n2', 'n4', TRIGGER_ID].sort())
    expect(h.taken).toEqual(new Set(['e:trigger:0:n1', 'e:n1:0:n2', 'e:n2:2:n4']))
    expect(h.taken.has('e:n2:0:n3')).toBe(false)
    expect(h.taken.has('e:n1:1:n4')).toBe(false)
    expect(h.ports.get('n2')).toEqual(new Set([2]))
    expect(h.errors.get('n4')).toBe('timeout')
  })

  it('reads run traces stored as JSON text', () => {
    expect(runTrace('[{"node_id":"n1","port":0,"error":null}]')).toHaveLength(1)
    expect(runTrace('garbage')).toEqual([])
    expect(runTrace(null)).toEqual([])
  })
})

describe('editing helpers', () => {
  it('pastes copies with fresh ids and their internal edges only', () => {
    const g = graphToEditor(v2)
    const clip = copySelection(g.nodes, g.edges, new Set(['n1', 'n2', TRIGGER_ID]))
    expect(clip.nodes.map(n => n.id)).toEqual(['n1', 'n2'])
    expect(clip.edges.map(e => e.id)).toEqual(['e:n1:0:n2'])
    const p = pasteClip(clip, g.nodes.map(n => n.id))
    expect(p.nodes.map(n => n.id)).toEqual(['n5', 'n6'])
    expect(p.edges).toEqual([toEdge('n5', 0, 'n6')])
    expect(p.nodes[0].position).toEqual({ x: 160, y: 120 })
  })

  it('auto-layout puts each node right of its parents, trigger column first', () => {
    const g = graphToEditor(v2)
    const laid = autoLayout(g.nodes, g.edges)
    const x = (id: string) => laid.find(n => n.id === id)!.position.x
    expect(x('n1')).toBeLessThan(x('n2'))
    expect(x('n2')).toBeLessThan(x('n3'))
    // n4 is fed by n1 and n2: longest path puts it after n2
    expect(x('n4')).toBe(x('n3'))
    expect(triggerPosition(laid, g.edges).x).toBeLessThan(x('n1'))
  })

  it('survives cycles in auto-layout', () => {
    const g = graphToEditor({ version: 2, nodes: [{ id: 'a', type: 'x', x: 0, y: 0 }, { id: 'b', type: 'x', x: 0, y: 0 }],
      edges: [{ from: 'trigger', port: 0, to: 'a' }, { from: 'a', port: 0, to: 'b' }, { from: 'b', port: 0, to: 'a' }] })
    const laid = autoLayout(g.nodes, g.edges)
    expect(laid[0].position.x).toBeLessThan(laid[1].position.x)
  })

  it('finds the node a server error names', () => {
    expect(findErrorNode("node 'n3': invalid config", ['n1', 'n3'])).toBe('n3')
    expect(findErrorNode('cycle through n12 -> n1', ['n1', 'n12'])).toBe('n12')
    expect(findErrorNode('bad port', ['n1'], 'n1')).toBe('n1')
    expect(findErrorNode('something else', ['n1'])).toBeNull()
  })

  it('parses ports from handle ids', () => {
    expect(portOf('out-3')).toBe(3)
    expect(portOf(null)).toBe(0)
  })
})

describe('field validation', () => {
  const fields = [
    { key: 'url', label: 'URL', kind: 'string', required: true },
    { key: 'timeout_ms', label: 'Timeout', kind: 'duration_ms' },
    { key: 'retries', label: 'Retries', kind: 'number', required: true },
  ]
  it('flags missing required values and bad numbers', () => {
    expect(validateFields(fields, { url: '  ', retries: 0 })).toEqual({ url: 'URL is required' })
    expect(validateFields(fields, { url: 'x', retries: undefined, timeout_ms: -5 }))
      .toEqual({ retries: 'Retries is required', timeout_ms: 'Timeout cannot be negative' })
    expect(validateFields(fields, { url: 'x', retries: 1 })).toEqual({})
  })
  it('needs an expression on each switch rule', () => {
    expect(validateFields(switchDef.fields, { rules: [] })).toEqual({ rules: 'Add at least one rule' })
    expect(validateFields(switchDef.fields, { rules: [{ name: 'a', expression: '' }] })).toEqual({ rules: 'Rule 1 needs an expression' })
  })
})
