// The single signature prompt shared by every request. React-free: the dialog
// (SignatureDialog.tsx, mounted once in the app shell) subscribes to it.
import type { Signature, SignatureRequest } from './logic'

export interface PendingSignature {
  request: SignatureRequest
  /** Set when the previous attempt had a wrong password. */
  error?: string
  resolve: (signature: Signature | null) => void
}

const queue: PendingSignature[] = []
const listeners = new Set<() => void>()

function notify() {
  listeners.forEach(fn => fn())
}

/** The prompt to show now, if any. Requests arriving while one is open wait their turn. */
export function currentSignaturePrompt(): PendingSignature | null {
  return queue[0] ?? null
}

export function subscribeSignaturePrompt(fn: () => void): () => void {
  listeners.add(fn)
  return () => { listeners.delete(fn) }
}

/** Whether a dialog is mounted to answer prompts. Without one, a 428 is reported as an ordinary error. */
export function canPromptForSignature(): boolean {
  return listeners.size > 0
}

export function requestSignature(request: SignatureRequest, error?: string): Promise<Signature | null> {
  return new Promise(resolve => {
    queue.push({ request, error, resolve })
    notify()
  })
}

/** Called by the dialog: `null` cancels. */
export function answerSignaturePrompt(signature: Signature | null): void {
  const pending = queue.shift()
  if (!pending) return
  pending.resolve(signature)
  notify()
}

/** Cancel every waiting prompt, e.g. on sign-out. */
export function cancelAllSignaturePrompts(): void {
  while (queue.length) queue.shift()!.resolve(null)
  notify()
}
