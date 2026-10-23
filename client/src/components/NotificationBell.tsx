import { useState, useRef, useEffect, useCallback } from 'react'
import { useUserNotifications, useMarkNotificationRead, useDismissNotification, type UserNotification } from '../queries'
import { btnStyle } from '../styles'

export function NotificationBell({ authed }: { authed: boolean }) {
  const [open, setOpen] = useState(false)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const { data: notifications = [] } = useUserNotifications(authed, true)
  const markRead = useMarkNotificationRead()
  const dismiss = useDismissNotification()
  const unreadCount = notifications.length

  const closeDropdown = useCallback(() => setOpen(false), [])

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        closeDropdown()
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open, closeDropdown])

  const formatTime = (ts: number) => {
    const diff = Math.floor(Date.now() / 1000) - ts
    if (diff < 60) return 'just now'
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
    return `${Math.floor(diff / 86400)}d ago`
  }

  return (
    <div style={{ position: 'relative' }} ref={dropdownRef}>
      <button
        onClick={() => setOpen((o) => !o)}
        aria-label={`Notifications${unreadCount > 0 ? ` (${unreadCount} unread)` : ''}`}
        style={{
          ...btnStyle,
          position: 'relative',
          borderColor: unreadCount > 0 ? 'var(--warn)' : 'var(--border)',
          color: unreadCount > 0 ? 'var(--warn)' : 'var(--fg)',
        }}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/>
          <path d="M13.73 21a2 2 0 0 1-3.46 0"/>
        </svg>
        {unreadCount > 0 && (
          <span style={{
            position: 'absolute',
            top: -4,
            right: -4,
            background: 'var(--warn)',
            color: '#fff',
            borderRadius: '50%',
            width: 16,
            height: 16,
            fontSize: 10,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontWeight: 700,
          }}>
            {unreadCount > 99 ? '99+' : unreadCount}
          </span>
        )}
      </button>

      {open && (
        <div style={{
          position: 'absolute',
          top: 'calc(100% + 8px)',
          right: 0,
          width: 320,
          maxHeight: 400,
          background: 'var(--card)',
          border: '1px solid var(--border)',
          borderRadius: 8,
          boxShadow: '0 8px 24px rgba(0,0,0,0.2)',
          zIndex: 50,
          overflow: 'hidden',
          display: 'flex',
          flexDirection: 'column',
        }}>
          <div style={{
            padding: '12px 16px',
            borderBottom: '1px solid var(--border)',
            fontWeight: 600,
            fontSize: 14,
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <span>Notifications</span>
            {unreadCount > 0 && <span style={{ fontSize: 12, color: 'var(--muted)', fontWeight: 400 }}>{unreadCount} unread</span>}
          </div>

          <div style={{ overflowY: 'auto', flex: 1 }}>
            {notifications.length === 0 ? (
              <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)', fontSize: 13 }}>
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
                          fontSize: 11,
                          padding: '2px 8px',
                          borderRadius: 4,
                          border: '1px solid var(--border)',
                          background: 'var(--bg)',
                          color: 'var(--fg)',
                          cursor: 'pointer',
                        }}
                      >
                        Mark read
                      </button>
                      <button
                        onClick={() => dismiss.mutate(n.id)}
                        style={{
                          fontSize: 11,
                          padding: '2px 8px',
                          borderRadius: 4,
                          border: '1px solid var(--border)',
                          background: 'var(--bg)',
                          color: 'var(--crit)',
                          cursor: 'pointer',
                        }}
                      >
                        Dismiss
                      </button>
                    </div>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  )
}
