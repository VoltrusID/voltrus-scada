import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const ProgressBarWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const fillColor = config.fillColor ?? 'var(--ok)'
  const direction = config.direction ?? 'up'
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
    ? 'var(--crit)' : pct >= warnPct
    ? 'var(--warn)' : fillColor

  const isVertical = direction === 'up' || direction === 'down'
  const isInverted = direction === 'down' || direction === 'left'

  if (isVertical) {
    const barX = 35
    const barY = 10
    const barW = 40
    const barH = 140
    const fillH = barH * pct
    const fillY = isInverted ? barY : barY + barH - fillH

    return (
      <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <svg viewBox="0 0 100 170" style={{ width: '100%', height: '100%' }} preserveAspectRatio="xMidYMid meet">
          <defs>
            <clipPath id={`vpbar-clip-${instance.id}`}>
              <rect x={barX} y={barY} width={barW} height={barH} rx={3} ry={3} />
            </clipPath>
          </defs>

          <rect x={barX} y={barY} width={barW} height={barH} rx={3} ry={3}
            fill="var(--border)" opacity={0.3} />

          {pct > 0 && (
            <rect x={barX} y={fillY} width={barW} height={fillH}
              fill={activeFill} opacity={0.85}
              clipPath={`url(#vpbar-clip-${instance.id})`}
              style={{ transition: 'y 0.4s ease, height 0.4s ease' }}
            />
          )}

          <rect x={barX} y={barY} width={barW} height={barH} rx={3} ry={3}
            fill="none" stroke="var(--border)" strokeWidth={1.5} />

          {showTicks && [0, 0.25, 0.5, 0.75, 1].map((mark, i) => {
            const y = isInverted ? barY + barH * mark : barY + barH * (1 - mark)
            return (
              <g key={i}>
                <line x1={barX + barW} y1={y} x2={barX + barW + 4} y2={y} stroke="var(--muted)" strokeWidth={0.7} />
                <text x={barX + barW + 6} y={y + 3} fontSize="7" fill="var(--muted)">
                  {(min + range * mark).toFixed(0)}
                </text>
              </g>
            )
          })}

          <text x={barX + barW / 2} y={barY + barH + 16}
            textAnchor="middle" fontSize="12" fontWeight="700" fill="var(--fg)">
            {data?.value !== undefined ? value.toFixed(decimals) : '--'}
          </text>
          {unit && (
            <text x={barX + barW / 2} y={barY + barH + 28}
              textAnchor="middle" fontSize="9" fill="var(--muted)">{unit}</text>
          )}
        </svg>
      </div>
    )
  }

  const barX = 10
  const barY = 20
  const barW = 180
  const barH = 30
  const fillW = barW * pct
  const fillX = isInverted ? barX + barW - fillW : barX

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <svg viewBox="0 0 200 80" style={{ width: '100%', height: '100%' }} preserveAspectRatio="xMidYMid meet">
        <defs>
          <clipPath id={`hpbar-clip-${instance.id}`}>
            <rect x={barX} y={barY} width={barW} height={barH} rx={3} ry={3} />
          </clipPath>
        </defs>

        <rect x={barX} y={barY} width={barW} height={barH} rx={3} ry={3}
          fill="var(--border)" opacity={0.3} />

        {pct > 0 && (
          <rect x={fillX} y={barY} width={fillW} height={barH}
            fill={activeFill} opacity={0.85}
            clipPath={`url(#hpbar-clip-${instance.id})`}
            style={{ transition: 'x 0.4s ease, width 0.4s ease' }}
          />
        )}

        <rect x={barX} y={barY} width={barW} height={barH} rx={3} ry={3}
          fill="none" stroke="var(--border)" strokeWidth={1.5} />

        {showTicks && [0, 0.25, 0.5, 0.75, 1].map((mark, i) => {
          const x = isInverted ? barX + barW * (1 - mark) : barX + barW * mark
          return (
            <g key={i}>
              <line x1={x} y1={barY + barH} x2={x} y2={barY + barH + 4} stroke="var(--muted)" strokeWidth={0.7} />
              <text x={x} y={barY + barH + 12} textAnchor="middle" fontSize="7" fill="var(--muted)">
                {(min + range * mark).toFixed(0)}
              </text>
            </g>
          )
        })}

        <text x={barX + barW / 2} y={barY + barH / 2 + 4}
          textAnchor="middle" fontSize="12" fontWeight="700" fill="var(--fg)">
          {data?.value !== undefined ? `${value.toFixed(decimals)} ${unit}` : '--'}
        </text>
      </svg>
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.progress_bar,
  component: ProgressBarWidget,
})

export default ProgressBarWidget
