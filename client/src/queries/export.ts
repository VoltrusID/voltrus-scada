export interface ExportParams {
  from?: string
  to?: string
  device?: string
  metric?: string
  aggregation?: string
  interval?: string
  format?: 'json' | 'csv'
  limit?: number
}

export function buildExportUrl(params: ExportParams): string {
  const sp = new URLSearchParams()
  if (params.from) sp.set('from', params.from)
  if (params.to) sp.set('to', params.to)
  if (params.device) sp.set('device', params.device)
  if (params.metric) sp.set('metric', params.metric)
  if (params.aggregation && params.aggregation !== 'none') sp.set('aggregation', params.aggregation)
  if (params.interval) sp.set('interval', params.interval)
  if (params.format) sp.set('format', params.format)
  if (params.limit) sp.set('limit', String(params.limit))
  return `/api/v1/data/export?${sp.toString()}`
}

export async function triggerExport(params: ExportParams): Promise<void> {
  const url = buildExportUrl(params)
  if (params.format === 'csv') {
    // CSV: open URL directly, browser handles download via Content-Disposition
    window.open(url, '_blank')
  } else {
    // JSON: fetch + create blob + trigger download
    const r = await fetch(url, { credentials: 'include' })
    if (!r.ok) throw new Error(`Export failed: ${r.status}`)
    const blob = await r.blob()
    const blobUrl = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = blobUrl
    a.download = `export-${Date.now()}.json`
    document.body.appendChild(a)
    a.click()
    a.remove()
    URL.revokeObjectURL(blobUrl)
  }
}
