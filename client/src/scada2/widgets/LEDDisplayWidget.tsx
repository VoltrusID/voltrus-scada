import React from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const SEGMENTS: Record<string, [boolean, boolean, boolean, boolean, boolean, boolean, boolean]> = {
  '0': [true, true, true, false, true, true, true],
  '1': [false, false, true, false, false, true, false],
  '2': [true, false, true, true, true, false, true],
  '3': [true, false, true, true, false, true, true],
  '4': [false, true, true, true, false, true, false],
  '5': [true, true, false, true, false, true, true],
  '6': [true, true, false, true, true, true, true],
  '7': [true, false, true, false, false, true, false],
  '8': [true, true, true, true, true, true, true],
  '9': [true, true, true, true, false, true, true],
  '-': [false, false, false, true, false, false, false],
  '.': [false, false, false, false, false, false, false],
  ' ': [false, false, false, false, false, false, false],
}

const LEDDisplayWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor, data } = props
  const config = instance.config ?? {}
  const color = config.color ?? 'var(--cyan)'
  const showName = config.showName !== false

  const min = sensor?.min_val ?? 0
  const max = sensor?.max_val ?? 100
  const decimals = sensor?.decimals ?? 1
  const unit = sensor?.unit ?? ''

  const value = data?.value ?? min

  const text = data?.value !== undefined ? value.toFixed(decimals) : '--'

  const renderDigit = (char: string, idx: number) => {
    const segs = SEGMENTS[char] ?? SEGMENTS[' ']!
    const [a, b, c, d, e, f, g] = segs

    const ox = idx * 28
    const dw = 22
    const dh = 38
    const sw = 3

    const segColor = (on: boolean) => on ? color : 'var(--border)'
    const segOpacity = (on: boolean) => on ? 0.9 : 0.15

    return (
      <g key={idx}>
        {a && <line x1={ox + 3} y1={1} x2={ox + dw - 3} y2={1} stroke={segColor(a)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(a)} />}
        {a === false && <line x1={ox + 3} y1={1} x2={ox + dw - 3} y2={1} stroke={segColor(a)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(a)} />}

        <line x1={ox + 3} y1={1} x2={ox + dw - 3} y2={1}
          stroke={segColor(a)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(a)} />
        <line x1={ox + dw + 1} y1={3} x2={ox + dw + 1} y2={dh / 2 - 1}
          stroke={segColor(b)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(b)} />
        <line x1={ox + dw + 1} y1={dh / 2 + 1} x2={ox + dw + 1} y2={dh - 3}
          stroke={segColor(c)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(c)} />
        <line x1={ox + 3} y1={dh / 2} x2={ox + dw - 3} y2={dh / 2}
          stroke={segColor(d)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(d)} />
        <line x1={ox - 1} y1={dh / 2 + 1} x2={ox - 1} y2={dh - 3}
          stroke={segColor(e)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(e)} />
        <line x1={ox - 1} y1={3} x2={ox - 1} y2={dh / 2 - 1}
          stroke={segColor(f)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(f)} />
        <line x1={ox + 3} y1={dh - 1} x2={ox + dw - 3} y2={dh - 1}
          stroke={segColor(g)} strokeWidth={sw} strokeLinecap="round" opacity={segOpacity(g)} />
      </g>
    )
  }

  const chars = text.split('')
  const totalW = chars.length * 28 + (unit ? unit.length * 8 : 0)
  const viewW = Math.max(totalW + 16, 80)

  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
      alignItems: 'center', justifyContent: 'center',
      backgroundColor: 'rgba(0,0,0,0.15)', borderRadius: 4,
    }}>
      <svg viewBox={`0 0 ${viewW} 52`} style={{ width: '90%', height: '70%' }} preserveAspectRatio="xMidYMid meet">
        <g transform={`translate(8, 6)`}>
          {chars.map((char, i) => renderDigit(char, i))}
          {unit && (
            <text x={chars.length * 28 + 4} y={30} fontSize="10" fill="var(--muted)">
              {unit}
            </text>
          )}
        </g>
      </svg>
      {showName && sensor?.name && (
        <div style={{ fontSize: 9, color: 'var(--muted)', textAlign: 'center', marginTop: 2 }}>
          {sensor.name}
        </div>
      )}
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.led_display,
  component: LEDDisplayWidget,
})

export default LEDDisplayWidget
