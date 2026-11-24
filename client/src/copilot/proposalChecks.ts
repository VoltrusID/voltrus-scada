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
