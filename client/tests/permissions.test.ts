import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { ALL_PERMISSIONS } from '../src/permissions.ts'

// The server refuses a role holding any name outside `ALL_PERMISSIONS`
// (src/models/rbac_types.rs); the role editor must offer exactly those.
const rust = readFileSync(new URL('../../src/models/rbac_types.rs', import.meta.url), 'utf8')

test('the role editor offers exactly the permissions the server accepts', () => {
  const values = new Map<string, string>()
  for (const m of rust.matchAll(/pub const (PERM_\w+): &str = "([^"]+)";/g)) values.set(m[1], m[2])
  const list = rust.match(/pub const ALL_PERMISSIONS: &\[&str\] = &\[([^\]]*)\];/)
  assert.ok(list, 'ALL_PERMISSIONS found in rbac_types.rs')
  const server = [...list[1].matchAll(/PERM_\w+/g)].map(m => values.get(m[0]))
  assert.equal(server.length, 24)
  assert.deepEqual([...ALL_PERMISSIONS], server)
})
