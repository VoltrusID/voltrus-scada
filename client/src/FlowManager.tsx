import { useState } from 'react'
import { useFlows, useCreateFlow, useUpdateFlow, useDeleteFlow, useStartFlow, useStopFlow } from './queries'
import type { Flow } from './types'
import { btnStyle } from './styles'
import { FlowEditor } from './components/flow-editor/FlowEditor'
import { TRIGGER_META, TRIGGER_DEFAULTS } from './components/flow-editor/constants'

/* ── SVG Edge between two node positions ── */
function Edge({ from, to }: { from: { x: number; y: number }; to: { x: number; y: number } }) {
  const mx = (from.x + to.x) / 2
  const d = `M ${from.x} ${from.y} C ${mx} ${from.y}, ${mx} ${to.y}, ${to.x} ${to.y}`
  return (
    <svg style={{ position: 'absolute', top: 0, left: 0, width: '100%', height: '100%', pointerEvents: 'none', zIndex: 0 }}>
      <defs>
        <marker id="arrowhead" markerWidth="8" markerHeight="6" refX="7" refY="3" orient="auto">
          <polygon points="0 0, 8 3, 0 6" fill="var(--border)" />
        </marker>
      </defs>
      <path d={d} fill="none" stroke="var(--border)" strokeWidth={2} markerEnd="url(#arrowhead)" />
    </svg>
  )
}

/* ── Flow List View ── */
function FlowList({ flows, onEdit, onDelete, onStart, onStop, onCreate }: {
  flows: Flow[]
  onEdit: (f: Flow) => void
  onDelete: (id: string) => void
  onStart: (id: string) => void
  onStop: (id: string) => void
  onCreate: () => void
}) {
  return (
    <div style={{ padding: '24px 32px', maxWidth: 1200, margin: '0 auto' }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 28 }}>
        <div>
          <h2 style={{ fontSize: 22, fontWeight: 700, margin: '0 0 4px', letterSpacing: '-0.5px' }}>Flows</h2>
          <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>Data pipelines that transform, filter, and route sensor data</p>
        </div>
        <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)', fontSize: 13 }} onClick={onCreate}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          New Flow
        </button>
      </div>

      {flows.length === 0 ? (
        <div style={{ border: '1px dashed var(--border)', borderRadius: 12, padding: 60, textAlign: 'center' }}>
          <div style={{ width: 48, height: 48, margin: '0 auto 16px', borderRadius: 12, background: 'var(--card)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--muted)" strokeWidth="1.5"><path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>
          </div>
          <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 6 }}>No flows configured</div>
          <div style={{ fontSize: 13, color: 'var(--muted)' }}>Create a flow to transform data, compute metrics, or send alerts.</div>
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {flows.map(flow => {
            let triggerLabel = 'Unknown'
            let triggerColor = 'var(--muted)'
            let nodeCount = 0
            try {
              const t = JSON.parse(flow.trigger_config)
              triggerLabel = TRIGGER_META[t.type]?.label || t.type
              triggerColor = TRIGGER_META[t.type]?.color || 'var(--muted)'
              nodeCount = JSON.parse(flow.nodes).length
            } catch {}
            return (
              <div key={flow.id} style={{
                display: 'flex', alignItems: 'center', gap: 16, padding: '14px 20px', borderRadius: 10,
                border: '1px solid var(--border)', background: 'var(--card)',
                transition: 'border-color .15s', cursor: 'pointer',
              }} onClick={() => onEdit(flow)}>
                <div style={{
                  width: 10, height: 10, borderRadius: '50%', flexShrink: 0,
                  background: flow.enabled ? 'var(--green)' : 'var(--muted)',
                  boxShadow: flow.enabled ? '0 0 8px rgba(34,197,94,.4)' : 'none',
                }} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 3 }}>{flow.name}</div>
                  <div style={{ fontSize: 11, color: 'var(--muted)', fontFamily: "'var(--font-mono)', monospace", display: 'flex', gap: 12 }}>
                    <span>{flow.id}</span>
                    <span style={{ color: triggerColor }}>{triggerLabel}</span>
                    <span>{nodeCount} node{nodeCount !== 1 ? 's' : ''}</span>
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 6, flexShrink: 0 }} onClick={e => e.stopPropagation()}>
                  {flow.enabled ? (
                    <button style={{ ...btnStyle, padding: '5px 10px', fontSize: 12, color: 'var(--warn)' }} onClick={() => onStop(flow.id)}>Stop</button>
                  ) : (
                    <button style={{ ...btnStyle, padding: '5px 10px', fontSize: 12, color: 'var(--green)' }} onClick={() => onStart(flow.id)}>Start</button>
                  )}
                  <button style={{ ...btnStyle, padding: '5px 10px', fontSize: 12 }} onClick={() => onEdit(flow)}>Edit</button>
                  <button style={{ ...btnStyle, padding: '5px 10px', fontSize: 12, color: 'var(--crit)' }} onClick={() => onDelete(flow.id)}>Delete</button>
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

/* ── Main FlowManager ── */
export function FlowManager({ isAdmin }: { isAdmin: boolean }) {
  const [editingFlow, setEditingFlow] = useState<Flow | null>(null)
  const { data: flows = [] } = useFlows(isAdmin)
  const createFlow = useCreateFlow()
  const updateFlow = useUpdateFlow()
  const deleteFlow = useDeleteFlow()
  const startFlow = useStartFlow()
  const stopFlow = useStopFlow()

  const handleCreate = () => {
    setEditingFlow({
      id: `flow_${Date.now()}`,
      name: 'New Flow',
      enabled: true,
      trigger_config: JSON.stringify(TRIGGER_DEFAULTS.modbus, null, 2),
      nodes: JSON.stringify([], null, 2),
      created_at: 0,
      updated_at: 0,
    })
  }

  const handleSave = async (flow: Flow) => {
    if (flows.find(f => f.id === flow.id)) {
      await updateFlow.mutateAsync(flow)
    } else {
      await createFlow.mutateAsync(flow)
    }
    setEditingFlow(null)
  }

  if (editingFlow) {
    return <FlowEditor flow={editingFlow} onSave={handleSave} onCancel={() => setEditingFlow(null)} />
  }

  return (
    <FlowList
      flows={flows}
      onEdit={setEditingFlow}
      onDelete={id => deleteFlow.mutate(id)}
      onStart={id => startFlow.mutate(id)}
      onStop={id => stopFlow.mutate(id)}
      onCreate={handleCreate}
    />
  )
}
