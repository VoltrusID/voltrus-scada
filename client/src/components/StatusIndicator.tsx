import React from 'react'

export type StatusKind = 'ok' | 'fault' | 'warning' | 'off' | 'unknown'

interface Props {
  status: StatusKind
  size?: 'sm' | 'md' | 'lg'
  label?: string
}

const SIZES: Record<string, number> = { sm: 16, md: 24, lg: 32 }
const STROKES: Record<string, number> = { sm: 1.5, md: 2, lg: 2.5 }

function statusColor(status: StatusKind): string {
  switch (status) {
    case 'ok': return 'var(--ok)'
    case 'fault': return 'var(--crit)'
    case 'warning': return 'var(--warn)'
    case 'off': return 'var(--muted)'
    case 'unknown': return 'var(--text-dim)'
  }
}

export function mapStatus(status: string): StatusKind {
  switch (status) {
    case 'ok':
    case 'normal':
    case 'online':
    case 'active':
      return 'ok'
    case 'critical':
    case 'fault':
    case 'error':
      return 'fault'
    case 'warning':
    case 'warn':
      return 'warning'
    case 'offline':
    case 'disabled':
    case 'off':
      return 'off'
    default:
      return 'unknown'
  }
}

function trianglePath(s: number): string {
  const h = s * 0.866
  const cy = s / 2 + s * 0.07
  return `M${s / 2},${cy - h / 2} L${s / 2 + s / 2},${cy + h / 2} L${s / 2 - s / 2},${cy + h / 2} Z`
}

function diamondPath(s: number): string {
  const cx = s / 2, cy = s / 2, r = s / 2 - 1
  return `M${cx},${cy - r} L${cx + r},${cy} L${cx},${cy + r} L${cx - r},${cy} Z`
}

export const StatusIndicator = React.memo(function StatusIndicator({ status, size = 'md', label }: Props) {
  const s = SIZES[size]
  const sw = STROKES[size]
  const color = statusColor(status)
  const strokeColor = 'var(--text)'

  const shape = (() => {
    switch (status) {
      case 'ok':
        return (
          <circle cx={s / 2} cy={s / 2} r={s / 2 - 2} fill={color} stroke={strokeColor} strokeWidth={sw} />
        )
      case 'fault':
        return (
          <path d={trianglePath(s)} fill={color} stroke={strokeColor} strokeWidth={sw} strokeLinejoin="round" />
        )
      case 'warning':
        return (
          <path d={diamondPath(s)} fill={color} stroke={strokeColor} strokeWidth={sw} strokeLinejoin="round" />
        )
      case 'off':
        return (
          <rect x={2} y={2} width={s - 4} height={s - 4} rx={2} fill={color} stroke={strokeColor} strokeWidth={sw} />
        )
      case 'unknown':
        return (
          <g>
            <circle cx={s / 2} cy={s / 2} r={s / 2 - 2} fill="none" stroke={color} strokeWidth={sw} />
            <text x={s / 2} y={s / 2} textAnchor="middle" dominantBaseline="central" fill={color} fontSize={s * 0.55} fontWeight={700} fontFamily="Inter, system-ui, sans-serif">?</text>
          </g>
        )
    }
  })()

  return (
    <svg width={s} height={s} viewBox={`0 0 ${s} ${s}`} role="img" aria-label={label ?? `Status: ${status}`} {...(label ? { title: label ?? status } : {})}>
      {shape}
    </svg>
  )
})