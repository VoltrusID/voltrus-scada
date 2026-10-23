import { fetchJson, fetchOk } from '../queries/fetch'

// ── Types ──────────────────────────────────────────────────────────

export interface ConfigSnapshot {
  id: number
  label: string | null
  description: string | null
  snapshot_data: string
  trigger: string
  created_by: string | null
  created_at: number
}

export interface ConfigDiff {
  [tableName: string]: {
    added: unknown[]
    removed: unknown[]
    changed: { id: string; from: unknown; to: unknown }[]
  }
}

// ── API Functions ──────────────────────────────────────────────────

export async function listSnapshots(): Promise<ConfigSnapshot[]> {
  return fetchJson<ConfigSnapshot[]>('/api/v1/config/snapshots')
}

export async function getSnapshot(id: number): Promise<ConfigSnapshot> {
  return fetchJson<ConfigSnapshot>(`/api/v1/config/snapshots/${id}`)
}

export async function createSnapshot(label?: string): Promise<ConfigSnapshot> {
  const res = await fetchOk('/api/v1/config/snapshots', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ label }),
  })
  return res.json()
}

export async function diffSnapshots(
  fromId: number,
  toId: number,
): Promise<ConfigDiff> {
  return fetchJson<ConfigDiff>(`/api/v1/config/snapshots/${fromId}/diff/${toId}`)
}

export async function rollback(id: number): Promise<void> {
  await fetchOk(`/api/v1/config/snapshots/${id}/rollback`, { method: 'POST' })
}
