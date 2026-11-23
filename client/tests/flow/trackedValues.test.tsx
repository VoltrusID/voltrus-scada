import { describe, expect, it } from 'vitest'
import { render, act } from '@testing-library/react'
import { createDashboardStore, DashboardStoreProvider, useTrackedCurrentValues, useSensorDataPointsMany } from '../../src/store/dashboardStore'
import type { SensorData } from '../../src/types'

const v = (id: number, value: number) => ({ id, points: [{ ts: value, value }], value: { id, key: `t${id}`, value, warn: 0, crit: 0, source_type: 'sim', status: 'normal' } as SensorData })

describe('per-widget live subscriptions', () => {
  it('re-render only for the tags a component read', () => {
    const store = createDashboardStore()
    store.mergeSensorData([v(1, 10), v(2, 20)])
    let renders = 0
    let seen: number | undefined
    function Widget() {
      const values = useTrackedCurrentValues()
      renders += 1
      seen = values['1']?.value
      return null
    }
    render(<DashboardStoreProvider value={store}><Widget /></DashboardStoreProvider>)
    expect(renders).toBe(1)
    expect(seen).toBe(10)

    act(() => store.mergeSensorData([v(2, 21)]))
    act(() => store.mergeSensorData([v(3, 30)]))
    expect(renders).toBe(1)

    act(() => store.mergeSensorData([v(1, 11)]))
    expect(renders).toBe(2)
    expect(seen).toBe(11)
  })

  it('a newly read tag gives its current value, not the one at the last render', () => {
    const store = createDashboardStore()
    store.mergeSensorData([v(1, 1), v(2, 2)])
    let readTwo = false
    let two: number | undefined
    function Widget({ flag }: { flag: boolean }) {
      const values = useTrackedCurrentValues()
      void values['1']
      if (flag) two = values['2']?.value
      return null
    }
    const { rerender } = render(<DashboardStoreProvider value={store}><Widget flag={readTwo} /></DashboardStoreProvider>)
    act(() => store.mergeSensorData([v(2, 22)]))
    readTwo = true
    rerender(<DashboardStoreProvider value={store}><Widget flag={readTwo} /></DashboardStoreProvider>)
    expect(two).toBe(22)
  })

  it('a component that lists every value follows every change', () => {
    const store = createDashboardStore()
    store.mergeSensorData([v(1, 1)])
    let renders = 0
    function Table() {
      const values = useTrackedCurrentValues()
      renders += 1
      return <span>{Object.keys(values).length}</span>
    }
    const { container } = render(<DashboardStoreProvider value={store}><Table /></DashboardStoreProvider>)
    act(() => store.mergeSensorData([v(2, 2)]))
    expect(renders).toBe(2)
    expect(container.textContent).toBe('2')
  })

  it('the record is read-only', () => {
    const store = createDashboardStore()
    let values: Record<string, SensorData> = {}
    function Grab() { values = useTrackedCurrentValues(); return null }
    render(<DashboardStoreProvider value={store}><Grab /></DashboardStoreProvider>)
    expect(() => { (values as Record<string, unknown>)['1'] = 1 }).toThrow()
  })

  it('points of several tags update only when one of theirs changes', () => {
    const store = createDashboardStore()
    store.mergeSensorData([v(1, 1), v(2, 2)])
    let renders = 0
    function Overlays() { useSensorDataPointsMany([1]); renders += 1; return null }
    render(<DashboardStoreProvider value={store}><Overlays /></DashboardStoreProvider>)
    act(() => store.mergeSensorData([v(2, 3)]))
    expect(renders).toBe(1)
    act(() => store.mergeSensorData([v(1, 4)]))
    expect(renders).toBe(2)
  })
})
