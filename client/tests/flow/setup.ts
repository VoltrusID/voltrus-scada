import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'
import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import en from '../../public/locales/en/translation.json'

// Components translate through react-i18next; tests read the English strings.
void i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: en } },
  interpolation: { escapeValue: false },
  react: { useSuspense: false },
})

afterEach(() => cleanup())

// jsdom lacks the layout APIs React Flow measures with.
class RO { observe() {} unobserve() {} disconnect() {} }
const g = globalThis as Record<string, unknown>
g.ResizeObserver ??= RO
g.DOMMatrixReadOnly ??= class { m22 = 1 }
if (!window.matchMedia) {
  window.matchMedia = ((q: string) => ({
    matches: false, media: q, onchange: null,
    addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {}, dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia
}

// styles.ts probes a 2D canvas for colour parsing; jsdom has none.
HTMLCanvasElement.prototype.getContext = (() => null) as unknown as HTMLCanvasElement["getContext"]
