// What the signed-in user's built-in role allows, mirroring the server's checks.
// The server stays the authority; these only decide which controls to show.
//
//   viewer, user (legacy)      read only
//   operator                   operate: write tags within their write policy, cancel
//                              commands, acknowledge/shelve alarms, load recipes,
//                              start/stop flows, shift handovers, OEE downtime
//   demo, integration, admin   operate and configure
//   admin                      also the admin-only pages (users, roles, API keys, licence)

const OPERATE_ROLES = new Set(['operator', 'demo', 'integration', 'admin'])
const CONFIGURE_ROLES = new Set(['demo', 'integration', 'admin'])

/** May run the plant: tag writes, alarm handling, recipe loads, flow start/stop. */
export function canOperate(role: string | null | undefined): boolean {
  return !!role && OPERATE_ROLES.has(role)
}

/** May change configuration: devices, screens, data sources, write policies, flows, recipes. */
export function canConfigure(role: string | null | undefined): boolean {
  return !!role && CONFIGURE_ROLES.has(role)
}

/** May open the admin-only pages. */
export function isAdmin(role: string | null | undefined): boolean {
  return role === 'admin'
}

export interface RoleAbilities {
  canOperate: boolean
  canConfigure: boolean
  isAdmin: boolean
}

export function roleAbilities(role: string | null | undefined): RoleAbilities {
  return { canOperate: canOperate(role), canConfigure: canConfigure(role), isAdmin: isAdmin(role) }
}

export const READ_ONLY: RoleAbilities = { canOperate: false, canConfigure: false, isAdmin: false }
