import { useState } from 'react'
import { triggerExport, type ExportParams } from './queries/export'

const AGGREGATIONS = ['none', 'avg', 'min', 'max', 'count', 'sum'] as const
const INTERVALS = ['1m', '5m', '15m', '1h', '1d'] as const

interface Props {
  onClose: () => void
}

export function DataExportPanel({ onClose }: Props) {
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [device, setDevice] = useState('')
  const [metric, setMetric] = useState('')
  const [aggregation, setAggregation] = useState('none')
  const [interval, setInterval_] = useState('5m')
  const [format, setFormat] = useState<'json' | 'csv'>('json')
  const [limit, setLimit] = useState(1000)
  const [exporting, setExporting] = useState(false)
  const [error, setError] = useState('')

  const showInterval = aggregation !== 'none'

  const handleExport = async () => {
    setError('')
    setExporting(true)
    try {
      const params: ExportParams = {
        from: from || undefined,
        to: to || undefined,
        device: device || undefined,
        metric: metric || undefined,
        aggregation: aggregation !== 'none' ? aggregation : undefined,
        interval: showInterval ? interval : undefined,
        format,
        limit,
      }
      await triggerExport(params)
      onClose()
    } catch (e: any) {
      setError(e.message || 'Export failed')
    } finally {
      setExporting(false)
    }
  }

  const inputStyle: React.CSSProperties = {
    padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)',
    background: 'var(--card)', color: 'var(--fg)', fontSize: 13, width: '100%', boxSizing: 'border-box',
  }

  const selectStyle: React.CSSProperties = {
    ...inputStyle, cursor: 'pointer', appearance: 'auto',
  }

  const labelStyle: React.CSSProperties = {
    fontSize: 12, fontWeight: 600, color: 'var(--muted)', marginBottom: 4, display: 'block',
  }

  return (
    <div
      style={{ position: 'fixed', inset: 0, zIndex: 1000, display: 'flex', alignItems: 'center', justifyContent: 'center' }}
      onClick={onClose}
    >
      <div style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,.5)' }} />
      <div
        style={{
          position: 'relative', maxWidth: 480, width: '90%', background: 'var(--bg)',
          borderRadius: 12, border: '1px solid var(--border)', padding: 24,
          maxHeight: '90vh', overflow: 'auto',
        }}
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 20 }}>
          <span style={{ fontSize: 16, fontWeight: 700 }}>Export Data</span>
          <button
            onClick={onClose}
            style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 20, padding: '0 4px', lineHeight: 1 }}
          >&times;</button>
        </div>

        {/* Form */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
          {/* Date range */}
          <div style={{ display: 'flex', gap: 10 }}>
            <div style={{ flex: 1 }}>
              <label style={labelStyle}>From (ISO 8601)</label>
              <input
                style={inputStyle}
                placeholder="2024-01-01T00:00:00Z"
                value={from}
                onChange={e => setFrom(e.target.value)}
              />
            </div>
            <div style={{ flex: 1 }}>
              <label style={labelStyle}>To (ISO 8601)</label>
              <input
                style={inputStyle}
                placeholder="2024-12-31T23:59:59Z"
                value={to}
                onChange={e => setTo(e.target.value)}
              />
            </div>
          </div>

          {/* Device */}
          <div>
            <label style={labelStyle}>Device ID</label>
            <input
              style={inputStyle}
              placeholder="device-001"
              value={device}
              onChange={e => setDevice(e.target.value)}
            />
          </div>

          {/* Metric */}
          <div>
            <label style={labelStyle}>Metric</label>
            <input
              style={inputStyle}
              placeholder="temperature"
              value={metric}
              onChange={e => setMetric(e.target.value)}
            />
          </div>

          {/* Aggregation */}
          <div>
            <label style={labelStyle}>Aggregation</label>
            <select style={selectStyle} value={aggregation} onChange={e => setAggregation(e.target.value)}>
              {AGGREGATIONS.map(a => (
                <option key={a} value={a}>{a === 'none' ? 'None' : a.charAt(0).toUpperCase() + a.slice(1)}</option>
              ))}
            </select>
          </div>

          {/* Interval (conditional) */}
          {showInterval && (
            <div>
              <label style={labelStyle}>Interval</label>
              <select style={selectStyle} value={interval} onChange={e => setInterval_(e.target.value)}>
                {INTERVALS.map(i => (
                  <option key={i} value={i}>{i}</option>
                ))}
              </select>
            </div>
          )}

          {/* Format toggle */}
          <div>
            <label style={labelStyle}>Format</label>
            <div style={{ display: 'flex', borderRadius: 6, overflow: 'hidden', border: '1px solid var(--border)' }}>
              {(['json', 'csv'] as const).map(f => (
                <button
                  key={f}
                  onClick={() => setFormat(f)}
                  style={{
                    flex: 1, padding: '8px 0', fontSize: 13, fontWeight: 600, cursor: 'pointer',
                    border: 'none', background: format === f ? 'rgba(34,197,94,.1)' : 'var(--card)',
                    color: format === f ? 'var(--cyan)' : 'var(--fg)',
                  }}
                >
                  {f.toUpperCase()}
                </button>
              ))}
            </div>
          </div>

          {/* Limit */}
          <div>
            <label style={labelStyle}>Limit</label>
            <input
              type="number"
              style={inputStyle}
              min={1}
              max={10000}
              value={limit}
              onChange={e => setLimit(Math.min(10000, Math.max(1, Number(e.target.value))))}
            />
          </div>

          {/* Error */}
          {error && (
            <div style={{ fontSize: 12, color: 'var(--crit)', background: 'rgba(239,68,68,.1)', padding: '8px 12px', borderRadius: 6 }}>
              {error}
            </div>
          )}

          {/* Export button */}
          <button
            onClick={handleExport}
            disabled={exporting}
            style={{
              padding: '10px 16px', borderRadius: 6, border: 'none',
              background: 'var(--cyan)', color: '#fff', fontWeight: 600,
              cursor: exporting ? 'wait' : 'pointer', fontSize: 14,
              opacity: exporting ? 0.7 : 1,
            }}
          >
            {exporting ? 'Exporting...' : 'Export'}
          </button>
        </div>
      </div>
    </div>
  )
}
