import { useEffect } from 'react'
import { activityIntervalMs, createActivityThrottle } from '../session'
import { fetchOk } from '../queries/fetch'

const EVENTS = ['pointerdown', 'keydown', 'wheel', 'touchstart'] as const

/**
 * Report real user input to the server so an idle timeout only signs out
 * people who walked away. Only input events trigger a report, never timers or
 * background polling, which would keep an abandoned session alive.
 */
export function useSessionActivity(idleTimeoutSecs: number | null, enabled: boolean): void {
  useEffect(() => {
    if (!enabled || !idleTimeoutSecs) return
    const throttle = createActivityThrottle(activityIntervalMs(idleTimeoutSecs))
    const onInput = () => {
      if (!throttle.shouldSend(Date.now())) return
      fetchOk('/api/v1/auth/activity', { method: 'POST' }).catch(() => {})
    }
    for (const e of EVENTS) window.addEventListener(e, onInput, { passive: true, capture: true })
    return () => {
      for (const e of EVENTS) window.removeEventListener(e, onInput, { capture: true })
    }
  }, [idleTimeoutSecs, enabled])
}
