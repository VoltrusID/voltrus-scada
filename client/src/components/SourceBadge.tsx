import { sourceOf, SOURCE_BADGE } from '../commandSource'

/** "Who sent this": a small kind badge (USER, API KEY, MCP, FLOW, RECIPE) and the name. */
export function SourceBadge({ actor, source }: { actor: string | null | undefined; source?: { kind?: string; label?: string } | null }) {
  const s = sourceOf(actor, source)
  return (
    <span title={actor ? `Sent by ${actor}` : undefined} style={{ display: 'inline-flex', alignItems: 'center', gap: 4, fontSize: 10, color: 'var(--text-muted)' }}>
      <span style={{
        padding: '0 4px', borderRadius: 3, border: '1px solid var(--border)', fontWeight: 700,
        letterSpacing: '.05em', fontSize: 9, color: s.kind === 'user' ? 'var(--text-muted)' : 'var(--cyan)',
      }}>{SOURCE_BADGE[s.kind]}</span>
      <span style={{ color: 'var(--fg)' }}>{s.label}</span>
    </span>
  )
}
