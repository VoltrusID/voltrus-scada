import { fetchJson, fetchOk } from '../queries/fetch'

// ── Types ──────────────────────────────────────────────────────────

export interface Script {
  id: number
  name: string
  source: string
  syntax_error: string | null
  cpu_limit_ms: number
  memory_limit_bytes: number
  enabled: boolean
  created_at: number
  updated_at: number
}

export interface ScriptExecution {
  id: number
  script_id: number
  flow_id: string | null
  trigger_type: string | null
  duration_ms: number
  status: string
  error_message: string | null
  output: string | null
  created_at: number
}

export interface TestResult {
  success: boolean
  output?: string
  error?: string
  duration_ms: number
}

// ── API Functions ──────────────────────────────────────────────────

export async function listScripts(): Promise<Script[]> {
  return fetchJson<Script[]>('/api/v1/scripts')
}

export async function createScript(
  data: { name: string; source: string; cpu_limit_ms?: number; memory_limit_bytes?: number },
): Promise<Script> {
  const res = await fetchOk('/api/v1/scripts', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(data),
  })
  return res.json()
}

export async function updateScript(
  id: number,
  data: Partial<Pick<Script, 'name' | 'source' | 'cpu_limit_ms' | 'memory_limit_bytes' | 'enabled'>>,
): Promise<Script> {
  const res = await fetchOk(`/api/v1/scripts/${id}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(data),
  })
  return res.json()
}

export async function deleteScript(id: number): Promise<void> {
  await fetchOk(`/api/v1/scripts/${id}`, { method: 'DELETE' })
}

export async function testScript(id: number, input: unknown): Promise<TestResult> {
  const res = await fetchOk(`/api/v1/scripts/${id}/test`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ input }),
  })
  return res.json()
}

export async function listExecutions(
  scriptId: number,
  limit = 50,
): Promise<ScriptExecution[]> {
  return fetchJson<ScriptExecution[]>(`/api/v1/scripts/${scriptId}/executions?limit=${limit}`)
}
