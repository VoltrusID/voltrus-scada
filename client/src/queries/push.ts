import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson } from './fetch'
import { subscribePush, unsubscribePush, isPushSupported, getPushPermissionState } from '../sw-register'

interface VapidKeyResponse {
  publicKey: string
}

export function useVapidKey() {
  return useQuery({
    queryKey: ['vapid-key'],
    queryFn: () => fetchJson<VapidKeyResponse>('/api/v1/push/vapid-key'),
    enabled: isPushSupported(),
    retry: false,
  })
}

export function usePushPermission() {
  return useQuery({
    queryKey: ['push-permission'],
    queryFn: () => getPushPermissionState(),
    staleTime: 0, // always fresh check
  })
}

export function usePushSupported() {
  return isPushSupported()
}

export function useSubscribePush() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (vapidPublicKey: string) => {
      const ok = await subscribePush(vapidPublicKey)
      if (!ok) throw new Error('Push subscription failed')
      return true
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['push-permission'] })
    },
  })
}

export function useUnsubscribePush() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      const ok = await unsubscribePush()
      if (!ok) throw new Error('Push unsubscription failed')
      return true
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['push-permission'] })
    },
  })
}
