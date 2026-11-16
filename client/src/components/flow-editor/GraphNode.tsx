import { createContext, memo, useContext } from 'react'
import { Handle, Position, type NodeProps } from '@xyflow/react'
import type { FlowCatalogue, FlowField, FlowTriggerConfig } from '../../types'
import { CATEGORY_META, colorFor, iconFor } from './catalogue'
import { NODE_W, TRIGGER_ID, fmtDuration, outputsFor, triggerSummary, type EdNode, type Highlight } from './model'

export interface FlowView {
  catalogue: FlowCatalogue
  trigger: FlowTriggerConfig
  highlight: Highlight | null
  /** Field errors per node (missing required values). */
  invalid: Map<string, Record<string, string>>
  /** Error the server returned for a node on save or test. */
  serverError: { nodeId: string; message: string } | null
  readOnly: boolean
}

export const FlowViewContext = createContext<FlowView | null>(null)

function Icon({ d, color, size = 14 }: { d: string; color: string; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d={d} />
    </svg>
  )
}

function preview(config: Record<string, unknown>, fields: FlowField[]): string {
  for (const f of fields) {
    const v = config[f.key]
    if (v === undefined || v === null || v === '' || f.kind === 'bool' || f.kind === 'json') continue
    if (f.kind === 'duration_ms') return `${f.label.toLowerCase()} ${fmtDuration(v)}`
    if (f.kind === 'tag') {
      const ids = Array.isArray(v) ? v : [v]
      if (!ids.length) continue
      return ids.length === 1 ? `tag #${ids[0]}` : `${ids.length} tags`
    }
    if (f.kind === 'code') return String(v).split('\n')[0]
    return typeof v === 'object' ? JSON.stringify(v) : String(v)
  }
  return ''
}

export const GraphNode = memo(function GraphNode({ id, data, selected }: NodeProps<EdNode>) {
  const view = useContext(FlowViewContext)!
  const isTrigger = id === TRIGGER_ID
  const def = isTrigger ? undefined : view.catalogue.nodes.find(n => n.type === data.type)
  const tdef = isTrigger ? view.catalogue.triggers.find(t => t.type === view.trigger.type) : undefined
  const category = isTrigger ? 'trigger' : def?.category ?? 'advanced'
  const color = colorFor(category)
  const label = data.name || (isTrigger ? tdef?.label ?? view.trigger.type : def?.label ?? data.type)
  const subtitle = isTrigger ? 'trigger' : data.type
  const outputs = isTrigger ? ['out'] : outputsFor(data.type, data.config, def)
  const detail = isTrigger
    ? triggerSummary(view.trigger, view.catalogue).split(' · ')[1] ?? ''
    : preview(data.config, def?.fields ?? [])

  const hl = view.highlight
  const ran = hl?.ran.has(id) ?? false
  const chosen = hl?.ports.get(id)
  const runError = hl?.errors.get(id)
  const serverErr = view.serverError?.nodeId === id ? view.serverError.message : undefined
  const fieldErrs = view.invalid.get(id)
  const error = serverErr ?? runError
  const dim = hl && !ran

  const border = error ? 'var(--crit)' : selected ? color : ran ? 'var(--green)' : 'var(--border)'
  const ring = error ? '0 0 0 3px color-mix(in srgb, var(--crit) 25%, transparent)'
    : selected ? `0 0 0 3px color-mix(in srgb, ${color} 28%, transparent)`
    : ran ? '0 0 0 3px color-mix(in srgb, var(--green) 22%, transparent)' : 'none'

  return (
    <div
      data-testid={`flow-node-${id}`}
      data-ran={ran || undefined}
      data-error={error ? true : undefined}
      title={def?.description ?? tdef?.description}
      style={{
        width: NODE_W, borderRadius: 10, background: 'var(--card)', color: 'var(--fg)',
        border: `1px solid ${border}`, boxShadow: `${ring}, var(--shadow)`,
        opacity: dim ? 0.45 : 1, transition: 'opacity .2s, box-shadow .15s, border-color .15s',
        position: 'relative', fontFamily: 'var(--font-ui)',
      }}
    >
      {!isTrigger && (
        <Handle type="target" position={Position.Left} id="in" isConnectable={!view.readOnly}
          style={{ top: 26 }} className="vf-handle vf-handle-in" />
      )}
      <div style={{ display: 'flex', alignItems: 'center', gap: 9, padding: '9px 10px 8px', borderBottom: '1px solid var(--border)' }}>
        <span style={{
          width: 30, height: 30, borderRadius: 7, flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center',
          background: `color-mix(in srgb, ${color} 16%, transparent)`, border: `1px solid color-mix(in srgb, ${color} 35%, transparent)`,
        }}>
          <Icon d={iconFor(isTrigger ? view.trigger.type : data.type, category)} color={color} size={15} />
        </span>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontSize: 12.5, fontWeight: 650, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{label}</div>
          <div style={{ fontSize: 10, color: 'var(--muted)', fontFamily: 'var(--font-mono)', display: 'flex', gap: 6 }}>
            <span style={{ color, textTransform: 'uppercase', letterSpacing: '.04em', fontFamily: 'var(--font-ui)', fontWeight: 600 }}>
              {CATEGORY_META[category]?.label ?? category}
            </span>
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{subtitle}</span>
          </div>
        </div>
        {def?.side_effect && (
          <span title="Has side effects: skipped in test runs" aria-label="side effect" style={{ display: 'flex' }}>
            <Icon d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" color="var(--warn)" size={12} />
          </span>
        )}
        {!def && !isTrigger && (
          <span title="Unknown node type" style={{ fontSize: 10, color: 'var(--warn)', fontWeight: 700 }}>?</span>
        )}
      </div>
      {detail && (
        <div style={{
          padding: '6px 10px 0', fontSize: 10.5, color: 'var(--muted)', fontFamily: 'var(--font-mono)',
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>{detail}</div>
      )}
      <div style={{ padding: '6px 0 6px' }}>
        {outputs.map((name, port) => {
          const hit = chosen?.has(port)
          return (
            <div key={port} style={{ position: 'relative', height: 20, display: 'flex', alignItems: 'center', justifyContent: 'flex-end', paddingRight: 12 }}>
              <span data-testid={`port-${id}-${port}`} style={{
                fontSize: 10.5, fontFamily: 'var(--font-mono)', color: hit ? 'var(--green)' : 'var(--muted)', fontWeight: hit ? 700 : 400,
                maxWidth: NODE_W - 40, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>{name}</span>
              <Handle type="source" position={Position.Right} id={`out-${port}`} isConnectable={!view.readOnly}
                className={`vf-handle${hit ? ' vf-handle-hit' : ''}`} />
            </div>
          )
        })}
      </div>
      {(error || fieldErrs) && (
        <div role="note" title={error ?? Object.values(fieldErrs!).join('\n')} style={{
          borderTop: '1px solid var(--border)', padding: '5px 10px', fontSize: 10.5,
          color: error ? 'var(--crit)' : 'var(--warn)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
          background: `color-mix(in srgb, ${error ? 'var(--crit)' : 'var(--warn)'} 8%, transparent)`, borderRadius: '0 0 10px 10px',
        }}>
          {error ?? Object.values(fieldErrs!)[0]}
        </div>
      )}
    </div>
  )
})
