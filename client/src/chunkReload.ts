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

/** Vite reports a failed chunk preload with this event; reloading replaces the stale page. */
export function installChunkReload(): void {
  window.addEventListener('vite:preloadError', event => {
    if (reloadOnceForNewBuild()) event.preventDefault()
  })
}
