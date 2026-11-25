// Palette grouping and search over the symbol library. Plain TypeScript.

import { ISA_CATEGORIES } from './symbolFile.ts'

export type SymbolLibrary = 'isa' | 'shaded'

export interface PaletteEntry {
  key: string
  name: string
  category: string
  library?: SymbolLibrary
  keywords?: string[]
  /** False: kept for saved screens, not offered in the palette (still found by an exact key search). */
  palette?: boolean
}

export const LIBRARY_LABEL: Record<SymbolLibrary, string> = { isa: 'ISA-101', shaded: 'Shaded' }

/** A palette group id: `isa` (all ISA-101), `isa:Valves`, `shaded:Electrical`. */
export interface PaletteGroup { id: string; library: SymbolLibrary; category: string | null; label: string }

/** Groups of the palette's category picker: ISA-101 (all, then each category) first, then the shaded library's categories. */
export function paletteGroups(symbols: PaletteEntry[]): PaletteGroup[] {
  const out: PaletteGroup[] = []
  for (const library of ['isa', 'shaded'] as SymbolLibrary[]) {
    const cats: string[] = []
    for (const s of symbols) {
      if ((s.library ?? 'shaded') !== library || s.palette === false) continue
      if (!cats.includes(s.category)) cats.push(s.category)
    }
    if (!cats.length) continue
    if (library === 'isa') {
      const rank = (c: string) => { const i = (ISA_CATEGORIES as readonly string[]).indexOf(c); return i < 0 ? 99 : i }
      cats.sort((a, b) => rank(a) - rank(b))
    }
    if (library === 'isa') out.push({ id: 'isa', library, category: null, label: 'ISA-101: all' })
    for (const c of cats) out.push({ id: `${library}:${c}`, library, category: c, label: `${LIBRARY_LABEL[library]} · ${c}` })
  }
  return out
}

/**
 * Symbols of a group, or (with a query) every symbol whose name, key, keywords
 * or category contain all the query's words, ISA-101 first. Hidden symbols
 * show up only when the query is their exact key.
 */
export function filterSymbols<T extends PaletteEntry>(symbols: T[], query: string, group: string): T[] {
  const q = query.trim().toLowerCase()
  if (q) {
    const words = q.split(/\s+/)
    const hits = symbols.filter(s => {
      if (s.palette === false) return s.key === q
      const hay = `${s.name} ${s.key} ${s.category} ${(s.keywords ?? []).join(' ')} ${s.library === 'isa' ? 'isa-101 isa101' : ''}`.toLowerCase()
      return words.every(w => hay.includes(w))
    })
    return [...hits.filter(s => s.library === 'isa'), ...hits.filter(s => s.library !== 'isa')]
  }
  const [library, category] = group.includes(':') ? [group.slice(0, group.indexOf(':')), group.slice(group.indexOf(':') + 1)] : [group, null]
  return symbols.filter(s => s.palette !== false && (s.library ?? 'shaded') === library && (category == null || s.category === category))
}
