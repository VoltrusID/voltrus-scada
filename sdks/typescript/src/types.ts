// ── System ─────────────────────────────────────────────────────

export interface HealthResponse {
  status: string;
  /** device id → status ("online" | "offline") */
  devices: Record<string, string>;
}

export interface DeviceSnapshot {
  device_id: string;
  status: 'online' | 'offline';
  /** unix seconds */
  timestamp: number;
  metrics: Record<string, number>;
  simulated?: boolean;
  /** acquisition protocol: "modbus", "opcua", "ethernet_ip", "s7", "dnp3", "bacnet", … */
  protocol?: string;
}

export interface LiveResponse {
  devices: DeviceSnapshot[];
}

export interface ProcessInfo {
  pid: number;
  memory_mb: number;
  cpu_percent: number;
  uptime_secs: number;
  binary_size_mb: number;
}

export interface SystemInfo {
  hostname: string;
  os: string;
  uptime_secs: number;
  cpu_usage: number;
  memory_total_mb: number;
  memory_used_mb: number;
  memory_percent: number;
  disk_total_gb: number;
  disk_used_gb: number;
  disk_percent: number;
  db_size_mb: number;
  process: ProcessInfo;
}

export interface VersionInfo {
  version: string;
}

// ── Auth ───────────────────────────────────────────────────────

export interface LoginRequest {
  username: string;
  password: string;
}

/**
 * Body of a successful POST /api/v1/auth/login. The access token itself is
 * delivered as HttpOnly Set-Cookie headers (voltrus_token / voltrus_refresh),
 * not in the body — the client captures and replays them automatically.
 */
export interface LoginResponse {
  ok: boolean;
  username: string;
  role: string;
  must_change_password: boolean;
}

export interface AuthMe {
  authenticated: boolean;
  username?: string;
  role?: string;
}

export interface AuthProvider {
  provider: string;
  enabled: boolean;
  label: string;
}

export interface ChangePasswordRequest {
  old_password: string;
  new_password: string;
}

// ── Data / tags ────────────────────────────────────────────────

export interface DataQueryParams {
  screen?: string;
  device?: string;
  /** e.g. "30m", "1h", "1d" */
  range?: string;
  sensor?: string;
  device_id?: string;
  metric?: string;
  /** unix seconds */
  from?: number;
  /** unix seconds */
  to?: number;
  detail?: string;
  /** "sampling" (default) or another server-supported aggregation */
  agg?: string;
}

/** One evaluated data source reading from GET /api/v1/data. */
export interface DataSourceReading {
  id: number;
  key: string;
  name: string;
  value: number;
  warn: number;
  crit: number;
  source_type: string;
  status: string;
}

/** A data source definition (also used for screen sensor listings). */
export interface DataSource {
  id: number;
  key: string;
  name: string;
  unit: string;
  color: string;
  min_val: number;
  max_val: number;
  warn_val: number;
  crit_val: number;
  base_val: number;
  sort_order: number;
  decimals: number;
  source_type: string;
  source_config: string;
  device_id: string;
  equipment_id?: string;
  chart_type: string;
  path?: string;
}

export interface DataSourceSummary {
  id: number;
  key: string;
  name: string;
  unit: string;
  color: string;
  warn_val: number;
  crit_val: number;
  decimals: number;
  source_type: string;
}

export interface HistorySeries {
  values: number[];
  /** unix ms */
  start: number;
  /** ms between samples */
  interval: number;
  count: number;
}

export interface HistoryResponse {
  sensors: DataSourceSummary[];
  /** series keyed by sensor key */
  data: Record<string, HistorySeries>;
  range: string;
  interval: number;
}

export interface ExportQueryParams {
  /** ISO 8601 string or unix seconds */
  from?: string;
  to?: string;
  device?: string;
  metric?: string;
  /** avg | min | max | count | sum; default raw */
  aggregation?: string;
  /** 1m | 5m | 15m | 1h | 1d */
  interval?: string;
  limit?: number;
  offset?: number;
  /** json (default) | csv */
  format?: string;
}

export interface TelemetryExportRow {
  /** unix seconds */
  timestamp: number;
  device: string;
  metric: string;
  value: number;
}

export interface ExportMeta {
  total: number;
  limit: number;
  offset: number;
  aggregation: string;
  interval: string;
}

export interface ExportResponse {
  data: TelemetryExportRow[];
  meta: ExportMeta;
}

// ── Alarms ─────────────────────────────────────────────────────

export interface AlarmListParams {
  /** active | acknowledged | all */
  status?: string;
  priority?: string;
  state?: string;
  shelved?: boolean;
  limit?: number;
}

export interface Alarm {
  id: number;
  source_key: string;
  source_name: string;
  severity: string;
  priority: string;
  value: number;
  threshold: number;
  message: string;
  /** unix seconds */
  triggered_at: number;
  resolved_at?: number;
  acknowledged_at?: number;
  acknowledged_by?: string;
  cooldown_until?: number;
  shelved_until?: number;
  shelved_by?: string;
  shelved_reason?: string;
  deadband: number;
  state: string;
  /** "high" fires when value ≥ threshold, "low" when value ≤ threshold */
  alarm_type: 'high' | 'low';
}

export interface AlarmSummary {
  active_critical: number;
  active_warning: number;
  active_info: number;
  shelved: number;
  recent_24h: number;
  avg_ack_time_secs: number;
}

export interface AlarmAuditEntry {
  id: number;
  alarm_id: number;
  action: string;
  performed_by: string;
  details: string;
  /** unix seconds */
  created_at: number;
}

export interface AlarmAuditParams {
  alarm_id?: number;
  from?: number;
  to?: number;
  limit?: number;
}

export interface AlarmAnalyticsParams {
  from?: number;
  to?: number;
}

export interface ShelfAlarmRequest {
  duration_secs: number;
  reason: string;
}

// ── Commands (tag writes) ──────────────────────────────────────

export type CommandStatusValue =
  | 'pending'
  | 'queued'
  | 'sent'
  | 'acked'
  | 'failed'
  | 'expired'
  | 'cancelled';

export interface WriteRequest {
  value: number | string | boolean;
  /** default "immediate" */
  policy?: string;
  /** unix seconds; schedule the write instead of sending now */
  scheduled_at?: number;
  /** required when the tag's write policy demands a reason */
  reason?: string;
}

/** Response of POST /api/v1/tags/{id}/write (HTTP 202). */
export interface WriteQueuedResponse {
  command_id: string;
  status: 'pending';
  data_source_id: number;
  value: number;
  /** unix seconds */
  created_at: number;
}

export interface Command {
  id: string;
  data_source_id: number;
  device_id: number;
  protocol: string;
  value: string;
  status: CommandStatusValue;
  policy: string;
  scheduled_at?: number;
  retry_count: number;
  max_retries: number;
  error_message?: string;
  /** unix seconds */
  created_at: number;
  sent_at?: number;
  acked_at?: number;
  resolved_at?: number;
  created_by: string;
  readback_status?: string;
  readback_value?: number;
  readback_attempts: number;
  readback_ts?: string;
}

export interface CommandListParams {
  tag?: string;
  status?: CommandStatusValue;
  device?: string;
}

export interface CommandAuditEntry {
  id: number;
  command_id: string;
  old_status: string;
  new_status: string;
  changed_by: string;
  reason?: string;
  /** unix seconds */
  created_at: number;
}

export interface WritePolicy {
  writable: boolean;
  write_min?: number;
  write_max?: number;
  write_max_rate?: number;
  write_requires_reason: boolean;
  write_deadband?: number;
}

export interface WritePolicyUpdate {
  writable?: boolean;
  write_min?: number | null;
  write_max?: number | null;
  write_max_rate?: number | null;
  write_requires_reason?: boolean;
  write_deadband?: number | null;
}

// ── Screens ────────────────────────────────────────────────────

/** A screen ("device" in the storage model). */
export interface Screen {
  id: string;
  name: string;
  type: string;
  sort_order: number;
}

export interface AddScreenSensorRequest {
  data_source_id: number;
}

export interface UpdateScreenSensorRequest {
  sort_order: number;
}

// ── Devices: Modbus ────────────────────────────────────────────

export interface ModbusDevice {
  id: string;
  name: string;
  ip: string;
  port: number;
  unit_id: number;
  interval_ms: number;
  endianness: string;
  enabled: boolean;
  /** unix seconds */
  created_at: number;
  updated_at: number;
  register_count: number;
}

export interface ModbusDeviceInput {
  /** Unique device identifier, chosen by the client (e.g. "pm5560_main"). */
  id: string;
  name: string;
  ip: string;
  port?: number;
  unit_id?: number;
  interval_ms?: number;
  endianness?: string;
  enabled?: boolean;
}

export interface ModbusRegister {
  id: number;
  device_id: string;
  name: string;
  address: number;
  register_type: string;
  scale: number;
  register_area: string;
  /** `address` is a 0-based offset (0-65535) rather than a 5-digit reference such as 40001. */
  zero_based?: boolean;
}

/**
 * A register in a Modbus device request. Give `address` (a 0-based offset below 30001, or a 5-digit reference
 * from 30001 up, as before) or `offset` (explicit 0-based offset in `register_area`, 0-65535, valid above 30000).
 * A `ModbusRegister` returned by the API can be sent back unchanged.
 */
export interface ModbusRegisterInput {
  name: string;
  address?: number;
  offset?: number;
  zero_based?: boolean;
  register_type?: string;
  scale?: number;
  register_area?: string;
}

// ── Devices: OPC-UA ────────────────────────────────────────────

export interface OpcUaDevice {
  id: number;
  name: string;
  url: string;
  security_policy: string;
  security_mode: string;
  username?: string;
  interval_ms: number;
  enabled: boolean;
  /** unix seconds */
  created_at: number;
  node_count: number;
}

export interface OpcUaDeviceInput {
  name: string;
  url: string;
  security_policy?: string;
  security_mode?: string;
  username?: string;
  password?: string;
  interval_ms?: number;
  enabled?: boolean;
}

export interface OpcUaNode {
  id: number;
  device_id: number;
  node_id: string;
  name: string;
  display_name: string;
  data_type: string;
  /** unix seconds */
  created_at: number;
}

export interface OpcUaNodeInput {
  node_id: string;
  name: string;
  display_name?: string;
  data_type?: string;
}

// ── Devices: S7 ────────────────────────────────────────────────

export interface S7Device {
  id: number;
  name: string;
  url: string;
  rack: number;
  slot: number;
  interval_ms: number;
  enabled: boolean;
  /** unix seconds */
  created_at: number;
}

export interface S7DeviceInput {
  name: string;
  url: string;
  rack?: number;
  slot?: number;
  interval_ms?: number;
  enabled?: boolean;
}

export interface S7Tag {
  id: number;
  device_id: number;
  name: string;
  address: string;
}

// ── Devices: EtherNet/IP ───────────────────────────────────────

export interface EnipDevice {
  id: number;
  name: string;
  url: string;
  slot: number;
  interval_ms: number;
  enabled: boolean;
  assembly_instance: number;
  /** unix seconds */
  created_at: number;
}

export interface EnipDeviceInput {
  name: string;
  url: string;
  slot?: number;
  interval_ms?: number;
  enabled?: boolean;
}

export interface EnipTag {
  id: number;
  device_id: number;
  tag_name: string;
  name: string;
  display_name: string;
  data_type: string;
  assembly_offset: number;
  assembly_length: number;
  /** unix seconds */
  created_at: number;
}

export interface EnipTagInput {
  tag_name: string;
  name: string;
  display_name?: string;
  data_type?: string;
}

// ── Devices: DNP3 ──────────────────────────────────────────────

export interface Dnp3Device {
  id: number;
  name: string;
  ip: string;
  port: number;
  master_address: number;
  outstation_address: number;
  enable_unsolicited: boolean;
  integrity_poll_interval_secs: number;
  event_scan_interval_ms?: number;
  enabled: boolean;
  /** unix seconds */
  created_at: number;
  updated_at: number;
}

export interface Dnp3DeviceInput {
  name: string;
  ip: string;
  port?: number;
  master_address?: number;
  outstation_address?: number;
  enable_unsolicited?: boolean;
  integrity_poll_interval_secs?: number;
  event_scan_interval_ms?: number;
  enabled?: boolean;
}

export interface Dnp3Point {
  id: number;
  device_id: number;
  group_num: number;
  variation: number;
  point_index: number;
}

export interface Dnp3PointInput {
  group_num: number;
  variation: number;
  point_index: number;
}

// ── Devices: BACnet ────────────────────────────────────────────

export interface BacnetDevice {
  id: number;
  name: string;
  ip: string;
  port: number;
  device_instance: number;
  interval_ms: number;
  cov_enabled: boolean;
  cov_lifetime: number;
  enabled: boolean;
  /** unix seconds */
  created_at: number;
}

export interface BacnetDeviceInput {
  name: string;
  ip: string;
  port?: number;
  device_instance?: number;
  interval_ms?: number;
  cov_enabled?: boolean;
  cov_lifetime?: number;
  enabled?: boolean;
}

export interface BacnetObject {
  id: number;
  device_id: number;
  object_type: string;
  object_instance: number;
  property: string;
  name: string;
  display_name: string;
  cov_increment: number;
  /** unix seconds */
  created_at: number;
}

export interface BacnetObjectInput {
  object_type: string;
  object_instance: number;
  property: string;
  name: string;
  display_name?: string;
  cov_increment?: number;
}

// ── Devices: MQTT broker ───────────────────────────────────────

export interface MqttBrokerStatus {
  /** "running" | "stopped" | "starting" | error message */
  status: string;
  connected_clients: number;
  clients: unknown[];
  config?: Record<string, unknown>;
}

// ── OEE ────────────────────────────────────────────────────────

export interface OeeMetrics {
  availability: number;
  performance: number;
  quality: number;
  oee: number;
  total_running_secs: number;
  total_planned_secs: number;
  downtime_count: number;
}

export interface OeeMetricsParams {
  device_id: string;
  /** unix seconds */
  from?: number;
  to?: number;
}

export interface DowntimeListParams {
  device_id?: string;
  from?: number;
  to?: number;
}

export interface DowntimeEvent {
  id: number;
  device_id: string;
  state: string;
  /** unix seconds */
  started_at: number;
  ended_at?: number;
  duration_secs?: number;
  reason?: string;
}

export interface CreateDowntimeRequest {
  device_id: string;
  state: string;
}

export interface CloseDowntimeRequest {
  reason?: string;
}

export interface ShiftConfig {
  id: number;
  name: string;
  start_hour: number;
  start_minute: number;
  duration_hours: number;
  /** bitmask, bit 0 = Monday */
  days_mask: number;
  /** unix seconds */
  created_at: number;
}

export interface CreateShiftRequest {
  name: string;
  start_hour: number;
  start_minute: number;
  duration_hours: number;
  days_mask: number;
}

// ── Reports ────────────────────────────────────────────────────

export interface Report {
  id: number;
  name: string;
  schedule: string;
  enabled: boolean;
  device_ids: string;
  email_recipients: string;
  last_generated?: number;
  /** unix seconds */
  created_at: number;
}

export interface CreateReportRequest {
  name: string;
  schedule: string;
  device_ids?: string;
  email_recipients: string;
  enabled?: boolean;
}

export interface UpdateReportRequest {
  name?: string;
  schedule?: string;
  device_ids?: string;
  email_recipients?: string;
  enabled?: boolean;
}
