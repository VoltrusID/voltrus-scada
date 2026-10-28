// Display helpers for the storage panel.

/** "90 days", "1 hour", "10 minutes", or "forever" for null. */
export function formatDuration(secs: number | null): string {
  if (secs === null) return 'forever'
  const units: [number, string][] = [[86400, 'day'], [3600, 'hour'], [60, 'minute'], [1, 'second']]
  for (const [size, name] of units) {
    if (secs >= size && secs % size === 0) {
      const n = secs / size
      return `${n} ${name}${n === 1 ? '' : 's'}`
    }
  }
  return secs === 0 ? '0 seconds' : `${secs} seconds`
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let v = bytes / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++ }
  return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${units[i]}`
}
