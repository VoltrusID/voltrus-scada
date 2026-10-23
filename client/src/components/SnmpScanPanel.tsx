import { useState, useEffect } from 'react'
import { inputStyle, labelStyle, labelTextStyle } from '../styles'
import {
  useStartScan,
  useScanStatus,
  useScanResults,
  useImportScannedDevice,
} from '../queries/assets'

interface Props {
  onClose: () => void
}

export function SnmpScanPanel({ onClose }: Props) {
  const [subnet, setSubnet] = useState('')
  const [community, setCommunity] = useState('public')
  const [importingIp, setImportingIp] = useState<string | null>(null)

  const startScan = useStartScan()
  const { data: status } = useScanStatus()
  const { data: results, refetch: refetchResults } = useScanResults()
  const importDevice = useImportScannedDevice()

  const scanning = status?.scanning ?? false
  const progress = status?.progress ?? 0
  const total = status?.total ?? 0

  // Refetch results when scan completes
  useEffect(() => {
    if (!scanning && status) {
      refetchResults()
    }
  }, [scanning, status, refetchResults])

  // Escape key to close
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape' && !scanning) onClose() }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose, scanning])

  const handleStartScan = () => {
    const s = subnet.trim()
    if (s) startScan.mutate({ subnet: s })
  }

  const handleImport = async (ip: string) => {
    setImportingIp(ip)
    try {
      await importDevice.mutateAsync(ip)
      refetchResults()
    } catch {
      // error handled by mutation state
    } finally {
      setImportingIp(null)
    }
  }

  const handleBackdropClick = () => {
    if (!scanning) onClose()
  }

  const pct = total > 0 ? Math.round((progress / total) * 100) : 0

  const thStyle: React.CSSProperties = {
    textAlign: 'left', padding: '8px 10px', fontSize: 10, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    borderBottom: '1px solid var(--border)',
  }
  const tdStyle: React.CSSProperties = {
    padding: '6px 10px', fontSize: 12, borderBottom: '1px solid var(--border)',
    color: 'var(--fg)',
  }

  return (
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)',
        display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60,
      }}
      onClick={handleBackdropClick}
    >
      <div
        style={{
          background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10,
          width: 640, maxHeight: '85vh', display: 'flex', flexDirection: 'column',
        }}
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{
          display: 'flex', justifyContent: 'space-between', alignItems: 'center',
          padding: '12px 16px', borderBottom: '1px solid var(--border)',
        }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>SNMP Network Scan</div>
          <button
            onClick={onClose}
            disabled={scanning}
            style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18 }}
          >&times;</button>
        </div>

        {/* Body */}
        <div style={{ padding: 16, overflow: 'auto', flex: 1, display: 'flex', flexDirection: 'column', gap: 12 }}>
          {/* Scan inputs */}
          <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}>
            <label style={{ ...labelStyle, flex: 2 }}>
              <span style={labelTextStyle}>Subnet (CIDR)</span>
              <input
                type="text"
                value={subnet}
                onChange={e => setSubnet(e.target.value)}
                placeholder="192.168.1.0/24"
                style={inputStyle}
                disabled={scanning}
              />
            </label>
            <label style={{ ...labelStyle, flex: 1 }}>
              <span style={labelTextStyle}>Community</span>
              <input
                type="text"
                value={community}
                onChange={e => setCommunity(e.target.value)}
                placeholder="public"
                style={inputStyle}
                disabled={scanning}
              />
            </label>
            <button
              onClick={handleStartScan}
              disabled={scanning || startScan.isPending}
              style={{
                padding: '7px 16px', borderRadius: 6, border: '1px solid var(--cyan)',
                background: scanning || startScan.isPending ? 'transparent' : 'var(--cyan)',
                color: scanning || startScan.isPending ? 'var(--muted)' : '#fff',
                cursor: scanning || startScan.isPending ? 'not-allowed' : 'pointer',
                fontSize: 13, fontWeight: 600, whiteSpace: 'nowrap',
                marginBottom: 1,
              }}
            >
              {startScan.isPending ? 'Starting...' : scanning ? 'Scanning...' : 'Start Scan'}
            </button>
          </div>

          {/* Error */}
          {startScan.isError && (
            <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
              {startScan.error.message}
            </div>
          )}

          {/* Progress bar */}
          {scanning && (
            <div style={{ padding: '8px 12px', borderRadius: 6, background: 'var(--bg)', border: '1px solid var(--border)' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
                <span style={{ fontSize: 11, color: 'var(--muted)' }}>Scanning network...</span>
                <span style={{ fontSize: 11, fontFamily: "var(--font-mono)", color: 'var(--cyan)' }}>
                  {progress}/{total} ({pct}%)
                </span>
              </div>
              <div style={{ width: '100%', height: 6, background: 'var(--border)', borderRadius: 3, overflow: 'hidden' }}>
                <div style={{
                  width: `${pct}%`, height: '100%', borderRadius: 3,
                  background: 'var(--cyan)', transition: 'width 0.3s ease',
                }} />
              </div>
            </div>
          )}

          {/* Results table */}
          {results && results.length > 0 && (
            <div style={{ borderRadius: 6, border: '1px solid var(--border)', overflow: 'hidden' }}>
              <div style={{ overflowX: 'auto' }}>
                <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                  <thead>
                    <tr>
                      <th style={thStyle}>IP Address</th>
                      <th style={thStyle}>Name</th>
                      <th style={thStyle}>Vendor</th>
                      <th style={thStyle}>Model</th>
                      <th style={thStyle}>Firmware</th>
                      <th style={{ ...thStyle, width: 70, textAlign: 'center' }}>Import</th>
                    </tr>
                  </thead>
                  <tbody>
                    {results.map(r => (
                      <tr key={r.ip}>
                        <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontWeight: 600 }}>{r.ip}</td>
                        <td style={tdStyle}>{r.sys_name || '--'}</td>
                        <td style={tdStyle}>{r.vendor || '--'}</td>
                        <td style={tdStyle}>{r.model || '--'}</td>
                        <td style={tdStyle}>{r.firmware || '--'}</td>
                        <td style={{ ...tdStyle, textAlign: 'center' }}>
                          <button
                            onClick={() => handleImport(r.ip)}
                            disabled={importingIp === r.ip || importDevice.isPending}
                            style={{
                              padding: '3px 10px', borderRadius: 4, fontSize: 10, fontWeight: 600,
                              border: '1px solid var(--cyan)', background: 'transparent',
                              color: 'var(--cyan)', cursor: importingIp === r.ip ? 'wait' : 'pointer',
                              opacity: importDevice.isPending && importingIp !== r.ip ? 0.5 : 1,
                            }}
                          >
                            {importingIp === r.ip ? '...' : 'Import'}
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {/* Import error */}
          {importDevice.isError && (
            <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
              Import failed: {importDevice.error.message}
            </div>
          )}

          {/* No results */}
          {!scanning && results && results.length === 0 && !startScan.isPending && (
            <div style={{ padding: 20, textAlign: 'center', color: 'var(--muted)', fontSize: 13 }}>
              No scan results yet. Enter a subnet and start a scan.
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
