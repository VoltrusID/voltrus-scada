import { describe, expect, it } from 'vitest'
import { render } from '@testing-library/react'
import { SYMBOLS, SYMBOL_MAP } from '../../src/scada2/hmi/symbols'
import { DATA_SYMBOLS, SYMBOL_FILES } from '../../src/scada2/hmi/dataSymbols'
import { SymbolView, symbolLiveState, symbolTransform } from '../../src/scada2/hmi/SymbolView'
import { portPoint } from '../../src/scada2/hmi/resolve'
import { symbolSchema } from '../../src/scada2/widgets/SymbolWidget'
import type { SensorData } from '../../src/types'
import type { WidgetInstance } from '../../src/scada2/widgets/types'

const data = (id: number, value: number, status = 'ok'): SensorData => ({ id, key: `t${id}`, value, warn: 0, crit: 0, source_type: 'modbus', status })

function widget(symbol: string, config: Record<string, unknown> = {}, sensorId: number | null = 1): WidgetInstance {
  const def = SYMBOL_MAP[symbol]
  return { id: 'w1', type: 'symbol', sensorId, sensorIds: sensorId != null ? [sensorId] : [], x: 100, y: 100, width: def.size[0], height: def.size[1], config: { symbol, ...config } }
}

function draw(symbol: string, config: Record<string, unknown>, values: Record<string, SensorData>) {
  const w = widget(symbol, config)
  return render(
    <svg>
      <foreignObject width="400" height="400">
        <SymbolView def={SYMBOL_MAP[symbol]} instance={w} sensor={undefined} data={values['1']} sensors={[]} currentValues={values} editMode={false} selected={false} />
      </foreignObject>
    </svg>,
  ).container
}

describe('symbol library from symbols/*.json', () => {
  it('loads every symbol file (not the schema or style files) into the library, ISA-101 first in category order', () => {
    expect(SYMBOL_FILES.length).toBeGreaterThanOrEqual(35)
    expect(SYMBOL_FILES.every(f => f.key.startsWith('isa_'))).toBe(true)
    for (const k of ['isa_pump_centrifugal', 'isa_valve_3way', 'isa_tank_cone', 'isa_instrument', 'isa_pipe', 'isa_pump', 'isa_line']) {
      expect(SYMBOL_MAP[k]?.library).toBe('isa')
    }
    expect(SYMBOLS.filter(s => s.key === 'isa_motor')).toHaveLength(1)
    expect(DATA_SYMBOLS[0].category).toBe('Rotating equipment')
    expect(DATA_SYMBOLS.at(-1)!.category).toBe('Piping')
    // Shaded library still there.
    expect(SYMBOL_MAP.pump.library).toBeUndefined()
  })

  it('offers options, the run tag and the mode / interlock tags as editor fields', () => {
    const fields = (k: string) => symbolSchema(SYMBOL_MAP[k]).map(f => f.key)
    expect(fields('isa_valve_gate')).toEqual(expect.arrayContaining(['actuator', 'manualSensorId', 'interlockSensorId', 'faultSensorId', 'rotation', 'flipX', 'flipY']))
    expect(symbolSchema(SYMBOL_MAP.isa_valve_gate).find(f => f.key === 'actuator')!.options!.map(o => o.value)).toContain('diaphragm')
    expect(fields('isa_instrument')).toEqual(expect.arrayContaining(['mounting', 'tagFunction', 'loopNumber']))
    expect(fields('isa_filter')).toContain('runSensorId')
    expect(fields('isa_tank_vertical')).toContain('showPercent')
    expect(fields('isa_pipe')).not.toContain('flipY') // stretched symbols are not rotated or mirrored
  })
})

describe('drawing a data symbol', () => {
  const bodyFill = (c: HTMLElement) => [...c.querySelectorAll('path')].map(p => p.getAttribute('fill'))

  it('running is filled, stopped hollow, fault red and blinking', () => {
    expect(bodyFill(draw('isa_pump_centrifugal', {}, { 1: data(1, 1) }))).toContain('var(--isa-on)')
    const stopped = draw('isa_pump_centrifugal', {}, { 1: data(1, 0) })
    expect(bodyFill(stopped)).not.toContain('var(--isa-on)')
    expect(bodyFill(stopped)).toContain('var(--isa-off)')
    const fault = draw('isa_pump_centrifugal', { faultSensorId: 2 }, { 1: data(1, 1), 2: data(2, 1) })
    expect(bodyFill(fault)).toContain('var(--hmi-fault)')
    expect(fault.querySelector('g.hmi-blink')).not.toBeNull()
  })

  it('no data is magenta and dashed', () => {
    const c = draw('isa_valve_gate', {}, {})
    const body = [...c.querySelectorAll('path')].find(p => p.getAttribute('stroke') === 'var(--hmi-stale)')
    expect(body?.getAttribute('stroke-dasharray')).toBe('4 3')
  })

  it('draws the chosen actuator only', () => {
    const shells = (cfg: Record<string, unknown>) => draw('isa_valve_gate', cfg, { 1: data(1, 1) }).querySelectorAll('path').length
    expect(shells({ actuator: 'diaphragm' })).toBe(shells({}) + 2) // stem + dome
    const motor = draw('isa_valve_gate', { actuator: 'motor' }, { 1: data(1, 1) })
    expect([...motor.querySelectorAll('text')].map(t => t.textContent)).toContain('M')
  })

  it('shows manual, interlock and alarm priority badges', () => {
    const c = draw('isa_pump_centrifugal', { manualSensorId: 5, interlockSensorId: 6 }, { 1: data(1, 1, 'crit'), 5: data(5, 1), 6: data(6, 1) })
    expect([...c.querySelectorAll('[data-badge]')].map(b => b.getAttribute('data-badge'))).toEqual(['alarm', 'manual', 'interlock'])
    const none = draw('isa_pump_centrifugal', { manualSensorId: 5 }, { 1: data(1, 1), 5: data(5, 0) })
    expect(none.querySelector('[data-testid=symbol-badges]')).toBeNull()
    const live = symbolLiveState(SYMBOL_MAP.isa_pump_centrifugal, { manualSensorId: 5 }, 1, undefined, data(1, 1, 'warn'), { 5: data(5, 1) })
    expect(live).toMatchObject({ manual: true, interlocked: false, alarmPriority: 2, state: 'running' })
  })

  it('fills the level, clipped to the vessel, and prints the percent', () => {
    const c = draw('isa_tank_cone', {}, { 1: data(1, 25) })
    const rect = c.querySelector('rect[fill="var(--isa-level)"]')!
    expect(rect.getAttribute('clip-path')).toMatch(/^url\(#/)
    expect([...c.querySelectorAll('text')].map(t => t.textContent)).toContain('25%')
    expect([...draw('isa_tank_cone', { showPercent: false }, { 1: data(1, 25) }).querySelectorAll('text')].map(t => t.textContent)).not.toContain('25%')
  })

  it('keeps text upright when rotated or mirrored', () => {
    const c = draw('isa_instrument', { rotation: 90, flipX: true, tagFunction: 'FT', loopNumber: '320' }, { 1: data(1, 3) })
    const t = [...c.querySelectorAll('text')].find(x => x.textContent === 'FT')!
    expect(t.getAttribute('transform')).toContain('rotate(-90)')
    expect(t.getAttribute('transform')).toContain('scale(-1 1)')
  })
})

describe('mirroring', () => {
  it('flipY flips the drawing and the nozzles top to bottom', () => {
    expect(symbolTransform({ rotation: 90, flipY: true })).toBe('rotate(90deg) scaleY(-1)')
    expect(symbolTransform({})).toBeUndefined()
    const w = widget('isa_valve_3way', {}, null)
    const [bx, by] = portPoint(w, 'bottom')
    const flipped = widget('isa_valve_3way', { flipY: true }, null)
    const [tx, ty] = portPoint(flipped, 'top')
    expect(tx).toBeCloseTo(bx)
    expect(ty - w.y).toBeCloseTo(w.height - (by - w.y), 0)
    // Mirrored top to bottom then rotated 90°: the branch (local bottom) faces left.
    const both = widget('isa_valve_3way', { flipY: true, rotation: 90 }, null)
    const [lx] = portPoint(both, 'left')
    expect(lx).toBeLessThan(both.x + both.width / 2)
  })
})
