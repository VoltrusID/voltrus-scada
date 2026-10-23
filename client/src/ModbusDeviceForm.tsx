import { useState } from 'react'
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle } from './styles'
import { useScanModbusRegisters } from './queries'
import { useNetworks } from './queries/networks'
import type { ScannedRegister } from './queries'
import type { ModbusDevice, ModbusRegister } from './types'

interface RegisterRow {
  name: string
  address: number
  register_type: string
  register_area: string
  scale: number
  /** `address` is a 0-based offset (0-65535), not a 5-digit reference such as 40001. */
  zero_based: boolean
}

interface Props {
  device?: ModbusDevice & { registers?: ModbusRegister[] }
  dupDevice?: ModbusDevice & { registers?: ModbusRegister[] }
  onSave: (data: { id: string; name: string; ip: string; port: number; unit_id: number; interval_ms: number; endianness: string; registers: RegisterRow[] }) => void
  onClose: () => void
}

const REG_TYPES = ['float32', 'int32', 'uint16', 'int16']
const REG_AREAS = ['holding', 'input', 'coil', 'discrete']
const ENCODINGS = ['big', 'little', 'swapped']

export function ModbusDeviceForm({ device, dupDevice, onSave, onClose }: Props) {
  const source = device ?? dupDevice
  const isEdit = !!device
  const [id, setId] = useState(dupDevice ? '' : (source?.id ?? ''))
  const [name, setName] = useState(source?.name ?? '')
  const [ip, setIp] = useState(source?.ip ?? '')
  const [port, setPort] = useState(source?.port ?? 502)
  const [unitId, setUnitId] = useState(source?.unit_id ?? 1)
  const [intervalMs, setIntervalMs] = useState(source?.interval_ms ?? 1000)
  const [endianness, setEndianness] = useState(source?.endianness ?? 'big')
  const [network, setNetwork] = useState('')
  const { data: networksData } = useNetworks()
  const [registers, setRegisters] = useState<RegisterRow[]>(
    source?.registers?.map(r => ({ name: r.name, address: r.address, register_type: r.register_type, register_area: r.register_area || 'holding', scale: r.scale, zero_based: r.zero_based ?? false })) ?? []
  )

  // Scan state
  const [showScanDialog, setShowScanDialog] = useState(false)
  const [scanStart, setScanStart] = useState(0)
  const [scanEnd, setScanEnd] = useState(100)
  const [scanArea, setScanArea] = useState('holding')
  const [scanDelay, setScanDelay] = useState(500)
  const [scanResults, setScanResults] = useState<ScannedRegister[]>([])
  const [scanSelected, setScanSelected] = useState<Set<number>>(new Set())
  const [scanNames, setScanNames] = useState<Record<number, string>>({})
  const [scanError, setScanError] = useState('')
  const scanMut = useScanModbusRegisters()

  const addRegister = () => setRegisters([...registers, { name: '', address: 0, register_type: 'float32', register_area: 'holding', scale: 1, zero_based: false }])
  const removeRegister = (i: number) => setRegisters(registers.filter((_, j) => j !== i))
  const updateRegister = (i: number, field: keyof RegisterRow, value: any) => {
    const next = [...registers]
    next[i] = { ...next[i], [field]: value }
    setRegisters(next)
  }

  const handleSave = () => {
    if (!id || !ip) return
    onSave({ id, name: name || id, ip, port, unit_id: unitId, interval_ms: intervalMs, endianness, registers })
  }

  const handleScan = () => {
    setScanError('')
    setScanResults([])
    setScanSelected(new Set())
    scanMut.mutate(
      { ip, port, unit_id: unitId, start_addr: scanStart, end_addr: scanEnd, endianness, register_area: scanArea, scan_delay_ms: scanDelay },
      {
        onSuccess: (data) => {
          setScanResults(data.filter(r => r.readable))
        },
        onError: (err: any) => {
          setScanError(err?.message || 'Scan failed')
        },
      },
    )
  }

  const toggleSelect = (i: number) => {
    const next = new Set(scanSelected)
    if (next.has(i)) next.delete(i)
    else next.add(i)
    setScanSelected(next)
  }

  const selectAll = () => {
    if (scanSelected.size === scanResults.length) setScanSelected(new Set())
    else setScanSelected(new Set(scanResults.map((_, i) => i)))
  }

  const importSelected = () => {
    const newRegs = scanResults
      .filter((_, i) => scanSelected.has(i))
      .map(r => ({
        name: scanNames[r.address] || `reg_${String(r.address).padStart(5, '0')}`,
        address: r.address,
        register_type: (scanArea === 'coil' || scanArea === 'discrete') ? 'uint16' : r.likely_type,
        register_area: scanArea,
        scale: 1,
        zero_based: false,
      }))
    setRegisters([...registers, ...newRegs])
    setShowScanDialog(false)
    setScanResults([])
    setScanSelected(new Set())
    setScanNames({})
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }} onClick={onClose}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 520, maxHeight: '80vh', overflow: 'auto' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--border)', fontSize: 13, fontWeight: 600 }}>
          {isEdit ? `Edit: ${device!.name}` : 'Add Modbus Device'}
        </div>
        <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 8 }}>
          {!isEdit && (
            <label style={labelStyle}>
              <span style={labelTextStyle}>Device ID</span>
              <input value={id} onChange={e => setId(e.target.value)} placeholder="plts_1" style={inputStyle} />
            </label>
          )}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
            <label style={labelStyle}><span style={labelTextStyle}>Name</span><input value={name} onChange={e => setName(e.target.value)} placeholder="PLTS 1" style={inputStyle} /></label>
            <label style={labelStyle}><span style={labelTextStyle}>IP Address</span><input value={ip} onChange={e => setIp(e.target.value)} placeholder="192.168.1.10" style={monoInputStyle} /></label>
            <label style={labelStyle}><span style={labelTextStyle}>Port</span><input type="number" value={port} onChange={e => setPort(parseInt(e.target.value) || 502)} style={monoInputStyle} /></label>
            <label style={labelStyle}><span style={labelTextStyle}>Unit ID</span><input type="number" value={unitId} onChange={e => setUnitId(parseInt(e.target.value) || 1)} style={monoInputStyle} /></label>
            <label style={labelStyle}><span style={labelTextStyle}>Interval (ms)</span><input type="number" value={intervalMs} onChange={e => setIntervalMs(parseInt(e.target.value) || 1000)} style={monoInputStyle} /></label>
            <label style={labelStyle}><span style={labelTextStyle}>Endianness</span><select value={endianness} onChange={e => setEndianness(e.target.value)} style={inputStyle}>{ENCODINGS.map(e => <option key={e} value={e}>{e}</option>)}</select></label>
            <label style={labelStyle}><span style={labelTextStyle}>Network</span><select value={network} onChange={e => setNetwork(e.target.value)} style={inputStyle}><option value="">Default</option>{networksData?.configured_networks?.map(n => <option key={n} value={n}>{n}</option>)}</select></label>
          </div>

          <div style={{ marginTop: 8, borderTop: '1px solid var(--border)', paddingTop: 8 }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
              <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)' }}>Registers</span>
              <div style={{ display: 'flex', gap: 4 }}>
                <button onClick={() => { setShowScanDialog(true); setScanResults([]); setScanError('') }} style={{ padding: '3px 10px', borderRadius: 4, fontSize: 11, cursor: 'pointer', border: '1px solid var(--cyan)', background: 'var(--card)', color: 'var(--cyan)' }}>Scan</button>
                <button onClick={addRegister} style={{ padding: '3px 10px', borderRadius: 4, fontSize: 11, cursor: 'pointer', border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--cyan)' }}>+ Add</button>
              </div>
            </div>
            {registers.length === 0 && <div style={{ color: 'var(--muted)', fontSize: 12, padding: 8 }}>No registers configured.</div>}
            {/* Header row */}
            {registers.length > 0 && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 70px 34px 80px 70px 60px 28px', gap: 4, marginBottom: 2, fontSize: 10, color: 'var(--muted)', paddingLeft: 2 }}>
                <span>name</span><span>addr</span><span title="Address is a 0-based offset, not a 5-digit reference">0-b</span><span>type</span><span>area</span><span>scale</span><span></span>
              </div>
            )}
            {registers.map((reg, i) => (
              <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 70px 34px 80px 70px 60px 28px', gap: 4, marginBottom: 4, alignItems: 'end' }}>
                <input value={reg.name} onChange={e => updateRegister(i, 'name', e.target.value)} placeholder="voltage" style={{ ...monoInputStyle, fontSize: 12 }} />
                <input type="number" value={reg.address} onChange={e => updateRegister(i, 'address', parseInt(e.target.value) || 0)} placeholder="40001" style={{ ...monoInputStyle, fontSize: 12 }} />
                <input type="checkbox" checked={reg.zero_based} onChange={e => updateRegister(i, 'zero_based', e.target.checked)} title="Address is a 0-based offset (0-65535), not a 5-digit reference" aria-label="Zero-based address" style={{ justifySelf: 'center', marginBottom: 8 }} />
                <select value={reg.register_type} onChange={e => updateRegister(i, 'register_type', e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>{REG_TYPES.map(t => <option key={t} value={t}>{t}</option>)}</select>
                <select value={reg.register_area} onChange={e => updateRegister(i, 'register_area', e.target.value)} style={{ ...inputStyle, fontSize: 11 }}>{REG_AREAS.map(a => <option key={a} value={a}>{a}</option>)}</select>
                <input type="number" step="0.01" value={reg.scale} onChange={e => updateRegister(i, 'scale', parseFloat(e.target.value) || 1)} style={{ ...monoInputStyle, fontSize: 12 }} />
                <button onClick={() => removeRegister(i)} style={{ background: 'none', border: 'none', color: 'var(--crit)', cursor: 'pointer', fontSize: 14 }}>&times;</button>
              </div>
            ))}
          </div>
        </div>
        <div style={{ display: 'flex', gap: 8, padding: '0 16px 16px' }}>
          <button onClick={handleSave} style={{ padding: '6px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 13 }}>{isEdit ? 'Update' : 'Create'}</button>
          <button onClick={onClose} style={{ padding: '6px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--muted)', cursor: 'pointer', fontSize: 13 }}>Cancel</button>
        </div>
      </div>

      {/* Scan Dialog */}
      {showScanDialog && (
        <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 70 }} onClick={() => setShowScanDialog(false)}>
          <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 560, maxHeight: '80vh', overflow: 'auto' }} onClick={e => e.stopPropagation()}>
            <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center' }}>
              <span style={{ fontSize: 13, fontWeight: 600 }}>Scan Modbus Registers</span>
              <button onClick={() => setShowScanDialog(false)} aria-label="Close" style={{ marginLeft: 'auto', background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s' }}>
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              </button>
            </div>
            <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 8 }}>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr', gap: 8 }}>
                <label style={labelStyle}><span style={labelTextStyle}>Start Addr</span><input type="number" value={scanStart} onChange={e => setScanStart(parseInt(e.target.value) || 0)} style={monoInputStyle} /></label>
                <label style={labelStyle}><span style={labelTextStyle}>End Addr</span><input type="number" value={scanEnd} onChange={e => setScanEnd(parseInt(e.target.value) || 100)} style={monoInputStyle} /></label>
                <label style={labelStyle}>
                  <span style={labelTextStyle}>Area</span>
                  <select value={scanArea} onChange={e => setScanArea(e.target.value)} style={inputStyle}>
                    {REG_AREAS.map(a => <option key={a} value={a}>{a}</option>)}
                  </select>
                </label>
                <label style={labelStyle}><span style={labelTextStyle}>Delay (ms)</span><input type="number" value={scanDelay} onChange={e => setScanDelay(parseInt(e.target.value) || 500)} style={monoInputStyle} /></label>
              </div>
              <div style={{ fontSize: 11, color: 'var(--muted)' }}>Scanning device at {ip}:{port} (Unit {unitId}, {endianness})</div>
              <button onClick={handleScan} disabled={scanMut.isPending || !ip} style={{ padding: '6px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: scanMut.isPending ? 'wait' : 'pointer', fontSize: 12, opacity: scanMut.isPending ? 0.6 : 1 }}>
                {scanMut.isPending ? 'Scanning...' : 'Start Scan'}
              </button>
              {scanError && <div style={{ color: 'var(--crit)', fontSize: 12, padding: '4px 0' }}>{scanError}</div>}

              {scanResults.length > 0 && (
                <>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginTop: 8 }}>
                    <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)' }}>{scanResults.length} register{scanResults.length !== 1 ? 's' : ''} found</span>
                    <div style={{ display: 'flex', gap: 4 }}>
                      <button onClick={selectAll} style={{ padding: '2px 8px', borderRadius: 4, fontSize: 11, cursor: 'pointer', border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)' }}>
                        {scanSelected.size === scanResults.length ? 'Deselect All' : 'Select All'}
                      </button>
                      <button onClick={importSelected} disabled={scanSelected.size === 0} style={{ padding: '2px 8px', borderRadius: 4, fontSize: 11, cursor: scanSelected.size === 0 ? 'not-allowed' : 'pointer', border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', opacity: scanSelected.size === 0 ? 0.5 : 1 }}>
                        Import {scanSelected.size > 0 ? scanSelected.size : ''} Selected
                      </button>
                    </div>
                  </div>
                  <div style={{ maxHeight: 300, overflow: 'auto', border: '1px solid var(--border)', borderRadius: 6 }}>
                    <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 11 }}>
                      <thead>
                        <tr style={{ borderBottom: '1px solid var(--border)' }}>
                          <th style={{ padding: '4px 6px', textAlign: 'left', color: 'var(--muted)', fontWeight: 500 }}></th>
                          <th style={{ padding: '4px 6px', textAlign: 'left', color: 'var(--muted)', fontWeight: 500 }}>Name</th>
                          <th style={{ padding: '4px 6px', textAlign: 'left', color: 'var(--muted)', fontWeight: 500 }}>Address</th>
                          <th style={{ padding: '4px 6px', textAlign: 'left', color: 'var(--muted)', fontWeight: 500 }}>Value</th>
                          <th style={{ padding: '4px 6px', textAlign: 'left', color: 'var(--muted)', fontWeight: 500 }}>Changed</th>
                          <th style={{ padding: '4px 6px', textAlign: 'left', color: 'var(--muted)', fontWeight: 500 }}>Type</th>
                        </tr>
                      </thead>
                      <tbody>
                        {scanResults.map((r, i) => (
                          <tr key={i} style={{ borderBottom: '1px solid var(--border)', background: scanSelected.has(i) ? 'rgba(0,255,200,0.05)' : 'transparent' }}>
                            <td style={{ padding: '3px 6px' }}><input type="checkbox" checked={scanSelected.has(i)} onChange={() => toggleSelect(i)} style={{ cursor: 'pointer' }} /></td>
                            <td style={{ padding: '3px 4px' }}><input value={scanNames[r.address] ?? ''} onChange={e => setScanNames(prev => ({ ...prev, [r.address]: e.target.value }))} placeholder={`reg_${String(r.address).padStart(5, '0')}`} style={{ width: '100%', background: 'transparent', border: 'none', color: 'var(--fg)', fontFamily: 'monospace', fontSize: 11, outline: 'none' }} /></td>
                            <td style={{ padding: '3px 6px', fontFamily: 'monospace' }}>{r.address}</td>
                            <td style={{ padding: '3px 6px', fontFamily: 'monospace' }}>{r.interpreted_value != null ? r.interpreted_value.toFixed(2) : '-'}</td>
                            <td style={{ padding: '3px 6px' }}>{r.changed ? <span style={{ color: 'var(--green)' }}>yes</span> : <span style={{ color: 'var(--muted)' }}>no</span>}</td>
                            <td style={{ padding: '3px 6px' }}><span style={{ padding: '1px 5px', borderRadius: 3, background: 'var(--bg)', border: '1px solid var(--border)', fontSize: 10 }}>{r.likely_type}</span></td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
