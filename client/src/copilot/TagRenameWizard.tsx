import { useState } from 'react'
import { useGenerateTagRename } from '../queries/copilot'
import { useSensors } from '../queries/sensors'

const CONVENTIONS = ['ISA-5.1', 'IEC-61131', 'KKS', 'descriptive', 'custom']

export function TagRenameWizard({ onProposal }: { onProposal: () => void }) {
  const gen = useGenerateTagRename()
  const { data: sensors } = useSensors(true)
  const [convention, setConvention] = useState('ISA-5.1')
  const [customPattern, setCustomPattern] = useState('')
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set())
  const [loading, setLoading] = useState(false)

  const toggle = (key: string) => {
    setSelectedKeys(prev => {
      const next = new Set(prev)
      next.has(key) ? next.delete(key) : next.add(key)
      return next
    })
  }
  const selectAll = () => setSelectedKeys(new Set((sensors || []).map(s => s.key)))
  const selectNone = () => setSelectedKeys(new Set())

  const handleGenerate = async () => {
    if (selectedKeys.size === 0) return
    setLoading(true)
    const tags = (sensors || []).filter(s => selectedKeys.has(s.key)).map(s => ({ current: s.name }))
    try {
      await gen.mutateAsync({ tags, convention, custom_pattern: convention === 'custom' ? customPattern : undefined })
      onProposal()
    } finally { setLoading(false) }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
      <h3 style={{ margin: 0, fontSize: 15 }}>Tag Naming Conversion</h3>
      <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>Batch-rename tags to follow a naming convention. Review the diff before applying.</p>

      <div>
        <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Naming Convention</label>
        <select value={convention} onChange={e => setConvention(e.target.value)}
          style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)' }}>
          {CONVENTIONS.map(c => <option key={c} value={c}>{c}</option>)}
        </select>
      </div>

      {convention === 'custom' && (
        <div>
          <label style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4, display: 'block' }}>Custom Pattern</label>
          <input type="text" value={customPattern} onChange={e => setCustomPattern(e.target.value)} placeholder="e.g. {area}-{equipment}-{measurement}"
            style={{ width: '100%', padding: '8px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', boxSizing: 'border-box' }} />
        </div>
      )}

      <div>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 4 }}>
          <label style={{ fontSize: 12, color: 'var(--muted)' }}>Select Tags ({selectedKeys.size})</label>
          <div style={{ display: 'flex', gap: 8 }}>
            <button onClick={selectAll} style={{ background: 'none', border: 'none', color: 'var(--primary)', cursor: 'pointer', fontSize: 12 }}>All</button>
            <button onClick={selectNone} style={{ background: 'none', border: 'none', color: 'var(--primary)', cursor: 'pointer', fontSize: 12 }}>None</button>
          </div>
        </div>
        <div style={{ maxHeight: 200, overflow: 'auto', border: '1px solid var(--border)', borderRadius: 6 }}>
          {(sensors || []).map(s => (
            <label key={s.key} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px', cursor: 'pointer', fontSize: 13 }}>
              <input type="checkbox" checked={selectedKeys.has(s.key)} onChange={() => toggle(s.key)} />
              <span>{s.name}</span>
            </label>
          ))}
        </div>
      </div>

      <button onClick={handleGenerate} disabled={selectedKeys.size === 0 || loading}
        style={{ padding: '10px', borderRadius: 6, border: 'none', background: 'var(--primary)', color: '#fff', fontWeight: 600, cursor: 'pointer', opacity: selectedKeys.size === 0 ? 0.5 : 1 }}>
        {loading ? 'Generating...' : 'Generate Rename Mapping'}
      </button>
    </div>
  )
}
