import React from 'react'
import type { SensorConfig } from '../types'
import type { ConfigFieldDef, ConnectionInstance, WidgetInstance } from './widgets/types'
import { WIDGET_MANIFESTS } from './widgets/registry'
import { symbolSchema } from './widgets/SymbolWidget'
import { SYMBOL_MAP } from './hmi/symbols'
import { resolveSymbol } from './hmi/resolve'
import { CONNECTION_TYPES } from './connections/ConnectionLayer'

export const INSPECTOR_WIDTH = 288

interface WidgetProps {
  widget: WidgetInstance
  sensors: SensorConfig[]
  onUpdate: (updates: Partial<WidgetInstance>, coalesce?: string) => void
  onUpdateConfig: (config: Record<string, any>, coalesce?: string) => void
  onDuplicate: () => void
  onReorder: (to: 'front' | 'back') => void
  onDelete: () => void
}

interface ConnectionProps {
  connection: ConnectionInstance
  widgets: WidgetInstance[]
  onUpdate: (updates: Partial<ConnectionInstance>) => void
  onDelete: () => void
}

const panel: React.CSSProperties = {
  position: 'absolute', top: 0, right: 0, bottom: 0, width: INSPECTOR_WIDTH, zIndex: 20,
  background: 'var(--card)', borderLeft: '1px solid var(--border)', display: 'flex', flexDirection: 'column',
}

const input: React.CSSProperties = {
  width: '100%', padding: '5px 7px', borderRadius: 4, fontSize: 12, minWidth: 0,
  border: '1px solid var(--border)', background: 'var(--input-bg)', color: 'var(--fg)',
}

export function widgetTitle(w: WidgetInstance): string {
  if (w.type === 'symbol') return SYMBOL_MAP[w.config?.symbol]?.name ?? 'Symbol'
  return WIDGET_MANIFESTS[w.type]?.displayName ?? w.type
}

export const WidgetInspector: React.FC<WidgetProps> = ({ widget, sensors, onUpdate, onUpdateConfig, onDuplicate, onReorder, onDelete }) => {
  const manifest = WIDGET_MANIFESTS[widget.type]
  const resolved = resolveSymbol(widget)
  const def = resolved?.def
  let schema: ConfigFieldDef[] = manifest?.configSchema ?? []
  if (resolved) {
    schema = symbolSchema(resolved.def, { legacy: widget.type !== 'symbol', defaultWritable: resolved.defaultWritable })
    // Legacy pipe orientation lives in its own key.
    if (widget.type === 'pipe') schema = [...schema, ...(manifest?.configSchema ?? []).filter(f => f.key === 'direction')]
  }
  const multi = (manifest?.maxBindings ?? 1) > 1
  // Show what is rendered: legacy widgets map old keys (showValue → showStatus, showPercentage → showPercent).
  const config = resolved?.config ?? widget.config ?? {}

  const setPrimary = (id: number | null) => onUpdate({ sensorId: id, sensorIds: id != null ? [id] : [] })

  return (
    <div style={panel}>
      <Header title={widgetTitle(widget)} subtitle={widget.id} />
      <div style={{ flex: 1, overflowY: 'auto' }}>
        <Section title="Tag binding">
          {multi ? (
            <MultiTagPicker sensors={sensors} value={widget.sensorIds ?? []}
              onChange={ids => onUpdate({ sensorIds: ids, sensorId: ids[0] ?? null })} />
          ) : (
            <TagSelect sensors={sensors} value={widget.sensorId} onChange={setPrimary}
              placeholder={def && def.binding === 'none' ? '— Static (no tag) —' : '— Not bound —'} />
          )}
          {def && <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 6 }}>{bindingHint(def.binding)}</div>}
        </Section>

        <Section title="Position & size">
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
            {(['x', 'y', 'width', 'height'] as const).map(k => (
              <label key={k} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <span style={{ fontSize: 10, color: 'var(--muted)', width: 12, textTransform: 'uppercase' }}>{k[0]}</span>
                <NumberInput value={widget[k]} step={10}
                  onCommit={v => onUpdate({ [k]: Math.max(k === 'x' || k === 'y' ? 0 : 10, Math.round(v)) })} />
              </label>
            ))}
          </div>
        </Section>

        {schema.length > 0 && (
          <Section title="Appearance & behaviour">
            {schema.map(f => (
              <Field key={f.key} field={f} value={config[f.key] ?? f.default} sensors={sensors}
                onChange={v => onUpdateConfig({ [f.key]: v }, `${widget.id}:${f.key}`)} />
            ))}
          </Section>
        )}

        <Section title="Arrange">
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
            <Btn onClick={onDuplicate} kbd="Ctrl+D">Duplicate</Btn>
            <Btn onClick={() => onReorder('front')} kbd="]">To front</Btn>
            <Btn onClick={() => onReorder('back')} kbd="[">To back</Btn>
            <Btn onClick={onDelete} kbd="Del" danger>Delete</Btn>
          </div>
        </Section>
      </div>
    </div>
  )
}

const SHORTCUTS: [string, string][] = [
  ['V / C', 'Select / connect tool'],
  ['Ctrl+Z · Ctrl+Shift+Z', 'Undo / redo'],
  ['Ctrl+C · Ctrl+V · Ctrl+D', 'Copy / paste / duplicate'],
  ['Arrows · Shift+Arrows', 'Nudge 1 / 10 px'],
  ['[ · ]', 'Send to back / front'],
  ['Del', 'Delete selection'],
  ['Ctrl+ + · Ctrl+ - · Ctrl+0', 'Zoom in / out / fit'],
  ['Ctrl+S', 'Save'],
]

/** Shown when nothing is selected, so the inspector column (and the stage scale) stays put while editing. */
export const ScreenInspector: React.FC<{ widgets: number; connections: number; tags: number }> = ({ widgets, connections, tags }) => (
  <div style={panel}>
    <Header title="Screen" subtitle={`${widgets} items · ${connections} connections · ${tags} tags bound`} />
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <Section title="Build">
        <div style={{ fontSize: 12, color: 'var(--text-dim)', lineHeight: 1.55 }}>
          Drag equipment from <b>Symbols</b>, or drop a tag from <b>Tags</b> to pick how to show it.
          Select an item to bind tags and set its behaviour; use the connect tool to pipe nozzles together.
        </div>
      </Section>
      <Section title="Shortcuts">
        {SHORTCUTS.map(([k, d]) => (
          <div key={k} style={{ display: 'flex', justifyContent: 'space-between', gap: 8, fontSize: 11, padding: '3px 0' }}>
            <span style={{ color: 'var(--text-dim)' }}>{d}</span>
            <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>{k}</span>
          </div>
        ))}
      </Section>
    </div>
  </div>
)

export const ConnectionInspector: React.FC<ConnectionProps> = ({ connection, widgets, onUpdate, onDelete }) => {
  const name = (id: string) => {
    const w = widgets.find(x => x.id === id)
    return w ? (w.config?.label || widgetTitle(w)) : id
  }
  return (
    <div style={panel}>
      <Header title="Connection" subtitle={`${name(connection.fromWidgetId)} → ${name(connection.toWidgetId)}`} />
      <div style={{ flex: 1, overflowY: 'auto' }}>
        <Section title="Type">
          <select value={connection.connectionType} style={input}
            onChange={e => onUpdate({ connectionType: e.target.value as ConnectionInstance['connectionType'] })}>
            {CONNECTION_TYPES.map(t => <option key={t.value} value={t.value}>{t.label}</option>)}
          </select>
          <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 6, lineHeight: 1.4 }}>
            Process pipes animate flow while the source equipment is running.
          </div>
        </Section>
        <Section title="Arrange">
          <Btn onClick={onDelete} kbd="Del" danger>Delete connection</Btn>
        </Section>
      </div>
    </div>
  )
}

function bindingHint(b: string): string {
  switch (b) {
    case 'state': return 'Running when the tag is above the threshold.'
    case 'level': return 'Fill level follows the tag between its min and max.'
    case 'value': return 'Shows the live tag value.'
    default: return 'Optional. A bound tag above zero animates flow.'
  }
}

function Header({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div style={{ padding: '10px 12px', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
      <div style={{ fontSize: 13, fontWeight: 600 }}>{title}</div>
      <div style={{ fontSize: 10, color: 'var(--muted)', marginTop: 2, fontFamily: 'var(--font-mono)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {subtitle}
      </div>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ padding: '10px 12px 12px', borderBottom: '1px solid var(--border)' }}>
      <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.06em', marginBottom: 8 }}>
        {title}
      </div>
      {children}
    </div>
  )
}

function Btn({ children, onClick, kbd, danger }: { children: React.ReactNode; onClick: () => void; kbd?: string; danger?: boolean }) {
  return (
    <button onClick={onClick} style={{
      display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 6, width: '100%',
      padding: '6px 8px', borderRadius: 4, fontSize: 12, cursor: 'pointer', background: 'transparent',
      border: `1px solid ${danger ? 'color-mix(in srgb, var(--crit) 45%, transparent)' : 'var(--border)'}`,
      color: danger ? 'var(--crit)' : 'var(--fg)',
    }}>
      <span>{children}</span>
      {kbd && <span style={{ fontSize: 10, color: 'var(--muted)', fontFamily: 'var(--font-mono)' }}>{kbd}</span>}
    </button>
  )
}

function TagSelect({ sensors, value, onChange, placeholder = '— Not bound —' }: {
  sensors: SensorConfig[]; value: number | null | undefined; onChange: (id: number | null) => void; placeholder?: string
}) {
  const sorted = [...sensors].sort((a, b) => a.name.localeCompare(b.name))
  return (
    <select value={value ?? ''} style={input} onChange={e => onChange(e.target.value === '' ? null : Number(e.target.value))}>
      <option value="">{placeholder}</option>
      {sorted.map(s => <option key={s.id} value={s.id}>{s.name}{s.unit ? ` (${s.unit})` : ''}</option>)}
    </select>
  )
}

function MultiTagPicker({ sensors, value, onChange }: { sensors: SensorConfig[]; value: number[]; onChange: (ids: number[]) => void }) {
  const byId = new Map(sensors.map(s => [s.id, s]))
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      {value.map(id => (
        <div key={id} style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 12 }}>
          <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{byId.get(id)?.name ?? `Tag #${id}`}</span>
          <button onClick={() => onChange(value.filter(v => v !== id))} aria-label="Remove tag"
            style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 14 }}>×</button>
        </div>
      ))}
      <TagSelect sensors={sensors.filter(s => !value.includes(s.id))} value={null}
        placeholder="+ Add tag" onChange={id => { if (id != null) onChange([...value, id]) }} />
    </div>
  )
}

/** Number field that edits a local draft and commits on blur / Enter, so clearing it to retype doesn't write 0. */
function NumberInput({ value, onCommit, step, min, max }: {
  value: number; onCommit: (v: number) => void; step?: number | 'any'; min?: number; max?: number
}) {
  const [draft, setDraft] = React.useState<string | null>(null)
  const commit = () => {
    if (draft === null) return
    const v = Number(draft)
    setDraft(null)
    if (draft.trim() !== '' && Number.isFinite(v) && v !== value) {
      onCommit(Math.min(max ?? Infinity, Math.max(min ?? -Infinity, v)))
    }
  }
  return (
    <input type="number" step={step} min={min} max={max} style={input}
      value={draft ?? String(value)}
      onChange={e => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={e => { if (e.key === 'Enter') commit(); if (e.key === 'Escape') setDraft(null) }} />
  )
}

function Field({ field, value, sensors, onChange }: { field: ConfigFieldDef; value: any; sensors: SensorConfig[]; onChange: (v: any) => void }) {
  if (field.type === 'boolean') {
    return (
      <label style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, marginBottom: 8, cursor: 'pointer' }}>
        <span style={{ fontSize: 12 }}>{field.label}</span>
        <input type="checkbox" checked={!!value} onChange={e => onChange(e.target.checked)} />
      </label>
    )
  }
  let control: React.ReactNode
  if (field.type === 'tag') {
    control = <TagSelect sensors={sensors} value={value} onChange={onChange} placeholder="— None —" />
  } else if (field.type === 'select') {
    control = (
      <select value={String(value ?? '')} style={input} onChange={e => onChange(e.target.value)}>
        {field.options?.map(o => <option key={o.value} value={o.value}>{o.label}</option>)}
      </select>
    )
  } else if (field.type === 'color') {
    const str = String(value ?? '')
    control = (
      <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
        <input type="color" value={str.startsWith('#') ? str : '#22c55e'} onChange={e => onChange(e.target.value)}
          style={{ width: 28, height: 26, border: 'none', background: 'none', cursor: 'pointer', padding: 0 }} />
        <input value={str} onChange={e => onChange(e.target.value)} style={input} />
      </div>
    )
  } else if (field.type === 'number') {
    control = <NumberInput value={Number(value ?? 0)} step="any" min={field.min} max={field.max} onCommit={onChange} />
  } else {
    control = <input type="text" value={value ?? ''} style={input} onChange={e => onChange(e.target.value)} />
  }
  return (
    <label style={{ display: 'block', marginBottom: 8 }}>
      <span style={{ display: 'block', fontSize: 11, color: 'var(--text-dim)', marginBottom: 3 }}>{field.label}</span>
      {control}
    </label>
  )
}
