// Electronic signatures (21 CFR Part 11), React-free so it can be tested.
//
// An action the server wants signed is answered with 428 and
//   { code: 'signature_required', signature_required: { action, meaning } }.
// The client asks the user for their password and a reason and sends the same
// request again with `signature: { password, meaning, reason }` added to the
// JSON body. A wrong password comes back as 403 `signature_invalid` (ask
// again); 403 `account_locked` and `signature_not_possible` end the attempt.

export interface SignatureRequest {
  action: string
  /** Text the user signs; sent back unchanged. */
  meaning: string
}

export interface Signature {
  password: string
  meaning: string
  reason: string
}

export type SignatureAnswer =
  | { kind: 'required'; request: SignatureRequest }
  | { kind: 'invalid'; request: SignatureRequest; error: string }
  | { kind: 'locked'; error: string }
  | { kind: 'not_possible'; error: string }

/** Thrown when the user closes the signature prompt: the action is abandoned and nothing should be reported. */
export class SignatureCancelledError extends Error {
  constructor() {
    super('Signature cancelled')
    this.name = 'SignatureCancelledError'
  }
}

export function isSignatureCancelled(err: unknown): boolean {
  return err instanceof SignatureCancelledError || (err instanceof Error && err.name === 'SignatureCancelledError')
}

/** Statuses worth reading the body of; everything else is never a signature answer. */
export function mayBeSignatureAnswer(status: number): boolean {
  return status === 428 || status === 403
}

function readRequest(v: unknown): SignatureRequest | null {
  if (!v || typeof v !== 'object') return null
  const { action, meaning } = v as Record<string, unknown>
  if (typeof meaning !== 'string') return null
  return { action: typeof action === 'string' ? action : '', meaning }
}

/** Classify a response (status and parsed JSON body) as a signature answer, or null when it is not one. */
export function parseSignatureAnswer(status: number, body: unknown): SignatureAnswer | null {
  if (!mayBeSignatureAnswer(status) || !body || typeof body !== 'object') return null
  const b = body as Record<string, unknown>
  const error = typeof b.error === 'string' && b.error ? b.error : ''
  const request = readRequest(b.signature_required)
  if (status === 428) {
    return request && (b.code === undefined || b.code === 'signature_required') ? { kind: 'required', request } : null
  }
  switch (b.code) {
    case 'signature_invalid':
      return request ? { kind: 'invalid', request, error: error || 'Wrong password' } : null
    case 'account_locked':
      return { kind: 'locked', error: error || 'Too many wrong passwords: your account is locked' }
    case 'signature_not_possible':
      return { kind: 'not_possible', error: error || 'This account cannot sign electronically' }
    default:
      return null
  }
}

/** Trimmed form values, or the message to show instead. */
export function validateSignatureForm(password: string, reason: string): string | null {
  if (!password) return 'Enter your password'
  if (!reason.trim()) return 'Enter the reason'
  return null
}

/**
 * The request body with the signature merged in, as a JSON string. A missing
 * body becomes `{signature}`. Returns null when the body is not a JSON object
 * (form data, a JSON array), which cannot carry a signature.
 */
export function bodyWithSignature(body: unknown, signature: Signature): string | null {
  let base: Record<string, unknown> = {}
  if (body != null && body !== '') {
    if (typeof body !== 'string') return null
    let parsed: unknown
    try {
      parsed = JSON.parse(body)
    } catch {
      return null
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
    base = parsed as Record<string, unknown>
  }
  return JSON.stringify({ ...base, signature })
}

/** The init for the signed retry, or null when the request cannot be signed. */
export function signedRequestInit(init: RequestInit | undefined, signature: Signature): RequestInit | null {
  const body = bodyWithSignature(init?.body, signature)
  if (body === null) return null
  const headers = new Headers(init?.headers)
  if (!headers.has('Content-Type')) headers.set('Content-Type', 'application/json')
  return { ...init, headers, body }
}

export interface SignatureFlow {
  /** Sends the request again with the given init. */
  send: (init: RequestInit) => Promise<Response>
  /** Asks the user to sign; resolves null when they cancel. `error` is set after a wrong password. */
  prompt: (request: SignatureRequest, error?: string) => Promise<Signature | null>
}

/**
 * Given the first response to a request, prompt and resend while the server
 * asks for a signature. Returns the final response (which may still be an
 * error, e.g. account locked, for the caller to report). Throws
 * SignatureCancelledError when the user cancels.
 */
export async function completeSignature(first: Response, init: RequestInit | undefined, flow: SignatureFlow): Promise<Response> {
  let res = first
  for (;;) {
    if (!mayBeSignatureAnswer(res.status)) return res
    const body = await res.clone().json().catch(() => null)
    const answer = parseSignatureAnswer(res.status, body)
    if (!answer || answer.kind === 'locked' || answer.kind === 'not_possible') return res
    const signature = await flow.prompt(answer.request, answer.kind === 'invalid' ? answer.error : undefined)
    if (!signature) throw new SignatureCancelledError()
    const signed = signedRequestInit(init, signature)
    if (!signed) return res
    res = await flow.send(signed)
  }
}
