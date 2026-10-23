import { useState, useRef, useEffect, useCallback } from 'react'
import { useUserNotifications, useMarkNotificationRead, useDismissNotification, type UserNotification } from '../queries'

const ICON_BTN: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  width: 36,
  height: 36,
  borderRadius: 6,
  border: '1px solid var(--border)',
  background: 'var(--card)',
  color: 'var(--fg)',
  cursor: 'pointer',
  position: 'relative' as const,
  transition: 'border-color 0.15s, color 0.15s',
}

const TAB_STYLE = (active: boolean): React.CSSProperties => ({
  flex: 1,
  padding: '10px 0',
  fontSize: 12,
  fontWeight: 600,
  fontFamily: 'var(--font-ui)',
  textAlign: 'center' as const,
  cursor: 'pointer',
  border: 'none',
  borderBottom: active ? '2px solid var(--cyan)' : '2px solid transparent',
  color: active ? 'var(--cyan)' : 'var(--muted)',
  background: 'none',
  transition: 'color 0.15s',
})

interface Props {
  authed: boolean
  activeAlarmCount: number
  onOpenAlarmPanel: () => void
}

export function UnifiedBell({ authed, activeAlarmCount, onOpenAlarmPanel }: Props) {
  const [open, setOpen] = useState(false)
  const [tab, setTab] = useState<'notifications' | 'alarms'>('alarms')
  const dropdownRef = useRef<HTMLDivElement>(null)

  const { data: notifications = [] } = useUserNotifications(authed, true)
  const markRead = useMarkNotificationRead()
  const dismiss = useDismissNotification()
  const unreadCount = notifications.length

  // Pre-select alarms tab when there are active alarms, otherwise notifications
  useEffect(() => {
    if (open && activeAlarmCount > 0) setTab('alarms')
    else if (open && unreadCount > 0) setTab('notifications')
  }, [open, activeAlarmCount, unreadCount])

  const closeDropdown = useCallback(() => setOpen(false), [])

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) closeDropdown()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open, closeDropdown])

  useEffect(() => {
    if (!open) return
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') closeDropdown() }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [open, closeDropdown])

  const totalCount = activeAlarmCount + unreadCount
  const hasAlarm = activeAlarmCount > 0
  const hasNotif = unreadCount > 0

  const formatTime = (ts: number) => {
    const diff = Math.floor(Date.now() / 1000) - ts
    if (diff < 60) return 'just now'
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
    if (diff < 86400) return `${Math.floor(diff / 86400)}h ago`
    return `${Math.floor(diff / 86400)}d ago`
  }

  return (
    <div style={{ position: 'relative' }} ref={dropdownRef}>
      <button
        onClick={() => setOpen(o => !o)}
        aria-label={`Notifications & alarms${totalCount > 0 ? ` (${totalCount})` : ''}`}
        style={{
          ...ICON_BTN,
          borderColor: hasAlarm ? 'var(--crit)' : hasNotif ? 'var(--warn)' : 'var(--border)',
          color: hasAlarm ? 'var(--crit)' : hasNotif ? 'var(--warn)' : 'var(--fg)',
        }}
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/>
          <path d="M13.73 21a2 2 0 0 1-3.46 0"/>
        </svg>
        {totalCount > 0 && (
          <span style={{
            position: 'absolute', top: -4, right: -4,
            background: hasAlarm ? 'var(--crit)' : 'var(--warn)',
            color: '#fff', borderRadius: '50%', minWidth: 16, height: 16,
            fontSize: 10, display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontWeight: 700, padding: '0 3px',
          }}>
            {totalCount > 99 ? '99+' : totalCount}
          </span>
        )}
      </button>

      {open && (
        <div style={{
          position: 'absolute',
          top: 'calc(100% + 6px)',
          right: 0,
          width: 340,
          maxHeight: 440,
          background: 'var(--card)',
          border: '1px solid var(--border)',
          borderRadius: 8,
          boxShadow: '0 8px 32px rgba(0,0,0,0.35)',
          zIndex: 200,
          overflow: 'hidden',
          display: 'flex',
          flexDirection: 'column',
        }}>
          {/* Tab bar */}
          <div style={{ display: 'flex', borderBottom: '1px solid var(--border)' }}>
            <button style={TAB_STYLE(tab === 'alarms')} onClick={() => setTab('alarms')}>
              Alarms{activeAlarmCount > 0 && <span style={{ marginLeft: 6, fontSize: 11, color: 'var(--crit)', fontWeight: 700 }}>{activeAlarmCount}</span>}
            </button>
            <button style={TAB_STYLE(tab === 'notifications')} onClick={() => setTab('notifications')}>
              Notifications{unreadCount > 0 && <span style={{ marginLeft: 6, fontSize: 11, color: 'var(--warn)', fontWeight: 700 }}>{unreadCount}</span>}
            </button>
          </div>

          {/* Alarms tab */}
          {tab === 'alarms' && (
            <div style={{ overflowY: 'auto', flex: 1 }}>
              {activeAlarmCount === 0 ? (
                <div style={{ padding: '32px 24px', textAlign: 'center', color: 'var(--muted)', fontSize: 13 }}>
                  <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="var(--ok)" strokeWidth="1.5" style={{ margin: '0 auto 12px', display: 'block' }}>
                    <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
                    <polyline points="22 4 12 14.01 9 11.01"/>
                  </svg>
                  No active alarms
                </div>
              ) : (
                <>
                  <div style={{ padding: '16px 16px 12px', display: 'flex', flexDirection: 'column', gap: 8 }}>
                    <div style={{ fontSize: 13, color: 'var(--fg)', fontWeight: 600 }}>
                      {activeAlarmCount} active alarm{activeAlarmCount !== 1 ? 's' : ''}
                    </div>
                    <div style={{ fontSize: 12, color: 'var(--muted)' }}>
                      Open the alarm panel to view details, acknowledge, or silence alarms.
                    </div>
                  </div>
                  <div style={{ padding: '0 16px 16px' }}>
                    <button
                      onClick={() => { closeDropdown(); onOpenAlarmPanel() }}
                      style={{
                        width: '100%',
                        padding: '9px 0',
                        borderRadius: 6,
                        border: '1px solid var(--crit)',
                        background: 'rgba(239,68,68,.08)',
                        color: 'var(--crit)',
                        fontSize: 13,
                        fontWeight: 600,
                        cursor: 'pointer',
                        fontFamily: 'var(--font-ui)',
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        gap: 6,
                      }}
                    >
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>
                      Open Alarm Panel
                    </button>
                  </div>
                </>
              )}
            </div>
          )}

          {/* Notifications tab */}
          {tab === 'notifications' && (
            <div style={{ overflowY: 'auto', flex: 1 }}>
              {notifications.length === 0 ? (
                <div style={{ padding: '32px 24px', textAlign: 'center', color: 'var(--muted)', fontSize: 13 }}>
                  No unread notifications
                </div>
              ) : (
                notifications.map((n: UserNotification) => (
                  <div key={n.id} style={{
                    padding: '10px 14px',
                    borderBottom: '1px solid var(--border)',
                    fontSize: 13,
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 6,
                  }}>
                    <div style={{ lineHeight: 1.4 }}>{n.message}</div>
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                      <span style={{ fontSize: 11, color: 'var(--muted)' }}>{formatTime(n.created_at)}</span>
                      <div style={{ display: 'flex', gap: 6 }}>
                        <button
                          onClick={() => markRead.mutate(n.id)}
                          style={{
                            fontSize: 11, padding: '2px 8px', borderRadius: 4,
                            border: '1px solid var(--border)', background: 'var(--bg)',
                            color: 'var(--fg)', cursor: 'pointer', fontFamily: 'var(--font-ui)',
                          }}
                        >Mark read</button>
                        <button
                          onClick={() => dismiss.mutate(n.id)}
                          style={{
                            fontSize: 11, padding: '2px 8px', borderRadius: 4,
                            border: '1px solid var(--border)', background: 'var(--bg)',
                            color: 'var(--crit)', cursor: 'pointer', fontFamily: 'var(--font-ui)',
                          }}
                        >Dismiss</button>
                      </div>
                    </div>
                  </div>
                ))
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
