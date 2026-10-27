import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import {
  DEFAULT_CATALOG_URL, filterCatalog, filterQuery, fromDeviceBody, instantiateBody, isValidTemplateId, isValidVersion,
  matchesFilter, parseQuirks, refQuery, slugify, sourceLabel, templateKey,
} from '../src/deviceTemplates.ts'
import type { CatalogEntry, TemplateSummary } from '../src/deviceTemplates.ts'

const t = (over: Partial<TemplateSummary>): TemplateSummary => ({
  template_id: 'acme-meter', manufacturer: 'Acme', model: 'Meter 9', protocol: 'modbus', description: 'Three phase meter',
  version: '1.0.0', verified: false, source: 'local', license: 'CC-BY-4.0', item_count: 4, read_only: false, ...over,
})

test('the default catalog URL and the reserved ids are the server\'s', () => {
  const catalog = readFileSync(new URL('../../src/device_templates/catalog.rs', import.meta.url), 'utf8')
  assert.ok(catalog.includes(`"${DEFAULT_CATALOG_URL}"`), 'DEFAULT_CATALOG_URL in catalog.rs')
  const format = readFileSync(new URL('../../src/device_templates/format.rs', import.meta.url), 'utf8')
  const reserved = format.match(/RESERVED_IDS: \[&str; \d+\] = \[([^\]]*)\]/)
  assert.ok(reserved)
  for (const id of ['catalog', 'from-device']) {
    assert.ok(reserved[1].includes(`"${id}"`), id)
    assert.equal(isValidTemplateId(id), false, id)
  }
})

test('slugify matches the server\'s slugs for the built-in templates', () => {
  assert.equal(slugify('Schneider Electric PM5560'), 'schneider-electric-pm5560')
  assert.equal(slugify('Endress+Hauser Promag 10W DN50'), 'endress-hauser-promag-10w-dn50')
  assert.equal(slugify('Trane Tracer SC+'), 'trane-tracer-sc')
  assert.equal(slugify('  --A  b-- '), 'a-b')
  assert.equal(slugify('!!!'), '')
  assert.equal(slugify('x'.repeat(80)).length, 64)
})

test('template ids and versions', () => {
  for (const id of ['a', 'abb-azl402', 'x1-y2']) assert.ok(isValidTemplateId(id), id)
  for (const id of ['', 'A', 'a--b', '-a', 'a-', 'a_b', 'a b', 'x'.repeat(65)]) assert.ok(!isValidTemplateId(id), id)
  for (const v of ['1.0.0', '0.10.2', '2.0.0-rc.1']) assert.ok(isValidVersion(v), v)
  for (const v of ['1.0', '01.0.0', 'v1.0.0', '1.0.0-', '']) assert.ok(!isValidVersion(v), v)
})

test('source labels', () => {
  assert.equal(sourceLabel('builtin'), 'Built-in')
  assert.equal(sourceLabel('local'), 'Local')
  assert.equal(sourceLabel('catalog:https://raw.githubusercontent.com/VoltrusID/x/main/index.json'), 'Catalog (raw.githubusercontent.com)')
  assert.equal(sourceLabel('catalog:not a url'), 'Catalog')
})

test('filter query leaves empty fields out and encodes the rest', () => {
  assert.equal(filterQuery({}), '')
  assert.equal(filterQuery({ q: '  ', protocol: '' }), '')
  assert.equal(filterQuery({ q: 'pm 5560', protocol: 'modbus' }), '?q=pm+5560&protocol=modbus')
  assert.equal(refQuery({ version: '1.0.0', source: 'catalog:https://h/i.json' }), '?version=1.0.0&source=catalog%3Ahttps%3A%2F%2Fh%2Fi.json')
})

test('client-side search matches like the server', () => {
  const m = t({})
  assert.ok(matchesFilter(m, {}))
  assert.ok(matchesFilter(m, { q: 'METER' }))
  assert.ok(matchesFilter(m, { q: 'three phase' }))
  assert.ok(!matchesFilter(m, { q: 'inverter' }))
  assert.ok(matchesFilter(m, { manufacturer: 'acme' }))
  assert.ok(!matchesFilter(m, { manufacturer: 'acm' }))
  assert.ok(matchesFilter(m, { protocol: 'modbus' }))
  assert.ok(!matchesFilter(m, { protocol: 'bacnet' }))
  assert.ok(matchesFilter(m, { source: 'local' }))
  assert.ok(!matchesFilter(m, { source: 'catalog' }))
  assert.ok(matchesFilter(t({ source: 'catalog:https://h/i.json' }), { source: 'catalog' }))
  assert.notEqual(templateKey(m), templateKey(t({ source: 'builtin' })))
})

test('catalog entries filter by text and protocol', () => {
  const e = (id: string, manufacturer: string, protocol: 'modbus' | 'bacnet'): CatalogEntry =>
    ({ template_id: id, manufacturer, model: 'M', protocol, version: '1.0.0', verified: false, path: `${id}.json`, sha256: 'a'.repeat(64), installed: false })
  const list = [e('abb-azl402', 'ABB', 'modbus'), e('trane-tracer-sc', 'Trane', 'bacnet')]
  assert.equal(filterCatalog(list, '', '').length, 2)
  assert.deepEqual(filterCatalog(list, 'trane', '').map(x => x.template_id), ['trane-tracer-sc'])
  assert.deepEqual(filterCatalog(list, '', 'modbus').map(x => x.template_id), ['abb-azl402'])
  assert.deepEqual(filterCatalog(list, 'abb', 'bacnet'), [])
})

test('quirks are one per line', () => {
  assert.deepEqual(parseQuirks(' word order swapped \n\n  offsets are 1-based\n'), ['word order swapped', 'offsets are 1-based'])
  assert.deepEqual(parseQuirks(''), [])
})

const saveForm = { manufacturer: 'Acme', model: 'Meter 9', templateId: '', version: '', description: ' d ', verified: false, verifiedNotes: '', quirksText: 'a\nb', license: '' }

test('save-as-template body fills the id, version and license', () => {
  const r = fromDeviceBody(saveForm, 'modbus')
  assert.deepEqual(r.errors, [])
  assert.deepEqual(r.body, {
    manufacturer: 'Acme', model: 'Meter 9', template_id: 'acme-meter-9', version: '1.0.0', description: 'd',
    verified: false, verified_notes: '', quirks: ['a', 'b'], license: 'CC-BY-4.0', protocol: 'modbus',
  })
})

test('save-as-template body reports every problem', () => {
  const r = fromDeviceBody({ ...saveForm, manufacturer: ' ', model: '', templateId: 'Bad Id', version: '1.0', verified: true }, 'bacnet')
  assert.equal(r.body, undefined)
  assert.equal(r.errors.length, 5)
  assert.ok(r.errors.some(e => e.includes('Manufacturer')) && r.errors.some(e => e.includes('Model')) && r.errors.some(e => e.includes('Version')))
  assert.ok(r.errors.some(e => e.includes('verified')))
  // 'catalog' would shadow a route.
  assert.ok(fromDeviceBody({ ...saveForm, templateId: 'catalog' }, 'modbus').errors.length > 0)
})

const form = { name: ' Grid meter ', ip: ' 10.0.0.5 ', port: '', unitId: '', deviceInstance: '', intervalMs: '' }

test('instantiate body sends only what was filled in', () => {
  assert.deepEqual(instantiateBody(form, 'modbus').body, { name: 'Grid meter', ip: '10.0.0.5' })
  assert.deepEqual(
    instantiateBody({ ...form, port: '1502', unitId: '0', intervalMs: '500' }, 'modbus', { version: '1.2.0', source: 'local' }).body,
    { name: 'Grid meter', ip: '10.0.0.5', port: 1502, unit_id: 0, interval_ms: 500, version: '1.2.0', source: 'local' },
  )
  // BACnet always sends a device instance, 0 when blank; unit id does not apply.
  assert.deepEqual(instantiateBody({ ...form, unitId: '5' }, 'bacnet').body, { name: 'Grid meter', ip: '10.0.0.5', device_instance: 0 })
  assert.equal(instantiateBody({ ...form, deviceInstance: '4194303' }, 'bacnet').body?.device_instance, 4194303)
})

test('instantiate body rejects values the server would', () => {
  const bad = (over: Partial<typeof form>, protocol: 'modbus' | 'bacnet' = 'modbus') => instantiateBody({ ...form, ...over }, protocol)
  assert.equal(bad({ name: '' }).errors.length, 1)
  assert.equal(bad({ ip: ' ' }).errors.length, 1)
  assert.equal(bad({ port: '0' }).errors.length, 1)
  assert.equal(bad({ port: '70000' }).errors.length, 1)
  assert.equal(bad({ port: '5x' }).errors.length, 1)
  assert.equal(bad({ unitId: '256' }).errors.length, 1)
  assert.equal(bad({ intervalMs: '0' }).errors.length, 1)
  assert.equal(bad({ deviceInstance: '4194304' }, 'bacnet').errors.length, 1)
  assert.equal(bad({ name: '', ip: '', port: '-1' }).errors.length, 3)
  assert.equal(bad({ port: '0' }).body, undefined)
})


