import React, { useState, useRef, useCallback } from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const SliderWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data, onWriteValue } = props
  const config = instance.config ?? {}
  const showValue = config.showValue !== false
  const step = config.step ?? 1
  const sliderColor = config.sliderColor ?? 'var(--cyan)'

  const min = sensor?.min_val ?? 0
  const max = sensor?.max_val ?? 100
  const decimals = sensor?.decimals ?? 1
  const unit = sensor?.unit ?? ''

  const value = data?.value
  const hasData = value !== undefined && value !== null
  const displayValue = hasData ? value : min

  const [localValue, setLocalValue] = useState<number | null>(null)
  const commitTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)

  const handleChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const v = parseFloat(e.target.value)
    setLocalValue(v)
    if (commitTimer.current) clearTimeout(commitTimer.current)
    commitTimer.current = setTimeout(() => {
      onWriteValue?.(v)
      setLocalValue(null)
    }, 300)
  }, [onWriteValue])

  const range = max - min || 1
  const pct = Math.max(0, Math.min(1, ((localValue ?? displayValue) - min) / range))

  return (
    <div style={{
      display: 'flex', flexDirection: 'column',
      alignItems: 'stretch', justifyContent: 'center',
      width: '100%', height: '100%',
      padding: '4px 10px', gap: 2, overflow: 'hidden',
    }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span style={{ fontSize: 10, color: 'var(--muted)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: '60%' }}>
          {sensor?.name ?? ''}
        </span>
        {showValue && (
          <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--fg)', fontVariantNumeric: 'tabular-nums' }}>
            {(localValue ?? displayValue).toFixed(decimals)}{unit ? ` ${unit}` : ''}
          </span>
        )}
      </div>
      <div style={{ position: 'relative', width: '100%', height: 22 }}>
        <div style={{
          position: 'absolute', left: 0, top: '50%', width: '100%', height: 6,
          transform: 'translateY(-50%)', borderRadius: 3, background: 'var(--border)',
        }} />
        <div style={{
          position: 'absolute', left: 0, top: '50%', width: `${pct * 100}%`, height: 6,
          transform: 'translateY(-50%)', borderRadius: 3, background: sliderColor,
          transition: localValue === null ? 'width 0.3s' : 'none',
        }} />
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          value={localValue ?? displayValue}
          onChange={handleChange}
          style={{
            position: 'absolute', inset: 0, width: '100%', height: '100%',
            opacity: 0, cursor: onWriteValue ? 'pointer' : 'default',
            margin: 0,
          }}
          disabled={!onWriteValue}
        />
        <div style={{
          position: 'absolute', top: '50%', left: `${pct * 100}%`,
          width: 16, height: 16, borderRadius: '50%',
          background: onWriteValue ? sliderColor : 'var(--muted)',
          transform: 'translate(-50%, -50%)',
          transition: localValue === null ? 'left 0.3s' : 'none',
          boxShadow: '0 1px 4px rgba(0,0,0,.3)',
          pointerEvents: 'none',
        }} />
      </div>
      <div style={{ display: 'flex', justifyContent: 'space-between' }}>
        <span style={{ fontSize: 9, color: 'var(--muted)' }}>{min}</span>
        <span style={{ fontSize: 9, color: 'var(--muted)' }}>{max}</span>
      </div>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.slider,
  component: SliderWidget,
})

export default SliderWidget
