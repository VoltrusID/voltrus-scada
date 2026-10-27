import { useEffect, useState } from 'react'
import {
  useDeviceTemplates, useDeviceTemplate, useDeviceCatalog, useInstantiateDeviceTemplate, useSaveDeviceAsTemplate,
  useImportTemplateFile, useDeleteDeviceTemplate, useImportFromCatalog,
} from '../queries'
import { btnStyle, inputStyle, labelStyle, labelTextStyle, monoInputStyle, tabStyle } from '../styles'
import { ImportFileButton, ImportNotice } from './FileDropZone'
import {
  DEFAULT_CATALOG_URL, filterCatalog, fromDeviceBody, instantiateBody, matchesFilter, refQuery, sourceLabel, templateKey,
} from '../deviceTemplates'
import type { CatalogEntry, TemplateProtocol, TemplateSummary } from '../deviceTemplates'

const PROTOCOL_LABEL: Record<TemplateProtocol, string> = { modbus: 'Modbus', bacnet: 'BACnet' }

const thStyle: React.CSSProperties = { textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, borderBottom: '1px solid var(--border)' }
const tdStyle: React.CSSProperties = { padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)' }
const smallBtn: React.CSSProperties = { padding: '4px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 11, textDecoration: 'none' }

function Badge({ children, color }: { children: React.ReactNode; color: string }) {
  return <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: `color-mix(in srgb, ${color} 14%, transparent)`, color }}>{children}</span>
}

function VerifiedBadge({ verified }: { verified: boolean }) {
  return verified ? <Badge color="var(--ok)">Verified</Badge> : <Badge color="var(--muted)">Unverified</Badge>
}

function Modal({ title, onClose, children, width = 520 }: { title: string; onClose: () => void; children: React.ReactNode; width?: number }) {
  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 80 }} onClick={onClose}>
      <div role="dialog" aria-label={title} style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width, maxWidth: '95vw', maxHeight: '90vh', overflow: 'auto' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center' }}>
          <span style={{ fontSize: 14, fontWeight: 600 }}>{title}</span>
          <button onClick={onClose} aria-label="Close" style={{ marginLeft: 'auto', background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18, lineHeight: 1 }}>&times;</button>
        </div>
        <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 10 }}>{children}</div>
      </div>
    </div>
  )
}

function Errors({ lines }: { lines: string[] }) {
  if (lines.length === 0) return null
  return <div role="alert" style={{ color: 'var(--crit)', fontSize: 12 }}>{lines.map((l, i) => <div key={i}>{l}</div>)}</div>
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

/** Create a device from a template: where it is (IP, port, unit or device instance) and what to call it. */
function CreateFromTemplate({ template, onClose, onCreated }: { template: TemplateSummary; onClose: () => void; onCreated: (line: string) => void }) {
  const { data: detail } = useDeviceTemplate(template)
  const instantiate = useInstantiateDeviceTemplate()
  const [form, setForm] = useState({ name: '', ip: '', port: '', unitId: '', deviceInstance: '', intervalMs: '' })
  const [errors, setErrors] = useState<string[]>([])
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, [k]: e.target.value })

  const submit = () => {
    const built = instantiateBody(form, template.protocol, { version: template.version, source: template.source })
    if (!built.body) { setErrors(built.errors); return }
    setErrors([])
    instantiate.mutate({ templateId: template.template_id, body: built.body }, {
      onSuccess: r => { onCreated(`Created ${r.name} from ${template.manufacturer} ${template.model} with ${r.item_count} ${template.protocol === 'modbus' ? 'registers' : 'objects'}`); onClose() },
      onError: e => setErrors([message(e)]),
    })
  }

  return (
    <Modal title={`Create device from ${template.manufacturer} ${template.model}`} onClose={onClose}>
      <div style={{ fontSize: 12, color: 'var(--muted)' }}>
        {template.item_count} {template.protocol === 'modbus' ? 'registers' : 'objects'} from the template. Fields left empty keep the template's value.
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <label style={labelStyle}><span style={labelTextStyle}>Name</span><input value={form.name} onChange={set('name')} placeholder="Main incomer" style={inputStyle} autoFocus /></label>
        <label style={labelStyle}><span style={labelTextStyle}>IP address</span><input value={form.ip} onChange={set('ip')} placeholder="192.168.1.20" style={monoInputStyle} /></label>
        <label style={labelStyle}><span style={labelTextStyle}>Port</span><input value={form.port} onChange={set('port')} inputMode="numeric" placeholder={String(detail?.port ?? '')} style={monoInputStyle} /></label>
        {template.protocol === 'modbus' ? (
          <label style={labelStyle}><span style={labelTextStyle}>Unit id</span><input value={form.unitId} onChange={set('unitId')} inputMode="numeric" placeholder={String(detail?.unit_id ?? 1)} style={monoInputStyle} /></label>
        ) : (
          <label style={labelStyle}><span style={labelTextStyle}>Device instance</span><input value={form.deviceInstance} onChange={set('deviceInstance')} inputMode="numeric" placeholder="0" style={monoInputStyle} /></label>
        )}
        <label style={labelStyle}><span style={labelTextStyle}>Interval (ms)</span><input value={form.intervalMs} onChange={set('intervalMs')} inputMode="numeric" placeholder={String(detail?.interval_ms ?? 1000)} style={monoInputStyle} /></label>
      </div>
      {detail?.quirks && detail.quirks.length > 0 && (
        <div style={{ fontSize: 12, border: '1px solid var(--border)', borderRadius: 6, padding: '6px 10px' }}>
          <div style={{ color: 'var(--muted)', textTransform: 'uppercase', fontSize: 11, marginBottom: 2 }}>Quirks</div>
          {detail.quirks.map((q, i) => <div key={i}>{q}</div>)}
        </div>
      )}
      <Errors lines={errors} />
      <div style={{ display: 'flex', gap: 8 }}>
        <button onClick={submit} disabled={instantiate.isPending} style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}>{instantiate.isPending ? 'Creating...' : 'Create device'}</button>
        <button onClick={onClose} style={{ ...btnStyle, color: 'var(--muted)' }}>Cancel</button>
      </div>
    </Modal>
  )
}

/** "Save as template" for a device of the manager this is opened from. */
export function SaveAsTemplateDialog({ protocol, deviceId, deviceName, onClose, onSaved }: {
  protocol: TemplateProtocol
  deviceId: string | number
  deviceName: string
  onClose: () => void
  onSaved?: (line: string) => void
}) {
  const save = useSaveDeviceAsTemplate()
  const [form, setForm] = useState({ manufacturer: '', model: '', templateId: '', version: '1.0.0', description: '', verified: false, verifiedNotes: '', quirksText: '', license: 'CC-BY-4.0' })
  const [errors, setErrors] = useState<string[]>([])
  const set = (k: 'manufacturer' | 'model' | 'templateId' | 'version' | 'description' | 'verifiedNotes' | 'quirksText' | 'license') =>
    (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setForm({ ...form, [k]: e.target.value })

  const submit = () => {
    const built = fromDeviceBody(form, protocol)
    if (!built.body) { setErrors(built.errors); return }
    setErrors([])
    save.mutate({ deviceId, body: built.body }, {
      onSuccess: r => { onSaved?.(`Saved ${deviceName} as template ${r.template_id} ${r.version}`); onClose() },
      onError: e => setErrors([message(e)]),
    })
  }

  return (
    <Modal title={`Save ${deviceName} as a template`} onClose={onClose} width={560}>
      <div style={{ fontSize: 12, color: 'var(--muted)' }}>
        Saves the {protocol === 'modbus' ? 'port, unit id, interval, byte order and registers' : 'port, interval, COV settings and objects'}. The address of this device is not saved.
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <label style={labelStyle}><span style={labelTextStyle}>Manufacturer</span><input value={form.manufacturer} onChange={set('manufacturer')} placeholder="Schneider Electric" style={inputStyle} autoFocus /></label>
        <label style={labelStyle}><span style={labelTextStyle}>Model</span><input value={form.model} onChange={set('model')} placeholder="PM5560" style={inputStyle} /></label>
        <label style={labelStyle}><span style={labelTextStyle}>Template id</span><input value={form.templateId} onChange={set('templateId')} placeholder="from manufacturer and model" style={monoInputStyle} /></label>
        <label style={labelStyle}><span style={labelTextStyle}>Version</span><input value={form.version} onChange={set('version')} style={monoInputStyle} /></label>
      </div>
      <label style={labelStyle}><span style={labelTextStyle}>Description</span><input value={form.description} onChange={set('description')} style={inputStyle} /></label>
      <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 13 }}>
        <input type="checkbox" checked={form.verified} onChange={e => setForm({ ...form, verified: e.target.checked })} />
        Verified on a real deployment
      </label>
      {form.verified && (
        <label style={labelStyle}><span style={labelTextStyle}>Verified notes</span><input value={form.verifiedNotes} onChange={set('verifiedNotes')} placeholder="Firmware 2.1.4, checked on site against the front panel" style={inputStyle} /></label>
      )}
      <label style={labelStyle}><span style={labelTextStyle}>Quirks (one per line)</span><textarea value={form.quirksText} onChange={set('quirksText')} rows={3} placeholder={'Low word first\nOffsets are 1-based in the manual'} style={{ ...inputStyle, resize: 'vertical' }} /></label>
      <label style={labelStyle}><span style={labelTextStyle}>License</span><input value={form.license} onChange={set('license')} style={inputStyle} /></label>
      <Errors lines={errors} />
      <div style={{ display: 'flex', gap: 8 }}>
        <button onClick={submit} disabled={save.isPending} style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}>{save.isPending ? 'Saving...' : 'Save template'}</button>
        <button onClick={onClose} style={{ ...btnStyle, color: 'var(--muted)' }}>Cancel</button>
      </div>
    </Modal>
  )
}

/** What a template holds: description, verification, quirks and the registers or objects. */
function TemplateDetail({ template }: { template: TemplateSummary }) {
  const { data: d, isLoading, error } = useDeviceTemplate(template)
  if (isLoading) return <div style={{ padding: 12, color: 'var(--muted)', fontSize: 12 }}>Loading...</div>
  if (error || !d) return <div style={{ padding: 12, color: 'var(--crit)', fontSize: 12 }}>{message(error)}</div>
  const rows = d.protocol === 'modbus' ? (d.registers ?? []) : (d.objects ?? [])
  return (
    <div style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 8, fontSize: 12 }}>
      {d.description && <div>{d.description}</div>}
      <div style={{ color: 'var(--muted)' }}>License {d.license}{d.verified_notes ? ` · ${d.verified_notes}` : ''}</div>
      {d.quirks.length > 0 && (
        <div><div style={{ color: 'var(--muted)', textTransform: 'uppercase', fontSize: 11 }}>Quirks</div>{d.quirks.map((q, i) => <div key={i}>{q}</div>)}</div>
      )}
      <div style={{ maxHeight: 220, overflow: 'auto', border: '1px solid var(--border)', borderRadius: 6 }}>
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              {d.protocol === 'modbus'
                ? ['Name', 'Offset', 'Type', 'Area', 'Scale', 'Unit', 'Description'].map(h => <th key={h} style={thStyle}>{h}</th>)
                : ['Name', 'Type', 'Instance', 'Property', 'Description'].map(h => <th key={h} style={thStyle}>{h}</th>)}
            </tr>
          </thead>
          <tbody>
            {d.protocol === 'modbus'
              ? (d.registers ?? []).map(r => (
                <tr key={r.name}>
                  <td style={tdStyle}>{r.name}</td><td style={{ ...tdStyle, fontFamily: 'var(--font-mono)' }}>{r.offset}</td>
                  <td style={tdStyle}>{r.register_type}</td><td style={tdStyle}>{r.register_area}</td>
                  <td style={tdStyle}>{r.scale}</td><td style={tdStyle}>{r.unit ?? ''}</td><td style={tdStyle}>{r.description ?? ''}</td>
                </tr>
              ))
              : (d.objects ?? []).map(o => (
                <tr key={`${o.object_type}-${o.object_instance}-${o.property}`}>
                  <td style={tdStyle}>{o.name}</td><td style={tdStyle}>{o.object_type}</td>
                  <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)' }}>{o.object_instance}</td>
                  <td style={tdStyle}>{o.property}</td><td style={tdStyle}>{o.description ?? ''}</td>
                </tr>
              ))}
          </tbody>
        </table>
        {rows.length === 0 && <div style={{ padding: 8, color: 'var(--muted)' }}>Empty.</div>}
      </div>
    </div>
  )
}

function Library({ protocol, onNotice }: { protocol: TemplateProtocol; onNotice: (lines: string[]) => void }) {
  const { data: all = [], isLoading, error } = useDeviceTemplates()
  const del = useDeleteDeviceTemplate()
  const importFile = useImportTemplateFile()
  const [q, setQ] = useState('')
  const [source, setSource] = useState('')
  const [open, setOpen] = useState<string | null>(null)
  const [use, setUse] = useState<TemplateSummary | null>(null)

  const rows = all.filter(t => t.protocol === protocol && matchesFilter(t, { q, source }))

  const importFiles = async (files: File[]) => {
    const lines: string[] = []
    for (const file of files) {
      try {
        const r = await importFile.mutateAsync(await file.text())
        lines.push(`${file.name}: imported ${r.template_id} ${r.version}`)
      } catch (e) {
        lines.push(`${file.name}: ${message(e)}`)
      }
    }
    onNotice(lines)
  }

  const remove = (t: TemplateSummary) => {
    if (!confirm(`Delete template ${t.template_id} ${t.version} (${sourceLabel(t.source)})? Devices made from it are not affected.`)) return
    del.mutate(t, { onSuccess: () => onNotice([`Deleted template ${t.template_id} ${t.version}`]), onError: e => onNotice([message(e)]) })
  }

  return (
    <>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12, flexWrap: 'wrap' }}>
        <span style={{ color: 'var(--muted)', fontSize: 13, marginRight: 'auto' }}>{rows.length} template{rows.length !== 1 ? 's' : ''}</span>
        <input placeholder="Search manufacturer, model..." aria-label="Search templates" value={q} onChange={e => setQ(e.target.value)} style={{ ...inputStyle, width: 240 }} />
        <select aria-label="Source" value={source} onChange={e => setSource(e.target.value)} style={{ ...inputStyle, width: 140 }}>
          <option value="">All sources</option><option value="builtin">Built-in</option><option value="local">Local</option><option value="catalog">Catalog</option>
        </select>
        <ImportFileButton onFiles={importFiles} label="Import file" ariaLabel="Import device template file" />
      </div>
      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        {isLoading ? <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>Loading...</div>
          : error ? <div style={{ padding: 24, textAlign: 'center', color: 'var(--crit)' }}>{message(error)}</div>
          : rows.length === 0 ? <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>No templates match.</div>
          : (
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr>{['Manufacturer', 'Model', 'Version', 'Source', 'Status', protocol === 'modbus' ? 'Registers' : 'Objects'].map(h => <th key={h} style={thStyle}>{h}</th>)}<th style={{ ...thStyle, width: 220 }}>Actions</th></tr>
              </thead>
              <tbody>
                {rows.map(t => {
                  const key = templateKey(t)
                  return (
                    <RowFragment key={key} expanded={open === key} detail={<TemplateDetail template={t} />}>
                      <td style={tdStyle}>
                        <button onClick={() => setOpen(open === key ? null : key)} aria-label={open === key ? `Hide ${t.model}` : `Show ${t.model}`} style={{ border: 'none', background: 'transparent', color: 'var(--muted)', cursor: 'pointer', fontSize: 11, padding: '2px 4px' }}>{open === key ? '\u25BC' : '\u25B6'}</button>
                        {t.manufacturer}
                      </td>
                      <td style={tdStyle}>{t.model}</td>
                      <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)', fontSize: 12 }}>{t.version}</td>
                      <td style={tdStyle}><Badge color={t.source === 'builtin' ? 'var(--muted)' : 'var(--cyan)'}>{sourceLabel(t.source)}</Badge></td>
                      <td style={tdStyle}><VerifiedBadge verified={t.verified} /></td>
                      <td style={tdStyle}>{t.item_count}</td>
                      <td style={tdStyle}>
                        <div style={{ display: 'flex', gap: 4 }}>
                          <button onClick={() => setUse(t)} aria-label={`Create device from ${t.model}`} style={{ ...smallBtn, borderColor: 'var(--green)', color: 'var(--green)' }}>Use</button>
                          <a href={`/api/v1/device-templates/${encodeURIComponent(t.template_id)}/export${refQuery(t)}`} download aria-label={`Export ${t.model}`} style={smallBtn}>Export</a>
                          {!t.read_only && <button onClick={() => remove(t)} aria-label={`Delete ${t.model}`} style={{ ...smallBtn, borderColor: 'var(--crit)', color: 'var(--crit)' }}>Del</button>}
                        </div>
                      </td>
                    </RowFragment>
                  )
                })}
              </tbody>
            </table>
          )}
      </div>
      {use && <CreateFromTemplate template={use} onClose={() => setUse(null)} onCreated={line => onNotice([line])} />}
    </>
  )
}

/** A table row with an optional detail row under it. */
function RowFragment({ children, expanded, detail }: { children: React.ReactNode; expanded: boolean; detail: React.ReactNode }) {
  return (
    <>
      <tr>{children}</tr>
      {expanded && <tr><td colSpan={7} style={{ padding: 0, borderBottom: '1px solid var(--border)', background: 'var(--bg)' }}>{detail}</td></tr>}
    </>
  )
}

function Catalog({ protocol, isAdmin, onNotice }: { protocol: TemplateProtocol; isAdmin: boolean; onNotice: (lines: string[]) => void }) {
  const [urlInput, setUrlInput] = useState('')
  const [url, setUrl] = useState('')
  const [q, setQ] = useState('')
  const { data, isLoading, isFetching, error, refetch } = useDeviceCatalog(url, true)
  const importMut = useImportFromCatalog()
  const [busy, setBusy] = useState<string | null>(null)

  const rows = filterCatalog(data?.templates ?? [], q, protocol)

  const doImport = (e: CatalogEntry) => {
    setBusy(`${e.template_id}@${e.version}`)
    importMut.mutate({ template_id: e.template_id, version: e.version, url: url || undefined }, {
      onSuccess: () => onNotice([`Imported ${e.template_id} ${e.version}; it is in the Library tab`]),
      onError: err => onNotice([`${e.template_id} ${e.version} was not imported: ${message(err)}`]),
      onSettled: () => setBusy(null),
    })
  }

  return (
    <>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12, flexWrap: 'wrap' }}>
        <span style={{ color: 'var(--muted)', fontSize: 13, marginRight: 'auto', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 360 }} title={data?.url}>{data ? `${rows.length} of ${data.templates.length} · ${data.url}` : 'Public catalog'}</span>
        <input placeholder="Search manufacturer, model..." aria-label="Search catalog" value={q} onChange={e => setQ(e.target.value)} style={{ ...inputStyle, width: 220 }} />
        {isAdmin && (
          <>
            <input placeholder={DEFAULT_CATALOG_URL} aria-label="Catalog URL" value={urlInput} onChange={e => setUrlInput(e.target.value)} onKeyDown={e => { if (e.key === 'Enter') setUrl(urlInput.trim()) }} style={{ ...monoInputStyle, width: 280, fontSize: 11 }} />
            <button style={btnStyle} onClick={() => { setUrl(urlInput.trim()); if (urlInput.trim() === url) refetch() }} aria-label="Load catalog">Load</button>
          </>
        )}
        {!isAdmin && <button style={btnStyle} onClick={() => refetch()} aria-label="Reload catalog">Reload</button>}
      </div>
      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        {isLoading || isFetching ? <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>Loading catalog...</div>
          : error ? <div role="alert" style={{ padding: 24, textAlign: 'center', color: 'var(--crit)' }}>The catalog could not be loaded: {message(error)}</div>
          : rows.length === 0 ? <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>No {PROTOCOL_LABEL[protocol]} templates in the catalog match.</div>
          : (
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead><tr>{['Manufacturer', 'Model', 'Version', 'Status', 'SHA-256'].map(h => <th key={h} style={thStyle}>{h}</th>)}<th style={{ ...thStyle, width: 120 }}>Actions</th></tr></thead>
              <tbody>
                {rows.map(e => (
                  <tr key={`${e.template_id}@${e.version}`}>
                    <td style={tdStyle}>{e.manufacturer}</td>
                    <td style={tdStyle}>{e.model}</td>
                    <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)', fontSize: 12 }}>{e.version}</td>
                    <td style={tdStyle}><VerifiedBadge verified={e.verified} /></td>
                    <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)', fontSize: 11 }} title={e.sha256}>{e.sha256.slice(0, 12)}</td>
                    <td style={tdStyle}>
                      {e.installed ? <Badge color="var(--ok)">Imported</Badge>
                        : isAdmin ? <button onClick={() => doImport(e)} disabled={busy !== null} aria-label={`Import ${e.template_id} ${e.version}`} style={{ ...smallBtn, borderColor: 'var(--green)', color: 'var(--green)' }}>{busy === `${e.template_id}@${e.version}` ? 'Importing...' : 'Import'}</button>
                        : <span style={{ fontSize: 11, color: 'var(--muted)' }}>Admin only</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
      </div>
    </>
  )
}

/** Browse the template library and the public catalog, and create devices from templates. */
export function DeviceTemplatesPanel({ protocol, onClose, isAdmin = false }: { protocol: TemplateProtocol; onClose: () => void; isAdmin?: boolean }) {
  const [tab, setTab] = useState<'library' | 'catalog'>('library')
  const [notice, setNotice] = useState<string[]>([])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 60, display: 'flex', flexDirection: 'column' }}>
      <div style={{ padding: '16px 24px 0', display: 'flex', alignItems: 'center', borderBottom: '1px solid var(--border)', gap: 16 }}>
        <h2 style={{ fontSize: 18, fontWeight: 700, margin: 0, paddingBottom: 12 }}>{PROTOCOL_LABEL[protocol]} Device Templates</h2>
        <div style={{ display: 'flex', marginLeft: 12 }}>
          <button style={tabStyle(tab === 'library')} onClick={() => setTab('library')}>Library</button>
          <button style={tabStyle(tab === 'catalog')} onClick={() => setTab('catalog')}>Catalog</button>
        </div>
        <button onClick={onClose} aria-label="Close templates" style={{ marginLeft: 'auto', marginBottom: 12, background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4 }}>
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', padding: 24, maxWidth: 1200, width: '100%', margin: '0 auto', boxSizing: 'border-box' }}>
        <ImportNotice lines={notice} onDismiss={() => setNotice([])} />
        {tab === 'library' ? <Library protocol={protocol} onNotice={setNotice} /> : <Catalog protocol={protocol} isAdmin={isAdmin} onNotice={setNotice} />}
      </div>
    </div>
  )
}
