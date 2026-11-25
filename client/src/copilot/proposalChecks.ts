// Checks for the copilot wizards and proposal review, kept free of React so they can be tested.

export const PDF_GUIDANCE =
  'PDF datasheets cannot be read yet. Open the PDF, copy the register table (or export it to CSV) and upload it as a .csv or .txt file.'

/** Why a datasheet file cannot be used, or null when it can. */
export function datasheetFileError(file: { name: string; type?: string }): string | null {
  if (file.type === 'application/pdf' || /\.pdf$/i.test(file.name.trim())) return PDF_GUIDANCE
  return null
}

/** Whether text read from a file is really a PDF (its signature survives readAsText). */
export function looksLikePdf(text: string): boolean {
  return text.replace(/^﻿/, '').trimStart().startsWith('%PDF-')
}

export type EditedConfig =
  | { ok: true; config?: unknown }
  | { ok: false; error: string }

/**
 * The reviewer's edited configuration. Empty means "no edit"; text that is not
 * JSON is an error to show, never a silent fallback to the original proposal.
 */
export function parseEditedConfig(text: string): EditedConfig {
  if (!text.trim()) return { ok: true }
  try {
    return { ok: true, config: JSON.parse(text) }
  } catch (e) {
    const detail = e instanceof Error ? e.message : String(e)
    return { ok: false, error: `The edited configuration is not valid JSON: ${detail}` }
  }
}

/** A copilot request the server refused: its status, `code` and Retry-After. */
export class CopilotRequestError extends Error {
  readonly status: number
  readonly code?: string
  readonly retryAfterSecs?: number

  constructor(message: string, status: number, code?: string, retryAfterSecs?: number) {
    super(message)
    this.name = 'CopilotRequestError'
    this.status = status
    this.code = code
    this.retryAfterSecs = retryAfterSecs
  }
}

/** The error for a refused copilot response, from its JSON body and Retry-After header. */
export async function copilotRequestError(r: Response): Promise<CopilotRequestError> {
  const body = await r.json().catch(() => ({})) as { error?: unknown; code?: unknown; retry_after_secs?: unknown }
  const header = Number(r.headers.get('Retry-After'))
  const retryAfterSecs = typeof body.retry_after_secs === 'number'
    ? body.retry_after_secs
    : Number.isFinite(header) && header > 0 ? header : undefined
  const message = typeof body.error === 'string' && body.error ? body.error : `Request failed: ${r.status}`
  return new CopilotRequestError(message, r.status, typeof body.code === 'string' ? body.code : undefined, retryAfterSecs)
}

/** What to tell the user when generating a proposal failed. */
export function generationErrorMessage(err: unknown): string {
  if (!(err instanceof Error)) return String(err)
  const { code, retryAfterSecs } = err as Partial<CopilotRequestError>
  switch (code) {
    case 'ai_truncated':
      return 'The AI reply was cut off at its output limit, so it was not used. Send less at a time (part of the datasheet, or fewer tags).'
    case 'ai_invalid_api_key':
      return 'The AI provider rejected the API key. Check the key in the AI settings.'
    case 'ai_rate_limited':
      return retryAfterSecs
        ? `The AI provider's rate limit or quota was reached. Try again in ${retryAfterSecs} s.`
        : "The AI provider's rate limit or quota was reached. Try again later."
    case 'ai_not_configured':
      return 'No AI provider is set up. Configure one in the AI settings first.'
    case 'ai_timeout':
      return 'The AI provider took too long to answer. Try again, or send less at a time.'
    default:
      // The server's message is written for the user (a PDF refusal, a provider error).
      return err.message || 'Generating the proposal failed.'
  }
}
