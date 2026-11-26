// Browser smoke run of the phone operator layout and the PWA (docs/designer.md,
// phase 5) against a running server with the WTP showcase. Not part of
// `npm test`: it needs a server, the plant simulator and a browser.
//
//   VOLTRUS_URL=http://localhost:3998 VOLTRUS_PASSWORD=... \
//   PLAYWRIGHT_CORE=/path/to/node_modules/playwright-core \
//   node e2e/mobile.smoke.mjs [screenshot-dir]
//
// Or all in one (plant simulator, server, seed, browser):
//   scripts/smoke-mobile.sh target/release/voltrus [screenshot-dir]
//
// An engineer opens the Designer's Mobile pane on the Plant Overview, hides a
// component on phones and moves another, saves (the rows over REST carry
// config.mobile) and publishes. An operator on a phone-sized browser opens the
// screen: the generated layout without the hidden component, live values, a
// faceplate as a bottom sheet. The browser goes offline: the view says so and
// shows each value's age; reloaded offline, the installed shell shows the
// last-known screen. The site manifest carries the site name.

import assert from 'node:assert/strict'
import { mkdirSync } from 'node:fs'
import { join } from 'node:path'

const pw = await import(process.env.PLAYWRIGHT_CORE ?? 'playwright-core')
const { chromium } = pw.chromium ? pw : pw.default
const BASE = process.env.VOLTRUS_URL ?? 'http://localhost:5173'
const PASSWORD = process.env.VOLTRUS_PASSWORD
const SCREEN = process.env.VOLTRUS_SCREEN ?? 'wtp'
const shots = process.argv[2]
if (!PASSWORD) throw new Error('set VOLTRUS_PASSWORD (admin)')
if (shots) mkdirSync(shots, { recursive: true })

const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM || undefined })
const context = await browser.newContext({ viewport: { width: 1680, height: 1000 }, deviceScaleFactor: 1 })
const page = await context.newPage()
const errors = []
page.on('pageerror', e => errors.push(String(e)))
const step = msg => console.log(`· ${msg}`)

const login = await context.request.post(`${BASE}/api/v1/auth/login`, { data: { username: 'admin', password: PASSWORD } })
assert.equal(login.status(), 200, 'admin login')
// The single-user save path; the collaboration path is covered by api::tests::collab and the client tests.
assert.equal((await context.request.put(`${BASE}/api/v1/settings/design-history`, { data: { collaboration: false } })).status(), 200)
assert.equal((await context.request.put(`${BASE}/api/v1/mobile/settings`, {
  data: { site_name: 'WTP Clearwater', push_details: false, pinned_tags: [], public_urls: [] },
})).status(), 200, 'site name')
await context.addInitScript(() => localStorage.setItem('voltrus-theme', 'dark'))

const layoutRows = async (ctx = context.request, query = '') =>
  (await (await ctx.get(`${BASE}/api/v1/screens/${SCREEN}/scada-layout${query}`)).json())
const mobileOf = rows => Object.fromEntries(rows.equipment.map(r => [r.equipment_id, JSON.parse(r.config).mobile]).filter(([, m]) => m !== undefined))

let opId = null
let phone = null
try {
  step('the Designer Mobile pane lists the generated layout')
  await page.goto(`${BASE}/designer?screen=${SCREEN}`)
  await page.locator('[data-testid=designer-stage]').waitFor()
  await page.waitForFunction(() => document.querySelectorAll('[data-testid=designer-widget]').length > 0)
  await page.locator('[data-testid=designer-tab-mobile]').click()
  await page.locator('[data-testid=designer-mobile]').waitFor()
  const items = await page.locator('[data-testid^=designer-mobile-item-]').evaluateAll(els => els.map(e => ({ id: e.dataset.testid.replace('designer-mobile-item-', ''), hidden: e.dataset.hidden === 'true' })))
  const shown = items.filter(i => !i.hidden)
  assert.ok(shown.length > 5, `items on phones (${shown.length})`)
  const sections = await page.locator('[data-testid^=designer-mobile-section-]').count()
  assert.ok(sections >= 2, `sections from areas (${sections})`)

  step('hide one component, move another up, make one half width, save')
  const hide = shown[1].id
  const move = shown[3].id
  const half = shown[2].id
  await page.locator(`[data-testid=designer-mobile-hide-${hide}]`).click()
  await page.locator(`[data-testid=designer-mobile-up-${move}]`).click()
  await page.locator(`[data-testid=designer-mobile-span-${half}]`).click()
  await page.waitForTimeout(300)
  if (shots) await page.screenshot({ path: join(shots, '40-designer-mobile-pane.png') })
  await page.locator('[data-testid=designer-mobile-preview]').click()
  await page.locator('[data-testid=designer-mobile-phone] [data-testid=operator-view]').waitFor()
  await page.waitForTimeout(1500)
  if (shots) await page.screenshot({ path: join(shots, '41-designer-mobile-preview.png') })
  await page.keyboard.press('Control+s')
  await page.locator('[data-testid=designer-dirty]', { hasText: 'All changes saved' }).waitFor()
  const saved = mobileOf(await layoutRows())
  assert.equal(saved[hide]?.hide, true, `hidden: ${JSON.stringify(saved)}`)
  assert.equal(typeof saved[move]?.order, 'number', 'moved')
  assert.equal(saved[half]?.span, 1, 'half width')
  assert.equal((await context.request.post(`${BASE}/api/v1/screens/${SCREEN}/publish`, { data: { message: 'phone layout' } })).status(), 200, 'publish')

  step('an operator on a phone sees the generated layout')
  const opName = `smoke-phone-${Date.now()}`
  const opPassword = 'Smoke-Phone-2026!'
  const created = await context.request.post(`${BASE}/api/v1/users`, { data: { username: opName, password: opPassword, role: 'operator' } })
  assert.equal(created.status(), 200, `operator created: ${await created.text()}`)
  opId = (await created.json()).id
  phone = await browser.newContext({
    viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true,
    userAgent: 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Mobile Safari/537.36',
  })
  await phone.addInitScript(() => localStorage.setItem('voltrus-theme', 'dark'))
  const p = await phone.newPage()
  p.on('pageerror', e => errors.push(`phone: ${e}`))
  const phoneShot = async name => { if (shots) await p.screenshot({ path: join(shots, `${name}.png`) }) }
  assert.equal((await phone.request.post(`${BASE}/api/v1/auth/login`, { data: { username: opName, password: opPassword } })).status(), 200, 'operator login')
  const me = await (await phone.request.get(`${BASE}/api/v1/auth/me`)).json()
  if (me.must_change_password) {
    assert.equal((await phone.request.post(`${BASE}/api/v1/auth/change-password`, { data: { old_password: opPassword, new_password: `${opPassword}x` } })).status(), 200)
  }

  const manifest = await (await phone.request.get(`${BASE}/api/v1/site/manifest.webmanifest`)).json()
  assert.equal(manifest.name, 'WTP Clearwater')
  assert.equal(manifest.display, 'standalone')

  const openScreen = async () => {
    await p.goto(`${BASE}/?screen=${SCREEN}&view=scada`)
    await p.locator('[data-testid=operator-view]').waitFor({ timeout: 30_000 })
    await p.locator('[data-testid=operator-live]', { hasText: 'Live' }).waitFor({ timeout: 30_000 })
  }
  await openScreen()
  // The service worker takes over the page; the second load runs through it, so the shell is cached.
  await p.waitForFunction(() => navigator.serviceWorker?.controller != null || navigator.serviceWorker?.ready, null, { timeout: 15_000 })
  await openScreen()
  await p.waitForFunction(() => navigator.serviceWorker?.controller != null, null, { timeout: 15_000 })
  await p.waitForTimeout(2500)
  assert.equal(await p.locator(`[data-testid=operator-item-${hide}]`).count(), 0, 'the hidden component is not on the phone')
  assert.ok(await p.locator('[data-testid^=operator-item-]').count() > 4, 'rows on the phone')
  assert.equal(await p.getAttribute('link[rel=manifest]', 'href'), '/api/v1/site/manifest.webmanifest')
  await phoneShot('42-phone-operator-view')
  // The trends: a section of cards.
  await p.evaluate(() => {
    const sections = [...document.querySelectorAll('[data-testid=operator-view] section')]
    const trends = sections.find(s => /trend/i.test(s.querySelector('h2')?.textContent ?? '')) ?? sections[sections.length - 1]
    trends?.scrollIntoView({ block: 'start' })
  })
  await p.waitForTimeout(400)
  await phoneShot('43-phone-operator-cards')
  await p.evaluate(() => document.querySelector('[data-testid=operator-view]')?.scrollTo(0, 0))

  step('tapping equipment opens its faceplate as a sheet')
  const firstEquipment = p.locator('[data-testid=operator-view] button[data-testid^=operator-item-]').first()
  await firstEquipment.click()
  await p.locator('.vt-operator-sheet').waitFor()
  await p.waitForTimeout(600)
  await phoneShot('44-phone-faceplate-sheet')
  await p.locator('.vt-operator-backdrop').click({ position: { x: 20, y: 20 } })
  await p.locator('.vt-operator-sheet').waitFor({ state: 'detached' })

  step('offline: not live, values with their age')
  // Wait for a snapshot to be kept (every 15 s while live).
  await p.waitForFunction(() => !!localStorage.getItem('voltrus.operator.lastKnown.v1'), null, { timeout: 30_000 })
  await phone.setOffline(true)
  await p.locator('[data-testid=operator-live]', { hasText: 'Offline' }).waitFor({ timeout: 15_000 })
  await p.locator('[data-testid=operator-offline]').waitFor()
  const liveValues = await p.locator('[data-testid^=operator-value-][data-live=true]').count()
  assert.equal(liveValues, 0, 'nothing is shown as live while offline')
  await p.waitForTimeout(6000)
  await phoneShot('45-phone-offline-banner')

  step('reloaded offline, the installed shell shows the last-known screen')
  await p.reload().catch(() => {})
  await p.locator('[data-testid=offline-operator]').waitFor({ timeout: 30_000 })
  assert.equal(await p.locator('[data-testid^=operator-value-][data-live=true]').count(), 0)
  assert.ok(await p.locator('[data-testid^=operator-item-]').count() > 4, 'last-known rows')
  await p.waitForTimeout(800)
  await phoneShot('46-phone-offline-start')
  await phone.setOffline(false)
} catch (e) {
  if (shots) await page.screenshot({ path: join(shots, 'failure-designer.png') }).catch(() => {})
  console.error('page errors:', errors)
  throw e
} finally {
  if (phone) await phone.close()
  if (opId != null) await context.request.delete(`${BASE}/api/v1/users/${opId}`)
}

// Offline navigation errors are expected; anything else is not.
const unexpected = errors.filter(e => !/Failed to fetch|NetworkError|net::ERR_INTERNET_DISCONNECTED|Load failed/.test(e))
assert.deepEqual(unexpected, [], 'no page errors')
await browser.close()
console.log('mobile smoke: ok')
