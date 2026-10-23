import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const DataTableWidget: React.FC<WidgetProps> = ({ instance, sensors, currentValues }) => {
  const config = instance.config ?? {}
  const showStatus = config.showStatus !== false
  const compact = config.compact ?? false

  // Collect sensors: either from sensorIds or fall back to single sensorId
  const sensorIds: number[] = instance.sensorIds?.length
    ? instance.sensorIds
    : instance.sensorId != null
      ? [instance.sensorId]
      : sensors.slice(0, 6).map(s => s.id)

  const sensorMap = new Map(sensors.map(s => [s.id, s]))
  const rows = sensorIds
    .map(id => ({ sensor: sensorMap.get(id), data: currentValues[id] }))
    .filter(r => r.sensor)

  if (rows.length === 0) {
    return (
      <div style={{
        width: '100%', height: '100%', display: 'flex',
        alignItems: 'center', justifyContent: 'center',
        color: 'var(--muted)', fontSize: 12,
      }}>
        Drop tags to add rows
      </div>
    )
  }

  const cellPad = compact ? '3px 6px' : '5px 8px'
  const fontSize = compact ? 11 : 12

  const statusDot = (status: string) => {
    const c = status === 'crit' ? 'var(--crit)' : status === 'warn' ? 'var(--warn)' : 'var(--ok)'
    return { width: 6, height: 6, borderRadius: '50%', background: c, flexShrink: 0 }
  }

  return (
    <div style={{
      width: '100%', height: '100%', overflow: 'auto',
      display: 'flex', flexDirection: 'column',
    }}>
      <table style={{
        width: '100%', borderCollapse: 'collapse',
        fontSize, fontFamily: '"JetBrains Mono", monospace',
      }}>
        <thead>
          <tr>
            {showStatus && <th style={{ padding: cellPad, textAlign: 'left', borderBottom: '1px solid var(--border)', width: 16 }}></th>}
            <th style={{ padding: cellPad, textAlign: 'left', borderBottom: '1px solid var(--border)', color: 'var(--muted)', fontWeight: 600, fontSize: fontSize - 1, textTransform: 'uppercase', letterSpacing: '.05em' }}>Tag</th>
            <th style={{ padding: cellPad, textAlign: 'right', borderBottom: '1px solid var(--border)', color: 'var(--muted)', fontWeight: 600, fontSize: fontSize - 1, textTransform: 'uppercase', letterSpacing: '.05em' }}>Value</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ sensor, data }) => {
            if (!sensor) return null
            const value = data?.value
            const status = data?.status ?? ''
            const decimals = sensor.decimals ?? 1
            const statusColor = status === 'crit' ? 'var(--crit)' : status === 'warn' ? 'var(--warn)' : 'var(--fg)'

            return (
              <tr key={sensor.id}>
                {showStatus && (
                  <td style={{ padding: cellPad, borderBottom: '1px solid var(--border)' }}>
                    <div style={statusDot(status)} />
                  </td>
                )}
                <td style={{
                  padding: cellPad, borderBottom: '1px solid var(--border)',
                  color: 'var(--fg)', whiteSpace: 'nowrap',
                  overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: 120,
                }}>
                  {sensor.name}
                </td>
                <td style={{
                  padding: cellPad, borderBottom: '1px solid var(--border)',
                  textAlign: 'right', color: statusColor, fontWeight: 500,
                  whiteSpace: 'nowrap',
                }}>
                  {value !== undefined ? value.toFixed(decimals) : '--'}
                  {sensor.unit ? <span style={{ color: 'var(--muted)', marginLeft: 3, fontSize: fontSize - 1 }}>{sensor.unit}</span> : null}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

registerWidget({ manifest: WIDGET_MANIFESTS.data_table, component: DataTableWidget })
