import { useState, useEffect } from 'react'
import {
  useOpcUaDevices,
  useOpcUaNodes,
  useCreateOpcUaDevice,
  useUpdateOpcUaDevice,
  useDeleteOpcUaDevice,
  useBrowseOpcUaDevice,
  useAddOpcUaNode,
  useDeleteOpcUaNode,
  useStartOpcUaDevice,
  useStopOpcUaDevice,
} from './queries/opcua'
import { inputStyle, btnStyle } from './styles'
import { OPCUA_MODES, OPCUA_POLICIES } from './protocolPayloads'
import type { OpcUaDevice, OpcUaNode, BrowseResult } from './queries/opcua'

export function OpcUaDeviceManager({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const { data: devices = [], isLoading } = useOpcUaDevices()
  const createMut = useCreateOpcUaDevice()
  const updateMut = useUpdateOpcUaDevice()
  const deleteMut = useDeleteOpcUaDevice()
  const browseMut = useBrowseOpcUaDevice()
  const addNodeMut = useAddOpcUaNode()
  const deleteNodeMut = useDeleteOpcUaNode()
  const startMut = useStartOpcUaDevice()
  const stopMut = useStopOpcUaDevice()

  const [filter, setFilter] = useState('')
  const [showForm, setShowForm] = useState(false)
  const [expandedId, setExpandedId] = useState<number | null>(null)

  // Browse dialog state
  const [browseDeviceId, setBrowseDeviceId] = useState<number | null>(null)
  const [browseResults, setBrowseResults] = useState<BrowseResult[]>([])
  const [selectedNodes, setSelectedNodes] = useState<Set<string>>(new Set())

  const filtered = devices.filter(d =>
    d.name.toLowerCase().includes(filter.toLowerCase()) ||
    d.url.toLowerCase().includes(filter.toLowerCase()),
  )

  useEffect(() => {
    if (embedded) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, embedded])

  const handleCreate = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    const fd = new FormData(e.currentTarget)
    createMut.mutate(
      {
        name: fd.get('name') as string,
        url: fd.get('url') as string,
        security_policy: fd.get('security_policy') as string,
        security_mode: fd.get('security_mode') as string,
        username: (fd.get('username') as string) ?? '',
        password: (fd.get('password') as string) ?? '',
        interval_ms: Number(fd.get('interval_ms')),
      },
      { onSuccess: () => setShowForm(false) },
    )
  }

  const handleBrowse = async (deviceId: number) => {
    setBrowseDeviceId(deviceId)
    setSelectedNodes(new Set())
    setBrowseResults([])
    browseMut.mutate(deviceId, {
      onSuccess: (results) => setBrowseResults(results),
    })
  }

  const handleAddSelectedNodes = () => {
    if (!browseDeviceId) return
    for (const nodeId of selectedNodes) {
      const result = browseResults.find(r => r.node_id === nodeId)
      if (result) {
        addNodeMut.mutate({
          deviceId: browseDeviceId,
          node: {
            node_id: result.node_id,
            name: result.browse_name,
            display_name: result.display_name,
            data_type: '',
          },
        })
      }
    }
    setBrowseDeviceId(null)
    setBrowseResults([])
    setSelectedNodes(new Set())
  }

  const handleDeleteDevice = (id: number) => {
    if (!confirm('Delete this OPC-UA device?')) return
    deleteMut.mutate(id)
  }

  const thStyle: React.CSSProperties = {
    textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    borderBottom: '1px solid var(--border)',
  }
  const tdStyle: React.CSSProperties = {
    padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)',
  }

  const formSelectStyle: React.CSSProperties = {
    ...inputStyle,
    appearance: 'none' as const,
    cursor: 'pointer',
  }

  const tableContent = (
    <>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <span style={{ color: 'var(--muted)', fontSize: 13 }}>{devices.length} device{devices.length !== 1 ? 's' : ''}</span>
        <div style={{ display: 'flex', gap: 8 }}>
          <input
            placeholder="Search..."
            value={filter}
            onChange={e => setFilter(e.target.value)}
            style={{ ...inputStyle, width: 200 }}
          />
          <button
            style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}
            onClick={() => setShowForm(f => !f)}
            aria-label="Add OPC-UA device"
          >
            + Add Device
          </button>
        </div>
      </div>

      {showForm && (
        <form onSubmit={handleCreate} style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginBottom: 16, padding: 16, background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Name *</label>
            <input name="name" required style={inputStyle} placeholder="My OPC-UA Server" />
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>URL *</label>
            <input name="url" required style={inputStyle} placeholder="opc.tcp://192.168.1.100:4840" />
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Security Policy</label>
            <select name="security_policy" style={formSelectStyle}>
              {OPCUA_POLICIES.map(o => <option key={o.value} value={o.value}>{o.label}</option>)}
            </select>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Security Mode</label>
            <select name="security_mode" style={formSelectStyle}>
              {OPCUA_MODES.map(o => <option key={o.value} value={o.value}>{o.label}</option>)}
            </select>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Username</label>
            <input name="username" style={inputStyle} placeholder="(optional)" />
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Password</label>
            <input name="password" type="password" style={inputStyle} placeholder="(optional)" />
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Interval (ms)</label>
            <input name="interval_ms" type="number" min={100} defaultValue={1000} style={inputStyle} />
          </div>
          <div style={{ display: 'flex', alignItems: 'flex-end', gap: 8 }}>
            <button type="submit" style={btnStyle} disabled={createMut.isPending}>
              {createMut.isPending ? 'Creating...' : 'Create'}
            </button>
            <button type="button" style={{ ...btnStyle }} onClick={() => setShowForm(false)}>Cancel</button>
          </div>
        </form>
      )}

      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        {isLoading ? (
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>Loading...</div>
        ) : filtered.length === 0 ? (
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>No OPC-UA devices found.</div>
        ) : (
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={thStyle}>Name</th>
                <th style={thStyle}>URL</th>
                <th style={thStyle}>Security</th>
                <th style={thStyle}>Interval</th>
                <th style={thStyle}>Enabled</th>
                <th style={{ ...thStyle, width: 200 }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map(d => (
                <>
                  <tr key={d.id}>
                    <td style={tdStyle}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <button
                          onClick={() => setExpandedId(expandedId === d.id ? null : d.id)}
                          style={{ border: 'none', background: 'transparent', color: 'var(--muted)', cursor: 'pointer', fontSize: 11, padding: '2px 4px' }}
                          aria-label={expandedId === d.id ? 'Collapse nodes' : 'Expand nodes'}
                        >
                          {expandedId === d.id ? '\u25BC' : '\u25B6'}
                        </button>
                        {d.name}
                      </div>
                    </td>
                    <td style={{ ...tdStyle, fontFamily: "var(--font-mono)", fontSize: 12 }}>{d.url}</td>
                    <td style={tdStyle}>
                      <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: 'rgba(139,92,246,.1)', color: '#a78bfa' }}>
                        {d.security_policy}/{d.security_mode}
                      </span>
                    </td>
                    <td style={tdStyle}>{d.interval_ms}ms</td>
                    <td style={tdStyle}>
                      <input
                        type="checkbox"
                        checked={d.enabled}
                        onChange={() => updateMut.mutate({ id: d.id, enabled: !d.enabled })}
                        style={{ cursor: 'pointer' }}
                      />
                    </td>
                    <td style={tdStyle}>
                      <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                        {d.enabled ? (
                          <button
                            onClick={() => stopMut.mutate(d.id)}
                            style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                            aria-label={`Stop ${d.name}`}
                          >
                            Stop
                          </button>
                        ) : (
                          <button
                            onClick={() => startMut.mutate(d.id)}
                            style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: 'pointer', fontSize: 11 }}
                            aria-label={`Start ${d.name}`}
                          >
                            Start
                          </button>
                        )}
                        <button
                          onClick={() => handleBrowse(d.id)}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 11 }}
                          aria-label={`Browse ${d.name}`}
                        >
                          Browse
                        </button>
                        <button
                          onClick={() => handleDeleteDevice(d.id)}
                          style={{ padding: '4px 8px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                          aria-label={`Delete ${d.name}`}
                        >
                          Del
                        </button>
                      </div>
                    </td>
                  </tr>
                  {expandedId === d.id && (
                    <tr key={`${d.id}-nodes`}>
                      <td colSpan={6} style={{ padding: 0, borderBottom: '1px solid var(--border)' }}>
                        <ExpandedNodes deviceId={d.id} onDeleteNode={deleteNodeMut} />
                      </td>
                    </tr>
                  )}
                </>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* Browse dialog */}
      {browseDeviceId !== null && (
        <BrowseDialog
          isLoading={browseMut.isPending}
          results={browseResults}
          selectedNodes={selectedNodes}
          onToggleNode={(nodeId) => {
            setSelectedNodes(prev => {
              const next = new Set(prev)
              if (next.has(nodeId)) next.delete(nodeId)
              else next.add(nodeId)
              return next
            })
          }}
          onAdd={handleAddSelectedNodes}
          onClose={() => { setBrowseDeviceId(null); setBrowseResults([]); setSelectedNodes(new Set()) }}
        />
      )}
    </>
  )

  if (embedded) {
    return <>{tableContent}</>
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column', padding: 24 }}>
      <div style={{ maxWidth: 1200, width: '100%', margin: '0 auto', display: 'flex', flexDirection: 'column', height: '100%' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16, flexShrink: 0 }}>
          <button onClick={onClose} aria-label="Close OPC-UA devices" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
          </button>
          <h1 style={{ fontSize: 20, fontWeight: 700 }}>OPC-UA Devices</h1>
        </div>
        {tableContent}
      </div>
    </div>
  )
}

function ExpandedNodes({ deviceId, onDeleteNode }: { deviceId: number; onDeleteNode: ReturnType<typeof useDeleteOpcUaNode> }) {
  const { data: nodes = [], isLoading } = useOpcUaNodes(deviceId)

  if (isLoading) return <div style={{ padding: 16, color: 'var(--muted)', fontSize: 13 }}>Loading nodes...</div>
  if (nodes.length === 0) return <div style={{ padding: 16, color: 'var(--muted)', fontSize: 13 }}>No monitored nodes. Click "Browse" to discover and add nodes.</div>

  const nodeThStyle: React.CSSProperties = {
    textAlign: 'left', padding: '6px 12px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    background: 'rgba(255,255,255,.02)',
  }
  const nodeTdStyle: React.CSSProperties = { padding: '6px 12px', fontSize: 12 }

  return (
    <div style={{ maxHeight: 300, overflow: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse' }}>
        <thead>
          <tr>
            <th style={nodeThStyle}>Node ID</th>
            <th style={nodeThStyle}>Name</th>
            <th style={nodeThStyle}>Display Name</th>
            <th style={nodeThStyle}>Data Type</th>
            <th style={{ ...nodeThStyle, width: 80 }}>Action</th>
          </tr>
        </thead>
        <tbody>
          {nodes.map(n => (
            <tr key={n.id}>
              <td style={{ ...nodeTdStyle, fontFamily: "var(--font-mono)", fontSize: 11 }}>{n.node_id}</td>
              <td style={nodeTdStyle}>{n.name}</td>
              <td style={nodeTdStyle}>{n.display_name}</td>
              <td style={nodeTdStyle}>{n.data_type || '-'}</td>
              <td style={nodeTdStyle}>
                <button
                  onClick={() => {
                    if (confirm('Remove this node?')) {
                      onDeleteNode.mutate({ deviceId, nodeId: n.id })
                    }
                  }}
                  style={{ padding: '3px 6px', borderRadius: 4, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 11 }}
                  aria-label={`Remove node ${n.name}`}
                >
                  Remove
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function BrowseDialog({ isLoading, results, selectedNodes, onToggleNode, onAdd, onClose }: {
  isLoading: boolean
  results: BrowseResult[]
  selectedNodes: Set<string>
  onToggleNode: (nodeId: string) => void
  onAdd: () => void
  onClose: () => void
}) {
  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  const overlayStyle: React.CSSProperties = {
    position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', zIndex: 60,
    display: 'flex', alignItems: 'center', justifyContent: 'center',
  }
  const modalStyle: React.CSSProperties = {
    background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 12,
    width: '90%', maxWidth: 600, maxHeight: '80vh', display: 'flex', flexDirection: 'column',
    padding: 20,
  }

  return (
    <div style={overlayStyle} onClick={onClose}>
      <div style={modalStyle} onClick={e => e.stopPropagation()}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
          <h2 style={{ fontSize: 16, fontWeight: 600 }}>Browse Server Nodes</h2>
          <button onClick={onClose} aria-label="Close" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', borderRadius: 4, transition: 'color 0.15s' }}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>

        {isLoading ? (
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>Browsing server...</div>
        ) : results.length === 0 ? (
          <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)' }}>No nodes found on this server.</div>
        ) : (
          <div style={{ flex: 1, overflow: 'auto', border: '1px solid var(--border)', borderRadius: 8 }}>
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr>
                  <th style={{ ...thStyleForBrowse(), width: 40 }}></th>
                  <th style={thStyleForBrowse()}>Node ID</th>
                  <th style={thStyleForBrowse()}>Display Name</th>
                  <th style={thStyleForBrowse()}>Node Class</th>
                </tr>
              </thead>
              <tbody>
                {results.map(r => (
                  <tr key={r.node_id}>
                    <td style={tdStyleForBrowse()}>
                      <input
                        type="checkbox"
                        checked={selectedNodes.has(r.node_id)}
                        onChange={() => onToggleNode(r.node_id)}
                        style={{ cursor: 'pointer' }}
                      />
                    </td>
                    <td style={{ ...tdStyleForBrowse(), fontFamily: "var(--font-mono)", fontSize: 11 }}>{r.node_id}</td>
                    <td style={tdStyleForBrowse()}>{r.display_name}</td>
                    <td style={tdStyleForBrowse()}>
                      <span style={{ fontSize: 11, padding: '2px 6px', borderRadius: 4, background: 'rgba(139,92,246,.1)', color: '#a78bfa' }}>
                        {r.node_class}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: 16 }}>
          <span style={{ color: 'var(--muted)', fontSize: 12 }}>
            {selectedNodes.size} node{selectedNodes.size !== 1 ? 's' : ''} selected
          </span>
          <div style={{ display: 'flex', gap: 8 }}>
            <button style={btnStyle} onClick={onClose}>Cancel</button>
            <button
              style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }}
              onClick={onAdd}
              disabled={selectedNodes.size === 0}
            >
              Add Selected
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}

function thStyleForBrowse(): React.CSSProperties {
  return {
    textAlign: 'left', padding: '6px 10px', fontSize: 11, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    borderBottom: '1px solid var(--border)',
  }
}

function tdStyleForBrowse(): React.CSSProperties {
  return { padding: '6px 10px', fontSize: 12, borderBottom: '1px solid var(--border)' }
}
