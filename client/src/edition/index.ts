// Single entry point of the edition seam. `@voltrus/enterprise-client` is a
// build-time alias (vite.config.ts and tsconfig.json): it points at the private
// enterprise module when VOLTRUS_ENTERPRISE_CLIENT is set and at ./community.ts
// otherwise.
import enterprise from '@voltrus/enterprise-client'
import type { BrandingProvider, EnterpriseClientModule } from './types'

export * from './types'
export { COMMUNITY_LICENSE, FEATURE_FLAG_KEYS, hasFeature, normalizeLicense } from './license'
export { DEFAULT_BRANDING } from './branding'
export { mergeTabs } from './tabs'
export { ENTERPRISE_NS, registerLocales } from './locales'

/** Empty object for Community builds, so call sites never null-check the module. */
export const edition: EnterpriseClientModule = enterprise ?? {}

/** Branding for the app shell: the enterprise provider's when present, else null (Voltrus default). */
export const useEditionBranding: BrandingProvider['useBranding'] = edition.branding?.useBranding ?? (() => null)
