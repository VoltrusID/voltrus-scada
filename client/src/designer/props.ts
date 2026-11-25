import type { ConfigFieldDef, WidgetInstance } from '../scada2/widgets/types'
import { WIDGET_MANIFESTS } from '../scada2/widgets/registry'
import { symbolSchema } from '../scada2/widgets/SymbolWidget'
import { resolveSymbol } from '../scada2/hmi/resolve'
import { COMMON_PROPS, type PropType } from './bindings'

/** Everything the property editor lists for a widget: its own schema, then the common properties. */
export function propertiesOf(w: Pick<WidgetInstance, 'type' | 'config'>): ConfigFieldDef[] {
  const manifest = WIDGET_MANIFESTS[w.type]
  const resolved = resolveSymbol(w)
  let schema: ConfigFieldDef[] = manifest?.configSchema ?? []
  if (resolved) {
    schema = symbolSchema(resolved.def, { legacy: w.type !== 'symbol', defaultWritable: resolved.defaultWritable })
    if (w.type === 'pipe') schema = [...schema, ...(manifest?.configSchema ?? []).filter(f => f.key === 'direction')]
  }
  // Labels without a schema field still bind: the faceplate title and saved name read config.label.
  const out = [...schema]
  if (!out.some(f => f.key === 'label')) out.unshift({ key: 'label', label: 'Label', type: 'string', default: '' })
  for (const c of COMMON_PROPS) out.push({ key: c.key, label: c.label, type: c.type as ConfigFieldDef['type'], default: c.default })
  return out
}

const typeCache = new Map<string, Record<string, PropType>>()

/** Property types by key, for coercing binding results. */
export function propTypesOf(w: Pick<WidgetInstance, 'type' | 'config'>): Record<string, PropType> {
  const key = w.type === 'symbol' ? `symbol:${w.config?.symbol}` : w.type
  let t = typeCache.get(key)
  if (!t) {
    t = Object.fromEntries(propertiesOf(w).map(f => [f.key, f.type as PropType]))
    typeCache.set(key, t)
  }
  return t
}
