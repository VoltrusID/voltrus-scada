import { useMemo, useState } from 'react'
import type { FlowNodeTypeDef } from '../../types'
import { inputStyle } from '../../styles'
import { CATEGORY_META, CATEGORY_ORDER, colorFor, iconFor } from './catalogue'

export const DND_TYPE = 'application/x-voltrus-flow-node'

interface Props {
  nodes: FlowNodeTypeDef[]
  /** Click (or Enter) adds the node at the centre of the view. */
  onAdd: (type: string) => void
}

export function Palette({ nodes, onAdd }: Props) {
  const [q, setQ] = useState('')
  const [closed, setClosed] = useState<Record<string, boolean>>({})

  const groups = useMemo(() => {
    const needle = q.trim().toLowerCase()
    const hit = (n: FlowNodeTypeDef) => !needle
      || n.label.toLowerCase().includes(needle) || n.type.toLowerCase().includes(needle)
      || (n.description ?? '').toLowerCase().includes(needle)
    const cats = [...CATEGORY_ORDER, ...new Set(nodes.map(n => n.category).filter(c => !(CATEGORY_ORDER as readonly string[]).includes(c)))]
    return cats.map(c => ({ cat: c, items: nodes.filter(n => n.category === c && hit(n)) })).filter(g => g.items.length)
  }, [nodes, q])

  return (
    <aside aria-label="Node palette" style={{
      width: 232, flexShrink: 0, borderRight: '1px solid var(--border)', background: 'var(--card)',
      display: 'flex', flexDirection: 'column', minHeight: 0,
    }}>
      <div style={{ padding: 10, borderBottom: '1px solid var(--border)' }}>
        <input type="search" aria-label="Search nodes" placeholder="Search nodes…" value={q}
          onChange={e => setQ(e.target.value)} style={{ ...inputStyle, fontSize: 12 }} />
      </div>
      <div style={{ overflow: 'auto', flex: 1, padding: '4px 0 12px' }}>
        {groups.length === 0 && <div style={{ padding: 16, fontSize: 12, color: 'var(--muted)' }}>No node matches “{q}”.</div>}
        {groups.map(({ cat, items }) => {
          const meta = CATEGORY_META[cat]
          const isClosed = closed[cat] && !q
          return (
            <section key={cat}>
              <button type="button" aria-expanded={!isClosed}
                onClick={() => setClosed(s => ({ ...s, [cat]: !s[cat] }))}
                style={{
                  width: '100%', display: 'flex', alignItems: 'center', gap: 8, padding: '10px 14px 6px',
                  background: 'none', border: 'none', cursor: 'pointer', color: 'var(--muted)',
                  fontSize: 10.5, fontWeight: 700, textTransform: 'uppercase', letterSpacing: '.06em', fontFamily: 'var(--font-ui)',
                }}>
                <span style={{ width: 6, height: 6, borderRadius: 2, background: colorFor(cat) }} />
                <span style={{ flex: 1, textAlign: 'left' }}>{meta?.label ?? cat}</span>
                <span style={{ fontWeight: 500 }}>{items.length}</span>
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"
                  style={{ transform: isClosed ? 'rotate(-90deg)' : 'none', transition: 'transform .15s' }}><path d="M6 9l6 6 6-6" /></svg>
              </button>
              {!isClosed && items.map(n => (
                <PaletteItem key={n.type} def={n} onAdd={onAdd} />
              ))}
            </section>
          )
        })}
      </div>
      <div style={{ padding: '8px 12px', borderTop: '1px solid var(--border)', fontSize: 11, color: 'var(--muted)' }}>
        Drag onto the canvas, or click to add.
      </div>
    </aside>
  )
}

function PaletteItem({ def, onAdd }: { def: FlowNodeTypeDef; onAdd: (t: string) => void }) {
  const color = colorFor(def.category)
  return (
    <div
      role="button"
      tabIndex={0}
      draggable
      data-testid={`palette-${def.type}`}
      title={def.description}
      onDragStart={e => { e.dataTransfer.setData(DND_TYPE, def.type); e.dataTransfer.effectAllowed = 'copy' }}
      onClick={() => onAdd(def.type)}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onAdd(def.type) } }}
      className="vf-palette-item"
      style={{
        margin: '1px 8px', padding: '6px 8px', borderRadius: 7, display: 'flex', alignItems: 'center', gap: 9,
        cursor: 'grab', userSelect: 'none',
      }}
    >
      <span style={{
        width: 24, height: 24, borderRadius: 6, flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: `color-mix(in srgb, ${color} 15%, transparent)`,
      }}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d={iconFor(def.type, def.category)} /></svg>
      </span>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 12.5, fontWeight: 600 }}>{def.label}</div>
        {def.description && <div style={{ fontSize: 10.5, color: 'var(--muted)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{def.description}</div>}
      </div>
    </div>
  )
}
