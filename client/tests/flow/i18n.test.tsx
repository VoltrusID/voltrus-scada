import { afterEach, describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import i18n from 'i18next'
import id from '../../public/locales/id/translation.json'
import de from '../../public/locales/de/translation.json'
import { LoginScreen } from '../../src/LoginScreen'
import { ConfirmDialog } from '../../src/ScreenDialog'
import { sessionEndMessage } from '../../src/session'
import { DEFAULT_BRANDING } from '../../src/edition'

i18n.addResourceBundle('id', 'translation', id)
i18n.addResourceBundle('de', 'translation', de)

describe('translated screens', () => {
  afterEach(async () => { await i18n.changeLanguage('en') })

  it('the login screen follows the selected language', async () => {
    await i18n.changeLanguage('id')
    render(<LoginScreen onLogin={() => {}} onExternalLogin={() => {}} error="" branding={DEFAULT_BRANDING} />)
    expect(screen.getByRole('button', { name: 'Masuk' })).toBeTruthy()
    expect(screen.getByLabelText('Kata Sandi')).toBeTruthy()
  })

  it('common dialogs are translated', async () => {
    await i18n.changeLanguage('de')
    render(<ConfirmDialog title="X" message="m" onConfirm={() => {}} onClose={() => {}} />)
    expect(screen.getByRole('button', { name: 'Abbrechen' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Löschen' })).toBeTruthy()
  })

  it('session-ended messages use the language and its plural rules', async () => {
    const t = (key: string, opts: { defaultValue: string; count?: number }) => String(i18n.t(key, opts))
    expect(sessionEndMessage('session_idle', 60, t)).toBe('You were signed out after 1 minute of inactivity.')
    expect(sessionEndMessage('session_idle', 900, t)).toBe('You were signed out after 15 minutes of inactivity.')
    await i18n.changeLanguage('id')
    expect(sessionEndMessage('session_idle', 900, t)).toBe('Anda dikeluarkan setelah 15 menit tidak ada aktivitas.')
    expect(sessionEndMessage('session_ended', null, t)).toBe('Sesi Anda telah berakhir. Silakan masuk lagi.')
  })
})
