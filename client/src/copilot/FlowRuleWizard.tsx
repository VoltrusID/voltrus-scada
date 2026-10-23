import { useState } from 'react'
import { useGenerateFlowRule } from '../queries/copilot'
import { useSensors } from '../queries/sensors'

export function FlowRuleWizard({ onProposal }: { onProposal: () => void }) {
  const gen = useGenerateFlowRule()
  const { data: sensors } = useSensors(true)
  const [description, setDescription] = useState('')
  const [loading, setLoading] = useState(false)

  const tagKeys = (sensors || []).map(s => s.key)

  const handleGenerate = async () => {
    if (!description) return
    setLoading(true)
    try {
      await gen.mutateAsync({ description, available_tags: tagKeys })
      onProposal()
    } finally { setLoading(false) }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
      <h3 style={{ margin: 0, fontSize: 15 }}>Flow Rule Generator</h3>
      <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>Describe the automation rule in natural language. All available tags are sent as context.</p>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Rule Description</label>
        <textarea value={description} onChange={e => setDescription(e.target.value)}
          placeholder="e.g. When pump_1_pressure exceeds 80 psi, trigger an alarm and send an email notification. If flow_rate drops below 10 gpm for 5 minutes, start backup_pump_2."
          rows={5} style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', fontSize: 13, boxSizing: 'border-box', resize: 'vertical' }} />
      </div>

      <details>
        <summary style={{ fontSize: 12, color: 'var(--muted)', cursor: 'pointer' }}>Available tags ({tagKeys.length})</summary>
        <div style={{ maxHeight: 100, overflow: 'auto', fontSize: 12, color: 'var(--muted)', padding: '4px 0' }}>
          {tagKeys.map(k => <div key={k}>{k}</div>)}
        </div>
      </details>

      <button onClick={handleGenerate} disabled={!description || loading}
        style={{ padding: '10px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontWeight: 600, cursor: 'pointer', opacity: !description ? 0.5 : 1 }}>
        {loading ? 'Generating...' : 'Generate Flow Rule'}
      </button>
    </div>
  )
}
