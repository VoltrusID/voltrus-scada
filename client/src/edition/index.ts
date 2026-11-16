// Single entry point of the edition seam. `@voltrus/enterprise-client` is a
// build-time alias (vite.config.ts and tsconfig.json): it points at the private
// enterprise module when VOLTRUS_ENTERPRISE_CLIENT is set and at ./community.ts
// otherwise.
import enterprise from '@voltrus/enterprise-client'
import type { BrandingProvider, EnterpriseClientModule } from './types'
import { registerScreenWidgets } from './widgets'

export * from './types'
export { COMMUNITY_LICENSE, FEATURE_FLAG_KEYS, hasFeature, normalizeLicense } from './license'
export { DEFAULT_BRANDING, DEFAULT_ACCENT, applyBrandToDocument, brandAccent, brandAccentText } from './branding'
export { mergeTabs } from './tabs'
export { ENTERPRISE_NS, registerLocales } from './locales'
export {
  getScreenWidget, registerScreenWidget, resolveScreenWidget, screenWidgets, unregisterScreenWidget,
  UNAVAILABLE_WIDGET_MESSAGE,
} from './widgets'

/** Empty object for Community builds, so call sites never null-check the module. */
export const edition: EnterpriseClientModule = enterprise ?? {}

if (edition.screenWidgets) registerScreenWidgets(edition.screenWidgets)

/** Branding for the app shell: the enterprise provider's when present, else null (Voltrus default). */
export const useEditionBranding: BrandingProvider['useBranding'] = edition.branding?.useBranding ?? (() => null)
