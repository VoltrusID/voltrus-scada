import { useState, useEffect, Fragment } from 'react'
import { useBacnetDevices, useBacnetDevice, useCreateBacnetDevice, useUpdateBacnetDevice, useDeleteBacnetDevice, useAddBacnetObject, useRemoveBacnetObject, useDiscoverBacnet } from './queries'
import { btnStyle, inputStyle } from './styles'
import type { BacnetDevice, BacnetObject } from './queries'
import { FileDropZone, ImportFileButton, ImportNotice } from './components/FileDropZone'
import { deviceFileKind, parseBacnetDeviceFile } from './deviceFileImport'

const BACNET_OBJECT_TYPES = [
  'analogInput', 'analogOutput', 'analogValue',
  'binaryInput', 'binaryOutput', 'binaryValue',
  'multiStateInput', 'multiStateOutput', 'multiStateValue',
] as const

export function BacnetDeviceManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: devices = [], isLoading } = useBacnetDevices()
  const createMut = useCreateBacnetDevice()
  const updateMut = useUpdateBacnetDevice()
  const deleteMut = useDeleteBacnetDevice()
  const addObjectMut = useAddBacnetObject()
  const [importLines, setImportLines] = useState<string[]>([])

  const [filter, setFilter] = useState('')
  const [showForm, setShowForm] = useState(false)
  const [editDevice, setEditDevice] = useState<BacnetDevice | null>(null)
  const [expandedId, setExpandedId] = useState<number | null>(null)
  const [showDiscover, setShowDiscover] = useState(false)

  const filtered = devices.filter(d =>
    (d.name || String(d.id)).toLowerCase().includes(filter.toLowerCase()) ||
    d.ip.toLowerCase().includes(filter.toLowerCase()),
  )

  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const handleCreate = (device: Partial<BacnetDevice>) => {
    createMut.mutate(device, { onSuccess: () => setShowForm(false) })
  }

  const handleUpdate = (id: number, fields: Record<string, any>) => {
    updateMut.mutate({ id, ...fields }, { onSuccess: () => setEditDevice(null) })
  }

  // Device files from BACnet Explorer's "Export to SCADA" (Voltrus SCADA device), by drop or file picker.
  const importFiles = async (files: File[]) => {
    const lines: string[] = []
    for (const file of files) {
      const text = await file.text()
      if (deviceFileKind(text) === 'modbus') { lines.push(`${file.name}: this is a Modbus device file, import it on the Modbus devices page`); continue }
      const { devices: parsed, errors } = parseBacnetDeviceFile(text)
      errors.forEach(e => lines.push(`${file.name}: ${e}`))
      for (const d of parsed) {
        try {
          const { objects, ...device } = d
          const created = await createMut.mutateAsync(device)
          let added = 0
          for (const object of objects) {
            try {
              await addObjectMut.mutateAsync({ deviceId: created.id, object })
              added++
            } catch (e) {
              lines.push(`${file.name}: ${d.name}: ${object.name} was not added: ${e instanceof Error ? e.message : String(e)}`)
            }
          }
          lines.push(`${file.name}: added ${d.name} with ${added} of ${objects.length} objects`)
        } catch (e) {
          lines.push(`${file.name}: ${d.name} was not added: ${e instanceof Error ? e.message : String(e)}`)
        }
      }
    }
    setImportLines(lines)
  }

  const handleDelete = (id: number) => {
    if (!confirm('Delete this BACnet device?')) return
    deleteMut.mutate(id)
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
    <FileDropZone onFiles={importFiles} style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <span style={{ color: 'var(--muted)', fontSize: 13 }}>{devices.length} device{devices.length !== 1 ? 's' : ''}</span>
        <div style={{ display: 'flex', gap: 8 }}>
          <input placeholder="Search..." value={filter} onChange={e => setFilter(e.target.value)} style={{ ...inputStyle, width: 200 }} />
          <ImportFileButton onFiles={importFiles} ariaLabel="Import BACnet device file" />
          <button
            style={{ ...btnStyle, borderColor: 'var(--muted)', color: 'var(--muted)' }}
            onClick={() => setShowDiscover(s => !s)}
            aria-label="Network discovery"
          >
            Discover
          </button>
          <button
            style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}
            onClick={() => { setShowForm(true); setEditDevice(null) }}
            aria-label="Add BACnet device"
          >
            + Add Device
          </button>
        </div>
      </div>

      <ImportNotice lines={importLines} onDismiss={() => setImportLines([])} />

      {showDiscover && (
        <BacnetDiscoverPanel onClose={() => setShowDiscover(false)} />
      )}

      {(showForm || editDevice) && (
        <BacnetDeviceForm
          device={editDevice ?? undefined}
          onSubmit={editDevice ? (fields) => handleUpdate(editDevice.id, fields) : handleCreate}
          onClose={() => { setShowForm(false); setEditDevice(null) }}
          isPending={editDevice ? updateMut.isPending : createMut.isPending}
        />
      )}

      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        {isLoading ? (
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>Loading...</div>
        ) : filtered.length === 0 ? (
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>No BACnet devices found.</div>
        ) : (
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={thStyle}>Name</th>
                <th style={thStyle}>IP</th>
                <th style={thStyle}>Port</th>
                <th style={thStyle}>Instance</th>
                <th style={thStyle}>Interval</th>
                <th style={thStyle}>COV</th>
                <th style={thStyle}>Objects</th>
                <th style={thStyle}>Enabled</th>
                <th style={{ ...thStyle, width: 180 }}>Actions</th>
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
                          aria-label={expandedId === d.id ? 'Collapse objects' : 'Expand objects'}
                        >
                          {expandedId === d.id ? '\u25BC' : '\u25B6'}
                        </button>
                        <div style={{ width: 8, height: 8, borderRadius: '50%', background: d.enabled ? 'var(--ok)' : 'var(--crit)' }} />
                        {d.name || d.id}
                      </div>
                    </td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.ip}</td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.port}</td>
                    <td style={tdStyle}>{d.device_instance}</td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.interval_ms}ms</td>
                    <td style={tdStyle}>
                      {d.cov_enabled ? (
                        <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: 'rgba(34,197,94,.12)', color: '#4ade80' }}>On</span>
                      ) : (
                        <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: 'rgba(239,68,68,.12)', color: '#f87171' }}>Off</span>
                      )}
                    </td>
                    <td style={tdStyle}>{d.object_count ?? 0}</td>
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
                        <button
                          onClick={() => { setEditDevice(d); setShowForm(false) }}
                          aria-label={`Edit ${d.name || d.id}`}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 11 }}
                        >
                          Edit
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
                      <td colSpan={9} style={{ padding: 0, borderBottom: '1px solid var(--border)' }}>
                        <ExpandedBacnetObjects deviceId={d.id} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </FileDropZone>
  )

  if (embedded) {
    return <>{tableContent}</>
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column', padding: 24 }}>
      <div style={{ maxWidth: 1200, width: '100%', margin: '0 auto', display: 'flex', flexDirection: 'column', height: '100%' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16, flexShrink: 0 }}>
          <button onClick={onClose} aria-label="Close BACnet devices" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
          </button>
          {/* BACnet Building Automation Icon */}
          <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--cyan)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <rect x="3" y="3" width="7" height="7" rx="1" />
            <rect x="14" y="3" width="7" height="7" rx="1" />
            <rect x="3" y="14" width="7" height="7" rx="1" />
            <rect x="14" y="14" width="7" height="7" rx="1" />
            <circle cx="6.5" cy="6.5" r="1" fill="var(--cyan)" stroke="none" />
            <circle cx="17.5" cy="6.5" r="1" fill="var(--cyan)" stroke="none" />
            <circle cx="6.5" cy="17.5" r="1" fill="var(--cyan)" stroke="none" />
            <circle cx="17.5" cy="17.5" r="1" fill="var(--cyan)" stroke="none" />
          </svg>
          <h1 style={{ fontSize: 20, fontWeight: 700 }}>BACnet Devices</h1>
        </div>
        {tableContent}
      </div>
    </div>
  )
}

// -- Network Discover Panel --

function BacnetDiscoverPanel({ onClose }: { onClose: () => void }) {
  const discoverMut = useDiscoverBacnet()
  const createMut = useCreateBacnetDevice()
  const [timeout, setTimeout_] = useState(5)

  const handleDiscover = () => {
    discoverMut.mutate({ timeout_secs: timeout })
  }

  const results = discoverMut.data?.devices ?? []

  return (
    <div style={{ marginBottom: 16, padding: 16, background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
      <div style={{ display: 'flex', alignItems: 'center', paddingBottom: 8, borderBottom: '1px solid var(--border)', marginBottom: 12 }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>BACnet Network Discovery</span>
        <button onClick={onClose} aria-label="Close" style={{ marginLeft: 'auto', background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s' }}>
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
      <div style={{ display: 'flex', gap: 8, alignItems: 'end', marginBottom: 12 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Timeout (s)</label>
          <input type="number" value={timeout} onChange={e => setTimeout_(parseInt(e.target.value) || 5)} min={1} max={30} style={{ ...inputStyle, width: 90, fontFamily: "var(--font-mono)" }} />
        </div>
        <button
          onClick={handleDiscover}
          disabled={discoverMut.isPending}
          style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)', opacity: discoverMut.isPending ? 0.5 : 1 }}
        >
          {discoverMut.isPending ? 'Discovering...' : 'Discover'}
        </button>
      </div>
      {discoverMut.isError && (
        <div style={{ color: 'var(--crit)', fontSize: 12, marginBottom: 8 }}>Discovery failed: {String(discoverMut.error)}</div>
      )}
      {results.length > 0 && (
        <div style={{ maxHeight: 200, overflow: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>IP</th>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Port</th>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Instance</th>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Vendor</th>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Action</th>
              </tr>
            </thead>
            <tbody>
              {results.map((r, i) => (
                <tr key={i}>
                  <td style={{ padding: '4px 10px', fontSize: 12, fontFamily: "var(--font-mono)" }}>{r.ip}</td>
                  <td style={{ padding: '4px 10px', fontSize: 12, fontFamily: "var(--font-mono)" }}>{r.port}</td>
                  <td style={{ padding: '4px 10px', fontSize: 12 }}>{r.device_instance}</td>
                  <td style={{ padding: '4px 10px', fontSize: 12 }}>{r.vendor_name || '-'}</td>
                  <td style={{ padding: '4px 10px' }}>
                    <button
                      onClick={() => createMut.mutate({
                        name: `bacnet_${r.ip.replace(/\./g, '_')}`,
                        ip: r.ip,
                        port: r.port,
                        device_instance: r.device_instance,
                        enabled: true,
                      })}
                      disabled={createMut.isPending}
                      style={{ padding: '3px 8px', borderRadius: 4, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: 'pointer', fontSize: 11 }}
                    >
                      Add
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {!discoverMut.isPending && results.length === 0 && discoverMut.data !== undefined && (
        <div style={{ color: 'var(--muted)', fontSize: 12 }}>No BACnet devices found on this network.</div>
      )}
    </div>
  )
}

// -- Inline Object Expansion --

function ExpandedBacnetObjects({ deviceId }: { deviceId: number }) {
  const { data: objectData, isLoading } = useBacnetDevice(String(deviceId))
  const addMut = useAddBacnetObject()
  const removeMut = useRemoveBacnetObject()

  const [newObjectType, setNewObjectType] = useState<string>('analogInput')
  const [newInstance, setNewInstance] = useState(0)
  const [newName, setNewName] = useState('')
  const [newDisplayName, setNewDisplayName] = useState('')
  const [newProperty, setNewProperty] = useState('presentValue')
  const [newCovIncrement, setNewCovIncrement] = useState(0)

  const objects: BacnetObject[] = objectData?.objects ?? []

  const handleAdd = () => {
    if (!newName.trim()) return
    addMut.mutate(
      {
        deviceId,
        object: {
          object_type: newObjectType,
          object_instance: newInstance,
          name: newName.trim(),
          display_name: newDisplayName.trim() || newName.trim(),
          property: newProperty,
          cov_increment: newCovIncrement,
        },
      },
      {
        onSuccess: () => {
          setNewName('')
          setNewDisplayName('')
          setNewInstance(newInstance + 1)
        },
      },
    )
  }

  const handleRemove = (objectId: number) => {
    if (!confirm('Remove this object?')) return
    removeMut.mutate({ deviceId, objectId })
  }

  const TYPE_COLORS: Record<string, { bg: string; fg: string }> = {
    analogInput:      { bg: 'rgba(34,197,94,.12)', fg: '#4ade80' },
    analogOutput:     { bg: 'rgba(59,130,246,.12)', fg: '#60a5fa' },
    analogValue:      { bg: 'rgba(0,200,200,.08)', fg: 'var(--cyan)' },
    binaryInput:      { bg: 'rgba(251,191,36,.12)', fg: '#fbbf24' },
    binaryOutput:     { bg: 'rgba(168,85,247,.12)', fg: '#c084fc' },
    binaryValue:      { bg: 'rgba(239,68,68,.12)', fg: '#f87171' },
    multiStateInput:  { bg: 'rgba(139,92,246,.12)', fg: '#a78bfa' },
    multiStateOutput: { bg: 'rgba(251,191,36,.12)', fg: '#fbbf24' },
    multiStateValue:  { bg: 'rgba(59,130,246,.12)', fg: '#60a5fa' },
  }

  const nodeThStyle: React.CSSProperties = {
    textAlign: 'left', padding: '6px 12px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    background: 'rgba(255,255,255,.02)',
  }
  const nodeTdStyle: React.CSSProperties = { padding: '6px 12px', fontSize: 12, borderBottom: '1px solid var(--border)' }

  return (
    <div style={{ maxHeight: 400, overflow: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse' }}>
        <thead>
          <tr>
            <th style={nodeThStyle}>Name</th>
            <th style={nodeThStyle}>Display</th>
            <th style={nodeThStyle}>Type</th>
            <th style={nodeThStyle}>Instance</th>
            <th style={nodeThStyle}>Property</th>
            <th style={nodeThStyle}>COV Inc</th>
            <th style={{ ...nodeThStyle, width: 80 }}>Action</th>
          </tr>
        </thead>
        <tbody>
          {isLoading ? (
            <tr><td colSpan={7} style={{ ...nodeTdStyle, color: 'var(--muted)' }}>Loading objects...</td></tr>
          ) : objects.length === 0 ? (
            <tr><td colSpan={7} style={{ ...nodeTdStyle, color: 'var(--muted)' }}>No objects configured. Add objects below.</td></tr>
          ) : (
            objects.map(o => {
              const tc = TYPE_COLORS[o.object_type] ?? TYPE_COLORS.analogValue
              return (
                <tr key={o.id}>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{o.name}</td>
                  <td style={nodeTdStyle}>{o.display_name || '-'}</td>
                  <td style={nodeTdStyle}>
                    <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: tc.bg, color: tc.fg }}>
                      {o.object_type}
                    </span>
                  </td>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{o.object_instance}</td>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{o.property}</td>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{o.cov_increment}</td>
                  <td style={nodeTdStyle}>
                    <button
                      onClick={() => handleRemove(o.id)}
                      style={{ padding: '3px 6px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                      aria-label={`Remove object ${o.name}`}
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
                placeholder="object_name"
                style={{ ...inputStyle, fontSize: 11, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <input
                value={newDisplayName}
                onChange={e => setNewDisplayName(e.target.value)}
                placeholder="Display"
                style={{ ...inputStyle, fontSize: 11 }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <select value={newObjectType} onChange={e => setNewObjectType(e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>
                {BACNET_OBJECT_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
              </select>
            </td>
            <td style={{ padding: '6px 12px' }}>
              <input
                type="number"
                value={newInstance}
                onChange={e => setNewInstance(parseInt(e.target.value) || 0)}
                style={{ ...inputStyle, fontSize: 11, width: 60, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <input
                value={newProperty}
                onChange={e => setNewProperty(e.target.value)}
                placeholder="presentValue"
                style={{ ...inputStyle, fontSize: 11, width: 110, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <input
                type="number"
                step="0.01"
                value={newCovIncrement}
                onChange={e => setNewCovIncrement(parseFloat(e.target.value) || 0)}
                style={{ ...inputStyle, fontSize: 11, width: 60, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <button
                onClick={handleAdd}
                disabled={!newName.trim() || addMut.isPending}
                style={{ padding: '4px 10px', borderRadius: 4, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: newName.trim() ? 'pointer' : 'not-allowed', fontSize: 11, opacity: newName.trim() ? 1 : 0.5 }}
              >
                {addMut.isPending ? '...' : '+ Add'}
              </button>
            </td>
          </tr>
        </tfoot>
      </table>
    </div>
  )
}

// -- Inline Create / Edit Form --

function BacnetDeviceForm({
  device,
  onSubmit,
  onClose,
  isPending,
}: {
  device?: BacnetDevice
  onSubmit: (fields: Record<string, any>) => void
  onClose: () => void
  isPending: boolean
}) {
  const isEdit = !!device
  const src = device
  const [name, setName] = useState(src?.name ?? '')
  const [ip, setIp] = useState(src?.ip ?? '')
  const [port, setPort] = useState(src?.port ?? 47808)
  const [deviceInstance, setDeviceInstance] = useState(src?.device_instance ?? 0)
  const [intervalMs, setIntervalMs] = useState(src?.interval_ms ?? 5000)
  const [covEnabled, setCovEnabled] = useState(src?.cov_enabled ?? false)
  const [covLifetime, setCovLifetime] = useState(src?.cov_lifetime ?? 3600)
  const [enabled, setEnabled] = useState(src?.enabled ?? true)

  const handleSubmit = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    onSubmit({
      name: name || 'New BACnet Device',
      ip,
      port,
      device_instance: deviceInstance,
      interval_ms: intervalMs,
      cov_enabled: covEnabled,
      cov_lifetime: covLifetime,
      enabled,
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
        {isEdit ? `Edit: ${device!.name || device!.id}` : 'Add BACnet Device'}
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Name *</label>
          <input value={name} onChange={e => setName(e.target.value)} placeholder="BACnet Controller A" style={inputStyle} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>IP Address *</label>
          <input value={ip} onChange={e => setIp(e.target.value)} placeholder="192.168.1.100" required style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr', gap: 8 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Port</label>
          <input type="number" value={port} onChange={e => setPort(parseInt(e.target.value) || 47808)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Device Instance</label>
          <input type="number" value={deviceInstance} onChange={e => setDeviceInstance(parseInt(e.target.value) || 0)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Interval (ms)</label>
          <input type="number" value={intervalMs} onChange={e => setIntervalMs(parseInt(e.target.value) || 5000)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Enabled</label>
          <div style={{ display: 'flex', alignItems: 'center', height: 32 }}>
            <input type="checkbox" checked={enabled} onChange={e => setEnabled(e.target.checked)} style={{ cursor: 'pointer' }} />
          </div>
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>COV Enabled</label>
          <div style={{ display: 'flex', alignItems: 'center', height: 32 }}>
            <input type="checkbox" checked={covEnabled} onChange={e => setCovEnabled(e.target.checked)} style={{ cursor: 'pointer' }} />
          </div>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>COV Lifetime (s)</label>
          <input type="number" value={covLifetime} onChange={e => setCovLifetime(parseInt(e.target.value) || 3600)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
      </div>

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
