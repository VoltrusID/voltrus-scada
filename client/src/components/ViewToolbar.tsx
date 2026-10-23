import { useState, useEffect } from 'react'
import { HistoryRangeBar } from '../CustomRangePicker'

const useIsMobile = () => {
  const [mobile, setMobile] = useState(window.innerWidth <= 768)
  useEffect(() => {
    const mq = window.matchMedia('(max-width: 768px)')
    const handler = (e: MediaQueryListEvent) => setMobile(e.matches)
    mq.addEventListener('change', handler)
    return () => mq.removeEventListener('change', handler)
  }, [])
  return mobile
}

/* ── Shared styles ── */
const SEGMENT_BTN: React.CSSProperties = {
  fontFamily: 'var(--font-ui)',
  fontSize: 11,
  fontWeight: 600,
  padding: '4px 10px',
  border: 'none',
  cursor: 'pointer',
  display: 'flex',
  alignItems: 'center',
  gap: 4,
  background: 'transparent',
  color: 'var(--muted)',
  transition: 'color 0.15s, background 0.15s',
  whiteSpace: 'nowrap',
}

const SEGMENT_ACTIVE_CYAN: React.CSSProperties = {
  ...SEGMENT_BTN,
  color: 'var(--cyan)',
  background: 'var(--card)',
  boxShadow: 'inset 0 0 0 1px var(--border)',
  borderRadius: 6,
}

const SEGMENT_ACTIVE_WARN: React.CSSProperties = {
  ...SEGMENT_BTN,
  color: 'var(--warn)',
  background: 'var(--card)',
  boxShadow: 'inset 0 0 0 1px var(--border)',
  borderRadius: 6,
}

const SEGMENT_GROUP: React.CSSProperties = {
  display: 'flex',
  borderRadius: 6,
  overflow: 'hidden',
  border: '1px solid var(--border)',
  background: 'var(--bg)',
}

const DIVIDER_V: React.CSSProperties = {
  width: 1,
  height: 16,
  background: 'var(--border)',
  margin: '0 6px',
  flexShrink: 0,
}

interface Props {
  viewMode: 'cards' | 'canvas' | 'twin' | 'scada'
  onViewModeChange: (mode: 'cards' | 'canvas' | 'twin' | 'scada') => void
  mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts'
  onModeChange: (mode: 'live' | 'history') => void
  turbo: boolean
  onToggleTurbo: () => void
  historyRange: string
  onHistoryRangeChange: (range: string) => void
  historyDetail: 'auto' | 'full'
  onHistoryDetailChange: (d: 'auto' | 'full') => void
  historyAutoAgg: string
  scadaToolbarSlot?: React.ReactNode
}

export function ViewToolbar({
  viewMode, onViewModeChange,
  mode, onModeChange,
  turbo, onToggleTurbo,
  historyRange, onHistoryRangeChange,
  historyDetail, onHistoryDetailChange,
  historyAutoAgg,
  scadaToolbarSlot,
}: Props) {
  const isMobile = useIsMobile()

  if (isMobile) {
    return (
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
        padding: '4px 10px',
        borderBottom: '1px solid var(--border)',
        background: 'var(--card)',
      }}>
        {/* View mode */}
        <div style={{ ...SEGMENT_GROUP, borderRadius: 6 }}>
          <button style={{ ...(viewMode === 'cards' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN), padding: '4px 8px', fontSize: 11 }} onClick={() => onViewModeChange('cards')}>Grid</button>
          <button style={{ ...(viewMode === 'canvas' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN), padding: '4px 8px', fontSize: 11 }} onClick={() => onViewModeChange('canvas')}>Canvas</button>
          <button style={{ ...(viewMode === 'twin' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN), padding: '4px 8px', fontSize: 11 }} onClick={() => onViewModeChange('twin')}>3D</button>
          <button style={{ ...(viewMode === 'scada' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN), padding: '4px 8px', fontSize: 11 }} onClick={() => onViewModeChange('scada')}>SCADA</button>
        </div>

        {/* Live/History */}
        {viewMode !== 'twin' && viewMode !== 'scada' && (
          <div style={{ ...SEGMENT_GROUP, borderRadius: 6 }}>
            <button style={{ ...(mode === 'live' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN), padding: '4px 8px', fontSize: 11 }} onClick={() => onModeChange('live')}>Live</button>
            <button style={{ ...(mode === 'history' ? SEGMENT_ACTIVE_WARN : SEGMENT_BTN), padding: '4px 8px', fontSize: 11 }} onClick={() => onModeChange('history')}>History</button>
          </div>
        )}

        {viewMode !== 'twin' && viewMode !== 'scada' && mode === 'live' && (
          <button
            onClick={onToggleTurbo}
            title={turbo ? 'Turbo ON' : 'Turbo OFF'}
            style={{
              ...SEGMENT_BTN, fontSize: 11, padding: '4px 8px',
              border: '1px solid ' + (turbo ? 'var(--warn)' : 'var(--border)'),
              borderRadius: 6,
              color: turbo ? 'var(--warn)' : 'var(--muted)',
              background: turbo ? 'rgba(234,179,8,.1)' : 'var(--card)',
            }}
          >Turbo</button>
        )}

        {viewMode === 'scada' && scadaToolbarSlot}
      </div>
    )
  }

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      gap: 8,
      padding: '0 16px',
      height: 36,
      flexShrink: 0,
      borderBottom: '1px solid var(--border)',
      background: 'var(--card)',
    }}>
      {/* View mode selector */}
      <div style={SEGMENT_GROUP}>
        <button style={viewMode === 'cards' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN} onClick={() => onViewModeChange('cards')} title="Grid view">Grid</button>
        <button style={viewMode === 'canvas' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN} onClick={() => onViewModeChange('canvas')} title="Canvas view">Canvas</button>
        <button style={viewMode === 'twin' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN} onClick={() => onViewModeChange('twin')} title="3D digital twin">3D</button>
        <button style={viewMode === 'scada' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN} onClick={() => onViewModeChange('scada')} title="2D SCADA diagram">SCADA</button>
      </div>

      {/* Contextual controls */}
      {viewMode !== 'twin' && viewMode !== 'scada' && (
        <>
          <div style={DIVIDER_V} />

          <div style={SEGMENT_GROUP}>
            <button style={mode === 'live' ? SEGMENT_ACTIVE_CYAN : SEGMENT_BTN} onClick={() => onModeChange('live')}>Live</button>
            <button style={mode === 'history' ? SEGMENT_ACTIVE_WARN : SEGMENT_BTN} onClick={() => onModeChange('history')}>History</button>
          </div>

          {mode === 'live' && (
            <button
              onClick={onToggleTurbo}
              title={turbo ? 'Turbo ON' : 'Turbo OFF'}
              style={{
                ...SEGMENT_BTN,
                fontWeight: 600,
                padding: '4px 10px',
                border: '1px solid ' + (turbo ? 'var(--warn)' : 'var(--border)'),
                borderRadius: 6,
                color: turbo ? 'var(--warn)' : 'var(--muted)',
                background: turbo ? 'rgba(234,179,8,.1)' : 'var(--card)',
              }}
            >Turbo</button>
          )}

          {mode === 'history' && (
            <HistoryRangeBar range={historyRange} onChange={onHistoryRangeChange} detail={historyDetail} onDetailChange={onHistoryDetailChange} agg={historyAutoAgg} />
          )}
        </>
      )}

      {viewMode === 'scada' && scadaToolbarSlot}
    </div>
  )
}
