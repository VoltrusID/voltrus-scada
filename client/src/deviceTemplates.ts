// Device template library: types for /api/v1/device-templates and the pure helpers the template
// panel uses (request bodies, search, labels). The format is documented in docs/device-templates.md;
// the server (src/device_templates/) is the authority, these checks only give an early, readable error.

export type TemplateProtocol = 'modbus' | 'bacnet'

/** One row of `GET /api/v1/device-templates`. */
export interface TemplateSummary {
  template_id: string
  manufacturer: string
  model: string
  protocol: TemplateProtocol
  description: string
  version: string
  verified: boolean
  /** `builtin`, `local` or `catalog:<url>`. */
  source: string
  license: string
  /** Registers (Modbus) or objects (BACnet). */
  item_count: number
  read_only: boolean
}

export interface TemplateRegister {
  name: string
  offset: number
  register_type: string
  register_area: string
  scale: number
  unit?: string
  description?: string
  writable?: boolean
}

export interface TemplateObject {
  object_type: string
  object_instance: number
  property: string
  name: string
  display_name?: string
  cov_increment?: number
  unit?: string
  description?: string
  writable?: boolean
}

/** `GET /api/v1/device-templates/{id}`: the template file plus `read_only`. */
export interface DeviceTemplateDetail extends Omit<TemplateSummary, 'item_count'> {
  format: string
  verified_notes: string
  quirks: string[]
  port?: number
  interval_ms?: number
  unit_id?: number
  endianness?: string
  registers?: TemplateRegister[]
  cov_enabled?: boolean
  cov_lifetime?: number
  objects?: TemplateObject[]
}

export interface CatalogEntry {
  template_id: string
  manufacturer: string
  model: string
  protocol: TemplateProtocol
  version: string
  verified: boolean
  path: string
  sha256: string
  /** Already imported from this catalog (set by the server). */
  installed: boolean
}

export interface CatalogResponse {
  url: string
  format: string
  templates: CatalogEntry[]
}

export interface TemplateFilter {
  q?: string
  manufacturer?: string
  protocol?: TemplateProtocol | ''
  /** `builtin`, `local` or `catalog`. */
  source?: string
}

export const DEFAULT_CATALOG_URL = 'https://raw.githubusercontent.com/VoltrusID/voltrus-device-templates/main/index.json'

const SLUG = /^[a-z0-9]+(-[a-z0-9]+)*$/
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z.-]+)?$/
const RESERVED = ['catalog', 'from-device']

/** Lowercase, every run of other characters one `-`; the same slug the server derives. */
export function slugify(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64).replace(/-+$/, '')
}

export function isValidTemplateId(id: string): boolean {
  return id.length > 0 && id.length <= 64 && SLUG.test(id) && !RESERVED.includes(id)
}

export function isValidVersion(v: string): boolean {
  return SEMVER.test(v)
}

/** `builtin` → Built-in, `local` → Local, `catalog:https://host/x/index.json` → Catalog (host). */
export function sourceLabel(source: string): string {
  if (source === 'builtin') return 'Built-in'
  if (source === 'local') return 'Local'
  if (source.startsWith('catalog:')) {
    try {
      return `Catalog (${new URL(source.slice('catalog:'.length)).host})`
    } catch {
      return 'Catalog'
    }
  }
  return source
}

/** The query string of `GET /api/v1/device-templates`, empty fields left out. */
export function filterQuery(f: TemplateFilter): string {
  const p = new URLSearchParams()
  for (const k of ['q', 'manufacturer', 'protocol', 'source'] as const) {
    const v = f[k]?.trim()
    if (v) p.set(k, v)
  }
  const s = p.toString()
  return s ? `?${s}` : ''
}

/** The same matching the server does, for filtering a loaded list while the user types. */
export function matchesFilter(t: TemplateSummary, f: TemplateFilter): boolean {
  const q = f.q?.trim().toLowerCase()
  if (q && ![t.template_id, t.manufacturer, t.model, t.description].some(x => x.toLowerCase().includes(q))) return false
  const m = f.manufacturer?.trim().toLowerCase()
  if (m && t.manufacturer.toLowerCase() !== m) return false
  if (f.protocol && t.protocol !== f.protocol) return false
  if (f.source) {
    if (f.source === 'catalog' ? !t.source.startsWith('catalog:') : t.source !== f.source) return false
  }
  return true
}

/** Which version or source of an id, as the `?version=&source=` query of the item routes. */
export function refQuery(t: { version: string; source: string }): string {
  return `?${new URLSearchParams({ version: t.version, source: t.source }).toString()}`
}

export function templateKey(t: { template_id: string; version: string; source: string }): string {
  return `${t.source}|${t.template_id}|${t.version}`
}

/** Catalog entries, filtered by text over id, manufacturer and model, hiding nothing else. */
export function filterCatalog(entries: CatalogEntry[], q: string, protocol: TemplateProtocol | ''): CatalogEntry[] {
  const needle = q.trim().toLowerCase()
  return entries.filter(e =>
    (!protocol || e.protocol === protocol) &&
    (!needle || [e.template_id, e.manufacturer, e.model].some(x => x.toLowerCase().includes(needle))))
}

/** One quirk per non-empty line. */
export function parseQuirks(text: string): string[] {
  return text.split('\n').map(l => l.trim()).filter(Boolean)
}

export interface FromDeviceForm {
  manufacturer: string
  model: string
  templateId: string
  version: string
  description: string
  verified: boolean
  verifiedNotes: string
  quirksText: string
  license: string
}

export interface Built<T> {
  body?: T
  /** Readable problems; `body` is set only when there are none. */
  errors: string[]
}

/** Body of `POST /api/v1/device-templates/from-device/{id}`. */
export function fromDeviceBody(f: FromDeviceForm, protocol: TemplateProtocol) {
  const errors: string[] = []
  const manufacturer = f.manufacturer.trim()
  const model = f.model.trim()
  if (!manufacturer) errors.push('Manufacturer is required')
  if (!model) errors.push('Model is required')
  const templateId = f.templateId.trim() || slugify(`${manufacturer} ${model}`)
  if (!isValidTemplateId(templateId)) errors.push(`Template id '${templateId}' must be lowercase letters, digits and single dashes (not "catalog" or "from-device")`)
  const version = f.version.trim() || '1.0.0'
  if (!isValidVersion(version)) errors.push(`Version '${version}' must look like 1.0.0`)
  if (f.verified && !f.verifiedNotes.trim()) errors.push('Say what was verified (firmware, site) or untick Verified')
  if (errors.length > 0) return { errors }
  return {
    errors,
    body: {
      manufacturer,
      model,
      template_id: templateId,
      version,
      description: f.description.trim(),
      verified: f.verified,
      verified_notes: f.verifiedNotes.trim(),
      quirks: parseQuirks(f.quirksText),
      license: f.license.trim() || 'CC-BY-4.0',
      protocol,
    },
  }
}

export interface InstantiateForm {
  name: string
  ip: string
  port: string
  /** Modbus unit id. */
  unitId: string
  /** BACnet device instance. */
  deviceInstance: string
  intervalMs: string
}

const intIn = (s: string, min: number, max: number): number | null => {
  if (!/^\d+$/.test(s.trim())) return null
  const n = Number(s)
  return n >= min && n <= max ? n : null
}

/** Body of `POST /api/v1/device-templates/{id}/instantiate`; a blank optional field keeps the template's value. */
export function instantiateBody(
  f: InstantiateForm,
  protocol: TemplateProtocol,
  pick?: { version: string; source: string },
) {
  const errors: string[] = []
  const name = f.name.trim()
  const ip = f.ip.trim()
  if (!name) errors.push('Name is required')
  if (!ip) errors.push('IP address is required')
  const body: Record<string, unknown> = { name, ip }
  if (f.port.trim()) {
    const port = intIn(f.port, 1, 65535)
    if (port === null) errors.push('Port must be 1-65535')
    else body.port = port
  }
  if (f.intervalMs.trim()) {
    const ms = intIn(f.intervalMs, 1, 3_600_000)
    if (ms === null) errors.push('Interval must be 1-3600000 ms')
    else body.interval_ms = ms
  }
  if (protocol === 'modbus' && f.unitId.trim()) {
    const unit = intIn(f.unitId, 0, 255)
    if (unit === null) errors.push('Unit id must be 0-255')
    else body.unit_id = unit
  }
  if (protocol === 'bacnet') {
    const inst = intIn(f.deviceInstance.trim() || '0', 0, 4194303)
    if (inst === null) errors.push('Device instance must be 0-4194303')
    else body.device_instance = inst
  }
  if (pick) {
    body.version = pick.version
    body.source = pick.source
  }
  return errors.length > 0 ? { errors } : { errors, body }
}
