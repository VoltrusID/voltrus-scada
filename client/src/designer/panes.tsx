import React, { useMemo, useState } from 'react'
import type { Screen, SensorConfig } from '../types'
import type { FaceplateDef } from '../scada2/hmi/faceplateMap'
import type { WidgetInstance } from '../scada2/widgets/types'
import { WIDGET_MANIFESTS } from '../scada2/widgets/registry'
import { SYMBOLS } from '../scada2/hmi/symbols'
import { filterSymbols, paletteGroups } from '../scada2/hmi/symbolPalette'
import { SymbolThumb } from '../scada2/hmi/SymbolView'
import { widgetTitle } from '../scada2/ScadaInspector'
import { DND_SYMBOL, DND_TAG, DND_WIDGET } from '../scada2/ScadaCanvas'
import { useTrackedCurrentValues } from '../store/dashboardStore'
import { valueStyle } from '../valueStyle'
import { groupOf, isLocked, type Doc } from './model'
import { refsById, tagRef } from './bindings'
import { escapeRef } from './expr'
import { D, Icon, input, mono } from './ui'

// ── Project browser ──

const treeRow = (depth: number, active: boolean): React.CSSProperties => ({
  display: 'flex', alignItems: 'center', gap: 5, height: 22, padding: `0 6px 0 ${6 + depth * 14}px`, cursor: 'pointer', fontSize: D.font,
  background: active ? D.selBg : undefined, color: active ? 'var(--fg)' : 'var(--text-dim)', whiteSpace: 'nowrap', userSelect: 'none',
})

function Node({ label, icon, depth, active, onClick, children, count, right, defaultOpen = true, testId }: {
  label: React.ReactNode; icon: string; depth: number; active?: boolean; onClick?: () => void; children?: React.ReactNode
  count?: number; right?: React.ReactNode; defaultOpen?: boolean; testId?: string
}) {
  const [open, setOpen] = useState(defaultOpen)
  const hasKids = !!children
  return (
    <>
      <div data-testid={testId} style={treeRow(depth, !!active)} onClick={() => { if (onClick) onClick(); else if (hasKids) setOpen(o => !o) }}
        onMouseEnter={e => { if (!active) e.currentTarget.style.background = 'var(--hover-overlay)' }}
        onMouseLeave={e => { if (!active) e.currentTarget.style.background = '' }}>
        <span onClick={e => { if (hasKids) { e.stopPropagation(); setOpen(o => !o) } }}
          style={{ width: 10, display: 'inline-flex', visibility: hasKids ? 'visible' : 'hidden', transform: open ? 'rotate(90deg)' : undefined }}>
          <Icon name="chevron" size={10} />
        </span>
        <Icon name={icon} size={13} />
        <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>{label}</span>
        {count != null && <span style={{ ...mono, color: 'var(--text-muted)' }}>{count}</span>}
        {right}
      </div>
      {hasKids && open && children}
    </>
  )
}

export function ProjectBrowser({ screens, active, onOpenScreen, doc, selection, onSelect, onToggleLock, onToggleVisible, faceplates, onOpenPage, equipment, editors }: {
  screens: Screen[]; active: string; onOpenScreen: (id: string) => void
  doc: Doc; selection: string[]; onSelect: (ids: string[]) => void
  onToggleLock: (id: string) => void; onToggleVisible: (id: string) => void
  faceplates: FaceplateDef[]; onOpenPage: (page: 'scripts' | 'alarms' | 'tags') => void
  /** The Equipment folder (types and instances), drawn after the screens. */
  equipment?: React.ReactNode
  /** Who edits which screen (live collaboration): drawn after the screen's name. */
  editors?: (screenId: string) => React.ReactNode
}) {
  const sel = new Set(selection)
  // Components of the open screen, front-most first (like a layers list), groups folded together.
  const items = useMemo(() => {
    const out: ({ kind: 'w'; w: WidgetInstance } | { kind: 'g'; id: string; members: WidgetInstance[] })[] = []
    const seen = new Set<string>()
    for (const w of [...doc.widgets].reverse()) {
      const g = groupOf(w)
      if (!g) { out.push({ kind: 'w', w }); continue }
      if (seen.has(g)) continue
      seen.add(g)
      out.push({ kind: 'g', id: g, members: [...doc.widgets].reverse().filter(x => groupOf(x) === g) })
    }
    return out
  }, [doc.widgets])

  const widgetRow = (w: WidgetInstance, depth: number) => (
    <Node key={w.id} testId="designer-layer" depth={depth} icon={w.type === 'symbol' ? 'faceplate' : 'layers'} active={sel.has(w.id)}
      label={<>{w.config?.label || w.name || widgetTitle(w)} <span style={{ color: 'var(--text-muted)' }}>{w.config?.label || w.name ? widgetTitle(w) : ''}</span></>}
      onClick={() => onSelect([w.id])}
      right={<span style={{ display: 'flex', gap: 2 }}>
        <IconToggle on={w.config?.visible !== false} icon={w.config?.visible === false ? 'eyeOff' : 'eye'} title="Visible at runtime" onClick={() => onToggleVisible(w.id)} />
        <IconToggle on={isLocked(w)} icon={isLocked(w) ? 'lock' : 'unlock'} title="Locked" onClick={() => onToggleLock(w.id)} />
      </span>} />
  )

  return (
    <div style={{ padding: '2px 0' }}>
      <Node label="Screens" icon="folder" depth={0} count={screens.length}>
        {screens.map(s => (
          <Node key={s.id} testId={`designer-screen-${s.id}`} label={<>{s.name}{editors?.(s.id)}</>} icon="screen" depth={1} active={s.id === active && selection.length === 0}
            onClick={() => (s.id === active ? onSelect([]) : onOpenScreen(s.id))} defaultOpen={s.id === active}
            count={s.id === active ? doc.widgets.length : undefined}>
            {s.id === active ? items.map(it => it.kind === 'w' ? widgetRow(it.w, 2) : (
              <Node key={it.id} label={`Group (${it.members.length})`} icon="group" depth={2} active={it.members.every(m => sel.has(m.id))}
                onClick={() => onSelect(it.members.map(m => m.id))} defaultOpen={false}>
                {it.members.map(m => widgetRow(m, 3))}
              </Node>
            )) : undefined}
          </Node>
        ))}
      </Node>
      {equipment}
      <Node label="Faceplates" icon="folder" depth={0} count={faceplates.length} defaultOpen={false}>
        {faceplates.length === 0
          ? <div style={{ ...treeRow(1, false), cursor: 'default', color: 'var(--text-muted)' }}>None yet: add one from a symbol's properties</div>
          : faceplates.map(f => <Node key={f.id} label={<>{f.name} <span style={{ color: 'var(--text-muted)' }}>{f.kind} · {f.roles.length} roles</span></>} icon="faceplate" depth={1} />)}
      </Node>
      <Node label="Scripts" icon="script" depth={0} onClick={() => onOpenPage('scripts')} />
      <Node label="Alarms" icon="bell" depth={0} onClick={() => onOpenPage('alarms')} />
      <Node label="Tags" icon="tag" depth={0} onClick={() => onOpenPage('tags')} />
    </div>
  )
}

function IconToggle({ on, icon, title, onClick }: { on: boolean; icon: string; title: string; onClick: () => void }) {
  return (
    <button type="button" title={title} aria-label={title} aria-pressed={on} onClick={e => { e.stopPropagation(); onClick() }} style={{
      width: 18, height: 18, display: 'inline-grid', placeItems: 'center', padding: 0, border: 'none', background: 'none', borderRadius: 2,
      color: on ? 'var(--fg)' : 'var(--text-muted)', opacity: on ? 0.9 : 0.45, cursor: 'pointer',
    }}><Icon name={icon} size={12} /></button>
  )
}

// ── Tag browser ──

export function TagBrowserPane({ sensors, onPick }: { sensors: SensorConfig[]; onPick?: (id: number) => void }) {
  const [q, setQ] = useState('')
  const values = useTrackedCurrentValues()
  // Refs as the project format numbers them (`device/key#2` for a second tag with the same key).
  const refs = useMemo(() => refsById(sensors), [sensors])
  const groups = useMemo(() => {
    const ql = q.trim().toLowerCase()
    const m = new Map<string, SensorConfig[]>()
    for (const s of sensors) {
      if (ql && !`${s.name} ${s.key} ${refs.get(s.id) ?? tagRef(s)}`.toLowerCase().includes(ql)) continue
      const dev = s.device_id || '(no device)'
      if (!m.has(dev)) m.set(dev, [])
      m.get(dev)!.push(s)
    }
    return [...m.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([dev, list]) => [dev, list.sort((a, b) => a.key.localeCompare(b.key))] as const)
  }, [sensors, q, refs])

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div style={{ padding: 4, borderBottom: `1px solid ${D.line}` }}>
        <input value={q} onChange={e => setQ(e.target.value)} placeholder="Filter tags" aria-label="Filter tags" style={input} />
      </div>
      <div style={{ flex: 1, overflow: 'auto' }} data-testid="designer-tags">
        {groups.map(([dev, list]) => (
          // Filtering remounts the folders open, so a match is visible even in a folder closed before.
          <Node key={q ? `${dev}\u0000filtered` : dev} label={dev} icon="folder" depth={0} count={list.length} defaultOpen={groups.length < 4 || !!q}>
            {list.map(s => {
              const ref = refs.get(s.id) ?? tagRef(s)
              const d = values[String(s.id)]
              const vs = valueStyle(d)
              const v = d?.value
              return (
                <div key={s.id} draggable data-testid="designer-tag" data-ref={ref}
                  onDragStart={e => { e.dataTransfer.setData(DND_TAG, String(s.id)); e.dataTransfer.setData('text/plain', escapeRef(ref)); e.dataTransfer.effectAllowed = 'copy' }}
                  onDoubleClick={() => onPick?.(s.id)}
                  title={`${s.name}\n${escapeRef(ref)}\nDrag onto the screen to place a bound component, or onto a component to bind it.`}
                  style={{ ...treeRow(1, false), cursor: 'grab', gap: 6 }}
                  onMouseEnter={e => { e.currentTarget.style.background = 'var(--hover-overlay)' }}
                  onMouseLeave={e => { e.currentTarget.style.background = '' }}>
                  <span style={{ width: 10 }} />
                  <Icon name="tag" size={12} />
                  <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>{s.key}</span>
                  <span style={{ ...mono, color: vs.state === 'normal' ? 'var(--fg)' : vs.color, maxWidth: 90, overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {v == null ? '—' : Number.isInteger(v) ? v : v.toFixed(Math.min(3, s.decimals ?? 2))}{s.unit ? ` ${s.unit}` : ''}
                  </span>
                </div>
              )
            })}
          </Node>
        ))}
        {groups.length === 0 && <div style={{ padding: 10, fontSize: D.small, color: 'var(--text-muted)' }}>No tags match.</div>}
      </div>
    </div>
  )
}

// ── Component palette ──

const WIDGET_GROUPS: { title: string; types: string[] }[] = [
  { title: 'Display', types: ['big_number', 'indicator', 'label', 'led_display', 'status_light', 'on_off', 'annunciator', 'data_table'] },
  { title: 'Gauges & charts', types: ['gauge', 'semicircle_gauge', 'meter', 'bar', 'progress_bar', 'donut', 'trend', 'multi_trend', 'bar_chart', 'pie_chart'] },
  { title: 'Controls', types: ['push_button', 'toggle', 'slider', 'number_input', 'pid_faceplate'] },
  { title: 'Layout', types: ['area'] },
]

export function PalettePane({ onAdd, top }: {
  onAdd: (item: { kind: 'symbol' | 'widget'; key: string }) => void
  /** Drawn above the symbols (equipment types); a function gets the trimmed search text. */
  top?: React.ReactNode | ((query: string) => React.ReactNode)
}) {
  const [q, setQ] = useState('')
  const groups = useMemo(() => paletteGroups(SYMBOLS), [])
  const [group, setGroup] = useState<string>('isa')
  const ql = q.trim().toLowerCase()
  const tile: React.CSSProperties = {
    display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 2, padding: '5px 2px 4px', borderRadius: 3, cursor: 'grab',
    border: `1px solid ${D.line}`, background: 'var(--hmi-stage)', color: 'var(--text-dim)', fontSize: 9.5, lineHeight: 1.15, textAlign: 'center', minWidth: 0,
  }
  const symbols = filterSymbols(SYMBOLS, ql, group)
  // All of ISA-101 (or a search) is shown in category sections; one category is one grid.
  const sections = useMemo(() => {
    if (!ql && group !== 'isa') return [{ title: groups.find(g => g.id === group)?.label ?? group, items: symbols }]
    const out: { title: string; items: typeof symbols }[] = []
    for (const s of symbols) {
      const title = `${s.library === 'isa' ? 'ISA-101' : 'Shaded'} · ${s.category}`
      const sec = out.find(x => x.title === title)
      if (sec) sec.items.push(s)
      else out.push({ title, items: [s] })
    }
    return out
  }, [symbols, ql, group, groups])
  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }} data-testid="designer-palette">
      <div style={{ display: 'flex', gap: 4, padding: 4, borderBottom: `1px solid ${D.line}` }}>
        <input value={q} onChange={e => setQ(e.target.value)} placeholder="Search components" aria-label="Search components" style={input} />
        <select value={group} onChange={e => { setGroup(e.target.value); setQ('') }} aria-label="Symbol category" style={{ ...input, width: 118, flexShrink: 0 }}>
          {groups.map(g => <option key={g.id} value={g.id}>{g.label}</option>)}
        </select>
      </div>
      <div style={{ flex: 1, overflow: 'auto', padding: 6 }}>
        {typeof top === 'function' ? top(q.trim()) : top}
        {sections.map(sec => (
          <div key={sec.title}>
            <PaletteTitle>{sec.title}</PaletteTitle>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 4, marginBottom: 8 }}>
              {sec.items.map(s => (
                <div key={s.key} draggable title={`${s.name}\n${s.key}`} style={tile} data-testid={`designer-symbol-${s.key}`}
                  onDragStart={e => { e.dataTransfer.setData(DND_SYMBOL, s.key); e.dataTransfer.effectAllowed = 'copy' }}
                  onDoubleClick={() => onAdd({ kind: 'symbol', key: s.key })}>
                  <SymbolThumb def={s} size={28} />
                  <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: '100%', whiteSpace: 'nowrap' }}>{s.name.replace(' (ISA-101)', '')}</span>
                </div>
              ))}
            </div>
          </div>
        ))}
        {ql && symbols.length === 0 && <div style={{ padding: '2px 0 8px', fontSize: D.small, color: 'var(--text-muted)' }}>No symbols match.</div>}
        {WIDGET_GROUPS.map(g => {
          const types = g.types.filter(t => WIDGET_MANIFESTS[t as keyof typeof WIDGET_MANIFESTS] && (!ql || `${WIDGET_MANIFESTS[t as keyof typeof WIDGET_MANIFESTS].displayName} ${t}`.toLowerCase().includes(ql)))
          if (types.length === 0) return null
          return (
            <div key={g.title}>
              <PaletteTitle>{g.title}</PaletteTitle>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 3, marginBottom: 8 }}>
                {types.map(t => {
                  const m = WIDGET_MANIFESTS[t as keyof typeof WIDGET_MANIFESTS]
                  return (
                    <div key={t} draggable title={m.displayName} data-testid={`designer-widget-${t}`}
                      onDragStart={e => { e.dataTransfer.setData(DND_WIDGET, t); e.dataTransfer.effectAllowed = 'copy' }}
                      onDoubleClick={() => onAdd({ kind: 'widget', key: t })}
                      style={{ display: 'flex', alignItems: 'center', gap: 6, height: 24, padding: '0 6px', border: `1px solid ${D.line}`, borderRadius: 3, cursor: 'grab', fontSize: D.small, minWidth: 0 }}>
                      <span style={{ width: 14, textAlign: 'center', color: 'var(--text-muted)' }}>{m.icon}</span>
                      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{m.displayName}</span>
                    </div>
                  )
                })}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}

const PaletteTitle = ({ children }: { children: React.ReactNode }) => (
  <div style={{ fontSize: 10, fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', color: 'var(--text-muted)', margin: '2px 0 4px' }}>{children}</div>
)
