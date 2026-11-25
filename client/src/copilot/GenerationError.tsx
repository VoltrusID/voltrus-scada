import { generationErrorMessage } from './proposalChecks'

/** Why the last generation failed, or nothing. */
export function GenerationError({ error }: { error: unknown }) {
  if (!error) return null
  return (
    <div role="alert" style={{ padding: 8, borderRadius: 6, background: 'rgba(239,68,68,.1)', color: '#ef4444', fontSize: 13 }}>
      {generationErrorMessage(error)}
    </div>
  )
}
