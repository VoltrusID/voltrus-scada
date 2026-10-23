import { useRef, useEffect } from 'react'
import { useCanvasSize } from '../hooks/useCanvasSize'

export interface SpcChartProps {
  data: {
    chart_type: string
    data_points: Array<{ index: number; value: number; timestamp: number }>
    control_limits: { ucl: number; cl: number; lcl: number }
    violations: number[]
    in_control_percent: number
  } | null
  loading?: boolean
}

function formatTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit' })
}

export function SPCChart({ data, loading }: SpcChartProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const size = useCanvasSize(containerRef)

  useEffect(() => {
    const cv = canvasRef.current
    if (!cv || !data || data.data_points.length === 0) return
    const ctx = cv.getContext('2d')!
    const { width: W, height: H, dpr } = size
    if (W <= 0 || H <= 0) return

    cv.width = W * dpr; cv.height = H * dpr; ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    const pL = 50, pR = 20, pT = 10, pB = 30
    const cW = W - pL - pR, cH = H - pT - pB
    ctx.clearRect(0, 0, W, H)

    const pts = data.data_points
    const { ucl, cl, lcl } = data.control_limits
    const vals = pts.map(p => p.value).concat([ucl, cl, lcl])
    let yMin = Math.min(...vals), yMax = Math.max(...vals)
    const pad = (yMax - yMin) * 0.1 || 1
    yMin -= pad; yMax += pad

    const toX = (i: number) => pL + (i / (pts.length - 1 || 1)) * cW
    const toY = (v: number) => pT + cH - ((v - yMin) / (yMax - yMin)) * cH

    // Grid lines
    ctx.strokeStyle = '#2a2a2e'; ctx.lineWidth = 1; ctx.setLineDash([])
    for (let i = 0; i <= 4; i++) {
      const y = pT + (cH / 4) * i
      ctx.beginPath(); ctx.moveTo(pL, y); ctx.lineTo(W - pR, y); ctx.stroke()
    }

    // Y-axis labels
    ctx.fillStyle = '#888'; ctx.font = '10px JetBrains Mono,monospace'; ctx.textAlign = 'right'
    for (let i = 0; i <= 4; i++) {
      const v = yMin + ((yMax - yMin) / 4) * (4 - i)
      ctx.fillText(v.toFixed(1), pL - 6, pT + (cH / 4) * i + 3)
    }

    // Control limits
    ctx.setLineDash([6, 4]); ctx.lineWidth = 1.5
    // UCL
    ctx.strokeStyle = '#ef4444'; ctx.beginPath(); ctx.moveTo(pL, toY(ucl)); ctx.lineTo(W - pR, toY(ucl)); ctx.stroke()
    ctx.fillStyle = '#ef4444'; ctx.textAlign = 'left'; ctx.font = '10px JetBrains Mono,monospace'
    ctx.fillText(`UCL ${ucl.toFixed(2)}`, W - pR - 70, toY(ucl) - 4)
    // CL
    ctx.strokeStyle = '#22c55e'; ctx.beginPath(); ctx.moveTo(pL, toY(cl)); ctx.lineTo(W - pR, toY(cl)); ctx.stroke()
    ctx.fillStyle = '#22c55e'; ctx.fillText(`CL ${cl.toFixed(2)}`, W - pR - 60, toY(cl) - 4)
    // LCL
    ctx.strokeStyle = '#ef4444'; ctx.beginPath(); ctx.moveTo(pL, toY(lcl)); ctx.lineTo(W - pR, toY(lcl)); ctx.stroke()
    ctx.fillStyle = '#ef4444'; ctx.fillText(`LCL ${lcl.toFixed(2)}`, W - pR - 70, toY(lcl) + 12)

    // Data line
    ctx.setLineDash([]); ctx.strokeStyle = '#00d4aa'; ctx.lineWidth = 1.5; ctx.lineJoin = 'round'
    ctx.beginPath()
    for (let i = 0; i < pts.length; i++) {
      const x = toX(i), y = toY(pts[i].value)
      i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)
    }
    ctx.stroke()

    // Data points
    const violationSet = new Set(data.violations)
    for (let i = 0; i < pts.length; i++) {
      const x = toX(i), y = toY(pts[i].value)
      const isViolation = violationSet.has(i)
      ctx.beginPath(); ctx.arc(x, y, isViolation ? 5 : 3, 0, Math.PI * 2)
      ctx.fillStyle = isViolation ? '#ef4444' : '#00d4aa'; ctx.fill()
      if (isViolation) {
        ctx.strokeStyle = 'rgba(239,68,68,0.4)'; ctx.lineWidth = 1.5; ctx.stroke()
      }
    }

    // X-axis labels
    ctx.fillStyle = '#888'; ctx.font = '10px JetBrains Mono,monospace'; ctx.textAlign = 'center'
    if (pts.length > 0) {
      ctx.fillText(formatTime(pts[0].timestamp), pL, H - 6)
      ctx.fillText(formatTime(pts[pts.length - 1].timestamp), W - pR, H - 6)
    }
  }, [data, size])

  if (loading) {
    return <div style={{ color: 'var(--muted)', fontSize: 12, padding: 20 }}>Loading SPC data...</div>
  }
  if (!data) {
    return <div style={{ color: 'var(--muted)', fontSize: 12, padding: 20 }}>No SPC data available</div>
  }

  const pctColor = data.in_control_percent >= 95 ? '#22c55e' : data.in_control_percent >= 80 ? '#f59e0b' : '#ef4444'

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '8px 12px 4px' }}>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)', textTransform: 'uppercase', letterSpacing: '.04em' }}>
          {data.chart_type}
        </span>
        <span style={{ fontSize: 11, color: pctColor, fontWeight: 700, fontFamily: 'var(--font-mono)' }}>
          {data.in_control_percent.toFixed(1)}% in control
        </span>
      </div>
      <div ref={containerRef} style={{ flex: 1, minHeight: 0 }}>
        <canvas ref={canvasRef} style={{ display: 'block', width: '100%', height: '100%' }} />
      </div>
    </div>
  )
}
