import { useState } from 'react'
import { useGenerateDashboardLayout } from '../queries/copilot'

export function DashboardLayoutWizard({ onProposal }: { onProposal: () => void }) {
  const gen = useGenerateDashboardLayout()
  const [description, setDescription] = useState('')
  const [screenId, setScreenId] = useState('')
  const [loading, setLoading] = useState(false)

  const handleGenerate = async () => {
    if (!description) return
    setLoading(true)
    try {
      await gen.mutateAsync({ description, screen_id: screenId || undefined })
      onProposal()
    } finally { setLoading(false) }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
      <h3 style={{ margin: 0, fontSize: 15 }}>Dashboard Layout Generator</h3>
      <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>Describe your plant or process in natural language to generate a dashboard layout.</p>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Description</label>
        <textarea value={description} onChange={e => setDescription(e.target.value)}
          placeholder="e.g. A water treatment plant with 3 pumps, 2 pressure sensors, flow meters on inlet/outlet, and a chemical dosing system..."
          rows={5} style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', fontSize: 13, boxSizing: 'border-box', resize: 'vertical' }} />
      </div>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Screen ID (optional)</label>
        <input type="text" value={screenId} onChange={e => setScreenId(e.target.value)} placeholder="Target screen to apply layout"
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', boxSizing: 'border-box' }} />
      </div>

      <button onClick={handleGenerate} disabled={!description || loading}
        style={{ padding: '10px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontWeight: 600, cursor: 'pointer', opacity: !description ? 0.5 : 1 }}>
        {loading ? 'Generating...' : 'Generate Layout'}
      </button>
    </div>
  )
}
