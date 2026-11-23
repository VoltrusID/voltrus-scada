import { describe, expect, it } from 'vitest'
import type { LineData, Time } from 'lightweight-charts'
import { planSeriesUpdate } from '../../src/TvDetailChart'

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
