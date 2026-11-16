import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ReactFlow, ReactFlowProvider, Background, BackgroundVariant, Controls, MiniMap, useReactFlow,
  applyNodeChanges, applyEdgeChanges,
  type Connection, type EdgeChange, type NodeChange, type NodeTypes,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import './flowEditor.css'
import type { Flow, FlowCatalogue, FlowRun, FlowTestInput, FlowTriggerConfig } from '../../types'
import { btnStyle } from '../../styles'
import { useTheme } from '../../ThemeContext'
import { useFlowRuns, useRunFlow, useTestFlow, FlowApiError } from '../../queries'
import { colorFor, defaultConfig } from './catalogue'
import { GraphNode, FlowViewContext, type FlowView } from './GraphNode'
import { Palette, DND_TYPE } from './Palette'
import { NodePanel, TriggerPanel, EmptyPanel } from './ConfigPanel'
import { TestPanel, type TraceSource } from './TestPanel'
import type { PortMap } from './FieldForm'
import { useHistory } from './useHistory'
import {
  TRIGGER_ID, GRID, autoLayout, copySelection, editorToGraph, findErrorNode, graphToEditor, invalidNodes,
  isLegacyNodes, nextNodeId, parseGraph, parseTrigger, pasteClip, remapPorts, runTrace, sampleInput, snap,
  snapshotKey, switchRules, toEdge, traceHighlight, triggerPosition, portOf,
  type Clip, type EdEdge, type EdNode, type SwitchRule,
} from './model'

interface Doc {
  trigger: FlowTriggerConfig
  nodes: EdNode[]
  edges: EdEdge[]
}

export interface FlowEditorProps {
  flow: Flow
  catalogue: FlowCatalogue
  /** The flow already exists on the server. */
  saved: boolean
  readOnly: boolean
  /** Persist the flow; rejects with FlowApiError on a 400. */
  onSave: (flow: Flow) => Promise<void>
  onClose: () => void
}

const nodeTypes: NodeTypes = { flow: GraphNode }

// Survives switching flows, so nodes can be copied from one flow to another.
let clipboard: Clip | null = null

const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.platform)
const MOD = isMac ? '⌘' : 'Ctrl'

export const SHORTCUTS: [string, string][] = [
  ['Delete / Backspace', 'Delete selected nodes and wires'],
  [`${MOD}+C / ${MOD}+V`, 'Copy / paste'],
  [`${MOD}+D`, 'Duplicate selection'],
  [`${MOD}+A`, 'Select all nodes'],
  [`${MOD}+Z`, 'Undo'],
  [`${MOD}+Shift+Z / ${MOD}+Y`, 'Redo'],
  [`${MOD}+S`, 'Save'],
  ['Shift+drag', 'Box-select'],
  [`${MOD}+click`, 'Add to selection'],
  ['F', 'Fit the flow in view'],
  ['?', 'Show this help'],
  ['Esc', 'Clear selection / close'],
]

export function FlowEditor(props: FlowEditorProps) {
  return (
    <ReactFlowProvider>
      <EditorInner {...props} />
    </ReactFlowProvider>
  )
}

function initialDoc(flow: Flow): Doc {
  const g = graphToEditor(parseGraph(flow.nodes))
  return { trigger: parseTrigger(flow.trigger_config), nodes: g.nodes, edges: g.edges }
}

function EditorInner({ flow, catalogue, saved: savedInitially, readOnly, onSave, onClose }: FlowEditorProps) {
  const rf = useReactFlow()
  const { resolvedTheme } = useTheme()
  const wrapRef = useRef<HTMLDivElement>(null)

  const hist = useHistory<Doc>(() => initialDoc(flow))
  const doc = hist.present
  const [name, setName] = useState(flow.name)
  const [enabled, setEnabled] = useState(flow.enabled)
  const [triggerSelected, setTriggerSelected] = useState(false)
  const [triggerMeasured, setTriggerMeasured] = useState<{ width?: number; height?: number } | undefined>()
  const [saved, setSaved] = useState(savedInitially)
  const [saving, setSaving] = useState(false)
  const [banner, setBanner] = useState<{ kind: 'error' | 'ok' | 'info'; text: string } | null>(
    isLegacyNodes(flow.nodes) && JSON.parse(flow.nodes).length
      ? { kind: 'info', text: 'This flow was stored as a list of steps. It is shown as a chain; saving stores it as a graph.' }
      : null,
  )
  const [serverError, setServerError] = useState<{ nodeId: string; message: string } | null>(null)
  const [helpOpen, setHelpOpen] = useState(false)

  const [baseline, setBaseline] = useState(() => {
    const d = initialDoc(flow)
    return snapshotKey(flow.name, flow.enabled, d.trigger, d.nodes, d.edges)
  })
  const current = useMemo(() => snapshotKey(name, enabled, doc.trigger, doc.nodes, doc.edges), [name, enabled, doc])
  const dirty = !readOnly && (current !== baseline || !saved)

  // ── Test / runs ──
  const [panelOpen, setPanelOpen] = useState(false)
  const [panelTab, setPanelTab] = useState<'test' | 'runs'>(readOnly ? 'runs' : 'test')
  const [input, setInput] = useState(() => JSON.stringify(sampleInput(parseTrigger(flow.trigger_config)), null, 2))
  const [inputTouched, setInputTouched] = useState(false)
  const [source, setSource] = useState<TraceSource | null>(null)
  const [testError, setTestError] = useState<string | null>(null)
  const testMut = useTestFlow()
  const runMut = useRunFlow()
  const runsQ = useFlowRuns(flow.id, saved, { refetchInterval: panelOpen && panelTab === 'runs' ? 5000 : undefined })
  const showLatestRun = useRef(false)

  useEffect(() => {
    if (showLatestRun.current && runsQ.data?.length) {
      showLatestRun.current = false
      setSource({ kind: 'run', run: runsQ.data[0] })
    }
  }, [runsQ.data])

  const traceSteps = useMemo(() => {
    if (!source) return null
    return source.kind === 'test' ? source.result.trace : runTrace(source.run.trace)
  }, [source])
  const highlight = useMemo(() => (traceSteps ? traceHighlight(traceSteps, doc.edges) : null), [traceSteps, doc.edges])
  const invalid = useMemo(() => invalidNodes(doc.nodes, doc.trigger, catalogue), [doc.nodes, doc.trigger, catalogue])

  const view: FlowView = useMemo(() => ({
    catalogue, trigger: doc.trigger, highlight, invalid, serverError, readOnly,
  }), [catalogue, doc.trigger, highlight, invalid, serverError, readOnly])

  // ── Canvas data ──
  const triggerPos = useMemo(() => triggerPosition(doc.nodes, doc.edges), [doc.nodes, doc.edges])
  const rfNodes: EdNode[] = useMemo(() => [
    {
      id: TRIGGER_ID, type: 'flow', position: triggerPos, draggable: false, deletable: false,
      selected: triggerSelected, measured: triggerMeasured, data: { type: doc.trigger.type, config: {} },
    },
    ...doc.nodes,
  ], [doc.nodes, doc.trigger.type, triggerPos, triggerSelected, triggerMeasured])

  const rfEdges: EdEdge[] = useMemo(() => doc.edges.map(e => {
    if (!highlight) return e
    const taken = highlight.taken.has(e.id)
    return { ...e, animated: taken, className: taken ? 'vf-edge-taken' : 'vf-edge-dim' }
  }), [doc.edges, highlight])

  const selectedNodes = doc.nodes.filter(n => n.selected)

  // ── Mutations ──
  const update = hist.update

  const onNodesChange = useCallback((changes: NodeChange<EdNode>[]) => {
    const own: NodeChange<EdNode>[] = []
    for (const c of changes) {
      if ('id' in c && c.id === TRIGGER_ID) {
        if (c.type === 'select') setTriggerSelected(c.selected)
        // Keep its size so the minimap and fit view can see it.
        if (c.type === 'dimensions' && c.dimensions) setTriggerMeasured(c.dimensions)
        continue
      }
      if (c.type === 'remove' || c.type === 'add' || c.type === 'replace') continue // deletion goes through deleteSelected
      if (readOnly && c.type === 'position') continue
      own.push(c)
    }
    if (!own.length) return
    update(d => ({ ...d, nodes: applyNodeChanges(own, d.nodes) }), { record: false })
  }, [update, readOnly])

  const onEdgesChange = useCallback((changes: EdgeChange<EdEdge>[]) => {
    const sel = changes.filter(c => c.type === 'select')
    if (!sel.length) return
    update(d => ({ ...d, edges: applyEdgeChanges(sel, d.edges) }), { record: false })
  }, [update])

  const isValidConnection = useCallback((c: Connection | EdEdge) =>
    c.target !== TRIGGER_ID && c.source !== c.target, [])

  const onConnect = useCallback((c: Connection) => {
    if (readOnly || !c.source || !c.target || c.target === TRIGGER_ID || c.source === c.target) return
    const e = toEdge(c.source, portOf(c.sourceHandle), c.target)
    update(d => d.edges.some(x => x.id === e.id) ? d : ({ ...d, edges: [...d.edges, e] }))
  }, [update, readOnly])

  const addNode = useCallback((type: string, at?: { x: number; y: number }) => {
    if (readOnly) return
    const def = catalogue.nodes.find(n => n.type === type)
    if (!def) return
    let pos = at
    if (!pos) {
      const r = wrapRef.current?.getBoundingClientRect()
      pos = r ? rf.screenToFlowPosition({ x: r.left + r.width / 2, y: r.top + r.height / 2 }) : { x: 0, y: 0 }
      pos = { x: pos.x - 100, y: pos.y - 30 }
    }
    setTriggerSelected(false)
    update(d => {
      const id = nextNodeId(d.nodes.map(n => n.id))
      const node: EdNode = {
        id, type: 'flow', selected: true, position: { x: snap(pos!.x), y: snap(pos!.y) },
        data: { type, config: defaultConfig(def.fields) },
      }
      // A first node gets wired to the trigger, so a new flow works as soon as one node is in.
      const edges = d.nodes.length === 0 ? [...d.edges, toEdge(TRIGGER_ID, 0, id)] : d.edges
      return { ...d, nodes: [...d.nodes.map(n => (n.selected ? { ...n, selected: false } : n)), node], edges }
    })
  }, [catalogue, readOnly, rf, update])

  const deleteSelected = useCallback(() => {
    if (readOnly) return
    update(d => {
      const gone = new Set(d.nodes.filter(n => n.selected).map(n => n.id))
      const edges = d.edges.filter(e => !e.selected && !gone.has(e.source) && !gone.has(e.target))
      if (!gone.size && edges.length === d.edges.length) return d
      return { ...d, nodes: d.nodes.filter(n => !gone.has(n.id)), edges }
    })
  }, [update, readOnly])

  const copy = useCallback(() => {
    const ids = new Set(doc.nodes.filter(n => n.selected).map(n => n.id))
    if (ids.size) clipboard = copySelection(doc.nodes, doc.edges, ids)
  }, [doc])

  const paste = useCallback((clip: Clip | null) => {
    if (readOnly || !clip?.nodes.length) return
    setTriggerSelected(false)
    const pasted = pasteClip(clip, doc.nodes.map(n => n.id))
    update(d => ({
      ...d,
      nodes: [...d.nodes.map(n => (n.selected ? { ...n, selected: false } : n)), ...pasted.nodes],
      edges: [...d.edges.map(e => (e.selected ? { ...e, selected: false } : e)), ...pasted.edges],
    }))
    return pasted
  }, [readOnly, update, doc.nodes])

  const duplicate = useCallback(() => {
    const ids = new Set(doc.nodes.filter(n => n.selected).map(n => n.id))
    if (ids.size) paste(copySelection(doc.nodes, doc.edges, ids))
  }, [doc, paste])

  const selectOnly = useCallback((id: string | null) => {
    setTriggerSelected(id === TRIGGER_ID)
    update(d => ({
      ...d,
      nodes: d.nodes.map(n => (!!n.selected === (n.id === id) ? n : { ...n, selected: n.id === id })),
      edges: d.edges.map(e => (e.selected ? { ...e, selected: false } : e)),
    }), { record: false })
  }, [update])

  const updateNode = useCallback((id: string, fn: (n: EdNode) => EdNode, key?: string) => {
    update(d => ({ ...d, nodes: d.nodes.map(n => (n.id === id ? fn(n) : n)) }), { key })
  }, [update])

  const setConfig = useCallback((id: string, key: string, value: unknown) => {
    if (serverError?.nodeId === id) setServerError(null)
    updateNode(id, n => {
      const config = { ...n.data.config }
      if (value === undefined) delete config[key]
      else config[key] = value
      return { ...n, data: { ...n.data, config } }
    }, `cfg:${id}:${key}`)
  }, [updateNode, serverError])

  const setRules = useCallback((id: string, rules: SwitchRule[], map: PortMap) => {
    const node = doc.nodes.find(n => n.id === id)
    const before = node ? switchRules(node.data.config).length : 0
    let identity = rules.length === before
    for (let p = 0; identity && p <= before; p++) identity = map(p) === p
    update(d => ({
      ...d,
      nodes: d.nodes.map(n => (n.id === id ? { ...n, data: { ...n.data, config: { ...n.data.config, rules } } } : n)),
      edges: identity ? d.edges : remapPorts(d.edges, id, map),
    }), { key: identity ? `rules:${id}` : undefined })
  }, [doc.nodes, update])

  const setTrigger = useCallback((key: string, value: unknown) => {
    if (serverError?.nodeId === TRIGGER_ID) setServerError(null)
    update(d => {
      const trigger = { ...d.trigger }
      if (value === undefined) delete trigger[key]
      else trigger[key] = value
      return { ...d, trigger }
    }, { key: `trigger:${key}` })
  }, [update, serverError])

  const setTriggerType = useCallback((type: string) => {
    const def = catalogue.triggers.find(t => t.type === type)
    const trigger = { ...defaultConfig(def?.fields ?? []), type } as FlowTriggerConfig
    update(d => ({ ...d, trigger }))
    if (!inputTouched) setInput(JSON.stringify(sampleInput(trigger), null, 2))
  }, [catalogue, update, inputTouched])

  const layout = useCallback(() => {
    if (readOnly) return
    update(d => ({ ...d, nodes: autoLayout(d.nodes, d.edges) }))
    requestAnimationFrame(() => rf.fitView({ padding: 0.2, duration: 300, maxZoom: 1.1 }))
  }, [update, rf, readOnly])

  // ── Save ──
  const buildFlow = useCallback((): Flow => ({
    ...flow,
    name: name.trim(),
    enabled,
    trigger_config: JSON.stringify(doc.trigger),
    nodes: JSON.stringify(editorToGraph(doc.nodes, doc.edges)),
    updated_at: Math.floor(Date.now() / 1000),
  }), [flow, name, enabled, doc])

  const showServerError = useCallback((err: unknown, prefix: string) => {
    const msg = err instanceof Error ? err.message : String(err)
    const nodeId = findErrorNode(msg, doc.nodes.map(n => n.id), err instanceof FlowApiError ? err.nodeId : undefined)
    if (nodeId) {
      setServerError({ nodeId, message: msg })
      selectOnly(nodeId)
    }
    return `${prefix}: ${msg}`
  }, [doc.nodes, selectOnly])

  const save = useCallback(async () => {
    if (readOnly || saving) return
    if (!name.trim()) { setBanner({ kind: 'error', text: 'Give the flow a name.' }); return }
    if (invalid.size) {
      const first = [...invalid.keys()][0]
      selectOnly(first)
      setBanner({ kind: 'error', text: `${invalid.size} node${invalid.size === 1 ? ' has' : 's have'} missing or invalid settings.` })
      return
    }
    setSaving(true)
    setServerError(null)
    try {
      await onSave(buildFlow())
      setSaved(true)
      setBaseline(current)
      setBanner({ kind: 'ok', text: 'Saved.' })
    } catch (err) {
      setBanner({ kind: 'error', text: showServerError(err, 'Not saved') })
    } finally {
      setSaving(false)
    }
  }, [readOnly, saving, name, invalid, onSave, buildFlow, current, selectOnly, showServerError])

  useEffect(() => {
    if (banner?.kind !== 'ok') return
    const t = setTimeout(() => setBanner(null), 2500)
    return () => clearTimeout(t)
  }, [banner])

  // ── Test / run ──
  const parseInput = (): FlowTestInput | null => {
    try {
      const v = JSON.parse(input || '{}')
      if (v && typeof v === 'object' && 'payload' in v) return { payload: v.payload, metadata: v.metadata ?? {} }
      return { payload: v, metadata: {} }
    } catch {
      return null
    }
  }

  const runTest = async () => {
    const parsed = parseInput()
    if (!parsed) { setTestError('The input is not valid JSON.'); return }
    setTestError(null)
    setServerError(null)
    const f = buildFlow()
    try {
      const result = await testMut.mutateAsync({ flow: { trigger_config: f.trigger_config, nodes: f.nodes }, input: parsed })
      setSource({ kind: 'test', result })
    } catch (err) {
      setSource(null)
      setTestError(showServerError(err, 'Test failed'))
    }
  }

  const runNow = async () => {
    if (!saved) return
    if (dirty && !confirm('Run the saved version? Your unsaved changes are not included.')) return
    if (!confirm('Run this flow now? Output nodes act for real (writes, alarms, webhooks).')) return
    const parsed = parseInput()
    try {
      await runMut.mutateAsync({ id: flow.id, input: parsed ?? undefined })
      showLatestRun.current = true
      setPanelTab('runs')
      setPanelOpen(true)
      setBanner({ kind: 'ok', text: 'Run started.' })
      runsQ.refetch()
    } catch (err) {
      setBanner({ kind: 'error', text: showServerError(err, 'Run failed') })
    }
  }

  const close = useCallback(() => {
    if (dirty && !confirm('Discard unsaved changes to this flow?')) return
    onClose()
  }, [dirty, onClose])

  useEffect(() => {
    if (!dirty) return
    const h = (e: BeforeUnloadEvent) => { e.preventDefault() }
    window.addEventListener('beforeunload', h)
    return () => window.removeEventListener('beforeunload', h)
  }, [dirty])

  // ── Keyboard ──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      if (t && (t.closest('input, textarea, select, [contenteditable="true"]'))) return
      if (!wrapRef.current?.isConnected) return
      const mod = e.ctrlKey || e.metaKey
      const k = e.key.toLowerCase()
      if (mod && k === 's') { e.preventDefault(); save(); return }
      if (e.key === '?') { setHelpOpen(o => !o); return }
      if (e.key === 'Escape') { if (helpOpen) setHelpOpen(false); else selectOnly(null); return }
      if (!mod && k === 'f') { rf.fitView({ padding: 0.2, duration: 250, maxZoom: 1.1 }); return }
      if (readOnly) return
      if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); deleteSelected(); return }
      if (mod && k === 'z' && !e.shiftKey) { e.preventDefault(); hist.undo(); return }
      if (mod && ((k === 'z' && e.shiftKey) || k === 'y')) { e.preventDefault(); hist.redo(); return }
      if (mod && k === 'c') { copy(); return }
      if (mod && k === 'v') { e.preventDefault(); const p = paste(clipboard); if (p) clipboard = p; return }
      if (mod && k === 'd') { e.preventDefault(); duplicate(); return }
      if (mod && k === 'a') {
        e.preventDefault()
        update(d => ({ ...d, nodes: d.nodes.map(n => ({ ...n, selected: true })) }), { record: false })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [save, helpOpen, selectOnly, rf, readOnly, deleteSelected, hist, copy, paste, duplicate, update])

  // ── Drop from palette ──
  const onDragOver = (e: React.DragEvent) => {
    if (readOnly || !e.dataTransfer.types.includes(DND_TYPE)) return
    e.preventDefault()
    e.dataTransfer.dropEffect = 'copy'
  }
  const onDrop = (e: React.DragEvent) => {
    const type = e.dataTransfer.getData(DND_TYPE)
    if (readOnly || !type) return
    e.preventDefault()
    const p = rf.screenToFlowPosition({ x: e.clientX, y: e.clientY })
    addNode(type, { x: p.x - 100, y: p.y - 24 })
  }

  // ── Side panel ──
  const single = selectedNodes.length === 1 && !triggerSelected ? selectedNodes[0] : null
  let panel: React.ReactNode
  if (triggerSelected && selectedNodes.length === 0) {
    panel = <TriggerPanel trigger={doc.trigger} catalogue={catalogue} readOnly={readOnly} flowId={flow.id} saved={saved}
      serverError={serverError?.nodeId === TRIGGER_ID ? serverError.message : undefined}
      onChange={setTrigger} onType={setTriggerType} />
  } else if (single) {
    panel = <NodePanel node={single} catalogue={catalogue} readOnly={readOnly}
      serverError={serverError?.nodeId === single.id ? serverError.message : undefined}
      onConfig={(k, v) => setConfig(single.id, k, v)}
      onName={v => updateNode(single.id, n => ({ ...n, data: { ...n.data, name: v || undefined } }), `name:${single.id}`)}
      onRules={(r, m) => setRules(single.id, r, m)}
      onDuplicate={duplicate} onDelete={deleteSelected} />
  } else {
    panel = <EmptyPanel count={selectedNodes.length + (triggerSelected ? 1 : 0)} readOnly={readOnly} />
  }

  const tb = { ...btnStyle, padding: '5px 10px' }
  const iconTb = { ...tb, padding: '5px 8px' }

  return (
    <FlowViewContext.Provider value={view}>
      <div ref={wrapRef} className="vf-editor" style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, overflow: 'hidden', position: 'relative' }}>
        {/* Toolbar */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 14px', borderBottom: '1px solid var(--border)', background: 'var(--card)', flexShrink: 0 }}>
          <button type="button" style={iconTb} onClick={close} aria-label="Back to flows" title="Back to flows">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>
          </button>
          <input aria-label="Flow name" value={name} readOnly={readOnly} onChange={e => setName(e.target.value)} placeholder="Flow name"
            style={{ background: 'transparent', border: '1px solid transparent', borderRadius: 6, color: 'var(--fg)', fontSize: 15, fontWeight: 700, padding: '4px 6px', fontFamily: 'inherit', minWidth: 120, flex: '0 1 320px' }}
            className="vf-title" />
          {dirty && <span title="Unsaved changes" style={{ fontSize: 11, color: 'var(--warn)' }}>● unsaved</span>}
          {readOnly && <span style={{ fontSize: 11, color: 'var(--muted)', border: '1px solid var(--border)', borderRadius: 999, padding: '1px 8px' }}>read-only</span>}
          <div style={{ flex: 1 }} />
          {!readOnly && (
            <>
              <button type="button" style={iconTb} onClick={hist.undo} disabled={!hist.canUndo} aria-label="Undo" title={`Undo (${MOD}+Z)`}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M3 7v6h6M3 13a9 9 0 103-7.7L3 8" /></svg>
              </button>
              <button type="button" style={iconTb} onClick={hist.redo} disabled={!hist.canRedo} aria-label="Redo" title={`Redo (${MOD}+Shift+Z)`}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 7v6h-6M21 13a9 9 0 11-3-7.7L21 8" /></svg>
              </button>
              <button type="button" style={tb} onClick={layout} title="Arrange nodes left to right">Auto layout</button>
            </>
          )}
          <button type="button" style={iconTb} onClick={() => setHelpOpen(true)} aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)">?</button>
          <div style={{ width: 1, height: 20, background: 'var(--border)' }} />
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, cursor: readOnly ? 'default' : 'pointer' }}>
            <input type="checkbox" checked={enabled} disabled={readOnly} onChange={e => setEnabled(e.target.checked)} />
            Enabled
          </label>
          {!readOnly && (
            <>
              <button type="button" style={tb} onClick={() => { setPanelTab('test'); setPanelOpen(true) }}>Test</button>
              <button type="button" style={tb} onClick={runNow} disabled={!saved || runMut.isPending}
                title={saved ? 'Run the saved flow now' : 'Save the flow first'}>
                {runMut.isPending ? 'Running…' : 'Run'}
              </button>
              <button type="button" onClick={save} disabled={saving}
                style={{ ...tb, borderColor: 'var(--green)', color: 'var(--green)', opacity: dirty ? 1 : 0.7 }}>
                {saving ? 'Saving…' : 'Save'}
              </button>
            </>
          )}
        </div>

        {banner && (
          <div role={banner.kind === 'error' ? 'alert' : 'status'} style={{
            display: 'flex', alignItems: 'center', gap: 10, padding: '7px 16px', fontSize: 12.5, flexShrink: 0,
            borderBottom: '1px solid var(--border)',
            color: banner.kind === 'error' ? 'var(--crit)' : banner.kind === 'ok' ? 'var(--green)' : 'var(--fg)',
            background: banner.kind === 'error' ? 'color-mix(in srgb, var(--crit) 8%, transparent)'
              : banner.kind === 'ok' ? 'color-mix(in srgb, var(--green) 8%, transparent)' : 'var(--hover-overlay)',
          }}>
            <span style={{ flex: 1 }}>{banner.text}</span>
            <button type="button" aria-label="Dismiss" onClick={() => setBanner(null)}
              style={{ background: 'none', border: 'none', color: 'inherit', cursor: 'pointer', fontSize: 14 }}>✕</button>
          </div>
        )}

        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          {!readOnly && <Palette nodes={catalogue.nodes} onAdd={t => addNode(t)} />}
          <div style={{ flex: 1, minWidth: 0, position: 'relative' }} onDragOver={onDragOver} onDrop={onDrop} data-testid="flow-canvas">
            <ReactFlow<EdNode, EdEdge>
              nodes={rfNodes}
              edges={rfEdges}
              nodeTypes={nodeTypes}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              onConnect={onConnect}
              isValidConnection={isValidConnection}
              onNodeDragStart={() => { if (!readOnly) hist.checkpoint() }}
              onPaneClick={() => setTriggerSelected(false)}
              nodesDraggable={!readOnly}
              nodesConnectable={!readOnly}
              elementsSelectable
              deleteKeyCode={null}
              selectionKeyCode="Shift"
              multiSelectionKeyCode={isMac ? 'Meta' : 'Control'}
              snapToGrid
              snapGrid={[GRID, GRID]}
              fitView
              fitViewOptions={{ padding: 0.25, maxZoom: 1.1 }}
              minZoom={0.2}
              maxZoom={2}
              colorMode={resolvedTheme}
              defaultEdgeOptions={{ type: 'default' }}
              proOptions={{ hideAttribution: true }}
            >
              <Background variant={BackgroundVariant.Dots} gap={GRID} size={1.2} />
              <MiniMap pannable zoomable
                nodeColor={n => colorFor(n.id === TRIGGER_ID ? 'trigger' : catalogue.nodes.find(d => d.type === (n.data as EdNode['data']).type)?.category)}
                nodeStrokeWidth={0} style={{ width: 160, height: 100 }} />
              <Controls showInteractive={false} />
            </ReactFlow>
            {doc.nodes.length === 0 && (
              <div style={{
                position: 'absolute', left: '50%', top: '58%', transform: 'translateX(-50%)', pointerEvents: 'none',
                fontSize: 13, color: 'var(--muted)', textAlign: 'center', lineHeight: 1.6,
              }}>
                {readOnly ? 'This flow has no nodes.' : <>Drag a node from the palette.<br />The first one is wired to the trigger.</>}
              </div>
            )}
          </div>
          {panel}
        </div>

        <TestPanel
          open={panelOpen} onToggle={() => setPanelOpen(o => !o)} tab={panelTab} onTab={setPanelTab}
          readOnly={readOnly} catalogue={catalogue} nodes={doc.nodes}
          input={input} onInput={s => { setInput(s); setInputTouched(true) }}
          onResetInput={() => { setInput(JSON.stringify(sampleInput(doc.trigger), null, 2)); setInputTouched(false) }}
          onTest={runTest} testing={testMut.isPending} testError={testError}
          source={source} onClearTrace={() => setSource(null)}
          selectedNodeId={triggerSelected ? TRIGGER_ID : single?.id ?? null}
          onSelectNode={id => selectOnly(id)}
          runs={runsQ.data} runsError={runsQ.error ? runsQ.error.message : null}
          onShowRun={(run: FlowRun) => setSource({ kind: 'run', run })}
          saved={saved}
        />

        {helpOpen && <ShortcutsHelp onClose={() => setHelpOpen(false)} />}
      </div>
    </FlowViewContext.Provider>
  )
}

function ShortcutsHelp({ onClose }: { onClose: () => void }) {
  return (
    <div role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" onClick={onClose} style={{
      position: 'absolute', inset: 0, background: 'rgba(0,0,0,.45)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 20,
    }}>
      <div onClick={e => e.stopPropagation()} style={{
        background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 12, padding: 20, width: 420, maxWidth: '90%',
        boxShadow: '0 20px 50px rgba(0,0,0,.35)',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', marginBottom: 12 }}>
          <div style={{ fontSize: 15, fontWeight: 700, flex: 1 }}>Keyboard shortcuts</div>
          <button type="button" aria-label="Close" onClick={onClose} style={{ ...btnStyle, padding: '3px 8px' }}>✕</button>
        </div>
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12.5 }}>
          <tbody>
            {SHORTCUTS.map(([k, v]) => (
              <tr key={k} style={{ borderTop: '1px solid var(--border)' }}>
                <td style={{ padding: '6px 0', whiteSpace: 'nowrap' }}><kbd className="vf-kbd">{k}</kbd></td>
                <td style={{ padding: '6px 0 6px 12px', color: 'var(--muted)' }}>{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
