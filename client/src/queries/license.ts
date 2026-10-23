import { useQuery } from '@tanstack/react-query'
import { COMMUNITY_LICENSE, normalizeLicense, type LicenseInfo } from '../edition/license'
import { fetchJson } from './fetch'

export const licenseQueryKey = ['license'] as const

/** Never throws: an unreachable or malformed answer means Community, not a blocked app. */
export async function fetchLicenseStatus(): Promise<LicenseInfo> {
  try {
    return normalizeLicense(await fetchJson<unknown>('/api/license/status'))
  } catch {
    return COMMUNITY_LICENSE
  }
}

export function useLicenseStatus(): LicenseInfo {
  const { data } = useQuery({
    queryKey: licenseQueryKey,
    queryFn: fetchLicenseStatus,
    staleTime: 60_000,
  })
  return data ?? COMMUNITY_LICENSE
}
