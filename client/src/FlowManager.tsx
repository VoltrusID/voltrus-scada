import { useMemo, useState } from 'react'
import {
  useFlows, useCreateFlow, useUpdateFlow, useDeleteFlow, useStartFlow, useStopFlow, useFlowNodeTypes, useFlowRuns,
} from './queries'
import type { Flow, FlowCatalogue } from './types'
import { btnStyle, inputStyle } from './styles'
import { duplicateName } from './duplicateName'
import { FlowEditor } from './components/flow-editor/FlowEditor'
import { FALLBACK_CATALOGUE, colorFor, defaultConfig, iconFor } from './components/flow-editor/catalogue'
import { countNodes, parseTrigger, triggerSummary } from './components/flow-editor/model'
import { fmtWhen } from './components/flow-editor/TestPanel'

const small: React.CSSProperties = { ...btnStyle, padding: '5px 10px', fontSize: 12 }

export function newFlow(cat: FlowCatalogue): Flow {
  const t = cat.triggers[0]
  return {
    id: `flow_${Date.now()}`,
    name: 'New flow',
    enabled: false,
    trigger_config: JSON.stringify({ ...defaultConfig(t?.fields ?? []), type: t?.type ?? 'timer' }),
    nodes: JSON.stringify({ version: 2, nodes: [], edges: [] }),
    created_at: 0,
    updated_at: 0,
  }
}

function LastRun({ flowId }: { flowId: string }) {
  const { data, error } = useFlowRuns(flowId, true, { refetchInterval: 30_000 })
  if (error) return null
  const run = data?.[0]
  if (!run) return <span style={{ color: 'var(--muted)' }}>never run</span>
  return (
    <span title={run.error ?? undefined} style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
      <span style={{ width: 6, height: 6, borderRadius: '50%', background: run.success ? 'var(--green)' : 'var(--crit)' }} />
      <span style={{ color: run.success ? 'var(--muted)' : 'var(--crit)' }}>{run.success ? 'ok' : 'failed'} {fmtWhen(run.started_at)}</span>
    </span>
  )
}

function FlowRow({ flow, cat, canEdit, canRun, onEdit, onDuplicate, onDelete, onStart, onStop }: {
  flow: Flow; cat: FlowCatalogue; canEdit: boolean
  /** Start and stop: operators too, not only those who may edit. */
  canRun: boolean
  onEdit: () => void; onDuplicate: () => void; onDelete: () => void; onStart: () => void; onStop: () => void
}) {
  const trigger = parseTrigger(flow.trigger_config)
  const nodes = countNodes(flow.nodes)
  return (
    <div role="row" onClick={onEdit} className="vf-flow-row" style={{
      display: 'grid', gridTemplateColumns: '36px minmax(160px, 1.4fr) minmax(140px, 1fr) 80px minmax(110px, .8fr) 340px',
      alignItems: 'center', gap: 14, padding: '10px 14px', borderBottom: '1px solid var(--border)', cursor: 'pointer',
    }}>
      <span style={{
        width: 30, height: 30, borderRadius: 7, display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: `color-mix(in srgb, ${colorFor('trigger')} 14%, transparent)`,
      }}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={colorFor('trigger')} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={iconFor(trigger.type, 'trigger')} /></svg>
      </span>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 13.5, fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{flow.name}</div>
        <div style={{ fontSize: 11, color: 'var(--muted)', fontFamily: 'var(--font-mono)' }}>{flow.id}</div>
      </div>
      <div style={{ fontSize: 12, color: 'var(--muted)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {triggerSummary(trigger, cat)}
      </div>
      <div style={{ fontSize: 12, color: 'var(--muted)' }}>{nodes} node{nodes === 1 ? '' : 's'}</div>
      <div style={{ fontSize: 12 }}><LastRun flowId={flow.id} /></div>
      <div style={{ display: 'flex', gap: 6, alignItems: 'center', justifyContent: 'flex-end' }} onClick={e => e.stopPropagation()}>
        <span style={{
          fontSize: 11, fontWeight: 600, padding: '2px 8px', borderRadius: 999, marginRight: 4,
          color: flow.enabled ? 'var(--green)' : 'var(--muted)',
          background: flow.enabled ? 'color-mix(in srgb, var(--green) 12%, transparent)' : 'var(--hover-overlay)',
        }}>{flow.enabled ? 'Running' : 'Stopped'}</span>
        {canRun && (flow.enabled
          ? <button type="button" style={{ ...small, color: 'var(--warn)' }} onClick={onStop}>Stop</button>
          : <button type="button" style={{ ...small, color: 'var(--green)' }} onClick={onStart}>Start</button>)}
        <button type="button" style={small} onClick={onEdit}>{canEdit ? 'Edit' : 'View'}</button>
        {canEdit && <button type="button" style={small} onClick={onDuplicate}>Duplicate</button>}
        {canEdit && <button type="button" style={{ ...small, color: 'var(--crit)' }} onClick={onDelete}>Delete</button>}
      </div>
    </div>
  )
}

/**
 * Flows list and graph editor. `isAdmin` (admin or demo role) may edit; other
 * roles get a read-only view of the list and canvas. Only `isStrictAdmin`
 * (admin) may create webhook tokens.
 */
export function FlowManager({ isAdmin, isStrictAdmin = isAdmin, canOperate = isAdmin }: { isAdmin: boolean; isStrictAdmin?: boolean; canOperate?: boolean }) {
  const canEdit = isAdmin
  const [editing, setEditing] = useState<{ flow: Flow; saved: boolean } | null>(null)
  const [q, setQ] = useState('')
  const flowsQ = useFlows(true)
  const flows = flowsQ.data ?? []
  const catQ = useFlowNodeTypes()
  const cat = catQ.data ?? FALLBACK_CATALOGUE
  const createFlow = useCreateFlow()
  const updateFlow = useUpdateFlow()
  const deleteFlow = useDeleteFlow()
  const startFlow = useStartFlow()
  const stopFlow = useStopFlow()

  const shown = useMemo(() => {
    const n = q.trim().toLowerCase()
    return n ? flows.filter(f => f.name.toLowerCase().includes(n) || f.id.toLowerCase().includes(n)) : flows
  }, [flows, q])

  if (editing) {
    if (catQ.isLoading) return <div style={{ padding: 32, color: 'var(--muted)', fontSize: 13 }}>Loading node types…</div>
    return (
      <FlowEditor
        key={editing.flow.id}
        flow={editing.flow}
        saved={editing.saved}
        catalogue={cat}
        readOnly={!canEdit}
        canManageWebhook={isStrictAdmin}
        onSave={async f => {
          if (editing.saved) await updateFlow.mutateAsync(f)
          else {
            await createFlow.mutateAsync(f)
            // Later saves update it; the editor keeps its state (key is the id).
            setEditing({ flow: editing.flow, saved: true })
          }
        }}
        onClose={() => setEditing(null)}
      />
    )
  }

  const running = flows.filter(f => f.enabled).length
  return (
    <div style={{ padding: '24px 32px', maxWidth: 1240, margin: '0 auto' }}>
      <div style={{ display: 'flex', alignItems: 'flex-end', gap: 16, marginBottom: 20, flexWrap: 'wrap' }}>
        <div style={{ flex: 1, minWidth: 220 }}>
          <h2 style={{ fontSize: 22, fontWeight: 700, margin: '0 0 4px', letterSpacing: '-0.4px' }}>Flows</h2>
          <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>
            Event-driven graphs that filter, route and act on plant data.
            {flows.length > 0 && <> {flows.length} flow{flows.length === 1 ? '' : 's'}, {running} running.</>}
          </p>
        </div>
        {flows.length > 0 && (
          <input type="search" aria-label="Search flows" placeholder="Search flows…" value={q} onChange={e => setQ(e.target.value)}
            style={{ ...inputStyle, width: 220 }} />
        )}
        {canEdit && (
          <button type="button" style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)', fontSize: 13 }}
            onClick={() => setEditing({ flow: newFlow(cat), saved: false })}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 5v14M5 12h14" /></svg>
            New flow
          </button>
        )}
      </div>

      {flowsQ.error ? (
        <div role="alert" style={{ border: '1px solid var(--border)', borderRadius: 10, padding: 24, fontSize: 13, color: 'var(--muted)' }}>
          Flows could not be loaded: {flowsQ.error.message}
        </div>
      ) : flowsQ.isLoading ? (
        <div style={{ padding: 24, fontSize: 13, color: 'var(--muted)' }}>Loading flows…</div>
      ) : flows.length === 0 ? (
        <div style={{ border: '1px dashed var(--border)', borderRadius: 12, padding: 56, textAlign: 'center' }}>
          <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 6 }}>No flows yet</div>
          <div style={{ fontSize: 13, color: 'var(--muted)' }}>
            {canEdit ? 'Create a flow to react to device data: filter it, branch on rules, and raise alarms or write values.'
              : 'No flows have been set up.'}
          </div>
        </div>
      ) : (
        <div role="table" aria-label="Flows" style={{ border: '1px solid var(--border)', borderRadius: 10, background: 'var(--card)', overflow: 'hidden' }}>
          <div role="row" style={{
            display: 'grid', gridTemplateColumns: '36px minmax(160px, 1.4fr) minmax(140px, 1fr) 80px minmax(110px, .8fr) 340px',
            gap: 14, padding: '8px 14px', borderBottom: '1px solid var(--border)', fontSize: 10.5, fontWeight: 700,
            textTransform: 'uppercase', letterSpacing: '.06em', color: 'var(--muted)',
          }}>
            <span /><span>Name</span><span>Trigger</span><span>Nodes</span><span>Last run</span><span style={{ textAlign: 'right' }}>Status</span>
          </div>
          {shown.map(f => (
            <FlowRow key={f.id} flow={f} cat={cat} canEdit={canEdit} canRun={canOperate}
              onEdit={() => setEditing({ flow: f, saved: true })}
              onDuplicate={() => setEditing({
                flow: { ...f, id: `flow_${Date.now()}`, name: duplicateName(f.name), enabled: false, created_at: 0, updated_at: 0 },
                saved: false,
              })}
              onDelete={() => { if (confirm(`Delete flow “${f.name}”? This cannot be undone.`)) deleteFlow.mutate(f.id) }}
              onStart={() => startFlow.mutate(f.id)}
              onStop={() => stopFlow.mutate(f.id)} />
          ))}
          {shown.length === 0 && <div style={{ padding: 18, fontSize: 13, color: 'var(--muted)' }}>No flow matches “{q}”.</div>}
        </div>
      )}
      <style>{'.vf-flow-row:hover{background:var(--hover-overlay)}.vf-flow-row:last-child{border-bottom:none!important}'}</style>
    </div>
  )
}
