import { useState } from 'react'
import { useGenerateRegisterMap } from '../queries/copilot'

export function RegisterMapWizard({ onProposal }: { onProposal: () => void }) {
  const gen = useGenerateRegisterMap()
  const [fileContent, setFileContent] = useState('')
  const [deviceName, setDeviceName] = useState('')
  const [deviceId, setDeviceId] = useState('')
  const [fileType, setFileType] = useState('csv')
  const [loading, setLoading] = useState(false)

  const handleFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    const reader = new FileReader()
    reader.onload = ev => setFileContent(ev.target?.result as string || '')
    reader.readAsText(file)
  }

  const handleGenerate = async () => {
    if (!fileContent || !deviceName) return
    setLoading(true)
    try {
      await gen.mutateAsync({
        file_content: fileContent,
        device_name: deviceName,
        file_type: fileType,
        device_id: deviceId || undefined,
      })
      onProposal()
    } finally { setLoading(false) }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
      <h3 style={{ margin: 0, fontSize: 15 }}>Register Map Auto-Mapping</h3>
      <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>Upload a datasheet (CSV/PDF text) to auto-generate Modbus register mappings.</p>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Datasheet File</label>
        <input type="file" accept=".csv,.txt,.pdf" onChange={handleFile}
          style={{ width: '100%', padding: 8, borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', fontSize: 13 }} />
      </div>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Device Name</label>
        <input type="text" value={deviceName} onChange={e => setDeviceName(e.target.value)} placeholder="e.g. Power Meter A1"
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', boxSizing: 'border-box' }} />
      </div>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Device ID (optional)</label>
        <input type="text" value={deviceId} onChange={e => setDeviceId(e.target.value)} placeholder="Existing device ID to link registers"
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', boxSizing: 'border-box' }} />
      </div>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>File Type</label>
        <select value={fileType} onChange={e => setFileType(e.target.value)}
          style={{ padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)' }}>
          <option value="csv">CSV</option>
          <option value="pdf">PDF (extracted text)</option>
          <option value="text">Plain text</option>
        </select>
      </div>

      {fileContent && (
        <details>
          <summary style={{ fontSize: 12, color: 'var(--muted)', cursor: 'pointer' }}>Preview extracted text ({fileContent.length} chars)</summary>
          <pre style={{ maxHeight: 120, overflow: 'auto', fontSize: 11, background: 'var(--bg)', padding: 8, borderRadius: 6, border: '1px solid var(--border)' }}>
            {fileContent.slice(0, 2000)}
          </pre>
        </details>
      )}

      <button onClick={handleGenerate} disabled={!fileContent || !deviceName || loading}
        style={{ padding: '10px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontWeight: 600, cursor: 'pointer', opacity: (!fileContent || !deviceName) ? 0.5 : 1 }}>
        {loading ? 'Generating...' : 'Generate Register Map'}
      </button>
    </div>
  )
}
