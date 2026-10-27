import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { duplicateName } from '../src/duplicateName.ts'

// The default ("permissive") naming rule, from src/api/naming_convention.rs.
const rust = readFileSync(new URL('../../src/api/naming_convention.rs', import.meta.url), 'utf8')
const permissive = rust.match(/"permissive",\s*"[^"]*",\s*r"([^"]+)"/)

test('a duplicated tag name passes the default naming rule', () => {
  assert.ok(permissive, 'permissive pattern found in naming_convention.rs')
  const rule = new RegExp(permissive[1])
  for (const name of ['Pump 1', 'Temp-A.2', 'flow_rate']) {
    assert.equal(duplicateName(name), `${name} copy`)
    assert.ok(rule.test(duplicateName(name)), duplicateName(name))
  }
  assert.ok(!rule.test('Pump 1 (Copy)'), 'the old name was refused')
})
