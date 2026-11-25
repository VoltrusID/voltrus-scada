// ISA-101 state style rules for the data symbol library (symbols/*.json).
//
// Plain TypeScript (no React, no Vite) so node tests and any other renderer
// can use it. `STYLE_RULES` mirrors symbols/_styles.json, the copy other
// renderers (the native console) read; tests/symbolLibrary.test.ts keeps the
// two equal.
//
// The rules: grey, thin outlines, no gradients. Running / open is filled,
// stopped / closed is hollow, so state reads in greyscale. Colour appears only
// for abnormal conditions: fault (red, blinking), no data (magenta, dashed)
// and alarms (priority badge with its own shape and number). Manual mode and
// interlocks are not abnormal: they show as grey letter boxes.

export type EquipState = 'idle' | 'running' | 'stopped' | 'fault' | 'stale'

/** How a layer of a symbol is painted. */
export type Paint =
  /** Main body: filled when running, hollow when stopped, red on fault. */
  | 'body'
  /** Vessel walls, actuators, bubbles: always hollow, outline follows the state. */
  | 'shell'
  /** Stroke only (nozzles, stands, stems). */
  | 'line'
  /** Thin stroke only (ticks, media, internals). */
  | 'detail'
  /** Stroke only, drawn over a body: contrasts with the body fill (stage colour when filled). */
  | 'inner'
  /** Filled with the line colour (arrow heads, junction dots). */
  | 'solid'
  /** Moving flow: drawn only while the run tag is on, dashed and animated. */
  | 'flow'
  /** Secondary drive (agitator, rake): thin when stopped, bold when the run tag is on. */
  | 'run'

export const PAINTS: Paint[] = ['body', 'shell', 'line', 'detail', 'inner', 'solid', 'flow', 'run']

export interface PaintStyle {
  stroke: string
  strokeWidth: number
  fill: string
  dash?: string
  /** False: the layer is not drawn in this state. */
  visible: boolean
  /** CSS animation class (`hmi-flow`). */
  animate?: string
}

export type BadgeShape = 'box' | 'square' | 'triangle' | 'diamond'

export interface Badge {
  kind: 'alarm' | 'manual' | 'interlock'
  text: string
  shape: BadgeShape
  fill: string
  stroke: string
  color: string
  blink?: boolean
}

export interface StyleModifiers {
  /** State of the secondary run tag (pipe flow, agitator, rake, backwash). */
  runState?: EquipState
  manual?: boolean
  interlocked?: boolean
  alarmPriority?: 1 | 2 | 3 | null
}

export interface SymbolStyle {
  paints: Record<Paint, PaintStyle>
  /** The whole symbol blinks (fault). */
  blink: boolean
  /** Colour of plain text. */
  text: string
  /** Colour of text drawn on a body (letters inside a motor circle). */
  textOnBody: string
  /** Letter boxes and the alarm indicator, in drawing order (top to bottom). */
  badges: Badge[]
}

/** Theme variable names (without `--`); `v()` turns one into a CSS value. */
export const STYLE_RULES = {
  format: 'voltrus-symbol-styles/1',
  states: {
    idle: { stroke: 'isa-line', width: 1.75, fill: 'isa-off' },
    stopped: { stroke: 'isa-line', width: 1.75, fill: 'isa-off' },
    running: { stroke: 'isa-line', width: 1.75, fill: 'isa-on' },
    fault: { stroke: 'hmi-fault', width: 2.5, fill: 'hmi-fault', blink: true },
    stale: { stroke: 'hmi-stale', width: 1.75, fill: 'isa-off', dash: '4 3' },
  },
  paints: {
    body: { fill: 'state' },
    shell: { fill: 'isa-off' },
    line: { fill: 'none' },
    detail: { fill: 'none', width: 1 },
    inner: { fill: 'none', width: 1.25, strokeOnFilled: 'hmi-stage' },
    solid: { fill: 'stroke' },
    flow: { fill: 'none', stroke: 'isa-on', dash: '6 18', animate: 'hmi-flow', when: 'run' },
    run: { fill: 'none', width: 1, widthRunning: 2.5 },
  },
  text: { color: 'isa-text', onBodyFilled: 'hmi-stage', onBodyHollow: 'isa-line' },
  badges: {
    manual: { text: 'M', shape: 'box', fill: 'isa-off', stroke: 'isa-line', color: 'isa-text' },
    interlock: { text: 'I', shape: 'box', fill: 'isa-off', stroke: 'isa-line', color: 'isa-text' },
    alarm: {
      1: { text: '1', shape: 'square', fill: 'isa-alarm-1', stroke: 'isa-alarm-1', color: 'isa-alarm-1-text', blink: true },
      2: { text: '2', shape: 'triangle', fill: 'isa-alarm-2', stroke: 'isa-alarm-2', color: 'isa-alarm-2-text' },
      3: { text: '3', shape: 'diamond', fill: 'isa-alarm-3', stroke: 'isa-alarm-3', color: 'isa-alarm-3-text' },
    },
  },
  colors: {
    dark: {
      'isa-line': '#b4bcc6', 'isa-on': '#c9d0d8', 'isa-off': '#1d2128', 'isa-level': '#4a5361', 'isa-text': '#e8edf2',
      'hmi-stage': '#15181d', 'hmi-fault': '#f04646', 'hmi-stale': '#d65ce8',
      'isa-alarm-1': '#f04646', 'isa-alarm-1-text': '#ffffff', 'isa-alarm-2': '#f5821f', 'isa-alarm-2-text': '#111111',
      'isa-alarm-3': '#f2d22e', 'isa-alarm-3-text': '#111111',
    },
    light: {
      'isa-line': '#3d434b', 'isa-on': '#3d434b', 'isa-off': '#eceef1', 'isa-level': '#b3bac3', 'isa-text': '#111827',
      'hmi-stage': '#dfe2e6', 'hmi-fault': '#b91c1c', 'hmi-stale': '#9b2aaa',
      'isa-alarm-1': '#c81e1e', 'isa-alarm-1-text': '#ffffff', 'isa-alarm-2': '#e2700f', 'isa-alarm-2-text': '#111111',
      'isa-alarm-3': '#e8c00e', 'isa-alarm-3-text': '#111111',
    },
  },
} as const

/** CSS value of a theme colour name, `none` passes through. */
export const v = (name: string) => (name === 'none' ? 'none' : `var(--${name})`)

/** Style of every paint, the badges and the blink for one live state. */
export function symbolStyle(state: EquipState, mods: StyleModifiers = {}): SymbolStyle {
  const s = STYLE_RULES.states[state] as { stroke: string; width: number; fill: string; dash?: string; blink?: boolean }
  const stroke = v(s.stroke)
  const filled = state === 'running' || state === 'fault'
  const runOn = mods.runState === 'running'
  const base = { stroke, strokeWidth: s.width, dash: s.dash, visible: true }
  const thin = (w: number) => (state === 'fault' ? w + 0.75 : w)

  const paints: Record<Paint, PaintStyle> = {
    body: { ...base, fill: v(s.fill) },
    shell: { ...base, fill: v('isa-off') },
    line: { ...base, fill: 'none' },
    detail: { ...base, strokeWidth: thin(1), fill: 'none' },
    inner: { ...base, stroke: filled ? v('hmi-stage') : stroke, strokeWidth: thin(1.25), fill: 'none' },
    solid: { ...base, fill: stroke },
    flow: { stroke: v('isa-on'), strokeWidth: s.width, fill: 'none', dash: '6 18', animate: 'hmi-flow', visible: runOn },
    run: { ...base, strokeWidth: runOn ? 2.5 : thin(1), fill: 'none' },
  }

  const badges: Badge[] = []
  const b = STYLE_RULES.badges
  if (mods.alarmPriority) {
    const a = b.alarm[mods.alarmPriority] as { text: string; shape: BadgeShape; fill: string; stroke: string; color: string; blink?: boolean }
    badges.push({ kind: 'alarm', text: a.text, shape: a.shape, fill: v(a.fill), stroke: v(a.stroke), color: v(a.color), blink: !!a.blink })
  }
  for (const kind of ['manual', 'interlock'] as const) {
    if (kind === 'manual' ? !mods.manual : !mods.interlocked) continue
    const m = b[kind]
    badges.push({ kind, text: m.text, shape: m.shape, fill: v(m.fill), stroke: v(m.stroke), color: v(m.color) })
  }

  return {
    paints,
    blink: !!s.blink,
    text: v(STYLE_RULES.text.color),
    textOnBody: v(filled ? STYLE_RULES.text.onBodyFilled : STYLE_RULES.text.onBodyHollow),
    badges,
  }
}

/** Alarm priority of a tag status: crit is priority 1, warn priority 2. */
export function alarmPriorityOf(status: string | undefined | null): 1 | 2 | null {
  return status === 'crit' ? 1 : status === 'warn' ? 2 : null
}
