import { useEffect, useState, useCallback } from 'react'

const STORAGE_KEY = 'voltrus:kiosk_mode'

export function useKioskMode() {
  const [enabled, setEnabled] = useState(() => {
    return localStorage.getItem(STORAGE_KEY) === 'true'
  })

  const toggle = useCallback(() => {
    setEnabled(prev => {
      const next = !prev
      localStorage.setItem(STORAGE_KEY, String(next))
      return next
    })
  }, [])

  useEffect(() => {
    if (!enabled) return

    const blockedKeys = new Set(['Alt', 'Tab', 'Meta', 'Escape', 'F11', 'F12'])
    const blockedCombos = [
      'Control+W', 'Control+T', 'Control+N', 'Control+Shift+N',
      'Control+L', 'Control+Shift+I', 'Control+Shift+J', 'Control+Shift+C',
      'Control+R', 'Control+Shift+R', 'Control+P', 'Control+S',
      'Meta+W', 'Meta+T', 'Meta+N', 'Meta+L', 'Meta+R', 'Meta+Comma',
    ]

    const onKeyDown = (e: KeyboardEvent) => {
      if (blockedKeys.has(e.key)) {
        e.preventDefault()
        e.stopPropagation()
        return
      }
      const parts: string[] = []
      if (e.ctrlKey || e.metaKey) parts.push(e.ctrlKey ? 'Control' : 'Meta')
      if (e.shiftKey) parts.push('Shift')
      parts.push(e.key)
      const combo = parts.join('+')
      if (blockedCombos.includes(combo)) {
        e.preventDefault()
        e.stopPropagation()
      }
    }

    const onContextMenu = (e: Event) => {
      e.preventDefault()
    }

    const onDragStart = (e: Event) => {
      e.preventDefault()
    }

    document.addEventListener('keydown', onKeyDown, true)
    document.addEventListener('contextmenu', onContextMenu)
    document.addEventListener('dragstart', onDragStart)

    return () => {
      document.removeEventListener('keydown', onKeyDown, true)
      document.removeEventListener('contextmenu', onContextMenu)
      document.removeEventListener('dragstart', onDragStart)
    }
  }, [enabled])

  return { enabled, toggle }
}
