import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const SemicircleGaugeWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const showTicks = config.showTicks !== false
  const thickness = config.thickness ?? 12

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

  const valueColor = pct >= critPct ? 'var(--crit)' : pct >= warnPct ? 'var(--warn)' : 'var(--ok)'

  const cx = 100
  const cy = 85
  const r = 65

  const toRad = (p: number) => Math.PI * (1 - p)

  const arcPath = (startPct: number, endPct: number) => {
    const s = toRad(startPct)
    const e = toRad(endPct)
    const x1 = cx + r * Math.cos(s)
    const y1 = cy - r * Math.sin(s)
    const x2 = cx + r * Math.cos(e)
    const y2 = cy - r * Math.sin(e)
    const largeArc = (endPct - startPct) > 0.5 ? 1 : 0
    return `M ${x1} ${y1} A ${r} ${r} 0 ${largeArc} 1 ${x2} ${y2}`
  }

  const fullArc = arcPath(0, 1)

  const ticks = []
  if (showTicks) {
    const majorCount = 10
    const minorPerMajor = 5
    const totalTicks = majorCount * minorPerMajor
    for (let i = 0; i <= totalTicks; i++) {
      const p = i / totalTicks
      const angle = toRad(p)
      const major = i % minorPerMajor === 0
      const outerR = r + thickness / 2 + 2
      const innerR = major ? r - thickness / 2 - 5 : r - thickness / 2 - 1
      const labelR = r + thickness / 2 + 12
      ticks.push({
        x1: cx + outerR * Math.cos(angle),
        y1: cy - outerR * Math.sin(angle),
        x2: cx + innerR * Math.cos(angle),
        y2: cy - innerR * Math.sin(angle),
        major,
        label: major && i % (minorPerMajor * 2) === 0 ? (min + range * p).toFixed(0) : null,
        lx: cx + labelR * Math.cos(angle),
        ly: cy - labelR * Math.sin(angle),
      })
    }
  }

  const needleAngle = toRad(pct)
  const needleLen = r - 16
  const needleTail = 8
  const needleTip = { x: cx + needleLen * Math.cos(needleAngle), y: cy - needleLen * Math.sin(needleAngle) }
  const tailPt = { x: cx - needleTail * Math.cos(needleAngle), y: cy + needleTail * Math.sin(needleAngle) }
  const perp = needleAngle + Math.PI / 2
  const hw = 2.5
  const b1 = { x: tailPt.x + hw * Math.cos(perp), y: tailPt.y - hw * Math.sin(perp) }
  const b2 = { x: tailPt.x - hw * Math.cos(perp), y: tailPt.y + hw * Math.sin(perp) }

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'flex-end', justifyContent: 'center' }}>
      <svg viewBox="0 0 200 130" style={{ width: '100%', height: '100%' }} preserveAspectRatio="xMidYMid meet">
        <defs>
          <clipPath id={`semi-clip-${instance.id}`}>
            <path d={fullArc} fill="none" stroke="white" strokeWidth={thickness + 2} strokeLinecap="round" />
          </clipPath>
          <filter id={`semi-needle-${instance.id}`} x="-50%" y="-50%" width="200%" height="200%">
            <feDropShadow dx="0" dy="1" stdDeviation="1" floodColor="#000" floodOpacity="0.35" />
          </filter>
        </defs>

        {/* Background track */}
        <path d={fullArc} fill="none" stroke="var(--border)" strokeWidth={thickness} strokeLinecap="round" opacity={0.3} />

        {/* Zone colors (clipped) */}
        <g clipPath={`url(#semi-clip-${instance.id})`}>
          {warnPct > 0 && (
            <path d={arcPath(0, warnPct)} fill="none" stroke="var(--ok)" strokeWidth={thickness} opacity={0.45} />
          )}
          {critPct > warnPct && (
            <path d={arcPath(warnPct, critPct)} fill="none" stroke="var(--warn)" strokeWidth={thickness} opacity={0.45} />
          )}
          {critPct < 1 && (
            <path d={arcPath(critPct, 1)} fill="none" stroke="var(--crit)" strokeWidth={thickness} opacity={0.45} />
          )}
          {/* Active arc */}
          <path d={arcPath(0, pct)} fill="none" stroke={valueColor} strokeWidth={thickness} strokeLinecap="round" opacity={0.95} />
        </g>

        {/* Ticks */}
        {ticks.map((t, i) => (
          <g key={i}>
            <line x1={t.x1} y1={t.y1} x2={t.x2} y2={t.y2}
              stroke="var(--muted)" strokeWidth={t.major ? 1.2 : 0.5} opacity={t.major ? 0.7 : 0.35} />
            {t.label && (
              <text x={t.lx} y={t.ly + 3} textAnchor="middle" fontSize="6.5" fill="var(--muted)" opacity={0.7}>
                {t.label}
              </text>
            )}
          </g>
        ))}

        {/* Needle */}
        <polygon
          points={`${needleTip.x},${needleTip.y} ${b1.x},${b1.y} ${b2.x},${b2.y}`}
          fill={valueColor}
          filter={`url(#semi-needle-${instance.id})`}
          style={{ transition: 'fill 0.3s ease' }}
        />
        <circle cx={cx} cy={cy} r={5} fill="var(--card)" stroke="var(--border)" strokeWidth={0.8} />
        <circle cx={cx} cy={cy} r={2} fill="var(--fg)" />

        {/* Value below arc */}
        <text x={cx} y={cy + 18} textAnchor="middle" fontSize="20" fontWeight="700" fill="var(--fg)">
          {data?.value !== undefined ? value.toFixed(decimals) : '--'}
        </text>
        {unit && (
          <text x={cx} y={cy + 30} textAnchor="middle" fontSize="9" fill="var(--muted)">
            {unit}
          </text>
        )}
      </svg>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.semicircle_gauge,
  component: SemicircleGaugeWidget,
})

export default SemicircleGaugeWidget
