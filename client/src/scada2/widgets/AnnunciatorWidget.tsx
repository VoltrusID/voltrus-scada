import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const AnnunciatorWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensors, currentValues } = props
  const config = instance.config ?? {}
  const maxItems = config.maxItems ?? 12

  const boundSensors = instance.sensorIds.length > 0
    ? sensors.filter(s => instance.sensorIds.includes(s.id))
    : instance.sensorId
      ? sensors.filter(s => s.id === instance.sensorId)
      : sensors

  const displaySensors = boundSensors.length > 0 ? boundSensors.slice(0, maxItems) : sensors.slice(0, maxItems)

  if (displaySensors.length === 0) {
    return (
      <div style={{
        width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: 'var(--muted)', fontSize: 11,
      }}>
        Drop tags here
      </div>
    )
  }

  const items = displaySensors.map(s => {
    const d = currentValues[String(s.id)]
    const isAlarm = d?.status === 'crit' || d?.status === 'warn'
    return {
      sensor: s,
      value: d?.value,
      status: d?.status ?? '',
      isAlarm,
      severity: d?.status === 'crit' ? 'crit' as const : d?.status === 'warn' ? 'warn' as const : null,
    }
  })

  const severityColor = (s: 'crit' | 'warn' | null) =>
    s === 'crit' ? 'var(--hmi-fault)' : s === 'warn' ? 'var(--hmi-warn)' : 'var(--border)'

  // Annunciator windows: quiet when normal, filled and (critical) flashing when in alarm.
  return (
    <div style={{
      width: '100%', height: '100%', overflow: 'auto', display: 'grid',
      gridTemplateColumns: 'repeat(auto-fill, minmax(150px, 1fr))', gap: 4, alignContent: 'start',
    }}>
      {items.map((item, i) => {
        const color = severityColor(item.severity)
        return (
          <div key={i} className={item.severity === 'crit' ? 'hmi-blink' : undefined} style={{
            display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 2, minHeight: 40,
            padding: '5px 8px', borderRadius: 3, border: `1px solid ${color}`,
            background: item.isAlarm ? `color-mix(in srgb, ${color} 20%, transparent)` : 'var(--hmi-badge)',
            transition: 'border-color 0.3s ease, background-color 0.3s ease',
          }}>
            <div style={{
              fontSize: 11, fontWeight: 600, lineHeight: 1.2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              color: item.isAlarm ? 'var(--fg)' : 'var(--text-dim)',
            }}>
              {item.sensor.name}
            </div>
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: 6, fontSize: 11, fontFamily: 'var(--font-mono)' }}>
              <span style={{ color: item.isAlarm ? 'var(--fg)' : 'var(--text-muted)' }}>
                {item.value !== undefined ? `${item.value.toFixed(item.sensor.decimals ?? 1)} ${item.sensor.unit ?? ''}` : '----'}
              </span>
              {item.isAlarm && <span style={{ fontSize: 9, fontWeight: 700, letterSpacing: '.08em', color }}>{item.severity === 'crit' ? 'ALARM' : 'WARN'}</span>}
            </div>
          </div>
        )
      })}
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.annunciator,
  component: AnnunciatorWidget,
})

export default AnnunciatorWidget
