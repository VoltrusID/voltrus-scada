import React, { useMemo, useState, useEffect, useRef } from 'react'
import { SensorConfig, SOURCE_CATEGORY } from './types'
import { Chart } from './Chart'
import { valueStyle, framesWidget } from './valueStyle'
import { openTrend } from './trend/TrendPopup'
import { parseConfig } from './styles'
import { useSensorCurrentValue, useChartValues } from './store/dashboardStore'
import { useRemoveSensorFromScreen } from './queries'
import { useInView } from './hooks/useInView'

/** The live value is a button: it opens the trend popup for the tag. */
const valueButton = (color: string | undefined): React.CSSProperties => ({
  color, font: 'inherit', background: 'none', border: 'none', padding: 0, cursor: 'pointer', textAlign: 'left',
})

const useIsSmallScreen = () => {
  const [small, setSmall] = useState(window.innerWidth <= 480)
  useEffect(() => {
    const mq = window.matchMedia('(max-width: 480px)')
    const handler = (e: MediaQueryListEvent) => setSmall(e.matches)
    mq.addEventListener('change', handler)
    return () => mq.removeEventListener('change', handler)
  }, [])
  return small
}

interface Props {
  sensor: SensorConfig
  editMode: boolean
  activeScreen: string
  editingId: number | null
  shortcut: string | undefined
  loadSensorsForScreen: () => void
  setEditingId: (id: number | null) => void
  showDragHandle?: boolean
  selected?: boolean
  mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts'
  onClick?: (e: React.MouseEvent) => void
}

export const SensorCard = React.memo(function SensorCard({
  sensor: s, editMode, activeScreen, editingId,
  shortcut, loadSensorsForScreen, setEditingId,
  showDragHandle, selected, mode, onClick,
}: Props) {
  // Off-screen cards hold their last reading instead of re-rendering on every
  // update; scrolling one into view catches it up.
  const rootRef = useRef<HTMLDivElement>(null)
  const offScreen = !useInView(rootRef)
  const cv = useSensorCurrentValue(s.id, offScreen)
  const chartData = useChartValues(s.id, mode === 'history' ? 'history' : 'live', offScreen)
  const isSmall = useIsSmallScreen()
  const removeSensorMut = useRemoveSensorFromScreen()

  const handleRemove = (e: React.MouseEvent) => {
    e.stopPropagation()
    removeSensorMut.mutate({ screenId: activeScreen, sensorId: s.id })
  }

  // Alarm state and quality, the same way as every other bound value (valueStyle.ts).
  const vs = valueStyle(cv);
  const st = vs.state === 'normal' ? 'normal' : vs.label.toLowerCase();
  const stateColor = vs.color ?? 'var(--ok)';
  const stateTint = vs.tint ?? 'color-mix(in srgb, var(--ok) 12%, transparent)';
  const ct = s.chart_type || 'sparkline';
  const hideStatus = ct === 'image' || ct === 'text';
  const isCompact = isSmall && !hideStatus;

  // No reading yet shows a dash, never a stand-in number.
  const formattedValue = vs.state === 'offline' ? 'OFFLINE' : cv && Number.isFinite(cv.value) ? cv.value.toFixed(s.decimals) : '—';
  // A value that is not good stays visible for context, in the quality colour, with a marker.
  const qLabel = vs.title;
  const cardAriaLabel = `${s.name}: ${formattedValue} ${s.unit || ''}${vs.label ? ` (${vs.title || vs.label})` : ''}`;
  const valueColor = vs.state === 'normal' ? s.color : vs.color;
  const qualityMark = vs.marker && vs.state !== 'offline'
    ? <span title={vs.title} aria-hidden="true" style={{ fontSize: '0.5em', marginLeft: 4, verticalAlign: 'super', color: vs.color }}>{vs.marker}</span>
    : null;
  const frame = framesWidget(vs) && vs.state !== 'offline' ? vs.color : undefined;

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onClick?.(e as unknown as React.MouseEvent);
    }
  };

  const sourceTag = useMemo(() => SOURCE_CATEGORY[s.source_type]?.tag ?? s.source_type, [s.source_type]);
  const sourceTagColor = useMemo(() => SOURCE_CATEGORY[s.source_type]?.color ?? 'var(--muted)', [s.source_type]);
  const [hovered, setHovered] = useState(false);

  // Compact mode for small screens
  if (isCompact) {
    return (
      <div
        ref={rootRef}
        role="button"
        tabIndex={0}
        aria-label={cardAriaLabel}
        onKeyDown={handleKeyDown}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        style={{
          background: 'var(--card)',
          border: `1px solid ${selected ? 'var(--cyan)' : editingId === s.id ? 'var(--cyan)' : frame ?? (hovered ? 'var(--border-light)' : 'var(--border)')}`,
          borderRadius: 12,
          overflow: 'hidden',
          height: '100%',
          display: 'flex',
          flexDirection: 'column',
          justifyContent: 'center',
          padding: '12px 14px',
          transition: 'border-color 0.2s, box-shadow 0.2s',
          boxShadow: selected ? '0 0 0 2px var(--cyan)' : hovered ? '0 4px 16px rgba(0,0,0,0.3)' : '0 1px 3px rgba(0,0,0,0.2)',
        }}
        onClick={onClick}
      >
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 6 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.04em', minWidth: 0 }}>
            <div style={{ width: 8, height: 8, borderRadius: '50%', background: stateColor, flexShrink: 0 }} />
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{s.name}</span>
          </div>
          <div style={{ fontSize: 10, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.05em', padding: '3px 8px', borderRadius: 4, flexShrink: 0, minWidth: 44, minHeight: 28, display: 'flex', alignItems: 'center', justifyContent: 'center',
            background: stateTint, color: stateColor }}>{st}</div>
        </div>
        <div style={{ fontFamily: "var(--font-mono)", fontSize: 28, fontWeight: 700, lineHeight: 1.2 }}>
          <button type="button" onClick={e => { e.stopPropagation(); openTrend(s.id) }} title={qLabel ? `${qLabel}. Open the trend` : 'Open the trend'} aria-label={`Trend of ${s.name}`} style={valueButton(valueColor)}>{formattedValue}</button>{qualityMark}
          <span style={{ fontSize: 14, color: 'var(--muted)', fontWeight: 400, marginLeft: 4 }}>{s.unit}</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 6, fontSize: 10, color: 'var(--muted)' }}>
          <span style={{ padding: '1px 5px', borderRadius: 3, background: `${sourceTagColor}18`, color: sourceTagColor, fontWeight: 700, letterSpacing: '.03em' }}>{sourceTag}</span>
          {editMode && activeScreen !== 'all' && (
            <button
              onClick={handleRemove}
              aria-label={`Remove ${s.name} from screen`}
              style={{ background: 'var(--bg)', border: '1px solid var(--crit)', borderRadius: 4, color: 'var(--crit)', cursor: 'pointer', padding: '4px 8px', fontSize: 10, lineHeight: 1, minWidth: 44, minHeight: 28 }}
            >Remove</button>
          )}
        </div>
      </div>
    )
  }

  return (
    <div
      ref={rootRef}
      role="button"
      tabIndex={0}
      aria-label={cardAriaLabel}
      onKeyDown={handleKeyDown}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        background: 'var(--card)',
        border: `1px solid ${selected ? 'var(--cyan)' : editingId === s.id ? 'var(--cyan)' : frame ?? (hovered ? 'var(--border-light)' : 'var(--border)')}`,
        borderRadius: 12,
        overflow: 'hidden',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        transition: 'border-color 0.2s, box-shadow 0.2s',
        boxShadow: selected ? '0 0 0 2px var(--cyan)' : hovered ? '0 4px 16px rgba(0,0,0,0.3)' : '0 1px 3px rgba(0,0,0,0.2)',
      }}
      onClick={onClick}
    >
      {!hideStatus && <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '12px 14px 8px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.05em', minWidth: 0, overflow: 'hidden' }}>
          {showDragHandle && editMode && (
            <span className="drag-handle" style={{ cursor: 'grab', display: 'inline-flex', alignItems: 'center', padding: '0 2px', opacity: 0.4 }}>
              <svg width="8" height="12" viewBox="0 0 8 12">
                <circle cx="2" cy="2" r="1.2" fill="var(--fg)"/><circle cx="6" cy="2" r="1.2" fill="var(--fg)"/>
                <circle cx="2" cy="6" r="1.2" fill="var(--fg)"/><circle cx="6" cy="6" r="1.2" fill="var(--fg)"/>
                <circle cx="2" cy="10" r="1.2" fill="var(--fg)"/><circle cx="6" cy="10" r="1.2" fill="var(--fg)"/>
              </svg>
            </span>
          )}
          <div style={{ width: 8, height: 8, borderRadius: '50%', background: stateColor }} />
          {s.name}
          <span style={{ fontSize: 9, padding: '1px 5px', borderRadius: 3, background: `${sourceTagColor}18`, color: sourceTagColor, fontWeight: 700, letterSpacing: '.03em', textTransform: 'none' }}>{sourceTag}</span>
          {cv?.simulated && (
            <span style={{ fontSize: 9, padding: '1px 5px', borderRadius: 3, background: 'rgba(234,179,8,.15)', color: 'var(--warn)', fontWeight: 700, letterSpacing: '.05em', textTransform: 'uppercase' }}>SIM</span>
          )}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          {shortcut && (
            <kbd style={{
              fontFamily: "var(--font-mono)", fontSize: 10, fontWeight: 600,
              padding: '2px 6px', borderRadius: 3,
              background: 'var(--bg)', border: '1px solid var(--border)',
              color: 'var(--muted)', lineHeight: 1,
            }}>{shortcut}</kbd>
          )}
          {editMode && activeScreen !== 'all' && (
            <button aria-label={`Remove ${s.name} from screen`} onClick={handleRemove} style={{ background: 'var(--bg)', border: '1px solid var(--crit)', borderRadius: 4, color: 'var(--crit)', cursor: 'pointer', padding: '2px 6px', fontSize: 10, lineHeight: 1 }}>Remove</button>
          )}
          <div style={{ fontSize: 10, fontWeight: 700, textTransform: 'uppercase', letterSpacing: '.06em', padding: '3px 8px', borderRadius: 6, minWidth: 52, textAlign: 'center' as const,
            background: stateTint, color: stateColor }}>{st}</div>
        </div>
      </div>}
      {hideStatus && editMode && (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '4px 8px', opacity: 0.5 }}>
          <span className="drag-handle" style={{ cursor: 'grab', display: 'inline-flex', alignItems: 'center', gap: 4 }}>
            <svg width="8" height="12" viewBox="0 0 8 12">
              <circle cx="2" cy="2" r="1.2" fill="var(--fg)"/><circle cx="6" cy="2" r="1.2" fill="var(--fg)"/>
              <circle cx="2" cy="6" r="1.2" fill="var(--fg)"/><circle cx="6" cy="6" r="1.2" fill="var(--fg)"/>
              <circle cx="2" cy="10" r="1.2" fill="var(--fg)"/><circle cx="6" cy="10" r="1.2" fill="var(--fg)"/>
            </svg>
            <span style={{ fontSize: 10, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.04em' }}>{s.name}</span>
          </span>
          <button onClick={handleRemove} style={{ background: 'var(--bg)', border: '1px solid var(--crit)', borderRadius: 4, color: 'var(--crit)', cursor: 'pointer', padding: '2px 6px', fontSize: 10, lineHeight: 1 }}>Remove</button>
        </div>
      )}
      {!hideStatus && (
        <div style={{ fontFamily: "var(--font-mono)", fontSize: 24, fontWeight: 700, padding: '0 14px', lineHeight: 1.3 }}>
          <button type="button" onClick={e => { e.stopPropagation(); openTrend(s.id) }} title={qLabel ? `${qLabel}. Open the trend` : 'Open the trend'} aria-label={`Trend of ${s.name}`} style={valueButton(valueColor)}>{formattedValue}</button>{qualityMark}
          <span style={{ fontSize: 13, color: 'var(--muted)', fontWeight: 500, marginLeft: 4 }}>{s.unit}</span>
        </div>
      )}
      <div style={{ flex: 1, minHeight: 0 }}>
        <Chart
          chartType={s.chart_type || 'sparkline'}
          data={chartData}
          color={s.color}
          warn={s.warn_val}
          crit={s.crit_val}
          min={s.min_val}
          max={s.max_val}
          decimals={s.decimals}
          unit={s.unit}
          imageUrl={(parseConfig(s.source_config) as Record<string, string>).image_url}
          textContent={(parseConfig(s.source_config) as Record<string, string>).text_content}
          simulated={cv?.simulated}
        />
      </div>
    </div>
  )
}, (prev, next) => {
  // Custom comparison: only re-render if relevant props changed
  if (prev.sensor.id !== next.sensor.id) return false
  if (prev.editMode !== next.editMode) return false
  if (prev.activeScreen !== next.activeScreen) return false
  if (prev.editingId !== next.editingId) return false
  if (prev.selected !== next.selected) return false
  if (prev.shortcut !== next.shortcut) return false
  if (prev.showDragHandle !== next.showDragHandle) return false
  if (prev.mode !== next.mode) return false
  return true
})
