import type { SensorData } from './types'

/**
 * Whether a live update repeats the reading the dashboard already shows: same
 * sample time, value, quality and status. The server sends a tag again when
 * any device polls (older servers send every tag every time); a repeat is not
 * a new sample, so it must not add a trend point or redraw the card.
 * Updates without a sample time are never treated as repeats.
 */
export function isRepeatSample(prev: SensorData | undefined, next: SensorData): boolean {
  return next.ts_ms != null
    && prev !== undefined
    && prev.ts_ms === next.ts_ms
    && Object.is(prev.value, next.value)
    && prev.quality === next.quality
    && prev.status === next.status
}
