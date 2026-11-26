// The operator view of a screen on a phone (docs/designer.md, phase 5): the
// generated operator layout (operatorLayout.ts) drawn as sections of rows and
// cards. Taps open the same faceplates and trend popups as the 2D stage, and
// controls write through the same command path; the caller (ScadaView)
// provides both. Without the live stream every value shows its age and the
// view says it is not live.
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { SensorConfig, SensorData } from '../../types'
import type { WidgetInstance } from '../widgets/types'
import { getWidgetComponent } from '../widgets/registry'
import { ScreenWidgetView, WidgetErrorBoundary, WidgetPlaceholder } from '../../components/ScreenWidgetView'
import { useBoundInstance } from '../widgets/WidgetShell'
import { valueStyle } from '../../valueStyle'
import { resolveSymbol } from '../hmi/resolve'
import { symbolLiveState } from '../hmi/SymbolView'
import type { EquipState, SymbolDef } from '../hmi/symbols'
import { SYMBOL_MAP } from '../hmi/symbols'
import { generateOperatorLayout, visibleLayout, type GenerateOptions, type MobileItem, type OperatorLayout } from './operatorLayout'
import { ageParts, isLiveValue } from './lastKnown'

/** Library facts the generator uses: which symbols are piping. */
export const SYMBOL_INFO: GenerateOptions = {
  symbol: key => {
    const def = SYMBOL_MAP[key]
    return def ? { binding: def.binding, category: def.category } : undefined
  },
}

/** The operator layout of a set of widgets, as operators see it. */
export function useOperatorLayout(widgets: WidgetInstance[]): OperatorLayout {
  return useMemo(() => visibleLayout(generateOperatorLayout({ widgets }, SYMBOL_INFO)), [widgets])
}

export interface OperatorViewProps {
  widgets: WidgetInstance[]
  sensors: SensorConfig[]
  /** Current values: the live store, or a snapshot when offline. */
  values: Record<string, SensorData>
  /** The live stream is up. False: every value is shown with its age. */
  live: boolean
  screenName?: string
  /** When the values were last received (offline snapshot), for the banner. */
  asOf?: number | null
  /** Tapping a row or card: open its faceplate or trend (as on the stage). Absent: nothing to open (offline). */
  onOpen?: (id: string) => void
  /** Whether tapping does something for this widget. */
  activatable?: (w: WidgetInstance) => boolean
  /** Operator writes (controls); absent without the operate permission or when not live. */
  onWriteValue?: (widgetId: string, sensorId: number, value: number) => void
  /** The widget whose faceplate is open. */
  selectedId?: string | null
  /** Switch to the full 2D stage. */
  onShowStage?: () => void
  /** Open the phone app pairing page. */
  onPairPhone?: () => void
  /** Offline: try to reach the server again. */
  onRetry?: () => void
}

/** "12 min ago" for a reading time. */
export function useAgeText(): (ts: number | undefined, now: number) => string {
  const { t } = useTranslation()
  return (ts, now) => {
    if (ts == null) return ''
    const { n, unit } = ageParts(now - ts)
    return t(`operator.age.${unit}`, { n })
  }
}

/** A clock that ticks every `ms` (ages move on while nothing else changes). */
function useNow(ms: number): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms)
    return () => clearInterval(t)
  }, [ms])
  return now
}

const timeOf = (ts: number, lang: string) => {
  try { return new Date(ts).toLocaleTimeString(lang, { hour: '2-digit', minute: '2-digit' }) } catch { return new Date(ts).toISOString().slice(11, 16) }
}

export function OperatorView(props: OperatorViewProps) {
  const { widgets, sensors, values, live, screenName, asOf, onShowStage, onPairPhone, onRetry } = props
  const { t, i18n } = useTranslation()
  const layout = useOperatorLayout(widgets)
  const now = useNow(5000)
  const age = useAgeText()
  const byId = useMemo(() => new Map(widgets.map(w => [w.id, w])), [widgets])
  const lastSeen = asOf ?? null

  return (
    <div data-testid="operator-view" className="vt-operator" style={{ flex: 1, minHeight: 0, overflowY: 'auto', background: 'var(--bg)', paddingBottom: 24 }}>
      <div style={{
        position: 'sticky', top: 0, zIndex: 4, display: 'flex', alignItems: 'center', gap: 8, padding: '10px 14px',
        background: 'var(--bg)', borderBottom: '1px solid var(--border)',
      }}>
        <div style={{ flex: 1, minWidth: 0, fontSize: 15, fontWeight: 650, color: 'var(--fg)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{screenName}</div>
        <span data-testid="operator-live" role="status" style={{
          display: 'inline-flex', alignItems: 'center', gap: 6, padding: '3px 9px', borderRadius: 999, fontSize: 12, fontWeight: 650,
          border: `1px solid ${live ? 'color-mix(in srgb, var(--hmi-run, #22c55e) 45%, transparent)' : 'var(--warn)'}`,
          color: live ? 'var(--fg)' : 'var(--warn)',
        }}>
          <span aria-hidden style={{ width: 7, height: 7, borderRadius: 4, background: live ? 'var(--hmi-run, #22c55e)' : 'var(--warn)' }} />
          {live ? t('operator.live') : t('operator.offline')}
        </span>
        {onShowStage && (
          <button type="button" onClick={onShowStage} data-testid="operator-show-stage" style={chipBtn}>{t('operator.stageView')}</button>
        )}
      </div>
      {!live && (
        <div data-testid="operator-offline" role="alert" style={{
          margin: '10px 12px 0', padding: '10px 12px', borderRadius: 8, fontSize: 13, lineHeight: 1.4,
          border: '1px solid var(--warn)', background: 'color-mix(in srgb, var(--warn) 10%, transparent)', color: 'var(--fg)',
          display: 'flex', gap: 10, alignItems: 'center',
        }}>
          <span style={{ flex: 1 }}>
            {lastSeen != null
              ? t('operator.offlineSince', { time: timeOf(lastSeen, i18n.language), age: age(lastSeen, now) })
              : t('operator.reconnecting')}
          </span>
          {onRetry && <button type="button" onClick={onRetry} style={chipBtn}>{t('operator.retry')}</button>}
        </div>
      )}
      {layout.sections.length === 0 && (
        <div style={{ padding: 24, textAlign: 'center', color: 'var(--text-muted)', fontSize: 14 }}>{t('operator.empty')}</div>
      )}
      {layout.sections.map(s => (
        <section key={s.id ?? '_'} aria-label={s.title || undefined} style={{ padding: '8px 12px 0' }}>
          {s.title && <h2 style={{ margin: '10px 2px 8px', fontSize: 12, fontWeight: 700, letterSpacing: '.08em', textTransform: 'uppercase', color: 'var(--text-dim)' }}>{s.title}</h2>}
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, minmax(0, 1fr))', gap: 8 }}>
            {s.items.map(item => {
              const w = byId.get(item.id)
              if (!w) return null
              return (
                <WidgetErrorBoundary key={item.id} type={w.type} resetKey={w} fallback={<div style={{ gridColumn: `span ${item.span}` }}><WidgetPlaceholder text={t('operator.failed')} detail={w.type} /></div>}>
                  <OperatorItem item={item} widget={w} {...props} now={now} age={age} />
                </WidgetErrorBoundary>
              )
            })}
          </div>
        </section>
      ))}
      {onPairPhone && (
        <p style={{ margin: '18px 14px 0', fontSize: 12.5, lineHeight: 1.5, color: 'var(--text-muted)' }}>
          {t('operator.alarmsHint')}{' '}
          <button type="button" onClick={onPairPhone} data-testid="operator-pair-phone" style={{ ...linkBtn }}>{t('operator.pairPhone')}</button>
        </p>
      )}
    </div>
  )
}

const chipBtn: React.CSSProperties = {
  padding: '6px 10px', minHeight: 32, borderRadius: 6, border: '1px solid var(--border)', background: 'transparent',
  color: 'var(--fg)', fontSize: 12, fontWeight: 600, cursor: 'pointer', whiteSpace: 'nowrap',
}
const linkBtn: React.CSSProperties = {
  padding: 0, border: 'none', background: 'none', color: 'var(--cyan)', textDecoration: 'underline', cursor: 'pointer', font: 'inherit',
}

type ItemProps = OperatorViewProps & { item: MobileItem; widget: WidgetInstance; now: number; age: (ts: number | undefined, now: number) => string }

const STATE_KEY: Record<EquipState, string> = { idle: 'idle', running: 'running', stopped: 'stopped', fault: 'fault', stale: 'nodata' }

/** Running/stopped in the symbol's words (valves open and close). */
function stateKey(def: SymbolDef, state: EquipState): string {
  const valve = def.category === 'Valves' || def.key.includes('valve')
  if (valve && state === 'running') return 'open'
  if (valve && state === 'stopped') return 'closed'
  return STATE_KEY[state]
}

function OperatorItem({ item, widget, sensors, values, live, onOpen, activatable, onWriteValue, selectedId, now, age }: ItemProps) {
  const { t } = useTranslation()
  const instance = useBoundInstance(widget, sensors, values, true)
  // A bound `visible` hides it on phones as on the stage.
  if (instance.config?.visible === false) return null
  const sensor = instance.sensorId != null ? sensors.find(s => s.id === instance.sensorId) : undefined
  const data = instance.sensorId != null ? values[String(instance.sensorId)] : undefined
  const fresh = isLiveValue(live, data, now)
  const label = String(instance.config?.label || instance.config?.text || instance.name || sensor?.name || instance.type)
  const canTap = !!onOpen && (activatable?.(instance) ?? false)
  const tap = canTap ? () => onOpen!(instance.id) : undefined
  const selected = selectedId === instance.id
  const vs = valueStyle(data)
  const valueText = data && Number.isFinite(data.value) ? data.value.toFixed(sensor?.decimals ?? 1) : '—'
  const staleNote = data && !fresh ? age(data.ts_ms, now) || t('operator.notLive') : ''

  const box: React.CSSProperties = {
    gridColumn: `span ${item.span}`, minWidth: 0, borderRadius: 10, background: 'var(--card)',
    border: `1px solid ${selected ? 'var(--cyan)' : vs.state === 'crit' || vs.state === 'warn' ? vs.color! : 'var(--border)'}`,
  }

  if (item.kind === 'note') {
    return <div data-testid={`operator-item-${item.id}`} style={{ ...box, background: 'transparent', border: 'none', padding: '2px 4px', fontSize: 13, color: 'var(--text-dim)' }}>{label}</div>
  }

  if (item.kind === 'equipment' || item.kind === 'value') {
    const r = item.kind === 'equipment' ? resolveSymbol(instance) : null
    const st = r ? symbolLiveState(r.def, r.config, instance.sensorId, sensor, data, values) : null
    const state: EquipState | null = st ? st.state : null
    const stateText = r && state && state !== 'idle' ? t(`operator.state.${stateKey(r.def, state)}`) : ''
    // A state symbol's primary tag is its run state; level and value symbols show the number.
    const showValue = item.kind === 'value' || (r != null && r.def.binding !== 'state' && data != null)
    const content = (
      <>
        {r && st && <Thumb def={r.def} config={r.config} state={fresh ? state! : 'stale'} level={st.level} runState={fresh ? st.runState : 'stale'} />}
        <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
          <span style={{ fontSize: 14, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{label}</span>
          {stateText && (
            <span data-testid={`operator-state-${item.id}`} data-live={fresh ? 'true' : 'false'} style={{
              fontSize: 11.5, fontWeight: 700, letterSpacing: '.06em', textTransform: 'uppercase',
              color: fresh ? stateColor(state!) : 'var(--text-muted)',
            }}>
              {stateText}{st?.manual ? ` · ${t('operator.manual')}` : ''}{st?.interlocked ? ` · ${t('operator.interlock')}` : ''}
              {!showValue && staleNote ? <span style={{ marginLeft: 6, letterSpacing: 0, textTransform: 'none', fontWeight: 500, color: 'var(--warn)' }}>{staleNote}</span> : null}
            </span>
          )}
          {!stateText && vs.label && vs.state !== 'normal' && <span style={{ fontSize: 11.5, fontWeight: 700, color: vs.color }}>{vs.marker ? `${vs.marker} ` : ''}{vs.label}</span>}
        </span>
        {showValue && (
          <span style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', flexShrink: 0 }}>
            <span data-testid={`operator-value-${item.id}`} data-live={fresh ? 'true' : 'false'} style={{
              fontFamily: 'var(--font-mono)', fontSize: 17, fontWeight: 650, fontVariantNumeric: 'tabular-nums',
              color: fresh ? vs.color ?? 'var(--fg)' : 'var(--text-muted)',
              textDecoration: fresh ? undefined : 'underline dotted',
            }}>
              {valueText}{sensor?.unit ? <span style={{ fontSize: 12, marginLeft: 3, color: 'var(--text-muted)' }}>{sensor.unit}</span> : null}
            </span>
            {staleNote && <span data-testid={`operator-age-${item.id}`} style={{ fontSize: 11, color: 'var(--warn)' }}>{staleNote}</span>}
          </span>
        )}
      </>
    )
    const rowStyle: React.CSSProperties = {
      ...box, display: 'flex', alignItems: 'center', gap: 10, padding: item.kind === 'equipment' ? '8px 10px' : '10px 12px', minHeight: 56,
      textAlign: 'left', font: 'inherit', color: 'var(--fg)', cursor: tap ? 'pointer' : 'default', width: '100%', boxSizing: 'border-box',
    }
    return tap ? (
      <button type="button" data-testid={`operator-item-${item.id}`} onClick={tap} aria-label={t('operator.open', { name: label })} aria-pressed={selected} style={rowStyle}>{content}</button>
    ) : (
      <div data-testid={`operator-item-${item.id}`} style={rowStyle}>{content}</div>
    )
  }

  // Cards and controls: the widget itself, full width (or half), in a card.
  return <CardItem item={item} instance={instance} sensor={sensor} data={data} sensors={sensors} values={values} label={label} tap={tap}
    box={box} onWrite={item.kind === 'control' && onWriteValue && live ? (v: number) => instance.sensorId != null && onWriteValue(instance.id, instance.sensorId, v) : undefined}
    staleNote={!live ? t('operator.notLive') : ''} />
}

const stateColor = (state: EquipState) =>
  state === 'running' ? 'var(--hmi-run)' : state === 'fault' ? 'var(--hmi-fault)' : state === 'stale' ? 'var(--hmi-stale)' : 'var(--text-muted)'

function CardItem({ item, instance, sensor, data, sensors, values, label, tap, box, onWrite, staleNote }: {
  item: MobileItem; instance: WidgetInstance; sensor: SensorConfig | undefined; data: SensorData | undefined
  sensors: SensorConfig[]; values: Record<string, SensorData>; label: string; tap?: () => void; box: React.CSSProperties
  onWrite?: (v: number) => void; staleNote: string
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const measure = () => setWidth(el.clientWidth)
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  const control = item.kind === 'control'
  // Keep the widget's own proportions within the card, within limits.
  const aspect = instance.height > 0 && instance.width > 0 ? instance.height / instance.width : 0.5
  const height = control ? Math.max(48, Math.min(120, instance.height)) : Math.round(Math.max(140, Math.min(320, (width || 320) * aspect)))
  // Before the card is measured (and where nothing is laid out) draw at a phone's width.
  const sized = { ...instance, width: width || Math.min(instance.width, 340), height }
  const Widget = getWidgetComponent(instance.type)
  return (
    <div data-testid={`operator-item-${item.id}`} style={{ ...box, padding: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span style={{ flex: 1, minWidth: 0, fontSize: 13, fontWeight: 600, color: 'var(--text-dim)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{label}</span>
        {staleNote && <span style={{ fontSize: 11, color: 'var(--warn)' }}>{staleNote}</span>}
        {tap && <button type="button" onClick={tap} aria-label={label} style={{ ...chipBtn, padding: '2px 8px', minHeight: 28 }}>↗</button>}
      </div>
      <div ref={ref} style={{ position: 'relative', height, opacity: staleNote ? 0.6 : 1 }}>
        {(Widget ? (
          <Widget instance={sized} sensor={sensor} data={data} sensors={sensors} currentValues={values} editMode={false} selected={false} onWriteValue={onWrite} />
        ) : (
          <ScreenWidgetView type={instance.type} config={instance.config ?? {}} surface="scada" width={sized.width} height={height} editMode={false} sensors={sensors} values={values} />
        ))}
      </div>
    </div>
  )
}

/** The symbol drawn small in its live state. */
function Thumb({ def, config, state, level, runState }: { def: SymbolDef; config: Record<string, any>; state: EquipState; level: number | null; runState: EquipState }) {
  const uid = 'm' + useId().replace(/[^a-zA-Z0-9]/g, '')
  const [vw, vh] = def.viewBox
  return (
    <svg viewBox={`0 0 ${vw} ${vh}`} width={40} height={40} preserveAspectRatio="xMidYMid meet" aria-hidden style={{ flexShrink: 0, overflow: 'visible' }}>
      {def.render({ uid, state, level, value: '', unit: '', runState, config: { ...config, label: '', showValue: false, showStatus: false, showPercent: false } })}
    </svg>
  )
}
