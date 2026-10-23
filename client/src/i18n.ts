import { edition, registerLocales } from './edition'
import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import HttpBackend from 'i18next-http-backend'

const SUPPORTED_LANGS = ['en', 'id', 'zh', 'de'] as const
export type SupportedLang = (typeof SUPPORTED_LANGS)[number]

export { SUPPORTED_LANGS }

function detectLanguage(): string {
  const stored = localStorage.getItem('voltrus:ui_lang')
  if (stored && SUPPORTED_LANGS.includes(stored as any)) return stored

  const browserLang = navigator.language.slice(0, 2)
  if (SUPPORTED_LANGS.includes(browserLang as any)) return browserLang

  return 'en'
}

i18n
  .use(HttpBackend)
  .use(initReactI18next)
  .init({
    lng: detectLanguage(),
    fallbackLng: 'en',
    supportedLngs: [...SUPPORTED_LANGS],
    debug: import.meta.env.DEV,
    ns: ['translation'],
    defaultNS: 'translation',
    backend: {
      loadPath: '/locales/{{lng}}/{{ns}}.json',
    },
    interpolation: {
      escapeValue: false,
    },
    react: {
      useSuspense: false,
    },
  })

registerLocales(i18n, edition.locales)

export default i18n