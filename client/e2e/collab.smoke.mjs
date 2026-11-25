// Browser smoke run of live collaboration in the Designer (phase 4,
// docs/collaboration.md): two people, two browsers, one screen. Not part of
// `npm test`: it needs a server with the WTP showcase and a browser.
//
//   VOLTRUS_URL=http://localhost:3998 VOLTRUS_PASSWORD=... \
//   PLAYWRIGHT_CORE=/path/to/node_modules/playwright-core \
//   node e2e/collab.smoke.mjs [screenshot-dir]
//
// Or all in one, as CI runs it: scripts/smoke-collab.sh target/release/voltrus [screenshot-dir]
//
// The admin creates a second administrator (bea); both open a scratch screen
// in the Designer and see each other (avatars, cursors). The admin moves a
// widget and bea sees it move; bea locks another and the admin sees it;
// both move different widgets at once and end up with the same screen; a
// comment by the admin shows up for bea; the rows over REST hold all of it
// after a flush. With a screenshot directory, the PR pictures are taken on
// the Plant Overview: two cursors, selections, the presence avatars, a
// comment thread, and frames of a cursor moving (for a GIF).

import assert from 'node:assert/strict'
import { mkdirSync } from 'node:fs'
import { join } from 'node:path'

const pw = await import(process.env.PLAYWRIGHT_CORE ?? 'playwright-core')
const { chromium } = pw.chromium ? pw : pw.default
const BASE = process.env.VOLTRUS_URL ?? 'http://localhost:3998'
const PASSWORD = process.env.VOLTRUS_PASSWORD
const SHOWCASE = process.env.VOLTRUS_SCREEN ?? 'wtp'
const shots = process.argv[2]
if (!PASSWORD) throw new Error('set VOLTRUS_PASSWORD (admin)')
if (shots) mkdirSync(shots, { recursive: true })

const SECOND = 'bea'
const SECOND_PASSWORD = 'Collab-smoke-2026!pw'
const step = msg => console.log(`· ${msg}`)

const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM || undefined })
const errors = []

async function person(username, password) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 })
  const login = await context.request.post(`${BASE}/api/v1/auth/login`, { data: { username, password } })
  assert.equal(login.status(), 200, `${username} login`)
  await context.addInitScript(() => localStorage.setItem('voltrus-theme', 'dark'))
  const page = await context.newPage()
  page.on('pageerror', e => errors.push(`${username}: ${e}`))
  return { username, context, page, request: context.request }
}

const admin = await person('admin', PASSWORD)
const created = await admin.request.post(`${BASE}/api/v1/users`, { data: { username: SECOND, password: SECOND_PASSWORD, role: 'admin' } })
assert.ok([200, 201, 409].includes(created.status()), `second user: ${created.status()} ${await created.text()}`)
const bea = await person(SECOND, SECOND_PASSWORD)

const shot = async (p, name) => { if (shots) await p.page.screenshot({ path: join(shots, `${name}.png`) }) }

const status = await (await admin.request.get(`${BASE}/api/v1/screens/${SHOWCASE}/design-status`)).json()
assert.ok(status.collaboration?.enabled, 'the server offers live collaboration')

/** Client point of a stage coordinate. */
const stagePoint = (p, x, y) => p.page.evaluate(([x, y]) => {
  const r = document.querySelector('[data-testid=designer-stage]').getBoundingClientRect()
  const k = r.width / 1920
  return { x: r.left + x * k, y: r.top + y * k }
}, [x, y])
const leftOf = (p, id) => p.page.evaluate(id => document.querySelector(`[data-testid=designer-stage] [data-wid="${id}"]`)?.style.left, id)
const open = async (p, screen) => {
  await p.page.goto(`${BASE}/designer?screen=${screen}`)
  await p.page.locator('[data-testid=designer-collab-status]', { hasText: 'Live' }).waitFor({ timeout: 20_000 })
  await p.page.waitForFunction(() => document.querySelectorAll('[data-testid=designer-stage] [data-testid=designer-widget]').length > 0)
}
const sees = (p, other) => p.page.locator(`[data-testid=designer-peer-avatar][data-user="${other}"]`).waitFor({ timeout: 10_000 })
const hover = async (p, x, y) => { const pt = await stagePoint(p, x, y); await p.page.mouse.move(pt.x, pt.y, { steps: 4 }) }
const select = async (p, id) => { await p.page.locator(`[data-testid=designer-stage] [data-wid="${id}"]`).click() }
const rows = async () => (await (await admin.request.get(`${BASE}/api/v1/screens/${scratch.id}/scada-layout`)).json())

const label = (id, x, y) => ({
  equipment_id: id, equipment_type: 'label', equipment_name: id, x, y, width: 160, height: 50, rotation: 0,
  sensor_bindings: '[]', config: JSON.stringify({ text: id.toUpperCase(), z: 0 }),
})
const scratch = await (await admin.request.post(`${BASE}/api/v1/screens`, { data: { name: 'Collaboration smoke' } })).json()

try {
  const put = await admin.request.put(`${BASE}/api/v1/screens/${scratch.id}/scada-layout`, { data: {
    equipment: [label('w1', 200, 200), label('w2', 600, 200), label('w3', 1000, 200)], connections: [],
  } })
  assert.equal(put.status(), 200, 'scratch rows')

  step('both open the scratch screen and see each other')
  await open(admin, scratch.id)
  await open(bea, scratch.id)
  await sees(admin, SECOND)
  await sees(bea, 'admin')

  step('the admin moves w1; bea sees it move')
  await select(admin, 'w1')
  await admin.page.keyboard.press('Shift+ArrowRight')
  await bea.page.waitForFunction(() => document.querySelector('[data-testid=designer-stage] [data-wid="w1"]')?.style.left === '210px', null, { timeout: 5000 })

  step('bea locks w2; the admin sees the lock')
  await select(bea, 'w2')
  await bea.page.keyboard.press('Control+l')
  await select(admin, 'w2')
  await admin.page.locator('[data-testid=designer-lock][aria-pressed=true]').waitFor({ timeout: 5000 })

  step('both move different widgets at once and converge')
  await select(admin, 'w1')
  await select(bea, 'w3')
  await Promise.all([
    (async () => { for (let i = 0; i < 5; i++) await admin.page.keyboard.press('ArrowDown') })(),
    (async () => { for (let i = 0; i < 5; i++) await bea.page.keyboard.press('ArrowUp') })(),
  ])
  for (const p of [admin, bea]) {
    await p.page.waitForFunction(() => {
      const s = id => document.querySelector(`[data-testid=designer-stage] [data-wid="${id}"]`)?.style
      return s('w1')?.top === '205px' && s('w3')?.top === '195px'
    }, null, { timeout: 5000 })
  }
  assert.equal(await leftOf(admin, 'w1'), await leftOf(bea, 'w1'))

  step('cursors: each sees the other\'s, with a name')
  await hover(admin, 400, 500)
  await hover(bea, 900, 600)
  await bea.page.locator('[data-testid=designer-peer-cursor][data-user=admin]').waitFor({ timeout: 5000 })
  await admin.page.locator(`[data-testid=designer-peer-cursor][data-user="${SECOND}"]`).waitFor({ timeout: 5000 })

  step('a comment by the admin reaches bea')
  await admin.page.keyboard.press('m')
  await select(admin, 'w1')
  await admin.page.locator('[data-testid=designer-comment-body]').fill(`@${SECOND} please check the spacing`)
  await admin.page.keyboard.press('Enter')
  await bea.page.locator('[data-testid=designer-comment-pin]').first().waitFor({ timeout: 5000 })

  step('the rows over REST hold everyone\'s edits after a flush')
  await admin.page.keyboard.press('Control+s')
  let r
  for (let i = 0; i < 50; i++) {
    r = await rows()
    const by = Object.fromEntries(r.equipment.map(e => [e.equipment_id, e]))
    if (by.w1?.x === 210 && by.w1?.y === 205 && by.w3?.y === 195 && JSON.parse(by.w2.config).locked === true) break
    await admin.page.waitForTimeout(200)
  }
  const by = Object.fromEntries(r.equipment.map(e => [e.equipment_id, e]))
  assert.deepEqual([by.w1.x, by.w1.y, by.w3.y, JSON.parse(by.w2.config).locked], [210, 205, 195, true], JSON.stringify(r.equipment))
  const hist = await (await admin.request.get(`${BASE}/api/v1/screens/${scratch.id}/history?limit=50`)).json()
  const authors = new Set(hist.changes.map(c => c.username))
  assert.ok(authors.has('admin') && authors.has(SECOND), `both authors in the change log: ${[...authors]}`)
  const comments = await (await bea.request.get(`${BASE}/api/v1/screens/${scratch.id}/comments`)).json()
  assert.equal(comments.comments.length, 1)
  assert.deepEqual(comments.comments[0].mentions, [SECOND])

  if (shots) {
    step('screenshots on the Plant Overview')
    await open(admin, SHOWCASE)
    await open(bea, SHOWCASE)
    await sees(admin, SECOND)
    await sees(bea, 'admin')
    const ids = await admin.page.evaluate(() => [...document.querySelectorAll('[data-testid=designer-stage] [data-testid=designer-widget]')].map(e => e.getAttribute('data-wid')))
    const pick = ids.filter(Boolean).slice(-6)
    await select(admin, pick[0])
    await select(bea, pick[3])
    await hover(admin, 560, 420)
    await hover(bea, 1280, 640)
    await bea.page.locator('[data-testid=designer-peer-cursor][data-user=admin]').waitFor()
    await admin.page.locator(`[data-testid=designer-peer-cursor][data-user="${SECOND}"]`).waitFor()
    await admin.page.waitForTimeout(400)
    await shot(admin, '30-collab-two-cursors-admin')
    await shot(bea, '31-collab-two-cursors-bea')
    // Frames of bea's view while the admin's cursor travels (a GIF for the PR).
    for (let i = 0; i <= 24; i++) {
      await hover(admin, 560 + i * 30, 420 + Math.round(Math.sin(i / 4) * 120))
      await bea.page.waitForTimeout(90)
      await shot(bea, `gif-${String(i).padStart(2, '0')}`)
    }
    await admin.page.locator('[data-testid=designer-presence]').screenshot({ path: join(shots, '32-collab-presence.png') })
    await admin.page.keyboard.press('m')
    await select(admin, pick[0])
    await admin.page.locator('[data-testid=designer-comment-body]').fill(`@${SECOND} can we move this one up a grid step?`)
    await admin.page.keyboard.press('Enter')
    await bea.page.locator('[data-testid=designer-comment-pin]').first().waitFor()
    await bea.page.locator('[data-testid=designer-tab-comments]').click().catch(() => {})
    await bea.page.locator('[data-testid=designer-comment-pin]').first().click()
    await bea.page.locator('[data-testid=designer-comment-reply]').fill('Done, and aligned with the pump row.')
    await bea.page.keyboard.press('Enter')
    await admin.page.waitForTimeout(500)
    await shot(bea, '33-collab-comment-thread')
    // Clean the showcase: resolve and delete the comment.
    const cs = (await (await admin.request.get(`${BASE}/api/v1/screens/${SHOWCASE}/comments?resolved=1`)).json()).comments
    for (const c of cs.filter(c => c.parent_id == null)) await admin.request.delete(`${BASE}/api/v1/screens/${SHOWCASE}/comments/${c.id}`)
  }
} catch (e) {
  await shot(admin, 'failure-admin').catch(() => {})
  await shot(bea, 'failure-bea').catch(() => {})
  console.error('page errors:', errors)
  await browser.close()
  throw e
} finally {
  await admin.request.delete(`${BASE}/api/v1/screens/${scratch.id}`).catch(() => {})
}

assert.deepEqual(errors, [], 'no page errors')
await browser.close()
console.log('collab smoke: ok')
