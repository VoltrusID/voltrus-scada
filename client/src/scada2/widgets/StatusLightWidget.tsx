import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const StatusLightWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const onThreshold = config.onThreshold ?? 0.5

  const value = data?.value
  const status = data?.status ?? ''
  const isDigital = sensor?.source_type === 'digital'
  const hasData = value !== undefined && value !== null

  let color: string
  if (!hasData) {
    color = 'var(--muted)'
  } else if (isDigital) {
    color = value > onThreshold ? 'var(--ok)' : 'var(--muted)'
  } else {
    color = status === 'crit' ? 'var(--crit)' : status === 'warn' ? 'var(--warn)' : 'var(--ok)'
  }

  const size = Math.min(instance.width, instance.height) * 0.6

  const containerStyle: React.CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: '100%',
    height: '100%',
  }

  const circleStyle: React.CSSProperties = {
    width: size,
    height: size,
    borderRadius: '50%',
    backgroundColor: color,
    boxShadow: hasData
      ? `0 0 ${size * 0.4}px ${size * 0.15}px ${color}`
      : 'none',
    transition: 'background-color 0.3s ease, box-shadow 0.3s ease',
  }

  return (
    <div style={containerStyle}>
      <div style={circleStyle} />
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.status_light,
  component: StatusLightWidget,
})

export default StatusLightWidget
