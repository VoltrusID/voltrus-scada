// The 2D runtime path of Designer bindings: a widget's bound visible and
// opacity, a bound label in the drawing, and a widget that throws (in its own
// drawing or anywhere in its binding path) showing a placeholder while the
// rest of the screen keeps running.
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import WidgetShell from '../../src/scada2/widgets/WidgetShell'
import { registerWidget, WIDGET_MANIFESTS } from '../../src/scada2/widgets/registry'
import { withBinding } from '../../src/designer/bindings'
import type { WidgetInstance, WidgetProps } from '../../src/scada2/widgets/types'
import type { SensorConfig, SensorData } from '../../src/types'

const sensors = [
  { id: 1, key: 'run', name: 'Run', device_id: 'p', unit: '', min_val: 0, max_val: 1 },
  { id: 2, key: 'level', name: 'Level', device_id: 'p', unit: '%', min_val: 0, max_val: 100 },
] as unknown as SensorConfig[]

function Probe({ instance }: WidgetProps) {
  if (instance.config?.label === 'BOOM') throw new Error('widget bug')
  return <span data-testid={`probe-${instance.id}`}>{String(instance.config?.label ?? '')}</span>
}
registerWidget({ manifest: { ...WIDGET_MANIFESTS.label, type: 'label' }, component: Probe })

const values = (run: number, level: number) => ({ '1': { value: run }, '2': { value: level } }) as unknown as Record<string, SensorData>

function shell(instance: WidgetInstance, v: Record<string, SensorData>, editMode = false) {
  return (
    <div style={{ position: 'relative', width: 1920, height: 1080 }}>
      <WidgetShell instance={instance} sensor={undefined} data={undefined} sensors={sensors} currentValues={v}
        editMode={editMode} selected={false} scale={1} onSelect={() => {}} onMove={() => {}} onResize={() => {}} />
    </div>
  )
}

const inst = (id: string, config: Record<string, any>): WidgetInstance =>
  ({ id, type: 'label', sensorId: null, sensorIds: [], x: 10, y: 10, width: 100, height: 40, config })

afterEach(() => vi.restoreAllMocks())

describe('WidgetShell at runtime', () => {
  it('bound visible hides and shows the widget; bound opacity is clamped', () => {
    let config = withBinding({ label: 'fixed' }, 'visible', { kind: 'tag', tag: 'p/run' })
    config = withBinding(config, 'opacity', { kind: 'expr', expr: '{p/level} / 50' })
    const w = inst('w', config)
    const { rerender } = render(shell(w, values(0, 25)))
    const box = () => screen.getByTestId('probe-w').closest('.react-draggable') as HTMLElement
    expect(box().style.display).toBe('none')
    expect(box().style.opacity).toBe('0.5')
    rerender(shell(w, values(1, 80)))
    expect(box().style.display).toBe('flex')
    expect(box().style.opacity).toBe('1')
  })

  it('a bound label reaches the drawing; in edit mode the saved value shows', () => {
    const w = inst('w', withBinding({ label: 'fixed' }, 'label', { kind: 'expr', expr: '"L=" + text({p/level}, 1)' }))
    const { rerender } = render(shell(w, values(1, 42)))
    expect(screen.getByTestId('probe-w').textContent).toBe('L=42.0')
    rerender(shell(w, values(1, 42), true))
    expect(screen.getByTestId('probe-w').textContent).toBe('fixed')
  })

  it('a binding that would throw leaves the fixed value', () => {
    const w = inst('w', withBinding({ label: 'fixed' }, 'label', { kind: 'expr', expr: 'text({p/level}, {p/run} * 1e9)' }))
    render(shell(w, values(1, 42)))
    expect(screen.getByTestId('probe-w').textContent).toBe('42.00000000000000000000')
    const bad = inst('x', { label: 'fixed', bindings: { label: { kind: 'expr', expr: '('.repeat(500) } } })
    render(shell(bad, values(1, 42)))
    expect(screen.getByTestId('probe-x').textContent).toBe('fixed')
  })

  it('a widget that throws shows a placeholder; its neighbours keep drawing', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const broken = inst('b', withBinding({ label: 'ok' }, 'label', { kind: 'expr', expr: '{p/run} ? "BOOM" : "ok"' }))
    const fine = inst('f', { label: 'neighbour' })
    const draw = (run: number) => (
      <div style={{ position: 'relative' }}>
        {shell(broken, values(run, 0))}
        {shell(fine, values(run, 0))}
      </div>
    )
    const { rerender } = render(draw(0))
    expect(screen.getByTestId('probe-b').textContent).toBe('ok')
    rerender(draw(1))
    expect(screen.getByTestId('widget-failed').textContent).toContain('failed to draw')
    expect(screen.getByTestId('probe-f').textContent).toBe('neighbour')
  })
})
