import { useRef, useEffect, useState, useMemo, useCallback } from 'react';
import { DataPoint } from './types';
import { fmtTs, safeMinMax, toHexColor } from './styles';
import { useCanvasSize } from './hooks/useCanvasSize';

export interface OverlaySeries {
  data: DataPoint[];
  color: string;
  name: string;
  unit: string;
}

// Decimate data to at most one point per pixel column for performance
function decimateData(data: DataPoint[], startIndex: number, endIndex: number, pxPerPoint: number): { index: number; point: DataPoint }[] {
  if (pxPerPoint >= 1) {
    // Each point gets at least 1px — no decimation needed
    const out: { index: number; point: DataPoint }[] = []
    for (let i = Math.max(0, Math.floor(startIndex)); i <= Math.min(data.length - 1, Math.ceil(endIndex)); i++) {
      out.push({ index: i, point: data[i] })
    }
    return out
  }
  // pxPerPoint < 1 means more points than pixels — decimate
  const step = Math.max(1, Math.ceil(1 / pxPerPoint))
  const out: { index: number; point: DataPoint }[] = []
  const s = Math.max(0, Math.floor(startIndex))
  const e = Math.min(data.length - 1, Math.ceil(endIndex))
  for (let i = s; i <= e; i += step) {
    out.push({ index: i, point: data[i] })
  }
  // Always include the last point
  if (e > s && (e - s) % step !== 0) {
    out.push({ index: e, point: data[e] })
  }
  return out
}

export function DetailChart({ data: rawData, color: rawColor, warn, crit, unit, overlays = [], pointWidth = 0 }: { data: DataPoint[]; color: string; warn: number; crit: number; unit: string; overlays?: OverlaySeries[]; pointWidth?: number }) {
  const data = useMemo(() => rawData.filter(d => d.value != null && !Number.isNaN(d.value)), [rawData])
  const color = toHexColor(rawColor)
  const validOverlays = useMemo(() => overlays.map(o => ({ ...o, data: o.data.filter((d: DataPoint) => d.value != null && !Number.isNaN(d.value)), color: toHexColor(o.color) })).filter(o => o.data.length >= 2), [overlays])

  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{ idx: number; clientX: number; clientY: number; py: number } | null>(null)
  const areaRef = useRef({ pL: 60, pR: 20, pT: 20, pB: 44, cW: 0, cH: 0, yMin: 0, yMax: 0, dLen: 0, scrollLeft: 0 })
  const size = useCanvasSize(containerRef)

  // Virtual width for density mode
  const virtualWidth = pointWidth > 0 ? Math.max(pointWidth * data.length + 80, size.width) : size.width

  // Drawing effect — pure arithmetic, zero DOM reads
  useEffect(() => {
    const cv = canvasRef.current
    const scrollEl = scrollRef.current
    if (!cv || data.length < 2) return
    const ctx = cv.getContext('2d')!
    const { width: W, height: H, dpr } = size
    if (W <= 0 || H <= 0) return

    cv.width = W * dpr
    cv.height = H * dpr
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)

    const pL = 60, pR = 20, pT = 20, pB = 44, cW = W - pL - pR, cH = H - pT - pB
    if (cW <= 0 || cH <= 0) return

    const scrollLeft = scrollEl?.scrollLeft ?? 0
    const values = data.map(d => d.value)
    const [rawMin, rawMax] = safeMinMax(values)
    let yMin = rawMin - 2, yMax = rawMax + 2
    if (yMax - yMin < 4) { yMin--; yMax++; }
    areaRef.current = { pL, pR, pT, pB, cW, cH, yMin, yMax, dLen: data.length, scrollLeft }

    // Determine visible range
    let visibleStart = 0, visibleEnd = data.length - 1
    let pxPerPoint = cW / (data.length - 1)

    if (pointWidth > 0) {
      pxPerPoint = pointWidth
      visibleStart = Math.max(0, (scrollLeft - pL) / pxPerPoint)
      visibleEnd = Math.min(data.length - 1, (scrollLeft + W - pL) / pxPerPoint)
    }

    const toX = (i: number) => pL + i * pxPerPoint - scrollLeft
    const toY = (v: number) => pT + cH - ((v - yMin) / (yMax - yMin)) * cH

    ctx.clearRect(0, 0, W, H)

    // Grid
    ctx.strokeStyle = '#1a1a1e'; ctx.lineWidth = 1
    for (let i = 0; i <= 5; i++) { const y = pT + (cH / 5) * i; ctx.beginPath(); ctx.moveTo(pL, y); ctx.lineTo(W - pR, y); ctx.stroke() }

    // Y labels
    ctx.fillStyle = '#52525b'; ctx.font = "11px 'SF Mono','JetBrains Mono',Menlo,monospace"; ctx.textAlign = 'right'
    for (let i = 0; i <= 5; i++) { const v = yMin + ((yMax - yMin) / 5) * (5 - i); ctx.fillText(v.toFixed(1), pL - 8, pT + (cH / 5) * i + 4) }

    // X labels — adaptive, only for visible range
    ctx.textAlign = 'center'
    const labelGap = Math.max(60, cW / 12)
    const indexStep = Math.max(1, Math.ceil(labelGap / Math.max(pxPerPoint, 1)))
    const decimated = decimateData(data, visibleStart, visibleEnd, pxPerPoint)

    for (const { index: i } of decimated) {
      if (i % indexStep === 0) {
        const x = toX(i)
        if (x >= pL && x <= W - pR) {
          ctx.fillText(fmtTs(data[i].ts), x, H - pB + 16)
        }
      }
    }
    // Always label first and last visible
    const firstVisible = Math.max(0, Math.floor(visibleStart))
    const lastVisible = Math.min(data.length - 1, Math.ceil(visibleEnd))
    ctx.fillText(fmtTs(data[firstVisible].ts), toX(firstVisible), H - pB + 16)
    ctx.fillText(fmtTs(data[lastVisible].ts), toX(lastVisible), H - pB + 16)

    // Threshold lines
    ctx.setLineDash([6, 4])
    if (warn >= yMin && warn <= yMax) { ctx.strokeStyle = 'rgba(234,179,8,0.4)'; ctx.lineWidth = 1; ctx.beginPath(); ctx.moveTo(pL, toY(warn)); ctx.lineTo(W - pR, toY(warn)); ctx.stroke(); ctx.fillStyle = 'rgba(234,179,8,0.7)'; ctx.font = "10px 'SF Mono','JetBrains Mono',Menlo,monospace"; ctx.textAlign = 'left'; ctx.fillText('WARN', W - pR + 4, toY(warn) + 3) }
    if (crit >= yMin && crit <= yMax) { ctx.strokeStyle = 'rgba(239,68,68,0.4)'; ctx.lineWidth = 1; ctx.beginPath(); ctx.moveTo(pL, toY(crit)); ctx.lineTo(W - pR, toY(crit)); ctx.stroke(); ctx.fillStyle = 'rgba(239,68,68,0.7)'; ctx.textAlign = 'left'; ctx.fillText('CRIT', W - pR + 4, toY(crit) + 3) }
    ctx.setLineDash([])

    // Overlays (normalized to chart area)
    for (const ov of validOverlays) {
      const ovVals = ov.data.map(d => d.value)
      const [ovMin, ovMax] = safeMinMax(ovVals)
      const ovRange = Math.max(ovMax - ovMin, 0.001)
      const ovToY = (v: number) => pT + cH - ((v - ovMin) / ovRange) * cH
      ctx.beginPath()
      for (let i = 0; i < ov.data.length; i++) {
        const x = pL + (i / (ov.data.length - 1)) * cW
        if (x < 0 || x > W) continue
        const y = ovToY(ov.data[i].value)
        i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)
      }
      ctx.strokeStyle = ov.color; ctx.lineWidth = 1.5; ctx.lineJoin = 'round'; ctx.lineCap = 'round'; ctx.stroke()
      const lx = pL + ((ov.data.length - 1) / (ov.data.length - 1)) * cW
      const ly = ovToY(ov.data[ov.data.length - 1].value)
      ctx.beginPath(); ctx.arc(lx, ly, 3, 0, Math.PI * 2); ctx.fillStyle = ov.color; ctx.fill()
    }

    // Primary line — draw only visible decimated points
    ctx.beginPath()
    let firstMove = true
    for (const { index: i, point } of decimated) {
      const x = toX(i)
      if (x < -10 || x > W + 10) continue
      const y = toY(point.value)
      if (firstMove) { ctx.moveTo(x, y); firstMove = false }
      else { ctx.lineTo(x, y) }
    }
    ctx.strokeStyle = color; ctx.lineWidth = 2; ctx.lineJoin = 'round'; ctx.lineCap = 'round'; ctx.stroke()

    // Fill area under line
    if (decimated.length > 0) {
      const firstX = toX(decimated[0].index)
      const lastX = toX(decimated[decimated.length - 1].index)
      ctx.lineTo(lastX, pT + cH)
      ctx.lineTo(firstX, pT + cH)
      ctx.closePath()
      const g = ctx.createLinearGradient(0, pT, 0, pT + cH); g.addColorStop(0, color + '25'); g.addColorStop(1, color + '00')
      ctx.fillStyle = g; ctx.fill()
    }

    // Last value dot and label
    const lastIdx = data.length - 1
    const lx = toX(lastIdx)
    if (lx >= pL && lx <= W - pR) {
      const lv = data[lastIdx].value
      const cy = toY(lv)
      ctx.beginPath(); ctx.arc(lx, cy, 5, 0, Math.PI * 2); ctx.fillStyle = color; ctx.fill()
      ctx.beginPath(); ctx.arc(lx, cy, 9, 0, Math.PI * 2); ctx.strokeStyle = color + '40'; ctx.lineWidth = 2; ctx.stroke()
      ctx.fillStyle = color; ctx.font = "bold 13px 'SF Mono','JetBrains Mono',Menlo,monospace"; ctx.textAlign = 'right'
      ctx.fillText(lv.toFixed(1) + (unit ? ' ' + unit : ''), W - pR - 2, cy - 12)
    }
  }, [data, color, warn, crit, unit, validOverlays, size, pointWidth])

  // Track whether user is manually scrolling (not at end)
  const isFollowingTail = useRef(true)
  const prevPointWidth = useRef(pointWidth)

  // Scroll handler — triggers redraw + tracks tail-follow state
  useEffect(() => {
    const scrollEl = scrollRef.current
    if (!scrollEl || pointWidth <= 0) return
    let raf: number
    const onScroll = () => {
      if (raf) cancelAnimationFrame(raf)
      raf = requestAnimationFrame(() => {
        if (!scrollEl) return
        const nearEnd = scrollEl.scrollWidth - scrollEl.scrollLeft - scrollEl.clientWidth < 50
        isFollowingTail.current = nearEnd
        setScrollVersion(v => v + 1)
      })
    }
    scrollEl.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      scrollEl.removeEventListener('scroll', onScroll)
      if (raf) cancelAnimationFrame(raf)
    }
  }, [pointWidth])

  // Auto-scroll to end on density change (pointWidth transition)
  // Use double rAF to ensure browser has laid out the new virtual width
  useEffect(() => {
    if (pointWidth > 0 && scrollRef.current && prevPointWidth.current !== pointWidth) {
      prevPointWidth.current = pointWidth
      isFollowingTail.current = true
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          if (!scrollRef.current) return
          scrollRef.current.scrollLeft = scrollRef.current.scrollWidth - scrollRef.current.clientWidth
        })
      })
    }
  }, [pointWidth])

  // Follow tail on data updates — only if user was already near the end
  useEffect(() => {
    if (pointWidth > 0 && scrollRef.current && isFollowingTail.current) {
      requestAnimationFrame(() => {
        if (!scrollRef.current) return
        scrollRef.current.scrollLeft = scrollRef.current.scrollWidth - scrollRef.current.clientWidth
      })
    }
  }, [data.length, pointWidth])

  const [scrollVersion, setScrollVersion] = useState(0)

  // Throttled mouse move
  const rafRef = useRef<number>(0)
  const onMove = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    if (rafRef.current) return
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = 0
      const cv = canvasRef.current
      if (!cv || data.length < 2) { setHover(null); return }
      const rect = cv.getBoundingClientRect()
      const mx = e.clientX - rect.left
      const { pL, cW, dLen, pT, cH, yMin, yMax, scrollLeft } = areaRef.current

      let pxPerPoint = cW / (dLen - 1)
      if (pointWidth > 0) pxPerPoint = pointWidth

      const idx = Math.round(((mx - pL + scrollLeft) / (pxPerPoint * (dLen - 1))) * (dLen - 1))
      if (idx < 0 || idx >= dLen) { setHover(null); return }
      const toY = (v: number) => pT + cH - ((v - yMin) / (yMax - yMin)) * cH
      setHover({ idx, clientX: e.clientX, clientY: e.clientY, py: toY(data[idx].value) })
    })
  }, [data, pointWidth])

  // The scroll wrapper's width drives scroll extent; canvas stays viewport-sized
  return (
    <div ref={scrollRef} style={{ position: 'relative', width: '100%', height: '100%', overflowX: pointWidth > 0 ? 'auto' : 'hidden', overflowY: 'hidden' }}>
      <div ref={containerRef} style={{ position: 'relative', height: '100%', width: pointWidth > 0 ? virtualWidth : '100%' }}>
        <canvas ref={canvasRef} style={{ display: 'block', width: '100%', height: '100%', cursor: 'crosshair' }} onMouseMove={onMove} onMouseLeave={() => setHover(null)} />
        {/* Legend */}
        {validOverlays.length > 0 && (
          <div style={{ position: 'absolute', top: 8, right: 24, display: 'flex', gap: 10, pointerEvents: 'none', zIndex: 1 }}>
            {validOverlays.map((ov, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 11, fontFamily: "var(--font-mono)", color: ov.color, opacity: 0.85, background: 'rgba(17,17,19,0.8)', padding: '2px 6px', borderRadius: 4 }}>
                <div style={{ width: 12, height: 2, background: ov.color, borderRadius: 1 }} />
                {ov.name} ({ov.unit})
              </div>
            ))}
          </div>
        )}
      </div>
      {/* Tooltip — fixed position relative to viewport, unaffected by scroll */}
      {hover && data[hover.idx] && (() => {
        const scrollEl = scrollRef.current
        const scrollRect = scrollEl ? scrollEl.getBoundingClientRect() : null
        if (!scrollRect) return null
        const allSeries: { name: string; val: string; color: string }[] = []
        allSeries.push({ name: 'Primary', val: data[hover.idx].value.toFixed(2) + (unit ? ' ' + unit : ''), color })
        for (let oi = 0; oi < validOverlays.length; oi++) {
          const ov = validOverlays[oi]
          const ovIdx = Math.min(Math.round((hover.idx / (data.length - 1)) * (ov.data.length - 1)), ov.data.length - 1)
          if (ov.data[ovIdx]) allSeries.push({ name: ov.name, val: ov.data[ovIdx].value.toFixed(2) + (ov.unit ? ' ' + ov.unit : ''), color: ov.color })
        }
        const tipW = 190, tipH = 28 + allSeries.length * 18
        const vpX = hover.clientX - scrollRect.left
        const vpY = hover.clientY - scrollRect.top
        const tipLeft = vpX + 16 + tipW > scrollRect.width ? vpX - tipW - 12 : vpX + 16
        const tipTop = Math.max(4, Math.min(vpY - tipH / 2, scrollRect.height - tipH - 4))
        return (<>
          <div style={{ position: 'fixed', left: hover.clientX, top: scrollRect.top, width: 1, height: scrollRect.height, background: 'rgba(255,255,255,0.12)', pointerEvents: 'none', zIndex: 9999 }} />
          <div style={{ position: 'fixed', left: hover.clientX - 5, top: hover.clientY - 5, width: 10, height: 10, borderRadius: '50%', background: color, border: '2px solid #fff', pointerEvents: 'none', zIndex: 9999 }} />
          <div style={{ position: 'fixed', left: scrollRect.left + tipLeft, top: scrollRect.top + tipTop, background: 'rgba(17,17,19,0.95)', border: '1px solid var(--border)', borderRadius: 8, padding: '8px 12px', pointerEvents: 'none', zIndex: 9999, fontFamily: "var(--font-mono)", fontSize: 12, whiteSpace: 'nowrap' }}>
            {allSeries.map((s, i) => (
              <div key={i} style={{ color: s.color, fontWeight: i === 0 ? 700 : 500, fontSize: i === 0 ? 14 : 11, marginTop: i > 0 ? 2 : 0 }}>
                {allSeries.length > 1 && <span style={{ fontSize: 10, opacity: 0.7 }}>{s.name}: </span>}{s.val}
              </div>
            ))}
            <div style={{ color: 'var(--muted)', fontSize: 11, marginTop: 2 }}>{new Date(data[hover.idx].ts).toLocaleString('en-US', { hour12: false, month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' })}</div>
          </div>
        </>)
      })()}
      {/* Hidden element to force scrollVersion re-render on scroll */}
      <span style={{ display: 'none' }}>{scrollVersion}</span>
    </div>
  )
}
