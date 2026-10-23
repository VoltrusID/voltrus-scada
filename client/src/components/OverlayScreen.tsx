import { useEffect, useRef, type ReactNode } from 'react'
import { btnStyle } from '../styles'

/**
 * Standard full-screen overlay used by all configuration panels.
 * Matches the DataManager layout: fixed overlay, back button + title + optional tabs.
 *
 * Usage:
 *   <OverlayScreen title="Audit Log" onClose={() => setShow(false)}>
 *     <AuditLogPanel />
 *   </OverlayScreen>
 */
interface Props {
  title: string
  onClose: () => void
  children: ReactNode
  /** Optional tab bar rendered next to the title */
  tabs?: ReactNode
  /** Accessible label for the dialog */
  ariaLabel?: string
}

export function OverlayScreen({ title, onClose, children, tabs, ariaLabel }: Props) {
  const ref = useRef<HTMLDivElement>(null)

  // Escape key
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  // Focus trap
  useEffect(() => {
    const modal = ref.current
    if (!modal) return
    const handleTab = (e: KeyboardEvent) => {
      if (e.key !== 'Tab') return
      const sel = 'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
      const els = modal.querySelectorAll<HTMLElement>(sel)
      if (els.length === 0) return
      const first = els[0], last = els[els.length - 1]
      if (e.shiftKey) { if (document.activeElement === first) { e.preventDefault(); last.focus() } }
      else { if (document.activeElement === last) { e.preventDefault(); first.focus() } }
    }
    document.addEventListener('keydown', handleTab)
    return () => document.removeEventListener('keydown', handleTab)
  }, [])

  return (
    <div
      ref={ref}
      role="dialog"
      aria-label={ariaLabel ?? title}
      aria-modal="true"
      style={{
        position: 'fixed',
        inset: 0,
        background: 'var(--bg)',
        zIndex: 50,
        display: 'flex',
        flexDirection: 'column',
        padding: 24,
      }}
    >
      <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
        {/* Header row: back + title + tabs */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 0, flexShrink: 0 }}>
          <button onClick={onClose} style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6, padding: '5px 10px' }}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>
            Back
          </button>
          <h1 style={{ fontSize: 18, fontWeight: 700 }}>{title}</h1>
          {tabs}
        </div>
        {/* Content */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0, marginTop: 16 }}>
          {children}
        </div>
      </div>
    </div>
  )
}
