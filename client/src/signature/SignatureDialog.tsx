import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { useModalFocus } from '../components/Dialog'
import { inputStyle, labelStyle, labelTextStyle } from '../styles'
import { validateSignatureForm } from './logic'
import { answerSignaturePrompt, currentSignaturePrompt, subscribeSignaturePrompt } from './prompt'

/**
 * The electronic signature prompt. Mount once (the app shell does); any
 * request that the server wants signed opens it through fetchOk.
 */
export function SignatureDialog({ username }: { username?: string }) {
  const pending = useSyncExternalStore(subscribeSignaturePrompt, currentSignaturePrompt)
  const [password, setPassword] = useState('')
  const [reason, setReason] = useState('')
  const [formError, setFormError] = useState('')

  // Every prompt starts with an empty password; a retry after a wrong password keeps the reason.
  useEffect(() => {
    setPassword('')
    setFormError('')
    if (pending && !pending.error) setReason('')
  }, [pending])

  if (!pending) return null

  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    const err = validateSignatureForm(password, reason)
    if (err) { setFormError(err); return }
    const signature = { password, meaning: pending.request.meaning, reason: reason.trim() }
    setPassword('')
    answerSignaturePrompt(signature)
  }

  return (
    <ModalBackdrop>
      <form
        role="dialog" aria-modal="true" aria-labelledby="signature-title" aria-describedby="signature-signer"
        onSubmit={submit}
        style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 420, maxWidth: 'calc(100vw - 32px)', padding: 20, display: 'flex', flexDirection: 'column', gap: 12 }}
      >
        <div id="signature-title" style={{ fontSize: 15, fontWeight: 600 }}>Electronic signature required</div>
        <div id="signature-signer" style={{ fontSize: 13, color: 'var(--text-dim)' }}>
          {username ? <>Signing as <strong style={{ color: 'var(--fg)' }}>{username}</strong>. Your password confirms it is you.</> : 'Your password confirms it is you.'}
        </div>
        <div>
          <div style={{ ...labelTextStyle, marginBottom: 4 }}>You are signing</div>
          <div style={{ padding: '8px 12px', borderRadius: 6, background: 'var(--bg)', border: '1px solid var(--border)', fontSize: 13, whiteSpace: 'pre-wrap' }}>
            {pending.request.meaning}
          </div>
        </div>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Your password</span>
          <input
            type="password" autoComplete="current-password" autoFocus
            value={password} onChange={e => { setPassword(e.target.value); setFormError('') }}
            style={inputStyle}
          />
        </label>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Reason</span>
          <input
            value={reason} onChange={e => { setReason(e.target.value); setFormError('') }}
            placeholder="Why you are doing this"
            style={inputStyle}
          />
        </label>
        {(formError || pending.error) && (
          <div role="alert" style={{ fontSize: 12, color: 'var(--crit)' }}>{formError || pending.error}</div>
        )}
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 4 }}>
          <button type="button" onClick={() => answerSignaturePrompt(null)} style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', color: 'var(--fg)', cursor: 'pointer', fontSize: 13 }}>Cancel</button>
          <button type="submit" style={{ padding: '7px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'var(--cyan)', color: '#000', cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>Sign</button>
        </div>
      </form>
    </ModalBackdrop>
  )
}

/** Mounted while a prompt is open: traps focus in it and gives focus back when it closes. */
function ModalBackdrop({ children }: { children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  useModalFocus(ref, () => answerSignaturePrompt(null))
  return (
    <div ref={ref} style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 1000 }}>
      {children}
    </div>
  )
}
