// Equipment instances in the Designer, without React: placing a symbol bound
// to an instance, finding an instance's symbols, suggesting names. The
// binding itself comes from the server (`resolved.widget`), so the Designer
// writes exactly what the server writes when it re-binds a saved screen.

import type { WidgetInstance } from '../scada2/widgets/types'
import type { EquipmentInstance, EquipmentType, ResolvedWidget } from '../queries/equipment'
import type { Patch } from './model'

/** Config key naming the equipment instance a symbol shows. */
export const EQUIPMENT_KEY = 'equipment'

/** The instance a widget is bound to, if any. */
export function instanceOf(w: Pick<WidgetInstance, 'config'>): string | null {
  const v = w.config?.[EQUIPMENT_KEY]
  return typeof v === 'string' && v ? v : null
}

/** Default tag prefix of an instance name, as the server computes it: "P-101A" -> "p101a". */
export function defaultPrefix(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]/g, '')
}

/**
 * The patch that binds a widget to an instance: symbol type, primary tag,
 * every config key the server sets (faceplate, its role map, fault/run tags,
 * the type's symbol config), the keys it removes, and a label when there is none.
 */
export function bindInstancePatch(w: Pick<WidgetInstance, 'config'> | null, name: string, r: ResolvedWidget): Patch {
  // The label is the engineer's: set only when the symbol has none.
  const { label: serverLabel, ...owned } = r.config
  const config: Record<string, any> = { ...owned, [EQUIPMENT_KEY]: name }
  for (const k of r.remove) if (!(k in owned) && k !== 'label') config[k] = undefined
  const label = w?.config?.label
  if (typeof label !== 'string' || !label) config.label = typeof serverLabel === 'string' && serverLabel ? serverLabel : name
  const tag = r.sensor_id ?? null
  return { type: 'symbol', sensorId: tag, sensorIds: tag != null ? [tag] : [], name: undefined, config }
}

/** A new widget's config from a patch (undefined keys dropped). */
export function configOf(p: Patch): Record<string, any> {
  return Object.fromEntries(Object.entries(p.config ?? {}).filter(([, v]) => v !== undefined))
}

/** Ids of the widgets on the screen bound to each instance. */
export function widgetsByInstance(widgets: WidgetInstance[]): Map<string, string[]> {
  const m = new Map<string, string[]>()
  for (const w of widgets) {
    const n = instanceOf(w)
    if (!n) continue
    if (!m.has(n)) m.set(n, [])
    m.get(n)!.push(w.id)
  }
  return m
}

/** Letters a type's instances usually start with: Pump -> P, Control valve -> CV, Analog instrument -> AI. */
export function typeInitials(t: Pick<EquipmentType, 'id' | 'name'>): string {
  const words = t.name.split(/[\s_-]+/).filter(Boolean)
  if (words.length === 1) return words[0][0].toUpperCase()
  return words.map(w => w[0].toUpperCase()).join('').slice(0, 3)
}

/**
 * A free name for the next instance of a type: after the last instance's name
 * (P-101A -> P-101B, P-204 -> P-205), or the type's initials and 101.
 */
export function suggestName(t: Pick<EquipmentType, 'id' | 'name'>, instances: Pick<EquipmentInstance, 'name' | 'type'>[]): string {
  const taken = new Set(instances.map(i => i.name.toLowerCase()))
  const own = instances.filter(i => i.type === t.id).map(i => i.name)
  const free = (n: string) => !taken.has(n.toLowerCase())
  const last = own[own.length - 1]
  if (last) {
    // Trailing letter after a number: next letter (P-101A -> P-101B).
    const letter = /^(.*\d)([A-Z])$/.exec(last)
    if (letter) {
      for (let c = letter[2].charCodeAt(0) + 1; c <= 90; c++) {
        const n = letter[1] + String.fromCharCode(c)
        if (free(n)) return n
      }
    }
    const num = /^(.*?)(\d+)([^\d]*)$/.exec(last)
    if (num) {
      for (let i = Number(num[2]) + 1; i < Number(num[2]) + 1000; i++) {
        const n = num[1] + String(i).padStart(num[2].length, '0') + (letter ? '' : num[3])
        if (free(n)) return n
      }
    }
  }
  const base = typeInitials(t)
  for (let i = 101; i < 1000; i++) if (free(`${base}-${i}`)) return `${base}-${i}`
  return `${base}-${Date.now() % 100000}`
}

/** Problems with a new instance's fields before it is sent (the server checks the rest). */
export function instanceFieldErrors(f: { name: string; device: string; base: string }, instances: Pick<EquipmentInstance, 'name'>[]): string[] {
  const out: string[] = []
  const name = f.name.trim()
  if (!name) out.push('Name is required')
  else if (instances.some(i => i.name.toLowerCase() === name.toLowerCase())) out.push(`An instance named ${name} exists; pick it from the list instead`)
  if (!f.device.trim()) out.push('Device is required')
  if (f.base.trim() && !/^\d+$/.test(f.base.trim())) out.push('Base must be a whole register address, or empty')
  return out
}
