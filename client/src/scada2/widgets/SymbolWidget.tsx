import { registerWidget, WIDGET_MANIFESTS } from './registry'
import type { ConfigFieldDef, ScadaWidgetType, WidgetProps } from './types'
import { SymbolView } from '../hmi/SymbolView'
import { resolveSymbol } from '../hmi/resolve'
import type { SymbolDef } from '../hmi/symbols'

const SymbolWidget = (props: WidgetProps) => {
  const r = resolveSymbol(props.instance)
  if (!r) return null
  return <SymbolView {...props} def={r.def} instance={{ ...props.instance, config: r.config }} />
}

/**
 * Inspector fields for a symbol: common appearance + binding-specific + the symbol's own.
 * `legacy`: a pre-library pump/motor/valve widget, which has no value badge and is writable unless turned off.
 */
export function symbolSchema(def: SymbolDef, opts: { legacy?: boolean; defaultWritable?: boolean } = {}): ConfigFieldDef[] {
  const fields: ConfigFieldDef[] = [{ key: 'label', label: 'Label', type: 'string', default: '' }]
  if (!(opts.legacy && def.binding === 'state')) {
    fields.push({ key: 'showValue', label: 'Show Value', type: 'boolean', default: def.binding === 'level' || def.binding === 'value' })
  }
  if (def.binding === 'state') {
    fields.push(
      { key: 'showStatus', label: 'Show Status Text', type: 'boolean', default: false },
      { key: 'onThreshold', label: 'Running Above', type: 'number', default: 0.5 },
      { key: 'faultSensorId', label: 'Fault Tag', type: 'tag', default: null },
      { key: 'writable', label: 'Operator Start/Stop', type: 'boolean', default: !!opts.defaultWritable },
    )
  }
  fields.push(...(def.fields ?? []))
  if (!def.stretch) {
    fields.push(
      { key: 'rotation', label: 'Rotation', type: 'select', default: '0', options: [
        { value: '0', label: '0°' }, { value: '90', label: '90°' }, { value: '180', label: '180°' }, { value: '270', label: '270°' },
      ] },
      { key: 'flipX', label: 'Mirror', type: 'boolean', default: false },
    )
  }
  return fields
}

// The legacy equipment widgets render through the same symbol library, so old screens get the new look.
const SYMBOL_TYPES: ScadaWidgetType[] = ['symbol', 'pump', 'motor', 'valve', 'tank_level', 'horizontal_tank', 'pipe']
for (const t of SYMBOL_TYPES) registerWidget({ manifest: WIDGET_MANIFESTS[t], component: SymbolWidget })

export default SymbolWidget
