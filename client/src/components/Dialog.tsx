import { useEffect, useId, useRef, useState, type ReactNode, type RefObject } from 'react'

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(el => !el.hasAttribute('inert'))
}

/**
 * Modal focus handling: focus moves into the dialog when it opens (an
 * autoFocus field keeps it), Tab and Shift+Tab stay inside, Escape calls
 * `onEscape`, and focus returns to where it was when the dialog closes.
 * `active` false suspends the trap and Escape (e.g. while recording a shortcut).
 */
export function useModalFocus(ref: RefObject<HTMLElement | null>, onEscape?: () => void, active = true) {
  const onEscapeRef = useRef(onEscape)
  onEscapeRef.current = onEscape
  const activeRef = useRef(active)
  activeRef.current = active
  // Read while rendering, before an autoFocus field inside takes focus.
  const [opener] = useState(() => (typeof document !== 'undefined' && document.activeElement instanceof HTMLElement ? document.activeElement : null))

  useEffect(() => {
    const root = ref.current
    if (!root) return
    if (!root.contains(document.activeElement)) (focusables(root)[0] ?? root).focus()

    const onKeyDown = (e: KeyboardEvent) => {
      if (!activeRef.current) return
      if (e.key === 'Escape' && onEscapeRef.current) {
        e.stopPropagation()
        e.preventDefault()
        onEscapeRef.current()
        return
      }
      if (e.key !== 'Tab') return
      const items = focusables(root)
      if (items.length === 0) { e.preventDefault(); root.focus(); return }
      const first = items[0]
      const last = items[items.length - 1]
      if (e.shiftKey && (document.activeElement === first || !root.contains(document.activeElement))) {
        e.preventDefault(); last.focus()
      } else if (!e.shiftKey && (document.activeElement === last || !root.contains(document.activeElement))) {
        e.preventDefault(); first.focus()
      }
    }
    root.addEventListener('keydown', onKeyDown)
    return () => {
      root.removeEventListener('keydown', onKeyDown)
      if (opener && opener.isConnected) opener.focus()
    }
  }, [ref, opener])
}

interface DialogProps {
  title: ReactNode
  onClose: () => void
  children: ReactNode
  width?: number
  zIndex?: number
  /** Suspend Escape and the focus trap, e.g. while a field captures keys. */
  trapActive?: boolean
}

/** A modal dialog: backdrop click and Escape close it, focus is trapped and restored. */
export function Dialog({ title, onClose, children, width = 380, zIndex = 60, trapActive = true }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null)
  const titleId = useId()
  useModalFocus(ref, onClose, trapActive)
  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex }} onClick={onClose}>
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width, maxWidth: 'calc(100vw - 32px)', padding: 20, outline: 'none' }}
        onClick={e => e.stopPropagation()}
      >
        <div id={titleId} style={{ fontSize: 15, fontWeight: 600, marginBottom: 12 }}>{title}</div>
        {children}
      </div>
    </div>
  )
}
