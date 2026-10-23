import { useRef, useEffect } from 'react'
import { useCanvasSize } from '../hooks/useCanvasSize'

export interface RunChartProps {
  data: {
    points: Array<{ timestamp: number; value: number; above_median: boolean }>
    median: number
    signals: Array<{ rule: string; start_idx: number; end_idx: number; description: string }>
    runs_count: number
  } | null
  loading?: boolean
}

function formatTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit' })
}

export function RunChart({ data, loading }: RunChartProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const size = useCanvasSize(containerRef)

  useEffect(() => {
    const cv = canvasRef.current
    if (!cv || !data || data.points.length === 0) return
    const ctx = cv.getContext('2d')!
    const { width: W, height: H, dpr } = size
    if (W <= 0 || H <= 0) return

    cv.width = W * dpr; cv.height = H * dpr; ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    const pL = 50, pR = 20, pT = 10, pB = 30
    const cW = W - pL - pR, cH = H - pT - pB
    ctx.clearRect(0, 0, W, H)

    const pts = data.points
    const vals = pts.map(p => p.value).concat([data.median])
    let yMin = Math.min(...vals), yMax = Math.max(...vals)
    const pad = (yMax - yMin) * 0.1 || 1
    yMin -= pad; yMax += pad

    const toX = (i: number) => pL + (i / (pts.length - 1 || 1)) * cW
    const toY = (v: number) => pT + cH - ((v - yMin) / (yMax - yMin)) * cH

    // Signal regions (highlighted bands)
    for (const sig of data.signals) {
      const x1 = toX(sig.start_idx)
      const x2 = toX(sig.end_idx)
      ctx.fillStyle = 'rgba(239, 68, 68, 0.08)'
      ctx.fillRect(x1, pT, x2 - x1, cH)
    }

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

    // Median line
    ctx.setLineDash([6, 4]); ctx.strokeStyle = '#f59e0b'; ctx.lineWidth = 1.5
    const medY = toY(data.median)
    ctx.beginPath(); ctx.moveTo(pL, medY); ctx.lineTo(W - pR, medY); ctx.stroke()
    ctx.setLineDash([]); ctx.fillStyle = '#f59e0b'; ctx.textAlign = 'left'
    ctx.font = '10px JetBrains Mono,monospace'
    ctx.fillText(`Med ${data.median.toFixed(2)}`, W - pR - 70, medY - 4)

    // Data line
    ctx.strokeStyle = '#00d4aa'; ctx.lineWidth = 1.5; ctx.lineJoin = 'round'; ctx.setLineDash([])
    ctx.beginPath()
    for (let i = 0; i < pts.length; i++) {
      const x = toX(i), y = toY(pts[i].value)
      i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)
    }
    ctx.stroke()

    // Data points — color based on above/below median
    for (let i = 0; i < pts.length; i++) {
      const x = toX(i), y = toY(pts[i].value)
      ctx.beginPath(); ctx.arc(x, y, 3, 0, Math.PI * 2)
      ctx.fillStyle = pts[i].above_median ? '#00d4aa' : '#3b82f6'; ctx.fill()
    }

    // X-axis labels
    ctx.fillStyle = '#888'; ctx.font = '10px JetBrains Mono,monospace'; ctx.textAlign = 'center'
    ctx.fillText(formatTime(pts[0].timestamp), pL, H - 6)
    ctx.fillText(formatTime(pts[pts.length - 1].timestamp), W - pR, H - 6)
  }, [data, size])

  if (loading) {
    return <div style={{ color: 'var(--muted)', fontSize: 12, padding: 20 }}>Loading run chart...</div>
  }
  if (!data) {
    return <div style={{ color: 'var(--muted)', fontSize: 12, padding: 20 }}>No run chart data available</div>
  }

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '8px 12px 4px' }}>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--fg)', textTransform: 'uppercase', letterSpacing: '.04em' }}>
          Run Chart
        </span>
        <span style={{ fontSize: 11, color: 'var(--muted)', fontFamily: 'var(--font-mono)' }}>
          {data.runs_count} runs
        </span>
      </div>
      <div ref={containerRef} style={{ flex: 1, minHeight: 0 }}>
        <canvas ref={canvasRef} style={{ display: 'block', width: '100%', height: '100%' }} />
      </div>
      {data.signals.length > 0 && (
        <div style={{ padding: '6px 12px', borderTop: '1px solid var(--border)', maxHeight: 100, overflow: 'auto' }}>
          {data.signals.map((sig, i) => (
            <div key={i} style={{ fontSize: 11, color: '#ef4444', marginBottom: 2, display: 'flex', gap: 8 }}>
              <span style={{ fontWeight: 600, color: 'var(--fg)', fontFamily: 'var(--font-mono)' }}>{sig.rule}</span>
              <span style={{ color: 'var(--muted)' }}>{sig.description}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
