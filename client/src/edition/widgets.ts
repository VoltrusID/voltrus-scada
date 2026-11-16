// Extra widget types for dashboards and SCADA screens, contributed by an
// add-on (the enterprise client registers a live camera widget). Core keeps
// its own widgets; this registry only holds the extra ones, so a Community
// build has none and its pickers look exactly as before.
//
// Saved screens keep the widget's type and config whether or not the type is
// registered: an unknown type renders a placeholder and is saved back as is.
//
// React-free (type imports only) so it can be tested under node.
import type { ComponentType, ReactNode } from 'react'
import type { SensorConfig, SensorData } from '../types'

/** Where a widget can be placed: the card dashboard grid or the SCADA (2D) editor. */
export type ScreenSurface = 'dashboard' | 'scada'

export interface ScreenWidgetRenderProps {
  /** The widget's saved settings (its `defaultConfig` for a new one). */
  config: Record<string, unknown>
  /** Size of the widget's box in CSS pixels (SCADA: stage pixels, before zoom). */
  width: number
  height: number
  /** The screen is being edited: the widget should not react to clicks. */
  editMode: boolean
  surface: ScreenSurface
  /** All data sources the screen can see. */
  sensors: SensorConfig[]
  /** Latest live values, keyed by data source id as a string. */
  values: Record<string, SensorData>
}

export interface ScreenWidgetConfigEditorProps {
  config: Record<string, unknown>
  /** Replace the widget's config with this one. */
  onChange: (config: Record<string, unknown>) => void
  surface: ScreenSurface
  sensors: SensorConfig[]
}

export interface ScreenWidgetDef {
  /**
   * Stored with every placed widget. Must be namespaced (`vendor.name`, e.g.
   * `enterprise.camera`) so it can never clash with a core widget type.
   */
  type: string
  label: string
  icon?: ReactNode
  surfaces: readonly ScreenSurface[]
  /** Dashboard size in grid cells (24 columns at the widest breakpoint, 50 px rows). Default 6 x 5. */
  defaultSize?: { w: number; h: number }
  /** SCADA size in stage pixels. Default 320 x 240. */
  defaultPixelSize?: { width: number; height: number }
  defaultConfig?: Record<string, unknown>
  render: ComponentType<ScreenWidgetRenderProps>
  /** Settings form shown while editing the screen. Without one the widget has no settings. */
  configEditor?: ComponentType<ScreenWidgetConfigEditorProps>
}

export const UNAVAILABLE_WIDGET_MESSAGE = 'Widget not available in this edition'

export const DEFAULT_DASHBOARD_SIZE = { w: 6, h: 5 }
export const DEFAULT_SCADA_SIZE = { width: 320, height: 240 }

const TYPE_PATTERN = /^[a-z0-9][a-z0-9_-]*(\.[a-z0-9][a-z0-9_-]*)+$/i
const SURFACES: readonly ScreenSurface[] = ['dashboard', 'scada']

// Insertion order is the order the pickers list them in.
const registry = new Map<string, ScreenWidgetDef>()

/** Why `def` cannot be registered, or null when it is valid. */
export function screenWidgetError(def: ScreenWidgetDef): string | null {
  if (!def || typeof def !== 'object') return 'not a widget definition'
  if (typeof def.type !== 'string' || !TYPE_PATTERN.test(def.type)) {
    return `widget type "${String(def.type)}" must be namespaced, like "vendor.name"`
  }
  if (!def.label) return `widget "${def.type}" has no label`
  if (!Array.isArray(def.surfaces) || def.surfaces.length === 0 || def.surfaces.some(s => !SURFACES.includes(s))) {
    return `widget "${def.type}" must list its surfaces ("dashboard", "scada")`
  }
  if (typeof def.render !== 'function' && (typeof def.render !== 'object' || def.render === null)) {
    return `widget "${def.type}" has no render component`
  }
  return null
}

/**
 * Register a widget type. Throws on an invalid definition. Registering a type
 * that is already registered replaces the earlier definition (so a module
 * reloaded during development does not fail); the replaced one is returned.
 */
export function registerScreenWidget(def: ScreenWidgetDef): ScreenWidgetDef | undefined {
  const err = screenWidgetError(def)
  if (err) throw new Error(`registerScreenWidget: ${err}`)
  const previous = registry.get(def.type)
  // Replacing keeps the original slot in the picker order.
  registry.set(def.type, def)
  return previous
}

/** Register several, reporting (not throwing) the invalid ones so one bad entry cannot break startup. */
export function registerScreenWidgets(defs: readonly ScreenWidgetDef[], onError: (message: string) => void = console.warn): void {
  for (const def of defs) {
    try {
      registerScreenWidget(def)
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e))
    }
  }
}

export function unregisterScreenWidget(type: string): boolean {
  return registry.delete(type)
}

/** Registered widgets, optionally only those offered on `surface`, in registration order. */
export function screenWidgets(surface?: ScreenSurface): ScreenWidgetDef[] {
  const all = [...registry.values()]
  return surface ? all.filter(d => d.surfaces.includes(surface)) : all
}

/** The definition for `type`; with `surface`, only when it may be placed there. */
export function getScreenWidget(type: string, surface?: ScreenSurface): ScreenWidgetDef | undefined {
  const def = registry.get(type)
  return def && (!surface || def.surfaces.includes(surface)) ? def : undefined
}

export type ResolvedScreenWidget =
  | { available: true; def: ScreenWidgetDef }
  | { available: false; type: string; message: string }

/** What to draw for a saved widget: its definition, or the placeholder for a type this build does not have. */
export function resolveScreenWidget(type: string, surface: ScreenSurface): ResolvedScreenWidget {
  const def = getScreenWidget(type, surface)
  return def ? { available: true, def } : { available: false, type, message: UNAVAILABLE_WIDGET_MESSAGE }
}

/** Whether `type` looks like an add-on widget type (namespaced), registered or not. */
export function isAddonWidgetType(type: string): boolean {
  return TYPE_PATTERN.test(type)
}

export function dashboardSizeOf(def: ScreenWidgetDef | undefined): { w: number; h: number } {
  return def?.defaultSize ?? DEFAULT_DASHBOARD_SIZE
}

export function scadaSizeOf(def: ScreenWidgetDef | undefined): { width: number; height: number } {
  return def?.defaultPixelSize ?? DEFAULT_SCADA_SIZE
}

/** A fresh copy of the widget's default config, so instances never share one object. */
export function initialConfigOf(def: ScreenWidgetDef | undefined): Record<string, unknown> {
  return def?.defaultConfig ? structuredClone(def.defaultConfig) : {}
}
