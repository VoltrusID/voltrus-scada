import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { createdId, s7DeviceBody, s7TagBody } from '../protocolPayloads'
import type { S7DeviceForm, S7TagForm } from '../protocolPayloads'
import { fetchJson, fetchOk } from './fetch'

/** A row of `GET /api/v1/s7/devices`; `url` is `host:port`. */
export interface S7Device {
  id: number
  name: string
  url: string
  rack: number
  slot: number
  interval_ms: number
  enabled: boolean
  created_at: number
  tag_count: number
}

/** A row of `GET /api/v1/s7/devices/{id}/tags`; `tag_address` is e.g. `DB1.DBD0`. */
export interface S7Tag {
  id: number
  device_id: number
  tag_address: string
  name: string
  display_name: string
  data_type: string
  created_at: number
}

export function useS7Devices(enabled = true) {
  return useQuery({
    queryKey: queryKeys.s7Devices,
    queryFn: () => fetchJson<S7Device[]>('/api/v1/s7/devices'),
    enabled,
  })
}

export function useS7Device(id: number, enabled = true) {
  return useQuery({
    queryKey: queryKeys.s7Device(id),
    queryFn: () => fetchJson<S7Tag[]>(`/api/v1/s7/devices/${id}/tags`),
    enabled: enabled && !!id,
  })
}

/** Creates the device, then its tags (the create route takes no tags). */
export function useCreateS7Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ device, tags = [] }: { device: S7DeviceForm; tags?: S7TagForm[] }) => {
      const res = await fetchOk('/api/v1/s7/devices', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(s7DeviceBody(device)),
      })
      const id = createdId(await res.json())
      for (const tag of tags) {
        await fetchOk(`/api/v1/s7/devices/${id}/tags`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(s7TagBody(tag)),
        })
      }
      return id
    },
    onSettled: () => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Devices })
    },
  })
}

export function useUpdateS7Device() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...fields }: { id: number } & Record<string, any>) => {
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
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/s7/devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Devices })
    },
  })
}

export function useBrowseS7Device() {
  return useMutation({
    mutationFn: async ({ id, params }: { id: number; params?: Record<string, any> }) => {
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
    mutationFn: async (id: number) => {
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
    mutationFn: async (id: number) => {
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
    mutationFn: async ({ deviceId, tag }: { deviceId: number; tag: S7TagForm }) => {
      const res = await fetchOk(`/api/v1/s7/devices/${deviceId}/tags`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(s7TagBody(tag)),
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
    mutationFn: async ({ deviceId, tagId }: { deviceId: number; tagId: number }) => {
      await fetchOk(`/api/v1/s7/devices/${deviceId}/tags/${tagId}`, { method: 'DELETE' })
    },
    onSuccess: (_, { deviceId }) => {
      qc.invalidateQueries({ queryKey: queryKeys.s7Device(deviceId) })
    },
  })
}
