import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  bodyWithSignature, completeSignature, isSignatureCancelled, parseSignatureAnswer, signedRequestInit,
  validateSignatureForm, SignatureCancelledError, type Signature, type SignatureRequest,
} from '../src/signature/logic.ts'
import {
  answerSignaturePrompt, canPromptForSignature, cancelAllSignaturePrompts, currentSignaturePrompt,
  requestSignature, subscribeSignaturePrompt,
} from '../src/signature/prompt.ts'

const request = { action: 'tag_write', meaning: 'I approve writing 42 to TIC-101' }
const sig: Signature = { password: 'pw', meaning: request.meaning, reason: 'batch start' }
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

test('428 signature_required is a prompt', () => {
  assert.deepEqual(
    parseSignatureAnswer(428, { error: 'x', code: 'signature_required', signature_required: request }),
    { kind: 'required', request },
  )
})

test('403 answers are told apart by code', () => {
  assert.deepEqual(
    parseSignatureAnswer(403, { error: 'Wrong password', code: 'signature_invalid', signature_required: request }),
    { kind: 'invalid', request, error: 'Wrong password' },
  )
  assert.equal(parseSignatureAnswer(403, { code: 'account_locked', error: 'Locked' })?.kind, 'locked')
  assert.equal(parseSignatureAnswer(403, { code: 'signature_not_possible' })?.kind, 'not_possible')
  // An ordinary permission error is not a signature answer.
  assert.equal(parseSignatureAnswer(403, { error: 'Forbidden' }), null)
  assert.equal(parseSignatureAnswer(400, { code: 'signature_required', signature_required: request }), null)
  assert.equal(parseSignatureAnswer(428, null), null)
  assert.equal(parseSignatureAnswer(428, { code: 'signature_required' }), null)
})

test('the signature is merged into the JSON body, or becomes the body', () => {
  assert.deepEqual(JSON.parse(bodyWithSignature(JSON.stringify({ value: 42, policy: 'immediate' }), sig)!), {
    value: 42, policy: 'immediate', signature: sig,
  })
  assert.deepEqual(JSON.parse(bodyWithSignature(undefined, sig)!), { signature: sig })
  assert.deepEqual(JSON.parse(bodyWithSignature(null, sig)!), { signature: sig })
  assert.equal(bodyWithSignature('[1,2]', sig), null)
  assert.equal(bodyWithSignature('not json', sig), null)
  assert.equal(bodyWithSignature(new URLSearchParams('a=1'), sig), null)
})

test('a signed request without a body gets a JSON content type', () => {
  const init = signedRequestInit({ method: 'POST' }, sig)!
  assert.equal(init.method, 'POST')
  assert.equal(new Headers(init.headers).get('Content-Type'), 'application/json')
  const kept = signedRequestInit({ method: 'PATCH', headers: { 'Content-Type': 'application/json', 'X-A': '1' }, body: '{"writable":true}' }, sig)!
  assert.equal(new Headers(kept.headers).get('X-A'), '1')
  assert.deepEqual(JSON.parse(kept.body as string), { writable: true, signature: sig })
})

test('the form needs a password and a non-blank reason', () => {
  assert.ok(validateSignatureForm('', 'r'))
  assert.ok(validateSignatureForm('pw', '   '))
  assert.equal(validateSignatureForm('pw', 'r'), null)
})

test('a required signature is prompted for and the request resent with it', async () => {
  const sent: RequestInit[] = []
  const prompts: (string | undefined)[] = []
  const res = await completeSignature(json(428, { code: 'signature_required', signature_required: request }), { method: 'POST' }, {
    send: async init => { sent.push(init); return json(200, { ok: true }) },
    prompt: async (_r: SignatureRequest, error?: string) => { prompts.push(error); return sig },
  })
  assert.equal(res.status, 200)
  assert.equal(sent.length, 1)
  assert.deepEqual(JSON.parse(sent[0].body as string), { signature: sig })
  assert.deepEqual(prompts, [undefined])
})

test('a wrong password prompts again with the error', async () => {
  const answers = [
    json(403, { code: 'signature_invalid', error: 'Wrong password', signature_required: request }),
    json(200, {}),
  ]
  const prompts: (string | undefined)[] = []
  const res = await completeSignature(json(428, { code: 'signature_required', signature_required: request }), undefined, {
    send: async () => answers.shift()!,
    prompt: async (_r, error) => { prompts.push(error); return sig },
  })
  assert.equal(res.status, 200)
  assert.deepEqual(prompts, [undefined, 'Wrong password'])
})

test('a locked account ends the attempt with the server answer', async () => {
  let sends = 0
  const res = await completeSignature(json(428, { code: 'signature_required', signature_required: request }), undefined, {
    send: async () => { sends++; return json(403, { code: 'account_locked', error: 'Locked' }) },
    prompt: async () => sig,
  })
  assert.equal(res.status, 403)
  assert.equal(sends, 1)
  assert.equal((await res.json()).error, 'Locked')
})

test('cancelling throws a quiet cancellation error', async () => {
  await assert.rejects(
    completeSignature(json(428, { code: 'signature_required', signature_required: request }), undefined, {
      send: async () => { throw new Error('must not send') },
      prompt: async () => null,
    }),
    (err: unknown) => isSignatureCancelled(err),
  )
  assert.ok(isSignatureCancelled(new SignatureCancelledError()))
  assert.ok(!isSignatureCancelled(new Error('Write failed')))
})

test('other responses pass through untouched', async () => {
  const res = await completeSignature(json(400, { error: 'Bad' }), undefined, {
    send: async () => { throw new Error('must not send') },
    prompt: async () => { throw new Error('must not prompt') },
  })
  assert.equal(res.status, 400)
})

test('the shared prompt answers requests one at a time', async () => {
  assert.equal(canPromptForSignature(), false)
  const unsubscribe = subscribeSignaturePrompt(() => {})
  assert.equal(canPromptForSignature(), true)
  const first = requestSignature(request)
  const second = requestSignature({ action: 'alarm_ack', meaning: 'Ack' })
  assert.equal(currentSignaturePrompt()?.request, request)
  answerSignaturePrompt(sig)
  assert.deepEqual(await first, sig)
  assert.equal(currentSignaturePrompt()?.request.action, 'alarm_ack')
  cancelAllSignaturePrompts()
  assert.equal(await second, null)
  assert.equal(currentSignaturePrompt(), null)
  unsubscribe()
})
