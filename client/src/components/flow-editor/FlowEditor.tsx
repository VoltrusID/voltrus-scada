import { useState, useMemo, useEffect, useCallback } from 'react'
import { PaletteSidebar } from './PaletteSidebar'
import { CanvasArea } from './CanvasArea'
import { ConfigPanel } from './ConfigPanel'
import { DragGhost } from './DragGhost'
import { NODE_META, TRIGGER_DEFAULTS } from './constants'
import { useTestFlow } from '../../queries'
import type { Flow, FlowNodeConfig, FlowNodeType } from '../../types'
import { btnStyle } from '../../styles'

interface Props {
  flow: Flow
  onSave: (f: Flow) => void
  onCancel: () => void
}

export function FlowEditor({ flow, onSave, onCancel }: Props) {
  const [form, setForm] = useState<Flow>({ ...flow })
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null) // null = trigger, >=0 = node
  const [error, setError] = useState('')

  // Drag state
  const [dragState, setDragState] = useState<{
    type: 'palette' | 'reorder'
    nodeType?: FlowNodeType | string
    isTrigger?: boolean
    fromIndex?: number
  } | null>(null)
  const [ghostPos, setGhostPos] = useState({ x: 0, y: 0 })

  const [testPayload, setTestPayload] = useState('{"metrics": {"temp": 85}}')
  const [testResult, setTestResult] = useState<any>(null)
  const [testLoading, setTestLoading] = useState(false)
  const testFlow = useTestFlow()

  const parsedNodes = useMemo(() => {
    try { return JSON.parse(form.nodes) as FlowNodeConfig[] } catch { return [] }
  }, [form.nodes])

  const parsedTrigger = useMemo(() => {
    try { return JSON.parse(form.trigger_config) as any } catch { return { type: 'modbus' } }
  }, [form.trigger_config])

  // Document-level mouse move/up for drag
  useEffect(() => {
    if (!dragState) return
    const handleMove = (e: MouseEvent) => setGhostPos({ x: e.clientX, y: e.clientY })
    const handleUp = () => setDragState(null)
    document.addEventListener('mousemove', handleMove)
    document.addEventListener('mouseup', handleUp)
    return () => {
      document.removeEventListener('mousemove', handleMove)
      document.removeEventListener('mouseup', handleUp)
    }
  }, [dragState])

  const handleSave = () => {
    setError('')
    try { JSON.parse(form.trigger_config); JSON.parse(form.nodes) } catch { setError('Invalid JSON'); return }
    if (!form.id || !form.name) { setError('ID and Name required'); return }
    onSave({ ...form, updated_at: Math.floor(Date.now() / 1000) })
  }

  const addNode = useCallback((type: FlowNodeType | string, isTrigger: boolean, insertIndex: number) => {
    if (isTrigger) {
      setForm(f => ({ ...f, trigger_config: JSON.stringify(TRIGGER_DEFAULTS[type] || { type }, null, 2) }))
      setSelectedIndex(null)
    } else {
      const meta = NODE_META[type as FlowNodeType]
      if (!meta) return
      const newNode: FlowNodeConfig = { id: `n${Date.now()}`, type: type as FlowNodeType, ...meta.defaultConfig }
      const idx = Math.min(insertIndex, parsedNodes.length)
      const updated = [...parsedNodes.slice(0, idx), newNode, ...parsedNodes.slice(idx)]
      setForm(f => ({ ...f, nodes: JSON.stringify(updated, null, 2) }))
      setSelectedIndex(idx)
    }
  }, [parsedNodes])

  // Handle drop from palette
  const handlePaletteDrop = useCallback((insertIndex: number) => {
    if (!dragState || dragState.type !== 'palette' || !dragState.nodeType) return
    addNode(dragState.nodeType, !!dragState.isTrigger, insertIndex)
    setDragState(null)
  }, [dragState, addNode])

  // Handle reorder drop
  const handleReorderDrop = useCallback((insertIndex: number) => {
    if (!dragState || dragState.type !== 'reorder' || dragState.fromIndex === undefined) return
    const from = dragState.fromIndex
    if (from === insertIndex || from + 1 === insertIndex) {
      setDragState(null)
      return
    }
    const item = parsedNodes[from]
    const without = parsedNodes.filter((_, i) => i !== from)
    const targetIdx = insertIndex > from ? insertIndex - 1 : insertIndex
    const updated = [...without.slice(0, targetIdx), item, ...without.slice(targetIdx)]
    setForm(f => ({ ...f, nodes: JSON.stringify(updated, null, 2) }))
    setSelectedIndex(targetIdx)
    setDragState(null)
  }, [dragState, parsedNodes])

  const handleDrop = useCallback((insertIndex: number) => {
    if (!dragState) return
    if (dragState.type === 'palette') handlePaletteDrop(insertIndex)
    else if (dragState.type === 'reorder') handleReorderDrop(insertIndex)
  }, [dragState, handlePaletteDrop, handleReorderDrop])

  const removeNode = (index: number) => {
    const updated = parsedNodes.filter((_, i) => i !== index)
    setForm(f => ({ ...f, nodes: JSON.stringify(updated, null, 2) }))
    if (selectedIndex === index) setSelectedIndex(null)
    else if (selectedIndex !== null && selectedIndex > index) setSelectedIndex(selectedIndex - 1)
  }

  const updateNode = (index: number, key: string, value: any) => {
    const updated = parsedNodes.map((n, i) => i === index ? { ...n, [key]: value } : n)
    setForm(f => ({ ...f, nodes: JSON.stringify(updated, null, 2) }))
  }

  const updateTrigger = (key: string, value: any) => {
    const updated = { ...parsedTrigger, [key]: value }
    setForm(f => ({ ...f, trigger_config: JSON.stringify(updated, null, 2) }))
  }

  const handleTriggerTypeChange = (type: string) => {
    setForm(f => ({ ...f, trigger_config: JSON.stringify(TRIGGER_DEFAULTS[type] || { type }, null, 2) }))
  }

  const handleTest = async () => {
    if (!form.id) return
    setTestLoading(true)
    setTestResult(null)
    try {
      const payload = JSON.parse(testPayload)
      const result = await testFlow.mutateAsync({ id: form.id, payload })
      setTestResult(result)
    } catch (e: any) {
      setTestResult({ error: e.message })
    }
    setTestLoading(false)
  }

  const selectedNode = selectedIndex !== null ? parsedNodes[selectedIndex] : null

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      {/* Toolbar */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '12px 20px', borderBottom: '1px solid var(--border)', flexShrink: 0, background: 'var(--card)' }}>
        <button style={btnStyle} onClick={onCancel}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7"/></svg>
        </button>
        <div style={{ flex: 1 }}>
          <input
            style={{ background: 'transparent', border: 'none', color: 'var(--fg)', fontSize: 16, fontWeight: 700, padding: 0, fontFamily: 'inherit', width: '100%' }}
            value={form.name}
            onChange={e => setForm(f => ({ ...f, name: e.target.value }))}
            placeholder="Flow name"
          />
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontSize: 12, color: 'var(--muted)' }}>{form.id}</span>
          <div style={{ width: 1, height: 20, background: 'var(--border)' }} />
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, cursor: 'pointer' }}>
            <input type="checkbox" checked={form.enabled} onChange={e => setForm(f => ({ ...f, enabled: e.target.checked }))} />
            Enabled
          </label>
          <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} onClick={handleSave}>Save</button>
        </div>
      </div>

      {error && (
        <div style={{ background: 'rgba(239,68,68,.1)', borderBottom: '1px solid var(--crit)', color: 'var(--crit)', padding: '10px 24px', fontSize: 13, flexShrink: 0 }}>
          {error}
        </div>
      )}

      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
        <PaletteSidebar onDragStart={(type, isTrigger) => {
          setDragState({ type: 'palette', nodeType: type, isTrigger })
        }} />

        <CanvasArea
          trigger={parsedTrigger}
          nodes={parsedNodes}
          selectedIndex={selectedIndex}
          onSelectTrigger={() => setSelectedIndex(null)}
          onSelectNode={setSelectedIndex}
          onRemoveNode={removeNode}
          onDragNodeStart={(index) => setDragState({ type: 'reorder', fromIndex: index })}
          onDrop={handleDrop}
          isDragOver={!!dragState}
        />

        <ConfigPanel
          node={selectedNode}
          trigger={parsedTrigger}
          onNodeChange={(k, v) => selectedIndex !== null && updateNode(selectedIndex, k, v)}
          onTriggerChange={updateTrigger}
          onTriggerTypeChange={handleTriggerTypeChange}
        />
      </div>

      {/* Test panel (collapsible, below main area) */}
      <div style={{ borderTop: '1px solid var(--border)', background: 'var(--card)', flexShrink: 0 }}>
        <details style={{ padding: '0 20px' }}>
          <summary style={{ padding: '10px 0', cursor: 'pointer', fontSize: 12, fontWeight: 600, color: 'var(--muted)', userSelect: 'none' }}>Test Flow</summary>
          <div style={{ padding: '0 0 16px', display: 'flex', gap: 16, alignItems: 'flex-start' }}>
            <div style={{ flex: 1 }}>
              <label style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: 0.5, marginBottom: 4, display: 'block', fontWeight: 600 }}>Payload JSON</label>
              <textarea
                style={{ background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6, padding: '8px 10px', color: 'var(--fg)', fontSize: 12, fontFamily: "'var(--font-mono)', monospace", width: '100%', minHeight: 80, resize: 'vertical' }}
                value={testPayload}
                onChange={e => setTestPayload(e.target.value)}
              />
            </div>
            <div>
              <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} onClick={handleTest} disabled={testLoading || !form.id}>
                {testLoading ? 'Running...' : 'Run Test'}
              </button>
            </div>
            {testResult && (
              <div style={{ flex: 1, background: 'var(--bg)', borderRadius: 8, padding: 12, fontSize: 11, fontFamily: "'var(--font-mono)', monospace", overflow: 'auto', maxHeight: 120 }}>
                {testResult.error ? (
                  <div style={{ color: 'var(--crit)' }}>{testResult.error}</div>
                ) : (
                  <pre style={{ margin: 0, whiteSpace: 'pre-wrap' }}>{JSON.stringify(testResult, null, 2)}</pre>
                )}
              </div>
            )}
          </div>
        </details>
      </div>

      {/* Drag ghost overlay */}
      {dragState && dragState.nodeType && (
        <DragGhost
          type={dragState.nodeType}
          isTrigger={!!dragState.isTrigger}
          x={ghostPos.x}
          y={ghostPos.y}
        />
      )}
    </div>
  )
}
