import { fetchJson, fetchOk } from '../queries/fetch'

// ── Types ──────────────────────────────────────────────────────────

export interface SimProfile {
  id: number
  name: string
  data_type: string
  config: string
  created_at: number
}

export interface SimConfig {
  simulation_mode: boolean
  sim_profile_id?: number
  sim_config?: string
}

// ── API Functions ──────────────────────────────────────────────────

export async function listSimProfiles(): Promise<SimProfile[]> {
  return fetchJson<SimProfile[]>('/api/v1/sim-profiles')
}

export async function createSimProfile(
  data: { name: string; data_type: string; config: string },
): Promise<SimProfile> {
  const res = await fetchOk('/api/v1/sim-profiles', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(data),
  })
  return res.json()
}

export async function updateSimProfile(
  id: number,
  data: Partial<Pick<SimProfile, 'name' | 'data_type' | 'config'>>,
): Promise<SimProfile> {
  const res = await fetchOk(`/api/v1/sim-profiles/${id}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(data),
  })
  return res.json()
}

export async function deleteSimProfile(id: number): Promise<void> {
  await fetchOk(`/api/v1/sim-profiles/${id}`, { method: 'DELETE' })
}

export async function toggleSimulation(
  deviceId: string,
  config: SimConfig,
): Promise<void> {
  await fetchOk(`/api/v1/devices/${deviceId}/simulate`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  })
}

export async function goLive(deviceId: string): Promise<void> {
  await fetchOk(`/api/v1/devices/${deviceId}/go-live`, { method: 'POST' })
}

// ── Go-live verification and rollback ──────────────────────────────

export interface GoLiveStatus {
  active: boolean
  /** verifying: inside the grace window; connected; failed: grace window over, device never came online */
  state?: 'verifying' | 'connected' | 'failed'
  device_online?: boolean
  snapshot_id?: number
  went_live_at?: number
  grace_secs?: number
  grace_remaining_secs?: number
  auto_rollback?: boolean
}

export interface GoLiveSettings {
  grace_secs: number
  auto_rollback: boolean
}

export async function getGoLiveStatus(deviceId: string): Promise<GoLiveStatus> {
  return fetchJson<GoLiveStatus>(`/api/v1/devices/${deviceId}/go-live`)
}

/** Restore the pre-go-live snapshot and put the device back in simulation mode. */
export async function rollbackGoLive(deviceId: string): Promise<void> {
  await fetchOk(`/api/v1/devices/${deviceId}/go-live/rollback`, { method: 'POST' })
}

export async function getGoLiveSettings(): Promise<GoLiveSettings> {
  return fetchJson<GoLiveSettings>('/api/v1/settings/go-live')
}

export async function updateGoLiveSettings(settings: GoLiveSettings): Promise<GoLiveSettings> {
  const res = await fetchOk('/api/v1/settings/go-live', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(settings),
  })
  return res.json()
}
