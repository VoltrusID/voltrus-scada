// Per-tag history deadband: which samples of a tag reach the historian.
// The server reads three keys of the tag's source_config (src/deadband.rs):
//   history_deadband           >= 0, absent = store every sample
//   history_deadband_type      "absolute" (tag units, default) | "percent" of the last stored value
//   history_max_interval_secs  integer >= 1, store at least this often
// The alarm hysteresis key `deadband` is a different setting and is never touched here.

export type DeadbandType = 'absolute' | 'percent'

/** What the user types. Strings, so half-typed numbers survive re-renders. */
export interface HistoryDeadbandForm {
  type: DeadbandType
  value: string
  maxInterval: string
}

export const EMPTY_FORM: HistoryDeadbandForm = { type: 'absolute', value: '', maxInterval: '' }

export const HISTORY_KEYS = ['history_deadband', 'history_deadband_type', 'history_max_interval_secs'] as const

export const HISTORY_DEADBAND_HELP =
  'Store a sample only when it moves more than this from the last stored one. A band of 0 stores every change. ' +
  'Without a band every sample is stored, which is the largest on disk: a year of 10,000 tags polled each second is ' +
  'hundreds of GB, while a band above the signal noise plus a heartbeat brings it to tens of GB or less. ' +
  'The heartbeat stores a sample at least that often even when nothing changed, so a gap in a trend always means no data.'

export function readHistoryDeadband(cfg: Record<string, any>): HistoryDeadbandForm {
  const v = cfg.history_deadband
  if (typeof v !== 'number' || !Number.isFinite(v) || v < 0) return { ...EMPTY_FORM }
  const m = cfg.history_max_interval_secs
  return {
    type: cfg.history_deadband_type === 'percent' ? 'percent' : 'absolute',
    value: String(v),
    maxInterval: typeof m === 'number' && Number.isInteger(m) && m > 0 ? String(m) : '',
  }
}

/** Problems that would make the server ignore or reject the setting; empty when fine. */
export function validateHistoryDeadband(f: HistoryDeadbandForm): string[] {
  const errors: string[] = []
  const v = f.value.trim()
  const m = f.maxInterval.trim()
  if (v === '') {
    if (m !== '') errors.push('Set a deadband value too, the heartbeat only applies with a deadband (0 stores every change).')
  } else {
    const n = Number(v)
    if (!Number.isFinite(n) || n < 0) errors.push('Deadband must be a number of 0 or more.')
    else if (f.type === 'percent' && n > 100) errors.push('A percent deadband cannot exceed 100.')
  }
  if (m !== '') {
    const n = Number(m)
    if (!Number.isInteger(n) || n < 1) errors.push('Heartbeat must be a whole number of seconds, 1 or more.')
  }
  return errors
}

/**
 * Warnings that are valid but probably not intended. A band with no heartbeat leaves
 * long gaps on a flat signal; a band of 0 with a percent type filters nothing but repeats.
 */
export function historyDeadbandWarnings(f: HistoryDeadbandForm): string[] {
  if (validateHistoryDeadband(f).length > 0 || f.value.trim() === '') return []
  const warnings: string[] = []
  if (Number(f.value) > 0 && f.maxInterval.trim() === '') {
    warnings.push('No heartbeat: a flat signal will show no points until it moves.')
  }
  return warnings
}

/**
 * Copy of `cfg` with the history keys set from the form, or removed when the form has
 * no value (`null` also removes them). Every other key is kept as is.
 */
export function applyHistoryDeadband(cfg: Record<string, any>, f: HistoryDeadbandForm | null): Record<string, any> {
  const out = { ...cfg }
  for (const k of HISTORY_KEYS) delete out[k]
  if (!f || f.value.trim() === '') return out
  const errors = validateHistoryDeadband(f)
  if (errors.length > 0) throw new Error(errors[0])
  out.history_deadband = Number(f.value)
  if (f.type === 'percent') out.history_deadband_type = 'percent'
  if (f.maxInterval.trim() !== '') out.history_max_interval_secs = Number(f.maxInterval)
  return out
}

/** One-line description for lists, or null when the tag stores every sample. */
export function describeHistoryDeadband(cfg: Record<string, any>): string | null {
  const f = readHistoryDeadband(cfg)
  if (f.value === '') return null
  const band = `${f.value}${f.type === 'percent' ? ' %' : ''}`
  return f.maxInterval ? `${band}, heartbeat ${f.maxInterval} s` : band
}
