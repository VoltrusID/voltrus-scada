import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const BarChartWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensors, currentValues } = props
  const config = instance.config ?? {}
  const maxBars = config.maxBars ?? 8
  const showValues = config.showValues !== false

  const boundSensors = instance.sensorIds.length > 0
    ? sensors.filter(s => instance.sensorIds.includes(s.id))
    : instance.sensorId
      ? sensors.filter(s => s.id === instance.sensorId)
      : sensors.slice(0, maxBars)

  const displaySensors = boundSensors.length > 0 ? boundSensors.slice(0, maxBars) : sensors.slice(0, maxBars)

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

  let globalMin = Infinity
  let globalMax = -Infinity
  const items = displaySensors.map(s => {
    const d = currentValues[String(s.id)]
    const v = d?.value ?? 0
    const sMin = s.min_val ?? 0
    const sMax = s.max_val ?? 100
    globalMin = Math.min(globalMin, sMin)
    globalMax = Math.max(globalMax, sMax)
    return { sensor: s, value: v, status: d?.status ?? '' }
  })
  const range = globalMax - globalMin || 1

  // Ensure 0 is in the range for proper bar alignment
  if (globalMin > 0) globalMin = 0
  const adjRange = globalMax - globalMin || 1

  const padLeft = 28
  const padRight = 8
  const padTop = 10
  const padBottom = 22
  const chartW = 200 - padLeft - padRight
  const chartH = 120 - padTop - padBottom
  const barGap = Math.max(2, Math.min(6, chartW / items.length * 0.15))
  const barW = Math.max(8, (chartW - barGap * (items.length + 1)) / items.length)

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <svg viewBox="0 0 200 120" style={{ width: '100%', height: '100%' }} preserveAspectRatio="xMidYMid meet">
        <defs>
          <linearGradient id={`barchart-bar-${instance.id}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--cyan)" stopOpacity={0.95} />
            <stop offset="100%" stopColor="var(--cyan)" stopOpacity={0.7} />
          </linearGradient>
        </defs>

        {/* Grid lines */}
        {[0, 0.25, 0.5, 0.75, 1].map((p, i) => {
          const y = padTop + chartH * (1 - p)
          const label = (globalMin + adjRange * p).toFixed(0)
          return (
            <g key={i}>
              <line x1={padLeft} y1={y} x2={padLeft + chartW} y2={y}
                stroke="var(--border)" strokeWidth={0.4} strokeDasharray="2 2" opacity={0.6} />
              <text x={padLeft - 4} y={y + 3} textAnchor="end" fontSize="6" fill="var(--muted)" opacity={0.7}>
                {label}
              </text>
            </g>
          )
        })}

        {/* Bars */}
        {items.map((item, i) => {
          const pct = Math.max(0, Math.min(1, (item.value - globalMin) / adjRange))
          const barH = chartH * pct
          const x = padLeft + barGap + i * (barW + barGap)
          const y = padTop + chartH - barH

          const statusColor = item.status === 'crit' ? 'var(--crit)'
            : item.status === 'warn' ? 'var(--warn)' : 'var(--ok)'
          const barColor = item.status ? statusColor : 'var(--cyan)'

          const label = item.sensor.name.length > 6
            ? item.sensor.name.slice(0, 5) + '..'
            : item.sensor.name

          return (
            <g key={i}>
              {/* Bar background */}
              <rect x={x} y={padTop} width={barW} height={chartH}
                fill="var(--border)" opacity={0.08} rx={2} />

              {/* Bar fill with rounded top */}
              {barH > 0 && (
                <>
                  <rect x={x} y={y} width={barW} height={barH}
                    fill={barColor} rx={2} opacity={0.85}
                    style={{ transition: 'height 0.4s ease, y 0.4s ease' }} />
                  {/* Subtle top highlight */}
                  <rect x={x + 1} y={y} width={barW - 2} height={Math.min(3, barH)}
                    fill="white" opacity={0.15} rx={1} />
                </>
              )}

              {showValues && barH > 0 && (
                <text x={x + barW / 2} y={y - 3} textAnchor="middle" fontSize="6" fontWeight="600" fill="var(--fg)">
                  {item.value.toFixed(1)}
                </text>
              )}
              <text x={x + barW / 2} y={padTop + chartH + 11} textAnchor="middle" fontSize="5.5" fill="var(--muted)">
                {label}
              </text>
            </g>
          )
        })}
      </svg>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.bar_chart,
  component: BarChartWidget,
})

export default BarChartWidget
