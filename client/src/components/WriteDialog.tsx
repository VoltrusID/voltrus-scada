import { useState } from 'react'
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle } from '../styles'
import type { WritePolicy } from '../api/commands'
import { writeTag } from '../api/commands'

interface Props {
  tagId: number
  tagName: string
  currentValue: number | null
  policy: WritePolicy
  onClose: () => void
}

export function WriteDialog({ tagId, tagName, currentValue, policy, onClose }: Props) {
  const [value, setValue] = useState('')
  const [reason, setReason] = useState('')
  const [writePolicy, setWritePolicy] = useState<'immediate' | 'queued'>('immediate')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const [success, setSuccess] = useState<{ command_id: string } | null>(null)

  const numValue = parseFloat(value)
  const hasMin = policy.write_min != null
  const hasMax = policy.write_max != null
  const minError = hasMin && !isNaN(numValue) && numValue < policy.write_min!
  const maxError = hasMax && !isNaN(numValue) && numValue > policy.write_max!
  const reasonMissing = policy.write_requires_reason && !reason.trim()
  const canSubmit = !isNaN(numValue) && !minError && !maxError && !reasonMissing && !loading && !success

  const handleSubmit = async () => {
    setError('')
    setLoading(true)
    try {
      const result = await writeTag(tagId, numValue, writePolicy, reason.trim() || undefined)
      setSuccess({ command_id: result.command_id })
    } catch (e: any) {
      setError(e.message || 'Write failed')
    } finally {
      setLoading(false)
    }
  }

  const handleBackdropClick = () => {
    if (!loading) onClose()
  }

  return (
    <div
      style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }}
      onClick={handleBackdropClick}
    >
      <div
        style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 440, maxHeight: '85vh', display: 'flex', flexDirection: 'column' }}
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '12px 16px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Write Value</div>
          <button onClick={onClose} disabled={loading} style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18 }}>&times;</button>
        </div>

        {/* Body */}
        <div style={{ padding: 16, overflow: 'auto', flex: 1, display: 'flex', flexDirection: 'column', gap: 12 }}>
          {/* Tag info */}
          <div style={{ padding: '8px 12px', borderRadius: 6, background: 'var(--bg)', border: '1px solid var(--border)' }}>
            <div style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 4 }}>Tag</div>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <span style={{ fontSize: 14, fontWeight: 600 }}>{tagName}</span>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 13, color: 'var(--cyan)' }}>
                {currentValue != null ? currentValue.toFixed(2) : '--'}
              </span>
            </div>
          </div>

          {success ? (
            /* Success state */
            <div style={{ padding: '16px 12px', borderRadius: 6, background: 'rgba(34,197,94,.1)', border: '1px solid var(--ok)', textAlign: 'center' }}>
              <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--ok)', marginBottom: 8 }}>Command submitted</div>
              <div style={{ fontSize: 11, color: 'var(--muted)', fontFamily: "var(--font-mono)", wordBreak: 'break-all' }}>
                {success.command_id}
              </div>
              <button
                onClick={onClose}
                style={{
                  marginTop: 12, padding: '8px 20px', borderRadius: 6, border: '1px solid var(--border)',
                  background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 13, fontWeight: 600,
                }}
              >
                Close
              </button>
            </div>
          ) : (
            <>
              {/* Value input */}
              <label style={labelStyle}>
                <span style={labelTextStyle}>
                  New Value
                  {hasMin || hasMax ? (
                    <span style={{ marginLeft: 6, color: 'var(--muted)', textTransform: 'none', fontWeight: 400 }}>
                      ({hasMin ? `min: ${policy.write_min}` : ''}{hasMin && hasMax ? ', ' : ''}{hasMax ? `max: ${policy.write_max}` : ''})
                    </span>
                  ) : null}
                </span>
                <input
                  type="number"
                  step="any"
                  value={value}
                  onChange={e => { setValue(e.target.value); setError('') }}
                  placeholder={currentValue != null ? String(currentValue) : '0'}
                  style={monoInputStyle}
                  autoFocus
                />
              </label>

              {/* Validation errors */}
              {(minError || maxError) && (
                <div style={{ fontSize: 11, color: 'var(--crit)', padding: '0 4px' }}>
                  {minError && `Value below minimum (${policy.write_min})`}
                  {minError && maxError && '. '}
                  {maxError && `Value above maximum (${policy.write_max})`}
                </div>
              )}

              {/* Reason textarea */}
              {policy.write_requires_reason && (
                <label style={labelStyle}>
                  <span style={labelTextStyle}>Reason (required)</span>
                  <textarea
                    value={reason}
                    onChange={e => { setReason(e.target.value); setError('') }}
                    placeholder="Enter reason for this write..."
                    rows={3}
                    style={{
                      ...inputStyle,
                      resize: 'vertical',
                      minHeight: 60,
                      fontFamily: 'inherit',
                    }}
                  />
                </label>
              )}

              {/* Policy selector */}
              <label style={labelStyle}>
                <span style={labelTextStyle}>Policy</span>
                <select
                  value={writePolicy}
                  onChange={e => setWritePolicy(e.target.value as 'immediate' | 'queued')}
                  style={inputStyle}
                >
                  <option value="immediate">Immediate</option>
                  <option value="queued">Queued</option>
                </select>
              </label>

              {/* Error */}
              {error && (
                <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
                  {error}
                </div>
              )}

              {/* Submit */}
              <button
                onClick={handleSubmit}
                disabled={!canSubmit}
                style={{
                  padding: '10px 16px', borderRadius: 6,
                  border: `1px solid ${canSubmit ? 'var(--cyan)' : 'var(--border)'}`,
                  background: canSubmit ? 'var(--cyan)' : 'transparent',
                  color: canSubmit ? '#fff' : 'var(--muted)',
                  cursor: canSubmit ? 'pointer' : 'not-allowed',
                  fontSize: 13, fontWeight: 600, opacity: loading ? 0.6 : 1,
                }}
              >
                {loading ? 'Submitting...' : 'Write Value'}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
