import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { PDF_GUIDANCE, datasheetFileError, looksLikePdf, parseEditedConfig } from '../src/copilot/proposalChecks.ts'

test('an invalid JSON edit is an error, not the original proposal', () => {
  const r = parseEditedConfig('[{"name": "v", "address": 40001')
  assert.equal(r.ok, false)
  assert.match(!r.ok ? r.error : '', /not valid JSON/)
})

test('an empty edit means no edit; valid JSON is passed on', () => {
  assert.deepEqual(parseEditedConfig('  \n'), { ok: true })
  assert.deepEqual(parseEditedConfig('[{"name": "v"}]'), { ok: true, config: [{ name: 'v' }] })
})

test('PDF datasheets are refused with guidance', () => {
  assert.equal(datasheetFileError({ name: 'meter.PDF' }), PDF_GUIDANCE)
  assert.equal(datasheetFileError({ name: 'meter.bin', type: 'application/pdf' }), PDF_GUIDANCE)
  assert.equal(datasheetFileError({ name: 'meter.csv', type: 'text/csv' }), null)
  assert.match(PDF_GUIDANCE, /\.csv or \.txt/)
  assert.ok(looksLikePdf('%PDF-1.7\n%��'))
  assert.ok(looksLikePdf('﻿  %PDF-1.4'))
  assert.ok(!looksLikePdf('name,address\nvoltage,40001'))
})

test('copilot API calls use same-origin paths', () => {
  // `//api/...` is a protocol-relative URL: the browser sends it to a host named "api".
  const src = readFileSync(new URL('../src/queries/copilot.ts', import.meta.url), 'utf8')
  assert.doesNotMatch(src, /[`'"]\/\/api\//)
  assert.match(src, /`\/api\/v1\/ai\/proposals\/\$\{id\}\/reject`/)
})
