// Device files from Modbus Poll and BACnet Explorer ("Export to SCADA" > Voltrus SCADA device).
//
// Modbus: the body of POST /api/v1/modbus-devices (src/api/modbus_devices.rs create_modbus_device) with its registers.
// A register carries `address` (legacy: 0-based below 30001, 5-digit reference from 30001) or `offset` (explicit 0-based
// offset within register_area, valid up to 65535); the explicit form is what Modbus Poll exports.
// BACnet: the body of POST /api/v1/bacnet/devices (CreateBacnetDeviceRequest) plus `objects`, each the body of
// POST /api/v1/bacnet/devices/{id}/objects (CreateBacnetObjectRequest).
// A file may hold one device, an array of devices, or { devices: [...] }.

/** Give `address` (a 0-based offset below 30001, or a 5-digit reference such as 40001) or `offset` (0-based, 0-65535, any area). */
export interface ModbusImportRegister {
  name: string
  address?: number
  offset?: number
  register_type: string
  register_area: string
  scale: number
}

export interface ModbusImportDevice {
  id: string
  name: string
  ip: string
  port: number
  unit_id: number
  interval_ms: number
  endianness: string
  registers: ModbusImportRegister[]
}

export interface BacnetImportObject {
  object_type: string
  object_instance: number
  property: string
  name: string
}

export interface BacnetImportDevice {
  name: string
  ip: string
  port: number
  device_instance: number
  interval_ms: number
  objects: BacnetImportObject[]
}

export type DeviceFileKind = 'modbus' | 'bacnet'

export interface ParsedDeviceFile<T> {
  devices: T[]
  /** One message per rejected device or register. */
  errors: string[]
}

export const MODBUS_TYPES = ['int16', 'uint16', 'int32', 'float32', 'float64']
export const MODBUS_AREAS = ['holding', 'input', 'coil', 'discrete']
export const BACNET_TYPES = [
  'analogInput', 'analogOutput', 'analogValue',
  'binaryInput', 'binaryOutput', 'binaryValue',
  'multiStateInput', 'multiStateOutput', 'multiStateValue',
]

type Json = Record<string, unknown>

const isObject = (v: unknown): v is Json => typeof v === 'object' && v !== null && !Array.isArray(v)
const isInt = (v: unknown, min: number, max: number): v is number => typeof v === 'number' && Number.isInteger(v) && v >= min && v <= max
const str = (v: unknown): string => (typeof v === 'string' ? v.trim() : '')

function deviceList(text: string): { list: unknown[]; error?: string } {
  let doc: unknown
  try {
    doc = JSON.parse(text)
  } catch (e) {
    return { list: [], error: `Not valid JSON: ${String(e)}` }
  }
  if (Array.isArray(doc)) return { list: doc }
  if (isObject(doc) && Array.isArray(doc.devices)) return { list: doc.devices }
  if (isObject(doc)) return { list: [doc] }
  return { list: [], error: 'Expected a device object, an array of devices or { "devices": [...] }' }
}

/** Which kind of device a parsed JSON object describes, from the fields only that kind has. */
export function deviceFileKind(text: string): DeviceFileKind | null {
  const { list } = deviceList(text)
  const first = list.find(isObject)
  if (!first) return null
  if (Array.isArray(first.registers)) return 'modbus'
  if (Array.isArray(first.objects) || 'device_instance' in first) return 'bacnet'
  return null
}

export function parseModbusDeviceFile(text: string): ParsedDeviceFile<ModbusImportDevice> {
  const { list, error } = deviceList(text)
  const out: ParsedDeviceFile<ModbusImportDevice> = { devices: [], errors: error ? [error] : [] }
  list.forEach((raw, i) => {
    const label = isObject(raw) && str(raw.name) ? str(raw.name) : `device ${i + 1}`
    if (!isObject(raw)) { out.errors.push(`${label}: not an object`); return }
    const id = str(raw.id)
    const ip = str(raw.ip)
    if (!id) { out.errors.push(`${label}: id is missing`); return }
    if (!ip) { out.errors.push(`${label}: ip is missing (a serial device needs the address of a Modbus TCP gateway)`); return }
    const port = raw.port ?? 502
    const unit = raw.unit_id ?? 1
    const interval = raw.interval_ms ?? 1000
    if (!isInt(port, 1, 65535)) { out.errors.push(`${label}: port ${String(port)} is not 1-65535`); return }
    if (!isInt(unit, 0, 255)) { out.errors.push(`${label}: unit_id ${String(unit)} is not 0-255`); return }
    if (!isInt(interval, 1, 3_600_000)) { out.errors.push(`${label}: interval_ms ${String(interval)} is not a positive number`); return }
    const endianness = str(raw.endianness) || 'big'
    if (!['big', 'little', 'swapped'].includes(endianness)) { out.errors.push(`${label}: endianness '${endianness}' is not big, little or swapped`); return }
    if (!Array.isArray(raw.registers)) { out.errors.push(`${label}: registers is missing`); return }
    const registers: ModbusImportRegister[] = []
    raw.registers.forEach((r, j) => {
      if (!isObject(r)) { out.errors.push(`${label}: register ${j + 1} is not an object`); return }
      const name = str(r.name)
      const type = str(r.register_type) || 'float32'
      const area = str(r.register_area) || 'holding'
      const scale = r.scale ?? 1
      if (!name) out.errors.push(`${label}: register ${j + 1} has no name`)
      else if (r.address !== undefined && r.offset !== undefined) out.errors.push(`${label}: ${name} has both address and offset`)
      else if (r.address === undefined && r.offset === undefined) out.errors.push(`${label}: ${name} has no address or offset`)
      else if (r.address !== undefined && !isInt(r.address, 0, 65535)) out.errors.push(`${label}: ${name} has address ${String(r.address)}, not 0-65535`)
      else if (r.offset !== undefined && !isInt(r.offset, 0, 65535)) out.errors.push(`${label}: ${name} has offset ${String(r.offset)}, not 0-65535`)
      else if (!MODBUS_TYPES.includes(type)) out.errors.push(`${label}: ${name} has type '${type}' (use ${MODBUS_TYPES.join(', ')})`)
      else if (!MODBUS_AREAS.includes(area)) out.errors.push(`${label}: ${name} has area '${area}' (use ${MODBUS_AREAS.join(', ')})`)
      else if (typeof scale !== 'number' || !Number.isFinite(scale)) out.errors.push(`${label}: ${name} has a scale that is not a number`)
      else registers.push(r.offset !== undefined ? { name, offset: r.offset as number, register_type: type, register_area: area, scale } : { name, address: r.address as number, register_type: type, register_area: area, scale })
    })
    out.devices.push({ id, name: str(raw.name) || id, ip, port, unit_id: unit, interval_ms: interval, endianness, registers })
  })
  return out
}

export function parseBacnetDeviceFile(text: string): ParsedDeviceFile<BacnetImportDevice> {
  const { list, error } = deviceList(text)
  const out: ParsedDeviceFile<BacnetImportDevice> = { devices: [], errors: error ? [error] : [] }
  list.forEach((raw, i) => {
    const label = isObject(raw) && str(raw.name) ? str(raw.name) : `device ${i + 1}`
    if (!isObject(raw)) { out.errors.push(`${label}: not an object`); return }
    const name = str(raw.name)
    const ip = str(raw.ip)
    if (!name) { out.errors.push(`${label}: name is missing`); return }
    if (!ip) { out.errors.push(`${label}: ip is missing`); return }
    const port = raw.port ?? 47808
    const instance = raw.device_instance ?? 0
    const interval = raw.interval_ms ?? 1000
    if (!isInt(port, 1, 65535)) { out.errors.push(`${label}: port ${String(port)} is not 1-65535`); return }
    if (!isInt(instance, 0, 4194303)) { out.errors.push(`${label}: device_instance ${String(instance)} is not 0-4194303`); return }
    if (!isInt(interval, 1, 3_600_000)) { out.errors.push(`${label}: interval_ms ${String(interval)} is not a positive number`); return }
    const objects: BacnetImportObject[] = []
    ;(Array.isArray(raw.objects) ? raw.objects : []).forEach((o, j) => {
      if (!isObject(o)) { out.errors.push(`${label}: object ${j + 1} is not an object`); return }
      const type = str(o.object_type)
      const oname = str(o.name)
      if (!BACNET_TYPES.includes(type)) out.errors.push(`${label}: object ${j + 1} has type '${type}' (use ${BACNET_TYPES.join(', ')})`)
      else if (!isInt(o.object_instance, 0, 4194303)) out.errors.push(`${label}: object ${j + 1} has instance ${String(o.object_instance)}, not 0-4194303`)
      else if (!oname) out.errors.push(`${label}: object ${j + 1} has no name`)
      else objects.push({ object_type: type, object_instance: o.object_instance, property: str(o.property) || 'presentValue', name: oname })
    })
    out.devices.push({ name, ip, port, device_instance: instance, interval_ms: interval, objects })
  })
  return out
}
