import { useState, useEffect, useCallback } from 'react'

export interface ToastMessage {
  id: number
  text: string
  type: 'error' | 'success' | 'info'
}

let nextId = 0
type Listener = (t: ToastMessage) => void
const listeners = new Set<Listener>()

function emit(text: string, type: ToastMessage['type'] = 'error') {
  const toast: ToastMessage = { id: nextId++, text, type }
  listeners.forEach(fn => fn(toast))
}

export const toast = {
  error: (text: string) => emit(text, 'error'),
  success: (text: string) => emit(text, 'success'),
  info: (text: string) => emit(text, 'info'),
}

export function useToasts() {
  const [toasts, setToasts] = useState<ToastMessage[]>([])

  useEffect(() => {
    const handler: Listener = (t) => {
      setToasts(prev => [...prev, t])
      setTimeout(() => setToasts(prev => prev.filter(x => x.id !== t.id)), 5000)
    }
    listeners.add(handler)
    return () => { listeners.delete(handler) }
  }, [])

  return toasts
}

export function ToastContainer() {
  const toasts = useToasts()

  if (toasts.length === 0) return null

  return (
    <div style={{
      position: 'fixed',
      bottom: 20,
      right: 20,
      zIndex: 9999,
      display: 'flex',
      flexDirection: 'column',
      gap: 8,
      maxWidth: 420,
      pointerEvents: 'none',
    }}>
      {toasts.map(t => (
        <div
          key={t.id}
          style={{
            padding: '10px 16px',
            borderRadius: 8,
            fontSize: 13,
            fontWeight: 500,
            fontFamily: 'var(--font-ui)',
            boxShadow: '0 4px 16px rgba(0,0,0,0.3)',
            border: `1px solid ${t.type === 'error' ? 'var(--crit)' : t.type === 'success' ? 'var(--ok)' : 'var(--cyan)'}`,
            background: t.type === 'error' ? 'rgba(239,68,68,.15)' : t.type === 'success' ? 'rgba(34,197,94,.15)' : 'rgba(34,211,238,.1)',
            color: t.type === 'error' ? 'var(--crit)' : t.type === 'success' ? 'var(--ok)' : 'var(--cyan)',
            animation: 'toastIn 200ms ease-out',
            pointerEvents: 'auto',
          }}
        >
          {t.text}
        </div>
      ))}
    </div>
  )
}
