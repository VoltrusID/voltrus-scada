import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { FaceplateDef, RoleKind } from '../scada2/hmi/faceplateMap'
import { fetchJson, fetchSigned } from './fetch'

// Equipment types and instances (docs/equipment-types.md): a type defines the
// tags, alarms, faceplate and mimic symbol of one kind of equipment; an
// instance is one line (name, type, device, base) that the server expands.

export interface TypeTag {
  role: string
  label?: string
  kind: RoleKind
  key?: string
  name?: string | null
  offset?: number | null
  register_type?: string
  register_area?: string
  scale?: number
  unit?: string | null
  min?: number | null
  max?: number | null
  decimals?: number | null
  alarm?: Record<string, number | null> | null
  write?: Record<string, unknown> | null
  required?: boolean
  fallback?: string | null
}

export interface EquipmentType {
  id: string
  name: string
  description?: string
  category?: string
  symbol: string
  faceplate?: string | null
  tags: TypeTag[]
  bindings: { primary?: string; fault?: string; run?: string; manual?: string; interlock?: string }
  symbol_config?: Record<string, unknown>
  flows?: unknown[]
  instance_count?: number
}

export interface RoleOverride {
  key?: string
  name?: string
  unit?: string
  min?: number
  max?: number
  decimals?: number
  skip?: boolean
}

export interface EquipmentInstanceInput {
  name: string
  type: string
  device: string
  base?: number | null
  prefix?: string | null
  description?: string
  symbol?: string | null
  overrides?: Record<string, RoleOverride>
}

export type RoleStatus = 'bound' | 'created' | 'fallback' | 'missing' | 'skipped'

export interface ResolvedRole {
  role: string
  kind: RoleKind
  label?: string
  key: string
  ref: string
  tag_id: number | null
  status: RoleStatus
}

/** What the server writes on every symbol bound to the instance (applied as is by the Designer). */
export interface ResolvedWidget {
  equipment_type: 'symbol'
  sensor_id: number | null
  config: Record<string, unknown>
  remove: string[]
}

export interface Resolved {
  prefix: string
  symbol: string
  faceplate: string | null
  roles: ResolvedRole[]
  widget: ResolvedWidget
}

export interface TagChange { key: string; ref: string; action: 'create' | 'update' | 'bind' | 'unchanged'; fields?: string[] }

export interface InstanceChanges {
  instance?: string
  tags: TagChange[]
  registers_added: unknown[]
  flows: unknown[]
  widgets: { screen_id: string; equipment_id: string }[]
}

export interface EquipmentInstance extends EquipmentInstanceInput {
  resolved: Resolved
  changes?: InstanceChanges
}

export interface LibraryEntry { type: EquipmentType; faceplate: FaceplateDef | null }

const keys = {
  types: ['equipmentTypes'] as const,
  instances: ['equipmentInstances'] as const,
  library: ['equipmentLibrary'] as const,
}
export const equipmentKeys = keys

async function send<T>(url: string, method: string, body?: unknown): Promise<T> {
  const r = await fetchSigned(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const text = await r.text()
  let data: any = null
  try { data = text ? JSON.parse(text) : null } catch { data = null }
  if (!r.ok) throw new Error((data && (data.error || data.message)) || `Request failed: ${r.status}`)
  return data as T
}

export function useEquipmentTypes(enabled = true) {
  return useQuery({ queryKey: keys.types, queryFn: () => fetchJson<EquipmentType[]>('/api/v1/equipment-types'), enabled })
}

export function useEquipmentInstances(enabled = true) {
  return useQuery({ queryKey: keys.instances, queryFn: () => fetchJson<EquipmentInstance[]>('/api/v1/equipment-instances'), enabled })
}

export function useEquipmentLibrary(enabled = true) {
  return useQuery({ queryKey: keys.library, queryFn: () => fetchJson<LibraryEntry[]>('/api/v1/equipment-types/library'), enabled })
}

const instanceUrl = (name: string) => `/api/v1/equipment-instances/${encodeURIComponent(name)}`

/** Instantiate (or preview with `dryRun`). */
export function createInstance(body: EquipmentInstanceInput, dryRun: boolean): Promise<EquipmentInstance> {
  return send<EquipmentInstance>(`/api/v1/equipment-instances?dry_run=${dryRun}`, 'POST', body)
}

/** Change an instance (or preview with `dryRun`); the server re-binds every symbol of it. */
export function updateInstance(name: string, body: EquipmentInstanceInput, dryRun: boolean): Promise<EquipmentInstance> {
  return send<EquipmentInstance>(`${instanceUrl(name)}?dry_run=${dryRun}`, 'PUT', body)
}

export function useInstallLibrary() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (ids?: string[]) => send<{ installed: string[]; skipped: string[] }>('/api/v1/equipment-types/library/install', 'POST', ids ? { ids } : {}),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: keys.types })
      void qc.invalidateQueries({ queryKey: ['faceplates'] })
    },
  })
}

/** Refresh everything an instance change touches: instances, types (counts), tags (created ones), faceplates. */
export function useInvalidateEquipment() {
  const qc = useQueryClient()
  return () => {
    void qc.invalidateQueries({ queryKey: keys.instances })
    void qc.invalidateQueries({ queryKey: keys.types })
    void qc.invalidateQueries({ queryKey: ['dataSources'] })
  }
}
