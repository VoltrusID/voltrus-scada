import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

interface ApiKey {
  id: number
  name: string
  key_prefix: string
  created_at: number
  created_by: string
  last_used_at: number | null
  expires_at: number | null
  permissions: string  // JSON string
}

interface CreateApiKeyResponse {
  id: number
  name: string
  key: string  // FULL key shown only once!
  key_prefix: string
  created_at: number
  warning: string
}

export function useApiKeys() {
  return useQuery({
    queryKey: queryKeys.apiKeys,
    queryFn: () => fetchJson<ApiKey[]>('/api/v1/api-keys'),
  })
}

export function useCreateApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (params: { name: string; expires_at?: number; permissions?: string }) => {
      const r = await fetchOk('/api/v1/api-keys', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(params),
      })
      return r.json() as Promise<CreateApiKeyResponse>
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.apiKeys }),
  })
}

export function useRevokeApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/api-keys/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.apiKeys }),
  })
}
