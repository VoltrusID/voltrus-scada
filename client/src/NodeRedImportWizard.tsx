import { useState, useCallback, useRef } from 'react'
import { useImportNodeRed } from './queries'
import type { NodeRedImportResult } from './queries'
import { inputStyle, btnStyle } from './styles'

export function NodeRedImportWizard({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const [file, setFile] = useState<File | null>(null)
  const [result, setResult] = useState<NodeRedImportResult | null>(null)
  const [step, setStep] = useState<'upload' | 'preview' | 'report'>('upload')
  const [isDragging, setIsDragging] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const importMut = useImportNodeRed()

  const handleFile = useCallback((f: File) => {
    if (f.type !== 'application/json' && !f.name.endsWith('.json')) return
    setFile(f)
    importMut.mutate(f, {
      onSuccess: (res) => {
        setResult(res)
        setStep('preview')
      },
    })
  }, [importMut])

  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(true)
  }, [])

  const onDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)
  }, [])

  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)
    const f = e.dataTransfer.files[0]
    if (f) handleFile(f)
  }, [handleFile])

  const onFileChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0]
    if (f) handleFile(f)
  }, [handleFile])

  const handleReset = () => {
    setFile(null)
    setResult(null)
    setStep('upload')
    if (fileInputRef.current) fileInputRef.current.value = ''
  }

  const cardStyle: React.CSSProperties = {
    background: 'var(--card)',
    border: '1px solid var(--border)',
    borderRadius: 10,
    padding: 24,
  }

  const statCard = (label: string, value: number, color: string): React.ReactNode => (
    <div style={{ ...cardStyle, flex: 1, textAlign: 'center' }}>
      <div style={{ fontSize: 28, fontWeight: 700, color }}>{value}</div>
      <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginTop: 4 }}>{label}</div>
    </div>
  )

  const thStyle: React.CSSProperties = {
    textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    borderBottom: '1px solid var(--border)',
  }
  const tdStyle: React.CSSProperties = {
    padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)',
  }

  const licenseOk = result ? (!result.license.limit || result.license.current_devices + result.license.new_devices <= result.license.limit) : true

  const content = (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {step === 'upload' && (
        <>
          <div
            onDragOver={onDragOver}
            onDragLeave={onDragLeave}
            onDrop={onDrop}
            onClick={() => fileInputRef.current?.click()}
            style={{
              ...cardStyle,
              border: `2px dashed ${isDragging ? 'var(--cyan)' : 'var(--border)'}`,
              textAlign: 'center',
              cursor: 'pointer',
              transition: 'border-color 0.2s',
            }}
          >
            <input
              ref={fileInputRef}
              type="file"
              accept=".json,application/json"
              onChange={onFileChange}
              style={{ display: 'none' }}
            />
            <div style={{ fontSize: 36, marginBottom: 12, color: 'var(--muted)' }}><svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/></svg></div>
            <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--fg)' }}>
              {isDragging ? 'Drop your flows.json here' : 'Drag & drop a Node-RED flows.json file'}
            </div>
            <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 6 }}>
              or click to browse (accepts .json)
            </div>
          </div>

          {importMut.isPending && (
            <div style={{ textAlign: 'center', padding: 24, color: 'var(--muted)' }}>
              <div style={{ marginBottom: 8 }}>Parsing and converting flows...</div>
              <div style={{ fontSize: 12 }}>This may take a moment for large flows</div>
            </div>
          )}

          {importMut.error && (
            <div style={{ ...cardStyle, borderColor: 'var(--crit)', color: 'var(--crit)' }}>
              <div style={{ fontWeight: 600, marginBottom: 4 }}>Import failed</div>
              <div style={{ fontSize: 13 }}>{importMut.error.message}</div>
            </div>
          )}
        </>
      )}

      {step === 'preview' && result && (
        <>
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
            {statCard('Total Nodes', result.parse.total_nodes, 'var(--fg)')}
            {statCard('Convertible', result.parse.convertible, 'var(--green)')}
            {statCard('Manual Review', result.parse.manual_review, 'var(--warn)')}
            {statCard('Unsupported', result.parse.unsupported, 'var(--crit)')}
          </div>

          {!licenseOk && (
            <div style={{
              ...cardStyle,
              borderColor: 'var(--warn)',
              background: 'rgba(234,179,8,.08)',
              display: 'flex',
              alignItems: 'center',
              gap: 12,
            }}>
              <div style={{ fontSize: 20 }}>{'\u26A0\uFE0F'}</div>
              <div>
                <div style={{ fontWeight: 600, fontSize: 13 }}>License Limit Warning</div>
                <div style={{ fontSize: 12, color: 'var(--muted)' }}>
                  Current devices: {result.license.current_devices} + new devices: {result.license.new_devices}
                  {result.license.limit ? ` exceeds limit of ${result.license.limit}` : ''}
                </div>
              </div>
            </div>
          )}

          {result.warnings.length > 0 && (
            <div style={{ ...cardStyle, borderColor: 'var(--warn)' }}>
              <div style={{ fontWeight: 600, fontSize: 13, marginBottom: 8 }}>Warnings ({result.warnings.length})</div>
              <ul style={{ margin: 0, paddingLeft: 18, fontSize: 12, color: 'var(--muted)', display: 'flex', flexDirection: 'column', gap: 4 }}>
                {result.warnings.map((w, i) => (
                  <li key={i}>{w}</li>
                ))}
              </ul>
            </div>
          )}

          <div style={cardStyle}>
            <div style={{ fontWeight: 600, fontSize: 14, marginBottom: 12 }}>Conversion Preview</div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))', gap: 12, marginBottom: 16 }}>
              {[
                { label: 'Modbus Devices', value: result.conversion.modbus_devices },
                { label: 'MQTT Sources', value: result.conversion.mqtt_sources },
                { label: 'Widgets', value: result.conversion.widgets },
                { label: 'Flow Nodes', value: result.conversion.flow_nodes },
                { label: 'Function Nodes', value: result.conversion.function_nodes },
                { label: 'Skipped', value: result.conversion.skipped_nodes },
              ].map(s => (
                <div key={s.label} style={{ textAlign: 'center', padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                  <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--cyan)' }}>{s.value}</div>
                  <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 4 }}>{s.label}</div>
                </div>
              ))}
            </div>

            {result.details.modbus_devices.length > 0 && (
              <div style={{ marginBottom: 16 }}>
                <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--muted)', textTransform: 'uppercase' }}>Modbus Devices</div>
                <div style={{ overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr>
                        <th style={thStyle}>Name</th>
                        <th style={thStyle}>IP</th>
                        <th style={thStyle}>Port</th>
                        <th style={thStyle}>Unit ID</th>
                        <th style={thStyle}>Tags</th>
                      </tr>
                    </thead>
                    <tbody>
                      {result.details.modbus_devices.map(d => (
                        <tr key={d.device_id}>
                          <td style={tdStyle}>{d.name}</td>
                          <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)', fontSize: 12 }}>{d.ip}</td>
                          <td style={tdStyle}>{d.port}</td>
                          <td style={tdStyle}>{d.unit_id}</td>
                          <td style={tdStyle}>{d.tags.length}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {result.details.mqtt_sources.length > 0 && (
              <div style={{ marginBottom: 16 }}>
                <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--muted)', textTransform: 'uppercase' }}>MQTT Sources</div>
                <div style={{ overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr>
                        <th style={thStyle}>Name</th>
                        <th style={thStyle}>Topic</th>
                        <th style={thStyle}>QoS</th>
                        <th style={thStyle}>Direction</th>
                      </tr>
                    </thead>
                    <tbody>
                      {result.details.mqtt_sources.map((s, i) => (
                        <tr key={i}>
                          <td style={tdStyle}>{s.name}</td>
                          <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)', fontSize: 12 }}>{s.topic}</td>
                          <td style={tdStyle}>{s.qos}</td>
                          <td style={tdStyle}>{s.direction}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {result.details.widgets.length > 0 && (
              <div style={{ marginBottom: 16 }}>
                <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--muted)', textTransform: 'uppercase' }}>Widgets</div>
                <div style={{ overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr>
                        <th style={thStyle}>Type</th>
                        <th style={thStyle}>Name</th>
                        <th style={thStyle}>Label</th>
                      </tr>
                    </thead>
                    <tbody>
                      {result.details.widgets.map((w, i) => (
                        <tr key={i}>
                          <td style={tdStyle}>{w.widget_type}</td>
                          <td style={tdStyle}>{w.name}</td>
                          <td style={tdStyle}>{w.label || '-'}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {result.details.function_nodes.length > 0 && (
              <div style={{ marginBottom: 16 }}>
                <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--muted)', textTransform: 'uppercase' }}>Function Nodes</div>
                <div style={{ overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr>
                        <th style={thStyle}>Name</th>
                        <th style={thStyle}>Warning</th>
                      </tr>
                    </thead>
                    <tbody>
                      {result.details.function_nodes.map((f, i) => (
                        <tr key={i}>
                          <td style={tdStyle}>{f.name}</td>
                          <td style={{ ...tdStyle, color: 'var(--warn)' }}>{f.warning}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {result.details.skipped_nodes.length > 0 && (
              <div>
                <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--muted)', textTransform: 'uppercase' }}>Skipped Nodes</div>
                <div style={{ overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr>
                        <th style={thStyle}>Type</th>
                        <th style={thStyle}>Name</th>
                        <th style={thStyle}>Reason</th>
                      </tr>
                    </thead>
                    <tbody>
                      {result.details.skipped_nodes.map((s, i) => (
                        <tr key={i}>
                          <td style={tdStyle}>{s.node_type}</td>
                          <td style={tdStyle}>{s.name || '-'}</td>
                          <td style={{ ...tdStyle, color: 'var(--crit)' }}>{s.reason}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}
          </div>

          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button style={btnStyle} onClick={handleReset}>Import Another</button>
            <button style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)' }} onClick={() => setStep('report')}>
              View Summary Report
            </button>
          </div>
        </>
      )}

      {step === 'report' && result && (
        <>
          <div style={cardStyle}>
            <div style={{ fontWeight: 700, fontSize: 16, marginBottom: 16 }}>Import Summary Report</div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginBottom: 16 }}>
              <div style={{ padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>File</div>
                <div style={{ fontSize: 13, fontWeight: 500 }}>{file?.name || '-'}</div>
              </div>
              <div style={{ padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>Total Nodes</div>
                <div style={{ fontSize: 13, fontWeight: 500 }}>{result.parse.total_nodes}</div>
              </div>
              <div style={{ padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>Converted</div>
                <div style={{ fontSize: 13, fontWeight: 500, color: 'var(--green)' }}>
                  {result.parse.convertible} nodes
                </div>
              </div>
              <div style={{ padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>Skipped</div>
                <div style={{ fontSize: 13, fontWeight: 500, color: 'var(--crit)' }}>
                  {result.parse.unsupported} nodes
                </div>
              </div>
            </div>

            <div style={{ marginBottom: 16 }}>
              <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--muted)', textTransform: 'uppercase' }}>Conversion Results</div>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
                {[
                  { label: 'Modbus Devices', value: result.conversion.modbus_devices },
                  { label: 'MQTT Sources', value: result.conversion.mqtt_sources },
                  { label: 'Widgets', value: result.conversion.widgets },
                  { label: 'Flow Nodes', value: result.conversion.flow_nodes },
                  { label: 'Function Nodes', value: result.conversion.function_nodes },
                  { label: 'Skipped Nodes', value: result.conversion.skipped_nodes },
                ].map(s => (
                  <div key={s.label} style={{ display: 'flex', justifyContent: 'space-between', padding: '8px 12px', background: 'var(--bg)', borderRadius: 6 }}>
                    <span style={{ fontSize: 12, color: 'var(--muted)' }}>{s.label}</span>
                    <span style={{ fontSize: 12, fontWeight: 600 }}>{s.value}</span>
                  </div>
                ))}
              </div>
            </div>

            {result.warnings.length > 0 && (
              <div style={{ marginBottom: 16 }}>
                <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--muted)', textTransform: 'uppercase' }}>Warnings</div>
                <ul style={{ margin: 0, paddingLeft: 18, fontSize: 12, color: 'var(--muted)', display: 'flex', flexDirection: 'column', gap: 4 }}>
                  {result.warnings.map((w, i) => (
                    <li key={i}>{w}</li>
                  ))}
                </ul>
              </div>
            )}

            <div>
              <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 8, color: 'var(--muted)', textTransform: 'uppercase' }}>License</div>
              <div style={{ display: 'flex', gap: 12 }}>
                <div style={{ flex: 1, padding: 8, background: 'var(--bg)', borderRadius: 6 }}>
                  <span style={{ fontSize: 11, color: 'var(--muted)' }}>Current Devices: </span>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>{result.license.current_devices}</span>
                </div>
                <div style={{ flex: 1, padding: 8, background: 'var(--bg)', borderRadius: 6 }}>
                  <span style={{ fontSize: 11, color: 'var(--muted)' }}>New Devices: </span>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>{result.license.new_devices}</span>
                </div>
                <div style={{ flex: 1, padding: 8, background: 'var(--bg)', borderRadius: 6 }}>
                  <span style={{ fontSize: 11, color: 'var(--muted)' }}>Limit: </span>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>{result.license.limit ?? 'Unlimited'}</span>
                </div>
              </div>
            </div>
          </div>

          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button style={btnStyle} onClick={() => setStep('preview')}>Back to Preview</button>
            <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} onClick={handleReset}>Import Another File</button>
          </div>
        </>
      )}
    </div>
  )

  if (embedded) {
    return <>{content}</>
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column', padding: 24 }}>
      <div style={{ maxWidth: 1200, width: '100%', margin: '0 auto', display: 'flex', flexDirection: 'column', height: '100%' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <button onClick={onClose} aria-label="Close Node-RED import wizard" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
            </button>
            <h1 style={{ fontSize: 20, fontWeight: 700 }}>Node-RED Import Wizard</h1>
          </div>
        </div>
        <div style={{ flex: 1, overflow: 'auto' }}>
          {content}
        </div>
      </div>
    </div>
  )
}
