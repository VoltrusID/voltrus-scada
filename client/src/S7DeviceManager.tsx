import { useState, useEffect, Fragment } from 'react'
import { useS7Devices, useS7Device, useCreateS7Device, useUpdateS7Device, useDeleteS7Device, useStartS7Device, useStopS7Device, useAddS7Tag, useRemoveS7Tag } from './queries'
import { fetchJson } from './queries'
import { btnStyle, inputStyle } from './styles'
import type { S7Device, S7Tag } from './queries'
import { S7_DEFAULT_PORT, s7DeviceBody, splitHostPort } from './protocolPayloads'
import type { S7DeviceForm as S7DeviceValues, S7TagForm } from './protocolPayloads'

const S7_DATA_TYPES = ['bool', 'int16', 'uint16', 'int32', 'uint32', 'float32', 'float64', 'string'] as const

/** What the device form submits; tags are only collected when creating (the update route takes none). */
interface S7FormValues {
  device: S7DeviceValues
  tags: S7TagForm[]
}

/** A create form opened with the values of an existing device (Dup). */
interface S7Prefill extends S7DeviceValues {
  tags: S7TagForm[]
}

export function S7DeviceManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: devices = [], isLoading } = useS7Devices()
  const createMut = useCreateS7Device()
  const updateMut = useUpdateS7Device()
  const deleteMut = useDeleteS7Device()
  const startMut = useStartS7Device()
  const stopMut = useStopS7Device()

  const [filter, setFilter] = useState('')
  const [showForm, setShowForm] = useState(false)
  const [prefill, setPrefill] = useState<S7Prefill | undefined>(undefined)
  const [editDevice, setEditDevice] = useState<S7Device | null>(null)
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

  const handleCreate = (v: S7FormValues) => {
    createMut.mutate(
      { device: v.device, tags: v.tags },
      { onSuccess: () => { setShowForm(false); setPrefill(undefined) } },
    )
  }

  const handleUpdate = (v: S7FormValues) => {
    if (!editDevice) return
    updateMut.mutate(
      { id: editDevice.id, ...s7DeviceBody(v.device) },
      { onSuccess: () => setEditDevice(null) },
    )
  }

  const handleDelete = (id: number) => {
    if (!confirm('Delete this S7 device and stop its connection?')) return
    deleteMut.mutate(id)
  }

  const handleDup = async (d: S7Device) => {
    try {
      const tags = await fetchJson<S7Tag[]>(`/api/v1/s7/devices/${d.id}/tags`)
      const { host, port } = splitHostPort(d.url, S7_DEFAULT_PORT)
      setPrefill({
        name: `${d.name} copy`, host, port, rack: d.rack, slot: d.slot, enabled: d.enabled,
        tags: tags.map(t => ({ name: t.name, tag_address: t.tag_address, data_type: t.data_type, display_name: t.display_name })),
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
            aria-label="Add S7 device"
          >
            + Add Device
          </button>
        </div>
      </div>

      {(showForm || editDevice) && (
        <S7DeviceForm
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
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>No S7 devices found.</div>
        ) : (
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={thStyle}>Name</th>
                <th style={thStyle}>Address</th>
                <th style={thStyle}>Rack</th>
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
                    <td style={tdStyle}>{d.rack}</td>
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
                      <td colSpan={7} style={{ padding: 0, borderBottom: '1px solid var(--border)' }}>
                        <ExpandedS7Tags deviceId={d.id} />
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
          <button onClick={onClose} aria-label="Close S7 devices" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
          </button>
          {/* S7 Industrial Protocol Icon */}
          <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--cyan)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <rect x="2" y="6" width="20" height="12" rx="2" />
            <circle cx="6" cy="12" r="1.5" fill="var(--cyan)" stroke="none" />
            <line x1="10" y1="9" x2="10" y2="15" />
            <line x1="13" y1="9" x2="13" y2="15" />
            <line x1="16" y1="9" x2="16" y2="15" />
            <circle cx="19" cy="12" r="1.5" fill="var(--cyan)" stroke="none" />
          </svg>
          <h1 style={{ fontSize: 20, fontWeight: 700 }}>Siemens S7 Devices</h1>
        </div>
        {tableContent}
      </div>
    </div>
  )
}

// ── Inline Tag Expansion ──

function ExpandedS7Tags({ deviceId }: { deviceId: number }) {
  const { data, isLoading } = useS7Device(deviceId)
  const addTagMut = useAddS7Tag()
  const removeTagMut = useRemoveS7Tag()

  const [newName, setNewName] = useState('')
  const [newAddress, setNewAddress] = useState('')
  const [newType, setNewType] = useState<string>('float32')

  const tags = data ?? []
  const canAdd = !!newName.trim() && !!newAddress.trim()

  const handleAdd = () => {
    if (!canAdd) return
    addTagMut.mutate(
      { deviceId, tag: { name: newName, tag_address: newAddress, data_type: newType } },
      {
        onSuccess: () => {
          setNewName('')
          setNewAddress('')
        },
      },
    )
  }

  const handleRemove = (tagId: number) => {
    if (!confirm('Remove this tag?')) return
    removeTagMut.mutate({ deviceId, tagId })
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
            <th style={nodeThStyle}>Address</th>
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
            tags.map(t => (
              <tr key={t.id}>
                <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{t.name}</td>
                <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{t.tag_address}</td>
                <td style={nodeTdStyle}>
                  <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: 'rgba(0,200,200,.08)', color: 'var(--cyan)' }}>
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
            ))
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
                value={newAddress}
                onChange={e => setNewAddress(e.target.value)}
                placeholder="DB1.DBD0"
                style={{ ...inputStyle, fontSize: 11, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <select value={newType} onChange={e => setNewType(e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>
                {S7_DATA_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
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

function S7DeviceForm({
  device,
  prefill,
  onSubmit,
  onClose,
  isPending,
}: {
  device?: S7Device
  prefill?: S7Prefill
  onSubmit: (v: S7FormValues) => void
  onClose: () => void
  isPending: boolean
}) {
  const isEdit = !!device
  const initial = device ? { ...splitHostPort(device.url, S7_DEFAULT_PORT), name: device.name, rack: device.rack, slot: device.slot, enabled: device.enabled } : prefill
  const [name, setName] = useState(initial?.name ?? '')
  const [host, setHost] = useState(initial?.host ?? '')
  const [port, setPort] = useState(initial?.port ?? S7_DEFAULT_PORT)
  const [rack, setRack] = useState(initial?.rack ?? 0)
  const [slot, setSlot] = useState(initial?.slot ?? 1)
  const [enabled, setEnabled] = useState(initial?.enabled ?? true)
  const [tags, setTags] = useState<S7TagForm[]>(prefill?.tags ?? [])

  const addTag = () => setTags([...tags, { name: '', tag_address: '', data_type: 'float32' }])
  const removeTag = (i: number) => setTags(tags.filter((_, j) => j !== i))
  const updateTag = (i: number, field: keyof S7TagForm, value: string) => {
    const next = [...tags]
    next[i] = { ...next[i], [field]: value }
    setTags(next)
  }

  const handleSubmit = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    onSubmit({
      device: { name, host, port, rack, slot, enabled },
      tags: tags.filter(t => t.name.trim() && t.tag_address.trim()),
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
        {isEdit ? `Edit: ${device!.name}` : 'Add S7 Device'}
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Name</label>
          <input value={name} onChange={e => setName(e.target.value)} placeholder="S7-1200 CPU" style={inputStyle} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>IP Address *</label>
          <input value={host} onChange={e => setHost(e.target.value)} placeholder="192.168.0.1" required style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr auto', gap: 8, alignItems: 'end' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Port</label>
          <input type="number" value={port} onChange={e => setPort(parseInt(e.target.value) || S7_DEFAULT_PORT)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Rack</label>
          <input type="number" value={rack} onChange={e => setRack(parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Slot</label>
          <input type="number" value={slot} onChange={e => setSlot(parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
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
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 90px 28px', gap: 4, marginBottom: 2, fontSize: 10, color: 'var(--muted)', paddingLeft: 2 }}>
              <span>name</span><span>address</span><span>type</span><span></span>
            </div>
          )}
          {tags.map((tag, i) => (
            <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 90px 28px', gap: 4, marginBottom: 4, alignItems: 'end' }}>
              <input value={tag.name} onChange={e => updateTag(i, 'name', e.target.value)} placeholder="temperature" style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
              <input value={tag.tag_address} onChange={e => updateTag(i, 'tag_address', e.target.value)} placeholder="DB1.DBD0" style={{ ...inputStyle, fontSize: 12, fontFamily: "var(--font-mono)" }} />
              <select value={tag.data_type} onChange={e => updateTag(i, 'data_type', e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>
                {S7_DATA_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
              </select>
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
