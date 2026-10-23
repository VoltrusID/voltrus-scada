import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const BarWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const showValue = config.showValue !== false

  const min = sensor?.min_val ?? 0
  const max = sensor?.max_val ?? 100
  const warn = sensor?.warn_val ?? min + (max - min) * 0.75
  const crit = sensor?.crit_val ?? min + (max - min) * 0.9
  const decimals = sensor?.decimals ?? 1
  const unit = sensor?.unit ?? ''

  const value = data?.value ?? min
  const range = max - min || 1
  const pct = Math.max(0, Math.min(1, (value - min) / range))

  const fillColor = pct >= (crit - min) / range
    ? 'var(--crit)'
    : pct >= (warn - min) / range
      ? 'var(--warn)'
      : 'var(--ok)'

  const containerStyle: React.CSSProperties = {
    display: 'flex',
    flexDirection: 'column',
    justifyContent: 'center',
    width: '100%',
    height: '100%',
    padding: '6px 10px',
    boxSizing: 'border-box',
    overflow: 'hidden',
  }

  const trackStyle: React.CSSProperties = {
    position: 'relative',
    width: '100%',
    height: '100%',
    minHeight: 20,
    backgroundColor: 'var(--border)',
    borderRadius: 6,
    overflow: 'hidden',
  }

  const fillStyle: React.CSSProperties = {
    position: 'absolute',
    top: 0,
    left: 0,
    height: '100%',
    width: `${pct * 100}%`,
    backgroundColor: fillColor,
    borderRadius: 6,
    transition: 'width 0.3s ease',
    minWidth: pct > 0 ? 2 : 0,
  }

  const textStyle: React.CSSProperties = {
    position: 'absolute',
    top: '50%',
    left: '50%',
    transform: 'translate(-50%, -50%)',
    fontSize: 13,
    fontWeight: 600,
    color: 'var(--fg)',
    whiteSpace: 'nowrap',
    textShadow: '0 0 4px var(--bg), 0 0 4px var(--bg)',
    pointerEvents: 'none',
  }

  return (
    <div style={containerStyle}>
      <div style={trackStyle}>
        <div style={fillStyle} />
        {showValue && (
          <div style={textStyle}>
            {data?.value !== undefined ? `${value.toFixed(decimals)} ${unit}`.trim() : '--'}
          </div>
        )}
      </div>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.bar,
  component: BarWidget,
})

export default BarWidget
