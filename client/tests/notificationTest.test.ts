import test from 'node:test'
import assert from 'node:assert/strict'
import { summarizeTestOutcome } from '../src/notificationTest.ts'

test('a test with every target sent is a success', () => {
  assert.deepEqual(summarizeTestOutcome({ channel: 'telegram', results: [{ target: '1', status: 'sent' }] }), {
    success: true,
    message: 'Test notification sent',
  })
})

test('failures name the target and the reason', () => {
  const s = summarizeTestOutcome({
    channel: 'whatsapp',
    results: [
      { target: '+62811', status: 'sent' },
      { target: '+62822', status: 'error', error: 'WhatsApp error 131030' },
    ],
  })
  assert.equal(s.success, false)
  assert.equal(s.message, '1 sent, 1 failed. +62822: WhatsApp error 131030')
})

test('an empty result list is not a success', () => {
  assert.equal(summarizeTestOutcome({ channel: 'email', results: [] }).success, false)
})
