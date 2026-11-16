import type { FlowCatalogue, FlowField, FlowNodeTypeDef, FlowTriggerType } from '../../types'

/**
 * Presentation for the node catalogue. The catalogue itself (types, fields,
 * outputs) comes from `GET /api/v1/flows/node-types`; this file only adds
 * colours and icons, plus a small fallback for servers without that endpoint.
 */

export const CATEGORY_ORDER = ['logic', 'transform', 'timing', 'output', 'advanced'] as const

export const CATEGORY_META: Record<string, { label: string; color: string; icon: string }> = {
  trigger: { label: 'Trigger', color: '#2dd4bf', icon: 'M13 2L3 14h9l-1 8 10-12h-9l1-8z' },
  logic: { label: 'Logic', color: '#60a5fa', icon: 'M6 3v12M18 9a3 3 0 100-6 3 3 0 000 6zM6 21a3 3 0 100-6 3 3 0 000 6zM18 9a9 9 0 01-9 9' },
  transform: { label: 'Transform', color: '#a78bfa', icon: 'M4 7h16M4 7l4-4M4 7l4 4M20 17H4M20 17l-4-4M20 17l-4 4' },
  timing: { label: 'Timing', color: '#fbbf24', icon: 'M12 8v4l3 3M21 12a9 9 0 11-18 0 9 9 0 0118 0z' },
  output: { label: 'Output', color: '#22c55e', icon: 'M22 2L11 13M22 2l-7 20-4-9-9-4 20-7z' },
  advanced: { label: 'Advanced', color: '#f97316', icon: 'M16 18l6-6-6-6M8 6l-6 6 6 6' },
}

const ICONS: Record<string, string> = {
  filter: 'M22 3H2l8 9.46V19l4 2v-8.54L22 3z',
  switch: 'M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5',
  transform: 'M4 7h16M4 7l4-4M4 7l4 4M20 17H4M20 17l-4-4M20 17l-4 4',
  change: 'M12 20h9M16.5 3.5a2.12 2.12 0 013 3L7 19l-4 1 1-4L16.5 3.5z',
  set: 'M12 20h9M16.5 3.5a2.12 2.12 0 013 3L7 19l-4 1 1-4L16.5 3.5z',
  threshold: 'M3 12h4l3-9 4 18 3-9h4',
  lookup: 'M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z',
  expression: 'M9 7h6M9 11h6M9 15h6M5 3h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2z',
  script: 'M16 18l6-6-6-6M8 6l-6 6 6 6',
  function: 'M16 18l6-6-6-6M8 6l-6 6 6 6',
  dsl: 'M4 6h16M4 12h16M4 18h10',
  delay: 'M12 8v4l3 3M21 12a9 9 0 11-18 0 9 9 0 0118 0z',
  debounce: 'M12 8v4l3 3M21 12a9 9 0 11-18 0 9 9 0 0118 0z',
  throttle: 'M5 12h14M12 5l7 7-7 7',
  rate_limit: 'M5 12h14M12 5l7 7-7 7',
  data_source_out: 'M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5',
  http_out: 'M10 13a5 5 0 007.54.54l3-3a5 5 0 00-7.07-7.07l-1.72 1.71M14 11a5 5 0 00-7.54-.54l-3 3a5 5 0 007.07 7.07l1.71-1.71',
  http_request: 'M10 13a5 5 0 007.54.54l3-3a5 5 0 00-7.07-7.07l-1.72 1.71M14 11a5 5 0 00-7.54-.54l-3 3a5 5 0 007.07 7.07l1.71-1.71',
  alarm_out: 'M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0zM12 9v4M12 17h.01',
  write_out: 'M5 12l5 5L20 7',
  mqtt_out: 'M5 12.55a11 11 0 0114.08 0M1.42 9a16 16 0 0121.16 0M8.53 16.11a6 6 0 016.95 0M12 20h.01',
  notify: 'M18 8A6 6 0 006 8c0 7-3 9-3 9h18s-3-2-3-9M13.73 21a2 2 0 01-3.46 0',
  email_out: 'M4 4h16v16H4zM22 6l-10 7L2 6',
  log: 'M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8zM14 2v6h6M16 13H8M16 17H8',
  debug: 'M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8zM14 2v6h6M16 13H8M16 17H8',
  // triggers
  modbus: 'M13 2L3 14h9l-1 8 10-12h-9l1-8z',
  timer: 'M12 8v4l3 3M21 12a9 9 0 11-18 0 9 9 0 0118 0z',
  cron: 'M8 2v4M16 2v4M3 10h18M5 4h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V6a2 2 0 012-2z',
  mqtt: 'M5 12.55a11 11 0 0114.08 0M1.42 9a16 16 0 0121.16 0M8.53 16.11a6 6 0 016.95 0M12 20h.01',
  webhook: 'M18 16.98h-5.99c-1.1 0-1.95.94-2.48 1.9A4 4 0 012 17c.01-.7.2-1.4.57-2M6 17l3.13-5.78c.53-.97.1-2.18-.5-3.1a4 4 0 116.89-4.06M12 6l3.13 5.73C15.66 12.7 16.9 13 18 13a4 4 0 010 8',
  manual: 'M5 3l14 9-14 9V3z',
  tag_change: 'M20.59 13.41l-7.17 7.17a2 2 0 01-2.83 0L2 12V2h10l8.59 8.59a2 2 0 010 2.82zM7 7h.01',
  alarm: 'M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0zM12 9v4M12 17h.01',
}

export function iconFor(type: string, category?: string): string {
  return ICONS[type] ?? CATEGORY_META[category ?? '']?.icon ?? CATEGORY_META.advanced.icon
}

export function colorFor(category?: string): string {
  return CATEGORY_META[category ?? '']?.color ?? CATEGORY_META.advanced.color
}

// ── Fallback catalogue (server without /flows/node-types) ──

function f(key: string, label: string, kind: string, def?: unknown, extra: Partial<FlowField> = {}): FlowField {
  return { key, label, kind, default: def, ...extra }
}

const FALLBACK_TRIGGERS: FlowTriggerType[] = [
  { type: 'modbus', label: 'Modbus', description: 'Runs when a Modbus device reports new data', fields: [
    f('device_id', 'Device', 'device', '*', { required: true, help: '* matches every device' }),
    f('metric', 'Metric', 'string', ''),
    f('debounce_ms', 'Debounce', 'duration_ms', 1000),
    f('on_change_only', 'On change only', 'bool', false),
  ] },
  { type: 'timer', label: 'Timer', description: 'Runs on a fixed interval', fields: [
    f('interval_ms', 'Interval', 'duration_ms', 5000, { required: true }),
  ] },
  { type: 'mqtt', label: 'MQTT', description: 'Runs when a message arrives on a topic', fields: [
    f('topic', 'Topic', 'string', 'sensors/+/temp', { required: true }),
    f('debounce_ms', 'Debounce', 'duration_ms', 1000),
  ] },
]

const FALLBACK_NODES: FlowNodeTypeDef[] = [
  { type: 'filter', label: 'Filter', category: 'logic', description: 'Pass only if the condition is true', outputs: ['pass', 'reject'], fields: [
    f('condition', 'Condition', 'expression', 'payload.metrics.temp > 0', { required: true }),
  ] },
  { type: 'threshold', label: 'Threshold', category: 'logic', description: 'Check a value against rules', outputs: ['out'], fields: [
    f('field', 'Field', 'string', 'payload.metrics.temp', { required: true }),
    f('rules', 'Rules', 'json', [{ gt: 80, set: { alarm: 'HIGH' } }]),
  ] },
  { type: 'transform', label: 'Transform', category: 'transform', description: 'Map fields to a new shape', outputs: ['out'], fields: [
    f('map', 'Mapping', 'json', { status: "if(payload.metrics.temp > 80, 'CRITICAL', 'OK')" }, { required: true }),
  ] },
  { type: 'lookup', label: 'Lookup', category: 'transform', description: 'Map values through a table', outputs: ['out'], fields: [
    f('field', 'Field', 'string', 'payload.machine_code', { required: true }),
    f('output_field', 'Output field', 'string', 'payload.machine_name', { required: true }),
    f('table', 'Table', 'json', { M1: 'Cutting' }),
    f('default', 'Default', 'string', 'Unknown'),
  ] },
  { type: 'expression', label: 'Expression', category: 'transform', description: 'Compute a field from a formula', outputs: ['out'], fields: [
    f('output_field', 'Output field', 'string', 'payload.power_kw', { required: true }),
    f('expr', 'Expression', 'expression', 'payload.metrics.voltage * payload.metrics.current * 0.001', { required: true }),
  ] },
  { type: 'dsl', label: 'Transform DSL', category: 'transform', description: 'Inline transform DSL', outputs: ['out'], fields: [
    f('source', 'Source', 'code', 'set payload.status = "OK"', { required: true }),
  ] },
  { type: 'script', label: 'Script', category: 'advanced', description: 'Run a saved script', outputs: ['out'], fields: [
    f('script_id', 'Script ID', 'number', 0, { required: true }),
    f('inputs', 'Inputs', 'json', []),
  ] },
  { type: 'data_source_out', label: 'Data Source', category: 'output', description: 'Write to a virtual tag', outputs: ['out'], side_effect: true, fields: [
    f('value_field', 'Value field', 'string', 'payload.power_kw', { required: true }),
    f('source_key', 'Source key', 'string', 'flow_power', { required: true }),
    f('name', 'Name', 'string', 'Power'),
    f('unit', 'Unit', 'string', 'kW'),
    f('decimals', 'Decimals', 'number', 2),
  ] },
  { type: 'http_out', label: 'HTTP Webhook', category: 'output', description: 'POST to an external URL', outputs: ['out'], side_effect: true, fields: [
    f('url', 'URL', 'string', 'https://example.com/webhook', { required: true }),
    f('headers', 'Headers', 'json', {}),
    f('timeout_ms', 'Timeout', 'duration_ms', 5000),
    f('retry_count', 'Retries', 'number', 3),
  ] },
  { type: 'alarm_out', label: 'Alarm', category: 'output', description: 'Raise an alarm', outputs: ['out'], side_effect: true, fields: [
    f('severity', 'Severity', 'severity', 'critical', { required: true }),
    f('source_key', 'Source key', 'string', 'flow_alert', { required: true }),
    f('message', 'Message', 'string', 'Alert triggered'),
  ] },
  { type: 'write_out', label: 'Write', category: 'output', description: 'Write a value to a device tag', outputs: ['out'], side_effect: true, fields: [
    f('data_source_id', 'Tag', 'tag', 0, { required: true }),
    f('value_expr', 'Value', 'expression', 'payload.value', { required: true }),
    f('policy', 'Policy', 'select', 'immediate', { options: [{ value: 'immediate', label: 'Immediate' }, { value: 'queued', label: 'Queued' }] }),
    f('reason', 'Reason', 'string', 'flow:write'),
  ] },
]

export const FALLBACK_CATALOGUE: FlowCatalogue = { triggers: FALLBACK_TRIGGERS, nodes: FALLBACK_NODES }

export const SEVERITY_OPTIONS = [
  { value: 'critical', label: 'Critical' },
  { value: 'warning', label: 'Warning' },
  { value: 'info', label: 'Info' },
]

/** Default config of a node or trigger: every field's `default`. */
export function defaultConfig(fields: FlowField[]): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const fd of fields) {
    if (fd.default !== undefined) out[fd.key] = structuredClone(fd.default)
  }
  return out
}

export function findNodeDef(cat: FlowCatalogue, type: string): FlowNodeTypeDef | undefined {
  return cat.nodes.find(n => n.type === type)
}

export function findTriggerDef(cat: FlowCatalogue, type: string): FlowTriggerType | undefined {
  return cat.triggers.find(t => t.type === type)
}
