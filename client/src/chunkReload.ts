// After a redeploy the old index.html asks for chunk files that no longer
// exist. One reload fetches the new build; a guard stops a reload loop when
// the chunk is missing for another reason (server down, broken build).

const KEY = 'voltrus:chunk-reload-at'
/** A second reload within this window is refused: the first did not help. */
export const CHUNK_RELOAD_GUARD_MS = 60_000

/** Whether an error is a failed dynamic import or preload of a build chunk. */
export function isChunkLoadError(err: unknown): boolean {
  if (!err || typeof err !== 'object') return false
  const { name, message } = err as { name?: unknown; message?: unknown }
  if (name === 'ChunkLoadError') return true
  if (typeof message !== 'string') return false
  return /Failed to fetch dynamically imported module|error loading dynamically imported module|Importing a module script failed|Unable to preload CSS|Failed to load module script/i
    .test(message)
}

interface ReloadDeps {
  storage: Pick<Storage, 'getItem' | 'setItem'> | null
  reload: () => void
  now: () => number
}

function defaultDeps(): ReloadDeps {
  let storage: ReloadDeps['storage'] = null
  try { storage = window.sessionStorage } catch { /* storage disabled */ }
  return { storage, reload: () => window.location.reload(), now: Date.now }
}

/**
 * Reload the page to pick up a new build, unless that was already tried in the
 * last minute. Returns whether a reload was started.
 */
export function reloadOnceForNewBuild(deps: ReloadDeps = defaultDeps()): boolean {
  const { storage, reload, now } = deps
  // Without storage the guard cannot work; never risk a reload loop.
  if (!storage) return false
  const last = Number(storage.getItem(KEY) ?? 0)
  if (Number.isFinite(last) && now() - last < CHUNK_RELOAD_GUARD_MS) return false
  storage.setItem(KEY, String(now()))
  reload()
  return true
}

/**
 * Whether the server answers at all. A reload while it is down would replace
 * the app with the browser's own error page, which never retries.
 */
export async function serverReachable(fetchImpl: typeof fetch = fetch): Promise<boolean> {
  try {
    const r = await fetchImpl('/', { method: 'HEAD', cache: 'no-store', credentials: 'same-origin' })
    return r.status < 500
  } catch {
    return false
  }
}

export type RecoveryOutcome = 'reloaded' | 'server-down' | 'guarded'

let inFlight: Promise<RecoveryOutcome> | null = null

/** Reload for a new build only once the server is reachable; one probe at a time. */
export function reloadIfServerUp(
  deps: ReloadDeps = defaultDeps(),
  probe: () => Promise<boolean> = () => serverReachable(),
): Promise<RecoveryOutcome> {
  if (inFlight) return inFlight
  inFlight = (async () => {
    try {
      if (!(await probe())) return 'server-down'
      return reloadOnceForNewBuild(deps) ? 'reloaded' : 'guarded'
    } finally {
      inFlight = null
    }
  })()
  return inFlight
}

/** Delay before the next recovery attempt: 5 s doubling to a minute. */
export function recoveryDelayMs(attempt: number): number {
  return Math.min(5_000 * 2 ** attempt, CHUNK_RELOAD_GUARD_MS)
}

/**
 * Vite reports a failed chunk preload with this event. The error still reaches
 * the ErrorBoundary, which keeps an in-app screen and retries; here the page is
 * reloaded at once if the server is up (it was a redeploy).
 */
export function installChunkReload(): void {
  window.addEventListener('vite:preloadError', () => { void reloadIfServerUp() })
}
