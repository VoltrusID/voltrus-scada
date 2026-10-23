import { useState, useEffect, useCallback } from 'react'
import { monoInputStyle, labelStyle, labelTextStyle, btnStyle } from '../styles'
import { listScripts, createScript, updateScript, deleteScript, testScript } from '../api/scripts'
import type { Script, TestResult } from '../api/scripts'
import { ScriptExecutionHistory } from './ScriptExecutionHistory'

// ── Types ──────────────────────────────────────────────────────────

interface NewScript {
  name: string
  source: string
  cpu_limit_ms: number
  memory_limit_bytes: number
  enabled: boolean
  syntax_error: string | null
}

// ── Toggle switch ──────────────────────────────────────────────────

function ToggleSwitch({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean
  onChange: (val: boolean) => void
  disabled?: boolean
}) {
  return (
    <button
      onClick={() => !disabled && onChange(!checked)}
      disabled={disabled}
      style={{
        width: 40, height: 22, borderRadius: 11,
        border: 'none', cursor: disabled ? 'not-allowed' : 'pointer',
        position: 'relative',
        background: checked ? 'var(--ok)' : 'var(--border)',
        transition: 'background 0.2s',
        opacity: disabled ? 0.5 : 1,
      }}
    >
      <div style={{
        width: 16, height: 16, borderRadius: '50%', background: '#fff',
        position: 'absolute', top: 3,
        left: checked ? 21 : 3,
        transition: 'left 0.2s',
      }} />
    </button>
  )
}

// ── Confirmation dialog ────────────────────────────────────────────

function ConfirmDialog({
  title,
  message,
  confirmLabel,
  confirmColor,
  onConfirm,
  onCancel,
  loading,
}: {
  title: string
  message: string
  confirmLabel: string
  confirmColor: string
  onConfirm: () => void
  onCancel: () => void
  loading: boolean
}) {
  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 1100,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <div onClick={onCancel} style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,.5)' }} />
      <div style={{
        position: 'relative', background: 'var(--card)', border: '1px solid var(--border)',
        borderRadius: 10, padding: 20, maxWidth: 400, width: '90%',
        display: 'flex', flexDirection: 'column', gap: 14,
      }}>
        <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--fg)' }}>{title}</div>
        <div style={{ fontSize: 13, color: 'var(--muted)', lineHeight: 1.5 }}>{message}</div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button
            onClick={onCancel}
            disabled={loading}
            style={{
              ...btnStyle,
              border: '1px solid var(--border)',
              color: 'var(--muted)',
              cursor: loading ? 'not-allowed' : 'pointer',
              opacity: loading ? 0.5 : 1,
            }}
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={loading}
            style={{
              ...btnStyle,
              border: `1px solid ${confirmColor}`,
              background: confirmColor === 'var(--crit)'
                ? 'rgba(239,68,68,.15)'
                : confirmColor === 'var(--ok)'
                  ? 'rgba(34,197,94,.15)'
                  : 'rgba(34,211,238,.15)',
              color: confirmColor,
              cursor: loading ? 'not-allowed' : 'pointer',
              opacity: loading ? 0.5 : 1,
            }}
          >
            {loading ? 'Please wait...' : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Main component ─────────────────────────────────────────────────

export function ScriptEditor() {
  const [scripts, setScripts] = useState<Script[]>([])
  const [selectedId, setSelectedId] = useState<number | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [error, setError] = useState('')

  // New-script form state (used when selectedId is null)
  const [newScript, setNewScript] = useState<NewScript>({
    name: '',
    source: '// Write your script here\nfunction main(input) {\n  return input;\n}\n',
    cpu_limit_ms: 5000,
    memory_limit_bytes: 32 * 1024 * 1024,
    enabled: true,
    syntax_error: null,
  })

  // Test panel state
  const [testInput, setTestInput] = useState('{\n  "value": 42\n}')
  const [testResult, setTestResult] = useState<TestResult | null>(null)

  // Delete confirmation
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false)
  const [deleting, setDeleting] = useState(false)

  // Fetch scripts
  const fetchScripts = useCallback(async () => {
    try {
      const data = await listScripts()
      setScripts(data)
      setError('')
    } catch (e: any) {
      setError(e.message || 'Failed to load scripts')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    fetchScripts()
  }, [fetchScripts])

  // Current working script
  const currentScript = selectedId !== null ? scripts.find(s => s.id === selectedId) : null

  // Handlers
  const handleCreate = async () => {
    if (!newScript.name.trim()) {
      setError('Script name is required')
      return
    }
    setSaving(true)
    setError('')
    try {
      const created = await createScript({
        name: newScript.name.trim(),
        source: newScript.source,
        cpu_limit_ms: newScript.cpu_limit_ms,
        memory_limit_bytes: newScript.memory_limit_bytes,
      })
      await fetchScripts()
      setSelectedId(created.id)
    } catch (e: any) {
      setError(e.message || 'Failed to create script')
    } finally {
      setSaving(false)
    }
  }

  const handleSave = async () => {
    if (selectedId === null) return
    setSaving(true)
    setError('')
    try {
      const s = scripts.find(sc => sc.id === selectedId)
      if (!s) return
      await updateScript(selectedId, {
        name: s.name,
        source: s.source,
        cpu_limit_ms: s.cpu_limit_ms,
        memory_limit_bytes: s.memory_limit_bytes,
        enabled: s.enabled,
      })
      await fetchScripts()
    } catch (e: any) {
      setError(e.message || 'Failed to save script')
    } finally {
      setSaving(false)
    }
  }

  const handleDelete = async () => {
    if (selectedId === null) return
    setDeleting(true)
    setError('')
    try {
      await deleteScript(selectedId)
      setShowDeleteConfirm(false)
      setSelectedId(null)
      await fetchScripts()
    } catch (e: any) {
      setError(e.message || 'Failed to delete script')
    } finally {
      setDeleting(false)
    }
  }

  const handleTest = async () => {
    if (selectedId === null) return
    setTesting(true)
    setTestResult(null)
    setError('')
    try {
      let parsed: unknown
      try {
        parsed = JSON.parse(testInput)
      } catch {
        setError('Test input must be valid JSON')
        setTesting(false)
        return
      }
      const result = await testScript(selectedId, parsed)
      setTestResult(result)
    } catch (e: any) {
      setError(e.message || 'Test execution failed')
    } finally {
      setTesting(false)
    }
  }

  const updateCurrentScript = (patch: Partial<Script>) => {
    setScripts(prev => prev.map(s => s.id === selectedId ? { ...s, ...patch } : s))
  }

  const handleNewFieldChange = (field: keyof NewScript, value: any) => {
    setNewScript(prev => ({ ...prev, [field]: value }))
  }

  // ── Render ──────────────────────────────────────────────────────

  if (loading) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: 300, color: 'var(--muted)' }}>
        Loading scripts...
      </div>
    )
  }

  return (
    <div style={{ display: 'flex', gap: 0, height: '100%', minHeight: 500 }}>
      {/* ── Sidebar: Script list ─────────────────────────────────── */}
      <div style={{
        width: 220, minWidth: 220,
        borderRight: '1px solid var(--border)',
        display: 'flex', flexDirection: 'column',
        background: 'var(--card)',
      }}>
        <div style={{
          padding: '12px 12px 8px',
          fontSize: 11, fontWeight: 600, color: 'var(--muted)',
          textTransform: 'uppercase', letterSpacing: '.05em',
          borderBottom: '1px solid var(--border)',
          display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        }}>
          Scripts
          <button
            onClick={() => setSelectedId(null)}
            style={{
              padding: '3px 8px', borderRadius: 4, fontSize: 10, cursor: 'pointer',
              border: '1px solid var(--cyan)', background: 'transparent',
              color: 'var(--cyan)', fontWeight: 600,
            }}
          >
            + New
          </button>
        </div>
        <div style={{ flex: 1, overflow: 'auto' }}>
          {scripts.map(s => (
            <div
              key={s.id}
              onClick={() => { setSelectedId(s.id); setTestResult(null) }}
              style={{
                padding: '8px 12px', cursor: 'pointer',
                borderLeft: s.id === selectedId ? '2px solid var(--cyan)' : '2px solid transparent',
                background: s.id === selectedId ? 'rgba(34,211,238,.06)' : 'transparent',
                display: 'flex', justifyContent: 'space-between', alignItems: 'center',
              }}
            >
              <div style={{ minWidth: 0, flex: 1 }}>
                <div style={{
                  fontSize: 12, fontWeight: 500, color: 'var(--fg)',
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}>
                  {s.name}
                </div>
                <div style={{ fontSize: 10, color: s.enabled ? 'var(--ok)' : 'var(--muted)', marginTop: 2 }}>
                  {s.enabled ? 'Enabled' : 'Disabled'}
                  {s.syntax_error && <span style={{ color: 'var(--crit)', marginLeft: 6 }}>Error</span>}
                </div>
              </div>
            </div>
          ))}
          {scripts.length === 0 && (
            <div style={{ padding: '16px 12px', fontSize: 11, color: 'var(--muted)' }}>
              No scripts yet
            </div>
          )}
        </div>
      </div>

      {/* ── Main editor area ─────────────────────────────────────── */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'auto' }}>
        {/* New script form */}
        {selectedId === null && (
          <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div style={{ fontSize: 14, fontWeight: 600 }}>New Script</div>

            <label style={labelStyle}>
              <span style={labelTextStyle}>Name</span>
              <input
                type="text"
                value={newScript.name}
                onChange={e => handleNewFieldChange('name', e.target.value)}
                placeholder="Script name"
                style={monoInputStyle}
              />
            </label>

            <label style={labelStyle}>
              <span style={labelTextStyle}>Source Code (JavaScript)</span>
              <textarea
                value={newScript.source}
                onChange={e => handleNewFieldChange('source', e.target.value)}
                rows={14}
                spellCheck={false}
                style={{
                  ...monoInputStyle,
                  resize: 'vertical',
                  lineHeight: 1.5,
                  fontSize: 12,
                  tabSize: 2,
                }}
              />
            </label>

            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
              <label style={labelStyle}>
                <span style={labelTextStyle}>CPU Limit (ms)</span>
                <input
                  type="number"
                  min={100}
                  step={100}
                  value={newScript.cpu_limit_ms}
                  onChange={e => handleNewFieldChange('cpu_limit_ms', parseInt(e.target.value) || 5000)}
                  style={monoInputStyle}
                />
              </label>
              <label style={labelStyle}>
                <span style={labelTextStyle}>Memory Limit (MB)</span>
                <input
                  type="number"
                  min={1}
                  step={1}
                  value={Math.round(newScript.memory_limit_bytes / (1024 * 1024))}
                  onChange={e => handleNewFieldChange('memory_limit_bytes', (parseInt(e.target.value) || 32) * 1024 * 1024)}
                  style={monoInputStyle}
                />
              </label>
            </div>

            <button
              onClick={handleCreate}
              disabled={saving || !newScript.name.trim()}
              style={{
                ...btnStyle,
                alignSelf: 'flex-start',
                border: '1px solid var(--cyan)',
                background: 'var(--cyan)',
                color: '#fff',
                cursor: saving || !newScript.name.trim() ? 'not-allowed' : 'pointer',
                opacity: saving || !newScript.name.trim() ? 0.5 : 1,
              }}
            >
              {saving ? 'Creating...' : 'Create Script'}
            </button>
          </div>
        )}

        {/* Existing script editor */}
        {currentScript && (
          <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
            {/* Header row */}
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <input
                  type="text"
                  value={currentScript.name}
                  onChange={e => updateCurrentScript({ name: e.target.value })}
                  style={{ ...monoInputStyle, fontSize: 14, fontWeight: 600, border: 'none', padding: 0 }}
                />
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ fontSize: 11, color: currentScript.enabled ? 'var(--ok)' : 'var(--muted)' }}>
                  {currentScript.enabled ? 'Enabled' : 'Disabled'}
                </span>
                <ToggleSwitch
                  checked={currentScript.enabled}
                  onChange={val => updateCurrentScript({ enabled: val })}
                />
              </div>
            </div>

            {/* Code editor */}
            <label style={labelStyle}>
              <span style={labelTextStyle}>Source Code (JavaScript)</span>
              <textarea
                value={currentScript.source}
                onChange={e => updateCurrentScript({ source: e.target.value, syntax_error: null })}
                rows={16}
                spellCheck={false}
                style={{
                  ...monoInputStyle,
                  resize: 'vertical',
                  lineHeight: 1.5,
                  fontSize: 12,
                  tabSize: 2,
                }}
              />
            </label>

            {/* Syntax error banner */}
            {currentScript.syntax_error && (
              <div style={{
                fontSize: 12, color: 'var(--crit)',
                padding: '8px 12px', background: 'rgba(239,68,68,.1)',
                borderRadius: 6, border: '1px solid rgba(239,68,68,.2)',
                fontFamily: 'var(--font-mono)',
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-word',
              }}>
                {currentScript.syntax_error}
              </div>
            )}

            {/* Config row */}
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
              <label style={labelStyle}>
                <span style={labelTextStyle}>CPU Limit (ms)</span>
                <input
                  type="number"
                  min={100}
                  step={100}
                  value={currentScript.cpu_limit_ms}
                  onChange={e => updateCurrentScript({ cpu_limit_ms: parseInt(e.target.value) || 5000 })}
                  style={monoInputStyle}
                />
              </label>
              <label style={labelStyle}>
                <span style={labelTextStyle}>Memory Limit (MB)</span>
                <input
                  type="number"
                  min={1}
                  step={1}
                  value={Math.round(currentScript.memory_limit_bytes / (1024 * 1024))}
                  onChange={e => updateCurrentScript({ memory_limit_bytes: (parseInt(e.target.value) || 32) * 1024 * 1024 })}
                  style={monoInputStyle}
                />
              </label>
            </div>

            {/* Action buttons */}
            <div style={{ display: 'flex', gap: 8 }}>
              <button
                onClick={handleSave}
                disabled={saving}
                style={{
                  ...btnStyle,
                  border: '1px solid var(--cyan)',
                  background: 'var(--cyan)',
                  color: '#fff',
                  cursor: saving ? 'not-allowed' : 'pointer',
                  opacity: saving ? 0.5 : 1,
                }}
              >
                {saving ? 'Saving...' : 'Save'}
              </button>
              <button
                onClick={() => setShowDeleteConfirm(true)}
                style={{
                  ...btnStyle,
                  border: '1px solid var(--crit)',
                  background: 'transparent',
                  color: 'var(--crit)',
                }}
              >
                Delete
              </button>
            </div>

            {/* ── Test panel ───────────────────────────────────────── */}
            <div style={{
              borderTop: '1px solid var(--border)', paddingTop: 12,
              display: 'flex', flexDirection: 'column', gap: 10,
            }}>
              <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.04em' }}>
                Test Panel
              </div>

              <label style={labelStyle}>
                <span style={labelTextStyle}>Sample Input (JSON)</span>
                <textarea
                  value={testInput}
                  onChange={e => setTestInput(e.target.value)}
                  rows={4}
                  spellCheck={false}
                  style={{
                    ...monoInputStyle,
                    resize: 'vertical',
                    fontSize: 12,
                    lineHeight: 1.5,
                  }}
                />
              </label>

              <button
                onClick={handleTest}
                disabled={testing}
                style={{
                  ...btnStyle,
                  alignSelf: 'flex-start',
                  border: '1px solid var(--ok)',
                  background: 'rgba(34,197,94,.15)',
                  color: 'var(--ok)',
                  cursor: testing ? 'not-allowed' : 'pointer',
                  opacity: testing ? 0.5 : 1,
                }}
              >
                {testing ? 'Running...' : 'Run Test'}
              </button>

              {/* Test result */}
              {testResult && (
                <div style={{
                  padding: '12px', borderRadius: 6,
                  background: testResult.success ? 'rgba(34,197,94,.06)' : 'rgba(239,68,68,.06)',
                  border: `1px solid ${testResult.success ? 'rgba(34,197,94,.2)' : 'rgba(239,68,68,.2)'}`,
                  display: 'flex', flexDirection: 'column', gap: 6,
                }}>
                  <div style={{
                    fontSize: 12, fontWeight: 600,
                    color: testResult.success ? 'var(--ok)' : 'var(--crit)',
                  }}>
                    {testResult.success ? 'Success' : 'Error'} ({testResult.duration_ms}ms)
                  </div>
                  {testResult.output && (
                    <pre style={{
                      margin: 0, fontSize: 11, fontFamily: 'var(--font-mono)',
                      color: 'var(--fg)', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                      maxHeight: 200, overflow: 'auto',
                    }}>
                      {testResult.output}
                    </pre>
                  )}
                  {testResult.error && (
                    <pre style={{
                      margin: 0, fontSize: 11, fontFamily: 'var(--font-mono)',
                      color: 'var(--crit)', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                    }}>
                      {testResult.error}
                    </pre>
                  )}
                </div>
              )}

              {/* Execution history */}
              <div style={{ marginTop: 4 }}>
                <ScriptExecutionHistory scriptId={currentScript.id} />
              </div>
            </div>
          </div>
        )}

        {/* Error banner */}
        {error && (
          <div style={{
            fontSize: 12, color: 'var(--crit)',
            padding: '6px 10px', background: 'rgba(239,68,68,.1)', borderRadius: 4,
          }}>
            {error}
          </div>
        )}
      </div>

      {/* Delete confirmation dialog */}
      {showDeleteConfirm && (
        <ConfirmDialog
          title="Delete Script"
          message={`Are you sure you want to delete "${currentScript?.name ?? 'this script'}"? This action cannot be undone.`}
          confirmLabel="Delete"
          confirmColor="var(--crit)"
          onConfirm={handleDelete}
          onCancel={() => setShowDeleteConfirm(false)}
          loading={deleting}
        />
      )}
    </div>
  )
}
