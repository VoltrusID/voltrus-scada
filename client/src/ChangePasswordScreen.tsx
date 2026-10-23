import { useState } from 'react'

const INPUT: React.CSSProperties = {
  width: '100%',
  padding: '12px 14px',
  borderRadius: 8,
  border: '1px solid var(--border)',
  background: 'var(--bg)',
  color: 'var(--fg)',
  fontSize: 14,
  fontFamily: 'Inter, sans-serif',
  outline: 'none',
  transition: 'border-color 0.15s, box-shadow 0.15s',
  boxSizing: 'border-box',
}

export function ChangePasswordScreen({ onSubmit }: { onSubmit: (oldPassword: string, newPassword: string) => Promise<void> }) {
  const [oldPw, setOldPw] = useState('')
  const [pw, setPw] = useState('')
  const [pw2, setPw2] = useState('')
  const [showPw, setShowPw] = useState(false)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!oldPw) { setError('Current password is required'); return }
    if (pw !== pw2) { setError('Passwords do not match'); return }
    if (pw.length < 8) { setError('Password must be at least 8 characters'); return }
    setLoading(true)
    setError('')
    try {
      await onSubmit(oldPw, pw)
    } catch (err: any) {
      setError(err.message || 'Failed to change password')
    } finally {
      setLoading(false)
    }
  }

  const strength = pw.length === 0 ? 0 : pw.length < 8 ? 1 : pw.length < 12 ? 2 : 3
  const strengthLabel = ['', 'Weak', 'Good', 'Strong'][strength]
  const strengthColor = ['', '#ef4444', '#f59e0b', '#22c55e'][strength]

  return (
    <div style={{
      height: '100vh',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      background: 'var(--bg)',
      fontFamily: 'Inter, sans-serif',
      padding: 20,
    }}>
      <div style={{
        background: 'var(--card)',
        border: '1px solid var(--border)',
        borderRadius: 16,
        width: 400,
        maxWidth: '100%',
        overflow: 'hidden',
        boxShadow: '0 8px 40px rgba(0,0,0,0.25)',
      }}>
        {/* Header */}
        <div style={{
          padding: '32px 36px 24px',
          borderBottom: '1px solid var(--border)',
          background: 'linear-gradient(135deg, rgba(245,158,11,.06) 0%, rgba(245,158,11,.02) 100%)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 10 }}>
            <div style={{
              width: 44, height: 44, borderRadius: 12,
              background: 'rgba(245,158,11,.1)', border: '1px solid rgba(245,158,11,.2)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" strokeWidth="2">
                <rect x="3" y="11" width="18" height="11" rx="2" ry="2"/>
                <path d="M7 11V7a5 5 0 0 1 10 0v4"/>
              </svg>
            </div>
            <h1 style={{ margin: 0, fontSize: 20, fontWeight: 700, color: 'var(--fg)' }}>Change Your Password</h1>
          </div>
          <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)', lineHeight: 1.5 }}>
            You must set a new password before continuing.
          </p>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} style={{ padding: '28px 36px 32px', display: 'flex', flexDirection: 'column', gap: 16 }}>
          {/* Current password */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>Current Password</span>
            <input
              type="password"
              value={oldPw}
              onChange={e => { setOldPw(e.target.value); setError('') }}
              placeholder="Enter current password"
              required
              style={INPUT}
              autoFocus
              onFocus={e => { e.currentTarget.style.borderColor = 'rgba(245,158,11,.5)'; e.currentTarget.style.boxShadow = '0 0 0 3px rgba(245,158,11,.1)' }}
              onBlur={e => { e.currentTarget.style.borderColor = 'var(--border)'; e.currentTarget.style.boxShadow = 'none' }}
            />
          </div>

          {/* New password */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>New Password</span>
            <div style={{ position: 'relative' }}>
              <input
                type={showPw ? 'text' : 'password'}
                value={pw}
                onChange={e => { setPw(e.target.value); setError('') }}
                placeholder="Min 8 characters"
                required
                minLength={8}
                style={{ ...INPUT, paddingRight: 42 }}
                autoFocus
                onFocus={e => { e.currentTarget.style.borderColor = 'rgba(245,158,11,.5)'; e.currentTarget.style.boxShadow = '0 0 0 3px rgba(245,158,11,.1)' }}
                onBlur={e => { e.currentTarget.style.borderColor = 'var(--border)'; e.currentTarget.style.boxShadow = 'none' }}
              />
              <button
                type="button"
                onClick={() => setShowPw(v => !v)}
                tabIndex={-1}
                style={{
                  position: 'absolute', right: 1, top: 1, bottom: 1, width: 40,
                  background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}
              >
                {showPw ? (
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94"/><path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
                ) : (
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                )}
              </button>
            </div>
            {pw.length > 0 && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 2 }}>
                <div style={{ display: 'flex', gap: 3, flex: 1 }}>
                  {[1, 2, 3].map(i => (
                    <div key={i} style={{
                      height: 3,
                      flex: 1,
                      borderRadius: 2,
                      background: i <= strength ? strengthColor : 'var(--border)',
                      transition: 'background 0.2s',
                    }} />
                  ))}
                </div>
                <span style={{ fontSize: 11, fontWeight: 600, color: strengthColor }}>{strengthLabel}</span>
              </div>
            )}
          </div>

          {/* Confirm password */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>Confirm Password</span>
            <input
              type="password"
              value={pw2}
              onChange={e => { setPw2(e.target.value); setError('') }}
              placeholder="Re-enter your password"
              required
              style={INPUT}
              onFocus={e => { e.currentTarget.style.borderColor = 'rgba(245,158,11,.5)'; e.currentTarget.style.boxShadow = '0 0 0 3px rgba(245,158,11,.1)' }}
              onBlur={e => { e.currentTarget.style.borderColor = 'var(--border)'; e.currentTarget.style.boxShadow = 'none' }}
            />
            {pw2.length > 0 && pw !== pw2 && (
              <span style={{ fontSize: 11, color: 'var(--crit)', marginTop: 2 }}>Passwords do not match</span>
            )}
          </div>

          {error && (
            <div style={{
              fontSize: 13,
              color: 'var(--crit)',
              padding: '10px 14px',
              background: 'rgba(239,68,68,.1)',
              border: '1px solid rgba(239,68,68,.2)',
              borderRadius: 8,
              display: 'flex',
              alignItems: 'center',
              gap: 8,
            }}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
              {error}
            </div>
          )}

          <button
            type="submit"
            disabled={loading || !oldPw || pw !== pw2 || pw.length < 8}
            style={{
              padding: '12px 20px',
              borderRadius: 8,
              border: 'none',
              background: loading || !oldPw || pw !== pw2 || pw.length < 8 ? 'var(--border)' : '#22c55e',
              color: loading || !oldPw || pw !== pw2 || pw.length < 8 ? 'var(--muted)' : '#fff',
              fontSize: 14,
              fontWeight: 600,
              cursor: loading || !oldPw || pw !== pw2 || pw.length < 8 ? 'not-allowed' : 'pointer',
              fontFamily: 'Inter, sans-serif',
              transition: 'opacity 0.15s',
              marginTop: 4,
              opacity: loading ? 0.6 : 1,
            }}
          >
            {loading ? 'Updating...' : 'Update Password'}
          </button>
        </form>
      </div>
    </div>
  )
}
