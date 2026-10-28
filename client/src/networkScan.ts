// Helpers for the Modbus subnet scan (POST /api/v1/modbus-devices/scan-network).
// The server accepts an IPv4 range of at most /24 (254 hosts); these checks give the
// same answer before a request is made.

export interface DiscoveredRegister { address: number; value: number; raw: number[] }
export interface DiscoveredDevice {
  ip: string
  port: number
  unit_id: number
  responsive_registers: DiscoveredRegister[]
}

/** The shortest prefix (largest range) a scan accepts. */
export const MIN_SCAN_PREFIX = 24
/** Hosts one scan batch probes at a time on the server. */
export const SCAN_CONCURRENCY = 50

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/

function parseIpv4(text: string): number | null {
  const m = IPV4.exec(text.trim())
  if (!m) return null
  const parts = m.slice(1).map(Number)
  if (parts.some(p => p > 255)) return null
  return ((parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]) >>> 0
}

function formatIpv4(n: number): string {
  return [n >>> 24, (n >>> 16) & 255, (n >>> 8) & 255, n & 255].join('.')
}

/** Number of addresses a scan of `/prefix` probes: the usable hosts, 1 for a /32. */
export function hostCount(prefix: number): number {
  if (prefix >= 32) return 1
  return Math.max(0, 2 ** (32 - prefix) - 2)
}

/** Why `text` is not a range the scan accepts, or null when it is one. */
export function cidrError(text: string): string | null {
  const t = text.trim()
  if (!t) return 'Enter a range such as 192.168.1.0/24'
  const slash = t.indexOf('/')
  if (slash < 0) return 'Write the range as address/prefix, for example 192.168.1.0/24'
  if (parseIpv4(t.slice(0, slash)) === null) return `"${t.slice(0, slash)}" is not an IPv4 address`
  const prefixText = t.slice(slash + 1)
  if (!/^\d{1,2}$/.test(prefixText)) return `"${prefixText}" is not a prefix length`
  const prefix = Number(prefixText)
  if (prefix > 32) return 'The prefix length is 0 to 32'
  if (prefix < MIN_SCAN_PREFIX) {
    return `/${prefix} has ${hostCount(prefix).toLocaleString('en-US')} hosts; the largest range a scan accepts is /${MIN_SCAN_PREFIX} (254 hosts)`
  }
  if (prefix === 31) return '/31 has no usable host addresses; use /32 for one address or /30 or shorter'
  return null
}

/** Addresses the scan of a valid range probes, 0 when the range is not valid. */
export function scanSize(text: string): number {
  if (cidrError(text) !== null) return 0
  return hostCount(Number(text.trim().split('/')[1]))
}

/** Rough duration in seconds: batches of SCAN_CONCURRENCY hosts, each host tried with 3 unit ids at up to 0.5 s. */
export function estimatedSeconds(hosts: number): number {
  return Math.max(1, Math.ceil(hosts / SCAN_CONCURRENCY) * 1.5)
}

interface InterfaceLike { name: string; addresses: string[] }

const VIRTUAL_INTERFACE = /^(lo|docker|veth|br-|virbr|tun|tap|tailscale|zt|utun|vmnet|vboxnet)/i

/**
 * The /24 (or the interface's own smaller subnet) of the first real IPv4 address
 * the server has, from the interface list of GET /api/v1/networks. Null when none.
 */
export function defaultScanRange(interfaces: InterfaceLike[]): string | null {
  for (const nic of interfaces) {
    if (VIRTUAL_INTERFACE.test(nic.name)) continue
    for (const address of nic.addresses) {
      const slash = address.indexOf('/')
      const ip = parseIpv4(slash < 0 ? address : address.slice(0, slash))
      if (ip === null) continue
      const first = ip >>> 24
      const second = (ip >>> 16) & 255
      if (first === 127 || first === 0 || (first === 169 && second === 254)) continue
      let prefix = slash < 0 ? 24 : Number(address.slice(slash + 1))
      if (!Number.isInteger(prefix) || prefix > 30) continue
      if (prefix < MIN_SCAN_PREFIX) prefix = MIN_SCAN_PREFIX
      const mask = (~0 << (32 - prefix)) >>> 0
      return `${formatIpv4((ip & mask) >>> 0)}/${prefix}`
    }
  }
  return null
}

/** Whether a device at ip:port is already in the device list. */
export function isAlreadyAdded(found: DiscoveredDevice, existing: { ip: string; port: number }[]): boolean {
  return existing.some(d => d.ip === found.ip && d.port === found.port)
}

/** A device id for a discovered device that is not in `taken`: modbus-192-168-1-20, then -2, -3 on a clash. */
export function deviceIdFor(found: DiscoveredDevice, taken: ReadonlySet<string>): string {
  const base = `modbus-${found.ip.replace(/\./g, '-')}${found.port === 502 ? '' : `-${found.port}`}`
  const sameHost = found.unit_id === 1 ? base : `${base}-u${found.unit_id}`
  if (!taken.has(sameHost)) return sameHost
  for (let n = 2; ; n++) {
    if (!taken.has(`${sameHost}-${n}`)) return `${sameHost}-${n}`
  }
}

/**
 * The create request for a discovered device. The registers that answered the probe
 * become uint16 holding registers, by explicit 0-based offset, so the device
 * shows values as soon as it is added; the user edits names and types later.
 */
export function newDevicePayload(found: DiscoveredDevice, id: string, name: string) {
  return {
    id,
    name,
    ip: found.ip,
    port: found.port,
    unit_id: found.unit_id,
    interval_ms: 1000,
    endianness: 'big',
    registers: found.responsive_registers.map(r => ({
      name: `reg_${String(r.address).padStart(5, '0')}`,
      offset: r.address,
      register_type: 'uint16',
      register_area: 'holding',
      scale: 1,
    })),
  }
}
