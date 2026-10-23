import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const MeterWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const fillColor = config.fillColor ?? 'var(--ok)'
  const showTicks = config.showTicks !== false

  const min = sensor?.min_val ?? 0
  const max = sensor?.max_val ?? 100
  const warn = sensor?.warn_val ?? min + (max - min) * 0.75
  const crit = sensor?.crit_val ?? min + (max - min) * 0.9
  const decimals = sensor?.decimals ?? 1
  const unit = sensor?.unit ?? ''

  const value = data?.value ?? min
  const range = max - min || 1
  const pct = Math.max(0, Math.min(1, (value - min) / range))

  const warnPct = Math.max(0, Math.min(1, (warn - min) / range))
  const critPct = Math.max(0, Math.min(1, (crit - min) / range))

  const activeFill = pct >= critPct
    ? 'var(--crit)'
    : pct >= warnPct
      ? 'var(--warn)'
      : fillColor

  const barY = 30
  const barH = 24
  const barX = 10
  const barW = 180
  const bulbR = 18
  const bulbCx = barX + barW / 2
  const bulbCy = barY + barH + bulbR + 4

  const ticks = []
  if (showTicks) {
    for (let i = 0; i <= 10; i++) {
      const x = barX + (barW * i) / 10
      const major = i % 5 === 0
      ticks.push({ x, major, label: major ? (min + range * i / 10).toFixed(0) : null })
    }
  }

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <svg viewBox="0 0 200 100" style={{ width: '100%', height: '100%' }} preserveAspectRatio="xMidYMid meet">
        <defs>
          <linearGradient id={`meter-bg-${instance.id}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--border)" />
            <stop offset="100%" stopColor="var(--border)" stopOpacity={0.6} />
          </linearGradient>
          <linearGradient id={`meter-fill-${instance.id}`} x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stopColor={activeFill} stopOpacity={0.8} />
            <stop offset="100%" stopColor={activeFill} />
          </linearGradient>
        </defs>

        <rect x={barX} y={barY} width={barW} height={barH} rx={barH / 2} ry={barH / 2}
          fill={`url(#meter-bg-${instance.id})`} />

        {pct > 0 && (
          <rect
            x={barX} y={barY}
            width={Math.max(barH, barW * pct)}
            height={barH}
            rx={barH / 2} ry={barH / 2}
            fill={`url(#meter-fill-${instance.id})`}
            style={{ transition: 'width 0.5s ease' }}
          />
        )}

        {ticks.map((t, i) => (
          <g key={i}>
            <line x1={t.x} y1={barY - 3} x2={t.x} y2={barY + (t.major ? 0 : -1)}
              stroke="var(--muted)" strokeWidth={t.major ? 1.5 : 0.5} />
            {t.label && (
              <text x={t.x} y={barY - 7} textAnchor="middle" fontSize="7" fill="var(--muted)">
                {t.label}
              </text>
            )}
          </g>
        ))}

        <circle cx={barX + barW * pct} cy={barY + barH / 2} r={barH / 2 - 2}
          fill="var(--fg)" style={{ transition: 'cx 0.5s ease' }} />
        <circle cx={barX + barW * pct} cy={barY + barH / 2} r={barH / 2 - 5}
          fill={activeFill} style={{ transition: 'cx 0.5s ease' }} />

        <text x={barX + barW / 2} y={barY + barH + 16} textAnchor="middle" fontSize="14" fontWeight="700" fill="var(--fg)">
          {data?.value !== undefined ? `${value.toFixed(decimals)}` : '--'}
        </text>
        {unit && (
          <text x={barX + barW / 2} y={barY + barH + 28} textAnchor="middle" fontSize="9" fill="var(--muted)">
            {unit}
          </text>
        )}
      </svg>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.meter,
  component: MeterWidget,
})

export default MeterWidget
