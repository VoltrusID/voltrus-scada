import { useTranslation } from 'react-i18next'
import { useUiLang, useUpdateUiLang, type LangValue } from '../queries'

const LANGUAGES: { code: LangValue; label: string }[] = [
  { code: 'en', label: 'English' },
  { code: 'id', label: 'Bahasa Indonesia' },
  { code: 'zh', label: '\u4e2d\u6587' },
  { code: 'de', label: 'Deutsch' },
]

export function LanguageSelector() {
  const { i18n } = useTranslation()
  const { data } = useUiLang()
  const updateLangMut = useUpdateUiLang()

  const serverLang: LangValue = data?.lang ?? 'en'

  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const lang = e.target.value as LangValue
    i18n.changeLanguage(lang)
    localStorage.setItem('voltrus:ui_lang', lang)
    updateLangMut.mutate(lang)
  }

  return (
    <select
      value={serverLang}
      onChange={handleChange}
      aria-label="Language"
      style={{
        background: 'var(--card)',
        border: '1px solid var(--border)',
        color: 'var(--text)',
        borderRadius: 6,
        padding: '6px 10px',
        fontSize: 12,
        fontFamily: 'var(--font-ui)',
        cursor: 'pointer',
        outline: 'none',
        minWidth: 44,
        minHeight: 36,
      }}
    >
      {LANGUAGES.map(({ code, label }) => (
        <option key={code} value={code}>{label}</option>
      ))}
    </select>
  )
}
