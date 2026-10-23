import React, { useRef, useEffect } from 'react'
import { useChartValues } from '../../store/dashboardStore'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const TrendWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensor } = props
  const config = instance.config ?? {}
  const lineColor = config.lineColor ?? 'var(--cyan)'
  const pointsCount = config.pointsCount ?? 180

  const canvasRef = useRef<HTMLCanvasElement>(null)
  // The store keeps the live series per tag, backfilled from history on load, so trends start full.
  const series = useChartValues(instance.sensorId ?? '', 'live')

  const resolveColor = (color: string): string => {
    if (color.startsWith('var(')) {
      const varName = color.slice(4, -1)
      if (typeof window !== 'undefined') {
        return getComputedStyle(document.documentElement).getPropertyValue(varName).trim() || '#22d3ee'
      }
      return '#22d3ee'
    }
    return color
  }

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const dpr = window.devicePixelRatio || 1
    const rect = canvas.getBoundingClientRect()
    const w = rect.width
    const h = rect.height

    if (w === 0 || h === 0) return

    canvas.width = w * dpr
    canvas.height = h * dpr
    ctx.scale(dpr, dpr)
    ctx.clearRect(0, 0, w, h)

    const resolvedLineColor = resolveColor(lineColor)
    const resolvedMuted = resolveColor('var(--muted)')
    const resolvedBorder = resolveColor('var(--border)')
    const resolvedFg = resolveColor('var(--fg)')

    const buf = series.slice(-pointsCount)

    // Always draw grid
    const pad = { top: 18, right: 8, bottom: 14, left: 32 }
    const plotW = w - pad.left - pad.right
    const plotH = h - pad.top - pad.bottom

    // Determine y range
    let yMin: number, yMax: number
    if (buf.length < 2) {
      const sMin = sensor?.min_val ?? 0
      const sMax = sensor?.max_val ?? 100
      yMin = sMin
      yMax = sMax
    } else {
      yMin = Infinity
      yMax = -Infinity
      for (const v of buf) {
        if (v < yMin) yMin = v
        if (v > yMax) yMax = v
      }
      // At least 5 % of the tag's range, so a near-flat signal doesn't fill the plot with noise.
      const minSpan = Math.abs((sensor?.max_val ?? 0) - (sensor?.min_val ?? 0)) * 0.05 || 1e-6
      if (yMax - yMin < minSpan) {
        const mid = (yMax + yMin) / 2
        yMin = mid - minSpan / 2
        yMax = mid + minSpan / 2
      }
      const yRange = yMax - yMin
      yMin -= yRange * 0.05
      yMax += yRange * 0.05
    }
    const totalRange = yMax - yMin || 1
    // Enough decimals that the five grid labels differ.
    const axisDecimals = Math.min(4, Math.max(0, Math.ceil(-Math.log10(totalRange / 4))))

    // Grid lines (4 horizontal)
    ctx.font = `${9 * 1}px Inter, sans-serif`
    ctx.textAlign = 'right'
    ctx.textBaseline = 'middle'
    for (let i = 0; i <= 4; i++) {
      const p = i / 4
      const y = pad.top + plotH * (1 - p)
      const val = yMin + totalRange * p

      ctx.strokeStyle = resolvedBorder
      ctx.lineWidth = 0.5
      ctx.setLineDash([3, 3])
      ctx.beginPath()
      ctx.moveTo(pad.left, y)
      ctx.lineTo(pad.left + plotW, y)
      ctx.stroke()
      ctx.setLineDash([])

      ctx.fillStyle = resolvedMuted
      ctx.globalAlpha = 0.6
      ctx.fillText(val.toFixed(axisDecimals), pad.left - 4, y)
      ctx.globalAlpha = 1
    }

    if (buf.length < 2) {
      // Placeholder dashed line at center
      ctx.strokeStyle = resolvedMuted
      ctx.lineWidth = 1
      ctx.setLineDash([4, 4])
      ctx.beginPath()
      ctx.moveTo(pad.left, pad.top + plotH / 2)
      ctx.lineTo(pad.left + plotW, pad.top + plotH / 2)
      ctx.stroke()
      ctx.setLineDash([])
      return
    }

    // Map data points to canvas coordinates
    const points = buf.map((v, i) => ({
      x: pad.left + (i / (buf.length - 1)) * plotW,
      y: pad.top + plotH - ((v - yMin) / totalRange) * plotH,
    }))

    // Gradient fill under line
    const gradient = ctx.createLinearGradient(0, pad.top, 0, pad.top + plotH)
    gradient.addColorStop(0, resolvedLineColor + '30')
    gradient.addColorStop(1, resolvedLineColor + '05')

    // Smooth curve helper (cardinal spline)
    const drawSmoothLine = (pts: { x: number; y: number }[]) => {
      ctx.beginPath()
      ctx.moveTo(pts[0].x, pts[0].y)
      if (pts.length === 2) {
        ctx.lineTo(pts[1].x, pts[1].y)
        return
      }
      for (let i = 0; i < pts.length - 1; i++) {
        const p0 = pts[Math.max(0, i - 1)]
        const p1 = pts[i]
        const p2 = pts[i + 1]
        const p3 = pts[Math.min(pts.length - 1, i + 2)]
        const tension = 0.3
        const cp1x = p1.x + (p2.x - p0.x) * tension
        const cp1y = p1.y + (p2.y - p0.y) * tension
        const cp2x = p2.x - (p3.x - p1.x) * tension
        const cp2y = p2.y - (p3.y - p1.y) * tension
        ctx.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, p2.x, p2.y)
      }
    }

    // Fill area
    drawSmoothLine(points)
    ctx.lineTo(points[points.length - 1].x, pad.top + plotH)
    ctx.lineTo(points[0].x, pad.top + plotH)
    ctx.closePath()
    ctx.fillStyle = gradient
    ctx.fill()

    // Draw line
    drawSmoothLine(points)
    ctx.strokeStyle = resolvedLineColor
    ctx.lineWidth = 1.5
    ctx.lineJoin = 'round'
    ctx.stroke()

    // Current value dot
    const last = points[points.length - 1]
    ctx.beginPath()
    ctx.arc(last.x, last.y, 3, 0, Math.PI * 2)
    ctx.fillStyle = resolvedLineColor
    ctx.fill()
    ctx.beginPath()
    ctx.arc(last.x, last.y, 5, 0, Math.PI * 2)
    ctx.strokeStyle = resolvedLineColor + '40'
    ctx.lineWidth = 1
    ctx.stroke()

    // Current value label (top-right)
    const currentVal = buf[buf.length - 1]
    const decimals = sensor?.decimals ?? 1
    const unit = sensor?.unit ?? ''
    const valText = `${currentVal.toFixed(decimals)}${unit ? ' ' + unit : ''}`
    ctx.font = 'bold 10px Inter, sans-serif'
    ctx.textAlign = 'right'
    ctx.textBaseline = 'top'
    ctx.fillStyle = resolvedFg
    ctx.fillText(valText, pad.left + plotW, 2)

    if (config.showName !== false && sensor?.name) {
      ctx.font = '600 10px Inter, sans-serif'
      ctx.textAlign = 'left'
      ctx.fillStyle = resolvedMuted
      const room = plotW - ctx.measureText(valText).width - 12
      let name = sensor.name
      while (name.length > 4 && ctx.measureText(name).width > room) name = name.slice(0, -2)
      ctx.fillText(name === sensor.name ? name : name + '…', pad.left, 2)
    }

  }, [series, lineColor, pointsCount, sensor, config.showName])

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', overflow: 'hidden' }}>
      <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.trend,
  component: TrendWidget,
})

export default TrendWidget
