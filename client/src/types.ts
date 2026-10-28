export interface SensorConfig {
  id: number; key: string; name: string; unit: string; color: string;
  min_val: number; max_val: number; warn_val: number; crit_val: number;
  base_val: number; sort_order: number; decimals: number;
  source_type: string; source_config: string;
  device_id: string;
  equipment_id: string | null;
  chart_type: string;
  path: string | null;
  on_delay_secs: number;
  off_delay_secs: number;
}

export interface SensorData {
  id: number; key: string; value: number; warn: number; crit: number; source_type: string; status: string; simulated?: boolean;
}

export interface SystemInfo {
  hostname: string
  os: string
  uptime_secs: number
  cpu_usage: number
  memory_used_mb: number
  memory_total_mb: number
  memory_percent: number
  disk_used_gb: number
  disk_total_gb: number
  disk_percent: number
  db_size_mb: number
  process: {
    pid: number
    memory_mb: number
    cpu_percent: number
    uptime_secs: number
    binary_size_mb: number
  }
}

/** `lo`/`hi` are the bucket's min and max when the server answered `agg=minmax`. */
export interface DataPoint { value: number; ts: number; simulated?: boolean; lo?: number; hi?: number }

export interface Screen { id: string; name: string; type: string; sort_order: number }

export interface LayoutItem { i: string; x: number; y: number; w: number; h: number }

export type ScreenLayouts = { lg: LayoutItem[]; md: LayoutItem[]; sm: LayoutItem[]; xs: LayoutItem[] }

export interface CanvasLayoutItem {
  data_source_id: number
  x: number; y: number; width: number; height: number
}

export const LIVE_PTS = 180;
export const MAX_PTS = 300;
export const NORM_PTS = 60;
export const SOURCE_TYPES = ['random_walk', 'sine', 'step', 'sawtooth', 'constant', 'noise', 'api', 'modbus', 'none'] as const;

export const SIMULATE_TYPES = ['random_walk', 'sine', 'step', 'sawtooth', 'constant', 'noise'] as const;

export const SOURCE_CATEGORY: Record<string, { label: string; tag: string; color: string }> = {
  modbus:      { label: 'Modbus',        tag: 'REAL', color: 'var(--ok)' },
  random_walk: { label: 'Random Walk',   tag: 'SIM',  color: '#f59e0b' },
  sine:        { label: 'Sine Wave',     tag: 'SIM',  color: '#f59e0b' },
  step:        { label: 'Step / Spike',  tag: 'SIM',  color: '#f59e0b' },
  sawtooth:    { label: 'Sawtooth',      tag: 'SIM',  color: '#f59e0b' },
  constant:    { label: 'Constant',      tag: 'SIM',  color: '#f59e0b' },
  noise:       { label: 'Noise',         tag: 'SIM',  color: '#f59e0b' },
  api:         { label: 'API',           tag: 'API',  color: '#a78bfa' },
  none:        { label: 'None (static)', tag: 'NONE', color: 'var(--muted)' },
};

export const CHART_TYPES = ['sparkline', 'bignumber', 'gauge', 'image', 'text'] as const;

export type EquipmentType = 'solar_panel' | 'inverter' | 'gateway'

export interface TwinEquipment {
  id: string
  type: EquipmentType
  name: string
  position: [number, number, number]
  rotation: number
}

export interface TwinLayout {
  screen_id: string
  equipment_id: string
  equipment_type: EquipmentType
  position_x: number
  position_z: number
  rotation_y: number
}

export const EQUIPMENT_TYPES: { value: EquipmentType; label: string }[] = [
  { value: 'solar_panel', label: 'Solar Panel' },
  { value: 'inverter', label: 'Inverter' },
  { value: 'gateway', label: 'Gateway (PLC/HMI)' },
]

export interface Alarm {
  id: number
  source_key: string
  source_name: string
  severity: 'warning' | 'critical'
  priority: 'critical' | 'warning' | 'info'
  value: number
  threshold: number
  message: string
  triggered_at: number
  resolved_at: number | null
  acknowledged_at: number | null
  acknowledged_by: string | null
  cooldown_until: number | null
  shelved_until: number | null
  shelved_reason: string | null
  alarm_type?: 'high' | 'low'
}

export interface AlarmSummary {
  active_critical: number
  active_warning: number
  active_info: number
  shelved: number
  recent_24h: number
  avg_ack_time_secs: number
}

export interface AlarmAuditEntry {
  id: number
  alarm_id: number
  action: string
  performed_by: string
  details: string
  created_at: number
}

export interface AlarmJournalEntry {
  id: number
  alarm_id: number
  event: string
  at: number
  actor: string | null
}

export interface AlarmAnalyticsEntry {
  source_key: string
  source_name: string
  trigger_count: number
  avg_time_to_ack_secs: number
  last_triggered_at: number
}

export interface ModbusDevice {
  id: string
  name: string
  ip: string
  port: number
  unit_id: number
  interval_ms: number
  endianness: string
  enabled: boolean
  created_at: number
  updated_at: number
  register_count: number
}

export interface ModbusRegister {
  id: number
  device_id: string
  name: string
  address: number
  register_type: string
  register_area: string
  scale: number
  sort_order: number
  /** `address` is a 0-based offset, not a 5-digit reference. */
  zero_based?: boolean
}

// ── SCADA 2D view types ──

export type ScadaEquipmentType =
  | 'solar_panel' | 'inverter' | 'gateway' | 'transformer' | 'battery' | 'meter' | 'circuit_breaker'
  | 'busbar' | 'isolator' | 'earth' | 'fuse' | 'capacitor' | 'generator' | 'motor'
  | 'surge_arrester' | 'current_transformer' | 'voltage_transformer' | 'contactor' | 'relay' | 'load'
  | 'valve' | 'pump' | 'tank' | 'heat_exchanger' | 'compressor' | 'pipe' | 'pipe_elbow' | 'pipe_tee'
  | 'sensor' | 'transmitter' | 'controller' | 'indicator' | 'analyzer'
  | 'rectangle' | 'circle' | 'diamond' | 'arrow' | 'label'
export type ScadaConnectionType = 'power' | 'data' | 'flow'

export const SCADA_EQUIPMENT_TYPES: { value: ScadaEquipmentType; label: string; defaultWidth: number; defaultHeight: number }[] = [
  { value: 'solar_panel', label: 'Solar Panel', defaultWidth: 100, defaultHeight: 60 },
  { value: 'inverter', label: 'Inverter', defaultWidth: 80, defaultHeight: 60 },
  { value: 'gateway', label: 'Gateway/PLC', defaultWidth: 80, defaultHeight: 70 },
  { value: 'transformer', label: 'Transformer', defaultWidth: 90, defaultHeight: 80 },
  { value: 'battery', label: 'Battery', defaultWidth: 70, defaultHeight: 80 },
  { value: 'meter', label: 'Meter', defaultWidth: 60, defaultHeight: 60 },
  { value: 'circuit_breaker', label: 'Circuit Breaker', defaultWidth: 60, defaultHeight: 70 },
  { value: 'busbar', label: 'Busbar', defaultWidth: 120, defaultHeight: 30 },
  { value: 'isolator', label: 'Isolator', defaultWidth: 60, defaultHeight: 60 },
  { value: 'earth', label: 'Earth / Ground', defaultWidth: 40, defaultHeight: 60 },
  { value: 'fuse', label: 'Fuse', defaultWidth: 60, defaultHeight: 50 },
  { value: 'capacitor', label: 'Capacitor Bank', defaultWidth: 60, defaultHeight: 60 },
  { value: 'generator', label: 'Generator', defaultWidth: 70, defaultHeight: 70 },
  { value: 'motor', label: 'Motor / Load', defaultWidth: 70, defaultHeight: 70 },
  { value: 'surge_arrester', label: 'Surge Arrester', defaultWidth: 50, defaultHeight: 70 },
  { value: 'current_transformer', label: 'CT', defaultWidth: 60, defaultHeight: 50 },
  { value: 'voltage_transformer', label: 'PT / VT', defaultWidth: 60, defaultHeight: 50 },
  { value: 'contactor', label: 'Contactor', defaultWidth: 60, defaultHeight: 60 },
  { value: 'relay', label: 'Relay', defaultWidth: 60, defaultHeight: 60 },
  { value: 'load', label: 'Load / Resistor', defaultWidth: 60, defaultHeight: 60 },
  { value: 'valve', label: 'Valve', defaultWidth: 80, defaultHeight: 60 },
  { value: 'pump', label: 'Pump', defaultWidth: 80, defaultHeight: 70 },
  { value: 'tank', label: 'Tank', defaultWidth: 80, defaultHeight: 80 },
  { value: 'heat_exchanger', label: 'Heat Exchanger', defaultWidth: 90, defaultHeight: 70 },
  { value: 'compressor', label: 'Compressor', defaultWidth: 80, defaultHeight: 70 },
  { value: 'pipe', label: 'Pipe', defaultWidth: 100, defaultHeight: 30 },
  { value: 'pipe_elbow', label: 'Pipe Elbow', defaultWidth: 60, defaultHeight: 60 },
  { value: 'pipe_tee', label: 'Pipe Tee', defaultWidth: 100, defaultHeight: 60 },
  { value: 'sensor', label: 'Sensor (ISA)', defaultWidth: 60, defaultHeight: 60 },
  { value: 'transmitter', label: 'Transmitter (ISA)', defaultWidth: 80, defaultHeight: 60 },
  { value: 'controller', label: 'Controller (ISA)', defaultWidth: 60, defaultHeight: 60 },
  { value: 'indicator', label: 'Indicator / Display', defaultWidth: 60, defaultHeight: 60 },
  { value: 'analyzer', label: 'Analyzer (ISA)', defaultWidth: 60, defaultHeight: 60 },
  { value: 'rectangle', label: 'Rectangle', defaultWidth: 60, defaultHeight: 60 },
  { value: 'circle', label: 'Circle', defaultWidth: 60, defaultHeight: 60 },
  { value: 'diamond', label: 'Diamond', defaultWidth: 60, defaultHeight: 60 },
  { value: 'arrow', label: 'Arrow', defaultWidth: 80, defaultHeight: 40 },
  { value: 'label', label: 'Text Label', defaultWidth: 80, defaultHeight: 40 },
]

export interface ScadaEquipmentItem {
  equipment_id: string
  equipment_type: ScadaEquipmentType
  equipment_name: string
  x: number
  y: number
  width: number
  height: number
  rotation: number
  sensor_bindings: string
  config: string
}

export interface ScadaConnectionItem {
  connection_id: string
  from_equipment: string
  to_equipment: string
  waypoints: string
  connection_type: ScadaConnectionType
}

export interface UserItem {
  id: number
  username: string
  role: string
}

// ── Flow Engine types ──

export interface Flow {
  id: string
  name: string
  enabled: boolean
  trigger_config: string
  nodes: string
  created_at: number
  updated_at: number
}

export interface FlowRun {
  id: number
  flow_id: string
  started_at: number
  duration_ms: number
  success: boolean
  error: string | null
  output: string | null
}

export interface FlowTestResult {
  success: boolean
  output: Record<string, any>
  node_results: FlowNodeResult[]
}

export interface FlowNodeResult {
  node_id: string
  result?: string
  output?: Record<string, any>
  error?: string
}

export type FlowNodeType =
  | 'filter'
  | 'transform'
  | 'threshold'
  | 'lookup'
  | 'expression'
  | 'script'
  | 'data_source_out'
  | 'http_out'
  | 'alarm_out'
  | 'write_out'
  | 'dsl'

export type FlowTriggerType = 'modbus' | 'timer' | 'mqtt'

export interface FlowNodeConfig {
  id: string
  type: FlowNodeType
  [key: string]: any
}

export interface FlowTriggerConfig {
  type: FlowTriggerType
  [key: string]: any
}

// -- Notification Settings types --

export interface NotificationSetting {
  id: number
  source_key: string
  channel_type: 'email' | 'webhook' | 'push' | 'telegram' | 'whatsapp'
  // JSON string: email/webhook/push use string arrays; telegram and whatsapp use an object. The server
  // masks their token as '********' in responses.
  config: string
  enabled: boolean
  created_at: number
}

// WebSocket message types
export interface WsTagUpdate {
  tag_id: string
  value: number
  quality: string
  ts: number
  simulated: boolean
}

export interface WsCommandAck {
  command_id: string
  status: string
  device_response?: string
  latency_ms?: number
}

export interface WsDeviceStatus {
  device_id: string
  online: boolean
  latency_ms?: number
}

export interface WsAlarmEvent {
  alarm_id: number
  tag_id: string
  severity: string
  message: string
  value: number
  ts: number
}

export interface AiSettings {
  provider: string
  api_key: string
  api_key_set: boolean
  api_key_preview: string
  default_model: string
  enabled: boolean
}

export interface AiProposal {
  id: number
  proposal_type: string
  title: string
  description: string
  status: string
  proposed_config: any
  applied_config: any
  applied_at: number | null
  reviewed_at: number | null
  reviewed_by: string | null
  model_used: string | null
  tokens_used: number | null
  error_message: string | null
  created_at: number
  updated_at: number
}

export interface EquipmentTemplate {
  id: number
  name: string
  industry: string
  equipment_type: string
  description: string
  registers: any[]
}
