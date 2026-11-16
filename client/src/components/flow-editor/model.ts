import type { Edge, Node } from '@xyflow/react'
import type {
  FlowCatalogue, FlowField, FlowGraphEdgeV2, FlowGraphNodeV2, FlowGraphV2, FlowNodeTypeDef,
  FlowRunTraceStep, FlowTraceStep, FlowTriggerConfig,
} from '../../types'

/** Id of the virtual source node standing for the flow's trigger. */
export const TRIGGER_ID = 'trigger'

export interface FlowNodeData extends Record<string, unknown> {
  type: string
  name?: string
  config: Record<string, unknown>
}

export type EdNode = Node<FlowNodeData, 'flow'>
export type EdEdge = Edge

export interface EditorGraph {
  nodes: EdNode[]
  edges: EdEdge[]
}

// ── Grid and layout constants ──

export const GRID = 20
export const NODE_W = 200
export const COL_GAP = 280
export const ROW_GAP = 120
const LEGACY_ROW_GAP = 140

// ── Ports ──

export const handleId = (port: number) => `out-${port}`

export function portOf(handle: string | null | undefined): number {
  if (!handle) return 0
  const m = /^out-(\d+)$/.exec(handle)
  return m ? Number(m[1]) : 0
}

export const edgeId = (from: string, port: number, to: string) => `e:${from}:${port}:${to}`

export interface SwitchRule { name: string; expression: string }

export function switchRules(config: Record<string, unknown>): SwitchRule[] {
  const r = config.rules
  if (!Array.isArray(r)) return []
  return r.map(x => ({
    name: typeof x?.name === 'string' ? x.name : '',
    expression: typeof x?.expression === 'string' ? x.expression : '',
  }))
}

/**
 * Output names of a node. A switch has one output per rule (named after the
 * rule) followed by the catalogue's own outputs ("otherwise").
 */
export function outputsFor(type: string, config: Record<string, unknown>, def?: FlowNodeTypeDef): string[] {
  const base = def?.outputs?.length ? def.outputs : ['out']
  if (type === 'switch') {
    const rules = switchRules(config).map((r, i) => r.name.trim() || `rule ${i + 1}`)
    return [...rules, ...(def?.outputs?.length ? def.outputs : ['otherwise'])]
  }
  return base
}

/**
 * Re-number the ports of edges leaving `nodeId`. `map(old)` returns the new
 * port, or null to drop the edge. Used when switch rules move or disappear so
 * each wire stays with its rule.
 */
export function remapPorts(edges: EdEdge[], nodeId: string, map: (port: number) => number | null): EdEdge[] {
  const out: EdEdge[] = []
  for (const e of edges) {
    if (e.source !== nodeId) { out.push(e); continue }
    const np = map(portOf(e.sourceHandle))
    if (np === null) continue
    out.push({ ...e, id: edgeId(e.source, np, e.target), sourceHandle: handleId(np) })
  }
  return dedupeEdges(out)
}

function dedupeEdges(edges: EdEdge[]): EdEdge[] {
  const seen = new Set<string>()
  return edges.filter(e => (seen.has(e.id) ? false : (seen.add(e.id), true)))
}

// ── Parsing: nodes JSON (v2 or legacy array) ──

export function emptyGraph(): FlowGraphV2 {
  return { version: 2, nodes: [], edges: [] }
}

/**
 * Parse the `nodes` string of a flow. Version 2 is used as is; a legacy JSON
 * array (older server) becomes a chain from the trigger, laid out top to
 * bottom; anything else is an empty graph.
 */
export function parseGraph(nodesJson: string | null | undefined): FlowGraphV2 {
  let raw: unknown
  try { raw = JSON.parse(nodesJson || '') } catch { return emptyGraph() }
  if (Array.isArray(raw)) return legacyToV2(raw)
  if (raw && typeof raw === 'object' && (raw as { version?: unknown }).version === 2) {
    const g = raw as Partial<FlowGraphV2>
    return {
      version: 2,
      nodes: Array.isArray(g.nodes) ? g.nodes.filter(n => n && typeof n.id === 'string' && typeof n.type === 'string') : [],
      edges: Array.isArray(g.edges) ? g.edges.filter(e => e && typeof e.from === 'string' && typeof e.to === 'string')
        .map(e => ({ from: e.from, port: Number(e.port) || 0, to: e.to })) : [],
    }
  }
  return emptyGraph()
}

export function legacyToV2(arr: unknown[]): FlowGraphV2 {
  const used = new Set<string>()
  const nodes: FlowGraphNodeV2[] = []
  arr.forEach((item, i) => {
    if (!item || typeof item !== 'object') return
    const { id, type, ...config } = item as Record<string, unknown>
    if (typeof type !== 'string') return
    let nid = typeof id === 'string' && id && id !== TRIGGER_ID && !used.has(id) ? id : ''
    if (!nid) { let k = i + 1; while (used.has(`n${k}`)) k++; nid = `n${k}` }
    used.add(nid)
    nodes.push({ ...config, id: nid, type, x: 0, y: (nodes.length + 1) * LEGACY_ROW_GAP })
  })
  const edges: FlowGraphEdgeV2[] = []
  let prev = TRIGGER_ID
  for (const n of nodes) { edges.push({ from: prev, port: 0, to: n.id }); prev = n.id }
  return { version: 2, nodes, edges }
}

export function isLegacyNodes(nodesJson: string): boolean {
  try { return Array.isArray(JSON.parse(nodesJson)) } catch { return false }
}

export function countNodes(nodesJson: string): number {
  return parseGraph(nodesJson).nodes.length
}

// ── v2 <-> editor graph ──

const RESERVED = new Set(['id', 'type', 'x', 'y', 'name'])

export function graphToEditor(g: FlowGraphV2): EditorGraph {
  const nodes: EdNode[] = g.nodes.map(n => {
    const config: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(n)) if (!RESERVED.has(k)) config[k] = v
    const data: FlowNodeData = { type: n.type, config }
    if (typeof n.name === 'string' && n.name) data.name = n.name
    return { id: n.id, type: 'flow', position: { x: Number(n.x) || 0, y: Number(n.y) || 0 }, data }
  })
  const edges: EdEdge[] = dedupeEdges(g.edges.map(e => toEdge(e.from, e.port, e.to)))
  return { nodes, edges }
}

export function toEdge(from: string, port: number, to: string): EdEdge {
  return { id: edgeId(from, port, to), source: from, sourceHandle: handleId(port), target: to, targetHandle: 'in' }
}

export function editorToGraph(nodes: EdNode[], edges: EdEdge[]): FlowGraphV2 {
  return {
    version: 2,
    nodes: nodes.filter(n => n.id !== TRIGGER_ID).map(n => {
      const out: FlowGraphNodeV2 = {
        ...n.data.config,
        id: n.id,
        type: n.data.type,
        x: Math.round(n.position.x),
        y: Math.round(n.position.y),
      }
      if (n.data.name) out.name = n.data.name
      return out
    }),
    edges: edges.map(e => ({ from: e.source, port: portOf(e.sourceHandle), to: e.target })),
  }
}

// ── Ids, copy / paste ──

export function nextNodeId(taken: Iterable<string>): string {
  const set = new Set(taken)
  let k = 1
  while (set.has(`n${k}`)) k++
  return `n${k}`
}

export interface Clip { nodes: EdNode[]; edges: EdEdge[] }

/** Selected nodes plus the edges running between them. The trigger is never copied. */
export function copySelection(nodes: EdNode[], edges: EdEdge[], ids: Set<string>): Clip {
  const picked = nodes.filter(n => ids.has(n.id) && n.id !== TRIGGER_ID)
  const keep = new Set(picked.map(n => n.id))
  return {
    nodes: structuredClone(picked.map(n => ({ id: n.id, type: n.type, position: n.position, data: n.data }))) as EdNode[],
    edges: edges.filter(e => keep.has(e.source) && keep.has(e.target)).map(e => ({ ...e })),
  }
}

/** New nodes/edges for a paste: fresh ids, offset, and selected. */
export function pasteClip(clip: Clip, taken: Iterable<string>, offset = 40): Clip {
  const used = new Set(taken)
  const ids = new Map<string, string>()
  const nodes = clip.nodes.map(n => {
    const id = nextNodeId(used)
    used.add(id)
    ids.set(n.id, id)
    return { ...structuredClone(n), id, selected: true, position: { x: n.position.x + offset, y: n.position.y + offset } }
  })
  const edges = clip.edges.map(e => {
    const s = ids.get(e.source)!, t = ids.get(e.target)!
    return toEdge(s, portOf(e.sourceHandle), t)
  })
  return { nodes, edges }
}

// ── Trigger placement and auto layout ──

/** The trigger sits left of the left-most node, level with the nodes it feeds. */
export function triggerPosition(nodes: EdNode[], edges: EdEdge[]): { x: number; y: number } {
  if (!nodes.length) return { x: 0, y: 0 }
  const minX = Math.min(...nodes.map(n => n.position.x))
  const targets = new Set(edges.filter(e => e.source === TRIGGER_ID).map(e => e.target))
  const fed = nodes.filter(n => targets.has(n.id))
  const ys = (fed.length ? fed : nodes).map(n => n.position.y)
  const y = fed.length ? ys.reduce((a, b) => a + b, 0) / ys.length : Math.min(...ys)
  return { x: snap(minX - COL_GAP), y: snap(y) }
}

export const snap = (v: number) => Math.round(v / GRID) * GRID

/**
 * Left-to-right layering: each node goes one column right of its deepest
 * predecessor (longest path from the trigger; back edges of a cycle are
 * ignored). Within a column nodes are ordered by their parents' rows.
 */
export function autoLayout(nodes: EdNode[], edges: EdEdge[]): EdNode[] {
  const ids = new Set(nodes.map(n => n.id))
  const out = new Map<string, EdEdge[]>()
  const indeg = new Map<string, number>()
  for (const n of nodes) indeg.set(n.id, 0)
  for (const e of edges) {
    if (!ids.has(e.target) || (e.source !== TRIGGER_ID && !ids.has(e.source))) continue
    if (!out.has(e.source)) out.set(e.source, [])
    out.get(e.source)!.push(e)
  }

  // Drop back edges (DFS from the trigger, then from any unvisited node).
  const state = new Map<string, 1 | 2>()
  const forward: EdEdge[] = []
  const visit = (id: string) => {
    state.set(id, 1)
    const outs = [...(out.get(id) ?? [])].sort((a, b) => portOf(a.sourceHandle) - portOf(b.sourceHandle))
    for (const e of outs) {
      const s = state.get(e.target)
      if (s === 1) continue
      forward.push(e)
      if (!s) visit(e.target)
    }
    state.set(id, 2)
  }
  visit(TRIGGER_ID)
  for (const n of nodes) if (!state.has(n.id)) visit(n.id)

  const preds = new Map<string, string[]>()
  for (const e of forward) {
    if (!preds.has(e.target)) preds.set(e.target, [])
    preds.get(e.target)!.push(e.source)
    indeg.set(e.target, (indeg.get(e.target) ?? 0) + 1)
  }

  // Longest path layer via Kahn's order.
  const layer = new Map<string, number>([[TRIGGER_ID, 0]])
  const queue = [TRIGGER_ID, ...nodes.filter(n => !indeg.get(n.id) && !forward.some(e => e.target === n.id)).map(n => n.id)]
  for (const id of queue) if (id !== TRIGGER_ID && !layer.has(id)) layer.set(id, 1)
  const remaining = new Map(indeg)
  for (let i = 0; i < queue.length; i++) {
    const id = queue[i]
    for (const e of forward.filter(x => x.source === id)) {
      layer.set(e.target, Math.max(layer.get(e.target) ?? 1, (layer.get(id) ?? 0) + 1))
      const r = (remaining.get(e.target) ?? 1) - 1
      remaining.set(e.target, r)
      if (r === 0) queue.push(e.target)
    }
  }

  const cols = new Map<number, string[]>()
  for (const n of nodes) {
    const l = layer.get(n.id) ?? 1
    if (!cols.has(l)) cols.set(l, [])
    cols.get(l)!.push(n.id)
  }
  const row = new Map<string, number>([[TRIGGER_ID, 0]])
  const portRank = (id: string) => {
    const e = forward.find(x => x.target === id)
    return e ? portOf(e.sourceHandle) : 0
  }
  const pos = new Map<string, { x: number; y: number }>()
  for (const l of [...cols.keys()].sort((a, b) => a - b)) {
    const col = cols.get(l)!
    const key = (id: string) => {
      const ps = (preds.get(id) ?? []).map(p => row.get(p) ?? 0)
      return ps.length ? ps.reduce((a, b) => a + b, 0) / ps.length : Number.MAX_SAFE_INTEGER
    }
    const order = nodes.map(n => n.id)
    col.sort((a, b) => key(a) - key(b) || portRank(a) - portRank(b) || order.indexOf(a) - order.indexOf(b))
    const top = -((col.length - 1) * ROW_GAP) / 2
    col.forEach((id, i) => {
      row.set(id, i - (col.length - 1) / 2)
      pos.set(id, { x: l * COL_GAP, y: snap(top + i * ROW_GAP) })
    })
  }
  return nodes.map(n => ({ ...n, position: pos.get(n.id) ?? n.position }))
}

// ── Field validation ──

export function isEmptyValue(v: unknown): boolean {
  return v === undefined || v === null || (typeof v === 'string' && v.trim() === '') || (typeof v === 'number' && Number.isNaN(v))
}

/** Field errors for a config: missing required values and bad JSON text. */
export function validateFields(fields: FlowField[], config: Record<string, unknown>): Record<string, string> {
  const errs: Record<string, string> = {}
  for (const fd of fields) {
    const v = config[fd.key]
    if (fd.required && isEmptyValue(v)) { errs[fd.key] = `${fd.label} is required`; continue }
    if (fd.required && fd.key === 'rules' && Array.isArray(v) && v.length === 0) { errs[fd.key] = 'Add at least one rule'; continue }
    if (fd.kind === 'number' || fd.kind === 'duration_ms') {
      if (!isEmptyValue(v) && typeof v !== 'number') errs[fd.key] = `${fd.label} must be a number`
      else if (fd.kind === 'duration_ms' && typeof v === 'number' && v < 0) errs[fd.key] = `${fd.label} cannot be negative`
    }
  }
  if (Array.isArray(config.rules)) {
    switchRules(config).forEach((r, i) => {
      if (!r.expression.trim()) errs.rules ??= `Rule ${i + 1} needs an expression`
    })
  }
  return errs
}

/** Ids of nodes with field errors (including the trigger). */
export function invalidNodes(
  nodes: EdNode[], trigger: FlowTriggerConfig, cat: FlowCatalogue,
): Map<string, Record<string, string>> {
  const out = new Map<string, Record<string, string>>()
  const tdef = cat.triggers.find(t => t.type === trigger.type)
  if (tdef) {
    const e = validateFields(tdef.fields, trigger)
    if (Object.keys(e).length) out.set(TRIGGER_ID, e)
  }
  for (const n of nodes) {
    const def = cat.nodes.find(d => d.type === n.data.type)
    if (!def) continue
    const e = validateFields(def.fields, n.data.config)
    if (Object.keys(e).length) out.set(n.id, e)
  }
  return out
}

// ── Trace -> highlight ──

export interface Highlight {
  /** Nodes that ran (the trigger counts once the trace is non-empty). */
  ran: Set<string>
  /** Edges a message travelled. */
  taken: Set<string>
  /** Ports each node emitted on. */
  ports: Map<string, Set<number>>
  errors: Map<string, string>
}

type AnyStep = Pick<FlowTraceStep, 'node_id' | 'port' | 'error'> | FlowRunTraceStep

export function traceHighlight(trace: AnyStep[], edges: EdEdge[]): Highlight {
  const ran = new Set<string>()
  const ports = new Map<string, Set<number>>()
  const errors = new Map<string, string>()
  for (const s of trace) {
    ran.add(s.node_id)
    if (s.error) errors.set(s.node_id, s.error)
    if (s.port !== null && s.port !== undefined) {
      if (!ports.has(s.node_id)) ports.set(s.node_id, new Set())
      ports.get(s.node_id)!.add(Number(s.port))
    }
  }
  if (trace.length && !ran.has(TRIGGER_ID)) {
    ran.add(TRIGGER_ID)
    if (!ports.has(TRIGGER_ID)) ports.set(TRIGGER_ID, new Set([0]))
  }
  const taken = new Set<string>()
  for (const e of edges) {
    const p = ports.get(e.source)
    if (p?.has(portOf(e.sourceHandle)) && ran.has(e.target)) taken.add(e.id)
  }
  return { ran, taken, ports, errors }
}

/** Run trace from the run log; may be stored as a JSON string. */
export function runTrace(trace: FlowRunTraceStep[] | string | null | undefined): FlowRunTraceStep[] {
  if (!trace) return []
  if (Array.isArray(trace)) return trace
  try { const v = JSON.parse(trace); return Array.isArray(v) ? v : [] } catch { return [] }
}

// ── Save errors ──

/**
 * Which node a 400 from save/test is about: an explicit `node_id` in the body,
 * else the first node id the message quotes or names.
 */
export function findErrorNode(message: string, ids: string[], nodeId?: unknown): string | null {
  if (typeof nodeId === 'string' && (ids.includes(nodeId) || nodeId === TRIGGER_ID)) return nodeId
  const sorted = [...ids, TRIGGER_ID].sort((a, b) => b.length - a.length)
  for (const id of sorted) {
    const esc = id.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
    if (new RegExp(`(^|[^\\w-])${esc}($|[^\\w-])`).test(message)) return id
  }
  return null
}

// ── Test input samples, summaries ──

export function sampleInput(trigger: FlowTriggerConfig): { payload: unknown; metadata: Record<string, unknown> } {
  switch (trigger.type) {
    case 'modbus':
      return {
        payload: { device_id: trigger.device_id && trigger.device_id !== '*' ? trigger.device_id : 'plc-1', metrics: { [trigger.metric || 'temp']: 85 } },
        metadata: { trigger: 'modbus' },
      }
    case 'mqtt':
      return { payload: { value: 42.5, unit: 'C' }, metadata: { trigger: 'mqtt', topic: String(trigger.topic || 'sensors/line1/temp').replace(/[+#]/g, 'line1') } }
    case 'timer':
    case 'cron':
      return { payload: {}, metadata: { trigger: trigger.type } }
    case 'webhook':
      return { payload: { event: 'batch_complete', batch: 'B-1042', count: 120 }, metadata: { trigger: 'webhook' } }
    case 'tag_change':
    case 'data_source':
      return { payload: { value: 73.2, previous: 70.1, quality: 'good' }, metadata: { trigger: trigger.type } }
    case 'alarm':
      return { payload: { source_key: 'boiler_temp', severity: 'critical', message: 'High temperature' }, metadata: { trigger: 'alarm' } }
    default:
      return { payload: { value: 42 }, metadata: { trigger: trigger.type } }
  }
}

export function fmtDuration(ms: unknown): string {
  const n = Number(ms)
  if (!Number.isFinite(n)) return ''
  if (n < 1000) return `${n} ms`
  if (n < 60_000) return `${+(n / 1000).toFixed(1)} s`
  if (n < 3_600_000) return `${+(n / 60_000).toFixed(1)} min`
  return `${+(n / 3_600_000).toFixed(1)} h`
}

export function triggerSummary(trigger: FlowTriggerConfig, cat: FlowCatalogue): string {
  const label = cat.triggers.find(t => t.type === trigger.type)?.label ?? trigger.type ?? 'Unknown'
  const detail =
    trigger.type === 'timer' ? (trigger.interval_ms ? `every ${fmtDuration(trigger.interval_ms)}` : '')
    : trigger.type === 'mqtt' ? trigger.topic
    : trigger.type === 'modbus' ? (trigger.device_id === '*' ? 'any device' : trigger.device_id)
    : trigger.type === 'cron' ? (trigger.cron ?? trigger.schedule ?? trigger.expression)
    : ''
  return detail ? `${label} · ${detail}` : label
}

/** "would …" text for a side-effect node skipped in a dry run. */
export function wouldText(type: string, config: Record<string, unknown>, def?: FlowNodeTypeDef): string {
  const c = config
  if (c.url) return `would ${String(c.method ?? 'POST').toUpperCase()} to ${c.url}`
  if (type.includes('alarm')) return `would raise ${c.severity ?? ''} alarm${c.message ? ` "${c.message}"` : ''}`.replace('  ', ' ')
  if (type.includes('write')) return `would write ${c.value_expr ?? 'a value'} to tag ${c.data_source_id ?? c.tag ?? '?'}`
  if (c.topic) return `would publish to ${c.topic}`
  if (c.source_key) return `would update ${c.source_key}`
  return `would run ${def?.label ?? type}`
}

export function parseTrigger(json: string): FlowTriggerConfig {
  try {
    const v = JSON.parse(json)
    if (v && typeof v === 'object' && typeof v.type === 'string') return v
  } catch { /* fall through */ }
  return { type: 'modbus' }
}

/** Stable text of the editable state, used for the unsaved-changes check. */
export function snapshotKey(name: string, enabled: boolean, trigger: FlowTriggerConfig, nodes: EdNode[], edges: EdEdge[]): string {
  return JSON.stringify([name, enabled, trigger, editorToGraph(nodes, edges)])
}
