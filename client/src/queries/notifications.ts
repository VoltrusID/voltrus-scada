import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import type { NotificationSetting } from '../types'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export function useNotificationSettings() {
  return useQuery({
    queryKey: queryKeys.notificationSettings,
    queryFn: () => fetchJson<NotificationSetting[]>('/api/v1/notifications/settings'),
    staleTime: 10_000,
  })
}

export function useCreateNotificationSetting() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (data: { source_key: string; channel_type: string; config: string; enabled: boolean }) => {
      const res = await fetchOk('/api/v1/notifications/settings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.notificationSettings })
    },
  })
}

export function useUpdateNotificationSetting() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, ...fields }: { id: number } & Record<string, any>) => {
      const res = await fetchOk(`/api/v1/notifications/settings/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(fields),
      })
      return res.json()
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.notificationSettings })
    },
  })
}

export function useDeleteNotificationSetting() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/notifications/settings/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.notificationSettings })
    },
  })
}

export function useTestNotification() {
  return useMutation({
    mutationFn: async (data: { channel_type: string; config: string }) => {
      const res = await fetchOk('/api/v1/notifications/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      })
      return res.json() as Promise<{ success: boolean }>
    },
  })
}
