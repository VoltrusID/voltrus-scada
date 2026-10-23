import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface S7Device {
  id: string
  name: string
  ip: string
  port: number
  rack: number
  slot: number
  enabled: boolean
  tag_count?: number
  created_at: number
  updated_at: number
}

export interface S7Tag {
  id: number
  name: string
  data_type: string
  db_number: number
  offset: number
  area: string
}

export function useS7Devices(enabled = true) {
  return useQuery({
    queryKey: queryKeys.s7Devices,
    queryFn: () => fetchJson<S7Device[]>('/api/v1/s7/devices'),
    enabled,
  })
}

export function useS7Device(id: string, enabled = true) {
  return useQuery({
    queryKey: queryKeys.s7Device(id),
    queryFn: () => fetchJson<S7Tag[]>(`/api/v1/s7/devices/${id}/tags`),
    enabled: enabled && !!id,
  })
}

export function useCreateS7Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (device: Partial<S7Device> & { tags?: Partial<S7Tag>[] }) => {
      const res = await fetchOk('/api/v1/s7/devices', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(device),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Devices })
    },
  })
}

export function useUpdateS7Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...fields }: { id: string } & Record<string, any>) => {
      await fetchOk(`/api/v1/s7/devices/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(fields),
      })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Devices })
    },
  })
}

export function useDeleteS7Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/s7/devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Devices })
    },
  })
}

export function useBrowseS7Device() {
  return useMutation({
    mutationFn: async ({ id, params }: { id: string; params?: Record<string, any> }) => {
      const res = await fetchOk(`/api/v1/s7/devices/${id}/browse`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(params || {}),
      })
      return res.json()
    },
  })
}

export function useStartS7Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/s7/devices/${id}/start`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Devices })
    },
  })
}

export function useStopS7Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) => {
      await fetchOk(`/api/v1/s7/devices/${id}/stop`, { method: 'POST' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Devices })
    },
  })
}

export function useAddS7Tag() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, tag }: { deviceId: string; tag: Partial<S7Tag> }) => {
      const res = await fetchOk(`/api/v1/s7/devices/${deviceId}/tags`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(tag),
      })
      return res.json()
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Device(deviceId) })
    },
  })
}

export function useRemoveS7Tag() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ deviceId, tagId }: { deviceId: string; tagId: number }) => {
      await fetchOk(`/api/v1/s7/devices/${deviceId}/tags/${tagId}`, { method: 'DELETE' })
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Device(deviceId) })
    },
  })
}
