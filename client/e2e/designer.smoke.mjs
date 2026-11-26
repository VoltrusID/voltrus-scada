// Browser smoke run of the Voltrus Designer against a running server with the
// WTP showcase (scenarios/wtp-hmi). Not part of `npm test`: it needs a server,
// the plant simulator and a browser.
//
//   VOLTRUS_URL=http://localhost:5173 VOLTRUS_PASSWORD=... \
//   PLAYWRIGHT_CORE=/path/to/node_modules/playwright-core \
//   node e2e/designer.smoke.mjs [screenshot-dir]
//
// Or all in one (plant simulator, server, seed, browser), as CI runs it:
//   scripts/smoke-designer.sh target/release/voltrus [screenshot-dir]
//
// It opens the Designer on the Plant Overview screen, drags a tag onto the
// screen, binds a property to an expression, groups and aligns, undoes and
// redoes, saves, reloads and checks the new component came back from the
// server. Then, on a scratch screen, it drops the Pump equipment type, places
// the seeded instance P-101A and a new instance, checks both show live run
// states, saves, checks the rows over REST and opens the faceplate in preview.
// Then version history on the Plant Overview: an edit, a version, another
// edit, the History pane with a past state previewed, blame, compare (canvases
// and YAML diff), revert to the version, publish, and an operator who keeps
// seeing the published screen while the draft moves on; on a scratch screen,
// reverting a connection whose widget is gone restores the widget too.
// With a screenshot directory it also writes the PR screenshots.

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
const shot = async name => { if (shots) await page.screenshot({ path: join(shots, `${name}.png`) }) }
const step = msg => console.log(`· ${msg}`)

const login = await context.request.post(`${BASE}/api/v1/auth/login`, { data: { username: 'admin', password: PASSWORD } })
assert.equal(login.status(), 200, 'admin login')
// This run covers the single-user Designer (save, conflicts, versions);
// e2e/collab.smoke.mjs covers live collaboration, on by default.
const off = await context.request.put(`${BASE}/api/v1/settings/design-history`, { data: { collaboration: false } })
assert.equal(off.status(), 200, 'collaboration off')
// A server older than the faceplate API (v0.52) answers 404; the Designer only lists definitions.
if (process.env.MOCK_FACEPLATES) await page.route('**/api/v1/faceplates', r => r.fulfill({ json: [] }))
await page.addInitScript(() => localStorage.setItem('voltrus-theme', 'dark'))

const widgets = () => page.locator('[data-testid=designer-widget]').count()
const openDesigner = async () => {
  await page.goto(`${BASE}/designer?screen=${SCREEN}`)
  await page.locator('[data-testid=designer-stage]').waitFor()
  await page.waitForFunction(() => document.querySelectorAll('[data-testid=designer-widget]').length > 0)
  // Live values in: the tag browser shows numbers.
  await page.waitForFunction(() => /\d/.test(document.querySelector('[data-testid=designer-tags]')?.textContent ?? ''))
  await page.waitForTimeout(1500)
}

/** Client point of a stage coordinate. */
const stagePoint = async (x, y) => page.evaluate(([x, y]) => {
  const s = document.querySelector('[data-testid=designer-stage]')
  const r = s.getBoundingClientRect()
  const k = r.width / 1920
  return { x: r.left + x * k, y: r.top + y * k }
}, [x, y])

try {
step('open the Designer on the Plant Overview')
await openDesigner()
const before = await widgets()
const idsBefore = new Set(await page.evaluate(() => [...document.querySelectorAll('[data-testid=designer-widget]')].map(e => e.getAttribute('data-wid'))))
assert.ok(before > 10, `screen has components (${before})`)
await shot('01-designer-wtp')

step('drag a tag from the tag browser onto an empty spot')
const tag = page.locator('[data-testid=designer-tag][data-ref="wtp/clearwell_level"]')
await page.locator('[data-testid=designer-tag-browser] input').fill('clearwell')
await tag.waitFor()
const viewport = page.locator('[data-testid=designer-viewport]')
// An empty spot of the Filtration area, under the effluent valve FCV-320.
const drop = await stagePoint(1400, 560)
const vbox = await viewport.boundingBox()
await tag.dragTo(viewport, { targetPosition: { x: drop.x - vbox.x, y: drop.y - vbox.y } })
await page.locator('[data-testid=designer-drop-menu]').waitFor()
await shot('02-tag-drop-menu')
await page.locator('[data-testid=designer-drop-menu] [role=menuitem]').first().click()
assert.equal(await widgets(), before + 1, 'a bound component was added')
const newWidget = (await page.evaluate(() => [...document.querySelectorAll('[data-testid=designer-widget]')].map(e => e.getAttribute('data-wid')))).find(id => !idsBefore.has(id))
assert.equal(await page.locator('[data-testid=designer-dirty]').textContent(), 'Unsaved changes')
const tagRow = page.locator('[data-testid=designer-prop-tag] select')
const boundTag = await tagRow.inputValue()
assert.ok(boundTag, 'the new component is bound to the dropped tag')

step('bind its label to an expression')
await page.locator('[data-testid=designer-bind-label]').click()
await page.locator('[data-testid=designer-bindkind-expr]').click()
await page.locator('[data-testid=designer-expr]').fill('{wtp/clearwell_level} > 90 ? "CLEARWELL HIGH" : "Clearwell " + text({wtp/clearwell_level}, 1) + " %"')
await page.waitForFunction(() => /Now: /.test(document.querySelector('[data-testid=designer-binding-preview]')?.textContent ?? ''))
await shot('03-expression-binding')
await page.locator('[data-testid=designer-binding-apply]').click()
await page.locator('[data-testid=designer-prop-label]').filter({ hasText: 'clearwell_level' }).waitFor()

step('undo removes the binding, redo puts it back')
await page.keyboard.press('Control+z')
assert.equal(await page.locator('[data-testid=designer-prop-label]').filter({ hasText: 'clearwell_level' }).count(), 0)
await page.keyboard.press('Control+y')
await page.locator('[data-testid=designer-prop-label]').filter({ hasText: 'clearwell_level' }).waitFor()

step('marquee-select, shift-click, align, group')
// A marquee across the KPI row in the header selects several components.
await page.keyboard.press('Escape')
const a = await stagePoint(960, 3), b = await stagePoint(1915, 95)
await page.mouse.move(a.x, a.y)
await page.mouse.down()
await page.mouse.move((a.x + b.x) / 2, (a.y + b.y) / 2, { steps: 4 })
await page.mouse.move(b.x, b.y, { steps: 4 })
await shot('04a-marquee')
await page.mouse.up()
await page.locator('[data-testid=designer-props-multi]').waitFor()
// The new component plus the valve FCV-320 above it: click, then shift-click; centre them and group.
await page.keyboard.press('Escape')
const added = page.locator(`[data-wid="${newWidget}"]`)
await added.click()
const fcv = await stagePoint(1405, 345)
await page.keyboard.down('Shift')
await page.mouse.click(fcv.x, fcv.y)
await page.keyboard.up('Shift')
await page.locator('[data-testid=designer-props-multi]').waitFor()
await page.locator('[data-testid=designer-align-hcenter]').click()
await page.locator('[data-testid=designer-group]').click()
await shot('04-multi-select-group')

step('save, reload, the component is still there and bound')
await page.keyboard.press('Control+s')
await page.locator('[data-testid=designer-dirty]', { hasText: 'All changes saved' }).waitFor()
const rows = await (await context.request.get(`${BASE}/api/v1/screens/${SCREEN}/scada-equipment`)).json()
const saved = rows.items.find(r => r.equipment_id === newWidget)
assert.ok(saved, 'the bound component is in the saved rows')
assert.equal(JSON.parse(saved.sensor_bindings)[0], Number(boundTag))
assert.ok(JSON.parse(saved.config).group, 'and grouped')
await page.reload()
await openDesigner()
assert.equal(await widgets(), before + 1, 'after reload')
await page.locator(`[data-wid="${saved.equipment_id}"]`).click()
await page.locator('[data-testid=designer-props-multi]').waitFor()

step('ISA-101 symbols and property editor')
await page.keyboard.press('Escape')
await page.locator(`[data-wid="${saved.equipment_id}"]`).dblclick()
await page.locator('[data-testid=designer-props-widget]').waitFor()
await shot('05-property-editor')

step('preview with live data')
await page.keyboard.press('F5')
await page.locator('[data-testid=designer-preview]').waitFor()
await page.waitForTimeout(2500)
await shot('06-preview-live')
await page.keyboard.press('F5')

step('light theme')
await page.evaluate(() => { localStorage.setItem('voltrus-theme', 'light'); document.documentElement.setAttribute('data-theme', 'light') })
await page.waitForTimeout(500)
await shot('07-designer-light')

step('clean up: remove the test component and save')
await page.evaluate(() => { localStorage.setItem('voltrus-theme', 'dark'); document.documentElement.setAttribute('data-theme', 'dark') })
await page.locator(`[data-wid="${saved.equipment_id}"]`).dblclick()
await page.keyboard.press('Control+Shift+g')
await page.locator(`[data-wid="${saved.equipment_id}"]`).click()
await page.keyboard.press('Delete')
await page.keyboard.press('Control+s')
await page.locator('[data-testid=designer-dirty]', { hasText: 'All changes saved' }).waitFor()
assert.equal(await widgets(), before)

step('equipment types: drop a Pump, place an existing instance and a new one, bindings live')
const tagIds = Object.fromEntries((await (await context.request.get(`${BASE}/api/v1/data-sources`)).json()).map(t => [`${t.device_id}/${t.key}`, t.id]))
const scratch = await (await context.request.post(`${BASE}/api/v1/screens`, { data: { name: 'Equipment demo' } })).json()
try {
  await page.goto(`${BASE}/designer?screen=${scratch.id}`)
  await page.locator('[data-testid=designer-stage]').waitFor()
  await page.locator('[data-testid=designer-equipment-pump]').waitFor()
  const dropAt = async (loc, x, y) => {
    const p = await stagePoint(x, y)
    const vb = await viewport.boundingBox()
    await loc.dragTo(viewport, { targetPosition: { x: p.x - vb.x, y: p.y - vb.y } })
  }
  const picker = page.locator('[data-testid=designer-instance-picker]')
  const newWid = async before => (await page.evaluate(() => [...document.querySelectorAll('[data-testid=designer-widget]')].map(e => e.getAttribute('data-wid'))))
    .find(id => !before.includes(id))
  const wids = () => page.evaluate(() => [...document.querySelectorAll('[data-testid=designer-widget]')].map(e => e.getAttribute('data-wid')))

  // An existing instance from the seeded plant.
  let before = await wids()
  await dropAt(page.locator('[data-testid=designer-equipment-pump]'), 420, 420)
  await picker.waitFor()
  await shot('10-equipment-picker')
  await picker.locator('[data-testid="designer-instance-existing-P-101A"]').click()
  await picker.waitFor({ state: 'detached' })
  const wA = await newWid(before)
  assert.ok(wA, 'P-101A placed')
  assert.equal(await page.locator('[data-testid=designer-equipment-name]').textContent(), 'P-101A')
  // Live: the symbol shows the pump's run state from the PLC, not NO DATA.
  await page.waitForFunction(id => /RUNNING|STOPPED/.test(document.querySelector(`[data-wid="${id}"]`)?.textContent ?? ''), wA, { timeout: 15000 })

  // A new instance: a Pump bound by its prefix to a tag no instance has yet.
  before = await wids()
  await dropAt(page.locator('[data-testid=designer-equipment-pump]'), 760, 420)
  await picker.waitFor()
  await picker.locator('[data-testid=designer-instance-tab-new]').click()
  await picker.locator('[data-testid=designer-instance-name]').fill('P-311')
  await picker.locator('[data-testid=designer-instance-device]').selectOption('wtp')
  await picker.locator('[data-testid=designer-instance-prefix]').fill('air_blower')
  await picker.locator('[data-testid=designer-instance-role][data-role=run][data-status=bound]').waitFor()
  await shot('11-equipment-new-instance')
  await picker.locator('[data-testid=designer-instance-create]').click()
  await picker.waitFor({ state: 'detached' })
  const wB = await newWid(before)
  assert.ok(wB, 'P-311 placed')
  await page.waitForFunction(id => /RUNNING|STOPPED/.test(document.querySelector(`[data-wid="${id}"]`)?.textContent ?? ''), wB, { timeout: 15000 })
  await page.locator(`[data-wid="${wA}"]`).click()
  await page.locator('[data-testid=designer-props-equipment]').waitFor()
  await shot('12-equipment-properties')

  step('save: the rows carry the instances and their tags')
  await page.keyboard.press('Control+s')
  await page.locator('[data-testid=designer-dirty]', { hasText: 'All changes saved' }).waitFor()
  const eqRows = (await (await context.request.get(`${BASE}/api/v1/screens/${scratch.id}/scada-equipment`)).json()).items
  const byInstance = Object.fromEntries(eqRows.map(r => [JSON.parse(r.config).equipment, { ...r, config: JSON.parse(r.config) }]))
  assert.deepEqual(JSON.parse(byInstance['P-101A'].sensor_bindings), [tagIds['wtp/p101a_run']])
  assert.equal(byInstance['P-101A'].config.faceplate, 'pump')
  assert.equal(byInstance['P-101A'].config.faceplateMap.run, tagIds['wtp/p101a_run'])
  assert.deepEqual(JSON.parse(byInstance['P-311'].sensor_bindings), [tagIds['wtp/air_blower_run']])
  const inst = await (await context.request.get(`${BASE}/api/v1/equipment-instances/P-311`)).json()
  assert.equal(inst.type, 'pump')

  step('preview: clicking the pump opens its faceplate with live tags')
  await page.keyboard.press('F5')
  await page.locator('[data-testid=designer-preview]').waitFor()
  await page.waitForTimeout(1500)
  await page.locator('[data-testid=designer-preview]').getByText('P-101A', { exact: true }).click()
  await page.locator('[role=dialog][aria-label="P-101A faceplate"]').waitFor()
  await shot('13-equipment-faceplate')
  await page.locator('[aria-label="Close faceplate"]').click()
  await page.keyboard.press('F5')
  await page.locator('[data-testid=designer-stage]').waitFor()

  if (shots) {
    step('ISA-101 symbol library on the scratch screen (screenshots)')
    await page.locator('[data-testid=designer-components] input').first().fill('valve')
    await page.waitForTimeout(300)
    await shot('14-palette-search')
    await page.locator('[data-testid=designer-components] input').first().fill('')
    const sym = k => page.locator(`[data-testid=designer-symbol-${k}]`)
    const row = ['isa_pump_centrifugal', 'isa_pump_pd', 'isa_pump_dosing', 'isa_compressor', 'isa_blower', 'isa_fan', 'isa_agitator',
      'isa_valve_gate', 'isa_valve_ball', 'isa_valve_butterfly', 'isa_valve_check', 'isa_valve_control', 'isa_valve_3way', 'isa_valve_solenoid',
      'isa_tank_vertical', 'isa_tank_horizontal', 'isa_tank_cone', 'isa_silo', 'isa_clarifier', 'isa_hx_shell_tube', 'isa_hx_plate',
      'isa_filter', 'isa_cyclone', 'isa_instrument', 'isa_reducer', 'isa_junction', 'isa_flow_arrow']
    for (const [i, k] of row.entries()) {
      await page.locator('[data-testid=designer-components] input').first().fill(k)
      await sym(k).waitFor()
      await dropAt(sym(k), 120 + (i % 9) * 200, 640 + Math.floor(i / 9) * 150)
    }
    await page.locator('[data-testid=designer-components] input').first().fill('')
    const empty = await stagePoint(1880, 1060)
    await page.mouse.click(empty.x, empty.y)
    await page.waitForTimeout(800)
    await shot('15-isa101-library')
    await page.keyboard.press('F5')
    await page.waitForTimeout(2000)
    await shot('16-isa101-preview')
    await page.keyboard.press('F5')
    // Unsaved symbols: discard on leave.
  }
} finally {
  await context.request.delete(`${BASE}/api/v1/screens/${scratch.id}`)
  await context.request.delete(`${BASE}/api/v1/equipment-instances/P-311`)
}

// ── Version history (phase 3) ──
const layoutRows = async (screenId, ctx = context.request, query = '') =>
  (await (await ctx.get(`${BASE}/api/v1/screens/${screenId}/scada-layout${query}`)).json())
const tab = name => page.locator(`[data-testid=designer-tab-${name}]`).click()
const saveNow = async () => {
  await page.keyboard.press('Control+s')
  await page.locator('[data-testid=designer-dirty]', { hasText: 'All changes saved' }).waitFor()
}

step('history: an edit saved, a version, another edit')
await openDesigner()
// A free-standing component on top (not grouped, locked or an area) to move.
const draft0 = await layoutRows(SCREEN)
const movable = new Set(draft0.equipment.filter(r => {
  const c = JSON.parse(r.config || '{}')
  return !c.group && !c.locked && r.equipment_type !== 'area' && r.width < 400
}).map(r => r.equipment_id))
const target = await page.evaluate(ids => {
  for (const el of document.querySelectorAll('[data-testid=designer-widget]')) {
    const id = el.getAttribute('data-wid')
    if (!ids.includes(id)) continue
    const r = el.getBoundingClientRect()
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)
    if (hit && hit.closest('[data-wid]')?.getAttribute('data-wid') === id && r.width > 8) return { id, x: r.left + r.width / 2, y: r.top + r.height / 2 }
  }
  return null
}, [...movable])
assert.ok(target, 'a component to edit')
const x0 = draft0.equipment.find(r => r.equipment_id === target.id).x
const xOf = rows => rows.equipment.find(r => r.equipment_id === target.id)?.x
const nudge = async () => {
  await page.mouse.click(target.x, target.y)
  await page.locator('[data-testid=designer-props-widget]').waitFor()
  await page.keyboard.press('Shift+ArrowRight')
  target.x += 10 * ((await page.locator('[data-testid=designer-stage]').boundingBox()).width / 1920)
  await saveNow()
}
await nudge()
assert.equal(xOf(await layoutRows(SCREEN)), x0 + 10, 'first edit saved')

await tab('versions')
await page.locator('[data-testid=designer-version-name]').fill('v-smoke')
await page.locator('[data-testid=designer-version-message]').fill('Smoke test: one component moved 10 px')
await page.locator('[data-testid=designer-version-create]').click()
await page.locator('[data-testid=designer-version][data-version=v-smoke]').waitFor()

await tab('properties')
await nudge()
assert.equal(xOf(await layoutRows(SCREEN)), x0 + 20, 'second edit saved')

step('History pane: batches, preview of a past state')
await tab('history')
await page.locator('[data-testid=designer-history-batch]').first().waitFor()
await page.locator('[data-testid=designer-history-whole]').check()
const batches = page.locator('[data-testid=designer-history-batch]')
await page.waitForFunction(() => document.querySelectorAll('[data-testid=designer-history-batch]').length >= 2)
// The older save: preview the screen as it was then.
await batches.nth(1).locator('button').first().click()
await page.locator('[data-testid=designer-preview-banner]').waitFor()
await page.waitForFunction(() => document.querySelectorAll('[data-testid=designer-preview-stage] [data-testid=designer-widget]').length > 0)
await page.waitForTimeout(800)
await shot('16-history-pane')
await page.locator('[data-testid=designer-preview-back]').click()
await page.locator('[data-testid=designer-stage]').waitFor()

step('blame in the Properties pane')
await tab('properties')
await page.mouse.click(target.x, target.y)
await page.locator('[data-testid=designer-blame]', { hasText: 'Last changed by admin' }).waitFor()
await shot('17-blame')

step('compare v-smoke with the draft')
await tab('versions')
await page.locator('[data-testid=designer-version][data-version=v-smoke]').waitFor()
await shot('18-versions')
await page.locator('[data-testid=designer-version-vs-draft-v-smoke]').click()
const cmp = page.locator('[data-testid=designer-compare]')
await cmp.locator(`[data-testid=designer-compare-object][data-object="${target.id}"][data-kind=changed]`).waitFor()
assert.ok(await cmp.locator('[data-testid=designer-yaml-del]').count() >= 1, 'YAML diff has removed lines')
assert.ok(await cmp.locator('[data-testid=designer-yaml-add]').count() >= 1, 'YAML diff has added lines')
await cmp.locator(`[data-testid=designer-compare-to] [data-testid=designer-outline-changed][data-wid="${target.id}"]`).waitFor()
await page.waitForTimeout(800)
await shot('19-compare-diff')

step('revert the screen to v-smoke')
await cmp.locator('[data-testid=designer-compare-revert]').click()
await page.locator('[data-testid=designer-revert-dialog]').waitFor()
await page.locator('[data-testid=designer-revert-note]').fill('Back to the smoke version')
await page.locator('[data-testid=designer-revert-confirm]').click()
await page.locator('[data-testid=designer-revert-dialog]').waitFor({ state: 'detached' })
await page.waitForFunction(() => !document.querySelector('[data-testid=designer-compare]'))
for (let i = 0; i < 50 && xOf(await layoutRows(SCREEN)) !== x0 + 10; i++) await page.waitForTimeout(100)
assert.equal(xOf(await layoutRows(SCREEN)), x0 + 10, 'reverted rows over REST')
assert.equal(await page.locator('[data-testid=designer-dirty]').textContent(), 'All changes saved')

step('publish with a message')
await page.locator('[data-testid=designer-publish]').click()
const pub = page.locator('[data-testid=designer-publish-dialog]')
await pub.waitFor()
await pub.locator('[data-testid=designer-publish-message]').fill('Moved one component; reverted to v-smoke')
await page.waitForFunction(() => !/…/.test(document.querySelector('[data-testid=designer-publish-summary]')?.textContent ?? '…'))
await shot('20-publish-dialog')
await pub.locator('[data-testid=designer-publish-confirm]').click()
await pub.locator('[data-testid=designer-publish-result][data-status=published]').waitFor()
await pub.locator('[data-testid=designer-publish-close]').click()
await page.locator('[data-testid=designer-publish-state][data-tone=ok]').waitFor()
assert.match(await page.locator('[data-testid=designer-publish-state]').textContent(), /^Published /)
await shot('21-published-indicator')

step('operators see the published screen, not later draft edits')
const opName = `smoke-operator-${Date.now()}`
const opPassword = 'Smoke-Operator-2026!'
const created = await context.request.post(`${BASE}/api/v1/users`, { data: { username: opName, password: opPassword, role: 'operator' } })
assert.equal(created.status(), 200, `operator created: ${await created.text()}`)
const opId = (await created.json()).id
// Its own cookies: a second browser context.
const operator = await browser.newContext()
try {
  const opLogin = await operator.request.post(`${BASE}/api/v1/auth/login`, { data: { username: opName, password: opPassword } })
  assert.equal(opLogin.status(), 200, 'operator login')
  let opView = await layoutRows(SCREEN, operator.request)
  assert.equal(opView.view, 'published')
  assert.equal(xOf(opView), x0 + 10)

  await tab('properties')
  await nudge()
  await page.locator('[data-testid=designer-publish-state][data-tone=warn]').waitFor()
  assert.match(await page.locator('[data-testid=designer-publish-state]').textContent(), /^Draft · \d+ changes? since /)
  assert.equal(xOf(await layoutRows(SCREEN)), x0 + 20, 'the Designer (draft) has the new edit')
  opView = await layoutRows(SCREEN, operator.request)
  assert.equal(xOf(opView), x0 + 10, 'the operator does not see it')
  assert.equal(xOf(await layoutRows(SCREEN, context.request, '?published=1')), x0 + 10, 'nor does ?published=1')
} finally {
  await operator.close()
  if (opId != null) await context.request.delete(`${BASE}/api/v1/users/${opId}`)
}

step('revert a connection whose widget is gone: also restore it')
const hist = await (await context.request.post(`${BASE}/api/v1/screens`, { data: { name: 'History demo' } })).json()
try {
  const eqRow = (id, x, label, symbol) => ({
    screen_id: hist.id, equipment_id: id, equipment_type: 'symbol', equipment_name: label, x, y: 380, width: 110, height: 120, rotation: 0,
    sensor_bindings: '[]', config: JSON.stringify({ symbol, label }),
  })
  const conn = { screen_id: hist.id, connection_id: 'c-feed', from_equipment: 'tk-1', to_equipment: 'p-1', connection_type: 'flow', waypoints: '[]' }
  // Two saves from different editors, so the second is not squashed into the first (same author and source within a minute).
  const put = (data, client) => context.request.put(`${BASE}/api/v1/screens/${hist.id}/scada-layout`, { data, headers: client ? { 'X-Voltrus-Client': client } : {} })
  assert.equal((await put({ equipment: [eqRow('tk-1', 520, 'TK-1', 'isa_tank'), eqRow('p-1', 980, 'P-1', 'isa_pump')], connections: [conn] }, 'designer')).status(), 200)
  assert.equal((await put({ equipment: [eqRow('tk-1', 520, 'TK-1', 'isa_tank')], connections: [] })).status(), 200)
  await page.goto(`${BASE}/designer?screen=${hist.id}`)
  await page.locator('[data-testid=designer-stage]').waitFor()
  await tab('history')
  const first = page.locator('[data-testid=designer-history-batch]').nth(1)
  await first.waitFor()
  await first.locator('button').first().click()
  await page.locator('[data-testid=designer-revert-object-c-feed]').click()
  await page.locator('[data-testid=designer-revert-confirm]').click()
  const dangling = page.locator('[data-testid=designer-dangling-dialog]')
  await dangling.waitFor()
  assert.match(await dangling.textContent(), /p-1/)
  await shot('22-revert-dangling')
  await dangling.locator('[data-testid=designer-revert-also-restore]').click()
  await dangling.waitFor({ state: 'detached' })
  let rows
  for (let i = 0; i < 50; i++) {
    rows = await layoutRows(hist.id)
    if (rows.connections.length) break
    await page.waitForTimeout(100)
  }
  assert.deepEqual(rows.equipment.map(r => r.equipment_id).sort(), ['p-1', 'tk-1'])
  assert.deepEqual(rows.connections.map(c => c.connection_id), ['c-feed'])
  await page.waitForFunction(() => document.querySelectorAll('[data-testid=designer-stage] [data-testid=designer-widget]').length === 2)
} finally {
  await context.request.delete(`${BASE}/api/v1/screens/${hist.id}`)
}
} catch (e) {
  await shot('failure').catch(() => {})
  console.error('page errors:', errors)
  await browser.close()
  throw e
}

assert.deepEqual(errors, [], 'no page errors')
await browser.close()
console.log('designer smoke: ok')
