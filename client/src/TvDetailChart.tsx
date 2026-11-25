import { useEffect, useRef, useState, useCallback } from 'react'
import {
  createChart,
  createSeriesMarkers,
  LineSeries,
  LineType,
  CrosshairMode,
  type IChartApi,
  type ISeriesApi,
  type LineData,
  type Time,
  type SeriesMarker,
} from 'lightweight-charts'
import { DataPoint } from './types'
import { toHexColor } from './styles'
import { bandPoints } from './historyQuery'
import { MinMaxBand } from './minMaxBand'
import { axisLabel, overlayScales } from './trend/overlayAxes'

export interface OverlaySeries {
  data: DataPoint[]
  color: string
  name: string
  unit: string
}

export interface AnnotationMarker {
  id: number
  time: number // epoch seconds
  text: string
}

function toTvData(points: DataPoint[]): LineData<Time>[] {
  // Filter null/undefined/NaN — lightweight-charts LineSeries throws
  // "Value is null" when fed a point whose value isn't a finite number
  // (telemetry gaps / missing register reads arrive as JSON null despite
  // the `number` type). Also enforce ascending unique time, which the
  // library requires for both setData and update.
  const seen = new Map<number, LineData<Time>>()
  for (const p of points) {
    if (p.value === null || p.value === undefined || !Number.isFinite(p.value)) continue
    const t = p.ts / 1000
    seen.set(t, { time: t as Time, value: p.value })
  }
  return [...seen.values()].sort((a, b) => (a.time as number) - (b.time as number))
}

/** What to do with the series when new data arrives. */
export type SeriesPlan =
  | { kind: 'set' }
  | { kind: 'append'; points: LineData<Time>[] }
  | { kind: 'none' }

/** First and last time drawn so far. */
export interface DrawnSpan { first: number; last: number }

/**
 * Compare by time, not by length: the live buffer is capped, so once full it
 * keeps its length while sliding forward. Points newer than the last drawn are
 * appended; data that starts or ends earlier than what is drawn replaced it
 * (a history load) and is set in full.
 */
export function planSeriesUpdate(drawn: DrawnSpan | null, next: LineData<Time>[]): SeriesPlan {
  if (next.length === 0) return { kind: 'none' }
  if (!drawn) return { kind: 'set' }
  const first = next[0].time as number
  const last = next[next.length - 1].time as number
  if (first < drawn.first || last < drawn.last) return { kind: 'set' }
  if (last === drawn.last) return { kind: 'none' }
  return { kind: 'append', points: next.filter(p => (p.time as number) > drawn.last) }
}

/**
 * Live data appends (see planSeriesUpdate). History is a fresh answer each
 * time: a narrower range or another aggregation ends inside what is drawn, so
 * any new history array (or a reset key change) redraws it in full, empty included.
 */
export function planChartUpdate(o: {
  live: boolean
  keyChanged: boolean
  dataChanged: boolean
  drawn: DrawnSpan | null
  next: LineData<Time>[]
}): SeriesPlan {
  if (!o.live) return o.dataChanged || o.keyChanged || !o.drawn ? { kind: 'set' } : { kind: 'none' }
  if (o.next.length === 0) return { kind: 'none' }
  if (o.keyChanged) return { kind: 'set' }
  return planSeriesUpdate(o.drawn, o.next)
}

/** A series' axis and crosshair labels: two decimals and its own unit. */
function unitPriceFormat(unit: string) {
  return { type: 'custom' as const, minMove: 0.01, formatter: (p: number) => axisLabel(p, unit) }
}

interface Props {
  data: DataPoint[]
  /** Live data (appended as it arrives); false for history, which is redrawn whenever it changes. */
  live?: boolean
  /** Changing this (mode, range) redraws the series from scratch instead of appending. */
  resetKey?: string
  color: string
  warn: number
  crit: number
  unit: string
  overlays?: OverlaySeries[]
  annotations?: AnnotationMarker[]
  onAnnotationClick?: (time: number) => void
}

export function TvDetailChart({
  data: rawData,
  resetKey = '',
  live = true,
  color: rawColor,
  warn,
  crit,
  unit,
  overlays = [],
  annotations = [],
  onAnnotationClick,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null)
  const chartRef = useRef<IChartApi | null>(null)
  const seriesRef = useRef<ISeriesApi<'Line'> | null>(null)
  const bandRef = useRef<MinMaxBand | null>(null)
  const overlayRefs = useRef<ISeriesApi<'Line'>[]>([])
  const warnLineRef = useRef<ReturnType<ISeriesApi<'Line'>['createPriceLine']> | null>(null)
  const critLineRef = useRef<ReturnType<ISeriesApi<'Line'>['createPriceLine']> | null>(null)
  const markersPluginRef = useRef<ReturnType<typeof createSeriesMarkers<Time>> | null>(null)
  const drawnRef = useRef<DrawnSpan | null>(null)
  const drawnKeyRef = useRef(resetKey)
  const drawnDataRef = useRef<DataPoint[] | null>(null)
  const [userScrolled, setUserScrolled] = useState(false)

  const color = toHexColor(rawColor)

  // Initialise chart once
  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const chart = createChart(container, {
      layout: {
        background: { color: 'transparent' },
        textColor: '#d1d5db',
        attributionLogo: true,
      },
      grid: {
        vertLines: { color: '#1a1a1e' },
        horzLines: { color: '#1a1a1e' },
      },
      crosshair: {
        mode: CrosshairMode.Normal,
        vertLine: { color: 'rgba(255,255,255,0.12)', width: 1, style: 2 },
        horzLine: { color: 'rgba(255,255,255,0.12)', width: 1, style: 2 },
      },
      rightPriceScale: {
        borderColor: '#27272a',
        scaleMargins: { top: 0.1, bottom: 0.1 },
      },
      timeScale: {
        borderColor: '#27272a',
        timeVisible: true,
        secondsVisible: true,
        rightOffset: 12,
        barSpacing: 6,
        fixLeftEdge: false,
        fixRightEdge: false,
      },
      autoSize: true,
      handleScroll: { vertTouchDrag: false },
    })
    chartRef.current = chart

    const series = chart.addSeries(LineSeries, {
      color,
      lineWidth: 2,
      lineType: LineType.Curved,
      lastValueVisible: true,
      priceLineVisible: false,
      title: unit || undefined,
      priceFormat: unitPriceFormat(unit),
    })
    seriesRef.current = series

    // Min/max band of aggregated history, drawn behind the line
    const band = new MinMaxBand(color + '2e')
    series.attachPrimitive(band)
    bandRef.current = band

    // Create markers plugin on the series
    markersPluginRef.current = createSeriesMarkers(series, [], {
      zOrder: 'top',
    })

    // Threshold lines
    if (warn > 0) {
      warnLineRef.current = series.createPriceLine({
        price: warn,
        color: 'rgba(234,179,8,0.5)',
        lineWidth: 1,
        lineStyle: 2,
        title: 'WARN',
        axisLabelVisible: true,
      })
    }
    if (crit > 0) {
      critLineRef.current = series.createPriceLine({
        price: crit,
        color: 'rgba(239,68,68,0.5)',
        lineWidth: 1,
        lineStyle: 2,
        title: 'CRIT',
        axisLabelVisible: true,
      })
    }

    // Track when user manually scrolls/pans the time scale
    chart.timeScale().subscribeVisibleLogicalRangeChange(() => {
      // We detect user scroll by checking if they're not at the right edge
      const vr = chart.timeScale().getVisibleLogicalRange()
      if (vr) {
        const barsInfo = series.barsInLogicalRange(vr)
        if (barsInfo) {
          setUserScrolled(barsInfo.barsAfter > 3)
        }
      }
    })

    const handleResize = () => {
      chart.applyOptions({ width: container.clientWidth, height: container.clientHeight })
    }
    const ro = new ResizeObserver(handleResize)
    ro.observe(container)

    return () => {
      ro.disconnect()
      chart.remove()
      chartRef.current = null
      seriesRef.current = null
      bandRef.current = null
      overlayRefs.current = []
      markersPluginRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Update primary data — incremental update for live, full set only on initial load
  useEffect(() => {
    const series = seriesRef.current
    const chart = chartRef.current
    if (!series || !chart) return

    // Filter out initial zero-valued points (transition from 0)
    const filtered = rawData.length > 1 && rawData[0].value === 0
      ? rawData.slice(1)
      : rawData

    const tvData = toTvData(filtered)

    const keyChanged = drawnKeyRef.current !== resetKey
    const dataChanged = drawnDataRef.current !== rawData
    drawnKeyRef.current = resetKey
    drawnDataRef.current = rawData
    const firstDraw = drawnRef.current === null
    const plan = planChartUpdate({ live, keyChanged, dataChanged, drawn: drawnRef.current, next: tvData })
    if (plan.kind === 'none') return
    drawnRef.current = tvData.length ? { first: tvData[0].time as number, last: tvData[tvData.length - 1].time as number } : null

    if (plan.kind === 'append') {
      // Live update: only the new points, the view stays where the user left it.
      for (const pt of plan.points) series.update(pt)
      if (!userScrolled) chart.timeScale().scrollToRealTime()
      return
    }
    // First load, or data replaced (mode change, history reload).
    series.setData(tvData)
    if (firstDraw || !userScrolled) chart.timeScale().fitContent()
  }, [rawData, userScrolled, resetKey, live])

  // Min/max band: only history answered with `agg=minmax` carries one
  useEffect(() => {
    bandRef.current?.setData(bandPoints(rawData))
  }, [rawData])

  // Reset handler
  const handleReset = useCallback(() => {
    const chart = chartRef.current
    if (!chart) return
    setUserScrolled(false)
    chart.timeScale().fitContent()
  }, [])

  // Update overlays
  useEffect(() => {
    const chart = chartRef.current
    if (!chart) return
    overlayRefs.current.forEach(s => chart.removeSeries(s))
    overlayRefs.current = []

    // One axis per unit (trend/overlayAxes.ts): small-range tags are not flattened by a big one.
    const scales = overlayScales(unit, overlays.map(ov => ov.unit))
    chart.applyOptions({
      leftPriceScale: { visible: scales.some(sc => sc.side === 'left'), borderColor: '#27272a', scaleMargins: { top: 0.1, bottom: 0.1 } },
    })
    overlayRefs.current = overlays.map((ov, i) => {
      const scale = scales[i]
      const s = chart.addSeries(LineSeries, {
        color: toHexColor(ov.color),
        lineWidth: 1,
        lineType: LineType.Curved,
        // A scale of its own draws no axis: no label on another unit's axis, the legend shows its value.
        lastValueVisible: scale.side !== 'own',
        priceLineVisible: false,
        title: scale.side === 'own' ? '' : ov.name,
        priceScaleId: scale.id,
        priceFormat: unitPriceFormat(ov.unit),
      })
      if (scale.side === 'own') chart.priceScale(scale.id).applyOptions({ scaleMargins: { top: 0.1, bottom: 0.1 } })
      s.setData(toTvData(ov.data))
      return s
    })
  }, [overlays, unit])

  // Update annotation markers natively on the chart
  useEffect(() => {
    const plugin = markersPluginRef.current
    if (!plugin) return

    if (annotations.length === 0) {
      plugin.setMarkers([])
      return
    }

    // Sort by time (required by lightweight-charts)
    const sorted = [...annotations].sort((a, b) => a.time - b.time)

    const markers: SeriesMarker<Time>[] = sorted.map(ann => ({
      time: ann.time as Time,
      position: 'belowBar' as const,
      color: '#60a5fa',
      shape: 'arrowUp' as const,
      text: ann.text.length > 20 ? ann.text.slice(0, 20) + '…' : ann.text,
      id: String(ann.id),
      size: 1,
    }))

    plugin.setMarkers(markers)
  }, [annotations])

  // Update threshold lines when warn/crit changes
  useEffect(() => {
    const series = seriesRef.current
    if (!series) return
    if (warnLineRef.current) series.removePriceLine(warnLineRef.current)
    if (critLineRef.current) series.removePriceLine(critLineRef.current)

    if (warn > 0) {
      warnLineRef.current = series.createPriceLine({
        price: warn,
        color: 'rgba(234,179,8,0.5)',
        lineWidth: 1,
        lineStyle: 2,
        title: 'WARN',
        axisLabelVisible: true,
      })
    }
    if (crit > 0) {
      critLineRef.current = series.createPriceLine({
        price: crit,
        color: 'rgba(239,68,68,0.5)',
        lineWidth: 1,
        lineStyle: 2,
        title: 'CRIT',
        axisLabelVisible: true,
      })
    }
  }, [warn, crit])

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%' }}>
      <div
        ref={containerRef}
        onContextMenu={onAnnotationClick ? (e) => {
          e.preventDefault();
          const chart = chartRef.current;
          if (!chart) return;
          const rect = containerRef.current?.getBoundingClientRect();
          const x = rect ? e.clientX - rect.left : e.clientX;
          const t = chart.timeScale().coordinateToTime(x);
          if (t !== null && typeof t === 'number') onAnnotationClick(t);
        } : undefined}
        style={{ width: '100%', height: '100%' }}
      />
      {userScrolled && (
        <button
          onClick={handleReset}
          style={{
            position: 'absolute',
            top: 8,
            left: '50%',
            transform: 'translateX(-50%)',
            padding: '4px 12px',
            borderRadius: 6,
            border: '1px solid var(--border)',
            background: 'var(--card)',
            color: 'var(--text)',
            fontSize: 11,
            fontWeight: 600,
            fontFamily: 'Inter, sans-serif',
            cursor: 'pointer',
            boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
            zIndex: 10,
            display: 'flex',
            alignItems: 'center',
            gap: 4,
          }}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polyline points="1 4 1 10 7 10"/><polyline points="23 20 23 14 17 14"/><path d="M20.49 9A9 9 0 0 0 5.64 5.64L1 10m22 4l-4.64 4.36A9 9 0 0 1 3.51 15"/></svg>
          Reset
        </button>
      )}
    </div>
  )
}
