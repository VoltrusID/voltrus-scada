import { useEffect, useState } from 'react'
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle } from '../styles'
import {
  HISTORY_DEADBAND_HELP, applyHistoryDeadband, historyDeadbandWarnings, readHistoryDeadband,
  validateHistoryDeadband, type HistoryDeadbandForm,
} from '../historyDeadband'

const helpStyle: React.CSSProperties = { fontSize: 11, color: 'var(--muted)', lineHeight: 1.4, marginTop: 6 }
const errorStyle: React.CSSProperties = { fontSize: 11, color: 'var(--crit)', marginTop: 4 }
const warnStyle: React.CSSProperties = { fontSize: 11, color: 'var(--warn)', marginTop: 4 }

/** The deadband inputs without a surrounding config object: `form` in, `onChange` out. */
export function HistoryDeadbandInputs({ form, onChange }: { form: HistoryDeadbandForm; onChange: (f: HistoryDeadbandForm) => void }) {
  const errors = validateHistoryDeadband(form)
  const warnings = historyDeadbandWarnings(form)
  const invalid = (bad: boolean): React.CSSProperties => (bad ? { borderColor: 'var(--crit)' } : {})
  const valueBad = errors.some(e => e.startsWith('Deadband') || e.startsWith('A percent'))
  const beatBad = errors.some(e => e.startsWith('Heartbeat'))
  return (
    <div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        <label style={labelStyle}>
          <span style={labelTextStyle}>history band</span>
          <input
            type="number" step="any" min="0" value={form.value} placeholder="off"
            aria-label="History deadband value" aria-invalid={valueBad}
            onChange={e => onChange({ ...form, value: e.target.value })}
            style={{ ...monoInputStyle, ...invalid(valueBad) }}
          />
        </label>
        <label style={labelStyle}>
          <span style={labelTextStyle}>band type</span>
          <select
            value={form.type} aria-label="History deadband type"
            onChange={e => onChange({ ...form, type: e.target.value === 'percent' ? 'percent' : 'absolute' })}
            style={{ ...inputStyle, cursor: 'pointer' }}
          >
            <option value="absolute">absolute (tag units)</option>
            <option value="percent">percent of last value</option>
          </select>
        </label>
        <label style={labelStyle}>
          <span style={labelTextStyle}>heartbeat (s)</span>
          <input
            type="number" step="1" min="1" value={form.maxInterval} placeholder="none"
            aria-label="History heartbeat seconds" aria-invalid={beatBad}
            onChange={e => onChange({ ...form, maxInterval: e.target.value })}
            style={{ ...monoInputStyle, ...invalid(beatBad) }}
          />
        </label>
      </div>
      {errors.map(e => <div key={e} role="alert" style={errorStyle}>{e}</div>)}
      {warnings.map(w => <div key={w} style={warnStyle}>{w}</div>)}
      <div style={helpStyle}>{HISTORY_DEADBAND_HELP}</div>
    </div>
  )
}

/**
 * Edits the history deadband keys inside a source_config object. Valid edits are written
 * through `onChange`; invalid ones are not, and `onValidityChange` reports the problems
 * so the caller can hold back its Save button.
 */
export function HistoryDeadbandFields({ config, onChange, onValidityChange }: {
  config: Record<string, any>
  onChange: (c: Record<string, any>) => void
  onValidityChange?: (errors: string[]) => void
}) {
  const [form, setForm] = useState(() => readHistoryDeadband(config))
  const errors = validateHistoryDeadband(form)
  const errorKey = errors.join('|')

  useEffect(() => { onValidityChange?.(errors) }, [errorKey]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => () => onValidityChange?.([]), []) // eslint-disable-line react-hooks/exhaustive-deps

  const update = (f: HistoryDeadbandForm) => {
    setForm(f)
    if (validateHistoryDeadband(f).length === 0) onChange(applyHistoryDeadband(config, f))
  }
  return <HistoryDeadbandInputs form={form} onChange={update} />
}
