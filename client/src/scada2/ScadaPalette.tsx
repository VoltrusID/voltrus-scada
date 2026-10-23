import React, { useMemo, useState } from 'react'
import type { SensorConfig } from '../types'
import { SYMBOLS, SYMBOL_CATEGORIES } from './hmi/symbols'
import { SymbolThumb } from './hmi/SymbolView'
import { WIDGET_MANIFESTS } from './widgets/registry'
import type { ScadaWidgetType, WidgetManifest } from './widgets/types'
import { DND_SYMBOL, DND_TAG, DND_WIDGET } from './ScadaCanvas'

// Widget types that now render as library symbols; offered from the Symbols tab instead.
const SYMBOL_BACKED = new Set<ScadaWidgetType>(['symbol', 'pump', 'motor', 'valve', 'tank_level', 'horizontal_tank', 'pipe'])

type Tab = 'symbols' | 'widgets' | 'tags'

interface Props {
  sensors: SensorConfig[]
  onAddSymbol: (key: string) => void
  onAddWidget: (type: ScadaWidgetType) => void
  onPickTag: (sensorId: number) => void
}

export const PANEL_WIDTH = 252

export const ScadaPalette: React.FC<Props> = ({ sensors, onAddSymbol, onAddWidget, onPickTag }) => {
  const [tab, setTab] = useState<Tab>('symbols')
  const [search, setSearch] = useState('')
  const q = search.trim().toLowerCase()

  const symbols = useMemo(() => SYMBOLS.filter(s => !q || s.name.toLowerCase().includes(q) || s.category.toLowerCase().includes(q)), [q])
  const widgets = useMemo(() => (Object.values(WIDGET_MANIFESTS) as WidgetManifest[])
    .filter(m => !SYMBOL_BACKED.has(m.type) && (!q || m.displayName.toLowerCase().includes(q))), [q])
  const tags = useMemo(() => sensors.filter(s => !q || s.name.toLowerCase().includes(q) || s.key.toLowerCase().includes(q)), [sensors, q])

  return (
    <div style={{
      position: 'absolute', top: 0, left: 0, bottom: 0, width: PANEL_WIDTH, zIndex: 20,
      background: 'var(--card)', borderRight: '1px solid var(--border)', display: 'flex', flexDirection: 'column',
    }}>
      <div style={{ display: 'flex', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
        {(['symbols', 'widgets', 'tags'] as Tab[]).map(t => (
          <button key={t} onClick={() => setTab(t)} style={{
            flex: 1, padding: '9px 0 8px', background: 'none', border: 'none', cursor: 'pointer',
            fontSize: 11, fontWeight: 600, letterSpacing: '.04em', textTransform: 'uppercase',
            color: tab === t ? 'var(--fg)' : 'var(--muted)',
            boxShadow: tab === t ? 'inset 0 -2px 0 var(--cyan)' : 'none',
          }}>{t}</button>
        ))}
      </div>
      <div style={{ padding: 8, flexShrink: 0 }}>
        <input
          type="search"
          placeholder={tab === 'tags' ? 'Search tags…' : tab === 'symbols' ? 'Search symbols…' : 'Search widgets…'}
          value={search}
          onChange={e => setSearch(e.target.value)}
          style={{
            width: '100%', padding: '6px 8px', borderRadius: 5, border: '1px solid var(--border)',
            background: 'var(--input-bg)', color: 'var(--fg)', fontSize: 12, outline: 'none',
          }}
        />
      </div>
      <div style={{ flex: 1, overflowY: 'auto', padding: '0 8px 12px' }}>
        {tab === 'symbols' && SYMBOL_CATEGORIES.map(cat => {
          const items = symbols.filter(s => s.category === cat)
          if (items.length === 0) return null
          return (
            <div key={cat} style={{ marginBottom: 10 }}>
              <SectionLabel>{cat}</SectionLabel>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 4 }}>
                {items.map(s => (
                  <Tile key={s.key} title={`${s.name} — drag onto the screen or click to add`}
                    onClick={() => onAddSymbol(s.key)}
                    onDragStart={e => { e.dataTransfer.setData(DND_SYMBOL, s.key); e.dataTransfer.effectAllowed = 'copy' }}>
                    <SymbolThumb def={s} size={40} />
                    <span style={{ fontSize: 10, color: 'var(--text-dim)', lineHeight: 1.15, textAlign: 'center' }}>{s.name}</span>
                  </Tile>
                ))}
              </div>
            </div>
          )
        })}

        {tab === 'widgets' && (
          <>
            <Hint>Widgets need a tag. Drop one, then bind it in the inspector.</Hint>
            {widgets.map(m => (
              <Row key={m.type}
                onClick={() => onAddWidget(m.type)}
                onDragStart={e => { e.dataTransfer.setData(DND_WIDGET, m.type); e.dataTransfer.effectAllowed = 'copy' }}>
                <span style={{ width: 22, textAlign: 'center', fontSize: 14, color: 'var(--text-dim)' }}>{m.icon}</span>
                <span style={{ flex: 1, fontSize: 12 }}>{m.displayName}</span>
                <span style={{ fontSize: 10, color: 'var(--muted)' }}>{m.compatibleDataTypes.join(' · ')}</span>
              </Row>
            ))}
          </>
        )}

        {tab === 'tags' && (
          <>
            <Hint>Drag a tag onto the screen to pick a visualization.</Hint>
            {tags.map(s => (
              <Row key={s.id}
                onClick={() => onPickTag(s.id)}
                onDragStart={e => { e.dataTransfer.setData(DND_TAG, String(s.id)); e.dataTransfer.effectAllowed = 'copy' }}>
                <span style={{ width: 8, height: 8, borderRadius: 2, flexShrink: 0, background: s.color || 'var(--muted)' }} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{s.name}</div>
                  <div style={{ fontSize: 10, color: 'var(--muted)', fontFamily: 'var(--font-mono)' }}>{s.key}{s.unit ? ` · ${s.unit}` : ''}</div>
                </div>
              </Row>
            ))}
            {tags.length === 0 && <Hint>No tags found.</Hint>}
          </>
        )}
      </div>
    </div>
  )
}

const SectionLabel = ({ children }: { children: React.ReactNode }) => (
  <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.06em', margin: '4px 2px 6px' }}>
    {children}
  </div>
)

const Hint = ({ children }: { children: React.ReactNode }) => (
  <div style={{ fontSize: 11, color: 'var(--muted)', margin: '2px 2px 8px', lineHeight: 1.4 }}>{children}</div>
)

function Tile({ children, title, onClick, onDragStart }: {
  children: React.ReactNode; title: string; onClick: () => void; onDragStart: (e: React.DragEvent) => void
}) {
  return (
    <div
      draggable onDragStart={onDragStart} onClick={onClick} title={title}
      style={{
        display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 4,
        padding: '8px 2px 6px', minHeight: 76, borderRadius: 6, cursor: 'grab',
        border: '1px solid var(--border)', background: 'var(--hmi-stage)',
      }}
      onMouseEnter={e => { e.currentTarget.style.borderColor = 'var(--cyan)' }}
      onMouseLeave={e => { e.currentTarget.style.borderColor = 'var(--border)' }}
    >
      {children}
    </div>
  )
}

function Row({ children, onClick, onDragStart }: { children: React.ReactNode; onClick: () => void; onDragStart: (e: React.DragEvent) => void }) {
  return (
    <div
      draggable onDragStart={onDragStart} onClick={onClick}
      style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 6px', borderRadius: 5, cursor: 'grab' }}
      onMouseEnter={e => { e.currentTarget.style.background = 'var(--hover-overlay)' }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
    >
      {children}
    </div>
  )
}
