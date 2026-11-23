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
