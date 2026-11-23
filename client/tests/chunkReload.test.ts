import { test } from 'node:test'
import assert from 'node:assert/strict'
import { isChunkLoadError, reloadOnceForNewBuild, CHUNK_RELOAD_GUARD_MS } from '../src/chunkReload.ts'

test('failed dynamic imports are recognised in each browser wording', () => {
  assert.equal(isChunkLoadError(new TypeError('Failed to fetch dynamically imported module: https://x/assets/AlarmPanel-abc.js')), true)
  assert.equal(isChunkLoadError(new TypeError('error loading dynamically imported module')), true)
  assert.equal(isChunkLoadError(new TypeError('Importing a module script failed.')), true)
  assert.equal(isChunkLoadError(new Error('Unable to preload CSS for /assets/x.css')), true)
  assert.equal(isChunkLoadError(new Error('Cannot read properties of undefined')), false)
  assert.equal(isChunkLoadError(null), false)
})

test('a stale page reloads once, and not again within the guard window', () => {
  const store = new Map<string, string>()
  const storage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => { store.set(k, v) } }
  let reloads = 0
  let now = 1_000_000
  const deps = { storage, reload: () => { reloads += 1 }, now: () => now }
  assert.equal(reloadOnceForNewBuild(deps), true)
  assert.equal(reloads, 1)
  now += CHUNK_RELOAD_GUARD_MS - 1
  assert.equal(reloadOnceForNewBuild(deps), false, 'the reload did not help: no loop')
  assert.equal(reloads, 1)
  now += 1
  assert.equal(reloadOnceForNewBuild(deps), true, 'a later redeploy reloads again')
  assert.equal(reloads, 2)
})

test('without session storage there is no reload at all', () => {
  let reloads = 0
  assert.equal(reloadOnceForNewBuild({ storage: null, reload: () => { reloads += 1 }, now: Date.now }), false)
  assert.equal(reloads, 0)
})

import { reloadIfServerUp, serverReachable, recoveryDelayMs } from '../src/chunkReload.ts'

function memoryDeps() {
  const store = new Map<string, string>()
  let reloads = 0
  return {
    deps: {
      storage: { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => { store.set(k, v) } },
      reload: () => { reloads += 1 },
      now: () => 5_000_000,
    },
    reloads: () => reloads,
  }
}

test('no reload while the server is down: the app keeps its own error screen', async () => {
  const m = memoryDeps()
  assert.equal(await reloadIfServerUp(m.deps, async () => false), 'server-down')
  assert.equal(m.reloads(), 0)
  assert.equal(await reloadIfServerUp(m.deps, async () => true), 'reloaded')
  assert.equal(m.reloads(), 1)
})

test('the server probe is a no-store HEAD; network errors and 5xx count as down', async () => {
  let seen: RequestInit | undefined
  const ok = (async (_u: string, init?: RequestInit) => { seen = init; return new Response(null, { status: 200 }) }) as typeof fetch
  assert.equal(await serverReachable(ok), true)
  assert.equal(seen?.method, 'HEAD')
  assert.equal(seen?.cache, 'no-store')
  assert.equal(await serverReachable((async () => new Response(null, { status: 502 })) as typeof fetch), false)
  assert.equal(await serverReachable((async () => { throw new TypeError('Failed to fetch') }) as typeof fetch), false)
})

test('recovery retries back off from 5 s to a minute', () => {
  assert.deepEqual([0, 1, 2, 3, 4, 5].map(recoveryDelayMs), [5_000, 10_000, 20_000, 40_000, 60_000, 60_000])
})
