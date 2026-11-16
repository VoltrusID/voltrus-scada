import test from 'node:test'
import assert from 'node:assert/strict'
import { AI_PROVIDERS, baseUrlError, buildAiSettingsBody, keyWillBeRemoved } from '../src/copilot/aiSettings.ts'

const empty = { provider: '', apiKey: '', model: '', baseUrl: '', clearKey: false }

test('the provider list offers the OpenAI-compatible option', () => {
  assert.ok(AI_PROVIDERS.some(p => p.value === 'byok_openai_compatible'))
})

test('base URLs must be http(s) with a host and no credentials or query', () => {
  assert.equal(baseUrlError('http://localhost:11434/v1'), null)
  assert.equal(baseUrlError(' https://openrouter.ai/api/v1/ '), null)
  assert.ok(baseUrlError(''))
  assert.ok(baseUrlError('localhost:11434/v1'))
  assert.ok(baseUrlError('ftp://host/v1'))
  assert.ok(baseUrlError('https://u:p@host/v1'))
  assert.ok(baseUrlError('https://host/v1?key=1'))
})

test('an Ollama setup needs no key', () => {
  const r = buildAiSettingsBody(
    { ...empty, provider: 'byok_openai_compatible', baseUrl: ' http://localhost:11434/v1 ', model: 'llama3.2' },
    { provider: 'byok_openai' },
  )
  assert.deepEqual(r, { body: { provider: 'byok_openai_compatible', base_url: 'http://localhost:11434/v1', default_model: 'llama3.2' } })
})

test('the OpenAI-compatible provider needs a base URL and a model, stored or entered', () => {
  const compat = { ...empty, provider: 'byok_openai_compatible' }
  assert.ok('error' in buildAiSettingsBody({ ...compat, model: 'm' }, { provider: 'byok_openai' }))
  assert.ok('error' in buildAiSettingsBody({ ...compat, baseUrl: 'http://h/v1' }, { provider: 'byok_openai' }))
  assert.ok('error' in buildAiSettingsBody({ ...compat, baseUrl: 'nope', model: 'm' }, { provider: 'byok_openai' }))
  // Already stored: only the changed field is sent.
  const stored = { provider: 'byok_openai_compatible', base_url: 'http://h/v1', default_model: 'm' }
  assert.deepEqual(buildAiSettingsBody({ ...empty, apiKey: 'sk-1' }, stored), {
    body: { provider: 'byok_openai_compatible', api_key: 'sk-1' },
  })
})

test('other providers do not send a base URL', () => {
  const r = buildAiSettingsBody({ ...empty, provider: 'byok_anthropic', baseUrl: 'http://h/v1' }, { provider: 'byok_openai' })
  assert.deepEqual(r, { body: { provider: 'byok_anthropic' } })
})

test('the form warns when saving will remove the stored key', () => {
  const stored = { provider: 'byok_openai' }
  assert.equal(keyWillBeRemoved({ ...empty, provider: 'byok_openai_compatible' }, stored, true), true)
  assert.equal(keyWillBeRemoved({ ...empty, provider: 'byok_openai_compatible', apiKey: 'k' }, stored, true), false)
  assert.equal(keyWillBeRemoved({ ...empty, provider: 'byok_openai' }, stored, true), false)
  assert.equal(keyWillBeRemoved({ ...empty, provider: 'byok_anthropic' }, stored, false), false)
  assert.equal(keyWillBeRemoved({ ...empty, clearKey: true }, stored, true), true)
  assert.deepEqual(buildAiSettingsBody({ ...empty, clearKey: true }, stored), { body: { provider: 'byok_openai', clear_api_key: true } })
})
