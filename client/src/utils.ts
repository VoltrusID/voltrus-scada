import type { DataPoint, SensorConfig } from './types'

/**
 * Expand compact history data from the server into a map of DataPoint arrays.
 * The server returns: { sensor_key: { values: [f64], start: unix_ms, interval: ms } },
 * plus `min` and `max` arrays parallel to `values` for `agg=minmax`.
 */
export function expandCompactData(raw: Record<string, any>): Record<string, DataPoint[]> {
  const out: Record<string, DataPoint[]> = {};
  for (const key of Object.keys(raw)) {
    const s = raw[key];
    if (s && Array.isArray(s.values)) {
      const lows: unknown[] | null = Array.isArray(s.min) && s.min.length === s.values.length ? s.min : null;
      const highs: unknown[] | null = Array.isArray(s.max) && s.max.length === s.values.length ? s.max : null;
      const pts: DataPoint[] = [];
      for (let i = 0; i < s.values.length; i++) {
        const pt: DataPoint = { ts: s.start + i * s.interval, value: s.values[i] };
        if (lows && highs) { pt.lo = lows[i] as number; pt.hi = highs[i] as number; }
        pts.push(pt);
      }
      out[key] = pts;
    } else if (Array.isArray(s)) {
      out[key] = s; // legacy format fallback
    }
  }
  return out;
}

// ── Alarm status from tag limits ──

const lowLimits = new WeakMap<SensorConfig, { lw?: number; lc?: number }>()

/**
 * Alarm level of a value against its tag's limits, mirroring the server's alarm engine:
 * high limits fire at value >= limit (0 = not set); low limits live in source_config.low_warn / low_crit.
 */
export function alarmStatus(sensor: SensorConfig, value: number): 'crit' | 'warn' | 'normal' {
  let lows = lowLimits.get(sensor)
  if (!lows) {
    lows = {}
    try {
      const cfg = JSON.parse(sensor.source_config || '{}')
      if (typeof cfg.low_warn === 'number') lows.lw = cfg.low_warn
      if (typeof cfg.low_crit === 'number') lows.lc = cfg.low_crit
    } catch { /* not JSON: no low limits */ }
    lowLimits.set(sensor, lows)
  }
  if (sensor.crit_val > 0 && value >= sensor.crit_val) return 'crit'
  if (sensor.warn_val > 0 && value >= sensor.warn_val) return 'warn'
  if (lows.lc != null && value <= lows.lc) return 'crit'
  if (lows.lw != null && value <= lows.lw) return 'warn'
  return 'normal'
}
