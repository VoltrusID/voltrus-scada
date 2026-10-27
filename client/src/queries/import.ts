import { useMutation } from '@tanstack/react-query'
import { fetchJson } from './fetch'

export interface CsvImportResult {
  total_rows: number
  imported: number
  skipped: number
  errors: { row?: number; error: string; tag?: string }[]
  dry_run: boolean
  mode: string
  column_mapping: Record<string, number>
}

export interface CsvImportParams {
  file: File
  dryRun?: boolean
  delimiter?: string
  column_mapping?: string
}

export function useImportCsv() {
  return useMutation({
    mutationFn: async ({ file, dryRun, delimiter, column_mapping }: CsvImportParams) => {
      const sp = new URLSearchParams()
      if (dryRun) sp.set('dry_run', 'true')
      if (delimiter) sp.set('delimiter', delimiter)
      if (column_mapping) sp.set('column_mapping', column_mapping)
      const qs = sp.toString()

      const formData = new FormData()
      formData.append('file', file)

      const res = await fetch(`/api/v1/import/csv${qs ? '?' + qs : ''}`, {
        method: 'POST',
        credentials: 'include',
        body: formData,
      })
      if (!res.ok) {
        const body = await res.json().catch(() => ({}))
        throw new Error(body.error || `Import failed: ${res.status}`)
      }
      return res.json() as Promise<CsvImportResult>
    },
  })
}

export interface ExcelImportParams {
  file: File
  dryRun?: boolean
  sheet?: string
  column_mapping?: string
}

export interface ExcelImportResult extends CsvImportResult {
  sheet: string
  available_sheets: string[]
}

export function useImportExcel() {
  return useMutation({
    mutationFn: async ({ file, dryRun, sheet, column_mapping }: ExcelImportParams) => {
      const sp = new URLSearchParams()
      if (dryRun) sp.set('dry_run', 'true')
      if (sheet) sp.set('sheet', sheet)
      if (column_mapping) sp.set('column_mapping', column_mapping)
      const qs = sp.toString()

      const formData = new FormData()
      formData.append('file', file)

      const res = await fetch(`/api/v1/import/excel${qs ? '?' + qs : ''}`, {
        method: 'POST',
        credentials: 'include',
        body: formData,
      })
      if (!res.ok) {
        const body = await res.json().catch(() => ({}))
        throw new Error(body.error || `Import failed: ${res.status}`)
      }
      return res.json() as Promise<ExcelImportResult>
    },
  })
}
