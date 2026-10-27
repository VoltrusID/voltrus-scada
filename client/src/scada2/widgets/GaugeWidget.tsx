import React, { useMemo } from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'
import { dialArcPath } from '../../gaugeMath'

const GaugeWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const showLabels = config.showLabels !== false

  const min = sensor?.min_val ?? 0
  const max = sensor?.max_val ?? 100
  const warn = sensor?.warn_val ?? min + (max - min) * 0.75
  const crit = sensor?.crit_val ?? min + (max - min) * 0.9
  const decimals = sensor?.decimals ?? 1
  const unit = sensor?.unit ?? ''

  const value = data?.value ?? min
  const range = max - min || 1
  const pct = Math.max(0, Math.min(1, (value - min) / range))

  // Arc geometry: 270-degree sweep, centered at bottom
  const cx = 100
  const cy = 100
  const r = 75
  const arcWidth = 10
  const startAngle = 225
  const endAngle = -45

  const arcPath = (startA: number, endA: number, radius = r) => dialArcPath(cx, cy, radius, startA, endA)

  // Zone boundaries
  const warnPct = Math.max(0, Math.min(1, (warn - min) / range))
  const critPct = Math.max(0, Math.min(1, (crit - min) / range))

  const pctToAngle = (p: number) => startAngle - 270 * p

  // Color for current value
  const valueColor = pct >= critPct ? 'var(--crit)' : pct >= warnPct ? 'var(--warn)' : 'var(--ok)'

  // Tick marks
  const ticks = useMemo(() => {
    const result: { x1: number; y1: number; x2: number; y2: number; major: boolean; angle: number }[] = []
    const majorCount = 10
    const minorPerMajor = 5
    const totalTicks = majorCount * minorPerMajor
    for (let i = 0; i <= totalTicks; i++) {
      const angle = startAngle - (270 * i) / totalTicks
      const rad = (angle * Math.PI) / 180
      const major = i % minorPerMajor === 0
      const outerR = r + arcWidth / 2 + 2
      const innerR = major ? r - arcWidth / 2 - 6 : r - arcWidth / 2 - 2
      result.push({
        x1: cx + innerR * Math.cos(rad),
        y1: cy - innerR * Math.sin(rad),
        x2: cx + outerR * Math.cos(rad),
        y2: cy - outerR * Math.sin(rad),
        major,
        angle,
      })
    }
    return result
  }, [])

  // Needle
  const needleAngle = startAngle - 270 * pct
  const needleRad = (needleAngle * Math.PI) / 180
  const needleLen = r - 14
  const needleTail = 12
  const needleTip = { x: cx + needleLen * Math.cos(needleRad), y: cy - needleLen * Math.sin(needleRad) }
  const needleTailPt = { x: cx - needleTail * Math.cos(needleRad), y: cy + needleTail * Math.sin(needleRad) }
  const perpAngle = needleRad + Math.PI / 2
  const halfW = 3
  const b1 = { x: needleTailPt.x + halfW * Math.cos(perpAngle), y: needleTailPt.y - halfW * Math.sin(perpAngle) }
  const b2 = { x: needleTailPt.x - halfW * Math.cos(perpAngle), y: needleTailPt.y + halfW * Math.sin(perpAngle) }

  // Major tick labels
  const tickLabels = useMemo(() => {
    const result: { x: number; y: number; label: string }[] = []
    const count = 10
    for (let i = 0; i <= count; i++) {
      const angle = startAngle - (270 * i) / count
      const labelR = r + arcWidth / 2 + 14
      const rad = (angle * Math.PI) / 180
      const val = min + (range * i) / count
      result.push({
        x: cx + labelR * Math.cos(rad),
        y: cy - labelR * Math.sin(rad) + 3,
        label: val.toFixed(val % 1 === 0 ? 0 : 1),
      })
    }
    return result
  }, [min, max, range])

  const trackPath = arcPath(startAngle, endAngle)

  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center', overflow: 'hidden',
    }}>
      <svg viewBox="0 0 200 165" style={{ width: '100%', height: '90%' }} preserveAspectRatio="xMidYMid meet">
        <defs>
          {/* Clip zone colors to the track shape */}
          <clipPath id={`gauge-clip-${instance.id}`}>
            <path d={trackPath} fill="none" stroke="white" strokeWidth={arcWidth + 2} strokeLinecap="round" />
          </clipPath>
          {/* Needle shadow */}
          <filter id={`needle-shadow-${instance.id}`} x="-50%" y="-50%" width="200%" height="200%">
            <feDropShadow dx="0" dy="1" stdDeviation="1.5" floodColor="#000" floodOpacity="0.4" />
          </filter>
          {/* Track background gradient */}
          <linearGradient id={`track-bg-${instance.id}`} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--border)" stopOpacity={0.5} />
            <stop offset="100%" stopColor="var(--border)" stopOpacity={0.25} />
          </linearGradient>
        </defs>

        {/* Background arc track */}
        <path d={trackPath} fill="none" stroke={`url(#track-bg-${instance.id})`} strokeWidth={arcWidth} strokeLinecap="round" />

        {/* Zone colors (clipped to track) */}
        <g clipPath={`url(#gauge-clip-${instance.id})`}>
          {warnPct > 0 && (
            <path d={arcPath(startAngle, pctToAngle(warnPct))} fill="none"
              stroke="var(--ok)" strokeWidth={arcWidth} strokeLinecap="butt" opacity={0.5} />
          )}
          {critPct > warnPct && (
            <path d={arcPath(pctToAngle(warnPct), pctToAngle(critPct))} fill="none"
              stroke="var(--warn)" strokeWidth={arcWidth} strokeLinecap="butt" opacity={0.5} />
          )}
          {critPct < 1 && (
            <path d={arcPath(pctToAngle(critPct), endAngle)} fill="none"
              stroke="var(--crit)" strokeWidth={arcWidth} strokeLinecap="butt" opacity={0.5} />
          )}

          {/* Active value arc (bright) */}
          <path d={arcPath(startAngle, pctToAngle(pct))} fill="none"
            stroke={valueColor} strokeWidth={arcWidth} strokeLinecap="round" opacity={0.95} />
        </g>

        {/* Tick marks */}
        {ticks.map((t, i) => (
          <line key={i} x1={t.x1} y1={t.y1} x2={t.x2} y2={t.y2}
            stroke="var(--muted)" strokeWidth={t.major ? 1.5 : 0.6} opacity={t.major ? 0.8 : 0.4} />
        ))}

        {/* Major tick labels */}
        {showLabels && tickLabels.map((tl, i) => (
          <text key={`tl-${i}`} x={tl.x} y={tl.y} textAnchor="middle" fontSize="7" fill="var(--muted)" opacity={0.7}>
            {tl.label}
          </text>
        ))}

        {/* Needle */}
        <polygon
          points={`${needleTip.x},${needleTip.y} ${b1.x},${b1.y} ${b2.x},${b2.y}`}
          fill={valueColor}
          filter={`url(#needle-shadow-${instance.id})`}
          style={{ transition: 'fill 0.3s ease' }}
        />

        {/* Center cap */}
        <circle cx={cx} cy={cy} r={7} fill="var(--card)" stroke="var(--border)" strokeWidth={1} />
        <circle cx={cx} cy={cy} r={3} fill="var(--fg)" />

        {/* Value text */}
        <text x={cx} y={cy + 30} textAnchor="middle" fontSize="18" fontWeight="700" fill="var(--fg)">
          {data?.value !== undefined ? value.toFixed(decimals) : '--'}
        </text>
        {unit && (
          <text x={cx} y={cy + 43} textAnchor="middle" fontSize="10" fill="var(--muted)">
            {unit}
          </text>
        )}
      </svg>
      {showLabels && sensor?.name && (
        <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 2, textAlign: 'center' }}>{sensor.name}</div>
      )}
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.gauge,
  component: GaugeWidget,
})

export default GaugeWidget
