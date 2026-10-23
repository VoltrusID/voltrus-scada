import React, { useState, useCallback, useRef } from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const NumberInputWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data, onWriteValue } = props
  const config = instance.config ?? {}
  const showButtons = config.showButtons !== false
  const step = config.step ?? 1

  const min = sensor?.min_val ?? -Infinity
  const max = sensor?.max_val ?? Infinity
  const decimals = sensor?.decimals ?? 1
  const unit = sensor?.unit ?? ''

  const value = data?.value
  const hasData = value !== undefined && value !== null
  const displayValue = hasData ? value : 0

  const [editing, setEditing] = useState(false)
  const [inputVal, setInputVal] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)

  const commitValue = useCallback((v: number) => {
    const clamped = Math.max(min === -Infinity ? -1e9 : min, Math.min(max === Infinity ? 1e9 : max, v))
    onWriteValue?.(clamped)
  }, [onWriteValue, min, max])

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      const v = parseFloat(inputVal)
      if (!isNaN(v)) commitValue(v)
      setEditing(false)
    } else if (e.key === 'Escape') {
      setEditing(false)
    }
  }, [inputVal, commitValue])

  const handleBlur = useCallback(() => {
    const v = parseFloat(inputVal)
    if (!isNaN(v)) commitValue(v)
    setEditing(false)
  }, [inputVal, commitValue])

  const handleStep = useCallback((dir: -1 | 1) => {
    const current = hasData ? value! : 0
    commitValue(current + dir * step)
  }, [value, step, commitValue, hasData])

  const inputStyle: React.CSSProperties = {
    width: '100%',
    background: 'var(--bg)',
    border: '1px solid var(--cyan)',
    borderRadius: 4,
    color: 'var(--fg)',
    fontSize: 18,
    fontWeight: 700,
    textAlign: 'center',
    padding: '4px 6px',
    outline: 'none',
    fontVariantNumeric: 'tabular-nums',
  }

  return (
    <div style={{
      display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center',
      width: '100%', height: '100%',
      gap: 4, padding: '4px 6px', overflow: 'hidden',
    }}>
      <span style={{ fontSize: 10, color: 'var(--muted)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: '100%' }}>
        {sensor?.name ?? ''}
      </span>
      {editing ? (
        <input
          ref={inputRef}
          type="number"
          value={inputVal}
          onChange={e => setInputVal(e.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={handleBlur}
          onFocus={e => e.target.select()}
          style={inputStyle}
          step={step}
          min={min === -Infinity ? undefined : min}
          max={max === Infinity ? undefined : max}
          autoFocus
        />
      ) : (
        <div
          onClick={() => {
            if (!onWriteValue) return
            setInputVal(String(hasData ? value! : 0))
            setEditing(true)
            setTimeout(() => inputRef.current?.focus(), 0)
          }}
          style={{
            fontSize: 22,
            fontWeight: 700,
            color: hasData ? 'var(--fg)' : 'var(--muted)',
            cursor: onWriteValue ? 'pointer' : 'default',
            fontVariantNumeric: 'tabular-nums',
            lineHeight: 1.2,
            borderBottom: onWriteValue ? '1px dashed var(--border)' : 'none',
            padding: '0 4px',
            minWidth: 60,
            textAlign: 'center',
          }}
        >
          {hasData ? displayValue.toFixed(decimals) : '--'}
          {unit && <span style={{ fontSize: 12, fontWeight: 400, color: 'var(--muted)', marginLeft: 3 }}>{unit}</span>}
        </div>
      )}
      {showButtons && onWriteValue && (
        <div style={{ display: 'flex', gap: 4 }}>
          <button
            onClick={() => handleStep(-1)}
            style={{
              width: 28, height: 24, borderRadius: 4,
              border: '1px solid var(--border)', background: 'var(--card)',
              color: 'var(--fg)', cursor: 'pointer', fontSize: 14, fontWeight: 700,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}
          >−</button>
          <button
            onClick={() => handleStep(1)}
            style={{
              width: 28, height: 24, borderRadius: 4,
              border: '1px solid var(--border)', background: 'var(--card)',
              color: 'var(--fg)', cursor: 'pointer', fontSize: 14, fontWeight: 700,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}
          >+</button>
        </div>
      )}
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.number_input,
  component: NumberInputWidget,
})

export default NumberInputWidget
