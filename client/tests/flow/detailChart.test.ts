import { describe, expect, it } from 'vitest'
import type { LineData, Time } from 'lightweight-charts'
import { planChartUpdate, planSeriesUpdate } from '../../src/TvDetailChart'

const pts = (from: number, to: number): LineData<Time>[] =>
  Array.from({ length: to - from + 1 }, (_, i) => ({ time: (from + i) as Time, value: from + i }))
const span = (d: LineData<Time>[]) => ({ first: d[0].time as number, last: d[d.length - 1].time as number })

describe('detail chart updates', () => {
  it('keeps appending once the live buffer is full and slides', () => {
    // A full 300-point buffer, then the next sample: same length, one newer point.
    const full = pts(1, 300)
    const slid = pts(2, 301)
    expect(slid.length).toBe(full.length)
    const plan = planSeriesUpdate(span(full), slid)
    expect(plan.kind).toBe('append')
    expect(plan.kind === 'append' && plan.points.map(p => p.time)).toEqual([301])
  })

  it('appends every point newer than the last drawn, even several at once', () => {
    const plan = planSeriesUpdate({ first: 1, last: 300 }, pts(5, 304))
    expect(plan.kind === 'append' && plan.points.map(p => p.time)).toEqual([301, 302, 303, 304])
  })

  it('does nothing when no newer point arrived', () => {
    expect(planSeriesUpdate({ first: 1, last: 300 }, pts(1, 300)).kind).toBe('none')
    expect(planSeriesUpdate({ first: 1, last: 300 }, []).kind).toBe('none')
  })

  it('redraws when the data reaches further back (history replaced live)', () => {
    expect(planSeriesUpdate({ first: 100, last: 300 }, pts(1, 400)).kind).toBe('set')
    expect(planSeriesUpdate({ first: 1, last: 300 }, pts(1, 200)).kind).toBe('set')
    expect(planSeriesUpdate(null, pts(1, 3)).kind).toBe('set')
  })
})

describe('history in the detail chart', () => {
  const drawn24h = { first: 0, last: 86_400 }
  it('a narrower range (24h to 1h) replaces what is drawn when its answer arrives', () => {
    const oneHour = pts(82_800, 86_400)
    // The key changes first, with the old data still in hand...
    expect(planChartUpdate({ live: false, keyChanged: true, dataChanged: false, drawn: drawn24h, next: pts(0, 10) }).kind).toBe('set')
    // ...then the new answer ends where the old one did, and must still be drawn in full.
    expect(planChartUpdate({ live: false, keyChanged: false, dataChanged: true, drawn: drawn24h, next: oneHour }).kind).toBe('set')
  })

  it('another aggregation with the same span is redrawn', () => {
    expect(planChartUpdate({ live: false, keyChanged: false, dataChanged: true, drawn: drawn24h, next: pts(0, 86_400) }).kind).toBe('set')
  })

  it('an empty answer clears the chart', () => {
    expect(planChartUpdate({ live: false, keyChanged: false, dataChanged: true, drawn: drawn24h, next: [] }).kind).toBe('set')
  })

  it('nothing is redrawn when the history data did not change', () => {
    expect(planChartUpdate({ live: false, keyChanged: false, dataChanged: false, drawn: drawn24h, next: pts(0, 5) }).kind).toBe('none')
  })

  it('live data still appends', () => {
    const plan = planChartUpdate({ live: true, keyChanged: false, dataChanged: true, drawn: { first: 1, last: 300 }, next: pts(2, 301) })
    expect(plan.kind).toBe('append')
  })
})
