import test from 'node:test'
import assert from 'node:assert/strict'
import { disableRefused, toggleDisabledBody } from '../src/userStatus.ts'

const users = [
  { id: 1, username: 'admin', role: 'admin' },
  { id: 2, username: 'ops', role: 'operator' },
  { id: 3, username: 'oidc:ana', role: 'viewer', disabled: true },
]

test('an admin may disable another user but not themselves', () => {
  assert.equal(disableRefused('admin', users[1], users), null)
  assert.match(disableRefused('admin', users[0], users) ?? '', /yourself/)
})

test('the last enabled admin cannot be disabled', () => {
  const two = [...users, { id: 4, username: 'boss', role: 'admin' }]
  assert.equal(disableRefused('boss', two[0], two), null)
  const otherDisabled = [...users, { id: 4, username: 'boss', role: 'admin', disabled: true }]
  assert.match(disableRefused('ops', otherDisabled[0], otherDisabled) ?? '', /last enabled admin/)
})

test('enabling is always allowed', () => {
  assert.equal(disableRefused('admin', users[2], users), null)
  assert.equal(disableRefused('oidc:ana', users[2], users), null)
})

test('the toggle body flips the state', () => {
  assert.deepEqual(toggleDisabledBody(users[1]), { disabled: true })
  assert.deepEqual(toggleDisabledBody(users[2]), { disabled: false })
})
