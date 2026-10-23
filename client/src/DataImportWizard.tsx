import { useState, useCallback } from 'react';
import { useImportCsv, useImportExcel, type CsvImportResult } from './queries';
import { inputStyle, btnStyle } from './styles';
import { ImportColumnMapper, type ColumnMapping } from './ImportColumnMapper';

type WizardStep = 'upload' | 'mapping' | 'preview' | 'import' | 'report';

type FileType = 'csv' | 'excel';

function detectFileType(f: File): FileType | null {
  const name = f.name.toLowerCase();
  if (name.endsWith('.xlsx') || name.endsWith('.xls') || name.endsWith('.xlsm') || name.endsWith('.xlsb')) return 'excel';
  if (f.type === 'text/csv' || name.endsWith('.csv')) return 'csv';
  return null;
}

export function DataImportWizard({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const [step, setStep] = useState<WizardStep>('upload');
  const [file, setFile] = useState<File | null>(null);
  const [fileType, setFileType] = useState<FileType | null>(null);
  const [mode, setMode] = useState<'merge' | 'replace'>('merge');
  const [delimiter, setDelimiter] = useState(',');
  const [sheet, setSheet] = useState('');
  const [dragOver, setDragOver] = useState(false);
  const [previewResult, setPreviewResult] = useState<CsvImportResult | null>(null);
  const [reportResult, setReportResult] = useState<CsvImportResult | null>(null);
  const [fileHeaders, setFileHeaders] = useState<string[]>([]);
  const [columnMapping, setColumnMapping] = useState<Record<string, number> | null>(null);

  const importCsvMut = useImportCsv();
  const importExcelMut = useImportExcel();

  const importMut = fileType === 'excel' ? importExcelMut : importCsvMut;

  const handleFile = useCallback((f: File) => {
    const detected = detectFileType(f);
    if (!detected) {
      alert('Please select a CSV or Excel file (.csv, .xlsx, .xls).');
      return;
    }
    setFile(f);
    setFileType(detected);
    setPreviewResult(null);
    setReportResult(null);
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const f = e.dataTransfer.files[0];
    if (f) handleFile(f);
  }, [handleFile]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(true);
  }, []);

  const handleDragLeave = useCallback(() => {
    setDragOver(false);
  }, []);

  const runPreview = (mappingOverride?: Record<string, number> | null) => {
    if (!file || !fileType) return;
    const mappingParam = mappingOverride ? JSON.stringify(mappingOverride) : undefined;
    if (fileType === 'excel') {
      importExcelMut.mutate({ file, dryRun: true, mode, sheet: sheet || undefined, column_mapping: mappingParam }, {
        onSuccess: (res) => {
          setPreviewResult(res);
          const hdrs = Object.entries(res.column_mapping)
            .sort(([, a], [, b]) => a - b)
            .map(([name]) => name);
          setFileHeaders(hdrs);
          setStep('preview');
        },
      });
    } else {
      importCsvMut.mutate({ file, dryRun: true, mode, delimiter: delimiter || ',', column_mapping: mappingParam }, {
        onSuccess: (res) => {
          setPreviewResult(res);
          const hdrs = Object.entries(res.column_mapping)
            .sort(([, a], [, b]) => a - b)
            .map(([name]) => name);
          setFileHeaders(hdrs);
          setStep('preview');
        },
      });
    }
  };

  const runImport = () => {
    if (!file || !fileType) return;
    setStep('import');
    const mappingParam = columnMapping ? JSON.stringify(columnMapping) : undefined;
    if (fileType === 'excel') {
      importExcelMut.mutate({ file, dryRun: false, mode, sheet: sheet || undefined, column_mapping: mappingParam }, {
        onSuccess: (res) => {
          setReportResult(res);
          setStep('report');
        },
      });
    } else {
      importCsvMut.mutate({ file, dryRun: false, mode, delimiter: delimiter || ',', column_mapping: mappingParam }, {
        onSuccess: (res) => {
          setReportResult(res);
          setStep('report');
        },
      });
    }
  };

  const reset = () => {
    setStep('upload');
    setFile(null);
    setFileType(null);
    setPreviewResult(null);
    setReportResult(null);
    setFileHeaders([]);
    setMode('merge');
    setDelimiter(',');
    setSheet('');
  };

  const dropStyle: React.CSSProperties = {
    border: `2px dashed ${dragOver ? 'var(--cyan)' : 'var(--border)'}`,
    borderRadius: 10,
    padding: 40,
    textAlign: 'center',
    background: dragOver ? 'rgba(34,197,94,.05)' : 'var(--card)',
    transition: 'background .2s, border-color .2s',
    cursor: 'pointer',
  };

  const stepIndicator = (label: string, s: WizardStep, active: boolean, completed: boolean) => (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, opacity: active || completed ? 1 : 0.4 }}>
      <div style={{
        width: 24, height: 24, borderRadius: '50%', display: 'flex', alignItems: 'center', justifyContent: 'center',
        fontSize: 11, fontWeight: 700,
        background: completed ? 'var(--green)' : active ? 'var(--cyan)' : 'var(--border)',
        color: completed || active ? '#fff' : 'var(--muted)',
      }}>
        {completed ? '✓' : s === 'upload' ? '1' : s === 'mapping' ? '2' : s === 'preview' ? '3' : s === 'import' ? '4' : '5'}
      </div>
      <span style={{ fontSize: 12, fontWeight: 600, color: active ? 'var(--cyan)' : 'var(--fg)' }}>{label}</span>
    </div>
  );

  const content = (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 640 }}>
      {/* Steps */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 16, padding: '12px 16px', background: 'var(--card)', borderRadius: 10, border: '1px solid var(--border)' }}>
        {stepIndicator('Upload', 'upload', step === 'upload', step !== 'upload')}
        <span style={{ color: 'var(--border)' }}>→</span>
        {stepIndicator('Mapping', 'mapping', step === 'mapping', ['preview', 'import', 'report'].includes(step))}
        <span style={{ color: 'var(--border)' }}>→</span>
        {stepIndicator('Preview', 'preview', step === 'preview', step === 'import' || step === 'report')}
        <span style={{ color: 'var(--border)' }}>→</span>
        {stepIndicator('Import', 'import', step === 'import', step === 'report')}
        <span style={{ color: 'var(--border)' }}>→</span>
        {stepIndicator('Report', 'report', step === 'report', false)}
      </div>

      {/* Upload step */}
      {step === 'upload' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div
            style={dropStyle}
            onDrop={handleDrop}
            onDragOver={handleDragOver}
            onDragLeave={handleDragLeave}
            onClick={() => document.getElementById('csv-upload')?.click()}
          >
            <div style={{ fontSize: 32, marginBottom: 8, color: 'var(--muted)' }}>
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
            </div>
            <div style={{ fontSize: 14, color: 'var(--fg)', fontWeight: 600 }}>Drop CSV or Excel file here or click to browse</div>
            <div style={{ fontSize: 12, color: 'var(--muted)', marginTop: 4 }}>Supports .csv, .xlsx, .xls, .xlsm files</div>
            <input id="csv-upload" type="file" accept=".csv,.xlsx,.xls,.xlsm,.xlsb" style={{ display: 'none' }} onChange={e => { const f = e.target.files?.[0]; if (f) handleFile(f); }} />
          </div>
          {file && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 12px', background: 'var(--bg)', borderRadius: 6 }}>
              <span style={{ fontSize: 13, color: 'var(--fg)' }}>{file.name}</span>
              <span style={{ fontSize: 11, color: 'var(--muted)', marginLeft: 'auto' }}>{(file.size / 1024).toFixed(1)} KB</span>
            </div>
          )}
          <div style={{ display: 'flex', gap: 8 }}>
            <div style={{ flex: 1 }}>
              <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', display: 'block', marginBottom: 4 }}>Mode</label>
              <select value={mode} onChange={e => setMode(e.target.value as 'merge' | 'replace')} style={{ ...inputStyle, cursor: 'pointer' }}>
                <option value="merge">Merge (update existing, add new)</option>
                <option value="replace">Replace (clear existing first)</option>
              </select>
            </div>
            {fileType === 'csv' ? (
              <div style={{ flex: 1 }}>
                <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', display: 'block', marginBottom: 4 }}>Delimiter</label>
                <input value={delimiter} onChange={e => setDelimiter(e.target.value)} placeholder="," style={inputStyle} />
              </div>
            ) : fileType === 'excel' ? (
              <div style={{ flex: 1 }}>
                <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', display: 'block', marginBottom: 4 }}>Sheet (blank = first)</label>
                <input value={sheet} onChange={e => setSheet(e.target.value)} placeholder="Sheet1" style={inputStyle} />
              </div>
            ) : null}
          </div>
          <button onClick={() => runPreview()} disabled={!file || importMut.isPending} style={{ ...btnStyle, alignSelf: 'flex-start', borderColor: 'var(--cyan)', color: 'var(--cyan)' }}>
            {importMut.isPending ? 'Previewing...' : 'Preview Import'}
          </button>
        </div>
      )}

      {/* Preview step */}
      {step === 'preview' && previewResult && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div style={{ padding: 16, background: 'var(--card)', borderRadius: 10, border: '1px solid var(--border)' }}>
            <div style={{ fontSize: 14, fontWeight: 700, marginBottom: 12 }}>Dry Run Results</div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12 }}>
              <div style={{ textAlign: 'center', padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--fg)' }}>{previewResult.total_rows}</div>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Total Rows</div>
              </div>
              <div style={{ textAlign: 'center', padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--green)' }}>{previewResult.imported}</div>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Would Import</div>
              </div>
              <div style={{ textAlign: 'center', padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--warn)' }}>{previewResult.skipped}</div>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Would Skip</div>
              </div>
            </div>
            {previewResult.errors.length > 0 && (
              <div style={{ marginTop: 12 }}>
                <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--crit)', marginBottom: 8 }}>Errors ({previewResult.errors.length})</div>
                <div style={{ maxHeight: 160, overflow: 'auto', display: 'flex', flexDirection: 'column', gap: 4 }}>
                  {previewResult.errors.map((err, i) => (
                    <div key={i} style={{ fontSize: 12, padding: '6px 8px', background: 'rgba(239,68,68,.08)', borderRadius: 4, color: 'var(--crit)' }}>
                      Row {err.row ?? '?'}: {err.error} {err.tag ? `(${err.tag})` : ''}
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button onClick={() => setStep('upload')} style={btnStyle}>Back</button>
            <button onClick={() => setStep('mapping')} style={{ ...btnStyle, borderColor: 'var(--cyan)', color: 'var(--cyan)' }}>
              Edit Column Mapping
            </button>
            <button onClick={runImport} disabled={importMut.isPending} style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}>
              {importMut.isPending ? 'Importing...' : 'Confirm Import'}
            </button>
          </div>
        </div>
      )}

      {/* Mapping step */}
      {step === 'mapping' && previewResult && (
        <ImportColumnMapper
          headers={fileHeaders}
          detectedMapping={previewResult.column_mapping}
          onConfirm={(mapping: ColumnMapping) => {
            const override: Record<string, number> = {};
            for (const [k, v] of Object.entries(mapping)) {
              if (v !== null) override[k] = v;
            }
            const mappingState = Object.keys(override).length > 0 ? override : null;
            setColumnMapping(mappingState);
            runPreview(mappingState);
          }}
          onCancel={() => setStep('preview')}
        />
      )}

      {/* Import step */}
      {step === 'import' && (
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 16, padding: 40 }}>
          <div style={{ width: 40, height: 40, border: '3px solid var(--border)', borderTopColor: 'var(--cyan)', borderRadius: '50%', animation: 'spin 1s linear infinite' }} />
          <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
          <div style={{ fontSize: 14, color: 'var(--fg)', fontWeight: 600 }}>Importing data...</div>
          <div style={{ fontSize: 12, color: 'var(--muted)' }}>Please do not close this window.</div>
        </div>
      )}

      {/* Report step */}
      {step === 'report' && reportResult && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div style={{ padding: 16, background: 'var(--card)', borderRadius: 10, border: '1px solid var(--border)' }}>
            <div style={{ fontSize: 14, fontWeight: 700, marginBottom: 12 }}>Import Report</div>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12 }}>
              <div style={{ textAlign: 'center', padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--fg)' }}>{reportResult.total_rows}</div>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Total Rows</div>
              </div>
              <div style={{ textAlign: 'center', padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--green)' }}>{reportResult.imported}</div>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Imported</div>
              </div>
              <div style={{ textAlign: 'center', padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
                <div style={{ fontSize: 20, fontWeight: 700, color: 'var(--warn)' }}>{reportResult.skipped}</div>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Skipped</div>
              </div>
            </div>
            {reportResult.errors.length > 0 && (
              <div style={{ marginTop: 12 }}>
                <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--crit)', marginBottom: 8 }}>Errors ({reportResult.errors.length})</div>
                <div style={{ maxHeight: 160, overflow: 'auto', display: 'flex', flexDirection: 'column', gap: 4 }}>
                  {reportResult.errors.map((err, i) => (
                    <div key={i} style={{ fontSize: 12, padding: '6px 8px', background: 'rgba(239,68,68,.08)', borderRadius: 4, color: 'var(--crit)' }}>
                      Row {err.row ?? '?'}: {err.error} {err.tag ? `(${err.tag})` : ''}
                    </div>
                  ))}
                </div>
              </div>
            )}
            {reportResult.errors.length === 0 && (
              <div style={{ marginTop: 12, padding: '10px 12px', background: 'rgba(34,197,94,.08)', borderRadius: 6, color: 'var(--green)', fontSize: 13, fontWeight: 600 }}>
                Import completed successfully with no errors.
              </div>
            )}
          </div>
          <button onClick={reset} style={{ ...btnStyle, alignSelf: 'flex-start', borderColor: 'var(--cyan)', color: 'var(--cyan)' }}>Import Another File</button>
        </div>
      )}
    </div>
  )

  if (embedded) {
    return <>{content}</>
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column', padding: 24 }}>
      <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <button onClick={onClose} aria-label="Close import wizard" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
            </button>
            <h1 style={{ fontSize: 20, fontWeight: 700 }}>Import Data</h1>
          </div>
        </div>
        <div style={{ flex: 1, overflow: 'auto' }}>
          {content}
        </div>
      </div>
    </div>
  )
}
