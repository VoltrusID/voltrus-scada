import { useRef, useEffect } from 'react'
import { useCanvasSize } from '../hooks/useCanvasSize'

export interface HistogramChartProps {
  data: {
    bins: Array<{ lower: number; upper: number; count: number; frequency: number }>
    normal_curve: Array<{ x: number; y: number }>
    stats: { mean: number; stddev: number; count: number }
    capability: { cp: number; cpk: number; pp: number; ppk: number } | null
  } | null
  lsl?: number
  usl?: number
  loading?: boolean
}

function formatNum(v: number, d = 2): string { return Number.isFinite(v) ? v.toFixed(d) : '--' }

export function HistogramChart({ data, lsl, usl, loading }: HistogramChartProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const size = useCanvasSize(containerRef)

  useEffect(() => {
    const cv = canvasRef.current
    if (!cv || !data || data.bins.length === 0) return
    const ctx = cv.getContext('2d')!
    const { width: W, height: H, dpr } = size
    if (W <= 0 || H <= 0) return

    cv.width = W * dpr; cv.height = H * dpr; ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    const pL = 50, pR = 20, pT = 10, pB = 40
    const cW = W - pL - pR, cH = H - pT - pB
    ctx.clearRect(0, 0, W, H)

    const bins = data.bins
    const maxCount = Math.max(...bins.map(b => b.count), 1)
    const xMin = bins[0].lower, xMax = bins[bins.length - 1].upper
    const xRange = xMax - xMin || 1

    const toX = (v: number) => pL + ((v - xMin) / xRange) * cW
    const toY = (v: number) => pT + cH - (v / maxCount) * cH

    // Grid
    ctx.strokeStyle = '#2a2a2e'; ctx.lineWidth = 1; ctx.setLineDash([])
    for (let i = 0; i <= 4; i++) {
      const y = pT + (cH / 4) * i
      ctx.beginPath(); ctx.moveTo(pL, y); ctx.lineTo(W - pR, y); ctx.stroke()
    }

    // Y-axis labels
    ctx.fillStyle = '#888'; ctx.font = '10px JetBrains Mono,monospace'; ctx.textAlign = 'right'
    for (let i = 0; i <= 4; i++) {
      const v = maxCount * (4 - i) / 4
      ctx.fillText(v.toFixed(0), pL - 6, pT + (cH / 4) * i + 3)
    }

    // Bars
    const barW = (cW / bins.length) * 0.85
    for (const bin of bins) {
      const x = toX((bin.lower + bin.upper) / 2) - barW / 2
      const h = (bin.count / maxCount) * cH
      ctx.fillStyle = 'rgba(0, 212, 170, 0.4)'
      ctx.fillRect(x, pT + cH - h, barW, h)
      ctx.strokeStyle = 'rgba(0, 212, 170, 0.6)'; ctx.lineWidth = 1
      ctx.strokeRect(x, pT + cH - h, barW, h)
    }

    // Normal curve
    if (data.normal_curve.length > 1) {
      const curveMax = Math.max(...data.normal_curve.map(p => p.y))
      const curveScale = maxCount / (curveMax || 1)
      ctx.beginPath(); ctx.strokeStyle = '#00d4aa'; ctx.lineWidth = 2; ctx.setLineDash([])
      for (let i = 0; i < data.normal_curve.length; i++) {
        const p = data.normal_curve[i]
        const x = toX(p.x), y = toY(p.y * curveScale)
        i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)
      }
      ctx.stroke()
    }

    // LSL line
    if (lsl != null && lsl >= xMin && lsl <= xMax) {
      const x = toX(lsl)
      ctx.setLineDash([6, 4]); ctx.strokeStyle = '#f59e0b'; ctx.lineWidth = 1.5
      ctx.beginPath(); ctx.moveTo(x, pT); ctx.lineTo(x, pT + cH); ctx.stroke()
      ctx.setLineDash([]); ctx.fillStyle = '#f59e0b'; ctx.textAlign = 'center'
      ctx.font = '10px JetBrains Mono,monospace'; ctx.fillText('LSL', x, pT - 2)
    }

    // USL line
    if (usl != null && usl >= xMin && usl <= xMax) {
      const x = toX(usl)
      ctx.setLineDash([6, 4]); ctx.strokeStyle = '#ef4444'; ctx.lineWidth = 1.5
      ctx.beginPath(); ctx.moveTo(x, pT); ctx.lineTo(x, pT + cH); ctx.stroke()
      ctx.setLineDash([]); ctx.fillStyle = '#ef4444'; ctx.textAlign = 'center'
      ctx.font = '10px JetBrains Mono,monospace'; ctx.fillText('USL', x, pT - 2)
    }

    // X-axis labels (first few bin edges)
    ctx.fillStyle = '#888'; ctx.font = '10px JetBrains Mono,monospace'; ctx.textAlign = 'center'
    const step = Math.max(1, Math.floor(bins.length / 5))
    for (let i = 0; i < bins.length; i += step) {
      ctx.fillText(bins[i].lower.toFixed(1), toX(bins[i].lower), H - 8)
    }
    ctx.fillText(xMax.toFixed(1), toX(xMax), H - 8)
  }, [data, lsl, usl, size])

  if (loading) {
    return <div style={{ color: 'var(--muted)', fontSize: 12, padding: 20 }}>Loading histogram...</div>
  }
  if (!data) {
    return <div style={{ color: 'var(--muted)', fontSize: 12, padding: 20 }}>No histogram data available</div>
  }

  const cardStyle: React.CSSProperties = {
    display: 'flex', flexDirection: 'column', alignItems: 'center', padding: '6px 10px',
    background: 'var(--card)', borderRadius: 6, border: '1px solid var(--border)', minWidth: 70,
  }

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
      <div ref={containerRef} style={{ flex: 1, minHeight: 0 }}>
        <canvas ref={canvasRef} style={{ display: 'block', width: '100%', height: '100%' }} />
      </div>
      <div style={{ display: 'flex', gap: 8, padding: '6px 12px', flexWrap: 'wrap' }}>
        <div style={cardStyle}>
          <span style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase' }}>Mean</span>
          <span style={{ fontSize: 12, color: 'var(--fg)', fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{formatNum(data.stats.mean)}</span>
        </div>
        <div style={cardStyle}>
          <span style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase' }}>Std Dev</span>
          <span style={{ fontSize: 12, color: 'var(--fg)', fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{formatNum(data.stats.stddev)}</span>
        </div>
        <div style={cardStyle}>
          <span style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase' }}>Count</span>
          <span style={{ fontSize: 12, color: 'var(--fg)', fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{data.stats.count}</span>
        </div>
        {data.capability && (
          <>
            <div style={cardStyle}>
              <span style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase' }}>Cp</span>
              <span style={{ fontSize: 12, color: '#00d4aa', fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{formatNum(data.capability.cp)}</span>
            </div>
            <div style={cardStyle}>
              <span style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase' }}>Cpk</span>
              <span style={{ fontSize: 12, color: data.capability.cpk >= 1.33 ? '#22c55e' : '#f59e0b', fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{formatNum(data.capability.cpk)}</span>
            </div>
            <div style={cardStyle}>
              <span style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase' }}>Pp</span>
              <span style={{ fontSize: 12, color: '#00d4aa', fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{formatNum(data.capability.pp)}</span>
            </div>
            <div style={cardStyle}>
              <span style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase' }}>Ppk</span>
              <span style={{ fontSize: 12, color: data.capability.ppk >= 1.33 ? '#22c55e' : '#f59e0b', fontFamily: 'var(--font-mono)', fontWeight: 600 }}>{formatNum(data.capability.ppk)}</span>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
