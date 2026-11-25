import React, { useMemo, useState } from 'react'
import type { SensorConfig } from '../types'
import type { ConfigFieldDef, WidgetInstance } from '../scada2/widgets/types'
import { WIDGET_MANIFESTS } from '../scada2/widgets/registry'
import { resolveSymbol } from '../scada2/hmi/resolve'
import { FaceplateSection, MultiTagPicker, NumberInput, widgetTitle } from '../scada2/ScadaInspector'
import { getScreenWidget, isAddonWidgetType } from '../edition/widgets'
import { DND_TAG } from '../scada2/ScadaCanvas'
import { useTrackedCurrentValues } from '../store/dashboardStore'
import {
  bindingErrors, bindingsOf, describeBinding, evaluateBinding, liveReader, refsById, sharedRefIndex, withBinding,
  type Binding, type MapCase, type PropType,
} from './bindings'
import { escapeRef, FUNCTIONS } from './expr'
import { propertiesOf } from './props'
import { bindPrimaryPatch, boundsOf, groupOf, isLocked, type Doc, type Patch } from './model'
import { D, GridSection, Icon, PropRow, SmallBtn, input, mono } from './ui'

interface Props {
  doc: Doc
  selection: string[]
  sensors: SensorConfig[]
  screenName: string
  onPatch: (patches: Record<string, Patch>, coalesce?: string) => void
  arrange: React.ReactNode
  /** The equipment instance section of a symbol bound to one (Designer.tsx). */
  equipment?: (w: WidgetInstance) => React.ReactNode
}

/** Property editor: the selected component's properties, each fixed or bound to a tag, an expression or a map. */
export function PropertyEditor({ doc, selection, sensors, screenName, onPatch, arrange, equipment }: Props) {
  const ws = doc.widgets.filter(w => selection.includes(w.id))
  if (ws.length === 0) return <ScreenProps doc={doc} sensors={sensors} screenName={screenName} />
  if (ws.length > 1) return <MultiProps ws={ws} onPatch={onPatch} arrange={arrange} />
  return <WidgetProps key={ws[0].id} w={ws[0]} sensors={sensors} onPatch={onPatch} equipment={equipment?.(ws[0])} />
}

function ScreenProps({ doc, sensors, screenName }: { doc: Doc; sensors: SensorConfig[]; screenName: string }) {
  const tags = new Set(doc.widgets.flatMap(w => w.sensorIds ?? []))
  const bound = doc.widgets.filter(w => Object.keys(bindingsOf(w.config)).length > 0).length
  return (
    <div data-testid="designer-props-screen">
      <GridSection title="Screen">
        <PropRow label="Name"><span style={{ fontSize: D.font }}>{screenName}</span></PropRow>
        <PropRow label="Size"><span style={mono}>1920 × 1080</span></PropRow>
        <PropRow label="Components"><span style={mono}>{doc.widgets.length}</span></PropRow>
        <PropRow label="Connections"><span style={mono}>{doc.connections.length}</span></PropRow>
        <PropRow label="Tags shown"><span style={mono}>{tags.size} of {sensors.length}</span></PropRow>
        <PropRow label="With bindings"><span style={mono}>{bound}</span></PropRow>
      </GridSection>
      <GridSection title="Keyboard">
        {SHORTCUTS.map(([k, v]) => <PropRow key={k} label={v}><span style={{ ...mono, color: 'var(--text-muted)' }}>{k}</span></PropRow>)}
      </GridSection>
    </div>
  )
}

const SHORTCUTS: [string, string][] = [
  ['Ctrl+S', 'Save'], ['Ctrl+Z / Ctrl+Y', 'Undo / redo'], ['Ctrl+C / X / V', 'Copy, cut, paste'], ['Ctrl+D', 'Duplicate'],
  ['Ctrl+A', 'Select all'], ['Shift+click, drag', 'Add to selection, marquee'], ['Ctrl+G / Ctrl+Shift+G', 'Group / ungroup'],
  ['Ctrl+L', 'Lock / unlock'], ['Ctrl+] / Ctrl+[', 'Forward / backward'], ['Ctrl+Shift+] / [', 'To front / back'],
  ['Arrows, Shift+arrows', 'Nudge 1 / 10 px'], ['Alt while dragging', 'No snapping'], ['Space+drag, wheel', 'Pan'],
  ['Ctrl+wheel, Ctrl+0', 'Zoom, fit'], ['F5', 'Preview with live data'], ['Del', 'Delete'], ['Esc', 'Clear selection'],
]

function MultiProps({ ws, onPatch, arrange }: { ws: WidgetInstance[]; onPatch: Props['onPatch']; arrange: React.ReactNode }) {
  const b = boundsOf(ws)!
  const set = (k: 'x' | 'y', v: number) => onPatch(Object.fromEntries(ws.filter(w => !isLocked(w)).map(w => [w.id, { [k]: w[k] + (v - b[k]) }])))
  const groups = new Set(ws.map(groupOf))
  return (
    <div data-testid="designer-props-multi">
      <GridSection title={`${ws.length} components${groups.size === 1 && groupOf(ws[0]) ? ' · group' : ''}`}>
        <PropRow label="X"><NumberInput value={b.x} onCommit={v => set('x', Math.round(v))} /></PropRow>
        <PropRow label="Y"><NumberInput value={b.y} onCommit={v => set('y', Math.round(v))} /></PropRow>
        <PropRow label="Bounds"><span style={mono}>{b.width} × {b.height}</span></PropRow>
        <PropRow label="Locked"><span style={mono}>{ws.filter(isLocked).length}</span></PropRow>
      </GridSection>
      <GridSection title="Arrange">{arrange}</GridSection>
    </div>
  )
}

function WidgetProps({ w, sensors, onPatch, equipment }: { w: WidgetInstance; sensors: SensorConfig[]; onPatch: Props['onPatch']; equipment?: React.ReactNode }) {
  const values = useTrackedCurrentValues()
  const index = sharedRefIndex(sensors)
  const read = useMemo(() => liveReader(index, values), [index, values])
  const [editing, setEditing] = useState<string | null>(null)
  const manifest = WIDGET_MANIFESTS[w.type]
  const resolved = resolveSymbol(w)
  const addon = !manifest && isAddonWidgetType(w.type)
  const AddonEditor = addon ? getScreenWidget(w.type, 'scada')?.configEditor : undefined
  const props = propertiesOf(w)
  const bs = bindingsOf(w.config)
  const config = resolved?.config ?? w.config ?? {}
  const multi = (manifest?.maxBindings ?? 1) > 1
  const takesTag = !addon && (manifest?.maxBindings ?? 1) > 0
  const primary = w.sensorId != null ? sensors.find(s => s.id === w.sensorId) : undefined

  const setConfig = (c: Record<string, any>, key?: string) => onPatch({ [w.id]: { config: c } }, key)
  const setBinding = (prop: string, b: Binding | null) => {
    // Replace the whole bindings object (a patch merges per top-level key).
    // Each apply or removal is its own undo step (no coalescing key).
    const next = withBinding(w.config ?? {}, prop, b)
    setConfig({ bindings: next.bindings })
  }

  return (
    <div data-testid="designer-props-widget">
      <div style={{ padding: '6px 10px', borderBottom: `1px solid ${D.line}`, display: 'flex', alignItems: 'baseline', gap: 6 }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>{w.config?.label || w.name || widgetTitle(w)}</span>
        <span style={{ ...mono, color: 'var(--text-muted)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{widgetTitle(w)} · {w.id}</span>
      </div>

      {equipment}

      {takesTag && (
        <GridSection title="Value">
          {multi ? (
            <div style={{ padding: '4px 10px' }}>
              <MultiTagPicker sensors={sensors} value={w.sensorIds ?? []} onChange={ids => onPatch({ [w.id]: { sensorIds: ids, sensorId: ids[0] ?? null } })} />
            </div>
          ) : (
            <PropRow label="Tag" bound={w.sensorId != null} testId="designer-prop-tag"
              title="The tag this component shows. Drop a tag from the tag browser here or onto the component.">
              <TagPicker sensors={sensors} value={w.sensorId} onChange={id => onPatch({ [w.id]: bindPrimaryPatch(id) })}
                placeholder={resolved?.def.binding === 'none' ? 'Static (no tag)' : 'Not bound'} />
            </PropRow>
          )}
          {primary && (
            <PropRow label="Live">
              <span style={{ ...mono, color: D.bind }}>{values[String(primary.id)]?.value?.toFixed(primary.decimals ?? 2) ?? '—'} {primary.unit}</span>
            </PropRow>
          )}
        </GridSection>
      )}

      <GridSection title="Layout">
        {(['x', 'y', 'width', 'height'] as const).map(k => (
          <PropRow key={k} label={k === 'x' ? 'X' : k === 'y' ? 'Y' : k[0].toUpperCase() + k.slice(1)}>
            <NumberInput value={w[k]} step={1} onCommit={v => onPatch({ [w.id]: { [k]: Math.max(k === 'x' || k === 'y' ? 0 : 10, Math.round(v)) } })} />
          </PropRow>
        ))}
        <PropRow label="Locked">
          <input type="checkbox" checked={isLocked(w)} onChange={e => setConfig({ locked: e.target.checked ? true : undefined })} />
        </PropRow>
      </GridSection>

      {AddonEditor && (
        <GridSection title="Settings">
          <div style={{ padding: 8 }}>
            <AddonEditor config={w.config ?? {}} onChange={c => onPatch({ [w.id]: { config: c } }, `${w.id}:config`)} surface="scada" sensors={sensors} />
          </div>
        </GridSection>
      )}

      <GridSection title="Properties">
        {props.map(f => {
          const b = bs[f.key]
          const open = editing === f.key
          // Tag-picker fields hold a tag id: pick the tag itself, there is nothing to bind.
          const bindable = f.type !== 'tag'
          const live = b ? evaluateBinding(b, read, f.type as PropType) : undefined
          return (
            <React.Fragment key={f.key}>
              <PropRow label={f.label} bound={!!b} testId={`designer-prop-${f.key}`} title={b ? `Bound: ${describeBinding(b)}` : undefined}
                trailing={!bindable ? undefined :
                  <button type="button" title={b ? 'Edit binding' : 'Bind to a tag or expression'} aria-label={`Bind ${f.label}`} data-testid={`designer-bind-${f.key}`}
                    onClick={() => setEditing(open ? null : f.key)}
                    style={{
                      width: 20, height: 20, display: 'grid', placeItems: 'center', padding: 0, borderRadius: 3, cursor: 'pointer',
                      border: `1px solid ${b || open ? D.bind : 'transparent'}`, background: b ? 'color-mix(in srgb, #5aa9e6 16%, transparent)' : 'transparent',
                      color: b ? D.bind : 'var(--text-muted)',
                    }}><Icon name="link" size={12} /></button>
                }>
                {b ? (
                  <span onClick={() => setEditing(open ? null : f.key)} style={{ ...mono, color: D.bind, cursor: 'pointer', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {live === undefined ? '—' : typeof live === 'number' ? Math.round(live * 1000) / 1000 : String(live)}
                    <span style={{ color: 'var(--text-muted)' }}> ← {describeBinding(b)}</span>
                  </span>
                ) : (
                  <FixedValue field={f} value={config[f.key] ?? f.default} sensors={sensors}
                    onChange={v => setConfig({ [f.key]: v }, `${w.id}:${f.key}`)} />
                )}
              </PropRow>
              {open && (
                <BindingEditor prop={f} binding={b} sensors={sensors} index={index} read={read}
                  onChange={nb => setBinding(f.key, nb)} onClose={() => setEditing(null)} />
              )}
            </React.Fragment>
          )
        })}
      </GridSection>

      {!addon && (resolved || w.sensorId != null) && (
        <div style={{ borderTop: `1px solid ${D.line}` }}>
          <FaceplateSection widget={w} sensors={sensors}
            onUpdate={(updates, coalesce) => onPatch({ [w.id]: updates as Patch }, coalesce)}
            onUpdateConfig={(c, coalesce) => setConfig(c, coalesce)} />
        </div>
      )}
    </div>
  )
}

/** Fixed (unbound) value editor, compact for the grid. */
function FixedValue({ field, value, sensors, onChange }: { field: ConfigFieldDef; value: any; sensors: SensorConfig[]; onChange: (v: any) => void }) {
  switch (field.type) {
    case 'boolean':
      return <input type="checkbox" checked={!!value} onChange={e => onChange(e.target.checked)} aria-label={field.label} />
    case 'tag':
      return <TagPicker sensors={sensors} value={value} onChange={onChange} placeholder="None" />
    case 'select':
      return (
        <select value={String(value ?? '')} style={input} onChange={e => onChange(e.target.value)} aria-label={field.label}>
          {field.options?.map(o => <option key={o.value} value={o.value}>{o.label}</option>)}
        </select>
      )
    case 'color': {
      const str = String(value ?? '')
      return (
        <div style={{ display: 'flex', gap: 4, alignItems: 'center', width: '100%' }}>
          <input type="color" value={/^#[0-9a-f]{6}$/i.test(str) ? str : '#22c55e'} onChange={e => onChange(e.target.value)}
            style={{ width: 22, height: 20, border: 'none', background: 'none', padding: 0, cursor: 'pointer', flexShrink: 0 }} aria-label={field.label} />
          <input value={str} onChange={e => onChange(e.target.value)} style={input} />
        </div>
      )
    }
    case 'number':
      return <NumberInput value={Number(value ?? 0)} step="any" min={field.min} max={field.max} onCommit={onChange} />
    default:
      return <input type="text" value={value ?? ''} style={input} onChange={e => onChange(e.target.value)} aria-label={field.label} />
  }
}

/** Tag select grouped by device; also takes a tag dropped from the tag browser. */
function TagPicker({ sensors, value, onChange, placeholder }: { sensors: SensorConfig[]; value: number | null | undefined; onChange: (id: number | null) => void; placeholder: string }) {
  const [over, setOver] = useState(false)
  const groups = useMemo(() => {
    const m = new Map<string, SensorConfig[]>()
    for (const s of [...sensors].sort((a, b) => a.key.localeCompare(b.key))) {
      const d = s.device_id || '(no device)'
      if (!m.has(d)) m.set(d, [])
      m.get(d)!.push(s)
    }
    return [...m.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [sensors])
  return (
    <select value={value ?? ''} aria-label="Tag" style={{ ...input, outline: over ? `1px solid ${D.bind}` : undefined }}
      onChange={e => onChange(e.target.value === '' ? null : Number(e.target.value))}
      onDragOver={e => { if (e.dataTransfer.types.includes(DND_TAG)) { e.preventDefault(); setOver(true) } }}
      onDragLeave={() => setOver(false)}
      onDrop={e => { setOver(false); const id = parseInt(e.dataTransfer.getData(DND_TAG), 10); if (!isNaN(id)) { e.preventDefault(); onChange(id) } }}>
      <option value="">{placeholder}</option>
      {groups.map(([dev, list]) => (
        <optgroup key={dev} label={dev}>
          {list.map(s => <option key={s.id} value={s.id}>{s.key}{s.name && s.name !== s.key ? ` · ${s.name}` : ''}</option>)}
        </optgroup>
      ))}
    </select>
  )
}

function BindingEditor({ prop, binding, sensors, index, read, onChange, onClose }: {
  prop: ConfigFieldDef; binding: Binding | undefined; sensors: SensorConfig[]; index: Map<string, number>
  read: (ref: string) => number | null; onChange: (b: Binding | null) => void; onClose: () => void
}) {
  const [kind, setKind] = useState<Binding['kind']>(binding?.kind ?? 'tag')
  const [draft, setDraft] = useState<Binding>(binding ?? { kind: 'tag', tag: '' })
  const switchKind = (k: Binding['kind']) => {
    setKind(k)
    const seed = draft.kind === 'tag' ? (draft.tag ? `{${draft.tag}}` : '') : draft.expr
    setDraft(k === 'tag' ? { kind: 'tag', tag: draft.kind === 'tag' ? draft.tag : '' }
      : k === 'expr' ? { kind: 'expr', expr: seed }
      : { kind: 'map', expr: seed, cases: draft.kind === 'map' ? draft.cases : [{ op: '>=', v: 1, out: prop.type === 'boolean' ? true : '' }], else: draft.kind === 'map' ? draft.else : undefined })
  }
  const errors = (draft.kind === 'tag' ? !!draft.tag : !!draft.expr.trim()) ? bindingErrors(draft, index) : []
  const empty = draft.kind === 'tag' ? !draft.tag : !draft.expr.trim()
  const preview = !empty && errors.length === 0 ? evaluateBinding(draft, read, prop.type as PropType) : undefined
  const refs = useMemo(() => refsById(sensors), [sensors])
  const tagById = (id: number | null) => (id == null ? '' : refs.get(id) ?? '')
  const insertTag = (id: number | null) => {
    if (id == null || draft.kind === 'tag') return
    setDraft({ ...draft, expr: `${draft.expr}${draft.expr && !draft.expr.endsWith(' ') ? ' ' : ''}${escapeRef(tagById(id))}` })
  }
  const seg = (k: Binding['kind'], label: string) => (
    <button type="button" onClick={() => switchKind(k)} aria-pressed={kind === k} data-testid={`designer-bindkind-${k}`} style={{
      flex: 1, height: 22, fontSize: D.small, cursor: 'pointer', border: `1px solid ${kind === k ? D.bind : D.line}`,
      background: kind === k ? 'color-mix(in srgb, #5aa9e6 16%, transparent)' : 'transparent', color: kind === k ? D.bind : 'var(--fg)',
    }}>{label}</button>
  )
  return (
    <div data-testid="designer-binding-editor" style={{ margin: '0 4px 6px 10px', padding: 8, border: `1px solid ${D.bind}`, borderRadius: 3, background: 'var(--bg)', display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ display: 'flex' }}>{seg('tag', 'Tag')}{seg('expr', 'Expression')}{seg('map', 'Map')}</div>
      {draft.kind === 'tag' && (
        <TagPicker sensors={sensors} value={draft.tag ? index.get(draft.tag) ?? null : null} placeholder="Pick a tag"
          onChange={id => setDraft({ kind: 'tag', tag: tagById(id) })} />
      )}
      {draft.kind !== 'tag' && (
        <>
          <textarea value={draft.expr} rows={draft.kind === 'expr' ? 3 : 1} spellCheck={false} aria-label="Expression" data-testid="designer-expr"
            placeholder={draft.kind === 'expr' ? '{device/tag} > 80 ? "HIGH" : "OK"' : '{device/tag}'}
            onChange={e => setDraft({ ...draft, expr: e.target.value })}
            onDragOver={e => { if (e.dataTransfer.types.includes(DND_TAG)) e.preventDefault() }}
            onDrop={e => { const id = parseInt(e.dataTransfer.getData(DND_TAG), 10); if (!isNaN(id)) { e.preventDefault(); insertTag(id) } }}
            style={{ ...input, height: 'auto', padding: '4px 6px', resize: 'vertical', ...mono }} />
          <div style={{ display: 'flex', gap: 4 }}>
            <TagPicker sensors={sensors} value={null} placeholder="Insert tag…" onChange={insertTag} />
          </div>
          {draft.kind === 'expr' && (
            <div style={{ fontSize: 10, color: 'var(--text-muted)', lineHeight: 1.4 }}>
              + − × ÷ %, comparisons, && || !, a ? b : c, "text". Functions: {Object.keys(FUNCTIONS).join(', ')}.
            </div>
          )}
        </>
      )}
      {draft.kind === 'map' && <MapCases prop={prop} draft={draft} onChange={setDraft} />}
      <div style={{ fontSize: D.small, ...mono, color: errors.length ? 'var(--crit)' : 'var(--text-dim)' }} data-testid="designer-binding-preview">
        {errors.length ? errors.join(' · ') : empty ? 'Nothing bound yet' : `Now: ${preview === undefined ? 'no value' : JSON.stringify(preview)}`}
      </div>
      <div style={{ display: 'flex', gap: 4, justifyContent: 'flex-end' }}>
        {binding && <SmallBtn danger onClick={() => { onChange(null); onClose() }}>Remove binding</SmallBtn>}
        <SmallBtn onClick={onClose}>Cancel</SmallBtn>
        <SmallBtn testId="designer-binding-apply" disabled={empty || errors.length > 0} onClick={() => { onChange(draft); onClose() }}>Apply</SmallBtn>
      </div>
    </div>
  )
}

function MapCases({ prop, draft, onChange }: { prop: ConfigFieldDef; draft: Extract<Binding, { kind: 'map' }>; onChange: (b: Binding) => void }) {
  const setCase = (i: number, c: Partial<MapCase>) => onChange({ ...draft, cases: draft.cases.map((x, j) => (j === i ? { ...x, ...c } : x)) })
  const outInput = (v: unknown, set: (v: unknown) => void, label: string) => prop.type === 'boolean'
    ? <input type="checkbox" checked={!!v} onChange={e => set(e.target.checked)} aria-label={label} />
    : prop.type === 'color'
      ? <input type="color" value={typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v) ? v : '#8b939e'} onChange={e => set(e.target.value)} aria-label={label} style={{ width: 34, height: 20, border: 'none', padding: 0, background: 'none' }} />
      // An emptied number keeps no value (the property keeps its fixed value) instead of turning into 0.
      : <input value={v == null ? '' : String(v)} onChange={e => set(prop.type === 'number' ? (e.target.value.trim() === '' ? undefined : Number(e.target.value)) : e.target.value)} style={input} aria-label={label} />
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
      {draft.cases.map((c, i) => (
        <div key={i} style={{ display: 'grid', gridTemplateColumns: '44px 52px 1fr 20px', gap: 3, alignItems: 'center' }}>
          <select value={c.op} onChange={e => setCase(i, { op: e.target.value as MapCase['op'] })} style={input} aria-label="Compare">
            {['==', '>=', '>', '<=', '<'].map(o => <option key={o} value={o}>{o}</option>)}
          </select>
          <input type="number" value={Number.isFinite(c.v) ? c.v : ''} onChange={e => setCase(i, { v: e.target.value === '' ? NaN : Number(e.target.value) })} style={input} aria-label="Value" />
          {outInput(c.out, v => setCase(i, { out: v }), 'Result')}
          <button type="button" aria-label="Remove case" onClick={() => onChange({ ...draft, cases: draft.cases.filter((_, j) => j !== i) })}
            style={{ border: 'none', background: 'none', color: 'var(--text-muted)', cursor: 'pointer' }}>×</button>
        </div>
      ))}
      <div style={{ display: 'grid', gridTemplateColumns: '99px 1fr 20px', gap: 3, alignItems: 'center' }}>
        <span style={{ fontSize: D.small, color: 'var(--text-muted)' }}>otherwise</span>
        {outInput(draft.else, v => onChange({ ...draft, else: v }), 'Otherwise')}
      </div>
      <SmallBtn onClick={() => onChange({ ...draft, cases: [...draft.cases, { op: '>=', v: 0, out: prop.type === 'boolean' ? true : '' }] })}>+ Case</SmallBtn>
    </div>
  )
}
