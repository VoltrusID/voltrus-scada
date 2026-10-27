import type { WritePolicy } from './api/commands'

function numberOrNull(v: unknown, field: string): number | null {
  if (v == null) return null
  if (typeof v !== 'number') throw new Error(`write policy: ${field} is not a number`)
  return v
}

/** Reads the body of `GET /api/v1/data-sources/{id}/write-policy`. */
export function parseWritePolicy(answer: unknown): WritePolicy {
  const a = answer as Record<string, unknown> | null
  if (!a || typeof a !== 'object' || typeof a.writable !== 'boolean' || typeof a.write_requires_reason !== 'boolean') {
    throw new Error('write policy: unexpected answer from the server')
  }
  return {
    writable: a.writable,
    write_min: numberOrNull(a.write_min, 'write_min'),
    write_max: numberOrNull(a.write_max, 'write_max'),
    write_max_rate: numberOrNull(a.write_max_rate, 'write_max_rate'),
    write_requires_reason: a.write_requires_reason,
    write_deadband: numberOrNull(a.write_deadband, 'write_deadband'),
  }
}
