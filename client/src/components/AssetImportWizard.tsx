import { useState, useEffect, useRef } from 'react'
import { inputStyle, labelStyle, labelTextStyle } from '../styles'
import {
  useImportAssets,
  useConfirmImport,
} from '../queries/assets'

interface Props {
  onClose: () => void
}

type Step = 1 | 2 | 3

interface UploadResult {
  import_id: string
  rows: Record<string, unknown>[]
  total: number
  valid: number
  errors: string[]
}

export function AssetImportWizard({ onClose }: Props) {
  const [step, setStep] = useState<Step>(1)
  const [file, setFile] = useState<File | null>(null)
  const [preview, setPreview] = useState<UploadResult | null>(null)
  const [confirmResult, setConfirmResult] = useState<{ created: number; skipped_duplicates: number } | null>(null)
  const [errors, setErrors] = useState<string[]>([])
  const fileInputRef = useRef<HTMLInputElement>(null)

  const importAssets = useImportAssets()
  const confirmImport = useConfirmImport()

  // Escape key to close
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !importAssets.isPending && !confirmImport.isPending) onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose, importAssets.isPending, confirmImport.isPending])

  const handleBackdropClick = () => {
    if (!importAssets.isPending && !confirmImport.isPending) onClose()
  }

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const selected = e.target.files?.[0] ?? null
    setFile(selected)
    setPreview(null)
    setConfirmResult(null)
    setErrors([])
    setStep(1)
  }

  const handleUpload = async () => {
    if (!file) return
    setErrors([])
    try {
      const result = await importAssets.mutateAsync(file)
      setPreview(result)
      setStep(2)
    } catch (e: any) {
      setErrors([e.message || 'Failed to upload file'])
    }
  }

  const handleConfirm = async () => {
    if (!preview) return
    setErrors([])
    try {
      const result = await confirmImport.mutateAsync(preview.import_id)
      setConfirmResult(result)
      setStep(3)
    } catch (e: any) {
      setErrors([e.message || 'Failed to confirm import'])
    }
  }

  const handleReset = () => {
    setFile(null)
    setPreview(null)
    setConfirmResult(null)
    setErrors([])
    setStep(1)
    if (fileInputRef.current) fileInputRef.current.value = ''
  }

  const busy = importAssets.isPending || confirmImport.isPending

  const stepLabel = (n: Step, label: string) => (
    <span style={{
      fontSize: 11, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '.04em',
      color: step === n ? 'var(--cyan)' : 'var(--muted)',
      borderBottom: step === n ? '2px solid var(--cyan)' : '2px solid transparent',
      paddingBottom: 6,
    }}>
      {n}. {label}
    </span>
  )

  const validCount = preview?.valid ?? 0
  const errorCount = preview ? preview.total - preview.valid : 0

  return (
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)',
        display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60,
      }}
      onClick={handleBackdropClick}
    >
      <div
        style={{
          background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10,
          width: 520, maxHeight: '85vh', display: 'flex', flexDirection: 'column',
        }}
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{
          display: 'flex', justifyContent: 'space-between', alignItems: 'center',
          padding: '12px 16px', borderBottom: '1px solid var(--border)',
        }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Import Assets</div>
          <button
            onClick={onClose}
            disabled={busy}
            style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18 }}
          >&times;</button>
        </div>

        {/* Step indicators */}
        <div style={{
          display: 'flex', gap: 24, padding: '10px 16px',
          borderBottom: '1px solid var(--border)',
        }}>
          {stepLabel(1, 'Upload')}
          {stepLabel(2, 'Preview')}
          {stepLabel(3, 'Complete')}
        </div>

        {/* Body */}
        <div style={{ padding: 16, overflow: 'auto', flex: 1, display: 'flex', flexDirection: 'column', gap: 12 }}>
          {/* Errors */}
          {errors.length > 0 && (
            <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
              {errors.map((err, i) => <div key={i}>{err}</div>)}
            </div>
          )}

          {/* Step 1: File upload */}
          {step === 1 && (
            <>
              <label style={labelStyle}>
                <span style={labelTextStyle}>Select file (.csv or .xlsx)</span>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".csv,.xlsx"
                  onChange={handleFileChange}
                  style={inputStyle}
                  disabled={busy}
                />
              </label>
              {file && (
                <div style={{ padding: '8px 12px', borderRadius: 6, background: 'var(--bg)', border: '1px solid var(--border)' }}>
                  <div style={{ fontSize: 11, color: 'var(--muted)', marginBottom: 2 }}>Selected</div>
                  <div style={{ fontSize: 13, fontWeight: 600 }}>{file.name}</div>
                  <div style={{ fontSize: 11, color: 'var(--muted)' }}>
                    {(file.size / 1024).toFixed(1)} KB
                  </div>
                </div>
              )}
              {importAssets.isError && !errors.length && (
                <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
                  {importAssets.error.message}
                </div>
              )}
              <button
                onClick={handleUpload}
                disabled={!file || busy}
                style={{
                  padding: '10px 16px', borderRadius: 6,
                  border: `1px solid ${file && !busy ? 'var(--cyan)' : 'var(--border)'}`,
                  background: file && !busy ? 'var(--cyan)' : 'transparent',
                  color: file && !busy ? '#fff' : 'var(--muted)',
                  cursor: file && !busy ? 'pointer' : 'not-allowed',
                  fontSize: 13, fontWeight: 600, opacity: busy ? 0.6 : 1,
                }}
              >
                {importAssets.isPending ? 'Uploading...' : 'Upload & Preview'}
              </button>
            </>
          )}

          {/* Step 2: Preview */}
          {step === 2 && preview && (
            <>
              <div style={{
                display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8,
              }}>
                <div style={{ padding: '10px 12px', borderRadius: 6, background: 'var(--bg)', border: '1px solid var(--border)', textAlign: 'center' }}>
                  <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--fg)' }}>{preview.total}</div>
                  <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Total Rows</div>
                </div>
                <div style={{ padding: '10px 12px', borderRadius: 6, background: 'rgba(34,197,94,.08)', border: '1px solid var(--ok)', textAlign: 'center' }}>
                  <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--ok)' }}>{validCount}</div>
                  <div style={{ fontSize: 10, color: 'var(--ok)', textTransform: 'uppercase', fontWeight: 600 }}>Valid</div>
                </div>
                <div style={{ padding: '10px 12px', borderRadius: 6, background: 'rgba(239,68,68,.08)', border: '1px solid var(--crit)', textAlign: 'center' }}>
                  <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--crit)' }}>{errorCount}</div>
                  <div style={{ fontSize: 10, color: 'var(--crit)', textTransform: 'uppercase', fontWeight: 600 }}>Errors</div>
                </div>
              </div>

              {/* Preview table (first 5 rows) */}
              {preview.rows.length > 0 && (
                <div style={{ borderRadius: 6, border: '1px solid var(--border)', overflow: 'hidden', maxHeight: 200, overflowY: 'auto' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                    <thead>
                      <tr>
                        <th style={{ textAlign: 'left', padding: '6px 8px', fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600, borderBottom: '1px solid var(--border)', position: 'sticky', top: 0, background: 'var(--card)' }}>Name</th>
                        <th style={{ textAlign: 'left', padding: '6px 8px', fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600, borderBottom: '1px solid var(--border)', position: 'sticky', top: 0, background: 'var(--card)' }}>Type</th>
                        <th style={{ textAlign: 'left', padding: '6px 8px', fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600, borderBottom: '1px solid var(--border)', position: 'sticky', top: 0, background: 'var(--card)' }}>IP</th>
                        <th style={{ textAlign: 'left', padding: '6px 8px', fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600, borderBottom: '1px solid var(--border)', position: 'sticky', top: 0, background: 'var(--card)' }}>Vendor</th>
                      </tr>
                    </thead>
                    <tbody>
                      {preview.rows.slice(0, 5).map((a, i) => (
                        <tr key={i}>
                          <td style={{ padding: '5px 8px', fontSize: 11, borderBottom: '1px solid var(--border)' }}>{(a.name as string) || '--'}</td>
                          <td style={{ padding: '5px 8px', fontSize: 11, borderBottom: '1px solid var(--border)' }}>{(a.asset_type as string) || '--'}</td>
                          <td style={{ padding: '5px 8px', fontSize: 11, fontFamily: "var(--font-mono)", borderBottom: '1px solid var(--border)' }}>{(a.ip_address as string) || '--'}</td>
                          <td style={{ padding: '5px 8px', fontSize: 11, borderBottom: '1px solid var(--border)' }}>{(a.vendor as string) || '--'}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  {preview.rows.length > 5 && (
                    <div style={{ padding: '6px 8px', fontSize: 10, color: 'var(--muted)', textAlign: 'center', borderTop: '1px solid var(--border)' }}>
                      ...and {preview.rows.length - 5} more rows
                    </div>
                  )}
                </div>
              )}

              {confirmImport.isError && (
                <div style={{ fontSize: 12, color: 'var(--crit)', padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4 }}>
                  {confirmImport.error.message}
                </div>
              )}

              <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                <button
                  onClick={() => setStep(1)}
                  disabled={busy}
                  style={{
                    padding: '8px 14px', borderRadius: 6, border: '1px solid var(--border)',
                    background: 'transparent', color: 'var(--fg)', cursor: busy ? 'not-allowed' : 'pointer',
                    fontSize: 13, fontWeight: 600,
                  }}
                >
                  Back
                </button>
                <button
                  onClick={handleConfirm}
                  disabled={validCount === 0 || busy}
                  style={{
                    padding: '8px 14px', borderRadius: 6, border: `1px solid ${validCount > 0 && !busy ? 'var(--cyan)' : 'var(--border)'}`,
                    background: validCount > 0 && !busy ? 'var(--cyan)' : 'transparent',
                    color: validCount > 0 && !busy ? '#fff' : 'var(--muted)',
                    cursor: validCount > 0 && !busy ? 'pointer' : 'not-allowed',
                    fontSize: 13, fontWeight: 600, opacity: busy ? 0.6 : 1,
                  }}
                >
                  {confirmImport.isPending ? 'Importing...' : `Confirm Import (${validCount})`}
                </button>
              </div>
            </>
          )}

          {/* Step 3: Complete */}
          {step === 3 && confirmResult && (
            <>
              <div style={{ padding: '16px 12px', borderRadius: 6, background: 'rgba(34,197,94,.1)', border: '1px solid var(--ok)', textAlign: 'center' }}>
                <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--ok)', marginBottom: 12 }}>Import complete</div>
                <div style={{ display: 'flex', gap: 16, justifyContent: 'center' }}>
                  <div>
                    <div style={{ fontSize: 24, fontWeight: 700, color: 'var(--ok)' }}>{confirmResult.created}</div>
                    <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Created</div>
                  </div>
                  <div style={{ width: 1, background: 'var(--border)' }} />
                  <div>
                    <div style={{ fontSize: 24, fontWeight: 700, color: 'var(--muted)' }}>{confirmResult.skipped_duplicates}</div>
                    <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', fontWeight: 600 }}>Skipped</div>
                  </div>
                </div>
              </div>

              <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                <button
                  onClick={handleReset}
                  style={{
                    padding: '8px 14px', borderRadius: 6, border: '1px solid var(--border)',
                    background: 'transparent', color: 'var(--fg)', cursor: 'pointer',
                    fontSize: 13, fontWeight: 600,
                  }}
                >
                  Import Another
                </button>
                <button
                  onClick={onClose}
                  style={{
                    padding: '8px 14px', borderRadius: 6, border: '1px solid var(--cyan)',
                    background: 'var(--cyan)', color: '#fff', cursor: 'pointer',
                    fontSize: 13, fontWeight: 600,
                  }}
                >
                  Done
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
