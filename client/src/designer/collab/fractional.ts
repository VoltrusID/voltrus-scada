// Fractional indexes: strings that sort between any two others, so a widget
// can be put between two neighbours without renumbering anything. The same
// algorithm as collab/src/fractional.rs (the server and the native console):
// base-62 digits (0-9A-Za-z, ASCII order), never ending in '0'.

const DIGITS = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'

const digit = (c: string | undefined): number => (c == null ? -1 : DIGITS.indexOf(c))

export const isValid = (key: string | null | undefined): key is string =>
  !!key && !key.endsWith('0') && [...key].every(c => digit(c) >= 0)

function midpoint(a: string, b: string | null): string {
  if (b != null) {
    let n = 0
    while (n < b.length && (a[n] ?? '0') === b[n]) n++
    if (n > 0) return b.slice(0, n) + midpoint(a.slice(n), b.slice(n))
  }
  const da = a.length ? Math.max(0, digit(a[0])) : 0
  const db = b != null && b.length ? (digit(b[0]) < 0 ? 62 : digit(b[0])) : 62
  if (db > da + 1) return DIGITS[Math.ceil((da + db) / 2)]
  if (b != null && b.length > 1) return b[0]
  return DIGITS[da] + midpoint(a.slice(1), null)
}

/** A key between `a` and `b` (a < b), either end open. Bad bounds count as open. */
export function keyBetween(a: string | null | undefined, b: string | null | undefined): string {
  let lo = isValid(a) ? a : null
  let hi = isValid(b) ? b : null
  if (lo != null && hi != null && lo >= hi) hi = null
  return midpoint(lo ?? '', hi)
}

/** `n` keys between `a` and `b`, in order, kept short by bisecting. */
export function nKeysBetween(a: string | null | undefined, b: string | null | undefined, n: number): string[] {
  if (n <= 0) return []
  const mid = keyBetween(a, b)
  const left = Math.floor(n / 2)
  return [...nKeysBetween(a, mid, left), mid, ...nKeysBetween(mid, b, n - left - 1)]
}
