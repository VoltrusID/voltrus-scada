// The analytics compare route takes two periods in Unix seconds and labels the first
// "current" and the second "previous". Both ends of a period are inclusive.

export type Period = readonly [from: number, to: number]

/** The range split into its recent half (period 1, "current") and the older half before it
 *  (period 2, "previous"). The older half stops one second before the recent one starts,
 *  so a sample on the boundary counts once. */
export function comparePeriods(from: number, to: number): { current: Period; previous: Period } {
  const mid = Math.floor((from + to) / 2)
  return { current: [mid, to], previous: [from, mid - 1] }
}
