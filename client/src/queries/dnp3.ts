import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface Dnp3Device {
  id: number
  name: string
  ip: string
  port: number
  master_address: number
  outstation_address: number
  enable_unsolicited: boolean
  integrity_poll_interval_secs: number
  event_scan_interval_ms: number | null
  enabled: boolean
  point_count?: number
  created_at: number
  updated_at: number
}

export interface Dnp3Point {
  id: number
  device_id: number
  group_num: number
  variation: number
  point_index: number
  name: string
  display_name: string
  data_type: string
  scale: number
  deadband: number
}

export interface Dnp3GroupVariation {
  group: number
  variation: number
  name: string
}

export interface Dnp3ScanResult {
  ip: string
  port: number
  outstation_address: number
}

export function useDnp3Devices(enabled = true) {
  return useQuery({
    queryKey: queryKeys.dnp3Devices,
    queryFn: () => fetchJson<Dnp3Device[]>('/api/v1/dnp3/devices'),
    enabled,
  })
}

export function useDnp3Device(id: string, enabled = true) {
  return useQuery({
    queryKey: queryKeys.dnp3Device(id),
    queryFn: () => fetchJson<{ device: Dnp3Device; points: Dnp3Point[] }>(`/api/v1/dnp3/devices/${id}`),
    enabled: enabled && !!id,
  })
}

export function useDnp3GroupVariations(enabled = true) {
  return useQuery({
    queryKey: queryKeys.dnp3GroupVariations,
    queryFn: () => fetchJson<Dnp3GroupVariation[]>('/api/v1/dnp3/group-variations'),
    enabled,
  })
}

export function useCreateDnp3Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (device: Partial<Dnp3Device>) => {
      const res = await fetchOk('/api/v1/dnp3/devices', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(device),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.dnp3Devices })
    },
  })
}

export function useUpdateDnp3Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...fields }: { id: number } & Record<string, any>) => {
      await fetchOk(`/api/v1/dnp3/devices/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(fields),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.dnp3Devices })
    },
  })
}

export function useDeleteDnp3Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/dnp3/devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.dnp3Devices })
    },
  })
}

export function useAddDnp3Point() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, point }: { deviceId: number; point: Partial<Dnp3Point> }) => {
      const res = await fetchOk(`/api/v1/dnp3/devices/${deviceId}/points`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(point),
      })
      return res.json()
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.dnp3Device(String(deviceId)) })
    },
  })
}

export function useRemoveDnp3Point() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, pointId }: { deviceId: number; pointId: number }) => {
      await fetchOk(`/api/v1/dnp3/devices/${deviceId}/points/${pointId}`, { method: 'DELETE' })
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.dnp3Device(String(deviceId)) })
    },
  })
}

export function useScanDnp3Network() {
  return useMutation({
    mutationFn: async ({ range, port }: { range: string; port?: number }) => {
      const res = await fetchOk('/api/v1/dnp3/scan', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ range, port: port || 20000 }),
      })
      return res.json() as Promise<Dnp3ScanResult[]>
    },
  })
}
