import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'
import { valueStyle } from '../../valueStyle'

const BigNumberWidget: React.FC<WidgetProps> = ({ instance, sensor, data }) => {
  const config = instance.config ?? {}
  const fontSize = config.fontSize ?? 48
  const showUnit = config.showUnit !== false
  const showName = config.showName !== false
  const decimals = config.decimals ?? sensor?.decimals ?? 1

  const value = data?.value
  const unit = sensor?.unit ?? ''
  const hasData = value !== undefined && value !== null
  const vs = valueStyle(data)
  const statusColor = vs.color ?? 'var(--ok)'
  const showStatus = vs.state !== 'normal' && vs.state !== 'nodata'

  return (
    <div style={{
      width: '100%', height: '100%',
      display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center',
      padding: 8, boxSizing: 'border-box', overflow: 'hidden',
    }}>
      {showName && sensor?.name && (
        <div style={{
          fontSize: 11, color: 'var(--muted)', marginBottom: 2,
          textTransform: 'uppercase', letterSpacing: '.06em',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          maxWidth: '100%',
        }}>
          {sensor.name}
        </div>
      )}
      <div style={{
        fontSize, fontWeight: 700, color: vs.state === 'normal' ? 'var(--fg)' : vs.color,
        lineHeight: 1, fontFamily: '"JetBrains Mono", monospace',
        display: 'flex', alignItems: 'baseline', gap: 4,
      }}>
        <span>{hasData ? value.toFixed(decimals) : '--'}</span>
        {showUnit && unit && (
          <span style={{ fontSize: fontSize * 0.35, color: 'var(--muted)', fontWeight: 400 }}>
            {unit}
          </span>
        )}
      </div>
      {showStatus && (
        <div style={{
          marginTop: 4, padding: '2px 8px', borderRadius: 4,
          fontSize: 10, fontWeight: 600, color: statusColor,
          background: `color-mix(in srgb, ${statusColor} 12%, transparent)`,
          textTransform: 'uppercase', letterSpacing: '.05em',
        }}>
          {vs.label}
        </div>
      )}
    </div>
  )
}

registerWidget({ manifest: WIDGET_MANIFESTS.big_number, component: BigNumberWidget })
