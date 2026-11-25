import { test } from 'node:test'
import assert from 'node:assert/strict'
import { sourceOf } from '../src/commandSource.ts'

test('the server source wins when it is present and known', () => {
  assert.deepEqual(sourceOf('admin', { kind: 'mcp', label: 'MCP (admin)' }), { kind: 'mcp', label: 'MCP (admin)' })
  assert.deepEqual(sourceOf('api_key:3', { kind: 'api_key', label: 'API key line-2' }), { kind: 'api_key', label: 'API key line-2' })
})

test('without a server source the actor string is classified', () => {
  assert.deepEqual(sourceOf('api_key:7'), { kind: 'api_key', label: 'API key #7' })
  assert.deepEqual(sourceOf('flow:pump_rotation'), { kind: 'flow', label: 'flow pump_rotation' })
  assert.deepEqual(sourceOf('operator1'), { kind: 'user', label: 'operator1' })
  assert.deepEqual(sourceOf(''), { kind: 'system', label: 'system' })
  assert.deepEqual(sourceOf(null), { kind: 'system', label: 'system' })
  assert.deepEqual(sourceOf('ana', { kind: 'bogus', label: 'x' }), { kind: 'user', label: 'ana' })
})
