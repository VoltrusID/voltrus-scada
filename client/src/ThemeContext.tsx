import { createContext, useContext, useEffect, useState, useCallback, type ReactNode } from 'react'
import { useUiTheme, useUpdateUiTheme } from './queries'
import type { ThemeValue } from './queries'
export type { ThemeValue }

type ResolvedTheme = 'light' | 'dark'

interface ThemeContextValue {
  theme: ThemeValue
  resolvedTheme: ResolvedTheme
  setTheme: (t: ThemeValue) => void
}

const ThemeContext = createContext<ThemeContextValue>({
  theme: 'dark',
  resolvedTheme: 'dark',
  setTheme: () => {},
})

export function useTheme() {
  return useContext(ThemeContext)
}

function getSystemTheme(): ResolvedTheme {
  if (typeof window === 'undefined') return 'dark'
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

function resolveTheme(preference: ThemeValue): ResolvedTheme {
  if (preference === 'system') return getSystemTheme()
  return preference
}

function applyTheme(resolved: ResolvedTheme) {
  document.documentElement.setAttribute('data-theme', resolved)
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const { data } = useUiTheme()
  const updateThemeMut = useUpdateUiTheme()

  const serverTheme: ThemeValue = data?.theme ?? 'dark'
  const [localTheme, setLocalTheme] = useState<ThemeValue>(() => {
    const stored = localStorage.getItem('voltrus:ui_theme') as ThemeValue | null
    return stored ?? 'dark'
  })

  useEffect(() => {
    if (data) {
      localStorage.setItem('voltrus:ui_theme', data.theme)
      setLocalTheme(data.theme)
    }
  }, [data])

  const resolvedTheme = resolveTheme(localTheme)

  useEffect(() => {
    applyTheme(resolvedTheme)
  }, [resolvedTheme])

  useEffect(() => {
    if (localTheme !== 'system') return
    const mq = window.matchMedia('(prefers-color-scheme: light)')
    const handler = () => applyTheme(getSystemTheme())
    mq.addEventListener('change', handler)
    return () => mq.removeEventListener('change', handler)
  }, [localTheme])

  const setTheme = useCallback((t: ThemeValue) => {
    setLocalTheme(t)
    localStorage.setItem('voltrus:ui_theme', t)
    updateThemeMut.mutate(t)
  }, [updateThemeMut])

  return (
    <ThemeContext.Provider value={{ theme: localTheme, resolvedTheme, setTheme }}>
      {children}
    </ThemeContext.Provider>
  )
}
