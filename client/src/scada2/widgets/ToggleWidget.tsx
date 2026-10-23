import React, { useCallback } from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const ToggleWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data, onWriteValue } = props
  const config = instance.config ?? {}
  const onThreshold = config.onThreshold ?? 0.5
  const onValue = config.onValue ?? 1
  const offValue = config.offValue ?? 0
  const onLabel = config.onLabel ?? 'ON'
  const offLabel = config.offLabel ?? 'OFF'

  const value = data?.value
  const hasData = value !== undefined && value !== null
  const isOn = hasData && value > onThreshold

  const handleToggle = useCallback(() => {
    if (!onWriteValue) return
    onWriteValue(isOn ? offValue : onValue)
  }, [onWriteValue, isOn, onValue, offValue])

  return (
    <div style={{
      display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center',
      width: '100%', height: '100%',
      gap: 6, overflow: 'hidden',
    }}>
      <span style={{ fontSize: 10, color: 'var(--muted)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: '100%' }}>
        {sensor?.name ?? ''}
      </span>
      <button
        onClick={handleToggle}
        disabled={!onWriteValue}
        style={{
          position: 'relative',
          width: 56,
          height: 30,
          borderRadius: 15,
          border: 'none',
          backgroundColor: isOn ? 'var(--ok)' : 'var(--border)',
          cursor: onWriteValue ? 'pointer' : 'default',
          transition: 'background-color 0.3s ease',
          padding: 0,
          outline: 'none',
        }}
      >
        <div style={{
          position: 'absolute',
          top: 3,
          left: isOn ? 29 : 3,
          width: 24,
          height: 24,
          borderRadius: '50%',
          backgroundColor: '#fff',
          transition: 'left 0.3s ease',
          boxShadow: '0 1px 3px rgba(0,0,0,.3)',
        }} />
      </button>
      <div style={{
        fontSize: 12,
        fontWeight: 700,
        color: hasData ? (isOn ? 'var(--ok)' : 'var(--muted)') : 'var(--muted)',
        textTransform: 'uppercase',
        letterSpacing: 0.5,
        transition: 'color 0.3s ease',
      }}>
        {hasData ? (isOn ? onLabel : offLabel) : '--'}
      </div>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.toggle,
  component: ToggleWidget,
})

export default ToggleWidget
