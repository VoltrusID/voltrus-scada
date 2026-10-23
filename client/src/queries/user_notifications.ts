import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './keys'
import { fetchJson, fetchOk } from './fetch'

export interface UserNotification {
  id: number
  user_id: number
  from_user_id: number
  notification_type: string
  message: string
  reference_id: number | null
  read: boolean
  created_at: number
}

export function useUserNotifications(enabled: boolean, unreadOnly = false) {
  return useQuery({
    queryKey: [...queryKeys.userNotifications(), unreadOnly],
    queryFn: async () => {
      const url = `/api/v1/user-notifications${unreadOnly ? '?unread_only=true' : ''}`
      const r = await fetchJson<{ notifications: UserNotification[]; count: number }>(url)
      return r.notifications
    },
    enabled,
    refetchInterval: 30_000,
    staleTime: 20_000,
  })
}

export function useMarkNotificationRead() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/user-notifications/${id}/read`, { method: 'PUT' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.userNotifications() })
    },
  })
}

export function useDismissNotification() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/user-notifications/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.userNotifications() })
    },
  })
}
