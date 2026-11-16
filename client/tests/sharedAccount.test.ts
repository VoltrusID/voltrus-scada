import { test } from 'node:test'
import assert from 'node:assert/strict'
import { isSharedAccount, loginHintFrom, setSharedAccount } from '../src/sharedAccount.ts'

test('the login hint is read from the providers response as trimmed text', () => {
  assert.equal(loginHintFrom({ providers: [], login_hint: '  Demo: guest / voltrus-demo ' }), 'Demo: guest / voltrus-demo')
  // Markup stays text: the login page renders it as a text node.
  assert.equal(loginHintFrom({ login_hint: '<b>guest</b>' }), '<b>guest</b>')
})

test('a missing, blank or non-string hint shows nothing', () => {
  for (const body of [null, undefined, 'x', {}, { login_hint: null }, { login_hint: '   ' }, { login_hint: 42 }, { login_hint: ['a'] }]) {
    assert.equal(loginHintFrom(body), null, JSON.stringify(body))
  }
})

test('an over-long hint is cut to the server limit', () => {
  assert.equal(loginHintFrom({ login_hint: 'x'.repeat(500) })!.length, 200)
})

test('the shared-account flag follows the session', () => {
  assert.equal(isSharedAccount(), false)
  setSharedAccount(true)
  assert.equal(isSharedAccount(), true)
  setSharedAccount(false)
  assert.equal(isSharedAccount(), false)
})
