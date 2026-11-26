// When the 2D view shows the phone operator layout instead of the stage.
import { useCallback, useState, useSyncExternalStore } from 'react'

/** Screens at most this wide (CSS px) get the operator layout. */
export const NARROW_MAX_PX = 640
const QUERY = `(max-width: ${NARROW_MAX_PX}px)`

function subscribe(cb: () => void) {
  if (typeof window === 'undefined' || !window.matchMedia) return () => {}
  const mq = window.matchMedia(QUERY)
  mq.addEventListener?.('change', cb)
  return () => mq.removeEventListener?.('change', cb)
}

const snapshot = () => typeof window !== 'undefined' && !!window.matchMedia && window.matchMedia(QUERY).matches

/** True on a phone-sized screen. */
export function useNarrowScreen(): boolean {
  return useSyncExternalStore(subscribe, snapshot, () => false)
}

const CHOICE_KEY = 'voltrus.operator.stage'

/** Whether the operator chose the full stage over the phone layout (kept for the browser tab's session). */
export function usePhoneLayoutChoice(): [boolean, (stage: boolean) => void] {
  const [stage, setStage] = useState(() => {
    try { return sessionStorage.getItem(CHOICE_KEY) === '1' } catch { return false }
  })
  const choose = useCallback((next: boolean) => {
    setStage(next)
    try { if (next) sessionStorage.setItem(CHOICE_KEY, '1'); else sessionStorage.removeItem(CHOICE_KEY) } catch { /* no storage */ }
  }, [])
  return [stage, choose]
}
