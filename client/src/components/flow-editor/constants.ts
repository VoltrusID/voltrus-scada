import type { FlowNodeType } from '../../types'

export const NODE_WIDTH = 180
export const NODE_GAP = 48
export const NODE_HEIGHT = 120
export const CANVAS_PADDING = 60

export const NODE_META: Record<FlowNodeType, { label: string; color: string; icon: string; desc: string; defaultConfig: Record<string, any>; category: 'process' | 'output' }> = {
  filter:        { label: 'Filter',        color: '#22d3ee', icon: 'M3 6h18M3 12h18M3 18h18', desc: 'Pass only if condition is true', defaultConfig: { condition: 'payload.metrics.temp > 0' }, category: 'process' },
  transform:     { label: 'Transform',     color: '#60a5fa', icon: 'M23 4l-11 16-5-5-5 5M23 4h-6', desc: 'Map fields to new schema', defaultConfig: { map: { status: "if(payload.metrics.temp > 80, 'CRITICAL', 'OK')" } }, category: 'process' },
  threshold:     { label: 'Threshold',     color: '#fbbf24', icon: 'M12 2v20M2 12h20', desc: 'Check values against rules', defaultConfig: { field: 'payload.metrics.temp', rules: [{ gt: 80, set: { alarm: 'HIGH' } }] }, category: 'process' },
  lookup:        { label: 'Lookup',        color: '#c084fc', icon: 'M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z', desc: 'Map values via lookup table', defaultConfig: { field: 'payload.machine_code', output_field: 'payload.machine_name', table: { M1: 'Cutting' }, default: 'Unknown' }, category: 'process' },
  expression:    { label: 'Expression',    color: '#4ade80', icon: 'M9 7h6M9 11h6M9 15h6M5 3h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2z', desc: 'Compute new field from formula', defaultConfig: { output_field: 'payload.power_kw', expr: 'payload.metrics.voltage * payload.metrics.current * 0.001' }, category: 'process' },
  script:        { label: 'Script',        color: '#f97316', icon: 'M16 18l6-6-6-6M8 6l-6 6 6 6', desc: 'Run a QuickJS-WASM script', defaultConfig: { script_id: 0, inputs: [] }, category: 'process' },
  data_source_out: { label: 'Data Source',  color: '#a3e635', icon: 'M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5', desc: 'Write to virtual sensor', defaultConfig: { value_field: 'payload.power_kw', source_key: 'flow_power', name: 'Power', unit: 'kW', decimals: 2 }, category: 'output' },
  http_out:      { label: 'HTTP Webhook',  color: '#818cf8', icon: 'M10 13a5 5 0 007 0M4 15l3-3-3-3M20 15l-3-3 3-3', desc: 'POST to external URL', defaultConfig: { url: 'https://example.com/webhook', headers: {}, timeout_ms: 5000, retry_count: 3 }, category: 'output' },
  alarm_out:     { label: 'Alarm',         color: '#f87171', icon: 'M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0zM12 9v4M12 17h.01', desc: 'Trigger an alarm', defaultConfig: { severity: 'critical', source_key: 'flow_alert', message: 'Alert triggered' }, category: 'output' },
  write_out:     { label: 'Write',        color: '#34d399', icon: 'M5 12l5 5L20 7', desc: 'Write a value to a real device/sensor', defaultConfig: { data_source_id: 0, value_expr: 'payload.value', policy: 'immediate', reason: 'flow:write' }, category: 'output' },
  dsl:           { label: 'Transform DSL', color: '#2dd4bf', icon: 'M4 6h16M4 12h16M4 18h16', desc: 'Inline transform DSL expression', defaultConfig: { source: 'set payload.status = "OK"' }, category: 'process' },
}

export const TRIGGER_META: Record<string, { label: string; color: string; icon: string }> = {
  modbus: { label: 'Modbus', color: '#22d3ee', icon: 'M13 2L3 14h9l-1 8 10-12h-9l1-8z' },
  timer:  { label: 'Timer',  color: '#fbbf24', icon: 'M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z' },
  mqtt:   { label: 'MQTT',   color: '#c084fc', icon: 'M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071a9 9 0 0112.728 0M1.394 9.393a13 13 0 0121.212 0' },
}

export const TRIGGER_DEFAULTS: Record<string, Record<string, any>> = {
  modbus: { type: 'modbus', device_id: '*', metric: '', debounce_ms: 1000, on_change_only: false },
  timer:  { type: 'timer', interval_ms: 5000 },
  mqtt:   { type: 'mqtt', topic: 'sensors/+/temp', debounce_ms: 1000 },
}

export const PALETTE_CATEGORIES = [
  { key: 'input', label: 'INPUT', items: Object.entries(TRIGGER_META).map(([type, meta]) => ({ type, ...meta, isTrigger: true })) },
  { key: 'process', label: 'PROCESS', items: Object.entries(NODE_META).filter(([,m]) => m.category === 'process').map(([type, meta]) => ({ type, ...meta, isTrigger: false })) },
  { key: 'output', label: 'OUTPUT', items: Object.entries(NODE_META).filter(([,m]) => m.category === 'output').map(([type, meta]) => ({ type, ...meta, isTrigger: false })) },
]