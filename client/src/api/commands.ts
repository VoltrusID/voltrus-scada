import { fetchJson, fetchOk } from '../queries/fetch'
import { parseWritePolicy } from '../writePolicy'

// ── Types ──────────────────────────────────────────────────────────

export interface Command {
  id: string
  data_source_id: number
  /** Numeric device id; 0 for a Modbus device, whose text id is `device_ref`. */
  device_id: number
  device_ref?: string
  protocol: string
  value: string
  status: 'pending' | 'queued' | 'sent' | 'acked' | 'failed' | 'expired' | 'cancelled'
  policy: string
  scheduled_at: number | null
  retry_count: number
  max_retries: number
  error_message: string | null
  created_at: number
  sent_at: number | null
  acked_at: number | null
  resolved_at: number | null
  created_by: string
}

export interface CommandAuditEntry {
  id: number
  command_id: string
  old_status: string
  new_status: string
  changed_by: string
  reason: string | null
  created_at: number
}

export interface CommandQuery {
  tag?: number
  status?: string
  device?: number
  from?: number
  to?: number
  limit?: number
  offset?: number
}

export interface WritePolicy {
  writable: boolean
  write_min: number | null
  write_max: number | null
  write_max_rate: number | null
  write_requires_reason: boolean
  write_deadband: number | null
}

// ── API Functions ──────────────────────────────────────────────────

export async function writeTag(
  tagId: number,
  value: number,
  policy: string = 'immediate',
  reason?: string,
): Promise<{ command_id: string; status: string }> {
  const body: Record<string, unknown> = { value, policy }
  if (reason) body.reason = reason
  const res = await fetchOk(`/api/v1/tags/${tagId}/write`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  return res.json()
}

export async function getCommand(commandId: string): Promise<Command> {
  return fetchJson<Command>(`/api/v1/commands/${commandId}`)
}

export async function listCommands(query?: CommandQuery): Promise<Command[]> {
  const qs = new URLSearchParams()
  if (query?.tag) qs.set('tag', String(query.tag))
  if (query?.status) qs.set('status', query.status)
  if (query?.device) qs.set('device', String(query.device))
  if (query?.from) qs.set('from', String(query.from))
  if (query?.to) qs.set('to', String(query.to))
  if (query?.limit) qs.set('limit', String(query.limit))
  if (query?.offset) qs.set('offset', String(query.offset))
  const q = qs.toString()
  return fetchJson<Command[]>(`/api/v1/commands${q ? `?${q}` : ''}`)
}

export async function cancelCommand(commandId: string): Promise<void> {
  await fetchOk(`/api/v1/commands/${commandId}/cancel`, { method: 'POST' })
}

export async function getCommandAudit(commandId: string): Promise<CommandAuditEntry[]> {
  return fetchJson<CommandAuditEntry[]>(`/api/v1/commands/${commandId}/audit`)
}

export async function getWritePolicy(tagId: number): Promise<WritePolicy> {
  return parseWritePolicy(await fetchJson<unknown>(`/api/v1/data-sources/${tagId}/write-policy`))
}

export async function updateWritePolicy(
  tagId: number,
  policy: Partial<WritePolicy>,
): Promise<void> {
  await fetchOk(`/api/v1/data-sources/${tagId}/write-policy`, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(policy),
  })
}
