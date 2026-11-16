import { createContext, useContext } from 'react'
import { READ_ONLY, type RoleAbilities } from './roles'

/** The signed-in user's abilities, provided once by the app shell so deep components need no prop drilling. */
export const RoleContext = createContext<RoleAbilities>(READ_ONLY)

export function useRole(): RoleAbilities {
  return useContext(RoleContext)
}
