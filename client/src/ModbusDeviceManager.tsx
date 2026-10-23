import { useState, useEffect } from 'react'
import { useModbusDevices, useModbusPollerStatus, useCreateModbusDevice, useUpdateModbusDevice, useDeleteModbusDevice, useRestartModbusPoller, fetchJson } from './queries'
import { inputStyle, btnStyle } from './styles'
import { ModbusDeviceForm } from './ModbusDeviceForm'
import { SimControls } from './components/SimControls'
import { SimProfileManager } from './components/SimProfileManager'
import type { ModbusDevice, ModbusRegister } from './types'
import { FileDropZone, ImportFileButton, ImportNotice } from './components/FileDropZone'
import { deviceFileKind, parseModbusDeviceFile } from './deviceFileImport'

export function ModbusDeviceManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: devices = [], isLoading } = useModbusDevices()
  const { data: statuses = [] } = useModbusPollerStatus()
  const createMut = useCreateModbusDevice()
  const updateMut = useUpdateModbusDevice()
  const deleteMut = useDeleteModbusDevice()
  const restartMut = useRestartModbusPoller()

  const [filter, setFilter] = useState('')
  const [showForm, setShowForm] = useState(false)
  const [editDevice, setEditDevice] = useState<(ModbusDevice & { registers?: ModbusRegister[] }) | undefined>(undefined)
  const [dupDevice, setDupDevice] = useState<(ModbusDevice & { registers?: ModbusRegister[] }) | undefined>(undefined)
  const [simDevice, setSimDevice] = useState<ModbusDevice | undefined>(undefined)
  const [showSimProfiles, setShowSimProfiles] = useState(false)
  const [importLines, setImportLines] = useState<string[]>([])
  const [simStates, setSimStates] = useState<Record<string, { simulation_mode: boolean; sim_profile_id: number | null; sim_config: string | null }>>({})

  const statusMap = Object.fromEntries(statuses.map(s => [s.device_id, s.status || (s.running ? 'online' : 'offline')]))
  const filtered = devices.filter(d => (d.name || d.id).toLowerCase().includes(filter) || d.ip.toLowerCase().includes(filter))

  useEffect(() => {
    if (embedded) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, embedded]);

  const thStyle: React.CSSProperties = { textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, borderBottom: '1px solid var(--border)' };
  const tdStyle: React.CSSProperties = { padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)' };

  const handleDup = async (d: ModbusDevice) => {
    const data = await fetchJson<{ registers: ModbusRegister[] }>(`/api/v1/modbus-devices/${d.id}`)
    setDupDevice({ ...d, registers: data.registers })
    setShowForm(true)
  }

  const handleCreate = (data: any) => {
    createMut.mutate(data, { onSuccess: () => { setShowForm(false); setDupDevice(undefined) } })
  }

  const handleUpdate = (data: any) => {
    if (!editDevice) return
    updateMut.mutate({ id: editDevice.id, ...data }, {
      onSuccess: () => { setEditDevice(undefined) }
    })
  }

  // Device files from Modbus Poll's "Export to SCADA" (Voltrus SCADA device), by drop or file picker.
  const importFiles = async (files: File[]) => {
    const lines: string[] = []
    for (const file of files) {
      const text = await file.text()
      if (deviceFileKind(text) === 'bacnet') { lines.push(`${file.name}: this is a BACnet device file, import it on the BACnet devices page`); continue }
      const { devices: parsed, errors } = parseModbusDeviceFile(text)
      errors.forEach(e => lines.push(`${file.name}: ${e}`))
      for (const d of parsed) {
        try {
          await createMut.mutateAsync(d)
          lines.push(`${file.name}: added ${d.name} with ${d.registers.length} registers`)
        } catch (e) {
          lines.push(`${file.name}: ${d.name} was not added: ${e instanceof Error ? e.message : String(e)}`)
        }
      }
    }
    setImportLines(lines)
  }

  const handleDelete = (id: string) => {
    if (!confirm('Delete this device and stop its poller?')) return
    deleteMut.mutate(id)
  }

  const handleEdit = async (d: ModbusDevice) => {
    const data = await fetchJson<{ registers: ModbusRegister[] }>(`/api/v1/modbus-devices/${d.id}`)
    setEditDevice({ ...d, registers: data.registers })
  }

  const tableContent = (
    <FileDropZone onFiles={importFiles} style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <span style={{ color: 'var(--muted)', fontSize: 13 }}>{devices.length} devices</span>
        <div style={{ display: 'flex', gap: 8 }}>
          <input placeholder="Search..." value={filter} onChange={e => setFilter(e.target.value)} style={{ ...inputStyle, width: 200 }} />
          <ImportFileButton onFiles={importFiles} ariaLabel="Import modbus device file" />
          <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} onClick={() => setShowForm(true)} aria-label="Add modbus device">+ Add Device</button>
          <button style={{ ...btnStyle, borderColor: 'var(--warn)', color: 'var(--warn)' }} onClick={() => setShowSimProfiles(true)} aria-label="Simulation profiles">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83"/></svg>
            Sim Profiles
          </button>
        </div>
      </div>
      <ImportNotice lines={importLines} onDismiss={() => setImportLines([])} />
      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th style={thStyle}>Name</th>
              <th style={thStyle}>IP</th>
              <th style={thStyle}>Port</th>
              <th style={thStyle}>Unit</th>
              <th style={thStyle}>Interval</th>
              <th style={thStyle}>Endian</th>
              <th style={thStyle}>Registers</th>
              <th style={thStyle}>Status</th>
              <th style={thStyle}>Sim</th>
              <th style={thStyle}>Enabled</th>
              <th style={{ ...thStyle, width: 180 }}>Actions</th>
            </tr>
          </thead>
          <tbody>
            {filtered.map(d => (
              <tr key={d.id}>
                <td style={tdStyle}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <div style={{ width: 8, height: 8, borderRadius: '50%', background: statusMap[d.id] === 'online' ? 'var(--cyan)' : 'var(--crit)' }} />
                    {d.name || d.id}
                  </div>
                </td>
                <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.ip}</td>
                <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.port}</td>
                <td style={tdStyle}>{d.unit_id}</td>
                <td style={tdStyle}>{d.interval_ms}ms</td>
                <td style={tdStyle}><span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: 'rgba(139,92,246,.1)', color: '#a78bfa' }}>{d.endianness}</span></td>
                <td style={tdStyle}>{d.register_count}</td>
                <td style={tdStyle}>
                  <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: statusMap[d.id] === 'online' ? 'rgba(34,197,94,.1)' : 'rgba(239,68,68,.1)', color: statusMap[d.id] === 'online' ? 'var(--cyan)' : 'var(--crit)' }}>
                    {d.enabled ? (statusMap[d.id] === 'online' ? 'online' : 'offline') : 'disabled'}
                  </span>
                </td>
                <td style={tdStyle}>
                  <button
                    onClick={() => {
                      // Fetch current sim state then open controls
                      fetchJson<any>(`/api/v1/devices/${d.id}/simulate`).then(data => {
                        setSimStates(prev => ({ ...prev, [d.id]: { simulation_mode: data.simulation_mode ?? false, sim_profile_id: data.sim_profile_id ?? null, sim_config: data.sim_config ?? null } }))
                        setSimDevice(d)
                      }).catch(() => {
                        setSimStates(prev => ({ ...prev, [d.id]: { simulation_mode: false, sim_profile_id: null, sim_config: null } }))
                        setSimDevice(d)
                      })
                    }}
                    style={{
                      padding: '2px 8px', borderRadius: 4, border: `1px solid ${simStates[d.id]?.simulation_mode ? 'var(--warn)' : 'var(--border)'}`,
                      background: simStates[d.id]?.simulation_mode ? 'rgba(234,179,8,.12)' : 'transparent',
                      color: simStates[d.id]?.simulation_mode ? 'var(--warn)' : 'var(--muted)',
                      cursor: 'pointer', fontSize: 11, fontWeight: 700, textTransform: 'uppercase', letterSpacing: '.04em',
                    }}
                  >{simStates[d.id]?.simulation_mode ? 'SIM ON' : 'SIM'}</button>
                </td>
                <td style={tdStyle}>
                  <input type="checkbox" checked={d.enabled} onChange={() => updateMut.mutate({ id: d.id, enabled: !d.enabled })} style={{ cursor: 'pointer' }} />
                </td>
                <td style={tdStyle}>
                  <div style={{ display: 'flex', gap: 4 }}>
                    <button onClick={() => handleEdit(d)} aria-label={`Edit ${d.name || d.id}`} style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 11 }}>Edit</button>
                    <button onClick={() => handleDup(d)} aria-label={`Duplicate ${d.name || d.id}`} style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 11 }}>Dup</button>
                    <button onClick={() => restartMut.mutate(d.id)} aria-label={`Restart ${d.name || d.id}`} style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 11 }}>Restart</button>
                    <button onClick={() => handleDelete(d.id)} aria-label={`Delete ${d.name || d.id}`} style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}>Del</button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </FileDropZone>
  );

  const modals = (<>
    {showForm && <ModbusDeviceForm dupDevice={dupDevice} onSave={handleCreate} onClose={() => { setShowForm(false); setDupDevice(undefined) }} />}
    {editDevice && <ModbusDeviceForm device={editDevice} onSave={handleUpdate} onClose={() => setEditDevice(undefined)} />}
    {simDevice && (() => {
      const state = simStates[simDevice.id] ?? { simulation_mode: false, sim_profile_id: null, sim_config: null }
      return (
        <SimControls
          deviceId={simDevice.id}
          deviceName={simDevice.name || simDevice.id}
          simulationMode={state.simulation_mode}
          simProfileId={state.sim_profile_id}
          simConfig={state.sim_config}
          onUpdate={() => {
            // Refresh sim state
            fetchJson<any>(`/api/v1/devices/${simDevice.id}/simulate`).then(data => {
              setSimStates(prev => ({ ...prev, [simDevice.id]: { simulation_mode: data.simulation_mode ?? false, sim_profile_id: data.sim_profile_id ?? null, sim_config: data.sim_config ?? null } }))
            }).catch(() => {})
          }}
        />
      )
    })()}
    {showSimProfiles && (
      <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 60, display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '16px 24px', display: 'flex', alignItems: 'center', borderBottom: '1px solid var(--border)' }}>
          <h2 style={{ fontSize: 18, fontWeight: 700, margin: 0 }}>Simulation Profiles</h2>
          <button onClick={() => setShowSimProfiles(false)} aria-label="Close" style={{ marginLeft: 'auto', background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s' }}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
        <div style={{ flex: 1, overflow: 'auto' }}>
          <SimProfileManager />
        </div>
      </div>
    )}
  </>);

  if (embedded) {
    return (<>{tableContent}{modals}</>);
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column', padding: 24 }}>
      <div style={{ maxWidth: 1200, width: '100%', margin: '0 auto', display: 'flex', flexDirection: 'column', height: '100%' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <button onClick={onClose} aria-label="Close modbus devices" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
            </button>
            <h1 style={{ fontSize: 20, fontWeight: 700 }}>Modbus Devices</h1>
          </div>
        </div>
        {tableContent}
      </div>
      {modals}
    </div>
  )
}
