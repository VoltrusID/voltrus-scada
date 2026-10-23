import { useState } from 'react'
import { useAiTemplates, useGenerateFromTemplate } from '../queries/copilot'

export function TemplateWizard({ onProposal }: { onProposal: () => void }) {
  const [industry, setIndustry] = useState<string>('')
  const { data: templates } = useAiTemplates(industry || undefined)
  const gen = useGenerateFromTemplate()
  const [deviceId, setDeviceId] = useState('')
  const [prefix, setPrefix] = useState('')
  const [loading, setLoading] = useState(false)

  const industries = [...new Set((templates || []).map(t => t.industry))]

  const handleInstantiate = async (templateId: number) => {
    setLoading(true)
    try {
      await gen.mutateAsync({ template_id: templateId, device_id: deviceId || undefined, prefix: prefix || undefined })
      onProposal()
    } finally { setLoading(false) }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
      <h3 style={{ margin: 0, fontSize: 15 }}>Equipment Template Wizard</h3>
      <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>Pre-built equipment configurations. Select a template to generate a proposal.</p>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Filter by Industry</label>
        <select value={industry} onChange={e => setIndustry(e.target.value)}
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)' }}>
          <option value="">All industries</option>
          {industries.map(i => <option key={i} value={i}>{i}</option>)}
        </select>
      </div>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Device ID (optional)</label>
        <input type="text" value={deviceId} onChange={e => setDeviceId(e.target.value)} placeholder="Link to an existing device"
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', boxSizing: 'border-box' }} />
      </div>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Tag Prefix (optional)</label>
        <input type="text" value={prefix} onChange={e => setPrefix(e.target.value)} placeholder="e.g. PU-101"
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', boxSizing: 'border-box' }} />
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        {(templates || []).map(t => (
          <div key={t.id} style={{ border: '1px solid var(--border)', borderRadius: 6, padding: 12 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <div>
                <div style={{ fontWeight: 600, fontSize: 14 }}>{t.name}</div>
                <div style={{ fontSize: 12, color: 'var(--muted)' }}>{t.industry} &middot; {t.equipment_type}</div>
              </div>
              <button onClick={() => handleInstantiate(t.id)} disabled={loading}
                style={{ padding: '6px 12px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontSize: 12, cursor: 'pointer' }}>
                Use
              </button>
            </div>
          </div>
        ))}
        {templates && templates.length === 0 && <div style={{ color: 'var(--muted)', fontSize: 13, textAlign: 'center', padding: 16 }}>No templates found</div>}
      </div>
    </div>
  )
}
