// Tag-swap faceplates: one definition (GET /api/v1/faceplates) per equipment
// kind, opened from any symbol whose config carries
// { faceplate: '<id>', faceplateMap: { <role>: <tag id> } }.

export type FaceplateKind = 'pump' | 'motor' | 'valve' | 'analog'
export type RoleKind = 'state' | 'fault' | 'analog' | 'command' | 'setpoint'

export interface FaceplateRole {
  role: string
  label?: string
  kind: RoleKind
  required?: boolean
}

export interface FaceplateDef {
  id: string
  name: string
  kind: FaceplateKind
  description?: string
  roles: FaceplateRole[]
}

export type FaceplateMap = Record<string, number>

/** Starting definitions the editor offers to save when none fits. */
export const FACEPLATE_PRESETS: FaceplateDef[] = [
  {
    id: 'pump', name: 'Pump', kind: 'pump',
    roles: [
      { role: 'run', label: 'Running', kind: 'state', required: true },
      { role: 'fault', label: 'Fault', kind: 'fault' },
      { role: 'cmd', label: 'Start / stop', kind: 'command' },
      { role: 'speed', label: 'Speed', kind: 'analog' },
    ],
  },
  {
    id: 'valve', name: 'Valve', kind: 'valve',
    roles: [
      { role: 'open', label: 'Open', kind: 'state', required: true },
      { role: 'fault', label: 'Fault', kind: 'fault' },
      { role: 'cmd', label: 'Open / close', kind: 'command' },
      { role: 'position', label: 'Position', kind: 'analog' },
    ],
  },
  {
    id: 'analog', name: 'Analog measurement', kind: 'analog',
    roles: [
      { role: 'pv', label: 'Process value', kind: 'analog', required: true },
      { role: 'sp', label: 'Setpoint', kind: 'setpoint' },
    ],
  },
]

/** The faceplate reference of a symbol's config, if it opens one. */
export function faceplateOf(config: Record<string, unknown> | undefined): { id: string; map: FaceplateMap } | null {
  const id = config?.faceplate
  if (typeof id !== 'string' || !id) return null
  const raw = config?.faceplateMap
  const map: FaceplateMap = {}
  if (raw && typeof raw === 'object') {
    for (const [role, tag] of Object.entries(raw as Record<string, unknown>)) {
      if (typeof tag === 'number' && Number.isFinite(tag)) map[role] = tag
    }
  }
  return { id, map }
}

/** Required roles the mapping leaves out. */
export function missingRoles(def: FaceplateDef, map: FaceplateMap): string[] {
  return def.roles.filter(r => r.required && map[r.role] == null).map(r => r.role)
}

/** The config change that maps (or unmaps, with null) one role of a symbol. */
export function withRole(config: Record<string, unknown>, role: string, tag: number | null): Record<string, unknown> {
  const map = { ...(faceplateOf(config)?.map ?? {}) }
  if (tag == null) delete map[role]
  else map[role] = tag
  return { faceplateMap: map }
}

/** Command words of a faceplate kind: [on, off]. */
export function commandWords(kind: FaceplateKind): [string, string] {
  return kind === 'valve' ? ['OPEN', 'CLOSE'] : ['START', 'STOP']
}

/** On/off text of a state or fault role. */
export function stateWords(kind: FaceplateKind, role: FaceplateRole): [string, string] {
  if (role.kind === 'fault') return ['TRIPPED', 'HEALTHY']
  return kind === 'valve' ? ['OPEN', 'CLOSED'] : ['RUNNING', 'STOPPED']
}

/** The tag id a symbol binds first (what it draws), from its faceplate mapping. */
export function primaryTag(def: FaceplateDef, map: FaceplateMap): number | null {
  for (const kind of ['state', 'analog'] as RoleKind[]) {
    const r = def.roles.find(x => x.kind === kind && map[x.role] != null)
    if (r) return map[r.role]
  }
  return null
}
