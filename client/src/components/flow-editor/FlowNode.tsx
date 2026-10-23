import { NODE_META, TRIGGER_META, NODE_WIDTH } from './constants'
import type { FlowNodeConfig } from '../../types'

interface Props {
  node?: FlowNodeConfig
  triggerType?: string
  isTrigger: boolean
  isSelected: boolean
  index: number
  onSelect: () => void
  onRemove?: () => void
  onDragStart?: (index: number) => void
}

export function FlowNode({ node, triggerType, isTrigger, isSelected, index, onSelect, onRemove, onDragStart }: Props) {
  const meta = isTrigger ? TRIGGER_META[triggerType || 'modbus'] : NODE_META[node!.type]
  const color = meta?.color || '#9ca3af'
  const label = meta?.label || node?.type || triggerType || 'Unknown'

  const cardStyle: React.CSSProperties = {
    width: NODE_WIDTH, borderRadius: 10,
    border: `1px solid ${isSelected ? color : 'var(--border)'}`,
    background: 'var(--card)', overflow: 'hidden', cursor: 'pointer',
    boxShadow: isSelected ? `0 0 0 2px ${color}33, 0 4px 12px rgba(0,0,0,.2)` : '0 2px 8px rgba(0,0,0,.15)',
    transition: 'box-shadow .15s, border-color .15s',
  }

  if (isTrigger) {
    return (
      <div style={{ width: NODE_WIDTH }}>
        <div style={cardStyle} onClick={onSelect}>
          <div style={{ height: 4, background: color, borderRadius: '10px 10px 0 0' }} />
          <div style={{ padding: '12px 14px' }}>
            <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: 0.5, fontWeight: 600, marginBottom: 4 }}>Trigger</div>
            <div style={{ fontSize: 12, fontWeight: 700, display: 'flex', alignItems: 'center', gap: 6 }}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="2">
                <path d={meta?.icon || ''} />
              </svg>
              {label}
            </div>
          </div>
        </div>
      </div>
    )
  }

  // Mini config preview value
  const preview = node ? (
    node.condition || node.field || node.output_field || node.url || node.message ||
    (node.map ? JSON.stringify(node.map).slice(0, 30) : '') ||
    (node.table ? JSON.stringify(node.table).slice(0, 30) : '') ||
    ''
  ) : ''

  return (
    <div style={{ width: NODE_WIDTH }}>
      <div
        style={cardStyle}
        onClick={onSelect}
      >
        {/* Drag handle header */}
        <div
          style={{ height: 32, background: `${color}15`, borderBottom: `1px solid ${color}30`, display: 'flex', alignItems: 'center', gap: 8, padding: '0 12px', cursor: onDragStart ? 'grab' : 'pointer' }}
          onMouseDown={e => {
            if (onDragStart) {
              e.stopPropagation()
              onDragStart(index)
            }
          }}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="2">
            <path d={(meta as any).icon} />
          </svg>
          <span style={{ fontSize: 11, fontWeight: 700, flex: 1 }}>{label}</span>
          {onRemove && (
            <button
              style={{ background: 'none', border: 'none', color: 'var(--crit)', cursor: 'pointer', fontSize: 16, padding: '0 2px', lineHeight: 1 }}
              onClick={e => { e.stopPropagation(); onRemove() }}
              title="Remove"
            >&times;</button>
          )}
        </div>
        <div style={{ padding: '10px 12px' }}>
          <div style={{ fontSize: 10, color: 'var(--muted)', marginBottom: 6, lineHeight: 1.4 }}>{(meta as any).desc}</div>
          {preview && (
            <div style={{ fontSize: 10, color: 'var(--muted)', fontFamily: "'var(--font-mono)', monospace", whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {preview}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
