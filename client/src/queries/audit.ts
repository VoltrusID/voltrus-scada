import { useQuery, useMutation } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson } from './fetch'

export interface AuditLogEntry {
  id: number
  action: string
  username: string
  resource: string
  details: string | null
  created_at: number
}

export interface AuditLogResponse {
  entries: AuditLogEntry[]
  total: number
  page: number
  limit: number
  total_pages: number
}

export interface AuditQueryParams {
  search?: string
  action?: string
  username?: string
  from?: number
  to?: number
  page?: number
  limit?: number
  sort?: string
  order?: string
}

export function useAuditLogs(params: AuditQueryParams = {}, enabled = true) {
  const sp = new URLSearchParams()
  if (params.search) sp.set('search', params.search)
  if (params.action) sp.set('action', params.action)
  if (params.username) sp.set('username', params.username)
  if (params.from) sp.set('from', String(params.from))
  if (params.to) sp.set('to', String(params.to))
  if (params.page) sp.set('page', String(params.page))
  if (params.limit) sp.set('limit', String(params.limit))
  if (params.sort) sp.set('sort', params.sort)
  if (params.order) sp.set('order', params.order)
  const qs = sp.toString()

  return useQuery({
    queryKey: queryKeys.auditLogs(params),
    queryFn: () => fetchJson<AuditLogResponse>(`/api/v1/audit${qs ? '?' + qs : ''}`),
    enabled,
  })
}

export function useExportAuditCsv() {
  return useMutation({
    mutationFn: async (params: AuditQueryParams = {}) => {
      const sp = new URLSearchParams()
      if (params.search) sp.set('search', params.search)
      if (params.action) sp.set('action', params.action)
      if (params.username) sp.set('username', params.username)
      if (params.from) sp.set('from', String(params.from))
      if (params.to) sp.set('to', String(params.to))
      const qs = sp.toString()
      const res = await fetch(`/api/v1/audit/export${qs ? '?' + qs : ''}`, { credentials: 'include' })
      if (!res.ok) throw new Error('Export failed')
      return res.text()
    },
  })
}
