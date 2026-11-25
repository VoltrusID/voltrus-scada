import type { ConfigFieldDef } from '../widgets/types'
import type { EquipState, SymbolDef, SymbolRenderProps } from './symbols'

// ISA-101 high-performance symbols: flat, grey, thin outlines, no gradients
// or 3D shading. Colour appears only for abnormal state (fault red, stale
// magenta); running vs stopped is fill vs hollow, so the state still reads
// in greyscale and for colour-blind operators.
//
// Same SymbolDef contract as the shaded library (symbols.tsx), so they bind,
// open faceplates, connect at ports and save the same way. Keys start with
// `isa_` and they sit in their own palette category.

const thin = { vectorEffect: 'non-scaling-stroke' as const, strokeLinejoin: 'round' as const }

function stroke(state: EquipState) {
  return {
    stroke: state === 'fault' ? 'var(--hmi-fault)' : state === 'stale' ? 'var(--hmi-stale)' : 'var(--isa-line)',
    strokeWidth: state === 'fault' ? 2.5 : 1.75,
    strokeDasharray: state === 'stale' ? '4 3' : undefined,
    ...thin,
  }
}

/** Running / open: filled. Stopped, idle, unknown: hollow. Fault: the fill turns red. */
const fill = (state: EquipState) => (state === 'running' ? 'var(--isa-on)' : state === 'fault' ? 'var(--hmi-fault)' : 'var(--isa-off)')

function Group({ p, children }: { p: SymbolRenderProps; children: React.ReactNode }) {
  return <g className={p.state === 'fault' ? 'hmi-blink' : undefined}>{children}</g>
}

const isaPump: SymbolDef = {
  key: 'isa_pump', name: 'Pump (ISA-101)', category: 'ISA-101', viewBox: [100, 100], size: [90, 104], binding: 'state',
  ports: { left: [12, 54], right: [94, 18] },
  render: p => (
    <Group p={p}>
      <path d="M50 54 L50 18 L94 18 L94 30 L72 30" fill="none" {...stroke(p.state)} />
      <circle cx="50" cy="54" r="34" fill={fill(p.state)} {...stroke(p.state)} />
      <path d="M30 82 L22 96 L78 96 L70 82" fill="none" {...stroke(p.state)} />
      <line x1="2" y1="54" x2="16" y2="54" {...stroke(p.state)} />
    </Group>
  ),
}

const isaMotor: SymbolDef = {
  key: 'isa_motor', name: 'Motor (ISA-101)', category: 'ISA-101', viewBox: [100, 100], size: [80, 96], binding: 'state',
  ports: { right: [96, 50], top: [50, 4] },
  render: p => (
    <Group p={p}>
      <line x1="84" y1="50" x2="98" y2="50" {...stroke(p.state)} />
      <circle cx="50" cy="50" r="34" fill={fill(p.state)} {...stroke(p.state)} />
      <text x="50" y="51" textAnchor="middle" dominantBaseline="central" fontSize="30" fontWeight={600}
        fill={p.state === 'running' || p.state === 'fault' ? 'var(--hmi-stage)' : 'var(--isa-line)'} style={{ fontFamily: 'var(--font-ui)' }}>M</text>
    </Group>
  ),
}

const isaValve: SymbolDef = {
  key: 'isa_valve', name: 'Valve (ISA-101)', category: 'ISA-101', viewBox: [100, 70], size: [90, 92], binding: 'state',
  ports: { left: [4, 45], right: [96, 45] },
  render: p => (
    <Group p={p}>
      <path d="M8 25 L50 45 L8 65 Z M92 25 L50 45 L92 65 Z" fill={fill(p.state)} {...stroke(p.state)} />
      <line x1="50" y1="45" x2="50" y2="14" {...stroke(p.state)} />
      <line x1="38" y1="12" x2="62" y2="12" {...stroke(p.state)} />
    </Group>
  ),
}

const isaControlValve: SymbolDef = {
  key: 'isa_control_valve', name: 'Control Valve (ISA-101)', category: 'ISA-101', viewBox: [100, 100], size: [80, 100], binding: 'value',
  ports: { left: [4, 75], right: [96, 75] },
  render: p => (
    <Group p={p}>
      <path d="M8 55 L50 75 L8 95 Z M92 55 L50 75 L92 95 Z" fill="var(--isa-off)" {...stroke(p.state)} />
      <line x1="50" y1="75" x2="50" y2="36" {...stroke(p.state)} />
      <path d="M24 36 A26 26 0 0 1 76 36 Z" fill="var(--isa-off)" {...stroke(p.state)} />
    </Group>
  ),
}

const levelFields: ConfigFieldDef[] = [{ key: 'showPercent', label: 'Show %', type: 'boolean', default: true }]

const isaTank: SymbolDef = {
  key: 'isa_tank', name: 'Tank (ISA-101)', category: 'ISA-101', viewBox: [100, 160], size: [90, 144], binding: 'level',
  ports: { top: [50, 4], bottom: [50, 156], left: [4, 130], right: [96, 130] },
  fields: levelFields,
  render: p => {
    const top = 16, bottom = 150
    const h = p.level == null ? 0 : (bottom - top) * p.level
    return (
      <Group p={p}>
        <path d={`M6 ${top + 10} Q6 ${top} 16 ${top} L84 ${top} Q94 ${top} 94 ${top + 10} L94 ${bottom} L6 ${bottom} Z`} fill="var(--isa-off)" {...stroke(p.state)} />
        {p.level != null && (
          <rect x="7" y={bottom - h} width="86" height={h} fill="var(--isa-level)" style={{ transition: 'y .6s ease, height .6s ease' }} />
        )}
        <path d={`M6 ${top + 10} Q6 ${top} 16 ${top} L84 ${top} Q94 ${top} 94 ${top + 10} L94 ${bottom} L6 ${bottom} Z`} fill="none" {...stroke(p.state)} />
        {/* Level scale on the right wall: 0, 25, 50, 75, 100 % */}
        {[0, 1, 2, 3, 4].map(i => {
          const y = bottom - ((bottom - top) * i) / 4
          return <line key={i} x1="86" x2="94" y1={y} y2={y} stroke="var(--isa-line)" strokeWidth={1} {...thin} />
        })}
        {p.level != null && p.config.showPercent !== false && (
          <text x="50" y={(top + bottom) / 2} textAnchor="middle" dominantBaseline="central" fontSize="18" fontWeight={600}
            fill="var(--isa-text)" style={{ fontFamily: 'var(--font-mono)' }}>{Math.round(p.level * 100)}%</text>
        )}
      </Group>
    )
  },
}

const isaHeatExchanger: SymbolDef = {
  key: 'isa_heat_exchanger', name: 'Heat Exchanger (ISA-101)', category: 'ISA-101', viewBox: [100, 100], size: [80, 80], binding: 'none',
  ports: { left: [4, 50], right: [96, 50], top: [50, 4], bottom: [50, 96] },
  render: p => (
    <Group p={p}>
      <circle cx="50" cy="50" r="40" fill="var(--isa-off)" {...stroke(p.state)} />
      <path d="M4 50 L22 50 L34 30 L50 70 L66 30 L78 50 L96 50" fill="none" {...stroke(p.state)} />
      <line x1="50" y1="4" x2="50" y2="10" {...stroke(p.state)} />
      <line x1="50" y1="90" x2="50" y2="96" {...stroke(p.state)} />
    </Group>
  ),
}

const lineFields: ConfigFieldDef[] = [
  { key: 'runSensorId', label: 'Flow Tag (animates)', type: 'tag', default: null },
]

function Line({ p, d }: { p: SymbolRenderProps; d: string }) {
  return (
    <g>
      <path d={d} fill="none" stroke="var(--isa-line)" strokeWidth={3} {...thin} />
      {p.runState === 'running' && (
        <path d={d} fill="none" stroke="var(--isa-on)" strokeWidth={3} strokeDasharray="6 18" className="hmi-flow" {...thin} />
      )}
    </g>
  )
}

const isaLine: SymbolDef = {
  key: 'isa_line', name: 'Process Line (ISA-101)', category: 'ISA-101', viewBox: [200, 10], size: [200, 10], binding: 'none', stretch: true,
  fields: lineFields,
  render: p => <Line p={p} d="M0 5 L200 5" />,
}

const isaLineVertical: SymbolDef = {
  key: 'isa_line_vertical', name: 'Process Line, vertical (ISA-101)', category: 'ISA-101', viewBox: [10, 200], size: [10, 200], binding: 'none', stretch: true,
  fields: lineFields,
  render: p => <Line p={p} d="M5 0 L5 200" />,
}

export const ISA101_SYMBOLS: SymbolDef[] = [
  isaPump, isaMotor, isaValve, isaControlValve, isaTank, isaHeatExchanger, isaLine, isaLineVertical,
]
