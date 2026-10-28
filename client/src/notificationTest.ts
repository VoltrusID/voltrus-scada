/** One target of `POST /api/v1/notifications/test`. */
export interface TestTargetResult {
  to?: string
  url?: string
  target?: string
  status: 'sent' | 'error'
  error?: string
}

export interface TestOutcome {
  channel: string
  results: TestTargetResult[]
}

export interface TestSummary {
  success: boolean
  message: string
}

function targetOf(r: TestTargetResult): string {
  return r.to ?? r.url ?? r.target ?? 'target'
}

/** One line for the outcome of a test: all sent, or the first failures with their reasons. */
export function summarizeTestOutcome(outcome: TestOutcome): TestSummary {
  const results = outcome.results ?? []
  if (results.length === 0) return { success: false, message: 'No target was tested' }
  const failed = results.filter(r => r.status !== 'sent')
  if (failed.length === 0) {
    return { success: true, message: results.length === 1 ? 'Test notification sent' : `Test notification sent to ${results.length} targets` }
  }
  const first = failed.slice(0, 2).map(r => `${targetOf(r)}: ${r.error ?? 'failed'}`).join('; ')
  const more = failed.length > 2 ? ` (+${failed.length - 2} more)` : ''
  const sent = results.length - failed.length
  return { success: false, message: `${sent > 0 ? `${sent} sent, ` : ''}${failed.length} failed. ${first}${more}` }
}
