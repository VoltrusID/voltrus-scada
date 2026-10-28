import { useEffect, useMemo, useState } from 'react'
import { useModbusDevices, useCreateModbusDevice, useScanModbusNetwork } from '../queries'
import { useNetworks } from '../queries/networks'
import { inputStyle, btnStyle, labelStyle, labelTextStyle } from '../styles'
import {
  cidrError, defaultScanRange, deviceIdFor, estimatedSeconds, isAlreadyAdded, newDevicePayload, scanSize,
  type DiscoveredDevice,
} from '../networkScan'

const keyOf = (d: DiscoveredDevice) => `${d.ip}:${d.port}:${d.unit_id}`

/** Scan a subnet for Modbus TCP devices and add the ones found, in one step. */
export function NetworkScanDialog({ onClose }: { onClose: () => void }) {
  const { data: networks } = useNetworks()
  const { data: existing = [] } = useModbusDevices()
  const scan = useScanModbusNetwork()
  const create = useCreateModbusDevice()

  const suggested = useMemo(() => defaultScanRange(networks?.interfaces ?? []), [networks])
  const [range, setRange] = useState('')
  const [rangeTouched, setRangeTouched] = useState(false)
  const [port, setPort] = useState(502)
  const [elapsed, setElapsed] = useState(0)
  const [scannedRange, setScannedRange] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [names, setNames] = useState<Record<string, string>>({})
  const [adding, setAdding] = useState(false)
  const [report, setReport] = useState<string[]>([])

  // Start from the server's own subnet unless the user has typed one.
  useEffect(() => {
    if (!rangeTouched && suggested) setRange(suggested)
  }, [suggested, rangeTouched])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !adding) onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, adding])

  const scanning = scan.isPending
  useEffect(() => {
    if (!scanning) return
    setElapsed(0)
    const started = Date.now()
    const id = setInterval(() => setElapsed((Date.now() - started) / 1000), 200)
    return () => clearInterval(id)
  }, [scanning])

  const rangeProblem = cidrError(range)
  const portProblem = Number.isInteger(port) && port >= 1 && port <= 65535 ? null : 'The port is 1 to 65535'
  const hosts = scanSize(range)
  const estimate = estimatedSeconds(hosts)
  const progress = Math.min(95, (elapsed / estimate) * 100)

  const found = scan.data?.devices ?? []
  const addable = found.filter(d => !isAlreadyAdded(d, existing))

  const runScan = () => {
    if (rangeProblem || portProblem) return
    setReport([])
    setSelected(new Set())
    setScannedRange(range.trim())
    scan.mutate({ range: range.trim(), port }, {
      onSuccess: data => {
        // Everything new is selected; the user unticks what they do not want.
        setSelected(new Set(data.devices.filter(d => !isAlreadyAdded(d, existing)).map(keyOf)))
      },
    })
  }

  const toggle = (k: string) => {
    const next = new Set(selected)
    if (next.has(k)) next.delete(k); else next.add(k)
    setSelected(next)
  }

  const addSelected = async () => {
    setAdding(true)
    const taken = new Set(existing.map(d => d.id))
    const lines: string[] = []
    const failed = new Set<string>()
    for (const d of found) {
      const k = keyOf(d)
      if (!selected.has(k)) continue
      const id = deviceIdFor(d, taken)
      const name = (names[k] ?? '').trim() || `Modbus ${d.ip}`
      try {
        await create.mutateAsync(newDevicePayload(d, id, name))
        taken.add(id)
        lines.push(`Added ${name} (${d.ip}:${d.port}, unit ${d.unit_id}) with ${d.responsive_registers.length} registers`)
      } catch (e) {
        failed.add(k)
        lines.push(`${d.ip}:${d.port} was not added: ${e instanceof Error ? e.message : String(e)}`)
      }
    }
    setAdding(false)
    setReport(lines)
    // What was added is no longer selectable: the device list now has it.
    setSelected(failed)
  }

  const th: React.CSSProperties = { textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, borderBottom: '1px solid var(--border)' }
  const td: React.CSSProperties = { padding: '6px 10px', fontSize: 13, borderBottom: '1px solid var(--border)' }
  const mono: React.CSSProperties = { fontFamily: 'var(--font-mono)', fontSize: 12 }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }} onClick={() => { if (!adding) onClose() }}>
      <div
        role="dialog"
        aria-label="Scan network for Modbus devices"
        style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 680, maxWidth: '95vw', maxHeight: '85vh', display: 'flex', flexDirection: 'column' }}
        onClick={e => e.stopPropagation()}
      >
        <div style={{ display: 'flex', alignItems: 'center', padding: '12px 16px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Scan Network for Modbus Devices</div>
          <button onClick={onClose} disabled={adding} aria-label="Close" style={{ marginLeft: 'auto', background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18 }}>&times;</button>
        </div>

        <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 12, overflow: 'auto' }}>
          <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}>
            <label style={{ ...labelStyle, flex: 1 }}>
              <span style={labelTextStyle}>Range (CIDR, up to /24)</span>
              <input
                value={range}
                onChange={e => { setRange(e.target.value); setRangeTouched(true) }}
                onKeyDown={e => { if (e.key === 'Enter') runScan() }}
                placeholder="192.168.1.0/24"
                spellCheck={false}
                aria-label="Range to scan"
                aria-invalid={!!rangeProblem && range !== ''}
                style={{ ...inputStyle, fontFamily: 'var(--font-mono)' }}
              />
            </label>
            <label style={{ ...labelStyle, width: 90 }}>
              <span style={labelTextStyle}>Port</span>
              <input type="number" min={1} max={65535} value={port} onChange={e => setPort(Number(e.target.value))} aria-label="Modbus port" style={inputStyle} />
            </label>
            <button
              onClick={runScan}
              disabled={scanning || adding || !!rangeProblem || !!portProblem}
              style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)', opacity: scanning || !!rangeProblem || !!portProblem ? 0.5 : 1, height: 32 }}
            >{scanning ? 'Scanning' : 'Scan'}</button>
          </div>
          {range !== '' && rangeProblem && <div style={{ fontSize: 12, color: 'var(--crit)' }} role="alert">{rangeProblem}</div>}
          {range === '' && !suggested && <div style={{ fontSize: 12, color: 'var(--muted)' }}>The server's own network could not be determined; enter the range of the devices' network.</div>}
          {portProblem && <div style={{ fontSize: 12, color: 'var(--crit)' }} role="alert">{portProblem}</div>}

          {scanning && (
            <div role="status" aria-live="polite">
              <div style={{ height: 6, background: 'var(--bg)', borderRadius: 3, overflow: 'hidden', border: '1px solid var(--border)' }}>
                <div style={{ width: `${progress}%`, height: '100%', background: 'var(--cyan)', transition: 'width .2s linear' }} />
              </div>
              <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 6 }}>
                Probing {hosts} address{hosts === 1 ? '' : 'es'} on port {port}, {elapsed.toFixed(0)} s of about {estimate.toFixed(0)} s
              </div>
            </div>
          )}

          {scan.isError && !scanning && (
            <div role="alert" style={{ fontSize: 13, color: 'var(--crit)', border: '1px solid var(--crit)', borderRadius: 6, padding: '8px 10px' }}>
              The scan failed: {scan.error instanceof Error ? scan.error.message : String(scan.error)}
            </div>
          )}

          {scan.isSuccess && !scanning && found.length === 0 && (
            <div style={{ fontSize: 13, color: 'var(--muted)', border: '1px dashed var(--border)', borderRadius: 6, padding: 16, textAlign: 'center' }}>
              No Modbus TCP device answered on {scannedRange}, port {port}. Check the range and the port, and that the devices are powered and reachable from this server.
            </div>
          )}

          {scan.isSuccess && !scanning && found.length > 0 && (
            <>
              <div style={{ fontSize: 12, color: 'var(--muted)' }}>
                {found.length} device{found.length === 1 ? '' : 's'} found on {scannedRange}
                {addable.length < found.length ? `, ${found.length - addable.length} already added` : ''}
              </div>
              <div style={{ border: '1px solid var(--border)', borderRadius: 8, overflow: 'auto' }}>
                <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                  <thead>
                    <tr>
                      <th style={{ ...th, width: 32 }}>
                        <input
                          type="checkbox"
                          aria-label="Select all new devices"
                          checked={addable.length > 0 && addable.every(d => selected.has(keyOf(d)))}
                          disabled={addable.length === 0 || adding}
                          onChange={e => setSelected(e.target.checked ? new Set(addable.map(keyOf)) : new Set())}
                        />
                      </th>
                      <th style={th}>Address</th>
                      <th style={th}>Unit</th>
                      <th style={th}>Registers that answered</th>
                      <th style={th}>Name</th>
                    </tr>
                  </thead>
                  <tbody>
                    {found.map(d => {
                      const k = keyOf(d)
                      const dup = isAlreadyAdded(d, existing)
                      return (
                        <tr key={k} style={{ opacity: dup ? 0.55 : 1 }}>
                          <td style={td}>
                            <input type="checkbox" aria-label={`Add ${d.ip}`} checked={selected.has(k)} disabled={dup || adding} onChange={() => toggle(k)} />
                          </td>
                          <td style={{ ...td, ...mono }}>{d.ip}:{d.port}</td>
                          <td style={td}>{d.unit_id}</td>
                          <td style={{ ...td, ...mono }}>
                            {d.responsive_registers.slice(0, 4).map(r => `${r.address}=${r.raw[0] ?? r.value}`).join('  ')}
                            {d.responsive_registers.length > 4 ? `  +${d.responsive_registers.length - 4}` : ''}
                          </td>
                          <td style={td}>
                            {dup
                              ? <span style={{ fontSize: 11, color: 'var(--muted)' }}>already added</span>
                              : <input
                                  value={names[k] ?? ''}
                                  placeholder={`Modbus ${d.ip}`}
                                  onChange={e => setNames({ ...names, [k]: e.target.value })}
                                  disabled={adding}
                                  aria-label={`Name for ${d.ip}`}
                                  style={{ ...inputStyle, padding: '3px 6px' }}
                                />}
                          </td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              </div>
            </>
          )}

          {report.length > 0 && (
            <div style={{ fontSize: 12, border: '1px solid var(--border)', borderRadius: 6, padding: '8px 10px', display: 'flex', flexDirection: 'column', gap: 2 }} role="status">
              {report.map((l, i) => <div key={i} style={{ color: l.includes('was not added') ? 'var(--crit)' : 'var(--green)' }}>{l}</div>)}
            </div>
          )}
        </div>

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: '10px 16px', borderTop: '1px solid var(--border)' }}>
          <button onClick={onClose} disabled={adding} style={btnStyle}>Close</button>
          <button
            onClick={addSelected}
            disabled={selected.size === 0 || adding || scanning}
            style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)', opacity: selected.size === 0 || adding ? 0.5 : 1 }}
          >{adding ? 'Adding' : `Add ${selected.size} selected`}</button>
        </div>
      </div>
    </div>
  )
}
