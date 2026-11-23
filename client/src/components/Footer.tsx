import { useState, useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { useWsConnected, useLiveTransport } from '../store/dashboardStore'
import type { Screen } from '../types'
import { useLicense } from '../LicenseContext'
import { siteLabel, supportLabel, supportState } from '../edition/license'
import { useServerVersion } from '../queries/system'

interface Props {
  activeAlarmCount: number
  /** The active alarm count could not be fetched. */
  alarmsUnavailable?: boolean
  viewMode: 'cards' | 'canvas' | 'twin' | 'scada'
  activeScreen: string
  screens: Screen[]
}

const VIEW_LABEL_KEYS: Record<Props['viewMode'], string> = {
  cards: 'footer.viewCards',
  canvas: 'footer.viewCanvas',
  twin: 'footer.viewTwin',
  scada: 'footer.viewScada',
}

export function Footer({ activeAlarmCount, alarmsUnavailable = false, viewMode, activeScreen, screens }: Props) {
  const { t, i18n } = useTranslation()
  const wsConnected = useWsConnected()
  const transport = useLiveTransport()
  const license = useLicense()
  const site = siteLabel(license)
  const support = supportLabel(license)
  const { data: version } = useServerVersion()
  const [now, setNow] = useState(() => new Date())

  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 1000)
    return () => clearInterval(id)
  }, [])

  const screenName = activeScreen === 'all'
    ? t('footer.overview')
    : screens.find(s => s.id === activeScreen)?.name ?? t('footer.overview')

  // The clock follows the UI language (24-hour, as plants run shifts around the clock).
  const locale = i18n.resolvedLanguage ?? i18n.language ?? 'en'
  const timeStr = now.toLocaleTimeString(locale, { hour12: false })
  const dateStr = now.toLocaleDateString(locale, { month: 'short', day: 'numeric' })

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
      fontFamily: 'var(--font-mono), monospace',
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
          {wsConnected ? (transport === 'sse' ? t('footer.liveSse') : t('footer.live')) : t('footer.connecting')}
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
          {t(VIEW_LABEL_KEYS[viewMode])}
        </span>
      </div>

      {alarmsUnavailable && (
        <>
          <div style={{ width: 1, height: 14, background: 'var(--border)', flexShrink: 0 }} />
          <span role="status" style={{ color: 'var(--warn)', fontWeight: 700, flexShrink: 0 }}>{t('footer.alarmsUnavailable')}</span>
        </>
      )}

      {/* Alarm count */}
      {activeAlarmCount > 0 && (
        <>
          <div style={{ width: 1, height: 14, background: 'var(--border)', flexShrink: 0 }} />
          <div style={{ display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 }}>
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="var(--crit)" strokeWidth="2.5"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>
            <span style={{ color: 'var(--crit)', fontWeight: 700 }}>{t('footer.alarmCount', { count: activeAlarmCount })}</span>
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
        <span style={{ opacity: 0.5 }}>{version ? `Voltrus v${version}` : 'Voltrus'}</span>
      </div>
    </div>
  )
}
