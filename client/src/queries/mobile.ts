import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { fetchJson, fetchOk } from './fetch'

/** A phone paired with the Voltrus SCADA app. */
export interface MobileDevice {
  id: number
  username: string
  name: string
  platform: 'android' | 'ios' | string
  push: boolean
  min_priority: 'info' | 'warning' | 'critical' | string
  lang: string
  created_at: number
  last_seen_at: number | null
  expires_at: number | null
}

export interface PairingResponse {
  expires_at: number
  link: string
  push_available: boolean
  payload: {
    v: number
    site: string
    urls: string[]
    relay?: { url: string; site_id: string }
    code: string
  }
}

export interface MobileSettings {
  site_name: string
  push_details: boolean
  pinned_tags: string[]
  public_urls: string[]
}

export interface MobileSettingsResponse extends MobileSettings {
  effective_site_name: string
  push_licensed: boolean
  relay: { url: string; site_id: string; connected: boolean } | null
  device_limit: number
}

const keys = {
  devices: (all: boolean) => ['mobile', 'devices', all] as const,
  settings: ['mobile', 'settings'] as const,
}

export function useMobileDevices(all = false) {
  return useQuery({
    queryKey: keys.devices(all),
    queryFn: () => fetchJson<MobileDevice[]>(`/api/v1/mobile/devices${all ? '?all=true' : ''}`),
  })
}

export function useRemoveMobileDevice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (id: number) => {
      await fetchOk(`/api/v1/mobile/devices/${id}`, { method: 'DELETE' })
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ['mobile', 'devices'] }),
  })
}

export function useCreatePairing() {
  return useMutation({
    mutationFn: async () => {
      const r = await fetchOk('/api/v1/mobile/pairings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ origin: window.location.origin }),
      })
      return (await r.json()) as PairingResponse
    },
  })
}

export function useMobileSettings(enabled: boolean) {
  return useQuery({
    queryKey: keys.settings,
    queryFn: () => fetchJson<MobileSettingsResponse>('/api/v1/mobile/settings'),
    enabled,
  })
}

export function useSaveMobileSettings() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (settings: MobileSettings) => {
      const r = await fetchOk('/api/v1/mobile/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(settings),
      })
      return (await r.json()) as MobileSettings
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: keys.settings }),
  })
}
