import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'
import { bareList } from '../protocolPayloads'

export interface BacnetDevice {
  id: number
  name: string
  ip: string
  port: number
  device_instance: number
  interval_ms: number
  cov_enabled: boolean
  cov_lifetime: number
  enabled: boolean
  object_count?: number
  created_at: number
}

export interface BacnetObject {
  id: number
  device_id: number
  object_type: string
  object_instance: number
  property: string
  name: string
  display_name: string
  cov_increment: number
  created_at: number
}

export interface BacnetDiscoveredDevice {
  device_instance: number
  ip: string
  port: number
  vendor_name: string | null
  model_name: string | null
  firmware_version: string | null
  object_list_count: number | null
}

export interface BacnetBrowsedObject {
  object_type: string
  object_instance: number
  object_name: string
}

export function useBacnetDevices(enabled = true) {
  return useQuery({
    queryKey: queryKeys.bacnetDevices,
    queryFn: () => fetchJson<BacnetDevice[]>('/api/v1/bacnet/devices'),
    enabled,
  })
}

export function useBacnetDevice(id: string, enabled = true) {
  return useQuery({
    queryKey: queryKeys.bacnetDevice(id),
    queryFn: async () => bareList<BacnetObject>(await fetchJson<unknown>(`/api/v1/bacnet/devices/${id}/objects`)),
    enabled: enabled && !!id,
  })
}

export function useCreateBacnetDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (device: Partial<BacnetDevice>) => {
      const res = await fetchOk('/api/v1/bacnet/devices', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(device),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.bacnetDevices })
    },
  })
}

export function useUpdateBacnetDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...fields }: { id: number } & Record<string, any>) => {
      await fetchOk(`/api/v1/bacnet/devices/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(fields),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.bacnetDevices })
    },
  })
}

export function useDeleteBacnetDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/bacnet/devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.bacnetDevices })
    },
  })
}

export function useAddBacnetObject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, object: obj }: { deviceId: number; object: Partial<BacnetObject> }) => {
      const res = await fetchOk(`/api/v1/bacnet/devices/${deviceId}/objects`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(obj),
      })
      return res.json()
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.bacnetDevice(String(deviceId)) })
    },
  })
}

export function useRemoveBacnetObject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, objectId }: { deviceId: number; objectId: number }) => {
      await fetchOk(`/api/v1/bacnet/devices/${deviceId}/objects/${objectId}`, { method: 'DELETE' })
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.bacnetDevice(String(deviceId)) })
    },
  })
}

export function useBrowseBacnetDevice() {
  return useMutation({
    mutationFn: async (deviceId: number) => {
      const res = await fetchOk(`/api/v1/bacnet/devices/${deviceId}/browse`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
      })
      return res.json() as Promise<{ objects: BacnetBrowsedObject[]; count: number }>
    },
  })
}

export function useDiscoverBacnet() {
  return useMutation({
    mutationFn: async ({ timeout_secs }: { timeout_secs?: number } = {}) => {
      const res = await fetchOk('/api/v1/bacnet/discover', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ timeout_secs: timeout_secs ?? 5 }),
      })
      return res.json() as Promise<{ devices: BacnetDiscoveredDevice[]; count: number }>
    },
  })
}
