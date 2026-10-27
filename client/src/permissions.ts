// Permission names the server accepts in a role (`models::ALL_PERMISSIONS`,
// src/models/rbac_types.rs). It refuses any other name with 400; the test in
// tests/permissions.test.ts compares this list with the Rust source.
export const ALL_PERMISSIONS = [
  'screens:read', 'screens:write',
  'devices:read', 'devices:write',
  'alarms:read', 'alarms:ack',
  'settings:read', 'settings:write',
  'users:manage',
  'flows:manage',
  'notifications:manage',
  'hub:manage',
  'reports:manage',
  'recipes:manage',
  'commands:write', 'commands:read', 'commands:cancel',
  'tags:write_policy',
  'sim:read', 'sim:write',
  'scripts:read', 'scripts:write',
  'config:read', 'config:write',
] as const
