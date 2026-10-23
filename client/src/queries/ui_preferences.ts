import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export type ThemeValue = 'light' | 'dark' | 'system'

export function useUiTheme() {
  return useQuery({
    queryKey: queryKeys.uiTheme,
    queryFn: () => fetchJson<{ theme: ThemeValue }>('/api/v1/settings/ui-theme'),
    staleTime: Infinity,
  })
}

export function useUpdateUiTheme() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (theme: ThemeValue) => {
      await fetchOk('/api/v1/settings/ui-theme', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ theme }),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.uiTheme })
    },
  })
}

export type LangValue = 'en' | 'id' | 'zh' | 'de'

export function useUiLang() {
  return useQuery({
    queryKey: queryKeys.uiLang,
    queryFn: () => fetchJson<{ lang: LangValue }>('/api/v1/settings/ui-lang'),
    staleTime: Infinity,
  })
}

export function useUpdateUiLang() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (lang: LangValue) => {
      await fetchOk('/api/v1/settings/ui-lang', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ lang }),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.uiLang })
    },
  })
}
