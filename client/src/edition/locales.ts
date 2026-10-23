import type { LocaleBundles } from './types'

export const ENTERPRISE_NS = 'enterprise'

interface BundleSink {
  addResourceBundle: (lng: string, ns: string, resources: object, deep?: boolean, overwrite?: boolean) => unknown
}

/**
 * Register the enterprise module's translations under their own namespace so
 * they never collide with, or get skipped by, the HTTP-loaded core bundle.
 */
export function registerLocales(sink: BundleSink, locales: LocaleBundles | undefined): void {
  if (!locales) return
  for (const [lng, resources] of Object.entries(locales)) {
    if (resources) sink.addResourceBundle(lng, ENTERPRISE_NS, resources, true, true)
  }
}
