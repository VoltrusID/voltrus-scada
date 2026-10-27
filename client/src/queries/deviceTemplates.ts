import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { QueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'
import { refQuery } from '../deviceTemplates'
import type { CatalogResponse, DeviceTemplateDetail, TemplateProtocol, TemplateSummary } from '../deviceTemplates'

const BASE = '/api/v1/device-templates'
const json = (body: unknown): RequestInit => ({
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
})

export function useDeviceTemplates(enabled = true) {
  return useQuery({
    queryKey: queryKeys.deviceTemplates,
    queryFn: async () => (await fetchJson<{ templates: TemplateSummary[] }>(BASE)).templates,
    enabled,
  })
}

export function useDeviceTemplate(t: { template_id: string; version: string; source: string } | null) {
  return useQuery({
    queryKey: queryKeys.deviceTemplate(t ? `${t.source}|${t.template_id}|${t.version}` : ''),
    queryFn: () => fetchJson<DeviceTemplateDetail>(`${BASE}/${encodeURIComponent(t!.template_id)}${refQuery(t!)}`),
    enabled: !!t,
  })
}

/** The public catalog index; `url` is an admin-only override of the configured catalog. */
export function useDeviceCatalog(url: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.deviceCatalog(url),
    queryFn: () => fetchJson<CatalogResponse>(`${BASE}/catalog${url ? `?url=${encodeURIComponent(url)}` : ''}`),
    enabled,
    retry: false,
    staleTime: 60_000,
  })
}

function invalidateDevices(qc: QueryClient) {
  qc.invalidateQueries({ queryKey: queryKeys.modbusDevices })
  qc.invalidateQueries({ queryKey: queryKeys.modbusStatus })
  qc.invalidateQueries({ queryKey: queryKeys.bacnetDevices })
}

export function useInstantiateDeviceTemplate() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ templateId, body }: { templateId: string; body: Record<string, unknown> }) => {
      const res = await fetchOk(`${BASE}/${encodeURIComponent(templateId)}/instantiate`, json(body))
      return res.json() as Promise<{ ok: boolean; protocol: TemplateProtocol; id: string | number; name: string; item_count: number }>
    },
    onSuccess: () => invalidateDevices(qc),
  })
}

export function useSaveDeviceAsTemplate() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, body }: { deviceId: string | number; body: Record<string, unknown> }) => {
      const res = await fetchOk(`${BASE}/from-device/${encodeURIComponent(String(deviceId))}`, json(body))
      return res.json() as Promise<{ ok: boolean; template_id: string; version: string; source: string }>
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.deviceTemplates }),
  })
}

/** Import a template file's JSON text. */
export function useImportTemplateFile() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (text: string) => {
      const res = await fetchOk(BASE, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: text })
      return res.json() as Promise<{ ok: boolean; template_id: string; version: string; source: string }>
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.deviceTemplates }),
  })
}

export function useDeleteDeviceTemplate() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (t: { template_id: string; version: string; source: string }) => {
      await fetchOk(`${BASE}/${encodeURIComponent(t.template_id)}${refQuery(t)}`, { method: 'DELETE' })
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.deviceTemplates }),
  })
}

export function useImportFromCatalog() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (p: { template_id: string; version: string; url?: string }) => {
      const res = await fetchOk(`${BASE}/catalog/import`, json(p))
      return res.json() as Promise<{ ok: boolean; template_id: string; version: string; source: string }>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.deviceTemplates })
      qc.invalidateQueries({ queryKey: ['deviceCatalog'] })
    },
  })
}
