import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const IndicatorWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const fontSize = config.fontSize ?? 28
  const showUnit = config.showUnit !== false
  const showStatus = config.showStatus !== false

  const decimals = sensor?.decimals ?? 1
  const value = data?.value
  const unit = sensor?.unit ?? ''
  const status = data?.status ?? ''

  const hasValue = value !== undefined && value !== null

  const displayValue = hasValue ? value.toFixed(decimals) : '--'

  const statusColor = status === 'crit'
    ? 'var(--crit)'
    : status === 'warn'
      ? 'var(--warn)'
      : 'var(--ok)'

  const statusLabel = status === 'crit'
    ? 'CRIT'
    : status === 'warn'
      ? 'WARN'
      : 'OK'

  const containerStyle: React.CSSProperties = {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    width: '100%',
    height: '100%',
    padding: 8,
    boxSizing: 'border-box',
    overflow: 'hidden',
  }

  const valueStyle: React.CSSProperties = {
    fontSize,
    fontWeight: 700,
    color: 'var(--fg)',
    lineHeight: 1.2,
    whiteSpace: 'nowrap',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    maxWidth: '100%',
  }

  const unitStyle: React.CSSProperties = {
    fontSize: Math.max(10, fontSize * 0.4),
    color: 'var(--muted)',
    marginTop: 2,
    whiteSpace: 'nowrap',
  }

  const badgeStyle: React.CSSProperties = {
    display: 'inline-block',
    padding: '2px 8px',
    borderRadius: 10,
    fontSize: 10,
    fontWeight: 700,
    letterSpacing: 0.5,
    color: 'var(--bg)',
    backgroundColor: statusColor,
    marginTop: 4,
    textTransform: 'uppercase',
  }

  return (
    <div style={containerStyle}>
      <div style={valueStyle}>{displayValue}</div>
      {showUnit && unit && <div style={unitStyle}>{unit}</div>}
      {showStatus && hasValue && <span style={badgeStyle}>{statusLabel}</span>}
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.indicator,
  component: IndicatorWidget,
})

export default IndicatorWidget
