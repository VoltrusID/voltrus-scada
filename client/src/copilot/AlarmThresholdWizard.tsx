import { useState } from 'react'
import { useGenerateAlarmThresholds } from '../queries/copilot'
import { useSensors } from '../queries/sensors'

const DEVICE_TYPES = [
  'pump', 'motor', 'vfd', 'power_meter', 'flow_meter', 'temperature_sensor',
  'pressure_sensor', 'level_sensor', 'ahu', 'compressor', 'heat_exchanger',
]

export function AlarmThresholdWizard({ onProposal }: { onProposal: () => void }) {
  const gen = useGenerateAlarmThresholds()
  const { data: sensors } = useSensors(true)
  const [deviceType, setDeviceType] = useState('')
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set())
  const [loading, setLoading] = useState(false)

  const toggle = (key: string) => {
    setSelectedKeys(prev => {
      const next = new Set(prev)
      next.has(key) ? next.delete(key) : next.add(key)
      return next
    })
  }

  const handleGenerate = async () => {
    if (!deviceType || selectedKeys.size === 0) return
    setLoading(true)
    const tags = (sensors || [])
      .filter(s => selectedKeys.has(s.key))
      .map(s => ({ source_key: s.key, name: s.name, unit: s.unit || undefined, min_val: s.min_val || undefined, max_val: s.max_val || undefined }))
    try {
      await gen.mutateAsync({ device_type: deviceType, tags })
      onProposal()
    } finally { setLoading(false) }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
      <h3 style={{ margin: 0, fontSize: 15 }}>Alarm Threshold Suggestions</h3>
      <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>Select tags and device type to get ISA-18.2 alarm threshold recommendations.</p>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Device Type</label>
        <select value={deviceType} onChange={e => setDeviceType(e.target.value)}
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)' }}>
          <option value="">Select device type...</option>
          {DEVICE_TYPES.map(d => <option key={d} value={d}>{d}</option>)}
        </select>
      </div>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Select Tags ({selectedKeys.size} selected)</label>
        <div style={{ maxHeight: 200, overflow: 'auto', border: '1px solid var(--border)', borderRadius: 6 }}>
          {(sensors || []).map(s => (
            <label key={s.key} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px', cursor: 'pointer', fontSize: 13 }}>
              <input type="checkbox" checked={selectedKeys.has(s.key)} onChange={() => toggle(s.key)} />
              <span>{s.name}</span>
              <span style={{ color: 'var(--muted)', fontSize: 11 }}>[{s.key}]</span>
            </label>
          ))}
        </div>
      </div>

      <button onClick={handleGenerate} disabled={!deviceType || selectedKeys.size === 0 || loading}
        style={{ padding: '10px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontWeight: 600, cursor: 'pointer', opacity: (!deviceType || selectedKeys.size === 0) ? 0.5 : 1 }}>
        {loading ? 'Generating...' : 'Generate Thresholds'}
      </button>
    </div>
  )
}
