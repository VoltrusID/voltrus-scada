import React, { useRef, useEffect } from 'react'
import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { WidgetProps } from './types'

const MultiTrendWidget: React.FC<WidgetProps> = (props) => {
  const { instance, sensors, currentValues } = props
  const config = instance.config ?? {}
  const maxTags = config.maxTags ?? 4
  const pointsCount = config.pointsCount ?? 60

  const canvasRef = useRef<HTMLCanvasElement>(null)
  const bufferRef = useRef<Map<string, number[]>>(new Map())

  const boundSensors = instance.sensorIds.length > 0
    ? sensors.filter(s => instance.sensorIds.includes(s.id))
    : instance.sensorId
      ? sensors.filter(s => s.id === instance.sensorId)
      : sensors.slice(0, maxTags)

  const displaySensors = boundSensors.length > 0 ? boundSensors.slice(0, maxTags) : sensors.slice(0, maxTags)

  const palette = ['var(--cyan)', 'var(--ok)', 'var(--warn)', '#8b5cf6', '#f97316']

  useEffect(() => {
    for (const sensor of displaySensors) {
      const d = currentValues[String(sensor.id)]
      if (d?.value !== undefined) {
        let buf = bufferRef.current.get(sensor.key)
        if (!buf) {
          buf = []
          bufferRef.current.set(sensor.key, buf)
        }
        buf.push(d.value)
        if (buf.length > pointsCount) buf.shift()
      }
    }
  }, [currentValues, displaySensors, pointsCount])

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

    const resolvedBorder = resolveColor('var(--border)')
    const resolvedMuted = resolveColor('var(--muted)')

    const pad = { top: 8, right: 8, bottom: 18, left: 32 }
    const plotW = w - pad.left - pad.right
    const plotH = h - pad.top - pad.bottom

    let yMin = Infinity
    let yMax = -Infinity
    for (const sensor of displaySensors) {
      const buf = bufferRef.current.get(sensor.key) ?? []
      for (const v of buf) {
        if (v < yMin) yMin = v
        if (v > yMax) yMax = v
      }
    }

    // Grid lines
    ctx.font = '9px Inter, sans-serif'
    ctx.textAlign = 'right'
    ctx.textBaseline = 'middle'
    if (yMin !== Infinity) {
      const yRange = yMax - yMin || 1
      yMin -= yRange * 0.05
      yMax += yRange * 0.05
      const totalRange = yMax - yMin || 1

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
        ctx.fillText(val.toFixed(val % 1 === 0 ? 0 : 1), pad.left - 4, y)
        ctx.globalAlpha = 1
      }
    }

    if (yMin === Infinity) {
      ctx.strokeStyle = resolvedMuted
      ctx.lineWidth = 1
      ctx.setLineDash([4, 4])
      ctx.beginPath()
      ctx.moveTo(pad.left, h / 2)
      ctx.lineTo(pad.left + plotW, h / 2)
      ctx.stroke()
      ctx.setLineDash([])
      return
    }

    const totalRange = yMax - yMin || 1

    // Smooth curve helper
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

    for (let si = 0; si < displaySensors.length; si++) {
      const sensor = displaySensors[si]
      const buf = bufferRef.current.get(sensor.key) ?? []
      if (buf.length < 2) continue

      const color = resolveColor(palette[si % palette.length])

      const points = buf.map((v, i) => ({
        x: pad.left + (i / (buf.length - 1)) * plotW,
        y: pad.top + plotH - ((v - yMin) / totalRange) * plotH,
      }))

      // Gradient fill
      const gradient = ctx.createLinearGradient(0, pad.top, 0, pad.top + plotH)
      gradient.addColorStop(0, color + '20')
      gradient.addColorStop(1, color + '03')

      drawSmoothLine(points)
      ctx.lineTo(points[points.length - 1].x, pad.top + plotH)
      ctx.lineTo(points[0].x, pad.top + plotH)
      ctx.closePath()
      ctx.fillStyle = gradient
      ctx.fill()

      drawSmoothLine(points)
      ctx.strokeStyle = color
      ctx.lineWidth = 1.5
      ctx.lineJoin = 'round'
      ctx.stroke()

      const last = points[points.length - 1]
      ctx.beginPath()
      ctx.arc(last.x, last.y, 3, 0, Math.PI * 2)
      ctx.fillStyle = color
      ctx.fill()
    }

    // Legend at bottom
    const legendY = h - 4
    let legendX = pad.left
    ctx.font = '9px Inter, sans-serif'
    ctx.textAlign = 'left'
    ctx.textBaseline = 'middle'
    for (let si = 0; si < displaySensors.length; si++) {
      const color = resolveColor(palette[si % palette.length])
      const name = displaySensors[si].name.length > 10
        ? displaySensors[si].name.slice(0, 9) + '..'
        : displaySensors[si].name

      ctx.fillStyle = color
      ctx.beginPath()
      ctx.arc(legendX + 4, legendY, 3, 0, Math.PI * 2)
      ctx.fill()
      legendX += 10
      ctx.fillStyle = resolvedMuted
      ctx.fillText(name, legendX, legendY)
      legendX += ctx.measureText(name).width + 12
    }
  }, [currentValues, displaySensors, pointsCount])

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', overflow: 'hidden' }}>
      {displaySensors.length === 0 ? (
        <div style={{
          position: 'absolute', inset: 0, display: 'flex',
          alignItems: 'center', justifyContent: 'center',
          color: 'var(--muted)', fontSize: 11,
        }}>
          Drop tags here
        </div>
      ) : null}
      <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
    </div>
  )
}

registerWidget({
  manifest: WIDGET_MANIFESTS.multi_trend,
  component: MultiTrendWidget,
})

export default MultiTrendWidget
