import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

const LANGS = ['en', 'id', 'zh', 'de']
const load = (lang: string) => JSON.parse(readFileSync(new URL(`../public/locales/${lang}/translation.json`, import.meta.url), 'utf8'))

function flatten(o: Record<string, unknown>, prefix = ''): Map<string, string> {
  const out = new Map<string, string>()
  for (const [k, v] of Object.entries(o)) {
    if (v && typeof v === 'object') for (const [kk, vv] of flatten(v as Record<string, unknown>, `${prefix}${k}.`)) out.set(kk, vv)
    else out.set(`${prefix}${k}`, String(v))
  }
  return out
}

/** Plural forms (`_one`, `_other`) count as one key: languages without a singular only have `_other`. */
const base = (k: string) => k.replace(/_(zero|one|two|few|many|other)$/, '')

const flat = Object.fromEntries(LANGS.map(l => [l, flatten(load(l))]))

test('every language offered in the selector has every key, translated', () => {
  const en = new Set([...flat.en.keys()].map(base))
  for (const lang of LANGS.slice(1)) {
    const keys = new Set([...flat[lang].keys()].map(base))
    assert.deepEqual([...en].filter(k => !keys.has(k)), [], `${lang} is missing keys`)
    assert.deepEqual([...keys].filter(k => !en.has(k)), [], `${lang} has keys English lacks`)
    for (const [k, v] of flat[lang]) assert.ok(v.trim() !== '', `${lang} ${k} is empty`)
  }
})

test('interpolation variables match English', () => {
  const vars = (s: string) => [...s.matchAll(/\{\{(\w+)\}\}/g)].map(m => m[1]).sort().join(',')
  const enVars = new Map([...flat.en].map(([k, v]) => [base(k), vars(v)]))
  for (const lang of LANGS.slice(1)) {
    for (const [k, v] of flat[lang]) assert.equal(vars(v), enVars.get(base(k)), `${lang} ${k}`)
  }
})

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap(name => {
    const p = join(dir, name)
    return statSync(p).isDirectory() ? sources(p) : /\.tsx?$/.test(name) ? [p] : []
  })
}

test('every literal key the code translates exists in English', () => {
  const en = new Set([...flat.en.keys()].map(base))
  const missing: string[] = []
  for (const file of sources(new URL('../src', import.meta.url).pathname)) {
    const src = readFileSync(file, 'utf8')
    for (const m of src.matchAll(/(?:\bt|i18n\.t)\(\s*'([a-zA-Z][\w]*\.[\w.]+)'/g)) if (!en.has(m[1])) missing.push(`${m[1]} (${file.split('/src/')[1]})`)
    for (const m of src.matchAll(/i18nKey="([\w.]+)"/g)) if (!en.has(m[1])) missing.push(`${m[1]} (${file.split('/src/')[1]})`)
  }
  assert.deepEqual(missing, [])
})
