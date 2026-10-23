import { createContext, useContext } from 'react'
import { COMMUNITY_LICENSE, type LicenseInfo } from './edition/license'

export type { LicenseInfo } from './edition/license'

/** Defaults to Community: the app is fully usable before (or without) a licence answer. */
export const LicenseContext = createContext<LicenseInfo>(COMMUNITY_LICENSE)

export function useLicense(): LicenseInfo {
  return useContext(LicenseContext)
}
