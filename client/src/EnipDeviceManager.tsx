import { useState, useEffect, Fragment } from 'react'
import { useEnipDevices, useEnipDevice, useCreateEnipDevice, useUpdateEnipDevice, useDeleteEnipDevice, useStartEnipDevice, useStopEnipDevice, useAddEnipTag, useRemoveEnipTag } from './queries'
import { fetchJson } from './queries'
import { btnStyle, inputStyle } from './styles'
import type { EnipDevice, EnipTag } from './queries'
import { ENIP_DEFAULT_PORT, enipDeviceBody, splitHostPort } from './protocolPayloads'
import type { EnipDeviceForm as EnipDeviceValues, EnipTagForm } from './protocolPayloads'

const ENIP_DATA_TYPES = ['bool', 'sint8', 'int16', 'dint32', 'lint64', 'real32', 'string'] as const

/** What the device form submits; tags are only collected when creating (the update route takes none). */
interface EnipFormValues {
  device: EnipDeviceValues
  tags: EnipTagForm[]
}

/** A create form opened with the values of an existing device (Dup). */
interface EnipPrefill extends EnipDeviceValues {
  tags: EnipTagForm[]
}

export function EnipDeviceManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: devices = [], isLoading } = useEnipDevices()
  const createMut = useCreateEnipDevice()
  const updateMut = useUpdateEnipDevice()
  const deleteMut = useDeleteEnipDevice()
  const startMut = useStartEnipDevice()
  const stopMut = useStopEnipDevice()

  const [filter, setFilter] = useState('')
  const [showForm, setShowForm] = useState(false)
  const [prefill, setPrefill] = useState<EnipPrefill | undefined>(undefined)
  const [editDevice, setEditDevice] = useState<EnipDevice | null>(null)
  const [expandedId, setExpandedId] = useState<number | null>(null)

  const filtered = devices.filter(d =>
    d.name.toLowerCase().includes(filter.toLowerCase()) ||
    d.url.toLowerCase().includes(filter.toLowerCase()),
  )

  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const handleCreate = (v: EnipFormValues) => {
    createMut.mutate(
      { device: v.device, tags: v.tags },
      { onSuccess: () => { setShowForm(false); setPrefill(undefined) } },
    )
  }

  const handleUpdate = (v: EnipFormValues) => {
    if (!editDevice) return
    updateMut.mutate(
      { id: editDevice.id, ...enipDeviceBody(v.device) },
      { onSuccess: () => setEditDevice(null) },
    )
  }

  const handleDelete = (id: number) => {
    if (!confirm('Delete this EtherNet/IP device and stop its connection?')) return
    deleteMut.mutate(id)
  }

  const handleDup = async (d: EnipDevice) => {
    try {
      const tags = await fetchJson<EnipTag[]>(`/api/v1/enip/devices/${d.id}/tags`)
      const { host, port } = splitHostPort(d.url, ENIP_DEFAULT_PORT)
      setPrefill({
        name: `${d.name} copy`, host, port, slot: d.slot, assembly_instance: d.assembly_instance, enabled: d.enabled,
        tags: tags.map(t => ({
          name: t.name, tag_name: t.tag_name, data_type: t.data_type, display_name: t.display_name,
          assembly_offset: t.assembly_offset, assembly_length: t.assembly_length,
        })),
      })
      setEditDevice(null)
      setShowForm(true)
    } catch { /* ignore */ }
  }

  const thStyle: React.CSSProperties = {
    textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    borderBottom: '1px solid var(--border)',
  }
  const tdStyle: React.CSSProperties = {
    padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)',
  }

  const tableContent = (
    <>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <span style={{ color: 'var(--muted)', fontSize: 13 }}>{devices.length} device{devices.length !== 1 ? 's' : ''}</span>
        <div style={{ display: 'flex', gap: 8 }}>
          <input placeholder="Search..." value={filter} onChange={e => setFilter(e.target.value)} style={{ ...inputStyle, width: 200 }} />
          <button
            style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}
            onClick={() => { setShowForm(true); setEditDevice(null); setPrefill(undefined) }}
            aria-label="Add EtherNet/IP device"
          >
            + Add Device
          </button>
        </div>
      </div>

      {(showForm || editDevice) && (
        <EnipDeviceForm
          key={editDevice ? `edit-${editDevice.id}` : prefill ? 'dup' : 'new'}
          device={editDevice ?? undefined}
          prefill={prefill}
          onSubmit={editDevice ? handleUpdate : handleCreate}
          onClose={() => { setShowForm(false); setEditDevice(null); setPrefill(undefined) }}
          isPending={editDevice ? updateMut.isPending : createMut.isPending}
        />
      )}

      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        {isLoading ? (
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>Loading...</div>
        ) : filtered.length === 0 ? (
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>No EtherNet/IP devices found.</div>
        ) : (
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={thStyle}>Name</th>
                <th style={thStyle}>Address</th>
                <th style={thStyle}>Slot</th>
                <th style={thStyle}>Tags</th>
                <th style={thStyle}>Enabled</th>
                <th style={{ ...thStyle, width: 220 }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map(d => (
                <Fragment key={d.id}>
                  <tr>
                    <td style={tdStyle}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <button
                          onClick={() => setExpandedId(expandedId === d.id ? null : d.id)}
                          style={{ border: 'none', background: 'transparent', color: 'var(--muted)', cursor: 'pointer', fontSize: 11, padding: '2px 4px' }}
                          aria-label={expandedId === d.id ? 'Collapse tags' : 'Expand tags'}
                        >
                          {expandedId === d.id ? '\u25BC' : '\u25B6'}
                        </button>
                        <div style={{ width: 8, height: 8, borderRadius: '50%', background: d.enabled ? 'var(--ok)' : 'var(--crit)' }} />
                        {d.name}
                      </div>
                    </td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.url}</td>
                    <td style={tdStyle}>{d.slot}</td>
                    <td style={tdStyle}>{d.tag_count}</td>
                    <td style={tdStyle}>
                      <input
                        type="checkbox"
                        checked={d.enabled}
                        onChange={() => updateMut.mutate({ id: d.id, enabled: !d.enabled })}
                        style={{ cursor: 'pointer' }}
                      />
                    </td>
                    <td style={tdStyle}>
                      <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                        {d.enabled ? (
                          <button
                            onClick={() => stopMut.mutate(d.id)}
                            style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                            aria-label={`Stop ${d.name}`}
                          >
                            Stop
                          </button>
                        ) : (
                          <button
                            onClick={() => startMut.mutate(d.id)}
                            style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: 'pointer', fontSize: 11 }}
                            aria-label={`Start ${d.name}`}
                          >
                            Start
                          </button>
                        )}
                        <button
                          onClick={() => { setEditDevice(d); setShowForm(false); setPrefill(undefined) }}
                          aria-label={`Edit ${d.name}`}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 11 }}
                        >
                          Edit
                        </button>
                        <button
                          onClick={() => handleDup(d)}
                          aria-label={`Duplicate ${d.name}`}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 11 }}
                        >
                          Dup
                        </button>
                        <button
                          onClick={() => handleDelete(d.id)}
                          aria-label={`Delete ${d.name}`}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                        >
                          Del
                        </button>
                      </div>
                    </td>
                  </tr>
                  {expandedId === d.id && (
                    <tr>
                      <td colSpan={6} style={{ padding: 0, borderBottom: '1px solid var(--border)' }}>
                        <ExpandedEnipTags deviceId={d.id} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  )

  if (embedded) {
    return <>{tableContent}</>
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column', padding: 24 }}>
      <div style={{ maxWidth: 1200, width: '100%', margin: '0 auto', display: 'flex', flexDirection: 'column', height: '100%' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16, flexShrink: 0 }}>
          <button onClick={onClose} aria-label="Close EtherNet/IP devices" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
          </button>
          {/* EtherNet/IP Network Icon */}
          <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--cyan)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="5" r="2.5" fill="var(--cyan)" stroke="none" />
            <circle cx="5" cy="19" r="2.5" fill="none" />
            <circle cx="19" cy="19" r="2.5" fill="none" />
            <line x1="12" y1="7.5" x2="5" y2="16.5" />
            <line x1="12" y1="7.5" x2="19" y2="16.5" />
            <line x1="7.5" y1="19" x2="16.5" y2="19" />
          </svg>
          <h1 style={{ fontSize: 20, fontWeight: 700 }}>EtherNet/IP Devices</h1>
        </div>
        {tableContent}
      </div>
    </div>
  )
}

// ── Inline Tag Expansion ──

function ExpandedEnipTags({ deviceId }: { deviceId: number }) {
  const { data, isLoading } = useEnipDevice(deviceId)
  const addTagMut = useAddEnipTag()
  const removeTagMut = useRemoveEnipTag()

  const [newName, setNewName] = useState('')
  const [newSymbol, setNewSymbol] = useState('')
  const [newType, setNewType] = useState<string>('dint32')

  const tags = data ?? []
  const canAdd = !!newName.trim() && !!newSymbol.trim()

  const handleAdd = () => {
    if (!canAdd) return
    addTagMut.mutate(
      { deviceId, tag: { name: newName, tag_name: newSymbol, data_type: newType } },
      {
        onSuccess: () => {
          setNewName('')
          setNewSymbol('')
        },
      },
    )
  }

  const handleRemove = (tagId: number) => {
    if (!confirm('Remove this tag?')) return
    removeTagMut.mutate({ deviceId, tagId })
  }

  const TYPE_COLORS: Record<string, { bg: string; fg: string }> = {
    bool:   { bg: 'rgba(168,85,247,.12)', fg: '#c084fc' },
    sint8:  { bg: 'rgba(251,191,36,.12)', fg: '#fbbf24' },
    int16:  { bg: 'rgba(59,130,246,.12)', fg: '#60a5fa' },
    dint32: { bg: 'rgba(0,200,200,.08)', fg: 'var(--cyan)' },
    lint64: { bg: 'rgba(34,197,94,.12)', fg: '#4ade80' },
    real32: { bg: 'rgba(239,68,68,.12)', fg: '#f87171' },
    string: { bg: 'rgba(139,92,246,.12)', fg: '#a78bfa' },
  }

  const nodeThStyle: React.CSSProperties = {
    textAlign: 'left', padding: '6px 12px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    background: 'rgba(255,255,255,.02)',
  }
  const nodeTdStyle: React.CSSProperties = { padding: '6px 12px', fontSize: 12, borderBottom: '1px solid var(--border)' }

  return (
    <div style={{ maxHeight: 300, overflow: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse' }}>
        <thead>
          <tr>
            <th style={nodeThStyle}>Name</th>
            <th style={nodeThStyle}>Tag (CIP symbol)</th>
            <th style={nodeThStyle}>Data Type</th>
            <th style={{ ...nodeThStyle, width: 80 }}>Action</th>
          </tr>
        </thead>
        <tbody>
          {isLoading ? (
            <tr><td colSpan={4} style={{ ...nodeTdStyle, color: 'var(--muted)' }}>Loading tags...</td></tr>
          ) : tags.length === 0 ? (
            <tr><td colSpan={4} style={{ ...nodeTdStyle, color: 'var(--muted)' }}>No tags configured. Add tags below.</td></tr>
          ) : (
            tags.map(t => {
              const typeColor = TYPE_COLORS[t.data_type] ?? TYPE_COLORS.dint32
              return (
                <tr key={t.id}>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{t.name}</td>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{t.tag_name}</td>
                  <td style={nodeTdStyle}>
                    <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: typeColor.bg, color: typeColor.fg }}>
                      {t.data_type}
                    </span>
                  </td>
                  <td style={nodeTdStyle}>
                    <button
                      onClick={() => handleRemove(t.id)}
                      style={{ padding: '3px 6px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                      aria-label={`Remove tag ${t.name}`}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              )
            })
          )}
        </tbody>
        <tfoot>
          <tr style={{ borderTop: '1px solid var(--border)' }}>
            <td style={{ padding: '6px 12px' }}>
              <input
                value={newName}
                onChange={e => setNewName(e.target.value)}
                placeholder="tag_name"
                style={{ ...inputStyle, fontSize: 11, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <input
                value={newSymbol}
                onChange={e => setNewSymbol(e.target.value)}
                placeholder="Program:Tag.Name"
                style={{ ...inputStyle, fontSize: 11, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <select value={newType} onChange={e => setNewType(e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>
                {ENIP_DATA_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
              </select>
            </td>
            <td style={{ padding: '6px 12px' }}>
              <button
                onClick={handleAdd}
                disabled={!canAdd || addTagMut.isPending}
                style={{ padding: '4px 10px', borderRadius: 4, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: canAdd ? 'pointer' : 'not-allowed', fontSize: 11, opacity: canAdd ? 1 : 0.5 }}
              >
                {addTagMut.isPending ? '...' : '+ Add'}
              </button>
            </td>
          </tr>
        </tfoot>
      </table>
      {addTagMut.isError && (
        <div style={{ padding: '6px 12px', fontSize: 11, color: 'var(--crit)' }}>{addTagMut.error?.message || 'Failed to add the tag'}</div>
      )}
    </div>
  )
}

// ── Inline Create / Edit Form ──

function EnipDeviceForm({
  device,
  prefill,
  onSubmit,
  onClose,
  isPending,
}: {
  device?: EnipDevice
  prefill?: EnipPrefill
  onSubmit: (v: EnipFormValues) => void
  onClose: () => void
  isPending: boolean
}) {
  const isEdit = !!device
  const initial = device
    ? { ...splitHostPort(device.url, ENIP_DEFAULT_PORT), name: device.name, slot: device.slot, assembly_instance: device.assembly_instance, enabled: device.enabled }
    : prefill
  const [name, setName] = useState(initial?.name ?? '')
  const [host, setHost] = useState(initial?.host ?? '')
  const [port, setPort] = useState(initial?.port ?? ENIP_DEFAULT_PORT)
  const [slot, setSlot] = useState(initial?.slot ?? 0)
  const [assemblyInstance, setAssemblyInstance] = useState(initial?.assembly_instance ?? 0)
  const [enabled, setEnabled] = useState(initial?.enabled ?? true)
  const [tags, setTags] = useState<EnipTagForm[]>(prefill?.tags ?? [])

  const addTag = () => setTags([...tags, { name: '', tag_name: '', data_type: 'dint32', assembly_offset: 0, assembly_length: 0 }])
  const removeTag = (i: number) => setTags(tags.filter((_, j) => j !== i))
  const updateTag = (i: number, field: keyof EnipTagForm, value: string | number) => {
    const next = [...tags]
    next[i] = { ...next[i], [field]: value }
    setTags(next)
  }

  const handleSubmit = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    onSubmit({
      device: { name, host, port, slot, assembly_instance: assemblyInstance, enabled },
      tags: tags.filter(t => t.name.trim() && t.tag_name.trim()),
    })
  }

  return (
    <form
      onSubmit={handleSubmit}
      style={{
        marginBottom: 16, padding: 16, background: 'var(--card)', border: '1px solid var(--border)',
        borderRadius: 10, display: 'flex', flexDirection: 'column', gap: 12,
      }}
    >
      <div style={{ fontSize: 13, fontWeight: 600, paddingBottom: 4, borderBottom: '1px solid var(--border)' }}>
        {isEdit ? `Edit: ${device!.name}` : 'Add EtherNet/IP Device'}
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Name</label>
          <input value={name} onChange={e => setName(e.target.value)} placeholder="ControlLogix L85E" style={inputStyle} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>IP Address *</label>
          <input value={host} onChange={e => setHost(e.target.value)} placeholder="192.168.1.10" required style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr auto', gap: 8, alignItems: 'end' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Port</label>
          <input type="number" value={port} onChange={e => setPort(parseInt(e.target.value) || ENIP_DEFAULT_PORT)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Slot</label>
          <input type="number" value={slot} onChange={e => setSlot(parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Assembly (0=poll)</label>
          <input type="number" value={assemblyInstance} onChange={e => setAssemblyInstance(parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Enabled</label>
          <div style={{ display: 'flex', alignItems: 'center', height: 32 }}>
            <input type="checkbox" checked={enabled} onChange={e => setEnabled(e.target.checked)} style={{ cursor: 'pointer' }} />
          </div>
        </div>
      </div>

      {/* Tags are added with the device; an existing device's tags are edited in its expanded row. */}
      {!isEdit && (
        <div style={{ borderTop: '1px solid var(--border)', paddingTop: 8 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
            <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)' }}>Tags</span>
            <button type="button" onClick={addTag} style={{ padding: '3px 10px', borderRadius: 4, fontSize: 11, cursor: 'pointer', border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--cyan)' }}>
              + Add Tag
            </button>
          </div>
          {tags.length === 0 && <div style={{ color: 'var(--muted)', fontSize: 12, padding: 8 }}>No tags configured. You can also add tags after creation.</div>}
          {tags.length > 0 && (
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 100px 64px 64px 28px', gap: 4, marginBottom: 2, fontSize: 10, color: 'var(--muted)', paddingLeft: 2 }}>
              <span>name</span><span>CIP symbol</span><span>type</span><span>off</span><span>len</span><span></span>
            </div>
          )}
          {tags.map((tag, i) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 100px 64px 64px 28px', gap: 4, marginBottom: 4, alignItems: 'end' }}>
              <input value={tag.name} onChange={e => updateTag(i, 'name', e.target.value)} placeholder="temperature" style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
              <input value={tag.tag_name} onChange={e => updateTag(i, 'tag_name', e.target.value)} placeholder="Program:Tag.Name" style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
              <select value={tag.data_type} onChange={e => updateTag(i, 'data_type', e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>
                {ENIP_DATA_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
              </select>
              <input type="number" value={tag.assembly_offset ?? 0} onChange={e => updateTag(i, 'assembly_offset', parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
              <input type="number" value={tag.assembly_length ?? 0} onChange={e => updateTag(i, 'assembly_length', parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
              <button type="button" onClick={() => removeTag(i)} style={{ background: 'none', border: 'none', color: 'var(--crit)', cursor: 'pointer', fontSize: 14 }}>&times;</button>
            </div>
          ))}
        </div>
      )}

      <div style={{ display: 'flex', gap: 8 }}>
        <button type="submit" style={{ padding: '6px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 13 }} disabled={isPending}>
          {isPending ? 'Saving...' : (isEdit ? 'Update' : 'Create')}
        </button>
        <button type="button" onClick={onClose} style={{ padding: '6px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--muted)', cursor: 'pointer', fontSize: 13 }}>
          Cancel
        </button>
      </div>
    </form>
  )
}
