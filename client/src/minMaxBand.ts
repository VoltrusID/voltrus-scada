import type {
  IChartApiBase,
  IPrimitivePaneRenderer,
  IPrimitivePaneView,
  ISeriesApi,
  ISeriesPrimitive,
  SeriesAttachedParameter,
  Time,
} from 'lightweight-charts'
import type { BandPoint } from './historyQuery'

type Target = Parameters<IPrimitivePaneRenderer['draw']>[0]

/**
 * Fills the area between a series' min and max edges, drawn behind the series
 * line: the "average line with min/max band" of the history aggregation.
 */
export class MinMaxBand implements ISeriesPrimitive<Time> {
  private chart: IChartApiBase<Time> | null = null
  private series: ISeriesApi<'Line'> | null = null
  private requestUpdate: (() => void) | null = null
  private band: BandPoint[] = []

  constructor(private readonly fill: string) {}

  attached(param: SeriesAttachedParameter<Time>) {
    this.chart = param.chart
    this.series = param.series as ISeriesApi<'Line'>
    this.requestUpdate = param.requestUpdate
  }

  detached() {
    this.chart = null
    this.series = null
    this.requestUpdate = null
  }

  setData(band: BandPoint[]) {
    this.band = band
    this.requestUpdate?.()
  }

  private readonly renderer: IPrimitivePaneRenderer = {
    draw: (target: Target) => {
      const { chart, series, band } = this
      if (!chart || !series || band.length === 0) return
      const timeScale = chart.timeScale()
      target.useBitmapCoordinateSpace(({ context, horizontalPixelRatio: hr, verticalPixelRatio: vr }) => {
        const top: [number, number][] = []
        const bottom: [number, number][] = []
        for (const p of band) {
          const x = timeScale.timeToCoordinate(p.time as Time)
          const yHi = series.priceToCoordinate(p.hi)
          const yLo = series.priceToCoordinate(p.lo)
          if (x === null || yHi === null || yLo === null) continue
          top.push([x * hr, yHi * vr])
          bottom.push([x * hr, yLo * vr])
        }
        if (top.length < 2) return
        context.beginPath()
        context.moveTo(top[0][0], top[0][1])
        for (const [x, y] of top) context.lineTo(x, y)
        for (let i = bottom.length - 1; i >= 0; i--) context.lineTo(bottom[i][0], bottom[i][1])
        context.closePath()
        context.fillStyle = this.fill
        context.fill()
      })
    },
  }

  private readonly view: IPrimitivePaneView = {
    zOrder: () => 'bottom',
    renderer: () => this.renderer,
  }

  paneViews() {
    return [this.view]
  }
}
