import { useState, useEffect, Fragment } from 'react'
import { useEnipDevices, useEnipDevice, useCreateEnipDevice, useUpdateEnipDevice, useDeleteEnipDevice, useStartEnipDevice, useStopEnipDevice, useAddEnipTag, useRemoveEnipTag } from './queries'
import { fetchJson } from './queries'
import { btnStyle, inputStyle } from './styles'
import type { EnipDevice, EnipTag } from './queries'

const ENIP_DATA_TYPES = ['bool', 'sint8', 'int16', 'dint32', 'lint64', 'real32', 'string'] as const

export function EnipDeviceManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: devices = [], isLoading } = useEnipDevices()
  const createMut = useCreateEnipDevice()
  const updateMut = useUpdateEnipDevice()
  const deleteMut = useDeleteEnipDevice()
  const startMut = useStartEnipDevice()
  const stopMut = useStopEnipDevice()

  const [filter, setFilter] = useState('')
  const [showForm, setShowForm] = useState(false)
  const [editDevice, setEditDevice] = useState<EnipDevice | null>(null)
  const [expandedId, setExpandedId] = useState<string | null>(null)

  const filtered = devices.filter(d =>
    (d.name || d.id).toLowerCase().includes(filter.toLowerCase()) ||
    d.ip.toLowerCase().includes(filter.toLowerCase()),
  )

  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const handleCreate = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    const fd = new FormData(e.currentTarget)
    const tags = collectFormTags(fd)
    createMut.mutate(
      {
        id: fd.get('id') as string,
        name: (fd.get('name') as string) || (fd.get('id') as string),
        ip: fd.get('ip') as string,
        port: Number(fd.get('port')) || 44818,
        slot: Number(fd.get('slot')) || 0,
        assembly_instance: Number(fd.get('assembly_instance')) || 0,
        enabled: fd.get('enabled') === 'on',
        tags,
      },
      { onSuccess: () => setShowForm(false) },
    )
  }

  const handleUpdate = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    if (!editDevice) return
    const fd = new FormData(e.currentTarget)
    const tags = collectFormTags(fd)
    updateMut.mutate(
      {
        id: editDevice.id,
        name: fd.get('name') as string,
        ip: fd.get('ip') as string,
        port: Number(fd.get('port')) || 44818,
        slot: Number(fd.get('slot')) || 0,
        assembly_instance: Number(fd.get('assembly_instance')) || 0,
        enabled: fd.get('enabled') === 'on',
        tags,
      },
      { onSuccess: () => setEditDevice(null) },
    )
  }

  const handleDelete = (id: string) => {
    if (!confirm('Delete this EtherNet/IP device and stop its connection?')) return
    deleteMut.mutate(id)
  }

  const handleDup = async (d: EnipDevice) => {
    try {
      const tags = await fetchJson<EnipTag[]>(`/api/v1/enip/devices/${d.id}/tags`)
      setEditDevice({ ...d, tags } as EnipDevice & { tags: EnipTag[] })
      setShowForm(false)
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
            onClick={() => { setShowForm(true); setEditDevice(null) }}
            aria-label="Add EtherNet/IP device"
          >
            + Add Device
          </button>
        </div>
      </div>

      {(showForm || editDevice) && (
        <EnipDeviceForm
          device={editDevice ? { ...editDevice } as any : undefined}
          onSubmit={editDevice ? handleUpdate : handleCreate}
          onClose={() => { setShowForm(false); setEditDevice(null) }}
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
                <th style={thStyle}>IP</th>
                <th style={thStyle}>Port</th>
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
                        {d.name || d.id}
                      </div>
                    </td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.ip}</td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.port}</td>
                    <td style={tdStyle}>{d.slot}</td>
                    <td style={tdStyle}>{d.tag_count ?? 0}</td>
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
                            aria-label={`Stop ${d.name || d.id}`}
                          >
                            Stop
                          </button>
                        ) : (
                          <button
                            onClick={() => startMut.mutate(d.id)}
                            style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: 'pointer', fontSize: 11 }}
                            aria-label={`Start ${d.name || d.id}`}
                          >
                            Start
                          </button>
                        )}
                        <button
                          onClick={() => { setEditDevice(d); setShowForm(false) }}
                          aria-label={`Edit ${d.name || d.id}`}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 11 }}
                        >
                          Edit
                        </button>
                        <button
                          onClick={() => handleDup(d)}
                          aria-label={`Duplicate ${d.name || d.id}`}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 11 }}
                        >
                          Dup
                        </button>
                        <button
                          onClick={() => handleDelete(d.id)}
                          aria-label={`Delete ${d.name || d.id}`}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                        >
                          Del
                        </button>
                      </div>
                    </td>
                  </tr>
                  {expandedId === d.id && (
                    <tr>
                      <td colSpan={7} style={{ padding: 0, borderBottom: '1px solid var(--border)' }}>
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

function ExpandedEnipTags({ deviceId }: { deviceId: string }) {
  const { data, isLoading } = useEnipDevice(deviceId)
  const addTagMut = useAddEnipTag()
  const removeTagMut = useRemoveEnipTag()

  const [newName, setNewName] = useState('')
  const [newType, setNewType] = useState<string>('dint32')

  const tags = data ?? []

  const handleAdd = () => {
    if (!newName.trim()) return
    addTagMut.mutate(
      { deviceId, tag: { name: newName.trim(), data_type: newType } },
      {
        onSuccess: () => {
          setNewName('')
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
            <th style={nodeThStyle}>Data Type</th>
            <th style={{ ...nodeThStyle, width: 80 }}>Action</th>
          </tr>
        </thead>
        <tbody>
          {isLoading ? (
            <tr><td colSpan={3} style={{ ...nodeTdStyle, color: 'var(--muted)' }}>Loading tags...</td></tr>
          ) : tags.length === 0 ? (
            <tr><td colSpan={3} style={{ ...nodeTdStyle, color: 'var(--muted)' }}>No tags configured. Add tags below.</td></tr>
          ) : (
            tags.map(t => {
              const typeColor = TYPE_COLORS[t.data_type] ?? TYPE_COLORS.dint32
              return (
                <tr key={t.id}>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{t.name}</td>
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
                disabled={!newName.trim() || addTagMut.isPending}
                style={{ padding: '4px 10px', borderRadius: 4, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: newName.trim() ? 'pointer' : 'not-allowed', fontSize: 11, opacity: newName.trim() ? 1 : 0.5 }}
              >
                {addTagMut.isPending ? '...' : '+ Add'}
              </button>
            </td>
          </tr>
        </tfoot>
      </table>
    </div>
  )
}

// ── Inline Create / Edit Form ──

interface TagFormRow {
  name: string
  data_type: string
  assembly_offset: number
  assembly_length: number
}

function EnipDeviceForm({
  device,
  onSubmit,
  onClose,
  isPending,
}: {
  device?: EnipDevice & { tags?: EnipTag[] }
  onSubmit: (e: React.FormEvent<HTMLFormElement>) => void
  onClose: () => void
  isPending: boolean
}) {
  const isEdit = !!device
  const src = device
  const [name, setName] = useState(src?.name ?? '')
  const [ip, setIp] = useState(src?.ip ?? '')
  const [port, setPort] = useState(src?.port ?? 44818)
  const [slot, setSlot] = useState(src?.slot ?? 0)
  const [assemblyInstance, setAssemblyInstance] = useState(src?.assembly_instance ?? 0)
  const [enabled, setEnabled] = useState(src?.enabled ?? true)
  const [tags, setTags] = useState<TagFormRow[]>(
    (src as any)?.tags?.map((t: any) => ({
      name: t.name,
      data_type: t.data_type,
      assembly_offset: t.assembly_offset ?? 0,
      assembly_length: t.assembly_length ?? 0,
    })) ?? [],
  )
  const [deviceId, setDeviceId] = useState(src?.id ?? '')

  const addTag = () => setTags([...tags, { name: '', data_type: 'dint32', assembly_offset: 0, assembly_length: 0 }])
  const removeTag = (i: number) => setTags(tags.filter((_, j) => j !== i))
  const updateTag = (i: number, field: keyof TagFormRow, value: any) => {
    const next = [...tags]
    next[i] = { ...next[i], [field]: value }
    setTags(next)
  }

  return (
    <form
      onSubmit={onSubmit}
      style={{
        marginBottom: 16, padding: 16, background: 'var(--card)', border: '1px solid var(--border)',
        borderRadius: 10, display: 'flex', flexDirection: 'column', gap: 12,
      }}
    >
      <div style={{ fontSize: 13, fontWeight: 600, paddingBottom: 4, borderBottom: '1px solid var(--border)' }}>
        {isEdit ? `Edit: ${device!.name || device!.id}` : 'Add EtherNet/IP Device'}
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: isEdit ? '1fr 1fr' : '1fr 1fr 1fr', gap: 8 }}>
        {!isEdit && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Device ID *</label>
            <input name="id" value={deviceId} onChange={e => setDeviceId(e.target.value)} placeholder="plc_enip_1" required style={inputStyle} />
          </div>
        )}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Name</label>
          <input name="name" value={name} onChange={e => setName(e.target.value)} placeholder="ControlLogix L85E" style={inputStyle} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>IP Address *</label>
          <input name="ip" value={ip} onChange={e => setIp(e.target.value)} placeholder="192.168.1.10" required style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr auto', gap: 8, alignItems: 'end' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Port</label>
          <input name="port" type="number" value={port} onChange={e => setPort(parseInt(e.target.value) || 44818)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Slot</label>
          <input name="slot" type="number" value={slot} onChange={e => setSlot(parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Assembly (0=poll)</label>
          <input name="assembly_instance" type="number" value={assemblyInstance} onChange={e => setAssemblyInstance(parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Enabled</label>
          <div style={{ display: 'flex', alignItems: 'center', height: 32 }}>
            <input name="enabled" type="checkbox" checked={enabled} onChange={e => setEnabled(e.target.checked)} style={{ cursor: 'pointer' }} />
          </div>
        </div>
      </div>

      {/* Tags section */}
      <div style={{ borderTop: '1px solid var(--border)', paddingTop: 8 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
          <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)' }}>Tags</span>
          <button type="button" onClick={addTag} style={{ padding: '3px 10px', borderRadius: 4, fontSize: 11, cursor: 'pointer', border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--cyan)' }}>
            + Add Tag
          </button>
        </div>
        {tags.length === 0 && <div style={{ color: 'var(--muted)', fontSize: 12, padding: 8 }}>No tags configured. You can also add tags after creation.</div>}
        {tags.length > 0 && (
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 100px 64px 64px 28px', gap: 4, marginBottom: 2, fontSize: 10, color: 'var(--muted)', paddingLeft: 2 }}>
            <span>name</span><span>type</span><span>off</span><span>len</span><span></span>
          </div>
        )}
        {tags.map((tag, i) => (
          <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 100px 64px 64px 28px', gap: 4, marginBottom: 4, alignItems: 'end' }}>
            <input value={tag.name} onChange={e => updateTag(i, 'name', e.target.value)} placeholder="Program:Tag.Name" style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
            <select value={tag.data_type} onChange={e => updateTag(i, 'data_type', e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>
              {ENIP_DATA_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
            </select>
            <input type="number" value={tag.assembly_offset} onChange={e => updateTag(i, 'assembly_offset', parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
            <input type="number" value={tag.assembly_length} onChange={e => updateTag(i, 'assembly_length', parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
            <button type="button" onClick={() => removeTag(i)} style={{ background: 'none', border: 'none', color: 'var(--crit)', cursor: 'pointer', fontSize: 14 }}>&times;</button>
          </div>
        ))}
      </div>

      {/* Hidden fields for tag data */}
      {tags.map((t, i) => (
        <span key={`hidden-${i}`} style={{ display: 'none' }}>
          <input type="hidden" name={`tag_${i}_name`} value={t.name} />
          <input type="hidden" name={`tag_${i}_data_type`} value={t.data_type} />
          <input type="hidden" name={`tag_${i}_assembly_offset`} value={t.assembly_offset} />
          <input type="hidden" name={`tag_${i}_assembly_length`} value={t.assembly_length} />
        </span>
      ))}

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

// ── Helper to collect tags from FormData ──

function collectFormTags(fd: FormData): Partial<EnipTag>[] {
  const tags: Partial<EnipTag>[] = []
  let i = 0
  while (true) {
    const name = fd.get(`tag_${i}_name`) as string | null
    if (!name) break
    tags.push({
      name,
      data_type: (fd.get(`tag_${i}_data_type`) as string) || 'dint32',
      assembly_offset: Number(fd.get(`tag_${i}_assembly_offset`)) || 0,
      assembly_length: Number(fd.get(`tag_${i}_assembly_length`)) || 0,
    })
    i++
  }
  return tags
}
