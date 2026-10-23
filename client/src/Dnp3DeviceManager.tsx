import { useState, useEffect, Fragment } from 'react'
import { useDnp3Devices, useDnp3Device, useDnp3GroupVariations, useCreateDnp3Device, useUpdateDnp3Device, useDeleteDnp3Device, useAddDnp3Point, useRemoveDnp3Point, useScanDnp3Network } from './queries'
import { btnStyle, inputStyle } from './styles'
import type { Dnp3Device, Dnp3Point } from './queries'

const DNP3_DATA_TYPES = ['float64', 'float32', 'int32', 'int16', 'uint32', 'uint16', 'bool', 'string'] as const

export function Dnp3DeviceManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: devices = [], isLoading } = useDnp3Devices()
  const createMut = useCreateDnp3Device()
  const updateMut = useUpdateDnp3Device()
  const deleteMut = useDeleteDnp3Device()

  const [filter, setFilter] = useState('')
  const [showForm, setShowForm] = useState(false)
  const [editDevice, setEditDevice] = useState<Dnp3Device | null>(null)
  const [expandedId, setExpandedId] = useState<number | null>(null)
  const [showScan, setShowScan] = useState(false)

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

  const handleCreate = (device: Partial<Dnp3Device>) => {
    createMut.mutate(device, { onSuccess: () => setShowForm(false) })
  }

  const handleUpdate = (id: number, fields: Record<string, any>) => {
    updateMut.mutate({ id, ...fields }, { onSuccess: () => setEditDevice(null) })
  }

  const handleDelete = (id: number) => {
    if (!confirm('Delete this DNP3 device?')) return
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
    <>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <span style={{ color: 'var(--muted)', fontSize: 13 }}>{devices.length} device{devices.length !== 1 ? 's' : ''}</span>
        <div style={{ display: 'flex', gap: 8 }}>
          <input placeholder="Search..." value={filter} onChange={e => setFilter(e.target.value)} style={{ ...inputStyle, width: 200 }} />
          <button
            style={{ ...btnStyle, borderColor: 'var(--muted)', color: 'var(--muted)' }}
            onClick={() => setShowScan(s => !s)}
            aria-label="Network discovery"
          >
            Scan
          </button>
          <button
            style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}
            onClick={() => { setShowForm(true); setEditDevice(null) }}
            aria-label="Add DNP3 device"
          >
            + Add Device
          </button>
        </div>
      </div>

      {showScan && (
        <Dnp3ScanPanel onClose={() => setShowScan(false)} />
      )}

      {(showForm || editDevice) && (
        <Dnp3DeviceForm
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
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>No DNP3 devices found.</div>
        ) : (
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={thStyle}>Name</th>
                <th style={thStyle}>IP</th>
                <th style={thStyle}>Port</th>
                <th style={thStyle}>Master</th>
                <th style={thStyle}>Outstation</th>
                <th style={thStyle}>Points</th>
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
                          aria-label={expandedId === d.id ? 'Collapse points' : 'Expand points'}
                        >
                          {expandedId === d.id ? '\u25BC' : '\u25B6'}
                        </button>
                        <div style={{ width: 8, height: 8, borderRadius: '50%', background: d.enabled ? 'var(--ok)' : 'var(--crit)' }} />
                        {d.name || d.id}
                      </div>
                    </td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.ip}</td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.port}</td>
                    <td style={tdStyle}>{d.master_address}</td>
                    <td style={tdStyle}>{d.outstation_address}</td>
                    <td style={tdStyle}>{d.point_count ?? 0}</td>
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
                      <td colSpan={8} style={{ padding: 0, borderBottom: '1px solid var(--border)' }}>
                        <ExpandedDnp3Points deviceId={d.id} />
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
          <button onClick={onClose} aria-label="Close DNP3 devices" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
          </button>
          {/* DNP3 SCADA Protocol Icon */}
          <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--cyan)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="3" />
            <circle cx="8" cy="8" r="1.5" fill="var(--cyan)" stroke="none" />
            <circle cx="16" cy="8" r="1.5" fill="var(--cyan)" stroke="none" />
            <circle cx="8" cy="16" r="1.5" fill="none" />
            <circle cx="16" cy="16" r="1.5" fill="none" />
            <line x1="9.5" y1="8" x2="14.5" y2="8" />
            <line x1="8" y1="9.5" x2="8" y2="14.5" />
            <line x1="16" y1="9.5" x2="16" y2="14.5" />
          </svg>
          <h1 style={{ fontSize: 20, fontWeight: 700 }}>DNP3 Devices</h1>
        </div>
        {tableContent}
      </div>
    </div>
  )
}

// ── Network Scan Panel ──

function Dnp3ScanPanel({ onClose }: { onClose: () => void }) {
  const scanMut = useScanDnp3Network()
  const [cidr, setCidr] = useState('192.168.1.0/24')
  const [port, setPort] = useState(20000)
  const createMut = useCreateDnp3Device()

  const handleScan = () => {
    if (!cidr.trim()) return
    scanMut.mutate({ range: cidr.trim(), port })
  }

  const results = scanMut.data ?? []

  return (
    <div style={{ marginBottom: 16, padding: 16, background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
      <div style={{ display: 'flex', alignItems: 'center', paddingBottom: 8, borderBottom: '1px solid var(--border)', marginBottom: 12 }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>Network Discovery</span>
        <button onClick={onClose} aria-label="Close" style={{ marginLeft: 'auto', background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s' }}>
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
      <div style={{ display: 'flex', gap: 8, alignItems: 'end', marginBottom: 12 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>CIDR Range</label>
          <input value={cidr} onChange={e => setCidr(e.target.value)} placeholder="192.168.1.0/24" style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Port</label>
          <input type="number" value={port} onChange={e => setPort(parseInt(e.target.value) || 20000)} style={{ ...inputStyle, width: 90, fontFamily: "var(--font-mono)" }} />
        </div>
        <button
          onClick={handleScan}
          disabled={scanMut.isPending}
          style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)', opacity: scanMut.isPending ? 0.5 : 1 }}
        >
          {scanMut.isPending ? 'Scanning...' : 'Scan'}
        </button>
      </div>
      {scanMut.isError && (
        <div style={{ color: 'var(--crit)', fontSize: 12, marginBottom: 8 }}>Scan failed: {String(scanMut.error)}</div>
      )}
      {results.length > 0 && (
        <div style={{ maxHeight: 200, overflow: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>IP</th>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Port</th>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Outstation Addr</th>
                <th style={{ textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Action</th>
              </tr>
            </thead>
            <tbody>
              {results.map((r, i) => (
                <tr key={i}>
                  <td style={{ padding: '4px 10px', fontSize: 12, fontFamily: "var(--font-mono)" }}>{r.ip}</td>
                  <td style={{ padding: '4px 10px', fontSize: 12, fontFamily: "var(--font-mono)" }}>{r.port}</td>
                  <td style={{ padding: '4px 10px', fontSize: 12 }}>{r.outstation_address}</td>
                  <td style={{ padding: '4px 10px' }}>
                    <button
                      onClick={() => createMut.mutate({
                        name: `dnp3_${r.ip.replace(/\./g, '_')}`,
                        ip: r.ip,
                        port: r.port,
                        outstation_address: r.outstation_address,
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
      {!scanMut.isPending && results.length === 0 && scanMut.data !== undefined && (
        <div style={{ color: 'var(--muted)', fontSize: 12 }}>No DNP3 outstations found on this network.</div>
      )}
    </div>
  )
}

// ── Inline Point Expansion ──

function ExpandedDnp3Points({ deviceId }: { deviceId: number }) {
  const { data: deviceData, isLoading } = useDnp3Device(String(deviceId))
  const { data: groupVariations = [] } = useDnp3GroupVariations()
  const addPointMut = useAddDnp3Point()
  const removePointMut = useRemoveDnp3Point()

  const [newName, setNewName] = useState('')
  const [newGroup, setNewGroup] = useState(30)
  const [newVariation, setNewVariation] = useState(0)
  const [newPointIndex, setNewPointIndex] = useState(0)
  const [newDisplayName, setNewDisplayName] = useState('')
  const [newDataType, setNewDataType] = useState<string>('float64')
  const [newScale, setNewScale] = useState(1)
  const [newDeadband, setNewDeadband] = useState(0)

  const pts: Dnp3Point[] = deviceData?.points ?? []

  const handleAdd = () => {
    if (!newName.trim()) return
    addPointMut.mutate(
      {
        deviceId,
        point: {
          name: newName.trim(),
          display_name: newDisplayName.trim() || newName.trim(),
          group_num: newGroup,
          variation: newVariation,
          point_index: newPointIndex,
          data_type: newDataType,
          scale: newScale,
          deadband: newDeadband,
        },
      },
      {
        onSuccess: () => {
          setNewName('')
          setNewDisplayName('')
          setNewPointIndex(newPointIndex + 1)
        },
      },
    )
  }

  const handleRemove = (pointId: number) => {
    if (!confirm('Remove this point?')) return
    removePointMut.mutate({ deviceId, pointId })
  }

  const GV_COLORS: Record<number, { bg: string; fg: string }> = {
    1:  { bg: 'rgba(34,197,94,.12)', fg: '#4ade80' },
    2:  { bg: 'rgba(59,130,246,.12)', fg: '#60a5fa' },
    3:  { bg: 'rgba(251,191,36,.12)', fg: '#fbbf24' },
    4:  { bg: 'rgba(168,85,247,.12)', fg: '#c084fc' },
    10: { bg: 'rgba(239,68,68,.12)', fg: '#f87171' },
    20: { bg: 'rgba(139,92,246,.12)', fg: '#a78bfa' },
    30: { bg: 'rgba(0,200,200,.08)', fg: 'var(--cyan)' },
    31: { bg: 'rgba(0,200,200,.08)', fg: 'var(--cyan)' },
    32: { bg: 'rgba(34,197,94,.12)', fg: '#4ade80' },
    40: { bg: 'rgba(59,130,246,.12)', fg: '#60a5fa' },
    41: { bg: 'rgba(59,130,246,.12)', fg: '#60a5fa' },
    50: { bg: 'rgba(251,191,36,.12)', fg: '#fbbf24' },
    60: { bg: 'rgba(168,85,247,.12)', fg: '#c084fc' },
    70: { bg: 'rgba(239,68,68,.12)', fg: '#f87171' },
    80: { bg: 'rgba(139,92,246,.12)', fg: '#a78bfa' },
  }

  const nodeThStyle: React.CSSProperties = {
    textAlign: 'left', padding: '6px 12px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    background: 'rgba(255,255,255,.02)',
  }
  const nodeTdStyle: React.CSSProperties = { padding: '6px 12px', fontSize: 12, borderBottom: '1px solid var(--border)' }

  // Filter group variations to unique groups for the dropdown
  const uniqueGroups = Array.from(new Set(groupVariations.map(gv => gv.group))).sort((a, b) => a - b)
  const filteredVariations = newVariation === 0
    ? groupVariations.filter(gv => gv.group === newGroup)
    : groupVariations.filter(gv => gv.group === newGroup)

  // Auto-select first variation when group changes
  useEffect(() => {
    const vars = groupVariations.filter(gv => gv.group === newGroup)
    if (vars.length > 0 && !vars.some(v => v.variation === newVariation)) {
      setNewVariation(vars[0].variation)
    }
  }, [newGroup, groupVariations, newVariation])

  return (
    <div style={{ maxHeight: 400, overflow: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse' }}>
        <thead>
          <tr>
            <th style={nodeThStyle}>Name</th>
            <th style={nodeThStyle}>Display</th>
            <th style={nodeThStyle}>Group</th>
            <th style={nodeThStyle}>Var</th>
            <th style={nodeThStyle}>Index</th>
            <th style={nodeThStyle}>Type</th>
            <th style={nodeThStyle}>Scale</th>
            <th style={nodeThStyle}>Deadband</th>
            <th style={{ ...nodeThStyle, width: 80 }}>Action</th>
          </tr>
        </thead>
        <tbody>
          {isLoading ? (
            <tr><td colSpan={9} style={{ ...nodeTdStyle, color: 'var(--muted)' }}>Loading points...</td></tr>
          ) : pts.length === 0 ? (
            <tr><td colSpan={9} style={{ ...nodeTdStyle, color: 'var(--muted)' }}>No points configured. Add points below.</td></tr>
          ) : (
            pts.map(p => {
              const gvColor = GV_COLORS[p.group_num] ?? GV_COLORS[30]
              return (
                <tr key={p.id}>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{p.name}</td>
                  <td style={nodeTdStyle}>{p.display_name || '-'}</td>
                  <td style={nodeTdStyle}>
                    <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: gvColor.bg, color: gvColor.fg }}>
                      G{p.group_num}
                    </span>
                  </td>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>V{p.variation}</td>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{p.point_index}</td>
                  <td style={nodeTdStyle}>
                    <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: 'rgba(0,200,200,.08)', color: 'var(--cyan)' }}>
                      {p.data_type}
                    </span>
                  </td>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{p.scale}</td>
                  <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{p.deadband}</td>
                  <td style={nodeTdStyle}>
                    <button
                      onClick={() => handleRemove(p.id)}
                      style={{ padding: '3px 6px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                      aria-label={`Remove point ${p.name}`}
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
                placeholder="point_name"
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
              <select value={newGroup} onChange={e => setNewGroup(parseInt(e.target.value))} style={{ ...inputStyle, fontSize: 11 }}>
                {uniqueGroups.map(g => <option key={g} value={g}>G{g}</option>)}
              </select>
            </td>
            <td style={{ padding: '6px 12px' }}>
              <select value={newVariation} onChange={e => setNewVariation(parseInt(e.target.value))} style={{ ...inputStyle, fontSize: 11 }}>
                {filteredVariations.map(gv => <option key={`${gv.group}-${gv.variation}`} value={gv.variation}>V{gv.variation}</option>)}
              </select>
            </td>
            <td style={{ padding: '6px 12px' }}>
              <input
                type="number"
                value={newPointIndex}
                onChange={e => setNewPointIndex(parseInt(e.target.value) || 0)}
                style={{ ...inputStyle, fontSize: 11, width: 60, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <select value={newDataType} onChange={e => setNewDataType(e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>
                {DNP3_DATA_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
              </select>
            </td>
            <td style={{ padding: '6px 12px' }}>
              <input
                type="number"
                step="0.01"
                value={newScale}
                onChange={e => setNewScale(parseFloat(e.target.value) || 1)}
                style={{ ...inputStyle, fontSize: 11, width: 60, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <input
                type="number"
                step="0.001"
                value={newDeadband}
                onChange={e => setNewDeadband(parseFloat(e.target.value) || 0)}
                style={{ ...inputStyle, fontSize: 11, width: 60, fontFamily: "var(--font-mono)" }}
              />
            </td>
            <td style={{ padding: '6px 12px' }}>
              <button
                onClick={handleAdd}
                disabled={!newName.trim() || addPointMut.isPending}
                style={{ padding: '4px 10px', borderRadius: 4, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: newName.trim() ? 'pointer' : 'not-allowed', fontSize: 11, opacity: newName.trim() ? 1 : 0.5 }}
              >
                {addPointMut.isPending ? '...' : '+ Add'}
              </button>
            </td>
          </tr>
        </tfoot>
      </table>
    </div>
  )
}

// ── Inline Create / Edit Form ──

function Dnp3DeviceForm({
  device,
  onSubmit,
  onClose,
  isPending,
}: {
  device?: Dnp3Device
  onSubmit: (fields: Record<string, any>) => void
  onClose: () => void
  isPending: boolean
}) {
  const isEdit = !!device
  const src = device
  const [name, setName] = useState(src?.name ?? '')
  const [ip, setIp] = useState(src?.ip ?? '')
  const [port, setPort] = useState(src?.port ?? 20000)
  const [masterAddress, setMasterAddress] = useState(src?.master_address ?? 1)
  const [outstationAddress, setOutstationAddress] = useState(src?.outstation_address ?? 10)
  const [enableUnsolicited, setEnableUnsolicited] = useState(src?.enable_unsolicited ?? true)
  const [integrityInterval, setIntegrityInterval] = useState(src?.integrity_poll_interval_secs ?? 300)
  const [eventScanMs, setEventScanMs] = useState<number | ''>(src?.event_scan_interval_ms ?? '')
  const [enabled, setEnabled] = useState(src?.enabled ?? true)

  const handleSubmit = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    onSubmit({
      name: name || 'New DNP3 Device',
      ip,
      port,
      master_address: masterAddress,
      outstation_address: outstationAddress,
      enable_unsolicited: enableUnsolicited,
      integrity_poll_interval_secs: integrityInterval,
      event_scan_interval_ms: eventScanMs === '' ? null : eventScanMs,
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
        {isEdit ? `Edit: ${device!.name || device!.id}` : 'Add DNP3 Device'}
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Name *</label>
          <input value={name} onChange={e => setName(e.target.value)} placeholder="RTU Substation A" style={inputStyle} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>IP Address *</label>
          <input value={ip} onChange={e => setIp(e.target.value)} placeholder="192.168.1.100" required style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr', gap: 8 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Port</label>
          <input type="number" value={port} onChange={e => setPort(parseInt(e.target.value) || 20000)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Master Addr</label>
          <input type="number" value={masterAddress} onChange={e => setMasterAddress(parseInt(e.target.value) || 1)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Outstation Addr</label>
          <input type="number" value={outstationAddress} onChange={e => setOutstationAddress(parseInt(e.target.value) || 10)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
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
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Unsolicited</label>
          <div style={{ display: 'flex', alignItems: 'center', height: 32 }}>
            <input type="checkbox" checked={enableUnsolicited} onChange={e => setEnableUnsolicited(e.target.checked)} style={{ cursor: 'pointer' }} />
          </div>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Integrity Poll (s)</label>
          <input type="number" value={integrityInterval} onChange={e => setIntegrityInterval(parseInt(e.target.value) || 300)} style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Event Scan (ms)</label>
          <input type="number" value={eventScanMs} onChange={e => setEventScanMs(e.target.value ? parseInt(e.target.value) : '')} placeholder="Optional" style={{ ...inputStyle, fontFamily: "var(--font-mono)" }} />
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
