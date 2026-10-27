// Request bodies and response parsing for the S7, EtherNet/IP and OPC UA device APIs, in the
// shapes the server takes (src/models/protocol_device_types.rs). Kept free of React and fetch so
// the contract can be tested.

export const S7_DEFAULT_PORT = 102
export const ENIP_DEFAULT_PORT = 44818

/** `host:port`, the `url` the S7 and EtherNet/IP devices connect to. */
export function joinHostPort(host: string, port: number): string {
  return `${host.trim()}:${port}`
}

/** Splits a stored `host:port` url; a url without a numeric port keeps `defaultPort`. */
export function splitHostPort(url: string, defaultPort: number): { host: string; port: number } {
  const i = url.lastIndexOf(':')
  const tail = i >= 0 ? url.slice(i + 1) : ''
  if (i > 0 && /^\d+$/.test(tail)) return { host: url.slice(0, i), port: Number(tail) }
  return { host: url, port: defaultPort }
}

// ── S7 ──

export interface S7DeviceForm {
  name: string
  host: string
  port: number
  rack: number
  slot: number
  enabled: boolean
}

/** Body of `POST /api/v1/s7/devices` and `PUT /api/v1/s7/devices/{id}`. */
export function s7DeviceBody(f: S7DeviceForm) {
  return {
    name: f.name.trim() || f.host.trim(),
    url: joinHostPort(f.host, f.port),
    rack: f.rack,
    slot: f.slot,
    enabled: f.enabled,
  }
}

export interface S7TagForm {
  name: string
  tag_address: string
  data_type: string
  display_name?: string
}

/** Body of `POST /api/v1/s7/devices/{id}/tags`; `tag_address` is e.g. `DB1.DBD0` or `M0.0`. */
export function s7TagBody(f: S7TagForm) {
  return {
    tag_address: f.tag_address.trim(),
    name: f.name.trim(),
    display_name: f.display_name?.trim() ?? '',
    data_type: f.data_type,
  }
}

// ── EtherNet/IP ──

export interface EnipDeviceForm {
  name: string
  host: string
  port: number
  slot: number
  assembly_instance: number
  enabled: boolean
}

/** Body of `POST /api/v1/enip/devices` and `PUT /api/v1/enip/devices/{id}`. */
export function enipDeviceBody(f: EnipDeviceForm) {
  return {
    name: f.name.trim() || f.host.trim(),
    url: joinHostPort(f.host, f.port),
    slot: f.slot,
    assembly_instance: f.assembly_instance,
    enabled: f.enabled,
  }
}

export interface EnipTagForm {
  name: string
  tag_name: string
  data_type: string
  display_name?: string
  assembly_offset?: number
  assembly_length?: number
}

/** Body of `POST /api/v1/enip/devices/{id}/tags`; `tag_name` is the CIP symbol. */
export function enipTagBody(f: EnipTagForm) {
  return {
    tag_name: f.tag_name.trim(),
    name: f.name.trim(),
    display_name: f.display_name?.trim() ?? '',
    data_type: f.data_type,
    assembly_offset: f.assembly_offset ?? 0,
    assembly_length: f.assembly_length ?? 0,
  }
}

// ── OPC UA ──

/** The server's enum spellings: `None`, `basic256sha256`. */
export const OPCUA_POLICIES = [
  { value: 'None', label: 'None' },
  { value: 'basic256sha256', label: 'Basic256Sha256' },
] as const
/** The server's enum spellings: `None`, `Sign`, `signandencrypt`. */
export const OPCUA_MODES = [
  { value: 'None', label: 'None' },
  { value: 'Sign', label: 'Sign' },
  { value: 'signandencrypt', label: 'Sign & Encrypt' },
] as const

export type OpcUaSecurityPolicy = (typeof OPCUA_POLICIES)[number]['value']
export type OpcUaSecurityMode = (typeof OPCUA_MODES)[number]['value']

export interface OpcUaDeviceForm {
  name: string
  url: string
  security_policy: string
  security_mode: string
  username: string
  password: string
  interval_ms: number
}

function pick<T extends string>(options: readonly { value: T }[], value: string): T {
  return options.find(o => o.value === value)?.value ?? options[0].value
}

/** Body of `POST /api/v1/opcua/devices`; an unknown policy or mode falls back to `None`. */
export function opcuaDeviceBody(f: OpcUaDeviceForm) {
  return {
    name: f.name.trim(),
    url: f.url.trim(),
    security_policy: pick(OPCUA_POLICIES, f.security_policy),
    security_mode: pick(OPCUA_MODES, f.security_mode),
    username: f.username || null,
    password: f.password || null,
    interval_ms: f.interval_ms || 1000,
  }
}

/** `POST /api/v1/opcua/devices/{id}/browse` answers `{nodes: [...], count}`. */
export function opcuaBrowseNodes<T>(answer: { nodes?: T[] } | null | undefined): T[] {
  return answer?.nodes ?? []
}

// ── Shared ──

/** A route that answers a bare JSON array (BACnet objects); anything else is a contract error. */
export function bareList<T>(answer: unknown): T[] {
  if (!Array.isArray(answer)) throw new Error('Expected a list from the server')
  return answer as T[]
}

/** `{ok, id}` answered by the create routes (S7, EtherNet/IP and OPC UA ids are numbers). */
export function createdId(answer: unknown): number {
  const id = (answer as { id?: unknown } | null)?.id
  if (typeof id !== 'number') throw new Error('The server did not return the new id')
  return id
}
