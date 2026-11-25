import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import {
  PDF_GUIDANCE, copilotRequestError, datasheetFileError, generationErrorMessage, looksLikePdf, parseEditedConfig,
} from '../src/copilot/proposalChecks.ts'

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

const refused = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  copilotRequestError(new Response(JSON.stringify(body), { status, headers }))

test('generation errors keep the server code and Retry-After', async () => {
  const limited = await refused(429, { error: 'quota', code: 'ai_rate_limited', retry_after_secs: 20 })
  assert.equal(limited.status, 429)
  assert.equal(limited.code, 'ai_rate_limited')
  assert.equal(limited.retryAfterSecs, 20)
  // Only the header: still read.
  const headerOnly = await refused(429, { error: 'quota', code: 'ai_rate_limited' }, { 'Retry-After': '7' })
  assert.equal(headerOnly.retryAfterSecs, 7)
  const notJson = await copilotRequestError(new Response('Bad gateway', { status: 502 }))
  assert.equal(notJson.message, 'Request failed: 502')
  assert.equal(notJson.code, undefined)
})

test('each generation error code gets a clear message', async () => {
  const msg = async (status: number, body: unknown) => generationErrorMessage(await refused(status, body))
  assert.match(await msg(502, { error: 'x', code: 'ai_truncated' }), /cut off.*Send less at a time/)
  assert.match(await msg(400, { error: 'x', code: 'ai_invalid_api_key' }), /rejected the API key.*AI settings/)
  assert.match(await msg(429, { error: 'x', code: 'ai_rate_limited', retry_after_secs: 30 }), /Try again in 30 s/)
  assert.match(await msg(429, { error: 'x', code: 'ai_rate_limited' }), /Try again later/)
  assert.match(await msg(400, { error: 'x', code: 'ai_not_configured' }), /AI settings/)
  // No code (the PDF refusal): the server's message as is.
  assert.equal(await msg(400, { error: PDF_GUIDANCE }), PDF_GUIDANCE)
  assert.equal(generationErrorMessage(new Error('offline')), 'offline')
})
