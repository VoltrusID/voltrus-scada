// Add-on widgets on the card dashboard, React-free so it can be tested.
//
// The dashboard layout (PUT /screens/{id}/layouts) is stored per breakpoint
// as the grid items themselves, and the server keeps each item verbatim. A
// widget is therefore saved as an extra grid item next to the data-source
// cards:
//
//   { i: 'widget:<id>', x, y, w, h, widget: { type: 'enterprise.camera', config: {...} } }
//
// The payload is repeated in every breakpoint so any one of them is enough to
// rebuild the widget list; positions stay per breakpoint like the cards'.
import type { LayoutItem, ScreenLayouts } from './types'

export const WIDGET_ITEM_PREFIX = 'widget:'

export interface DashboardWidgetPayload {
  type: string
  config: Record<string, unknown>
}

export interface DashboardWidget extends DashboardWidgetPayload {
  id: string
}

const BREAKPOINTS = ['lg', 'md', 'sm', 'xs'] as const
const COLS: Record<(typeof BREAKPOINTS)[number], number> = { lg: 24, md: 12, sm: 4, xs: 2 }

export function isWidgetItem(i: string): boolean {
  return i.startsWith(WIDGET_ITEM_PREFIX)
}

export function widgetItemKey(id: string): string {
  return WIDGET_ITEM_PREFIX + id
}

export function newDashboardWidgetId(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`
}

function readPayload(v: unknown): DashboardWidgetPayload | null {
  if (!v || typeof v !== 'object') return null
  const { type, config } = v as Record<string, unknown>
  if (typeof type !== 'string' || !type) return null
  return { type, config: config && typeof config === 'object' && !Array.isArray(config) ? config as Record<string, unknown> : {} }
}

/** The widgets saved in a screen's layouts, in the order of the widest breakpoint that has them. */
export function extractDashboardWidgets(layouts: Partial<ScreenLayouts> | null | undefined): DashboardWidget[] {
  const found = new Map<string, DashboardWidget>()
  for (const bp of BREAKPOINTS) {
    for (const item of layouts?.[bp] ?? []) {
      if (!isWidgetItem(String(item.i))) continue
      const id = String(item.i).slice(WIDGET_ITEM_PREFIX.length)
      if (found.has(id)) continue
      const payload = readPayload(item.widget)
      if (payload) found.set(id, { id, ...payload })
    }
  }
  return [...found.values()]
}

/**
 * The layouts with exactly `widgets` placed: kept widgets keep their position
 * (and get the current payload), new ones go below everything else, removed
 * ones are dropped. Data-source cards are left alone.
 */
export function withDashboardWidgets(
  layouts: Partial<ScreenLayouts>,
  widgets: readonly DashboardWidget[],
  sizeOf: (w: DashboardWidget) => { w: number; h: number },
): ScreenLayouts {
  const byKey = new Map(widgets.map(w => [widgetItemKey(w.id), w]))
  const out = {} as ScreenLayouts
  for (const bp of BREAKPOINTS) {
    const items = layouts[bp]
    if (!items) {
      if (bp !== 'xs') out[bp] = []
      continue
    }
    const placed = new Set<string>()
    const next: LayoutItem[] = []
    for (const item of items) {
      const key = String(item.i)
      if (!isWidgetItem(key)) { next.push(item); continue }
      const w = byKey.get(key)
      if (!w || placed.has(key)) continue
      placed.add(key)
      next.push({ i: key, x: item.x, y: item.y, w: item.w, h: item.h, widget: { type: w.type, config: w.config } })
    }
    let bottom = next.reduce((m, it) => Math.max(m, it.y + it.h), 0)
    for (const w of widgets) {
      const key = widgetItemKey(w.id)
      if (placed.has(key)) continue
      const size = sizeOf(w)
      const width = Math.max(1, Math.min(size.w, COLS[bp]))
      const height = Math.max(1, size.h)
      next.push({ i: key, x: 0, y: bottom, w: width, h: height, widget: { type: w.type, config: w.config } })
      bottom += height
    }
    out[bp] = next
  }
  return out
}
