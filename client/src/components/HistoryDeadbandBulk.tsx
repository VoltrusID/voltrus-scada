import { useState } from 'react'
import { btnStyle } from '../styles'
import { EMPTY_FORM, validateHistoryDeadband, type HistoryDeadbandForm } from '../historyDeadband'
import { HistoryDeadbandInputs } from './HistoryDeadbandFields'

/**
 * Bulk bar of the tag list: set or clear the history deadband of every selected tag.
 * `onApply(null)` removes the history keys, `onApply(form)` sets them; other source_config
 * keys of each tag are kept by the caller.
 */
export function HistoryDeadbandBulk({ count, busy, onApply, onClearSelection }: {
  count: number
  busy: boolean
  onApply: (form: HistoryDeadbandForm | null) => void
  onClearSelection: () => void
}) {
  const [open, setOpen] = useState(false)
  const [form, setForm] = useState<HistoryDeadbandForm>({ ...EMPTY_FORM })
  const errors = validateHistoryDeadband(form)
  const canApply = !busy && errors.length === 0 && form.value.trim() !== ''
  const plural = count === 1 ? 'tag' : 'tags'

  return (
    <div style={{ background: 'var(--card)', border: '1px solid var(--cyan)', borderRadius: 8, padding: 10, marginBottom: 12, flexShrink: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <span style={{ fontSize: 13, fontWeight: 600 }}>{count} {plural} selected</span>
        <button style={btnStyle} onClick={() => setOpen(o => !o)} aria-expanded={open}>
          {open ? 'Hide history deadband' : 'History deadband...'}
        </button>
        <button
          style={{ ...btnStyle, borderColor: 'var(--crit)', color: 'var(--crit)' }}
          disabled={busy}
          onClick={() => { if (confirm(`Remove the history deadband from ${count} ${plural}? They will store every sample again.`)) onApply(null) }}
        >
          Remove deadband
        </button>
        <button style={{ ...btnStyle, marginLeft: 'auto' }} onClick={onClearSelection}>Clear selection</button>
      </div>
      {open && (
        <div style={{ marginTop: 10 }}>
          <HistoryDeadbandInputs form={form} onChange={setForm} />
          <button
            style={{ ...btnStyle, marginTop: 8, borderColor: 'var(--cyan)', color: 'var(--cyan)', opacity: canApply ? 1 : 0.5, cursor: canApply ? 'pointer' : 'not-allowed' }}
            disabled={!canApply}
            onClick={() => onApply(form)}
          >
            {busy ? 'Applying...' : `Apply to ${count} ${plural}`}
          </button>
        </div>
      )}
    </div>
  )
}
