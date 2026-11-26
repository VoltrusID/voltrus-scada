import { useSyncExternalStore } from 'react'

function subscribe(cb: () => void) {
  window.addEventListener('online', cb)
  window.addEventListener('offline', cb)
  return () => { window.removeEventListener('online', cb); window.removeEventListener('offline', cb) }
}

/**
 * `navigator.onLine`, following the browser's online and offline events. A
 * socket may take a while to notice a lost network; this flips at once, so
 * nothing keeps saying "live" meanwhile.
 */
export function useOnline(): boolean {
  return useSyncExternalStore(subscribe, () => navigator.onLine, () => true)
}
