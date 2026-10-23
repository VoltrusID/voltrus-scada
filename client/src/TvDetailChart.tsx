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
  type LineStyleOptions,
  type SeriesOptionsCommon,
  type Time,
  type SeriesMarker,
} from 'lightweight-charts'
import { DataPoint } from './types'
import { toHexColor } from './styles'

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

interface Props {
  data: DataPoint[]
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
  const overlayRefs = useRef<ISeriesApi<'Line'>[]>([])
  const warnLineRef = useRef<ReturnType<ISeriesApi<'Line'>['createPriceLine']> | null>(null)
  const critLineRef = useRef<ReturnType<ISeriesApi<'Line'>['createPriceLine']> | null>(null)
  const markersPluginRef = useRef<ReturnType<typeof createSeriesMarkers<Time>> | null>(null)
  const prevLenRef = useRef(0)
  const initialDataSet = useRef(false)
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
    } as LineStyleOptions & SeriesOptionsCommon)
    seriesRef.current = series

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

    chart.applyOptions({
      localization: {
        priceFormatter: (p: number) => p.toFixed(2) + (unit ? ' ' + unit : ''),
      },
    })

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
    if (tvData.length === 0) return

    const prevLen = prevLenRef.current
    prevLenRef.current = tvData.length

    if (!initialDataSet.current) {
      // First load — set all data and fit
      series.setData(tvData)
      initialDataSet.current = true
      chart.timeScale().fitContent()
    } else if (tvData.length > prevLen) {
      // Live update — only append new points, don't reset view
      const newPoints = tvData.slice(prevLen)
      for (const pt of newPoints) {
        series.update(pt)
      }

      // Only scroll to latest if user hasn't manually panned
      if (!userScrolled) {
        chart.timeScale().scrollToRealTime()
      }
    } else if (tvData.length !== prevLen) {
      // Data was replaced (mode change, history reload) — full reset
      series.setData(tvData)
      if (!userScrolled) {
        chart.timeScale().fitContent()
      }
    }
  }, [rawData, userScrolled])

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

    overlayRefs.current = overlays.map(ov => {
      const s = chart.addSeries(LineSeries, {
        color: toHexColor(ov.color),
        lineWidth: 1,
        lineType: LineType.Curved,
        lastValueVisible: false,
        priceLineVisible: false,
        title: ov.name,
      } as LineStyleOptions & SeriesOptionsCommon)
      s.setData(toTvData(ov.data))
      return s
    })
  }, [overlays])

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
