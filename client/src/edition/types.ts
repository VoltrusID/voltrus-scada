import type { ComponentType, ReactNode } from 'react'
import type { BrandingSettings } from './branding'
import type { LicenseInfo } from './license'

export type { BrandingSettings } from './branding'
export type { FeatureFlag, FeatureFlags, LicenseInfo, LicenseStatusName, LicenseTier } from './license'

/** Props the Data Manager hands to every enterprise tab. */
export interface DataManagerTabContext {
  onClose: () => void
  license: LicenseInfo
}

/**
 * A sidebar tab in the Data Manager contributed by the enterprise module.
 * The tab is always listed (the module itself renders an upgrade notice when
 * the licence does not cover the feature), so core stays unaware of flags.
 */
export interface DataManagerTab {
  /** Unique, URL-safe id used for the `?tab=` search param. Must not clash with a core tab id. */
  id: string
  label: string
  /** Sidebar group. Unknown group names create a new group after the core ones. */
  group: 'Data' | 'Protocols' | 'Insights' | 'Admin' | (string & {})
  icon: ReactNode
  admin?: boolean
  strictAdmin?: boolean
  render: (ctx: DataManagerTabContext) => ReactNode
}

export interface LoginProvidersProps {
  /** Called after a non-password login succeeded; core re-reads the session. */
  onLoggedIn: (session: { username?: string; role?: string; must_change_password?: boolean }) => void
}

export interface BrandingProvider {
  /**
   * Hook returning the branding to show, or null/undefined for the Voltrus
   * default. Called from the app root, so it receives the licence explicitly.
   * Must tolerate the endpoint answering 404.
   */
  useBranding: (license: LicenseInfo) => BrandingSettings | null | undefined
  /** Editor opened when an admin clicks the logo in edit mode. */
  Dialog: ComponentType<{ branding: BrandingSettings; onClose: () => void }>
}

/**
 * An extra notification channel type (for example SMS). Core handles
 * email, webhook and push itself.
 */
export interface NotificationChannel {
  /** Value stored in `channel_type`. */
  type: string
  label: string
  /** Background and text colour of the channel pill in the rules table. */
  pill: { background: string; color: string }
  /** Whether the current licence allows creating/editing rules of this type. */
  available: (license: LicenseInfo) => boolean
  /** Shown after the label in the type selector while `available` is false. */
  lockedHint: string
  /** Config stored for a brand new rule. */
  emptyConfig: string
  isValid: (config: string) => boolean
  /** Turns the editor's working config into the string persisted on save (for example drops blank rows). */
  serialize: (config: string) => string
  /** Short text for the targets column. */
  summarize: (config: string) => string
  Editor: ComponentType<{ config: string; onChange: (config: string) => void; locked: boolean }>
}

export type LocaleBundles = Partial<Record<'en' | 'id' | 'zh' | 'de', Record<string, unknown>>>

/**
 * Everything the private enterprise client contributes. All members are
 * optional so that a partial module (or the Community stub) is valid.
 */
export interface EnterpriseClientModule {
  /** Extra Data Manager tabs (hub, cameras, scheduled reports, security events, compliance...). */
  dataManagerTabs?: DataManagerTab[]
  /** Licence status and activation panel, shown as the "Licence" tab in the Admin group. */
  LicensePanel?: ComponentType
  /** Extra sign-in options (SSO) rendered under the password form. */
  LoginProviders?: ComponentType<LoginProvidersProps>
  /** White-label branding. */
  branding?: BrandingProvider
  /** Always-mounted banners rendered above the header (HA status...). */
  Banners?: ComponentType
  /** Extra notification channel types. */
  notificationChannels?: NotificationChannel[]
  /** Translation bundles, registered under the `enterprise` i18n namespace. */
  locales?: LocaleBundles
}
