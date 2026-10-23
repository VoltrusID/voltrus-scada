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
