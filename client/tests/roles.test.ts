import { test } from 'node:test'
import assert from 'node:assert/strict'
import { canConfigure, canOperate, isAdmin, roleAbilities } from '../src/roles.ts'

test('viewers and legacy users are read only', () => {
  for (const role of ['viewer', 'user']) {
    assert.deepEqual(roleAbilities(role), { canOperate: false, canConfigure: false, isAdmin: false }, role)
  }
})

test('operators operate but do not configure', () => {
  assert.equal(canOperate('operator'), true)
  assert.equal(canConfigure('operator'), false)
  assert.equal(isAdmin('operator'), false)
})

test('demo and integration operate and configure, without the admin pages', () => {
  for (const role of ['demo', 'integration']) {
    assert.deepEqual(roleAbilities(role), { canOperate: true, canConfigure: true, isAdmin: false }, role)
  }
})

test('admin can do everything', () => {
  assert.deepEqual(roleAbilities('admin'), { canOperate: true, canConfigure: true, isAdmin: true })
})

test('missing or unknown roles get nothing', () => {
  for (const role of [undefined, null, '', 'Admin', 'superuser']) {
    assert.deepEqual(roleAbilities(role), { canOperate: false, canConfigure: false, isAdmin: false }, String(role))
  }
})
