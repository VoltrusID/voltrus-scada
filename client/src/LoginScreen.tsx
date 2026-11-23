import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { edition, brandAccent, brandAccentText, type BrandingSettings, type LoginProvidersProps } from './edition'
import { loginHintFrom } from './sharedAccount'

/**
 * The UI font: the brand font when enterprise branding sets --font-ui (Core
 * sets it to the app's system font stack), Inter as a last resort.
 */
const LOGIN_FONT = 'var(--font-ui, Inter, sans-serif)'

/** `accent` at `percent` opacity, for tints of the brand accent (default green). */
function tint(accent: string, percent: number): string {
  return `color-mix(in srgb, ${accent} ${percent}%, transparent)`
}

const INPUT: React.CSSProperties = {
  width: '100%',
  padding: '12px 14px',
  borderRadius: 8,
  border: '1px solid var(--border)',
  background: 'var(--bg)',
  color: 'var(--fg)',
  fontSize: 14,
  fontFamily: LOGIN_FONT,
  outline: 'none',
  transition: 'border-color 0.15s, box-shadow 0.15s',
  boxSizing: 'border-box',
}

type LoginSession = LoginProvidersProps['onLoggedIn']

export function LoginScreen({ onLogin, onExternalLogin, error, branding }: { onLogin: (u: string, p: string) => void; onExternalLogin: LoginSession; error: string; branding: BrandingSettings }) {
  const { t } = useTranslation()
  const [user, setUser] = useState('')
  const [pass, setPass] = useState('')
  const [showPass, setShowPass] = useState(false)
  const [hint, setHint] = useState<string | null>(null)
  useEffect(() => {
    // Plain fetch: a failure only means no hint, not an error toast.
    let live = true
    fetch('/api/v1/auth/providers', { credentials: 'include' })
      .then(r => (r.ok ? r.json() : null))
      .then(body => { if (live) setHint(loginHintFrom(body)) })
      .catch(() => {})
    return () => { live = false }
  }, [])
  const submit = (e: React.FormEvent) => { e.preventDefault(); onLogin(user, pass) }
  const accent = brandAccent(branding)
  const focusRing = (el: HTMLInputElement) => { el.style.borderColor = tint(accent, 50); el.style.boxShadow = `0 0 0 3px ${tint(accent, 10)}` }

  return (
    <div className="viewport-min-height" style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      background: 'var(--bg)',
      fontFamily: LOGIN_FONT,
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
          background: `linear-gradient(135deg, ${tint(accent, 6)} 0%, ${tint(accent, 2)} 100%)`,
          borderBottom: '1px solid var(--border)',
        }}>
          <div style={{
            width: 56, height: 56, borderRadius: 14,
            background: tint(accent, 10), border: `1px solid ${tint(accent, 20)}`,
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
            <label htmlFor="login-username" style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>{t('auth.username')}</label>
            <input
              id="login-username"
              value={user}
              onChange={e => setUser(e.target.value)}
              placeholder={t('auth.usernamePlaceholder')}
              style={INPUT}
              autoComplete="username"
              autoFocus
              aria-required="true"
              onFocus={e => focusRing(e.currentTarget)}
              onBlur={e => { e.currentTarget.style.borderColor = 'var(--border)'; e.currentTarget.style.boxShadow = 'none' }}
            />
          </div>

          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <label htmlFor="login-password" style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>{t('auth.password')}</label>
            <div style={{ position: 'relative' }}>
              <input
                id="login-password"
                type={showPass ? 'text' : 'password'}
                value={pass}
                onChange={e => setPass(e.target.value)}
                placeholder={t('auth.passwordPlaceholder')}
                style={{ ...INPUT, paddingRight: 42 }}
                autoComplete="current-password"
                aria-required="true"
                onFocus={e => focusRing(e.currentTarget)}
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
                aria-label={showPass ? t('auth.hidePassword') : t('auth.showPassword')}
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
              background: accent,
              color: brandAccentText(branding),
              fontSize: 14,
              fontWeight: 600,
              cursor: 'pointer',
              fontFamily: LOGIN_FONT,
              transition: 'opacity 0.15s',
              marginTop: 4,
            }}
            onMouseEnter={e => { e.currentTarget.style.opacity = '0.85' }}
            onMouseLeave={e => { e.currentTarget.style.opacity = '1' }}
          >
            {t('auth.signIn')}
          </button>
          {hint && (
            <p style={{ margin: 0, fontSize: 12, color: 'var(--muted)', textAlign: 'center', overflowWrap: 'anywhere' }}>{hint}</p>
          )}
        </form>
        {edition.LoginProviders && <edition.LoginProviders onLoggedIn={onExternalLogin} />}
      </div>
    </div>
  )
}
