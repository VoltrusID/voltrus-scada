import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface SmtpConfigResponse {
  configured: boolean
  host?: string
  port?: number
  username?: string
  password?: string
  from_address?: string
  from_name?: string
  encryption?: 'starttls' | 'implicittls' | 'none'
}

export interface SmtpConfigUpdate {
  host: string
  port: number
  username: string
  password?: string
  from_address?: string
  from_name?: string
  encryption?: 'starttls' | 'implicittls' | 'none'
}

export function useSmtpConfig() {
  return useQuery({
    queryKey: queryKeys.smtpConfig,
    queryFn: () => fetchJson<SmtpConfigResponse>('/api/v1/settings/smtp'),
    staleTime: 30_000,
  })
}

export function useUpdateSmtpConfig() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (config: SmtpConfigUpdate) => {
      await fetchOk('/api/v1/settings/smtp', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.smtpConfig })
    },
  })
}

export function useTestSmtpConnection() {
  return useMutation({
    mutationFn: async (recipient: string) => {
      const res = await fetchOk('/api/v1/settings/smtp/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ recipient }),
      })
      return res.json()
    },
  })
}
