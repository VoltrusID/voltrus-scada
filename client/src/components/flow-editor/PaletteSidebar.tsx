import { useState } from 'react'
import { PALETTE_CATEGORIES } from './constants'
import type { FlowNodeType } from '../../types'

interface Props {
  onDragStart: (type: FlowNodeType, isTrigger: boolean) => void
}

export function PaletteSidebar({ onDragStart }: Props) {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    input: true, process: true, output: true,
  })

  return (
    <div style={{
      width: 220, flexShrink: 0, borderRight: '1px solid var(--border)',
      background: 'var(--card)', display: 'flex', flexDirection: 'column', overflow: 'auto',
    }}>
      <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--border)' }}>
        <div style={{ fontSize: 11, fontWeight: 700, textTransform: 'uppercase', letterSpacing: 1, color: 'var(--muted)' }}>Palette</div>
      </div>
      {PALETTE_CATEGORIES.map(cat => (
        <div key={cat.key} style={{ borderBottom: '1px solid var(--border)' }}>
          <button
            style={{
              width: '100%', padding: '10px 16px', background: 'none', border: 'none',
              color: 'var(--fg)', fontSize: 11, fontWeight: 700, textTransform: 'uppercase',
              letterSpacing: 0.5, cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            }}
            onClick={() => setExpanded(e => ({ ...e, [cat.key]: !e[cat.key] }))}
          >
            {cat.label}
            <span style={{ color: 'var(--muted)', fontSize: 10 }}>{expanded[cat.key] ? '▼' : '▶'}</span>
          </button>
          {expanded[cat.key] && (
            <div style={{ padding: '4px 12px 12px' }}>
              {cat.items.map(item => (
                <div
                  key={item.type}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 10, padding: '8px 10px',
                    borderRadius: 6, cursor: 'grab', userSelect: 'none',
                    transition: 'background .15s',
                  }}
                  onMouseDown={() => onDragStart(item.type as FlowNodeType, item.isTrigger)}
                  onMouseEnter={e => { e.currentTarget.style.background = 'var(--bg)' }}
                  onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
                >
                  <span style={{ width: 8, height: 8, borderRadius: '50%', background: item.color, flexShrink: 0 }} />
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={item.color} strokeWidth="2">
                    <path d={item.icon} />
                  </svg>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>{item.label}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      ))}
    </div>
  )
}
