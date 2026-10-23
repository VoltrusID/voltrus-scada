import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

/* ── Types ──────────────────────────────────────────────────────── */

export interface Asset {
  id: string
  device_ref: string | null
  name: string
  asset_type: string
  vendor: string | null
  model: string | null
  firmware_version: string | null
  serial_number: string | null
  ip_address: string | null
  mac_address: string | null
  location: string | null
  criticality: string
  tags: string[]
  risk_score: number
  last_maintenance: string | null
  notes: string | null
  created_at: string
  updated_at: string
}

export interface MaintenanceLog {
  id: string
  asset_id: string
  description: string
  technician: string
  performed_at: string
  created_by: string
  created_at: string
}

export interface FirmwareCatalogEntry {
  id: string
  vendor: string
  model: string
  latest_version: string
  release_date: string | null
  notes: string | null
  updated_at: string
}

export interface SnmpScanResult {
  ip: string
  sys_descr: string | null
  sys_name: string | null
  sys_uptime: string | null
  vendor: string | null
  model: string | null
  firmware: string | null
  mac_address: string | null
  scanned_at: string
}

export interface ScanStatus {
  scanning: boolean
  progress: number
  total: number
}

/* ── Constants ──────────────────────────────────────────────────── */

export const ASSET_TYPES = ['plc', 'rtu', 'hmi', 'sensor', 'switch', 'router', 'gateway', 'other'] as const
export const CRITICALITY_LEVELS = ['low', 'medium', 'high', 'critical'] as const

/* ── Query keys ─────────────────────────────────────────────────── */

export const assetQueryKeys = {
  assets:         ['assets'] as const,
  asset:          (id: string) => ['asset', id] as const,
  maintenance:    (assetId: string) => ['maintenance', assetId] as const,
  firmwareCatalog: (vendor?: string) => ['firmwareCatalog', vendor] as const,
  scanResults:    ['scanResults'] as const,
  scanStatus:     ['scanStatus'] as const,
  ipConflicts:    ['ipConflicts'] as const,
}

/* ── Asset CRUD ─────────────────────────────────────────────────── */

export interface AssetFilters {
  asset_type?: string
  criticality?: string
  vendor?: string
  location?: string
  search?: string
}

export function useAssets(filters?: AssetFilters) {
  const sp = new URLSearchParams()
  if (filters?.asset_type) sp.set('asset_type', filters.asset_type)
  if (filters?.criticality) sp.set('criticality', filters.criticality)
  if (filters?.vendor) sp.set('vendor', filters.vendor)
  if (filters?.location) sp.set('location', filters.location)
  if (filters?.search) sp.set('search', filters.search)
  const qs = sp.toString()
  return useQuery({
    queryKey: [...assetQueryKeys.assets, filters],
    queryFn: () => fetchJson<Asset[]>(`/api/v1/assets${qs ? '?' + qs : ''}`),
  })
}

export function useAsset(id: string) {
  return useQuery({
    queryKey: assetQueryKeys.asset(id),
    queryFn: () => fetchJson<Asset>(`/api/v1/assets/${id}`),
    enabled: !!id,
  })
}

export function useCreateAsset() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (asset: Partial<Asset>) => {
      const res = await fetchOk('/api/v1/assets', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(asset),
      })
      return res.json() as Promise<Asset>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: assetQueryKeys.assets })
    },
  })
}

export function useUpdateAsset() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...data }: Partial<Asset> & { id: string }) => {
      const res = await fetchOk(`/api/v1/assets/${id}`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return res.json() as Promise<Asset>
    },
    onSuccess: (_data, { id }) => {
      qc.invalidateQueries({ queryKey: assetQueryKeys.assets })
      qc.invalidateQueries({ queryKey: assetQueryKeys.asset(id) })
    },
  })
}

export function useDeleteAsset() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/assets/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: assetQueryKeys.assets })
    },
  })
}

/* ── Maintenance logs ───────────────────────────────────────────── */

export function useMaintenanceLogs(assetId: string) {
  return useQuery({
    queryKey: assetQueryKeys.maintenance(assetId),
    queryFn: () => fetchJson<MaintenanceLog[]>(`/api/v1/assets/${assetId}/maintenance`),
    enabled: !!assetId,
  })
}

export function useCreateMaintenanceLog(assetId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (log: { description: string; technician: string; performed_at: string }) => {
      const res = await fetchOk(`/api/v1/assets/${assetId}/maintenance`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(log),
      })
      return res.json() as Promise<MaintenanceLog>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: assetQueryKeys.maintenance(assetId) })
      qc.invalidateQueries({ queryKey: assetQueryKeys.asset(assetId) })
    },
  })
}

/* ── Firmware catalog ───────────────────────────────────────────── */

export function useFirmwareCatalog(vendor?: string) {
  const sp = new URLSearchParams()
  if (vendor) sp.set('vendor', vendor)
  const qs = sp.toString()
  return useQuery({
    queryKey: assetQueryKeys.firmwareCatalog(vendor),
    queryFn: () => fetchJson<FirmwareCatalogEntry[]>(`/api/v1/firmware-catalog${qs ? '?' + qs : ''}`),
  })
}

export function useCreateFirmwareEntry() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (entry: Partial<FirmwareCatalogEntry>) => {
      const res = await fetchOk('/api/v1/firmware-catalog', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(entry),
      })
      return res.json() as Promise<FirmwareCatalogEntry>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['firmwareCatalog'] })
    },
  })
}

export function useUpdateFirmwareEntry() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...data }: Partial<FirmwareCatalogEntry> & { id: string }) => {
      const res = await fetchOk(`/api/v1/firmware-catalog/${id}`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return res.json() as Promise<FirmwareCatalogEntry>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['firmwareCatalog'] })
    },
  })
}

export function useDeleteFirmwareEntry() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/firmware-catalog/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['firmwareCatalog'] })
    },
  })
}

/* ── SNMP scanning ──────────────────────────────────────────────── */

export function useStartScan() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (params: { subnet: string; community?: string; timeout_ms?: number }) => {
      const res = await fetchOk('/api/v1/snmp/scan', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(params),
      })
      return res.json() as Promise<{ ok: boolean; message: string }>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: assetQueryKeys.scanStatus })
      qc.invalidateQueries({ queryKey: assetQueryKeys.scanResults })
    },
  })
}

export function useScanStatus() {
  return useQuery({
    queryKey: assetQueryKeys.scanStatus,
    queryFn: () => fetchJson<ScanStatus>('/api/v1/snmp/scan/status'),
    refetchInterval: 2_000,
  })
}

export function useScanResults() {
  return useQuery({
    queryKey: assetQueryKeys.scanResults,
    queryFn: () => fetchJson<SnmpScanResult[]>('/api/v1/snmp/scan/results'),
  })
}

export function useImportScannedDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (ip: string) => {
      const res = await fetchOk(`/api/v1/snmp/scan/${encodeURIComponent(ip)}/import`, { method: 'POST' })
      return res.json() as Promise<Asset>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: assetQueryKeys.assets })
      qc.invalidateQueries({ queryKey: assetQueryKeys.scanResults })
    },
  })
}

/* ── Asset import (CSV) ─────────────────────────────────────────── */

export function useImportAssets() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (file: File) => {
      const fd = new FormData()
      fd.append('file', file)
      const res = await fetchOk('/api/v1/assets/import', {
        method: 'POST',
        body: fd,
      })
      return res.json() as Promise<{ import_id: string; rows: Record<string, unknown>[]; total: number; valid: number; errors: string[] }>
    },
  })
}

export function useConfirmImport() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (import_id: string) => {
      const res = await fetchOk('/api/v1/assets/import/confirm', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ import_id }),
      })
      return res.json() as Promise<{ total: number; created: number; skipped_duplicates: number; errors: string[] }>
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: assetQueryKeys.assets })
    },
  })
}

/* ── NIS2 CSV export ────────────────────────────────────────────── */

export function useNis2Export() {
  return useMutation({
    mutationFn: async () => {
      const res = await fetch('/api/v1/assets/nis2-export', { credentials: 'include' })
      if (!res.ok) throw new Error('NIS2 export failed')
      const blob = await res.blob()
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = 'nis2-asset-export.csv'
      a.click()
      URL.revokeObjectURL(url)
    },
  })
}

/* ── IP conflicts ───────────────────────────────────────────────── */

export function useIpConflicts() {
  return useQuery({
    queryKey: assetQueryKeys.ipConflicts,
    queryFn: () => fetchJson<{ ip: string; assets: { id: string; name: string }[] }[]>('/api/v1/assets/ip-conflicts'),
  })
}
