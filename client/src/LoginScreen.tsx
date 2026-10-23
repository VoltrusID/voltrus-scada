import { useState } from 'react'
import { edition, type BrandingSettings, type LoginProvidersProps } from './edition'

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

type LoginSession = LoginProvidersProps['onLoggedIn']

export function LoginScreen({ onLogin, onExternalLogin, error, branding }: { onLogin: (u: string, p: string) => void; onExternalLogin: LoginSession; error: string; branding: BrandingSettings }) {
  const [user, setUser] = useState('')
  const [pass, setPass] = useState('')
  const [showPass, setShowPass] = useState(false)
  const submit = (e: React.FormEvent) => { e.preventDefault(); onLogin(user, pass) }

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
          padding: '36px 36px 28px',
          textAlign: 'center',
          background: 'linear-gradient(135deg, rgba(34,197,94,.06) 0%, rgba(34,197,94,.02) 100%)',
          borderBottom: '1px solid var(--border)',
        }}>
          <div style={{
            width: 56, height: 56, borderRadius: 14,
            background: 'rgba(34,197,94,.1)', border: '1px solid rgba(34,197,94,.2)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            margin: '0 auto 16px',
          }}>
            <img src={branding.logo || '/favicon-32.png'} alt={branding.title + ' logo'} style={{ width: 32, height: 32, borderRadius: 6, objectFit: 'contain' }} />
          </div>
          <h1 style={{ margin: '0 0 4px', fontSize: 20, fontWeight: 700, color: 'var(--fg)' }}>
            {branding.title}
          </h1>
          {branding.subtitle && (
            <p style={{ margin: 0, fontSize: 13, color: 'var(--muted)' }}>{branding.subtitle}</p>
          )}
        </div>

        {/* Form */}
        <form onSubmit={submit} style={{ padding: '28px 36px 32px', display: 'flex', flexDirection: 'column', gap: 16 }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <label htmlFor="login-username" style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>Username</label>
            <input
              id="login-username"
              value={user}
              onChange={e => setUser(e.target.value)}
              placeholder="Enter your username"
              style={INPUT}
              autoComplete="username"
              autoFocus
              aria-required="true"
              onFocus={e => { e.currentTarget.style.borderColor = 'rgba(34,197,94,.5)'; e.currentTarget.style.boxShadow = '0 0 0 3px rgba(34,197,94,.1)' }}
              onBlur={e => { e.currentTarget.style.borderColor = 'var(--border)'; e.currentTarget.style.boxShadow = 'none' }}
            />
          </div>

          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <label htmlFor="login-password" style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>Password</label>
            <div style={{ position: 'relative' }}>
              <input
                id="login-password"
                type={showPass ? 'text' : 'password'}
                value={pass}
                onChange={e => setPass(e.target.value)}
                placeholder="Enter your password"
                style={{ ...INPUT, paddingRight: 42 }}
                autoComplete="current-password"
                aria-required="true"
                onFocus={e => { e.currentTarget.style.borderColor = 'rgba(34,197,94,.5)'; e.currentTarget.style.boxShadow = '0 0 0 3px rgba(34,197,94,.1)' }}
                onBlur={e => { e.currentTarget.style.borderColor = 'var(--border)'; e.currentTarget.style.boxShadow = 'none' }}
              />
              <button
                type="button"
                onClick={() => setShowPass(v => !v)}
                style={{
                  position: 'absolute', right: 1, top: 1, bottom: 1, width: 40,
                  background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}
                tabIndex={-1}
                aria-label={showPass ? 'Hide password' : 'Show password'}
              >
                {showPass ? (
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94"/><path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
                ) : (
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                )}
              </button>
            </div>
          </div>

          {error && (
            <div role="alert" style={{
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
            style={{
              padding: '12px 20px',
              borderRadius: 8,
              border: 'none',
              background: '#22c55e',
              color: '#fff',
              fontSize: 14,
              fontWeight: 600,
              cursor: 'pointer',
              fontFamily: 'Inter, sans-serif',
              transition: 'opacity 0.15s',
              marginTop: 4,
            }}
            onMouseEnter={e => { e.currentTarget.style.opacity = '0.85' }}
            onMouseLeave={e => { e.currentTarget.style.opacity = '1' }}
          >
            Sign In
          </button>
        </form>
        {edition.LoginProviders && <edition.LoginProviders onLoggedIn={onExternalLogin} />}
      </div>
    </div>
  )
}
