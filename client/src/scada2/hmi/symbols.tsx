import React from 'react'
import type { ConfigFieldDef } from '../widgets/types'

// ISA-101 style equipment symbols. Bodies are neutral grey; colour is
// reserved for state (running / fault / stale) so abnormal conditions stand out.

export type EquipState = 'idle' | 'running' | 'stopped' | 'fault' | 'stale'

/** How the primary tag drives the symbol. */
export type SymbolBinding = 'state' | 'level' | 'value' | 'none'

export type SymbolCategory = 'Rotating' | 'Valves' | 'Vessels' | 'Process' | 'Electrical' | 'Instruments' | 'Piping'

export type PortSide = 'left' | 'right' | 'top' | 'bottom'

export interface SymbolRenderProps {
  uid: string
  state: EquipState
  /** 0..1 fill for level-bound symbols, null when unknown. */
  level: number | null
  /** Formatted live value (meter display, valve position). */
  value: string
  unit: string
  /** State of the secondary run tag (agitator, pipe flow). */
  runState: EquipState
  config: Record<string, any>
}

export interface SymbolDef {
  key: string
  name: string
  category: SymbolCategory
  viewBox: [number, number]
  size: [number, number]
  binding: SymbolBinding
  /** Stretch to the widget box instead of keeping aspect ratio (pipes, busbars). */
  stretch?: boolean
  /** Nozzle points in viewBox units, where connections attach. Missing sides use the viewBox edge midpoint. */
  ports?: Partial<Record<PortSide, [number, number]>>
  fields?: ConfigFieldDef[]
  render: (p: SymbolRenderProps) => React.ReactNode
}

// ── Shared paint ──

function Defs({ uid }: { uid: string }) {
  const stop = (offset: string, v: string, opacity = 1) => (
    <stop offset={offset} style={{ stopColor: `var(${v})`, stopOpacity: opacity }} />
  )
  return (
    <defs>
      {/* Cylinder lit from the top-left, varying across y */}
      <linearGradient id={`${uid}-cv`} x1="0" y1="0" x2="0" y2="1">
        {stop('0', '--hmi-body-mid')}
        {stop('.28', '--hmi-body-hi')}
        {stop('.62', '--hmi-body-mid')}
        {stop('1', '--hmi-body-lo')}
      </linearGradient>
      {/* Cylinder varying across x */}
      <linearGradient id={`${uid}-ch`} x1="0" y1="0" x2="1" y2="0">
        {stop('0', '--hmi-body-mid')}
        {stop('.28', '--hmi-body-hi')}
        {stop('.62', '--hmi-body-mid')}
        {stop('1', '--hmi-body-lo')}
      </linearGradient>
      <radialGradient id={`${uid}-sp`} cx=".38" cy=".34" r=".75">
        {stop('0', '--hmi-body-hi')}
        {stop('.55', '--hmi-body-mid')}
        {stop('1', '--hmi-body-lo')}
      </radialGradient>
      <linearGradient id={`${uid}-pv`} x1="0" y1="0" x2="0" y2="1">
        {stop('0', '--hmi-pipe-lo')}
        {stop('.35', '--hmi-pipe-hi')}
        {stop('1', '--hmi-pipe-lo')}
      </linearGradient>
      <linearGradient id={`${uid}-ph`} x1="0" y1="0" x2="1" y2="0">
        {stop('0', '--hmi-pipe-lo')}
        {stop('.35', '--hmi-pipe-hi')}
        {stop('1', '--hmi-pipe-lo')}
      </linearGradient>
      <linearGradient id={`${uid}-lq`} x1="0" y1="0" x2="1" y2="0">
        {stop('0', '--hmi-liquid-lo')}
        {stop('.3', '--hmi-liquid-hi')}
        {stop('1', '--hmi-liquid-lo')}
      </linearGradient>
      <linearGradient id={`${uid}-bk`} x1="0" y1="0" x2="1" y2="0">
        {stop('0', '--hmi-bulk-lo')}
        {stop('.3', '--hmi-bulk-hi')}
        {stop('1', '--hmi-bulk-lo')}
      </linearGradient>
    </defs>
  )
}

const url = (uid: string, g: string) => `url(#${uid}-${g})`

interface OutlineProps {
  stroke: string
  strokeWidth: number
  strokeDasharray?: string
  vectorEffect: 'non-scaling-stroke'
  strokeLinejoin: 'round'
}

function outline(state: EquipState): OutlineProps {
  return {
    stroke: state === 'fault' ? 'var(--hmi-fault)' : state === 'stale' ? 'var(--hmi-stale)' : 'var(--hmi-stroke)',
    strokeWidth: state === 'fault' ? 2 : 1.25,
    strokeDasharray: state === 'stale' ? '4 3' : undefined,
    vectorEffect: 'non-scaling-stroke',
    strokeLinejoin: 'round',
  }
}

/** Colour of the moving / indicating part of a symbol. */
function accent(state: EquipState): string {
  switch (state) {
    case 'running': return 'var(--hmi-run)'
    case 'fault': return 'var(--hmi-fault)'
    case 'stale': return 'var(--hmi-stale)'
    default: return 'var(--hmi-detail)'
  }
}

const thin = { vectorEffect: 'non-scaling-stroke' as const }

function Frame({ p, children }: { p: SymbolRenderProps; children: React.ReactNode }) {
  return (
    <g className={p.state === 'fault' ? 'hmi-blink' : undefined}>
      <Defs uid={p.uid} />
      {children}
    </g>
  )
}

/** Domed vessel outline between `top` and `bottom` inside x 12..88. */
const vesselPath = (top: number, bottom: number) =>
  `M12 ${top + 18} Q12 ${top} 50 ${top} Q88 ${top} 88 ${top + 18} L88 ${bottom - 12} Q88 ${bottom} 50 ${bottom} Q12 ${bottom} 12 ${bottom - 12} Z`

function LevelFill({ p, clip, top, bottom, x = 0, w = 200, bulk }: {
  p: SymbolRenderProps; clip: string; top: number; bottom: number; x?: number; w?: number; bulk?: boolean
}) {
  if (p.level == null) return null
  const h = (bottom - top) * p.level
  const y = bottom - h
  return (
    <g clipPath={`url(#${p.uid}-${clip})`}>
      <rect x={x} y={y} width={w} height={h} fill={url(p.uid, bulk ? 'bk' : 'lq')} style={{ transition: 'y .6s ease, height .6s ease' }} />
      <rect x={x} y={y} width={w} height={1.5} fill="#fff" opacity={0.35} style={{ transition: 'y .6s ease' }} />
    </g>
  )
}

function LevelText({ p, x, y }: { p: SymbolRenderProps; x: number; y: number }) {
  if (p.level == null || p.config.showPercent === false) return null
  return (
    <text x={x} y={y} textAnchor="middle" dominantBaseline="central" fontSize={13} fontWeight={700}
      fill="#fff" stroke="rgba(0,0,0,.55)" strokeWidth={3} paintOrder="stroke"
      style={{ fontFamily: 'var(--font-mono)' }}>
      {Math.round(p.level * 100)}%
    </text>
  )
}

function Ticks({ x, top, bottom }: { x: number; top: number; bottom: number }) {
  const out = []
  for (let i = 0; i <= 10; i++) {
    const y = bottom - ((bottom - top) * i) / 10
    out.push(<line key={i} x1={x} x2={x + (i % 5 === 0 ? 7 : 4)} y1={y} y2={y} stroke="var(--hmi-detail)" strokeWidth={1} opacity={0.7} {...thin} />)
  }
  return <>{out}</>
}

function Lamp({ p, cx, cy, r = 3.2 }: { p: SymbolRenderProps; cx: number; cy: number; r?: number }) {
  return <circle cx={cx} cy={cy} r={r} fill={accent(p.state)} stroke="rgba(0,0,0,.35)" strokeWidth={0.75} {...thin} />
}

// ── Symbols ──

const pump: SymbolDef = {
  key: 'pump', name: 'Centrifugal Pump', category: 'Rotating', viewBox: [100, 100], size: [90, 90], binding: 'state',
  ports: { left: [2, 52.5], right: [97, 29.5] },
  render: p => (
    <Frame p={p}>
      <path d="M30 76 L70 76 L82 95 L18 95 Z" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <rect x="2" y="45" width="26" height="15" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="50" y="22" width="47" height="15" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <circle cx="50" cy="52" r="30" fill={url(p.uid, 'sp')} {...outline(p.state)} />
      <circle cx="50" cy="52" r="21" fill="var(--hmi-body-lo)" opacity={0.45} />
      <g className={p.state === 'running' ? 'hmi-spin' : undefined}>
        {[0, 120, 240].map(a => (
          <path key={a} d="M50 52 C 57 47, 62 40, 63 33" transform={`rotate(${a} 50 52)`}
            fill="none" stroke={accent(p.state)} strokeWidth={5} strokeLinecap="round" />
        ))}
      </g>
      <circle cx="50" cy="52" r="5" fill="var(--hmi-body-hi)" stroke="var(--hmi-detail)" strokeWidth={1} {...thin} />
    </Frame>
  ),
}

const fan: SymbolDef = {
  key: 'fan', name: 'Fan / Blower', category: 'Rotating', viewBox: [100, 100], size: [90, 90], binding: 'state',
  ports: { left: [6, 50], right: [94, 50], top: [50, 6], bottom: [50, 94] },
  render: p => (
    <Frame p={p}>
      <circle cx="50" cy="50" r="44" fill={url(p.uid, 'sp')} {...outline(p.state)} />
      <circle cx="50" cy="50" r="36" fill="var(--hmi-detail)" opacity={0.55} />
      <g className={p.state === 'running' ? 'hmi-spin' : undefined}>
        {[0, 90, 180, 270].map(a => (
          <path key={a} d="M50 50 C 56 40, 70 30, 80 36 C 80 46, 66 52, 50 50 Z" transform={`rotate(${a} 50 50)`}
            fill={p.state === 'idle' || p.state === 'stopped' ? 'var(--hmi-body-mid)' : accent(p.state)} opacity={0.9} />
        ))}
      </g>
      <circle cx="50" cy="50" r="6" fill="var(--hmi-body-hi)" stroke="var(--hmi-detail)" strokeWidth={1} {...thin} />
    </Frame>
  ),
}

const motor: SymbolDef = {
  key: 'motor', name: 'Electric Motor', category: 'Rotating', viewBox: [120, 80], size: [120, 80], binding: 'state',
  // Shaft on the right; supply cable into the terminal box on top.
  ports: { right: [117, 40], top: [59.5, 2] },
  render: p => (
    <Frame p={p}>
      <rect x="28" y="64" width="13" height="12" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <rect x="78" y="64" width="13" height="12" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <rect x="98" y="35" width="19" height="10" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="7" y="20" width="16" height="40" rx="4" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <rect x="20" y="12" width="79" height="56" rx="7" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      {[22, 30, 38, 46, 54].map(y => (
        <line key={y} x1="28" x2="92" y1={y + 2} y2={y + 2} stroke="var(--hmi-detail)" strokeWidth={1} opacity={0.35} {...thin} />
      ))}
      <rect x="46" y="2" width="27" height="12" rx="2" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <Lamp p={p} cx={59.5} cy={8} />
      <circle cx="59.5" cy="40" r="11" fill="var(--hmi-detail)" stroke={accent(p.state)} strokeWidth={2} {...thin} />
      <text x="59.5" y="40.5" textAnchor="middle" dominantBaseline="central" fontSize={12} fontWeight={700}
        fill={p.state === 'idle' || p.state === 'stopped' ? 'var(--hmi-body-hi)' : accent(p.state)}>M</text>
    </Frame>
  ),
}

const compressor: SymbolDef = {
  key: 'compressor', name: 'Compressor', category: 'Rotating', viewBox: [100, 100], size: [90, 90], binding: 'state',
  ports: { left: [2, 50.5], right: [98, 50.5] },
  render: p => (
    <Frame p={p}>
      <path d="M30 78 L70 78 L82 95 L18 95 Z" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <rect x="2" y="43" width="18" height="15" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="80" y="43" width="18" height="15" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <circle cx="50" cy="50" r="32" fill={url(p.uid, 'sp')} {...outline(p.state)} />
      <path d="M26 30 L74 41 M26 70 L74 59" stroke={accent(p.state)} strokeWidth={3.5} strokeLinecap="round" fill="none" />
    </Frame>
  ),
}

const conveyor: SymbolDef = {
  key: 'conveyor', name: 'Belt Conveyor', category: 'Rotating', viewBox: [200, 64], size: [220, 64], binding: 'state',
  ports: { left: [10, 28], right: [190, 28] },
  render: p => (
    <Frame p={p}>
      <path d="M40 42 L34 62 M160 42 L166 62" stroke="var(--hmi-body-lo)" strokeWidth={4} />
      <rect x="10" y="12" width="180" height="32" rx="16" fill="var(--hmi-detail)" {...outline(p.state)} />
      <line x1="26" x2="174" y1="13.5" y2="13.5" stroke={accent(p.state)} strokeWidth={3}
        strokeDasharray="10 14" className={p.state === 'running' ? 'hmi-flow' : undefined} />
      {[60, 85, 110, 135].map(x => (
        <circle key={x} cx={x} cy="28" r="5" fill={url(p.uid, 'sp')} stroke="var(--hmi-detail)" strokeWidth={1} {...thin} />
      ))}
      {[26, 174].map(x => (
        <g key={x}>
          <circle cx={x} cy="28" r="13" fill={url(p.uid, 'sp')} {...outline(p.state)} />
          <circle cx={x} cy="28" r="3" fill="var(--hmi-detail)" />
        </g>
      ))}
    </Frame>
  ),
}

const valve: SymbolDef = {
  key: 'valve', name: 'Gate Valve', category: 'Valves', viewBox: [100, 80], size: [90, 72], binding: 'state',
  ports: { left: [0, 52], right: [100, 52] },
  render: p => {
    const open = p.state === 'running'
    const fill = open ? 'var(--hmi-run)' : p.state === 'fault' ? 'var(--hmi-fault)' : url(p.uid, 'sp')
    return (
      <Frame p={p}>
        <rect x="0" y="45" width="20" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
        <rect x="80" y="45" width="20" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
        <line x1="50" y1="52" x2="50" y2="17" stroke="var(--hmi-stroke)" strokeWidth={3} />
        <rect x="33" y="10" width="34" height="7" rx="3.5" fill={url(p.uid, 'cv')} {...outline(p.state)} />
        <path d="M18 34 L50 52 L18 70 Z M82 34 L50 52 L82 70 Z" fill={fill} {...outline(p.state)} />
      </Frame>
    )
  },
}

const controlValve: SymbolDef = {
  key: 'control_valve', name: 'Control Valve', category: 'Valves', viewBox: [100, 100], size: [90, 90], binding: 'value',
  ports: { left: [0, 72], right: [100, 72] },
  render: p => (
    <Frame p={p}>
      <rect x="0" y="65" width="20" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="80" y="65" width="20" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <line x1="50" y1="72" x2="50" y2="32" stroke="var(--hmi-stroke)" strokeWidth={3} />
      <path d="M22 32 Q50 2 78 32 Z" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <line x1="20" x2="80" y1="32" y2="32" stroke="var(--hmi-stroke)" strokeWidth={2} {...thin} />
      <path d="M18 54 L50 72 L18 90 Z M82 54 L50 72 L82 90 Z" fill={url(p.uid, 'sp')} {...outline(p.state)} />
      {p.value && (
        <text x="50" y="22" textAnchor="middle" dominantBaseline="central" fontSize={10} fontWeight={700}
          fill="var(--hmi-detail)" style={{ fontFamily: 'var(--font-mono)' }}>{p.value}</text>
      )}
    </Frame>
  ),
}

const checkValve: SymbolDef = {
  key: 'check_valve', name: 'Check Valve', category: 'Valves', viewBox: [100, 60], size: [80, 48], binding: 'none',
  ports: { left: [0, 30], right: [100, 30] },
  render: p => (
    <Frame p={p}>
      <rect x="0" y="23" width="22" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="78" y="23" width="22" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <path d="M20 10 L50 30 L20 50 Z" fill={url(p.uid, 'sp')} {...outline(p.state)} />
      <path d="M80 10 L50 30 L80 50 Z" fill="var(--hmi-stroke)" {...outline(p.state)} />
    </Frame>
  ),
}

const tank: SymbolDef = {
  key: 'tank', name: 'Vertical Tank', category: 'Vessels', viewBox: [100, 170], size: [110, 180], binding: 'level',
  ports: { left: [12, 110], right: [88, 130], top: [50, 6], bottom: [50, 152] },
  fields: [{ key: 'showPercent', label: 'Show %', type: 'boolean', default: true }],
  render: p => {
    const d = vesselPath(6, 152)
    return (
      <Frame p={p}>
        <clipPath id={`${p.uid}-clip`}><path d={d} /></clipPath>
        <rect x="18" y="146" width="8" height="21" fill={url(p.uid, 'ch')} {...outline(p.state)} />
        <rect x="74" y="146" width="8" height="21" fill={url(p.uid, 'ch')} {...outline(p.state)} />
        <path d={d} fill={url(p.uid, 'ch')} />
        <LevelFill p={p} clip="clip" top={10} bottom={152} />
        <rect x="21" y="22" width="5" height="116" fill="#fff" opacity={0.1} />
        <Ticks x={80} top={24} bottom={140} />
        <path d={d} fill="none" {...outline(p.state)} />
        <LevelText p={p} x={50} y={80} />
      </Frame>
    )
  },
}

const tankHorizontal: SymbolDef = {
  key: 'tank_horizontal', name: 'Horizontal Tank', category: 'Vessels', viewBox: [180, 100], size: [200, 110], binding: 'level',
  ports: { left: [-1, 60], right: [181, 60], top: [90, 8], bottom: [90, 84] },
  fields: [{ key: 'showPercent', label: 'Show %', type: 'boolean', default: true }],
  render: p => {
    const d = 'M34 8 L146 8 A38 38 0 0 1 146 84 L34 84 A38 38 0 0 1 34 8 Z'
    return (
      <Frame p={p}>
        <clipPath id={`${p.uid}-clip`}><path d={d} /></clipPath>
        <path d="M44 78 L64 78 L68 97 L40 97 Z M116 78 L136 78 L140 97 L112 97 Z" fill={url(p.uid, 'cv')} {...outline(p.state)} />
        <path d={d} fill={url(p.uid, 'cv')} />
        <LevelFill p={p} clip="clip" top={8} bottom={84} />
        <rect x="34" y="16" width="112" height="4" fill="#fff" opacity={0.12} />
        <path d={d} fill="none" {...outline(p.state)} />
        <LevelText p={p} x={90} y={46} />
      </Frame>
    )
  },
}

const silo: SymbolDef = {
  key: 'silo', name: 'Silo / Hopper', category: 'Vessels', viewBox: [100, 170], size: [100, 170], binding: 'level',
  ports: { top: [50, 6], bottom: [50, 166] },
  fields: [{ key: 'showPercent', label: 'Show %', type: 'boolean', default: true }],
  render: p => {
    const d = 'M12 30 L50 6 L88 30 L88 108 L58 148 L42 148 L12 108 Z'
    return (
      <Frame p={p}>
        <clipPath id={`${p.uid}-clip`}><path d={d} /></clipPath>
        <rect x="43" y="146" width="14" height="20" fill={url(p.uid, 'ph')} {...outline(p.state)} />
        <path d={d} fill={url(p.uid, 'ch')} />
        <LevelFill p={p} clip="clip" top={6} bottom={148} bulk />
        <line x1="12" x2="88" y1="30" y2="30" stroke="var(--hmi-detail)" strokeWidth={1} opacity={0.5} {...thin} />
        <line x1="12" x2="88" y1="108" y2="108" stroke="var(--hmi-detail)" strokeWidth={1} opacity={0.5} {...thin} />
        <path d={d} fill="none" {...outline(p.state)} />
        <LevelText p={p} x={50} y={70} />
      </Frame>
    )
  },
}

const mixer: SymbolDef = {
  key: 'mixer', name: 'Mixing Tank', category: 'Vessels', viewBox: [100, 170], size: [110, 180], binding: 'level',
  ports: { left: [12, 110], right: [88, 140], bottom: [50, 160] },
  fields: [
    { key: 'showPercent', label: 'Show %', type: 'boolean', default: true },
    { key: 'runSensorId', label: 'Agitator Run Tag', type: 'tag', default: null },
  ],
  render: p => {
    const d = vesselPath(24, 160)
    const spinning = p.runState === 'running'
    return (
      <Frame p={p}>
        <clipPath id={`${p.uid}-clip`}><path d={d} /></clipPath>
        <path d={d} fill={url(p.uid, 'ch')} />
        <LevelFill p={p} clip="clip" top={28} bottom={160} />
        <line x1="50" y1="18" x2="50" y2="134" stroke="var(--hmi-stroke)" strokeWidth={2.5} />
        <g className={spinning ? 'hmi-agitate' : undefined}>
          <path d="M50 134 L30 128 L30 140 Z M50 134 L70 128 L70 140 Z" fill={spinning ? 'var(--hmi-run)' : 'var(--hmi-body-mid)'}
            stroke="var(--hmi-detail)" strokeWidth={1} {...thin} />
        </g>
        <path d={d} fill="none" {...outline(p.state)} />
        <rect x="37" y="0" width="26" height="20" rx="3" fill={url(p.uid, 'ch')} {...outline(p.state)} />
        <circle cx="50" cy="10" r="3.2" fill={spinning ? 'var(--hmi-run)' : 'var(--hmi-detail)'} />
        <LevelText p={p} x={50} y={90} />
      </Frame>
    )
  },
}

const heatExchanger: SymbolDef = {
  key: 'heat_exchanger', name: 'Heat Exchanger', category: 'Process', viewBox: [140, 80], size: [150, 86], binding: 'none',
  ports: { left: [0, 40], right: [140, 40], top: [42.5, 2], bottom: [97.5, 78] },
  render: p => (
    <Frame p={p}>
      <rect x="36" y="2" width="13" height="16" fill={url(p.uid, 'ph')} {...outline(p.state)} />
      <rect x="91" y="62" width="13" height="16" fill={url(p.uid, 'ph')} {...outline(p.state)} />
      <rect x="0" y="33" width="16" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="124" y="33" width="16" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <path d="M38 16 L102 16 A24 24 0 0 1 102 64 L38 64 A24 24 0 0 1 38 16 Z" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <path d="M16 40 L32 40 L42 26 L56 54 L70 26 L84 54 L98 26 L108 40 L124 40" fill="none"
        stroke="var(--hmi-detail)" strokeWidth={2.5} strokeLinejoin="round" />
    </Frame>
  ),
}

const filter: SymbolDef = {
  key: 'filter', name: 'Filter', category: 'Process', viewBox: [80, 120], size: [70, 110], binding: 'none',
  ports: { left: [0, 28.5], right: [80, 91.5] },
  render: p => (
    <Frame p={p}>
      <rect x="0" y="22" width="16" height="13" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="64" y="85" width="16" height="13" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="14" y="8" width="52" height="104" rx="20" fill={url(p.uid, 'ch')} {...outline(p.state)} />
      <line x1="16" y1="100" x2="64" y2="20" stroke="var(--hmi-detail)" strokeWidth={2} strokeDasharray="6 4" {...thin} />
    </Frame>
  ),
}

const clarifier: SymbolDef = {
  key: 'clarifier', name: 'Clarifier', category: 'Process', viewBox: [220, 100], size: [240, 110], binding: 'value',
  ports: { left: [0, 30], right: [220, 22], bottom: [110, 98] },
  fields: [{ key: 'runSensorId', label: 'Rake Drive Run Tag', type: 'tag', default: null }],
  render: p => {
    const basin = 'M14 14 L206 14 L206 58 L124 86 L96 86 L14 58 Z'
    const turning = p.runState === 'running'
    return (
      <Frame p={p}>
        <clipPath id={`${p.uid}-clip`}><path d={basin} /></clipPath>
        <rect x="0" y="23" width="18" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
        <rect x="203" y="15" width="17" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
        <rect x="102" y="84" width="16" height="14" fill={url(p.uid, 'ph')} {...outline(p.state)} />
        <path d={basin} fill={url(p.uid, 'lq')} opacity={0.85} />
        <g clipPath={`url(#${p.uid}-clip)`}>
          <path d="M14 66 L96 92 L124 92 L206 66 L206 100 L14 100 Z" fill={url(p.uid, 'bk')} opacity={0.9} />
          <rect x="14" y="14" width="192" height="3" fill="#fff" opacity={0.3} />
        </g>
        <rect x="94" y="14" width="32" height="38" fill="var(--hmi-body-mid)" opacity={0.6} stroke="var(--hmi-detail)" strokeWidth={1} {...thin} />
        <line x1="110" y1="4" x2="110" y2="80" stroke="var(--hmi-stroke)" strokeWidth={2.5} />
        <g className={turning ? 'hmi-agitate' : undefined}>
          <path d="M110 80 L40 60 M110 80 L180 60" stroke={turning ? 'var(--hmi-run)' : 'var(--hmi-body-hi)'} strokeWidth={3} strokeLinecap="round" />
        </g>
        <path d={basin} fill="none" {...outline(p.state)} />
        <rect x="8" y="10" width="204" height="5" fill={url(p.uid, 'cv')} {...outline(p.state)} />
        <rect x="96" y="0" width="28" height="11" rx="2" fill={url(p.uid, 'cv')} {...outline(p.state)} />
        <circle cx="110" cy="5.5" r="3" fill={turning ? 'var(--hmi-run)' : 'var(--hmi-detail)'} />
      </Frame>
    )
  },
}

const dosingPump: SymbolDef = {
  key: 'dosing_pump', name: 'Dosing Pump', category: 'Rotating', viewBox: [100, 90], size: [84, 76], binding: 'state',
  ports: { left: [0, 58], right: [100, 58] },
  render: p => (
    <Frame p={p}>
      <path d="M22 78 L78 78 L86 89 L14 89 Z" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <rect x="0" y="51" width="24" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="76" y="51" width="24" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="20" y="36" width="60" height="44" rx="6" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <path d="M34 58 L50 46 L66 58 L50 70 Z" fill={accent(p.state)} stroke="var(--hmi-detail)" strokeWidth={1} {...thin} />
      <rect x="38" y="4" width="24" height="34" rx="4" fill={url(p.uid, 'ch')} {...outline(p.state)} />
      <line x1="44" x2="56" y1="14" y2="14" stroke="var(--hmi-detail)" strokeWidth={1.5} {...thin} />
      <line x1="44" x2="56" y1="22" y2="22" stroke="var(--hmi-detail)" strokeWidth={1.5} {...thin} />
      <Lamp p={p} cx={50} cy={30} />
    </Frame>
  ),
}

const gravityFilter: SymbolDef = {
  key: 'gravity_filter', name: 'Gravity Filter', category: 'Process', viewBox: [140, 120], size: [140, 120], binding: 'value',
  ports: { left: [0, 26], right: [140, 106], bottom: [70, 120] },
  fields: [{ key: 'runSensorId', label: 'Backwash Tag', type: 'tag', default: null }],
  render: p => {
    const washing = p.runState === 'running'
    return (
      <Frame p={p}>
        <clipPath id={`${p.uid}-clip`}><rect x="14" y="10" width="112" height="90" /></clipPath>
        <rect x="0" y="19" width="16" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
        <rect x="124" y="99" width="16" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
        <rect x="63" y="104" width="14" height="16" fill={url(p.uid, 'ph')} {...outline(p.state)} />
        <rect x="14" y="10" width="112" height="96" fill={url(p.uid, 'ch')} />
        <g clipPath={`url(#${p.uid}-clip)`}>
          <rect x="14" y={washing ? 14 : 22} width="112" height={washing ? 40 : 32} fill={url(p.uid, 'lq')} opacity={0.85}
            style={{ transition: 'y .6s ease, height .6s ease' }} />
          <rect x="14" y="54" width="112" height="22" fill={url(p.uid, 'bk')} />
          <rect x="14" y="76" width="112" height="14" fill="var(--hmi-body-lo)" />
          {[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13].map(i => (
            <circle key={i} cx={20 + i * 8} cy={60 + (i % 3) * 5} r={1.4} fill="var(--hmi-bulk-lo)" />
          ))}
          {washing && [24, 46, 70, 94, 116].map((x, i) => (
            <path key={x} d={`M${x} 50 L${x} 26 M${x - 4} 31 L${x} 25 L${x + 4} 31`} stroke="#fff" strokeWidth={1.5} fill="none"
              opacity={0.7} className="hmi-blink" style={{ animationDelay: `${i * 0.15}s` }} {...thin} />
          ))}
        </g>
        <line x1="14" x2="126" y1="90" y2="90" stroke="var(--hmi-detail)" strokeWidth={1} strokeDasharray="3 3" {...thin} />
        <rect x="14" y="10" width="112" height="96" fill="none" {...outline(p.state)} />
        <rect x="104" y="0" width="22" height="10" rx="2" fill={washing ? 'var(--hmi-liquid-hi)' : url(p.uid, 'cv')} {...outline(p.state)} />
      </Frame>
    )
  },
}

const flowMeter: SymbolDef = {
  key: 'flow_meter', name: 'Mag Flowmeter', category: 'Instruments', viewBox: [100, 70], size: [96, 66], binding: 'value',
  ports: { left: [0, 46], right: [100, 46] },
  render: p => (
    <Frame p={p}>
      <rect x="0" y="39" width="100" height="14" fill={url(p.uid, 'pv')} {...outline(p.state)} />
      <rect x="20" y="30" width="7" height="32" rx="1.5" fill={url(p.uid, 'ch')} {...outline(p.state)} />
      <rect x="73" y="30" width="7" height="32" rx="1.5" fill={url(p.uid, 'ch')} {...outline(p.state)} />
      <rect x="31" y="32" width="38" height="28" rx="12" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <rect x="40" y="2" width="20" height="30" rx="3" fill={url(p.uid, 'ch')} {...outline(p.state)} />
      <rect x="43" y="7" width="14" height="10" rx="1.5" fill="var(--hmi-lcd)" />
      <text x="50" y="47" textAnchor="middle" dominantBaseline="central" fontSize={11} fontWeight={700} fill="var(--hmi-detail)">FT</text>
    </Frame>
  ),
}

const breaker: SymbolDef = {
  key: 'breaker', name: 'Circuit Breaker', category: 'Electrical', viewBox: [60, 100], size: [54, 90], binding: 'state',
  ports: { top: [30, 0], bottom: [30, 100] },
  render: p => {
    const closed = p.state === 'running'
    const line = closed ? 'var(--hmi-run)' : 'var(--hmi-stroke)'
    return (
      <Frame p={p}>
        <line x1="30" y1="0" x2="30" y2="28" stroke={line} strokeWidth={3} />
        <line x1="30" y1="72" x2="30" y2="100" stroke={line} strokeWidth={3} />
        <rect x="12" y="28" width="36" height="44" rx="3" fill={closed ? 'var(--hmi-run)' : url(p.uid, 'cv')} {...outline(p.state)} />
        {closed
          ? <line x1="30" y1="34" x2="30" y2="66" stroke="var(--hmi-detail)" strokeWidth={3} />
          : <line x1="30" y1="66" x2="42" y2="38" stroke="var(--hmi-detail)" strokeWidth={3} strokeLinecap="round" />}
      </Frame>
    )
  },
}

const transformer: SymbolDef = {
  key: 'transformer', name: 'Transformer', category: 'Electrical', viewBox: [80, 120], size: [72, 108], binding: 'state',
  ports: { top: [40, 0], bottom: [40, 120] },
  render: p => {
    const line = p.state === 'running' ? 'var(--hmi-run)' : 'var(--hmi-stroke)'
    return (
      <Frame p={p}>
        <line x1="40" y1="0" x2="40" y2="20" stroke={line} strokeWidth={3} />
        <line x1="40" y1="100" x2="40" y2="120" stroke={line} strokeWidth={3} />
        <circle cx="40" cy="44" r="24" fill={url(p.uid, 'sp')} fillOpacity={0.9} {...outline(p.state)} />
        <circle cx="40" cy="76" r="24" fill={url(p.uid, 'sp')} fillOpacity={0.75} {...outline(p.state)} />
        <circle cx="40" cy="44" r="24" fill="none" {...outline(p.state)} />
      </Frame>
    )
  },
}

const generator: SymbolDef = {
  key: 'generator', name: 'Generator', category: 'Electrical', viewBox: [100, 100], size: [90, 90], binding: 'state',
  ports: { left: [12, 50], right: [88, 50] },
  render: p => (
    <Frame p={p}>
      <circle cx="50" cy="50" r="38" fill={url(p.uid, 'sp')} {...outline(p.state)} />
      <text x="50" y="42" textAnchor="middle" dominantBaseline="central" fontSize={22} fontWeight={700} fill="var(--hmi-detail)">G</text>
      <path d="M30 64 Q 37.5 52, 45 64 T 60 64 T 75 64" fill="none" stroke={accent(p.state)} strokeWidth={3} strokeLinecap="round" />
    </Frame>
  ),
}

const solar: SymbolDef = {
  key: 'solar_panel', name: 'Solar Array', category: 'Electrical', viewBox: [120, 100], size: [120, 100], binding: 'state',
  ports: { bottom: [60, 97] },
  render: p => (
    <Frame p={p}>
      <line x1="60" y1="66" x2="60" y2="92" stroke="var(--hmi-body-lo)" strokeWidth={5} />
      <rect x="44" y="90" width="32" height="7" rx="2" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <path d="M14 16 L106 8 L114 62 L6 70 Z" fill="#1d3a6e" {...outline(p.state)} />
      <path d="M44.7 13.3 L42 67.3 M75.3 10.7 L78 64.7 M10 43 L110 35" stroke="#7aa6e0" strokeWidth={1} opacity={0.6} {...thin} />
      <path d="M14 16 L106 8 L108 22 L12 30 Z" fill="#fff" opacity={0.08} />
      <Lamp p={p} cx={104} cy={70} />
    </Frame>
  ),
}

const battery: SymbolDef = {
  key: 'battery', name: 'Battery (SoC)', category: 'Electrical', viewBox: [120, 70], size: [120, 70], binding: 'level',
  ports: { left: [5, 35], right: [115, 35] },
  render: p => {
    const lvl = p.level ?? 0
    const color = p.level == null ? 'var(--hmi-body-mid)' : lvl < 0.2 ? 'var(--hmi-fault)' : lvl < 0.4 ? 'var(--hmi-warn)' : 'var(--hmi-run)'
    return (
      <Frame p={p}>
        <rect x="106" y="24" width="9" height="22" rx="2" fill={url(p.uid, 'cv')} {...outline(p.state)} />
        <rect x="5" y="9" width="102" height="52" rx="7" fill={url(p.uid, 'cv')} {...outline(p.state)} />
        <rect x="12" y="16" width="88" height="38" rx="3" fill="var(--hmi-detail)" />
        <rect x="12" y="16" width={88 * lvl} height="38" rx="3" fill={color} style={{ transition: 'width .6s ease' }} />
        <LevelText p={p} x={56} y={35} />
      </Frame>
    )
  },
}

const inverter: SymbolDef = {
  key: 'inverter', name: 'Inverter', category: 'Electrical', viewBox: [90, 100], size: [80, 90], binding: 'state',
  ports: { left: [8, 50], right: [82, 50], top: [45, 8], bottom: [45, 92] },
  render: p => (
    <Frame p={p}>
      <rect x="8" y="8" width="74" height="84" rx="6" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <line x1="8" y1="92" x2="82" y2="8" stroke="var(--hmi-detail)" strokeWidth={1.5} {...thin} />
      <path d="M20 30 L40 30 M20 38 L40 38" stroke="var(--hmi-detail)" strokeWidth={2.5} />
      <path d="M50 68 Q 55 58, 60 68 T 70 68" fill="none" stroke={accent(p.state)} strokeWidth={2.5} strokeLinecap="round" />
      <Lamp p={p} cx={72} cy={18} />
    </Frame>
  ),
}

const meter: SymbolDef = {
  key: 'meter', name: 'Power Meter', category: 'Electrical', viewBox: [90, 90], size: [96, 96], binding: 'value',
  ports: { left: [4, 45], right: [86, 45] },
  render: p => (
    <Frame p={p}>
      <rect x="4" y="4" width="82" height="82" rx="6" fill={url(p.uid, 'cv')} {...outline(p.state)} />
      <rect x="12" y="16" width="66" height="34" rx="3" fill="var(--hmi-lcd)" stroke="var(--hmi-detail)" strokeWidth={1} {...thin} />
      <text x="72" y="34" textAnchor="end" dominantBaseline="central" fontSize={p.value.length > 6 ? 11 : 14} fontWeight={700}
        fill={p.state === 'stale' ? 'var(--hmi-stale)' : '#6ee7a8'} style={{ fontFamily: 'var(--font-mono)' }}>
        {p.value || '----'}
      </text>
      <text x="45" y="64" textAnchor="middle" dominantBaseline="central" fontSize={9} fontWeight={600} fill="var(--hmi-detail)">
        {p.unit || ' '}
      </text>
      <circle cx="20" cy="76" r="3" fill="var(--hmi-detail)" />
      <circle cx="70" cy="76" r="3" fill="var(--hmi-detail)" />
    </Frame>
  ),
}

const busbar: SymbolDef = {
  key: 'busbar', name: 'Busbar', category: 'Electrical', viewBox: [200, 16], size: [240, 14], binding: 'state', stretch: true,
  render: p => (
    <Frame p={p}>
      <rect x="0" y="2" width="200" height="12" rx="2"
        fill={p.state === 'running' ? 'var(--hmi-run)' : url(p.uid, 'pv')} {...outline(p.state)} />
    </Frame>
  ),
}

const instrument: SymbolDef = {
  key: 'instrument', name: 'Instrument (ISA)', category: 'Instruments', viewBox: [80, 80], size: [84, 108], binding: 'value',
  ports: { left: [10, 40], right: [70, 40], top: [40, 10], bottom: [40, 70] },
  fields: [
    { key: 'tagFunction', label: 'Function (e.g. PT)', type: 'string', default: 'PT' },
    { key: 'loopNumber', label: 'Loop number', type: 'string', default: '101' },
    { key: 'mounting', label: 'Mounting', type: 'select', default: 'field', options: [
      { value: 'field', label: 'Field' }, { value: 'panel', label: 'Control room' }, { value: 'dcs', label: 'DCS / shared display' },
    ] },
  ],
  render: p => {
    const mounting = p.config.mounting ?? 'field'
    return (
      <Frame p={p}>
        {mounting === 'dcs' && <rect x="8" y="8" width="64" height="64" fill="none" {...outline(p.state)} />}
        <circle cx="40" cy="40" r="30" fill="var(--hmi-body-hi)" {...outline(p.state)} />
        {mounting !== 'field' && <line x1="10" x2="70" y1="40" y2="40" {...outline(p.state)} />}
        <text x="40" y="29" textAnchor="middle" dominantBaseline="central" fontSize={14} fontWeight={700} fill="var(--hmi-detail)">
          {p.config.tagFunction ?? 'PT'}
        </text>
        <text x="40" y="52" textAnchor="middle" dominantBaseline="central" fontSize={12} fill="var(--hmi-detail)"
          style={{ fontFamily: 'var(--font-mono)' }}>
          {p.config.loopNumber ?? '101'}
        </text>
      </Frame>
    )
  },
}

const pipeFields: ConfigFieldDef[] = [
  { key: 'runSensorId', label: 'Flow Tag (animates)', type: 'tag', default: null },
]

function PipeStroke({ p, d }: { p: SymbolRenderProps; d: string }) {
  const flowing = p.runState === 'running'
  return (
    <Frame p={p}>
      <path d={d} fill="none" stroke="var(--hmi-pipe-lo)" strokeWidth={16} strokeLinejoin="round" />
      <path d={d} fill="none" stroke="var(--hmi-pipe-hi)" strokeWidth={7} strokeLinejoin="round" opacity={0.75} />
      {flowing && (
        <path d={d} fill="none" stroke="var(--hmi-liquid-hi)" strokeWidth={4} strokeDasharray="8 16"
          strokeLinecap="round" className="hmi-flow" />
      )}
    </Frame>
  )
}

const pipe: SymbolDef = {
  key: 'pipe', name: 'Pipe', category: 'Piping', viewBox: [200, 20], size: [200, 20], binding: 'none', stretch: true, fields: pipeFields,
  render: p => <PipeStroke p={p} d="M0 10 L200 10" />,
}

const pipeVertical: SymbolDef = {
  key: 'pipe_vertical', name: 'Pipe (vertical)', category: 'Piping', viewBox: [20, 200], size: [20, 200], binding: 'none', stretch: true, fields: pipeFields,
  render: p => <PipeStroke p={p} d="M10 0 L10 200" />,
}

const pipeElbow: SymbolDef = {
  key: 'pipe_elbow', name: 'Pipe Elbow', category: 'Piping', viewBox: [60, 60], size: [60, 60], binding: 'none', fields: pipeFields,
  render: p => <PipeStroke p={p} d="M0 30 L20 30 Q30 30 30 40 L30 60" />,
}

const pipeTee: SymbolDef = {
  key: 'pipe_tee', name: 'Pipe Tee', category: 'Piping', viewBox: [80, 50], size: [80, 50], binding: 'none', fields: pipeFields,
  render: p => <PipeStroke p={p} d="M0 12 L80 12 M40 12 L40 50" />,
}

export const SYMBOLS: SymbolDef[] = [
  pump, dosingPump, fan, motor, compressor, conveyor,
  valve, controlValve, checkValve,
  tank, tankHorizontal, silo, mixer,
  clarifier, gravityFilter, heatExchanger, filter,
  breaker, transformer, generator, solar, battery, inverter, meter, busbar,
  instrument, flowMeter,
  pipe, pipeVertical, pipeElbow, pipeTee,
]

export const SYMBOL_MAP: Record<string, SymbolDef> = Object.fromEntries(SYMBOLS.map(s => [s.key, s]))

export const SYMBOL_CATEGORIES: SymbolCategory[] = ['Rotating', 'Valves', 'Vessels', 'Process', 'Electrical', 'Instruments', 'Piping']
