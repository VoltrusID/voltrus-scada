import React, { useEffect, useMemo, useRef, useState } from 'react'
import type { SensorConfig } from '../types'
import type { WidgetInstance } from '../scada2/widgets/types'
import { SYMBOL_MAP, SYMBOLS } from '../scada2/hmi/symbols'
import { SymbolThumb } from '../scada2/hmi/SymbolView'
import {
  createInstance, updateInstance,
  type EquipmentInstance, type EquipmentInstanceInput, type EquipmentType, type ResolvedRole, type RoleOverride,
} from '../queries/equipment'
import { DND_EQUIPMENT } from './DesignerCanvas'
import { defaultPrefix, instanceFieldErrors, suggestName } from './equipmentModel'
import { D, GridSection, Icon, PropRow, SmallBtn, input, mono } from './ui'

// Equipment types in the Designer: the palette section, the instance picker
// that opens when a type is dropped, the Equipment folder of the project
// tree and the instance section of the property editor.

const STATUS_COLOR: Record<ResolvedRole['status'], string> = {
  bound: 'var(--fg)', created: D.bind, fallback: 'var(--text-dim)', missing: 'var(--crit)', skipped: 'var(--text-muted)',
}
const STATUS_TEXT: Record<ResolvedRole['status'], string> = {
  bound: 'bound', created: 'new tag', fallback: 'fallback', missing: 'missing', skipped: 'skipped',
}

const PaletteTitle = ({ children }: { children: React.ReactNode }) => (
  <div style={{ fontSize: 10, fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', color: 'var(--text-muted)', margin: '2px 0 4px' }}>{children}</div>
)

/** Palette tiles: one per equipment type. Drag onto the stage (or double click) to place an instance. */
export function EquipmentTypesSection({ types, query, onAdd, onInstall, installing }: {
  types: EquipmentType[]; query: string; onAdd: (typeId: string) => void; onInstall: () => void; installing: boolean
}) {
  const q = query.trim().toLowerCase()
  const shown = types.filter(t => !q || `${t.name} ${t.id} ${t.category ?? ''} ${t.description ?? ''}`.toLowerCase().includes(q))
  if (q && shown.length === 0) return null
  return (
    <div data-testid="designer-equipment-types">
      <PaletteTitle>Equipment types</PaletteTitle>
      {types.length === 0 ? (
        <div style={{ fontSize: D.small, color: 'var(--text-muted)', lineHeight: 1.5, marginBottom: 8 }}>
          No equipment types yet. A type is a pump, valve or tank with its tags, alarms, faceplate and symbol; an instance is one line.
          <div style={{ marginTop: 6 }}>
            <SmallBtn onClick={onInstall} disabled={installing} testId="designer-equipment-install">{installing ? 'Adding…' : 'Add the starter types'}</SmallBtn>
          </div>
        </div>
      ) : (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 4, marginBottom: 8 }}>
          {shown.map(t => {
            const def = SYMBOL_MAP[t.symbol]
            return (
              <div key={t.id} draggable data-testid={`designer-equipment-${t.id}`}
                title={`${t.name}${t.description ? `: ${t.description}` : ''}\n${t.tags.length} tags · ${t.instance_count ?? 0} instances\nDrop on the screen to place an instance.`}
                onDragStart={ev => { ev.dataTransfer.setData(DND_EQUIPMENT, t.id); ev.dataTransfer.effectAllowed = 'copy' }}
                onDoubleClick={() => onAdd(t.id)}
                style={{
                  display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 2, padding: '5px 2px 4px', borderRadius: 3, cursor: 'grab',
                  border: `1px solid ${D.line}`, background: 'var(--hmi-stage)', color: 'var(--text-dim)', fontSize: 9.5, lineHeight: 1.15,
                  textAlign: 'center', minWidth: 0,
                }}>
                {def ? <SymbolThumb def={def} size={30} /> : <Icon name="faceplate" size={24} />}
                <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: '100%', whiteSpace: 'nowrap' }}>{t.name}</span>
                <span style={{ ...mono, fontSize: 9, color: 'var(--text-muted)' }}>{t.instance_count ?? 0}</span>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

// ── Modal shell ──

function Modal({ title, children, onCancel, testId, width = 560 }: {
  title: React.ReactNode; children: React.ReactNode; onCancel: () => void; testId?: string; width?: number
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Keys stay in the dialog: the Designer's shortcuts (Delete, arrows, Ctrl+Z) must not act on the stage.
      e.stopImmediatePropagation()
      if (e.key === 'Escape') { e.preventDefault(); onCancel() }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onCancel])
  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 90, display: 'grid', placeItems: 'center', background: 'rgba(0,0,0,.45)' }}
      onMouseDown={e => { if (e.target === e.currentTarget) onCancel() }}>
      <div role="dialog" aria-modal="true" aria-label={typeof title === 'string' ? title : undefined} data-testid={testId} style={{
        width, maxWidth: 'calc(100vw - 32px)', maxHeight: 'calc(100vh - 64px)', display: 'flex', flexDirection: 'column', borderRadius: 6,
        background: 'var(--card)', color: 'var(--fg)', border: `1px solid ${D.line}`, boxShadow: '0 12px 40px rgba(0,0,0,.5)', fontSize: D.font,
        userSelect: 'text',
      }}>
        <div style={{ padding: '12px 16px 8px', fontSize: 14, fontWeight: 700, display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ flex: 1 }}>{title}</span>
          <button type="button" aria-label="Close" onClick={onCancel} style={{ background: 'none', border: 'none', color: 'var(--text-dim)', cursor: 'pointer' }}><Icon name="close" /></button>
        </div>
        {children}
      </div>
    </div>
  )
}

const label: React.CSSProperties = { fontSize: D.small, color: 'var(--text-dim)', display: 'flex', flexDirection: 'column', gap: 3 }

export function RolesTable({ roles, testId = 'designer-instance-role' }: { roles: ResolvedRole[]; testId?: string }) {
  return (
    <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: D.small }}>
      <thead>
        <tr style={{ color: 'var(--text-muted)', textAlign: 'left' }}>
          <th style={{ fontWeight: 600, padding: '2px 4px' }}>Role</th>
          <th style={{ fontWeight: 600, padding: '2px 4px' }}>Tag</th>
          <th style={{ fontWeight: 600, padding: '2px 4px', textAlign: 'right' }}>Status</th>
        </tr>
      </thead>
      <tbody>
        {roles.map(r => (
          <tr key={r.role} data-testid={testId} data-role={r.role} data-status={r.status} style={{ borderTop: `1px solid ${D.line}` }}>
            <td style={{ padding: '3px 4px', whiteSpace: 'nowrap' }}>{r.label || r.role} <span style={{ color: 'var(--text-muted)' }}>{r.kind}</span></td>
            <td style={{ padding: '3px 4px', ...mono, overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: 220 }}>{r.status === 'skipped' ? '—' : r.ref}</td>
            <td style={{ padding: '3px 4px', textAlign: 'right', color: STATUS_COLOR[r.status], whiteSpace: 'nowrap' }}>{STATUS_TEXT[r.status]}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

/**
 * Opens when an equipment type is dropped: pick an existing instance of the
 * type, or create one (name, device, base), previewed by a dry run.
 */
export function InstancePicker({ type, instances, onScreen, devices, modbusDevices, onPick, onCancel }: {
  type: EquipmentType; instances: EquipmentInstance[]; onScreen: Map<string, string[]>
  devices: string[]; modbusDevices: Set<string>
  onPick: (inst: EquipmentInstance) => void; onCancel: () => void
}) {
  const own = instances.filter(i => i.type === type.id)
  const [tab, setTab] = useState<'existing' | 'new'>(own.length ? 'existing' : 'new')
  const lastDevice = own[own.length - 1]?.device
  const [f, setF] = useState(() => ({
    name: suggestName(type, instances), device: lastDevice ?? devices[0] ?? '', base: '', prefix: '',
  }))
  const [preview, setPreview] = useState<EquipmentInstance | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const errs = instanceFieldErrors(f, instances)
  const body: EquipmentInstanceInput = {
    name: f.name.trim(), type: type.id, device: f.device.trim(),
    base: f.base.trim() ? Number(f.base.trim()) : null, prefix: f.prefix.trim() || null,
  }
  const bodyKey = JSON.stringify(body)
  const seq = useRef(0)

  // Dry run as the fields change: which tags the instance binds, creates or misses.
  useEffect(() => {
    if (tab !== 'new' || errs.length) { setPreview(null); return }
    const n = ++seq.current
    const t = setTimeout(() => {
      createInstance(body, true)
        .then(r => { if (n === seq.current) { setPreview(r); setError(null) } })
        .catch(e => { if (n === seq.current) { setPreview(null); setError(String(e?.message ?? e)) } })
    }, 250)
    return () => clearTimeout(t)
  }, [bodyKey, tab, errs.length]) // eslint-disable-line react-hooks/exhaustive-deps

  const create = async () => {
    if (errs.length || busy) return
    setBusy(true)
    try {
      onPick(await createInstance(body, false))
    } catch (e: any) {
      setError(String(e?.message ?? e))
    } finally {
      setBusy(false)
    }
  }

  const tabBtn = (id: 'existing' | 'new', text: string) => (
    <button type="button" onClick={() => setTab(id)} data-testid={`designer-instance-tab-${id}`} aria-pressed={tab === id} style={{
      height: 26, padding: '0 12px', border: 'none', borderBottom: `2px solid ${tab === id ? D.accent : 'transparent'}`, background: 'none',
      color: tab === id ? 'var(--fg)' : 'var(--text-dim)', cursor: 'pointer', fontSize: D.font, fontWeight: tab === id ? 600 : 400,
    }}>{text}</button>
  )

  return (
    <Modal title={<>Place {type.name}</>} onCancel={onCancel} testId="designer-instance-picker">
      <div style={{ display: 'flex', gap: 2, padding: '0 12px', borderBottom: `1px solid ${D.line}` }}>
        {tabBtn('existing', `Existing (${own.length})`)}
        {tabBtn('new', 'New instance')}
      </div>
      <div style={{ padding: 16, overflow: 'auto', display: 'flex', flexDirection: 'column', gap: 10 }}>
        {tab === 'existing' ? (
          own.length === 0 ? <div style={{ color: 'var(--text-muted)' }}>No {type.name} instances yet.</div> : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              {own.map(i => {
                const placed = onScreen.get(i.name)?.length ?? 0
                const roles = i.resolved?.roles ?? []
                const ok = roles.filter(r => r.status === 'bound' || r.status === 'created' || r.status === 'fallback').length
                return (
                  <button key={i.name} type="button" data-testid={`designer-instance-existing-${i.name}`} onClick={() => onPick(i)} style={{
                    display: 'grid', gridTemplateColumns: '1fr auto', gap: 2, textAlign: 'left', padding: '6px 8px', borderRadius: 3, cursor: 'pointer',
                    border: `1px solid ${D.line}`, background: 'transparent', color: 'var(--fg)', fontSize: D.font,
                  }}>
                    <span style={{ fontWeight: 600 }}>{i.name}</span>
                    <span style={{ ...mono, color: placed ? D.accent : 'var(--text-muted)' }}>{placed ? `on this screen ×${placed}` : ''}</span>
                    <span style={{ ...mono, color: 'var(--text-muted)' }}>{i.device}{i.base != null ? ` @${i.base}` : ''} · {i.resolved?.prefix ?? i.prefix}</span>
                    <span style={{ ...mono, color: 'var(--text-muted)' }}>{ok}/{roles.length} tags</span>
                  </button>
                )
              })}
            </div>
          )
        ) : (
          <>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
              <label style={label}>Name
                <input autoFocus value={f.name} data-testid="designer-instance-name" style={input}
                  onChange={e => setF({ ...f, name: e.target.value })} onKeyDown={e => { if (e.key === 'Enter') void create() }} />
              </label>
              <label style={label}>Device
                <select value={f.device} data-testid="designer-instance-device" style={input} onChange={e => setF({ ...f, device: e.target.value })}>
                  {!devices.includes(f.device) && <option value={f.device}>{f.device || '(pick a device)'}</option>}
                  {devices.map(d => <option key={d} value={d}>{d}{modbusDevices.has(d) ? ' (Modbus)' : ''}</option>)}
                </select>
              </label>
              <label style={label} title="Tag keys are the type's patterns with {prefix}; by default the name in lower case without punctuation.">Tag prefix
                <input value={f.prefix} placeholder={defaultPrefix(f.name) || 'from the name'} data-testid="designer-instance-prefix" style={input}
                  onChange={e => setF({ ...f, prefix: e.target.value })} />
              </label>
              <label style={label} title="Register address the type's offsets start from. Empty: bind to the device's existing tags by name.">Base address
                <input value={f.base} placeholder={modbusDevices.has(f.device) ? 'empty: existing tags' : 'Modbus devices only'} inputMode="numeric"
                  data-testid="designer-instance-base" style={input} disabled={!modbusDevices.has(f.device)}
                  onChange={e => setF({ ...f, base: e.target.value })} />
              </label>
            </div>
            {errs.length > 0 && <div style={{ color: 'var(--crit)', fontSize: D.small }}>{errs.join(' · ')}</div>}
            {error && <div data-testid="designer-instance-error" style={{ color: 'var(--crit)', fontSize: D.small, whiteSpace: 'pre-wrap' }}>{error}</div>}
            {preview && (
              <div>
                <div style={{ fontSize: D.small, color: 'var(--text-dim)', marginBottom: 4 }}>
                  {summary(preview)}
                </div>
                <RolesTable roles={preview.resolved.roles} />
              </div>
            )}
          </>
        )}
      </div>
      {tab === 'new' && (
        <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end', padding: '10px 16px', borderTop: `1px solid ${D.line}` }}>
          <SmallBtn onClick={onCancel}>Cancel</SmallBtn>
          <button type="button" onClick={() => void create()} disabled={errs.length > 0 || busy} data-testid="designer-instance-create" style={{
            height: 26, padding: '0 12px', borderRadius: 3, cursor: errs.length ? 'default' : 'pointer', fontSize: D.font, fontWeight: 600,
            border: `1px solid ${D.accent}`, background: D.accentBg, color: D.accent, opacity: errs.length || busy ? 0.5 : 1,
          }}>{busy ? 'Creating…' : 'Create and place'}</button>
        </div>
      )}
    </Modal>
  )
}

function summary(i: EquipmentInstance): string {
  const roles = i.resolved.roles
  const n = (s: ResolvedRole['status']) => roles.filter(r => r.status === s).length
  const parts = [`${n('bound')} bound to existing tags`]
  if (n('created')) parts.push(`${n('created')} new tags`)
  if (n('fallback')) parts.push(`${n('fallback')} via fallback`)
  if (n('missing')) parts.push(`${n('missing')} missing`)
  return parts.join(' · ')
}

// ── Project tree ──

const row = (depth: number, active = false): React.CSSProperties => ({
  display: 'flex', alignItems: 'center', gap: 5, height: 22, padding: `0 6px 0 ${6 + depth * 14}px`, cursor: 'pointer', fontSize: D.font,
  background: active ? D.selBg : undefined, color: active ? 'var(--fg)' : 'var(--text-dim)', whiteSpace: 'nowrap', userSelect: 'none',
})

/** The Equipment folder: types, and under each its instances (count of symbols on this screen). */
export function EquipmentTree({ types, instances, onScreen, selected, onSelectInstance }: {
  types: EquipmentType[]; instances: EquipmentInstance[]; onScreen: Map<string, string[]>; selected: string | null
  onSelectInstance: (name: string) => void
}) {
  const [open, setOpen] = useState<Record<string, boolean>>({ __root: false })
  const toggle = (k: string) => setOpen(o => ({ ...o, [k]: !(o[k] ?? false) }))
  const chevron = (on: boolean) => (
    <span style={{ width: 10, display: 'inline-flex', transform: on ? 'rotate(90deg)' : undefined }}><Icon name="chevron" size={10} /></span>
  )
  return (
    <div data-testid="designer-equipment-tree">
      <div style={row(0)} onClick={() => toggle('__root')}>
        {chevron(!!open.__root)}<Icon name="folder" size={13} />
        <span style={{ flex: 1 }}>Equipment</span>
        <span style={{ ...mono, color: 'var(--text-muted)' }}>{instances.length}</span>
      </div>
      {open.__root && (types.length === 0
        ? <div style={{ ...row(1), cursor: 'default', color: 'var(--text-muted)' }}>No types yet: add them from Components</div>
        : types.map(t => {
          const own = instances.filter(i => i.type === t.id)
          return (
            <React.Fragment key={t.id}>
              <div style={row(1)} onClick={() => toggle(t.id)} data-testid={`designer-equipment-type-${t.id}`}>
                {chevron(!!open[t.id])}<Icon name="faceplate" size={13} />
                <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis' }}>{t.name}</span>
                <span style={{ ...mono, color: 'var(--text-muted)' }}>{own.length}</span>
              </div>
              {open[t.id] && own.map(i => {
                const n = onScreen.get(i.name)?.length ?? 0
                return (
                  <div key={i.name} data-testid={`designer-equipment-instance-${i.name}`} style={row(2, selected === i.name)}
                    title={`${i.name}: ${t.name} on ${i.device}${n ? `, ${n} on this screen (click to select)` : ''}`}
                    onClick={() => onSelectInstance(i.name)}>
                    <span style={{ width: 10 }} />
                    <Icon name="tag" size={12} />
                    <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis' }}>{i.name}</span>
                    <span style={{ ...mono, color: n ? D.accent : 'var(--text-muted)' }}>{n ? `×${n}` : i.device}</span>
                  </div>
                )
              })}
            </React.Fragment>
          )
        }))}
    </div>
  )
}

// ── Property editor section ──

/**
 * The equipment a selected symbol shows: its type, device, tags per role, and
 * the instance's overrides (symbol, per-role tag key or skip). Applying
 * changes the instance, so every symbol of it on every screen follows.
 */
export function InstanceSection({ w, name, instance, type, sensors, onApply, onUnbind, onRebind }: {
  w: WidgetInstance; name: string; instance: EquipmentInstance | undefined; type: EquipmentType | undefined; sensors: SensorConfig[]
  onApply: (name: string, body: EquipmentInstanceInput) => Promise<void>
  onUnbind: () => void
  onRebind: (inst: EquipmentInstance) => void
}) {
  const [draft, setDraft] = useState<{ symbol: string; overrides: Record<string, RoleOverride> } | null>(null)
  const [preview, setPreview] = useState<EquipmentInstance | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const symbols = useMemo(() => SYMBOLS.filter(s => s.key.startsWith('isa_')).map(s => s.key).sort(), [])

  if (!instance) {
    return (
      <GridSection title="Equipment">
        <PropRow label="Instance"><span style={{ ...mono, color: 'var(--crit)' }}>{name} (not found)</span></PropRow>
        <div style={{ padding: '4px 10px 8px' }}><SmallBtn onClick={onUnbind}>Unbind</SmallBtn></div>
      </GridSection>
    )
  }
  const r = instance.resolved
  // Is this symbol bound as the instance says? (out of date after an edit made elsewhere, or a hand-edited tag)
  const stale = w.sensorId !== (r.widget.sensor_id ?? null) || w.config?.symbol !== r.widget.config.symbol
  const body = (d: NonNullable<typeof draft>): EquipmentInstanceInput => ({
    name: instance.name, type: instance.type, device: instance.device, base: instance.base ?? null, prefix: instance.prefix ?? null,
    description: instance.description ?? '', symbol: d.symbol || null,
    overrides: Object.fromEntries(Object.entries(d.overrides).filter(([, o]) => Object.keys(o).length > 0)),
  })
  const edit = draft ?? { symbol: instance.symbol ?? '', overrides: structuredClone(instance.overrides ?? {}) }
  const setOverride = (role: string, o: RoleOverride) => {
    const next = { ...edit, overrides: { ...edit.overrides, [role]: Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== '' && v !== false)) } }
    setDraft(next); setPreview(null)
  }
  const runPreview = async () => {
    try { setPreview(await updateInstance(instance.name, body(edit), true)); setError(null) } catch (e: any) { setError(String(e?.message ?? e)) }
  }
  const apply = async () => {
    setBusy(true)
    try { await onApply(instance.name, body(edit)); setDraft(null); setPreview(null); setError(null) } catch (e: any) { setError(String(e?.message ?? e)) } finally { setBusy(false) }
  }
  const tagName = (id: number | null) => (id == null ? '' : sensors.find(s => s.id === id)?.name ?? '')

  return (
    <GridSection title="Equipment">
      <div data-testid="designer-props-equipment">
        <PropRow label="Instance"><span style={{ fontSize: D.font, fontWeight: 600 }} data-testid="designer-equipment-name">{instance.name}</span></PropRow>
        <PropRow label="Type"><span style={{ fontSize: D.font }} data-testid="designer-equipment-type">{type?.name ?? instance.type}</span></PropRow>
        <PropRow label="Device"><span style={mono}>{instance.device}{instance.base != null ? ` · base ${instance.base}` : ''}</span></PropRow>
        <PropRow label="Prefix"><span style={mono}>{r.prefix}</span></PropRow>
        <PropRow label="Faceplate"><span style={mono}>{r.faceplate ?? '—'}</span></PropRow>
        <PropRow label="Symbol" title="Override the type's symbol for this instance (every symbol of it follows).">
          <select value={edit.symbol} style={input} data-testid="designer-equipment-symbol"
            onChange={e => { setDraft({ ...edit, symbol: e.target.value }); setPreview(null) }}>
            <option value="">Type default ({type?.symbol ?? r.symbol})</option>
            {symbols.map(k => <option key={k} value={k}>{SYMBOL_MAP[k]?.name ?? k}</option>)}
          </select>
        </PropRow>
        <div style={{ padding: '4px 6px' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: D.small }}>
            <tbody>
              {r.roles.map(role => {
                const o = edit.overrides[role.role] ?? {}
                return (
                  <tr key={role.role} data-testid="designer-equipment-role" data-role={role.role} data-status={role.status}
                    title={`${role.label || role.role} (${role.kind}): ${STATUS_TEXT[role.status]}${role.tag_id != null ? `\n${tagName(role.tag_id)}` : ''}`}
                    style={{ borderTop: `1px solid color-mix(in srgb, ${D.line} 55%, transparent)` }}>
                    <td style={{ padding: '2px 4px', color: 'var(--text-dim)', whiteSpace: 'nowrap' }}>{role.label || role.role}</td>
                    <td style={{ padding: '2px 4px', width: '100%' }}>
                      <input value={o.key ?? ''} placeholder={role.key} disabled={!!o.skip} aria-label={`Tag key of ${role.role}`}
                        style={{ ...input, ...mono, height: 20, color: o.key ? 'var(--fg)' : undefined }}
                        onChange={e => setOverride(role.role, { ...o, key: e.target.value || undefined })} />
                    </td>
                    <td style={{ padding: '2px 4px', color: STATUS_COLOR[role.status], whiteSpace: 'nowrap' }}>{STATUS_TEXT[role.status]}</td>
                    <td style={{ padding: '2px 0' }}>
                      <input type="checkbox" title="Skip this role" aria-label={`Skip ${role.role}`} checked={!!o.skip}
                        onChange={e => setOverride(role.role, { ...o, skip: e.target.checked || undefined })} />
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
        {error && <div style={{ padding: '2px 10px', color: 'var(--crit)', fontSize: D.small, whiteSpace: 'pre-wrap' }}>{error}</div>}
        {preview?.changes && (
          <div data-testid="designer-equipment-preview" style={{ padding: '2px 10px', fontSize: D.small, color: 'var(--text-dim)' }}>
            {preview.changes.tags.filter(t => t.action !== 'unchanged').length} tag changes · {preview.changes.widgets.length} symbols re-bound on all screens
          </div>
        )}
        <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', padding: '4px 10px 8px' }}>
          {draft && <SmallBtn onClick={() => void runPreview()} testId="designer-equipment-preview-btn">Preview</SmallBtn>}
          {draft && <SmallBtn onClick={() => void apply()} disabled={busy} testId="designer-equipment-apply"
            title="Saves the screen first if it has changes, then changes the instance; every symbol of it on every screen is re-bound">Apply to instance</SmallBtn>}
          {draft && <SmallBtn onClick={() => { setDraft(null); setPreview(null) }}>Reset</SmallBtn>}
          {stale && !draft && <SmallBtn onClick={() => onRebind(instance)} testId="designer-equipment-rebind" title="Bind this symbol as the instance says">Re-bind</SmallBtn>}
          <SmallBtn onClick={onUnbind} title="Keep the tags and the symbol, forget the instance">Unbind</SmallBtn>
        </div>
      </div>
    </GridSection>
  )
}
