import { useState } from 'react'
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle } from '../styles'
import { updateWritePolicy } from '../api/commands'
import type { WritePolicy } from '../api/commands'

interface Props {
  tagId: number
  currentPolicy?: WritePolicy
  tagName?: string
  isAdmin: boolean
  onSave?: () => void
  onClose: () => void
}

export function WritePolicyEditor({ tagId, tagName, currentPolicy, isAdmin, onSave, onClose }: Props) {
  const [policy, setPolicy] = useState<WritePolicy>(currentPolicy ?? {
    writable: false,
    write_min: null,
    write_max: null,
    write_max_rate: null,
    write_requires_reason: false,
    write_deadband: null,
  })
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState(false)
  const [confirmEnable, setConfirmEnable] = useState(false)

  if (!isAdmin) return null

  const handleToggleWritable = () => {
    if (!policy.writable && !confirmEnable) {
      setConfirmEnable(true)
      return
    }
    setConfirmEnable(false)
    setPolicy(p => ({ ...p, writable: !p.writable }))
    setSuccess(false)
  }

  const handleFieldChange = (field: keyof WritePolicy, value: number | boolean | null) => {
    setPolicy(p => ({ ...p, [field]: value }))
    setSuccess(false)
  }

  const handleSave = async () => {
    setError('')
    setSaving(true)
    try {
      await updateWritePolicy(tagId, policy)
      setSuccess(true)
      onSave?.()
    } catch (e: any) {
      setError(e.message || 'Failed to save write policy')
    } finally {
      setSaving(false)
    }
  }

  const handleBackdropClick = () => {
    if (!saving) onClose()
  }

  return (
    <div
      style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }}
      onClick={handleBackdropClick}
    >
      <div
        style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 460, maxHeight: '85vh', display: 'flex', flexDirection: 'column' }}
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '12px 16px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <span style={{ fontSize: 13, fontWeight: 600 }}>Write Policy</span>
            {tagName && <span style={{ fontSize: 11, color: 'var(--muted)' }}>{tagName}</span>}
          </div>
          <button onClick={onClose} disabled={saving} aria-label="Close write policy dialog" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18 }}>&times;</button>
        </div>

        {/* Body */}
        <div style={{ padding: 16, overflow: 'auto', flex: 1, display: 'flex', flexDirection: 'column', gap: 12 }}>
          {/* Writable toggle */}
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
              <span style={{ fontSize: 12, fontWeight: 600 }}>Allow writes to this tag</span>
              <span style={{ fontSize: 11, color: policy.writable ? 'var(--ok)' : 'var(--muted)' }}>
                {policy.writable ? 'Enabled' : 'Disabled'}
              </span>
            </div>
            <button
              onClick={handleToggleWritable}
              aria-label="Toggle writable"
              style={{
                width: 40, height: 22, borderRadius: 11,
                border: 'none', cursor: 'pointer', position: 'relative',
                background: policy.writable ? 'var(--ok)' : 'var(--border)',
                transition: 'background 0.2s',
              }}
            >
              <div style={{
                width: 16, height: 16, borderRadius: '50%', background: '#fff',
                position: 'absolute', top: 3,
                left: policy.writable ? 21 : 3,
                transition: 'left 0.2s',
              }} />
            </button>
          </div>

          {/* Confirmation warning */}
          {confirmEnable && (
            <div style={{
              padding: '10px 12px', borderRadius: 6,
              background: 'rgba(234,179,8,.1)', border: '1px solid var(--warn)',
              display: 'flex', flexDirection: 'column', gap: 8,
            }}>
              <div style={{ fontSize: 12, color: 'var(--warn)', fontWeight: 600 }}>
                Enabling writes allows operators to modify device values. Are you sure?
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <button
                  onClick={() => { setConfirmEnable(false); setPolicy(p => ({ ...p, writable: true })); setSuccess(false) }}
                  style={{
                    padding: '4px 12px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                    border: '1px solid var(--warn)', background: 'transparent', color: 'var(--warn)', fontWeight: 600,
                  }}
                >
                  Yes, enable writes
                </button>
                <button
                  onClick={() => setConfirmEnable(false)}
                  style={{
                    padding: '4px 12px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                    border: '1px solid var(--border)', background: 'transparent', color: 'var(--muted)',
                  }}
                >
                  Cancel
                </button>
              </div>
            </div>
          )}

          {/* Additional fields when writable */}
          {policy.writable && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10, paddingLeft: 8, borderLeft: '2px solid var(--ok)' }}>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                <label style={labelStyle}>
                  <span style={labelTextStyle}>Min Value</span>
                  <input
                    type="number"
                    step="any"
                    value={policy.write_min ?? ''}
                    onChange={e => handleFieldChange('write_min', e.target.value === '' ? null : parseFloat(e.target.value))}
                    placeholder="No limit"
                    style={monoInputStyle}
                  />
                </label>
                <label style={labelStyle}>
                  <span style={labelTextStyle}>Max Value</span>
                  <input
                    type="number"
                    step="any"
                    value={policy.write_max ?? ''}
                    onChange={e => handleFieldChange('write_max', e.target.value === '' ? null : parseFloat(e.target.value))}
                    placeholder="No limit"
                    style={monoInputStyle}
                  />
                </label>
              </div>

              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                <label style={labelStyle}>
                  <span style={labelTextStyle}>Max Writes / Min</span>
                  <input
                    type="number"
                    step="1"
                    min="1"
                    value={policy.write_max_rate ?? ''}
                    onChange={e => handleFieldChange('write_max_rate', e.target.value === '' ? null : parseInt(e.target.value))}
                    placeholder="No limit"
                    style={monoInputStyle}
                  />
                </label>
                <label style={labelStyle}>
                  <span style={labelTextStyle}>Write Deadband</span>
                  <input
                    type="number"
                    step="any"
                    min="0"
                    value={policy.write_deadband ?? ''}
                    onChange={e => handleFieldChange('write_deadband', e.target.value === '' ? null : parseFloat(e.target.value))}
                    placeholder="No deadband"
                    style={monoInputStyle}
                  />
                </label>
              </div>

              <label style={{ ...labelStyle, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
                <input
                  type="checkbox"
                  checked={policy.write_requires_reason}
                  onChange={e => handleFieldChange('write_requires_reason', e.target.checked)}
                  style={{ width: 16, height: 16, cursor: 'pointer' }}
                />
                <span style={{ fontSize: 12, color: 'var(--fg)' }}>Require reason for writes</span>
              </label>
            </div>
          )}

          {/* Error */}
          {error && (
            <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
              {error}
            </div>
          )}

          {/* Success */}
          {success && (
            <div style={{ fontSize: 12, color: 'var(--ok)', padding: '6px 10px', background: 'rgba(34,197,94,.1)', borderRadius: 4 }}>
              Write policy saved successfully.
            </div>
          )}

          {/* Save button */}
          <button
            onClick={handleSave}
            disabled={saving}
            style={{
              padding: '8px 16px', borderRadius: 6,
              border: `1px solid ${saving ? 'var(--border)' : 'var(--cyan)'}`,
              background: saving ? 'transparent' : 'var(--cyan)',
              color: saving ? 'var(--muted)' : '#fff',
              cursor: saving ? 'not-allowed' : 'pointer',
              fontSize: 13, fontWeight: 600, opacity: saving ? 0.6 : 1,
            }}
          >
            {saving ? 'Saving...' : 'Save Policy'}
          </button>
        </div>
      </div>
    </div>
  )
}
