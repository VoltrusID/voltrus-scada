import { useEffect, useId, useState } from 'react'
import type { FlowField } from '../../types'
import { inputStyle, monoInputStyle } from '../../styles'
import { useSensors, useModbusDevices, useOpcUaDevices, useS7Devices, useEnipDevices } from '../../queries'
import { SEVERITY_OPTIONS } from './catalogue'
import { fmtDuration, switchRules, type SwitchRule } from './model'

const fieldLabel: React.CSSProperties = {
  fontSize: 11, color: 'var(--muted)', fontWeight: 600, display: 'flex', alignItems: 'baseline', gap: 4,
}
const helpStyle: React.CSSProperties = { fontSize: 11, color: 'var(--muted)', lineHeight: 1.4 }
const errStyle: React.CSSProperties = { fontSize: 11, color: 'var(--crit)' }
const iconBtn: React.CSSProperties = {
  background: 'none', border: '1px solid var(--border)', borderRadius: 5, color: 'var(--muted)',
  cursor: 'pointer', width: 24, height: 24, display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
  padding: 0, fontSize: 12, flexShrink: 0,
}

export type PortMap = (oldPort: number) => number | null

interface FormProps {
  fields: FlowField[]
  config: Record<string, unknown>
  errors: Record<string, string>
  readOnly?: boolean
  /** Node type; a `switch` renders its `rules` field as a rule list. */
  nodeType?: string
  onChange: (key: string, value: unknown) => void
  /** Switch rules changed; `map` moves existing wires to their rule's new port. */
  onRules?: (rules: SwitchRule[], map: PortMap) => void
}

/** Config form generated from the catalogue's FIELD definitions. */
export function FieldForm({ fields, config, errors, readOnly, nodeType, onChange, onRules }: FormProps) {
  if (!fields.length) return <div style={helpStyle}>This node has no settings.</div>
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {fields.map(f => (
        nodeType === 'switch' && f.key === 'rules'
          ? <RulesEditor key={f.key} field={f} rules={switchRules(config)} error={errors[f.key]} readOnly={readOnly}
              onChange={(rules, map) => onRules ? onRules(rules, map) : onChange('rules', rules)} />
          : <FieldRow key={f.key} field={f} value={config[f.key]} error={errors[f.key]} readOnly={readOnly}
              onChange={v => onChange(f.key, v)} />
      ))}
    </div>
  )
}

function FieldRow({ field, value, error, readOnly, onChange }: {
  field: FlowField; value: unknown; error?: string; readOnly?: boolean; onChange: (v: unknown) => void
}) {
  const id = useId()
  const errId = `${id}-err`
  if (field.kind === 'bool') {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13, cursor: readOnly ? 'default' : 'pointer' }}>
          <input type="checkbox" checked={!!value} disabled={readOnly} onChange={e => onChange(e.target.checked)} />
          {field.label}
        </label>
        {field.help && <div style={helpStyle}>{field.help}</div>}
      </div>
    )
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }} data-field={field.key}>
      <label htmlFor={id} style={fieldLabel}>
        {field.label}
        {field.required && <span style={{ color: 'var(--crit)' }} aria-hidden>*</span>}
      </label>
      <FieldInput id={id} field={field} value={value} readOnly={readOnly} invalid={!!error} errId={errId} onChange={onChange} />
      {error ? <div id={errId} role="alert" style={errStyle}>{error}</div>
        : field.help ? <div style={helpStyle}>{field.help}</div> : null}
    </div>
  )
}

function FieldInput({ id, field, value, readOnly, invalid, errId, onChange }: {
  id: string; field: FlowField; value: unknown; readOnly?: boolean; invalid: boolean; errId: string; onChange: (v: unknown) => void
}) {
  const base = { ...inputStyle, ...(invalid ? { borderColor: 'var(--crit)' } : {}) }
  const mono = { ...monoInputStyle, ...(invalid ? { borderColor: 'var(--crit)' } : {}) }
  const aria = { id, 'aria-invalid': invalid || undefined, 'aria-describedby': invalid ? errId : undefined, 'aria-required': field.required || undefined }
  const str = value === undefined || value === null ? '' : String(value)

  switch (field.kind) {
    case 'number':
    case 'duration_ms': {
      const num = (s: string) => (s === '' ? undefined : Number(s))
      return (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <input {...aria} type="number" style={{ ...base, flex: 1 }} value={str} readOnly={readOnly}
            onChange={e => onChange(num(e.target.value))} />
          {field.kind === 'duration_ms' && (
            <span style={{ fontSize: 11, color: 'var(--muted)', whiteSpace: 'nowrap', minWidth: 48 }}>
              {typeof value === 'number' ? fmtDuration(value) : 'ms'}
            </span>
          )}
        </div>
      )
    }
    case 'select':
    case 'severity': {
      const opts = field.options?.length ? field.options : field.kind === 'severity' ? SEVERITY_OPTIONS : []
      return (
        <select {...aria} style={base} value={str} disabled={readOnly} onChange={e => onChange(e.target.value)}>
          {!opts.some(o => o.value === str) && <option value={str}>{str || 'Choose…'}</option>}
          {opts.map(o => <option key={o.value} value={o.value}>{o.label}</option>)}
        </select>
      )
    }
    case 'expression':
      return <textarea {...aria} rows={2} spellCheck={false} style={{ ...mono, resize: 'vertical', minHeight: 36 }}
        value={str} readOnly={readOnly} onChange={e => onChange(e.target.value)} />
    case 'code':
      return <textarea {...aria} rows={8} spellCheck={false} style={{ ...mono, resize: 'vertical', minHeight: 120, tabSize: 2 }}
        value={str} readOnly={readOnly} onChange={e => onChange(e.target.value)}
        onKeyDown={e => {
          if (e.key !== 'Tab' || readOnly) return
          e.preventDefault()
          const t = e.currentTarget, s = t.selectionStart, end = t.selectionEnd
          onChange(t.value.slice(0, s) + '  ' + t.value.slice(end))
          requestAnimationFrame(() => { t.selectionStart = t.selectionEnd = s + 2 })
        }} />
    case 'cron':
      return <input {...aria} style={mono} placeholder="*/5 * * * *" value={str} readOnly={readOnly} onChange={e => onChange(e.target.value)} />
    case 'json':
      return <JsonInput aria={aria} style={mono} value={value} readOnly={readOnly} onChange={onChange} />
    case 'tag':
      return <TagPicker aria={aria} style={base} value={value} readOnly={readOnly} onChange={onChange} />
    case 'device':
      return <DevicePicker aria={aria} style={base} value={str} readOnly={readOnly} onChange={onChange} />
    default:
      return <input {...aria} style={base} value={str} readOnly={readOnly} onChange={e => onChange(e.target.value)} />
  }
}

type Aria = Record<string, unknown> & { id: string }

function JsonInput({ aria, style, value, readOnly, onChange }: {
  aria: Aria; style: React.CSSProperties; value: unknown; readOnly?: boolean; onChange: (v: unknown) => void
}) {
  const pretty = (v: unknown) => (v === undefined ? '' : JSON.stringify(v, null, 2))
  const [text, setText] = useState(() => pretty(value))
  const [bad, setBad] = useState(false)
  // Follow outside changes (undo, another node selected) unless mid-edit with bad JSON.
  useEffect(() => {
    let same = false
    try { same = JSON.stringify(JSON.parse(text)) === JSON.stringify(value) } catch { same = false }
    if (!same && !bad) setText(pretty(value))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value])
  return (
    <>
      <textarea {...aria} aria-invalid={bad || aria['aria-invalid'] as boolean | undefined} spellCheck={false} rows={5}
        style={{ ...style, resize: 'vertical', minHeight: 80, ...(bad ? { borderColor: 'var(--crit)' } : {}) }}
        value={text} readOnly={readOnly}
        onChange={e => {
          setText(e.target.value)
          if (e.target.value.trim() === '') { setBad(false); onChange(undefined); return }
          try { onChange(JSON.parse(e.target.value)); setBad(false) } catch { setBad(true) }
        }} />
      {bad && <div role="alert" style={errStyle}>Invalid JSON: not saved until it parses</div>}
    </>
  )
}

function TagPicker({ aria, style, value, readOnly, onChange }: {
  aria: Aria; style: React.CSSProperties; value: unknown; readOnly?: boolean; onChange: (v: unknown) => void
}) {
  const { data: sensors = [], isLoading } = useSensors(true)
  const str = value === undefined || value === null ? '' : String(value)
  const known = sensors.some(s => String(s.id) === str)
  return (
    <select {...aria} style={style} value={str} disabled={readOnly}
      onChange={e => onChange(e.target.value === '' ? undefined : Number(e.target.value))}>
      <option value="">{isLoading ? 'Loading tags…' : 'Choose a tag…'}</option>
      {!known && str && str !== '0' && <option value={str}>#{str} (not found)</option>}
      {sensors.map(s => <option key={s.id} value={String(s.id)}>{s.name}{s.unit ? ` (${s.unit})` : ''} · {s.key}</option>)}
    </select>
  )
}

function DevicePicker({ aria, style, value, readOnly, onChange }: {
  aria: Aria; style: React.CSSProperties; value: string; readOnly?: boolean; onChange: (v: unknown) => void
}) {
  const groups: [string, { id: string | number; name: string }[]][] = [
    ['Modbus', useModbusDevices(true).data ?? []],
    ['OPC UA', useOpcUaDevices().data ?? []],
    ['S7', useS7Devices(true).data ?? []],
    ['EtherNet/IP', useEnipDevices(true).data ?? []],
  ]
  const known = value === '*' || groups.some(([, ds]) => ds.some(d => String(d.id) === value))
  return (
    <select {...aria} style={style} value={value} disabled={readOnly} onChange={e => onChange(e.target.value)}>
      <option value="">Choose a device…</option>
      <option value="*">Any device (*)</option>
      {!known && value && <option value={value}>{value} (not found)</option>}
      {groups.filter(([, ds]) => ds.length).map(([label, ds]) => (
        <optgroup key={label} label={label}>
          {ds.map(d => <option key={`${label}-${d.id}`} value={String(d.id)}>{d.name || d.id}</option>)}
        </optgroup>
      ))}
    </select>
  )
}

/** Switch rules: one output per rule, in order, then "otherwise". */
function RulesEditor({ field, rules, error, readOnly, onChange }: {
  field: FlowField; rules: SwitchRule[]; error?: string; readOnly?: boolean
  onChange: (rules: SwitchRule[], map: PortMap) => void
}) {
  const n = rules.length
  const update = (i: number, patch: Partial<SwitchRule>) =>
    onChange(rules.map((r, j) => (j === i ? { ...r, ...patch } : r)), p => p)
  const add = () => onChange([...rules, { name: `rule ${n + 1}`, expression: '' }], p => (p === n ? n + 1 : p))
  const remove = (i: number) => onChange(rules.filter((_, j) => j !== i), p => (p === i ? null : p > i ? p - 1 : p))
  const move = (i: number, d: -1 | 1) => {
    const j = i + d
    if (j < 0 || j >= n) return
    const next = [...rules]
    ;[next[i], next[j]] = [next[j], next[i]]
    onChange(next, p => (p === i ? j : p === j ? i : p))
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }} data-field="rules">
      <div style={fieldLabel}>
        {field.label || 'Rules'}
        {field.required && <span style={{ color: 'var(--crit)' }} aria-hidden>*</span>}
      </div>
      {rules.map((r, i) => (
        <div key={i} data-testid="rule-row" style={{
          border: '1px solid var(--border)', borderRadius: 8, padding: 8, display: 'flex', flexDirection: 'column', gap: 6,
          background: 'var(--bg)',
        }}>
          <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <span style={{ fontSize: 10, color: 'var(--muted)', fontFamily: 'var(--font-mono)', width: 16 }}>{i + 1}</span>
            <input aria-label={`Rule ${i + 1} name`} style={{ ...inputStyle, flex: 1, padding: '4px 6px' }} value={r.name}
              readOnly={readOnly} onChange={e => update(i, { name: e.target.value })} />
            {!readOnly && <>
              <button type="button" style={iconBtn} title="Move up" aria-label={`Move rule ${i + 1} up`} disabled={i === 0} onClick={() => move(i, -1)}>↑</button>
              <button type="button" style={iconBtn} title="Move down" aria-label={`Move rule ${i + 1} down`} disabled={i === n - 1} onClick={() => move(i, 1)}>↓</button>
              <button type="button" style={{ ...iconBtn, color: 'var(--crit)' }} title="Remove rule" aria-label={`Remove rule ${i + 1}`} onClick={() => remove(i)}>✕</button>
            </>}
          </div>
          <textarea aria-label={`Rule ${i + 1} expression`} rows={1} spellCheck={false} placeholder="payload.value > 80"
            style={{ ...monoInputStyle, resize: 'vertical', ...(r.expression.trim() ? {} : { borderColor: 'var(--warn)' }) }}
            value={r.expression} readOnly={readOnly} onChange={e => update(i, { expression: e.target.value })} />
        </div>
      ))}
      <div style={{ fontSize: 11, color: 'var(--muted)' }}>Messages matching no rule leave on <b>otherwise</b>.</div>
      {!readOnly && (
        <button type="button" onClick={add} style={{
          ...iconBtn, width: '100%', height: 30, borderStyle: 'dashed', fontSize: 12, fontWeight: 600, gap: 6,
        }}>+ Add rule</button>
      )}
      {error ? <div role="alert" style={errStyle}>{error}</div> : field.help ? <div style={helpStyle}>{field.help}</div> : null}
    </div>
  )
}
