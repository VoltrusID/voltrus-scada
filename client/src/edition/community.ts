import type { EnterpriseClientModule } from './types'

// Default target of the `@voltrus/enterprise-client` alias: the Community
// edition has no enterprise client module.
const community: EnterpriseClientModule | null = null

export default community
