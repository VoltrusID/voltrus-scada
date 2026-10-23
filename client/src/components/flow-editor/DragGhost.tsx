import { NODE_META, TRIGGER_META } from './constants'
import type { FlowNodeType } from '../../types'

interface Props {
  type: FlowNodeType | string
  isTrigger: boolean
  x: number
  y: number
}

export function DragGhost({ type, isTrigger, x, y }: Props) {
  const meta = isTrigger ? TRIGGER_META[type] : NODE_META[type as FlowNodeType]
  if (!meta) return null

  return (
    <div style={{
      position: 'fixed', left: x + 12, top: y + 12, pointerEvents: 'none', zIndex: 9999,
      opacity: 0.85, transform: 'scale(1.05) rotate(2deg)',
    }}>
      <div style={{
        width: 160, borderRadius: 8, border: `2px solid ${meta.color}`,
        background: 'var(--card)', padding: '10px 14px',
        boxShadow: `0 8px 24px rgba(0,0,0,.4), 0 0 0 1px ${meta.color}44`,
        display: 'flex', alignItems: 'center', gap: 8,
      }}>
        <span style={{ width: 8, height: 8, borderRadius: '50%', background: meta.color, flexShrink: 0 }} />
        <span style={{ fontSize: 12, fontWeight: 700, color: 'var(--fg)' }}>{meta.label}</span>
      </div>
    </div>
  )
}
