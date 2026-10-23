import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const OnOffWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const onThreshold = config.onThreshold ?? 0.5
  const onLabel = config.onLabel ?? 'ON'
  const offLabel = config.offLabel ?? 'OFF'

  const value = data?.value
  const hasData = value !== undefined && value !== null
  const isOn = hasData && value > onThreshold

  const containerStyle: React.CSSProperties = {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    width: '100%',
    height: '100%',
    gap: 6,
    overflow: 'hidden',
  }

  const trackStyle: React.CSSProperties = {
    position: 'relative',
    width: 48,
    height: 26,
    borderRadius: 13,
    backgroundColor: isOn ? 'var(--ok)' : 'var(--border)',
    transition: 'background-color 0.3s ease',
  }

  const knobStyle: React.CSSProperties = {
    position: 'absolute',
    top: 3,
    left: isOn ? 25 : 3,
    width: 20,
    height: 20,
    borderRadius: '50%',
    backgroundColor: 'var(--fg)',
    transition: 'left 0.3s ease',
  }

  const labelStyle: React.CSSProperties = {
    fontSize: 12,
    fontWeight: 600,
    color: isOn ? 'var(--ok)' : 'var(--muted)',
    transition: 'color 0.3s ease',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
  }

  return (
    <div style={containerStyle}>
      <div style={trackStyle}>
        <div style={knobStyle} />
      </div>
      <div style={labelStyle}>{hasData ? (isOn ? onLabel : offLabel) : '--'}</div>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.on_off,
  component: OnOffWidget,
})

export default OnOffWidget
