// Display helpers for the storage panel.

import type { CompressionConfig } from './queries/compression'

/** "90 days", "1 hour", "10 minutes", or "forever" for null. */
export function formatDuration(secs: number | null): string {
  if (secs === null) return 'forever'
  const units: [number, string][] = [[86400, 'day'], [3600, 'hour'], [60, 'minute'], [1, 'second']]
  for (const [size, name] of units) {
    if (secs >= size && secs % size === 0) {
      const n = secs / size
      return `${n} ${name}${n === 1 ? '' : 's'}`
    }
  }
  return secs === 0 ? '0 seconds' : `${secs} seconds`
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = bytes / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++ }
  return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${units[i]}`
}

/**
 * Rows of the retention table for the event and audit tables: [label, value, help].
 * The values are the ones in force, so a licence-held audit trail reads "forever".
 */
export function eventRetentionRows(cfg: CompressionConfig): [string, string, string][] {
  const held = cfg.compliance_hold
  return [
    ['Alarms and downtime kept', formatDuration(cfg.event_keep_secs), 'retention.event_keep_secs. Cleared alarms, their journal and ended downtime events. Active, acknowledged and shelved alarms are never removed.'],
    ['Security log kept', formatDuration(cfg.security_keep_secs), held ? 'retention.security_keep_secs. Held by the licence (IEC 62443): kept forever when that mode is licensed.' : 'retention.security_keep_secs.'],
    ['Audit trails kept', formatDuration(cfg.audit_keep_secs), held ? 'retention.audit_keep_secs. Held by the licence (21 CFR Part 11 or IEC 62443): kept forever.' : 'retention.audit_keep_secs. Audit log, alarm audit and finished commands; forever unless set.'],
    ['Run history kept', formatDuration(cfg.run_log_keep_secs), 'retention.run_log_keep_secs. Flow runs and script executions.'],
  ]
}
