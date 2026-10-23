import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const DonutWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const fillColor = config.fillColor ?? 'var(--cyan)'
  const showPercentage = config.showPercentage !== false
  const showName = config.showName !== true ? false : true
  const thickness = config.thickness ?? 14

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

  const activeColor = pct >= critPct
    ? 'var(--crit)'
    : pct >= warnPct
      ? 'var(--warn)'
      : fillColor

  const cx = 100
  const cy = 100
  const r = 75

  const circumference = 2 * Math.PI * r
  const dashOffset = circumference * (1 - pct)

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center' }}>
      <svg viewBox="0 0 200 200" style={{ width: '100%', height: '90%' }} preserveAspectRatio="xMidYMid meet">
        <defs>
          <filter id={`donut-shadow-${instance.id}`} x="-20%" y="-20%" width="140%" height="140%">
            <feDropShadow dx="0" dy="2" stdDeviation="3" floodColor="#000" floodOpacity="0.15" />
          </filter>
          <filter id={`donut-glow-${instance.id}`} x="-10%" y="-10%" width="120%" height="120%">
            <feGaussianBlur in="SourceGraphic" stdDeviation="2" />
          </filter>
        </defs>

        {/* Outer shadow ring */}
        <circle cx={cx} cy={cy} r={r}
          fill="none" stroke="var(--border)" strokeWidth={thickness + 4}
          opacity={0.1} filter={`url(#donut-shadow-${instance.id})`} />

        {/* Background track */}
        <circle cx={cx} cy={cy} r={r}
          fill="none" stroke="var(--border)" strokeWidth={thickness} opacity={0.2} />

        {/* Track border rings */}
        <circle cx={cx} cy={cy} r={r + thickness / 2}
          fill="none" stroke="var(--border)" strokeWidth={0.5} opacity={0.3} />
        <circle cx={cx} cy={cy} r={r - thickness / 2}
          fill="none" stroke="var(--border)" strokeWidth={0.5} opacity={0.3} />

        {/* Active arc */}
        <circle cx={cx} cy={cy} r={r}
          fill="none" stroke={activeColor} strokeWidth={thickness}
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={dashOffset}
          transform={`rotate(-90 ${cx} ${cy})`}
          style={{ transition: 'stroke-dashoffset 0.6s ease, stroke 0.3s ease' }} />

        {/* Inner subtle highlight */}
        <circle cx={cx} cy={cy} r={r - thickness / 2 - 2}
          fill="none" stroke="var(--fg)" strokeWidth={0.3} opacity={0.08} />

        {/* Value */}
        <text x={cx} y={cy - 6} textAnchor="middle" fontSize="28" fontWeight="700" fill="var(--fg)">
          {data?.value !== undefined ? value.toFixed(decimals) : '--'}
        </text>
        <text x={cx} y={cy + 12} textAnchor="middle" fontSize="11" fill="var(--muted)">
          {unit}
        </text>
        {showPercentage && (
          <text x={cx} y={cy + 27} textAnchor="middle" fontSize="10" fill="var(--muted)" opacity={0.7}>
            {data?.value !== undefined ? `${(pct * 100).toFixed(0)}%` : '--%'}
          </text>
        )}
      </svg>
      {showName && sensor?.name && (
        <div style={{ fontSize: 10, color: 'var(--muted)', textAlign: 'center' }}>{sensor.name}</div>
      )}
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.donut,
  component: DonutWidget,
})

export default DonutWidget
