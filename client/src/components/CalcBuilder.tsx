import { useState, useMemo } from 'react'

export interface CalcBuilderProps {
  metrics: Array<{
    id: number
    name: string
    description: string
    formula: string
    unit: string | null
    validation_error: string | null
  }>
  values: Array<{ id: number; name: string; value: number; unit: string | null }> | null
  onCreate: (input: { name: string; description: string; formula: string; unit?: string }) => void
  onUpdate: (id: number, input: { name?: string; description?: string; formula?: string; unit?: string }) => void
  onDelete: (id: number) => void
}

const inputStyle: React.CSSProperties = {
  background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6, padding: '6px 8px',
  color: 'var(--fg)', fontSize: 13, fontFamily: 'inherit', width: '100%', boxSizing: 'border-box',
}

const monoInputStyle: React.CSSProperties = { ...inputStyle, fontFamily: 'var(--font-mono)' }

const btnStyle: React.CSSProperties = {
  fontFamily: 'var(--font-ui)', fontSize: 13, fontWeight: 500, padding: '7px 14px', borderRadius: 6,
  border: '1px solid var(--border)', background: 'var(--card)', color: 'var(--fg)', cursor: 'pointer',
  display: 'flex', alignItems: 'center', gap: 6,
}

const primaryBtnStyle: React.CSSProperties = {
  ...btnStyle, background: 'var(--cyan)', color: '#000', border: 'none', fontWeight: 600,
}

/** Safe arithmetic-only evaluator — supports +, -, *, /, %, parentheses, and numbers. */
function safeMathEval(expr: string): number | null {
  // Only allow digits, whitespace, operators, parentheses, and dots
  if (!/^[\d\s+\-*/().%]+$/.test(expr)) return null
  try {
    // Tokenize and evaluate using a simple recursive descent parser
    let pos = 0
    const s = expr.replace(/\s+/g, '')
    if (s.length === 0) return null

    function parseExpr(): number | null {
      let left = parseTerm()
      if (left === null) return null
      while (pos < s.length && (s[pos] === '+' || s[pos] === '-')) {
        const op = s[pos++]
        const right = parseTerm()
        if (right === null) return null
        left = op === '+' ? left + right : left - right
      }
      return left
    }
    function parseTerm(): number | null {
      let left = parseFactor()
      if (left === null) return null
      while (pos < s.length && (s[pos] === '*' || s[pos] === '/' || s[pos] === '%')) {
        const op = s[pos++]
        const right = parseFactor()
        if (right === null) return null
        if (op === '*') left = left * right
        else if (op === '/') { if (right === 0) return null; left = left / right }
        else left = left % right
      }
      return left
    }
    function parseFactor(): number | null {
      if (pos < s.length && s[pos] === '(') {
        pos++ // skip (
        const val = parseExpr()
        if (val === null || pos >= s.length || s[pos] !== ')') return null
        pos++ // skip )
        return val
      }
      // Unary minus
      if (pos < s.length && s[pos] === '-') {
        pos++
        const val = parseFactor()
        return val !== null ? -val : null
      }
      if (pos < s.length && s[pos] === '+') {
        pos++
        return parseFactor()
      }
      // Number
      const start = pos
      while (pos < s.length && ((s[pos] >= '0' && s[pos] <= '9') || s[pos] === '.')) pos++
      if (pos === start) return null
      return Number(s.slice(start, pos))
    }

    const result = parseExpr()
    if (result === null || pos !== s.length) return null
    return Number.isFinite(result) ? result : null
  } catch {
    return null
  }
}

function evaluateFormula(formula: string, values: Map<string, number>): string | null {
  if (!formula.trim()) return null
  let expr = formula
  // Replace variable names (longest first to avoid partial matches)
  const names = Array.from(values.keys()).sort((a, b) => b.length - a.length)
  for (const name of names) {
    expr = expr.replace(new RegExp(`\\b${name}\\b`, 'g'), String(values.get(name)!))
  }
  const result = safeMathEval(expr)
  return result !== null ? result.toFixed(4) : null
}

export function CalcBuilder({ metrics, values, onCreate, onUpdate, onDelete }: CalcBuilderProps) {
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [formula, setFormula] = useState('')
  const [unit, setUnit] = useState('')
  const [editingId, setEditingId] = useState<number | null>(null)

  const valueMap = useMemo(() => {
    const m = new Map<string, number>()
    if (values) for (const v of values) m.set(v.name, v.value)
    return m
  }, [values])

  const preview = evaluateFormula(formula, valueMap)

  const handleSubmit = () => {
    if (!name.trim() || !formula.trim()) return
    const input = { name: name.trim(), description: description.trim(), formula: formula.trim(), unit: unit.trim() || undefined }
    if (editingId != null) {
      onUpdate(editingId, input)
      setEditingId(null)
    } else {
      onCreate(input)
    }
    setName(''); setDescription(''); setFormula(''); setUnit('')
  }

  const startEdit = (m: typeof metrics[0]) => {
    setEditingId(m.id); setName(m.name); setDescription(m.description); setFormula(m.formula); setUnit(m.unit || '')
  }

  const cancelEdit = () => {
    setEditingId(null); setName(''); setDescription(''); setFormula(''); setUnit('')
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 12 }}>
      {/* Formula editor */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Formula</span>
          <input style={monoInputStyle} value={formula} onChange={e => setFormula(e.target.value)} placeholder="e.g. (temp * 1.8) + 32" />
        </label>
        {preview != null && (
          <div style={{ fontSize: 12, color: 'var(--cyan)', fontFamily: 'var(--font-mono)', padding: '4px 8px', background: 'rgba(0,212,170,0.08)', borderRadius: 4 }}>
            Preview: {preview}
          </div>
        )}
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 80px', gap: 8 }}>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Name</span>
            <input style={inputStyle} value={name} onChange={e => setName(e.target.value)} placeholder="Metric name" />
          </label>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Unit</span>
            <input style={inputStyle} value={unit} onChange={e => setUnit(e.target.value)} placeholder="e.g. C" />
          </label>
        </div>
        <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Description</span>
          <input style={inputStyle} value={description} onChange={e => setDescription(e.target.value)} placeholder="Optional description" />
        </label>
        <div style={{ display: 'flex', gap: 8 }}>
          <button style={primaryBtnStyle} onClick={handleSubmit} disabled={!name.trim() || !formula.trim()}>
            {editingId != null ? 'Update' : 'Create'}
          </button>
          {editingId != null && (
            <button style={btnStyle} onClick={cancelEdit}>Cancel</button>
          )}
        </div>
      </div>

      {/* Available variables */}
      {values && values.length > 0 && (
        <div style={{ borderTop: '1px solid var(--border)', paddingTop: 8 }}>
          <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>Available Variables</div>
          <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
            {values.map(v => (
              <span key={v.id} style={{ fontSize: 11, fontFamily: 'var(--font-mono)', color: 'var(--fg)', padding: '2px 6px', background: 'var(--card)', borderRadius: 4, border: '1px solid var(--border)' }}>
                {v.name} = {v.value}
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Existing metrics list */}
      {metrics.length > 0 && (
        <div style={{ borderTop: '1px solid var(--border)', paddingTop: 8 }}>
          <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 6 }}>Calculated Metrics</div>
          {metrics.map(m => (
            <div key={m.id} style={{
              display: 'flex', alignItems: 'center', gap: 8, padding: '8px 10px', marginBottom: 4,
              background: 'var(--card)', borderRadius: 6, border: '1px solid var(--border)',
            }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--fg)' }}>
                  {m.name}
                  {m.unit && <span style={{ fontSize: 11, color: 'var(--muted)', marginLeft: 6 }}>{m.unit}</span>}
                </div>
                <div style={{ fontSize: 11, fontFamily: 'var(--font-mono)', color: 'var(--cyan)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {m.formula}
                </div>
                {m.validation_error && (
                  <div style={{ fontSize: 11, color: '#ef4444', marginTop: 2 }}>{m.validation_error}</div>
                )}
              </div>
              <button style={{ ...btnStyle, padding: '4px 8px', fontSize: 11 }} onClick={() => startEdit(m)}>Edit</button>
              <button style={{ ...btnStyle, padding: '4px 8px', fontSize: 11, color: '#ef4444' }} onClick={() => onDelete(m.id)}>Del</button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
