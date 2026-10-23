import React, { useState, useRef, useCallback } from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const PushButtonWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data, editMode, onWriteValue } = props
  const config = instance.config ?? {}
  const onLabel = config.onLabel ?? 'START'
  const offLabel = config.offLabel ?? 'STOP'
  const onValue = config.onValue ?? 1
  const offValue = config.offValue ?? 0
  const buttonColor = config.buttonColor ?? 'var(--ok)'
  const showLabel = config.showLabel !== false

  const value = data?.value
  const hasData = value !== undefined && value !== null

  const [pressed, setPressed] = useState(false)
  const holdTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const handlePress = useCallback(() => {
    if (editMode || !onWriteValue) return
    setPressed(true)
    onWriteValue(onValue)
  }, [editMode, onWriteValue, onValue])

  // Momentary release: write OFF so the commanded equipment de-energizes.
  const handleRelease = useCallback(() => {
    if (editMode || !onWriteValue) return
    setPressed(false)
    onWriteValue(offValue)
  }, [editMode, onWriteValue, offValue])

  const handleToggle = useCallback(() => {
    if (editMode || !onWriteValue) return
    const isActive = hasData && value === onValue
    onWriteValue(isActive ? offValue : onValue)
  }, [editMode, onWriteValue, hasData, value, onValue, offValue])

  const isMaintained = config.mode === 'maintained'
  const isActive = hasData && value === onValue
  const activeColor = isActive ? buttonColor : 'var(--fg)'

  return (
    <div style={{
      width: '100%', height: '100%',
      display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center', gap: 6,
    }}>
      <button
        onClick={isMaintained ? handleToggle : undefined}
        onMouseDown={() => !isMaintained && handlePress()}
        onMouseUp={() => !isMaintained && handleRelease()}
        onMouseLeave={() => { if (!isMaintained && pressed) handleRelease(); else setPressed(false) }}
        disabled={editMode}
        style={{
          width: '60%', aspectRatio: '1',
          borderRadius: '50%',
          border: `3px solid ${isActive ? activeColor : 'var(--border)'}`,
          backgroundColor: pressed
            ? activeColor
            : isActive
              ? `${activeColor}22`
              : 'var(--card)',
          color: pressed ? 'var(--bg)' : activeColor,
          cursor: editMode ? 'default' : 'pointer',
          fontSize: 11,
          fontWeight: 700,
          letterSpacing: 0.5,
          boxShadow: pressed
            ? `inset 0 2px 6px rgba(0,0,0,0.3), 0 0 12px ${activeColor}44`
            : isActive
              ? `0 0 12px ${activeColor}33, 0 2px 4px rgba(0,0,0,0.1)`
              : '0 2px 4px rgba(0,0,0,0.1)',
          transition: 'all 0.15s ease',
          transform: pressed ? 'scale(0.95)' : 'scale(1)',
        }}
      >
        {isActive ? onLabel : offLabel}
      </button>

      {showLabel && sensor?.name && (
        <div style={{ fontSize: 9, color: 'var(--muted)', textAlign: 'center', lineHeight: 1.2 }}>
          {sensor.name}
        </div>
      )}
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.push_button,
  component: PushButtonWidget,
})

export default PushButtonWidget
