import { useState, useEffect } from 'react'
import { useWsConnected } from '../store/dashboardStore'
import type { Screen } from '../types'
import { useLicense } from '../LicenseContext'
import { siteLabel, supportLabel, supportState } from '../edition/license'

interface Props {
  activeAlarmCount: number
  viewMode: 'cards' | 'canvas' | 'twin' | 'scada'
  activeScreen: string
  screens: Screen[]
}

const VIEW_LABELS: Record<string, string> = {
  cards: 'Grid',
  canvas: 'Canvas',
  twin: '3D',
  scada: 'SCADA',
}

export function Footer({ activeAlarmCount, viewMode, activeScreen, screens }: Props) {
  const wsConnected = useWsConnected()
  const license = useLicense()
  const site = siteLabel(license)
  const support = supportLabel(license)
  const [now, setNow] = useState(() => new Date())

  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 1000)
    return () => clearInterval(id)
  }, [])

  const screenName = activeScreen === 'all'
    ? 'Overview'
    : screens.find(s => s.id === activeScreen)?.name ?? 'Overview'

  const timeStr = now.toLocaleTimeString('en-US', { hour12: false })
  const dateStr = now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })

  return (
    <div style={{
      position: 'fixed',
      bottom: 0,
      left: 0,
      right: 0,
      height: 30,
      background: 'var(--card)',
      borderTop: '1px solid var(--border)',
      display: 'flex',
      alignItems: 'center',
      padding: '0 14px',
      gap: 14,
      fontFamily: "'var(--font-mono)', monospace",
      fontSize: 10.5,
      color: 'var(--muted)',
      zIndex: 5,
      userSelect: 'none',
    }}>
      {/* Connection status */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 5, flexShrink: 0 }}>
        <span style={{
          width: 6, height: 6, borderRadius: '50%',
          background: wsConnected ? 'var(--ok)' : 'var(--warn)',
          boxShadow: wsConnected ? '0 0 6px rgba(34,197,94,.4)' : '0 0 6px rgba(245,158,11,.4)',
        }} />
        <span style={{ color: wsConnected ? 'var(--ok)' : 'var(--warn)', fontWeight: 600 }}>
          {wsConnected ? 'LIVE' : 'CONNECTING'}
        </span>
      </div>

      <div style={{ width: 1, height: 14, background: 'var(--border)', flexShrink: 0 }} />

      {/* Screen + View */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexShrink: 0 }}>
        <span style={{ color: 'var(--fg)', fontWeight: 600 }}>{screenName}</span>
        <span style={{
          padding: '1px 5px',
          borderRadius: 3,
          background: 'var(--hover-overlay)',
          color: 'var(--muted)',
          fontWeight: 600,
          letterSpacing: '.03em',
        }}>
          {VIEW_LABELS[viewMode]}
        </span>
      </div>

      {/* Alarm count */}
      {activeAlarmCount > 0 && (
        <>
          <div style={{ width: 1, height: 14, background: 'var(--border)', flexShrink: 0 }} />
          <div style={{ display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 }}>
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="var(--crit)" strokeWidth="2.5"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>
            <span style={{ color: 'var(--crit)', fontWeight: 700 }}>{activeAlarmCount} Alarm{activeAlarmCount !== 1 ? 's' : ''}</span>
          </div>
        </>
      )}

      {(site || support) && (
        <>
          <div style={{ width: 1, height: 14, background: 'var(--border)', flexShrink: 0 }} />
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 1, minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>
            {site && <span style={{ color: 'var(--fg)', fontWeight: 600 }}>{site}</span>}
            {support && (
              <span style={{ color: supportState(license) === 'lapsed' ? 'var(--warn)' : 'var(--muted)' }}>{support}</span>
            )}
          </div>
        </>
      )}

      <div style={{ flex: 1 }} />

      {/* Clock + version */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexShrink: 0 }}>
        <span style={{ color: 'var(--text-dim)' }}>{dateStr} {timeStr}</span>
        <div style={{ width: 1, height: 14, background: 'var(--border)' }} />
        <span style={{ opacity: 0.5 }}>Voltrus v0.47.0</span>
      </div>
    </div>
  )
}
