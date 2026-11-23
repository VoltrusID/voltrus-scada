import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

// Theme colours used as text must meet WCAG AA (4.5:1) on the surfaces they sit on.
const css = readFileSync(new URL('../src/index.css', import.meta.url), 'utf8')

function theme(selector: string): Record<string, string> {
  const start = css.indexOf(selector)
  const block = css.slice(css.indexOf('{', start) + 1, css.indexOf('\n}', start))
  const vars: Record<string, string> = {}
  for (const m of block.matchAll(/(--[\w-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) vars[m[1]] = m[2]
  return vars
}

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map(c => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return (hi + 0.05) / (lo + 0.05)
}

const dark = theme('[data-theme="dark"]')
const light = theme('[data-theme="light"]')

test('dark theme text colours reach 4.5:1 on background and cards', () => {
  for (const fg of ['--text', '--text-dim', '--text-muted', '--warn', '--crit', '--ok', '--cyan']) {
    for (const bg of ['--bg', '--card']) {
      const r = contrast(dark[fg], dark[bg])
      assert.ok(r >= 4.5, `dark ${fg} ${dark[fg]} on ${bg} ${dark[bg]}: ${r.toFixed(2)}`)
    }
  }
})

test('light theme text colours reach 4.5:1 on background and cards', () => {
  for (const fg of ['--text', '--text-dim', '--text-muted', '--warn', '--crit', '--ok', '--cyan', '--green']) {
    for (const bg of ['--bg', '--card']) {
      const r = contrast(light[fg], light[bg])
      assert.ok(r >= 4.5, `light ${fg} ${light[fg]} on ${bg} ${light[bg]}: ${r.toFixed(2)}`)
    }
  }
})

test('HMI state colours label symbols legibly on the stage', () => {
  for (const [name, t] of [['dark', dark], ['light', light]] as const) {
    for (const fg of ['--hmi-run', '--hmi-warn', '--hmi-fault', '--hmi-stale']) {
      const r = contrast(t[fg], t['--hmi-stage'])
      assert.ok(r >= 4.5, `${name} ${fg} ${t[fg]} on --hmi-stage ${t['--hmi-stage']}: ${r.toFixed(2)}`)
    }
  }
})
