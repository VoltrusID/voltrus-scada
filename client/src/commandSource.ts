// Who sent a command or performed an alarm action, as the server reports it
// (`source` on a command) or, for older servers and for alarm audit rows, as
// derived from the actor string: `<user>`, `api_key:<id>`, `flow:<id>`.

export type SourceKind = 'user' | 'api_key' | 'mcp' | 'flow' | 'recipe' | 'system'

export interface CommandSource {
  kind: SourceKind
  label: string
}

const KINDS: SourceKind[] = ['user', 'api_key', 'mcp', 'flow', 'recipe', 'system']

/** The sender of an actor string, with the server's `source` when it sent one. */
export function sourceOf(actor: string | null | undefined, server?: { kind?: string; label?: string } | null): CommandSource {
  if (server?.kind && server.label && (KINDS as string[]).includes(server.kind)) {
    return { kind: server.kind as SourceKind, label: server.label }
  }
  const a = (actor ?? '').trim()
  if (!a || a === 'system') return { kind: 'system', label: 'system' }
  if (a.startsWith('api_key:')) return { kind: 'api_key', label: `API key #${a.slice(8)}` }
  if (a.startsWith('flow:')) return { kind: 'flow', label: `flow ${a.slice(5)}` }
  return { kind: 'user', label: a }
}

/** Short badge text for a sender kind. */
export const SOURCE_BADGE: Record<SourceKind, string> = {
  user: 'USER',
  api_key: 'API KEY',
  mcp: 'MCP',
  flow: 'FLOW',
  recipe: 'RECIPE',
  system: 'SYSTEM',
}
